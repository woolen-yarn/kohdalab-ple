//! External commands use the same GUI state and engine as local actions.
use super::*;
use crate::api::{self, Error, Session};
use serde_json::{Value, json};
use std::collections::VecDeque;
pub(super) struct Runtime {
    pub server: crate::remote_gui::Server,
    pub session: Session,
    active: Option<Active>,
    pending_common_release: bool,
}
struct Active {
    id: String,
    kind: String,
    remaining: VecDeque<usize>,
    current: Option<usize>,
    cancelled: bool,
}
pub(super) enum Note {
    Device(&'static str, bool),
    Finished(String, Option<String>),
    Manual(Result<f64, engine::RunError>),
    Spectrometer(Result<Value, String>),
    None,
}
impl Note {
    pub fn from_event(e: &Event) -> Self {
        match e {
            Event::Device { device, connected } => Self::Device(device, *connected),
            Event::Finished { state, error } => Self::Finished(state.label().into(), error.clone()),
            Event::ManualMoveFinished(r) => Self::Manual(r.clone()),
            Event::SpectrometerConfigured(r) => Self::Spectrometer(r.clone()),
            _ => Self::None,
        }
    }
}
impl Runtime {
    pub fn new(server: crate::remote_gui::Server) -> Self {
        Self {
            server,
            session: Session::default(),
            active: None,
            pending_common_release: false,
        }
    }
}
const DEVICES: [&str; 3] = ["spectrometer", "stage", "power"];
fn string<'a>(v: &'a Value, key: &str) -> api::Result<&'a str> {
    v.get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| Error::new("validation", format!("{key} must be a string")))
}
impl PleApp {
    pub(super) fn api_owned(&self) -> bool {
        self.api.as_ref().is_some_and(|a| a.session.lease.is_some())
    }
    fn operation_busy(&self) -> bool {
        self.busy
            || self.manual_moving
            || self.pending_connections.iter().any(|p| *p)
            || self.spectrometer_configuring
            || self.dialog.is_some()
            || self.opening.is_some()
            || self.closing
            || self.api_cleanup
    }
    pub(super) fn current_shutter_state(&self) -> Value {
        self.controller
            .shutter_status()
            .unwrap_or_else(|| self.shutter_state.clone())
    }
    fn api_snapshot(&self) -> Value {
        let (stop_pending, stop_error) = self.controller.stop_status();
        json!({"application":"kohdalab-ple","api_version":1,"phase":self.state,
            "busy":self.busy||self.manual_moving||self.pending_connections.iter().any(|p|*p)||self.spectrometer_configuring,
            "operation_busy":self.operation_busy(),
            "cleanup_pending":self.api_cleanup||stop_pending,"cleanup_error":stop_error.or_else(||self.cleanup_error()),
            "mode":self.config.mode,"rows":self.samples.len(),"planned_rows":self.run_count,"run_dir":self.run_dir,
            "awaiting":match self.awaiting{1=>"background_confirmation",2=>"signal_confirmation",3=>"analysis_selection",_=>"none"},
            "devices":{"spectrometer":self.connected[0],"stage":self.connected[1],"power":self.connected[2]},
            "stage_position_mm":self.stage_value,"sensor_power_w":self.power_value,"power_sensor_wavelength_nm":self.power_sensor_wavelength,
            "analysis":self.analysis_draft,"spectrometer_settings":self.spectrometer_settings,"power_hardware":self.power_hardware,"shutter_state":self.current_shutter_state(),"background_settings":self.config.background,
            "last_error":self.message})
    }
    pub(super) fn poll_api(&mut self, ctx: &egui::Context) {
        let (_stop_pending, stop_error) = self.controller.stop_status();
        let cleanup_error = stop_error.is_some() || self.cleanup_error().is_some();
        let cleanup_verified = self.api_cleanup
            && !self.busy
            && !self.manual_moving
            && !self.pending_connections.iter().any(|p| *p)
            && !self.controller.stop_status().0
            && !cleanup_error;
        if cleanup_verified {
            self.api_cleanup = false;
        }
        let Some(mut runtime) = self.api.take() else {
            return;
        };
        if cleanup_verified && runtime.pending_common_release {
            runtime.session.lease = None;
            runtime.pending_common_release = false;
        }
        if runtime.session.expired() {
            runtime.session.revoke();
            self.api_stop(&mut runtime);
            self.log("External control lease expired; Stop requested");
        }
        for _ in 0..8 {
            let Ok(request) = runtime.server.requests.try_recv() else {
                break;
            };
            let response = if Instant::now() > request.deadline {
                Error::new("timeout", "Expired request was not executed").response()
            } else {
                match self.api_dispatch(&mut runtime, &request.body) {
                    Ok(v) => api::success(v),
                    Err(e) => e.response(),
                }
            };
            let _ = request.reply.send(response);
        }
        self.api = Some(runtime);
        if self.api_owned() || self.api_cleanup {
            let mut stop = false;
            let mut reclaim = false;
            egui::Window::new("External control")
                .id(egui::Id::new("external_control"))
                .anchor(egui::Align2::CENTER_TOP, [0., 4.])
                .collapsible(false)
                .resizable(false)
                .show(ctx, |ui| {
                    ui.horizontal(|ui| {
                        ui.colored_label(
                            visuals::TEAL,
                            if self.api_cleanup {
                                "Waiting for Stop / cleanup"
                            } else {
                                "PYTHON / EXTERNAL CONTROL"
                            },
                        );
                        stop = ui.button("Stop").clicked();
                        reclaim = ui.button("Take control").clicked();
                    });
                });
            if (stop || reclaim)
                && let Some(mut runtime) = self.api.take()
            {
                if reclaim {
                    runtime.session.revoke();
                }
                self.api_stop(&mut runtime);
                self.api = Some(runtime);
            }
        }
    }
    fn api_stop(&mut self, runtime: &mut Runtime) {
        if let Some(a) = &mut runtime.active {
            a.cancelled = true;
            a.remaining.clear();
        }
        self.controller.stop();
        self.api_cleanup = true;
    }
    fn api_dispatch(&mut self, r: &mut Runtime, body: &Value) -> api::Result<Value> {
        if body
            .get("api_version")
            .is_some_and(|v| v.as_u64() != Some(1))
            || body.get("revision").is_some_and(|v| v.as_u64() != Some(1))
        {
            return Err(Error::new("version", "Unsupported PLE API revision"));
        }
        let method = body
            .get("method")
            .or_else(|| body.get("action"))
            .and_then(Value::as_str)
            .ok_or_else(|| Error::new("validation", "method or action is required"))?;
        let common = body.get("_control_method").is_some();
        let method = match method {
            "control_describe" => "capabilities",
            "control_status" => "status",
            "control_result" => "result",
            other => other,
        };
        match method {
            "capabilities" => {
                return Ok(
                    json!({"application":"kohdalab-ple","application_version":env!("CARGO_PKG_VERSION"),"api_version":1,"control_protocol":1,"transport_revision":1,
                "methods":["control.describe","control.status","control.devices","control.config","control.plan","control.output","control.spectrum","control.background","control.preview","control.shutter","control.acquire","control.heartbeat","control.release","control.result","control.configure","control.set_plan","control.set_background","control.set_output","control.apply_spectrometer","control.connect","control.disconnect","control.start","control.move","control.confirm_background","control.confirm_signal","control.select_analysis","control.stop"],
                "actions":["capabilities","status","devices","config","plan","output","spectrum","background","preview","shutter","acquire","heartbeat","release","configure","set_plan","set_background","set_output","apply_spectrometer","connect","disconnect","start","move","confirm_background","confirm_signal","select_analysis","result","stop"],
                "history_limit":api::HISTORY_LIMIT,"command_history_limit":api::HISTORY_LIMIT,"lease_timeout_s":10,"headless":false,"arbitrary_device_commands":false,
                "safety":{"lease_timeout_seconds":10,"heartbeat_seconds":2,"on_disconnect":"stop","on_release":"stop","stop_requires_lease":false,"stop_policy_defaults":{"stop_on_disconnect":true,"stop_on_error":true,"stop_on_timeout":true}},
                "safety_policy":{"stop_on_disconnect":true,"stop_on_error":true,"stop_on_timeout":true,"automatic_shutter":self.config.background.mode==BackgroundMode::Automatic,"shutter_backend":self.config.background.backend,"shutter_feedback":"controller_state_not_optical_sensor","manual_beam_block_required":self.config.background.mode==BackgroundMode::Manual},
                "stage_travel_mm":[0,25],"confirmation_required":true}),
                );
            }
            "status" => {
                let mut s = self.api_snapshot();
                s["control_owned"] = json!(r.session.lease.is_some());
                s["active_job"] = json!(r.active.as_ref().map(|a| &a.id));
                if common {
                    let cleanup_error = s["cleanup_error"].is_string();
                    let cleanup_pending = s["cleanup_pending"].as_bool().unwrap_or(false);
                    s["ownership"] = json!({"owned": r.session.lease.is_some(), "active_job": r.active.as_ref().map(|a| &a.id)});
                    s["cleanup"] = json!(if cleanup_error {
                        "unknown"
                    } else if cleanup_pending {
                        "pending"
                    } else {
                        "not_required"
                    });
                    s["phase"] = json!(if cleanup_error {
                        "error"
                    } else if cleanup_pending {
                        "cleanup_pending"
                    } else if s["busy"].as_bool().unwrap_or(false) {
                        "running"
                    } else {
                        "ready"
                    });
                }
                return Ok(s);
            }
            "devices" => return Ok(self.api_snapshot()["devices"].clone()),
            "shutter" => {
                return Ok(
                    json!({"settings":self.config.background,"state":self.current_shutter_state()}),
                );
            }
            "config" => return Ok(json!({"profile":self.config,"config_path":self.config_path})),
            "plan" => {
                return Ok(
                    json!({"scan":self.config.scan,"analysis":self.config.initial_analysis(),"resolved":self.config.plan().map_err(|e|Error::new("validation",e))?}),
                );
            }
            "output" => {
                return Ok(
                    json!({"directory":self.config.output_dir,"sample":self.config.sample,"run_dir":self.run_dir}),
                );
            }
            "result" => {
                let id = body
                    .get("job_id")
                    .or_else(|| body.get("id"))
                    .or_else(|| body.get("request_id"))
                    .and_then(Value::as_str)
                    .ok_or_else(|| Error::new("validation", "id is required"))?;
                if r.active.as_ref().is_some_and(|a| a.id == id) {
                    r.session
                        .update(id, "running", self.api_snapshot(), None, None);
                }
                let receipt = r.session.receipt(id)?;
                return if common {
                    let status = receipt["status"].as_str().unwrap_or("error").to_owned();
                    let kind = receipt["kind"].as_str().unwrap_or("run").to_owned();
                    let linked = receipt["run_job"].as_str().map(str::to_owned);
                    Ok(kohdalab_control_protocol::receipt(
                        id,
                        &kind,
                        &status,
                        if self.api_snapshot()["cleanup_error"].is_string() {
                            "unknown"
                        } else if self.api_snapshot()["cleanup_pending"]
                            .as_bool()
                            .unwrap_or(false)
                        {
                            "pending"
                        } else {
                            "not_required"
                        },
                        linked.as_deref(),
                        receipt,
                    ))
                } else {
                    Ok(receipt)
                };
            }
            "spectrum" | "preview" => {
                return if let Some(a) = &self.preview {
                    Ok(
                        json!({"kind":"preview","file_base":a.file_base,"wavelength_nm":a.spectrum.wavelength_nm,"counts":a.spectrum.counts,"corrected_counts":a.corrected_counts,"sample_power_w":a.sample_power_w}),
                    )
                } else if method == "spectrum" {
                    Ok(json!({"kind":"sample","sample":self.samples.last().map(Arc::as_ref)}))
                } else {
                    Ok(Value::Null)
                };
            }
            "background" => return Ok(json!(self.background)),
            "stop" => {
                let job_id = r.active.as_ref().map(|a| a.id.clone());
                self.api_stop(r);
                return Ok(if common {
                    json!({"requested":true,"job_id":job_id,"cleanup":"pending","native":{"stop_requested":true,"confirmed":false}})
                } else {
                    json!({"stop_requested":true,"confirmed":false})
                });
            }
            "acquire" => {
                let result = if common {
                    r.session.claim_common(
                        string(body, "client_id")?,
                        body.get("lease").and_then(Value::as_str),
                        self.operation_busy(),
                    )?
                } else {
                    r.session
                        .claim(string(body, "client_id")?, self.operation_busy())?
                };
                return if common {
                    Ok(
                        json!({"client_id":body["client_id"],"lease":result["lease"],"heartbeat_seconds":2,"timeout_seconds":10,"native":result}),
                    )
                } else {
                    Ok(result)
                };
            }
            _ => {}
        }
        if common {
            r.session.require_common(body)?;
        } else {
            r.session.require(body)?;
        }
        let tracked = matches!(
            method,
            "configure"
                | "set_plan"
                | "set_background"
                | "set_output"
                | "confirm_background"
                | "confirm_signal"
                | "select_analysis"
        );
        let command_id = if tracked {
            Some(match body.get("request_id") {
                Some(v) => v
                    .as_str()
                    .ok_or_else(|| Error::new("validation", "request_id must be a UUID"))?
                    .to_owned(),
                None => crate::remote_gui::new_id().map_err(|e| Error::new("internal", e))?,
            })
        } else {
            None
        };
        let fingerprint = api::fingerprint(method, body);
        if let Some(id) = &command_id {
            if let Some(previous) = r.session.previous(id, &fingerprint)? {
                return Ok(previous);
            }
            r.session.can_remember()?;
        }
        let result = match method {
            "heartbeat" => Ok(json!({"alive":true,"timeout_s":10})),
            "release" => {
                if self.operation_busy() || self.controller.stop_status().0 {
                    self.api_stop(r);
                    if common {
                        r.pending_common_release = true;
                    }
                    return Ok(if common {
                        json!({"released":false,"cleanup":"pending","native":{"cleanup_pending":true}})
                    } else {
                        json!({"released":false,"cleanup_pending":true})
                    });
                }
                r.session.lease = None;
                Ok(if common {
                    json!({"released":true,"cleanup":"verified","native":{"cleanup_pending":false}})
                } else {
                    json!({"released":true,"cleanup_pending":false})
                })
            }
            "configure" => {
                self.api_idle()?;
                if self.connected.iter().any(|v| *v) {
                    return Err(Error::new(
                        "busy",
                        "Disconnect every device before changing a profile",
                    ));
                }
                let c = parse_profile(
                    body.get("config")
                        .or_else(|| body.get("profile"))
                        .cloned()
                        .ok_or_else(|| Error::new("validation", "config is required"))?,
                )
                .map_err(|e| Error::new("validation", e))?;
                self.analysis_draft = c.initial_analysis();
                self.config = c;
                self.preview = None;
                self.stage_history.clear();
                self.power_history.clear();
                Ok(json!({"configured":true}))
            }
            "set_background" => {
                self.api_idle()?;
                let mut c = self.config.clone();
                c.background = serde_json::from_value(
                    body.get("background")
                        .cloned()
                        .ok_or_else(|| Error::new("validation", "background is required"))?,
                )
                .map_err(|e| Error::new("validation", e.to_string()))?;
                c.plan().map_err(|e| Error::new("validation", e))?;
                self.config = c;
                Ok(json!({"background":self.config.background}))
            }
            "set_plan" => {
                self.api_idle()?;
                let mut c = self.config.clone();
                c.scan = serde_json::from_value(
                    body.get("scan")
                        .or_else(|| body.get("plan"))
                        .cloned()
                        .ok_or_else(|| Error::new("validation", "scan is required"))?,
                )
                .map_err(|e| Error::new("validation", e.to_string()))?;
                let resolved = c.plan().map_err(|e| Error::new("validation", e))?;
                self.config = c;
                Ok(json!({"scan":self.config.scan,"resolved":resolved}))
            }
            "set_output" => {
                self.api_idle()?;
                let mut c = self.config.clone();
                if let Some(v) = body.get("directory") {
                    c.output_dir = v
                        .as_str()
                        .ok_or_else(|| Error::new("validation", "directory must be a string"))?
                        .into();
                }
                if let Some(v) = body.get("sample") {
                    c.sample = v
                        .as_str()
                        .ok_or_else(|| Error::new("validation", "sample must be a string"))?
                        .into();
                }
                c.plan().map_err(|e| Error::new("validation", e))?;
                self.config = c;
                Ok(json!({"directory":self.config.output_dir,"sample":self.config.sample}))
            }
            "confirm_background" | "confirm_signal" => {
                let expected = if method == "confirm_background" { 1 } else { 2 };
                if self.awaiting != expected || !self.busy {
                    return Err(Error::new(
                        "invalid_state",
                        "That confirmation is not currently requested",
                    ));
                }
                if body["confirmed"].as_bool() != Some(true) {
                    return Err(Error::new(
                        "validation",
                        "Explicit confirmed=true is required after checking the optical path",
                    ));
                }
                self.awaiting = 0;
                self.send(Command::Continue);
                Ok(json!({"accepted":true}))
            }
            "select_analysis" => {
                if self.awaiting != 3 || self.analysis_confirm_pending {
                    return Err(Error::new(
                        "invalid_state",
                        "Wait for the first PL preview before selecting analysis",
                    ));
                }
                let analysis: AnalysisConfig = serde_json::from_value(body["analysis"].clone())
                    .map_err(|e| Error::new("validation", e.to_string()))?;
                analysis
                    .validate(
                        &self
                            .preview
                            .as_ref()
                            .ok_or_else(|| Error::new("invalid_state", "No preview spectrum"))?
                            .spectrum
                            .wavelength_nm,
                    )
                    .map_err(|e| Error::new("validation", e))?;
                let mut c = self.config.clone();
                c.analysis = Some(analysis.clone());
                if let Some(v) = body.get("excitation_method") {
                    c.excitation_method = serde_json::from_value(v.clone())
                        .map_err(|e| Error::new("validation", e.to_string()))?;
                }
                for (key, value) in [
                    ("excitation_min_nm", &mut c.excitation_min_nm),
                    ("excitation_max_nm", &mut c.excitation_max_nm),
                    ("excitation_min_snr", &mut c.excitation_min_snr),
                ] {
                    if let Some(v) = body.get(key) {
                        *value = v.as_f64().ok_or_else(|| {
                            Error::new("validation", format!("{key} must be numeric"))
                        })?;
                    }
                }
                if let Some(v) = body.get("excitation_peaks") {
                    c.excitation_peaks = serde_json::from_value(v.clone())
                        .map_err(|e| Error::new("validation", e.to_string()))?;
                }
                c.plan().map_err(|e| Error::new("validation", e))?;
                self.analysis_draft = analysis;
                self.analysis_confirm_pending = true;
                self.send(Command::SelectAnalysis(Box::new(c)));
                Ok(json!({"accepted":true}))
            }
            "connect" | "disconnect" | "start" | "move" | "apply_spectrometer" => {
                let native = self.api_job(r, method, body)?;
                if common {
                    let id = native["id"]
                        .as_str()
                        .or_else(|| native["run_job"].as_str())
                        .ok_or_else(|| Error::new("validation", "Job receipt has no id"))?;
                    let id = id.to_owned();
                    let status = native["status"].as_str().unwrap_or("running").to_owned();
                    let kind = native["kind"].as_str().unwrap_or(method).to_owned();
                    let linked = native["run_job"].as_str().map(str::to_owned);
                    return Ok(kohdalab_control_protocol::receipt(
                        &id,
                        &kind,
                        &status,
                        if self.api_snapshot()["cleanup_error"].is_string() {
                            "unknown"
                        } else if self.api_snapshot()["cleanup_pending"]
                            .as_bool()
                            .unwrap_or(false)
                        {
                            "pending"
                        } else {
                            "not_required"
                        },
                        linked.as_deref(),
                        native,
                    ));
                }
                Ok(native)
            }
            _ => Err(Error::new(
                "unsupported",
                "Unsupported method; inspect capabilities",
            )),
        }?;
        if let Some(id) = command_id {
            r.session.remember_command(
                id,
                method,
                fingerprint,
                result,
                r.active.as_ref().map(|a| a.id.as_str()),
            )
        } else {
            Ok(result)
        }
    }
    fn api_idle(&self) -> api::Result<()> {
        if self.operation_busy() || self.controller.stop_status().0 {
            Err(Error::new(
                "busy",
                "An operation or dialog is active; changes are not queued",
            ))
        } else {
            Ok(())
        }
    }
    fn api_job(&mut self, r: &mut Runtime, method: &str, body: &Value) -> api::Result<Value> {
        let id = match body.get("request_id") {
            Some(v) => v
                .as_str()
                .ok_or_else(|| Error::new("validation", "request_id must be a UUID string"))?
                .to_owned(),
            None => crate::remote_gui::new_id().map_err(|e| Error::new("internal", e))?,
        };
        let id = id.to_ascii_lowercase();
        let f = api::fingerprint(method, body);
        if let Some(previous) = r.session.previous(&id, &f)? {
            return Ok(previous);
        }
        self.api_idle()?;
        if matches!(method, "start" | "move" | "apply_spectrometer")
            && let Some(reason) = self.recovery_reason()
        {
            return Err(Error::new("cleanup_unconfirmed", reason));
        }
        let mut devices = VecDeque::new();
        let mut target = 0.;
        match method {
            "start" => {
                if !self.connected.iter().all(|v| *v) {
                    return Err(Error::new("disconnected", "Connect all devices first"));
                }
                self.config
                    .plan()
                    .map_err(|e| Error::new("validation", e))?;
            }
            "move" => {
                target = body["position_mm"]
                    .as_f64()
                    .ok_or_else(|| Error::new("validation", "position_mm is required"))?;
                if !self.connected[1] {
                    return Err(Error::new("disconnected", "Connect the stage first"));
                }
                if !target.is_finite()
                    || target < self.config.stage.min_mm
                    || target > self.config.stage.max_mm
                {
                    return Err(Error::new(
                        "validation",
                        "Target is outside the configured CONEX limits (within 0–25 mm)",
                    ));
                }
            }
            "connect" | "disconnect" => {
                let requested = body
                    .get("devices")
                    .cloned()
                    .unwrap_or_else(|| json!(DEVICES));
                let roles = requested
                    .as_array()
                    .ok_or_else(|| Error::new("validation", "devices must be an array"))?;
                if roles.is_empty() {
                    return Err(Error::new("validation", "Choose at least one device"));
                }
                for role in roles {
                    let i = DEVICES
                        .iter()
                        .position(|d| Some(*d) == role.as_str())
                        .ok_or_else(|| Error::new("validation", "Unknown device role"))?;
                    if devices.contains(&i) {
                        return Err(Error::new("validation", "Device roles must be unique"));
                    }
                    if self.connected[i] != (method == "connect")
                        || (method == "disconnect"
                            && i == 0
                            && self.controller.has_spectrometer_owner())
                    {
                        devices.push_back(i);
                    }
                }
            }
            "apply_spectrometer" => {
                if !self.connected[0] {
                    return Err(Error::new("disconnected", "Connect the spectrometer first"));
                }
                let settings = body
                    .get("settings")
                    .and_then(Value::as_object)
                    .ok_or_else(|| Error::new("validation", "settings must be an object"))?;
                const ALLOWED: [&str; 5] = [
                    "exposure_ms",
                    "frames_combined",
                    "center_nm",
                    "grating",
                    "timeout_s",
                ];
                if settings.keys().any(|key| !ALLOWED.contains(&key.as_str())) {
                    return Err(Error::new(
                        "validation",
                        "settings may contain only exposure_ms, frames_combined, center_nm, grating, and timeout_s",
                    ));
                }
                let mut c = self.config.clone();
                let mut configured = serde_json::to_value(&c.spectrometer)
                    .map_err(|e| Error::new("validation", e.to_string()))?;
                let target = configured.as_object_mut().ok_or_else(|| {
                    Error::new("internal", "spectrometer config is not an object")
                })?;
                for (key, value) in settings {
                    target.insert(key.clone(), value.clone());
                }
                c.spectrometer = serde_json::from_value(configured)
                    .map_err(|e| Error::new("validation", e.to_string()))?;
                c.plan().map_err(|e| Error::new("validation", e))?;
                self.config.spectrometer = c.spectrometer.clone();
            }
            _ => unreachable!(),
        }
        let receipt = r.session.reserve(id.clone(), method, f)?;
        r.active = Some(Active {
            id,
            kind: method.into(),
            remaining: devices,
            current: None,
            cancelled: false,
        });
        match method {
            "start" => {
                self.start_for(r.active.as_ref().map(|a| a.id.clone()));
                if !self.busy {
                    self.api_complete(
                        r,
                        "error",
                        self.message.clone(),
                        Some("cleanup_unconfirmed"),
                    );
                }
            }
            "move" => match self.controller.manual_move(target) {
                Ok(()) => {
                    self.manual_moving = true;
                    self.manual_target_mm = target;
                }
                Err(e) => self.api_complete(r, "error", Some(e), Some("device")),
            },
            "apply_spectrometer" => {
                self.spectrometer_configuring = true;
                self.send(Command::ConfigureSpectrometer(Box::new(
                    self.config.clone(),
                )));
            }
            _ => self.api_next_device(r),
        }
        r.session.receipt(receipt["id"].as_str().unwrap())
    }
    fn api_next_device(&mut self, r: &mut Runtime) {
        let Some(active) = &mut r.active else {
            return;
        };
        if active.cancelled {
            self.api_complete(
                r,
                "stopped",
                Some("Stop requested; remaining device operations cancelled".into()),
                Some("stopped"),
            );
            return;
        }
        if let Some(index) = active.remaining.pop_front() {
            active.current = Some(index);
            self.pending_connections[index] = true;
            self.send(if active.kind == "connect" {
                Command::Connect(DEVICES[index], self.config.clone())
            } else {
                Command::Disconnect(DEVICES[index])
            });
        } else {
            self.api_complete(r, "completed", None, None);
        }
    }
    fn api_complete(
        &self,
        r: &mut Runtime,
        status: &str,
        error: Option<String>,
        code: Option<&str>,
    ) {
        if let Some(a) = r.active.take() {
            r.session
                .update(&a.id, status, self.api_snapshot(), error, code);
        }
    }
    pub(super) fn api_observe(&mut self, note: Note) {
        let Some(mut r) = self.api.take() else {
            return;
        };
        if let Some(a) = &r.active {
            match note {
                Note::Device(role, connected) if a.current.is_some_and(|i| DEVICES[i] == role) => {
                    if connected != (a.kind == "connect") {
                        self.api_complete(
                            &mut r,
                            "error",
                            Some(format!("{role} operation failed; inspect log")),
                            Some("device"),
                        );
                    } else {
                        self.api_next_device(&mut r);
                    }
                }
                Note::Finished(state, error) if a.kind == "start" => {
                    let (status, code) = if state == "Completed" {
                        ("completed", None)
                    } else if state == "Stopped" {
                        ("stopped", Some("stopped"))
                    } else {
                        ("error", Some("device"))
                    };
                    self.api_complete(&mut r, status, error, code);
                }
                Note::Manual(result) if a.kind == "move" => match result {
                    Ok(_) => self.api_complete(&mut r, "completed", None, None),
                    Err(e) => {
                        let stopped = matches!(e, engine::RunError::Stopped(_));
                        self.api_complete(
                            &mut r,
                            if stopped { "stopped" } else { "error" },
                            Some(e.to_string()),
                            Some(if stopped { "stopped" } else { "device" }),
                        );
                    }
                },
                Note::Spectrometer(result) if a.kind == "apply_spectrometer" => {
                    let cancelled = a.cancelled;
                    match result {
                        Ok(_) if cancelled => self.api_complete(
                            &mut r,
                            "stopped",
                            Some(
                                "Stop requested while spectrometer configuration was in progress"
                                    .into(),
                            ),
                            Some("stopped"),
                        ),
                        Ok(_) => self.api_complete(&mut r, "completed", None, None),
                        Err(error) => {
                            self.api_complete(&mut r, "error", Some(error), Some("device"))
                        }
                    }
                }
                _ => {}
            }
        }
        self.api = Some(r);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn call(app: &mut PleApp, body: Value) -> api::Result<Value> {
        let mut r = app.api.take().unwrap();
        let result = app.api_dispatch(&mut r, &body);
        app.api = Some(r);
        result
    }
    fn setup() -> (PleApp, egui::Context, tempfile::TempDir, String) {
        let dir = tempfile::tempdir().unwrap();
        let ctx = egui::Context::default();
        let launch = Launch {
            api_server: Some(crate::remote_gui::Server::start(dir.path().join("api")).unwrap()),
            ..Default::default()
        };
        let mut app = PleApp::with_context(&ctx, launch);
        let client = crate::remote_gui::new_id().unwrap();
        let lease =
            call(&mut app, json!({"action":"acquire","client_id":client})).unwrap()["lease"]
                .as_str()
                .unwrap()
                .to_owned();
        (app, ctx, dir, lease)
    }
    #[test]
    fn opening_saved_run_blocks_control_acquisition_until_finished() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = egui::Context::default();
        let launch = Launch {
            api_server: Some(crate::remote_gui::Server::start(dir.path().join("api")).unwrap()),
            ..Default::default()
        };
        let mut app = PleApp::with_context(&ctx, launch);
        let (release, gate) = std::sync::mpsc::channel::<()>();
        app.spawn_open(&ctx, move |_| {
            let _ = gate.recv();
            Err("synthetic unreadable run".into())
        });
        assert!(app.opening.is_some());
        let client = crate::remote_gui::new_id().unwrap();
        let acquire = json!({"action":"acquire","client_id":client});
        assert_eq!(call(&mut app, acquire.clone()).unwrap_err().code, "busy");
        let common = json!({"action":"acquire","_control_method":"acquire","client_id":client});
        assert_eq!(call(&mut app, common).unwrap_err().code, "busy");
        let status = call(&mut app, json!({"action":"status"})).unwrap();
        assert_eq!(status["operation_busy"], true);

        release.send(()).unwrap();
        let deadline = Instant::now() + Duration::from_secs(30);
        while app.opening.is_some() {
            assert!(Instant::now() < deadline, "saved-run reader did not finish");
            app.poll_open();
            std::thread::yield_now();
        }
        assert!(call(&mut app, acquire).unwrap()["lease"].is_string());
    }
    #[test]
    fn configure_accepts_saved_ple_profile_and_rejects_foreign_identity() {
        let (mut app, _ctx, dir, lease) = setup();
        let path = dir.path().join("config.json");
        let mut config = app.config.clone();
        config.sample = "saved_ple_profile".into();
        save_profile(&path, &config).unwrap();
        let mut profile: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        let body = json!({"action":"configure", "lease":lease, "request_id":crate::remote_gui::new_id().unwrap(), "config":profile});
        assert_eq!(call(&mut app, body.clone()).unwrap()["configured"], true);
        assert_eq!(call(&mut app, body).unwrap()["configured"], true);
        assert_eq!(app.config.sample, "saved_ple_profile");
        profile["application"] = json!("kohdalab-iv");
        let error = call(&mut app, json!({"action":"configure", "lease":lease, "request_id":crate::remote_gui::new_id().unwrap(), "config":profile})).unwrap_err();
        assert_eq!(error.code, "validation");
        assert_eq!(app.config.sample, "saved_ple_profile");
    }
    #[test]
    fn common_reacquire_requires_lease_and_matching_client() {
        let (mut app, _ctx, _dir, lease) = setup();
        let client = app
            .api
            .as_ref()
            .unwrap()
            .session
            .lease
            .as_ref()
            .unwrap()
            .client
            .clone();
        let missing = call(
            &mut app,
            json!({"action":"acquire","_control_method":"acquire","client_id":client}),
        )
        .unwrap_err();
        assert_eq!(missing.code, "invalid_lease");
        let wrong = call(
            &mut app,
            json!({"action":"acquire","_control_method":"acquire","client_id":crate::remote_gui::new_id().unwrap(),"lease":lease}),
        )
        .unwrap_err();
        assert_eq!(wrong.code, "ownership");
    }
    #[test]
    fn common_status_uses_cleanup_pending_phase_and_non_verified_receipt() {
        let (mut app, _ctx, _dir, lease) = setup();
        let client = app
            .api
            .as_ref()
            .unwrap()
            .session
            .lease
            .as_ref()
            .unwrap()
            .client
            .clone();
        let status = call(
            &mut app,
            json!({"action":"status","_control_method":"status","client_id":client,"lease":lease}),
        )
        .unwrap();
        assert_eq!(status["phase"], "ready");
        assert_eq!(status["cleanup"], "not_required");
        app.api_cleanup = true;
        let status = call(
            &mut app,
            json!({"action":"status","_control_method":"status","client_id":client,"lease":lease}),
        )
        .unwrap();
        assert_eq!(status["phase"], "cleanup_pending");
        assert_eq!(status["cleanup"], "pending");
    }
    #[test]
    fn common_describe_advertises_configuration_and_confirmation_actions() {
        let (mut app, _ctx, _dir, _lease) = setup();
        let description = call(
            &mut app,
            json!({"action":"control_describe","_control_method":"describe"}),
        )
        .unwrap();
        assert_eq!(description["safety"]["stop_requires_lease"], false);
        for method in [
            "control.configure",
            "control.set_plan",
            "control.confirm_background",
            "control.confirm_signal",
            "control.select_analysis",
        ] {
            assert!(
                description["methods"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|v| v == method)
            );
        }
    }
    #[test]
    fn common_release_during_active_work_releases_after_cleanup() {
        let (mut app, ctx, _dir, lease) = setup();
        let client = app
            .api
            .as_ref()
            .unwrap()
            .session
            .lease
            .as_ref()
            .unwrap()
            .client
            .clone();
        app.busy = true;
        let pending = call(
            &mut app,
            json!({"action":"release","_control_method":"release","client_id":client,"lease":lease}),
        )
        .unwrap();
        assert_eq!(pending["released"], false);
        assert_eq!(pending["cleanup"], "pending");
        assert!(app.api.as_ref().unwrap().session.lease.is_some());
        app.busy = false;
        let _ = ctx.run_ui(Default::default(), |ui| app.poll_api(ui.ctx()));
        assert!(app.api.as_ref().unwrap().session.lease.is_none());
        assert!(!app.api_cleanup);
    }
    #[test]
    fn common_release_cleanup_error_keeps_ownership_and_reports_unknown() {
        let (mut app, ctx, _dir, lease) = setup();
        let client = app
            .api
            .as_ref()
            .unwrap()
            .session
            .lease
            .as_ref()
            .unwrap()
            .client
            .clone();
        app.busy = true;
        let pending = call(
            &mut app,
            json!({"action":"release","_control_method":"release","client_id":client,"lease":lease}),
        )
        .unwrap();
        assert_eq!(pending["cleanup"], "pending");
        app.busy = false;
        app.cleanup_errors[crate::engine::CleanupCause::SpectrometerStop as usize] =
            Some("cleanup acknowledgement failed".into());
        let _ = ctx.run_ui(Default::default(), |ui| app.poll_api(ui.ctx()));
        assert!(app.api.as_ref().unwrap().session.lease.is_some());
        assert!(app.api_cleanup);
        let status = call(
            &mut app,
            json!({"action":"status","_control_method":"status","client_id":client,"lease":lease}),
        )
        .unwrap();
        assert_eq!(status["phase"], "error");
        assert_eq!(status["cleanup"], "unknown");
    }
    #[test]
    fn native_cleanup_completion_does_not_revoke_native_lease() {
        let (mut app, ctx, _dir, _lease) = setup();
        assert!(app.api.as_ref().unwrap().session.lease.is_some());
        app.api_cleanup = true;
        let _ = ctx.run_ui(Default::default(), |ui| app.poll_api(ui.ctx()));
        assert!(!app.api_cleanup);
        assert!(app.api.as_ref().unwrap().session.lease.is_some());
    }
    #[test]
    fn background_api_validates_idle_ownership_and_idempotency() {
        let (mut app, _ctx, _dir, lease) = setup();
        let body = json!({"action":"set_background","lease":lease,"request_id":crate::remote_gui::new_id().unwrap(),"background":{"mode":"automatic","backend":"kohdalab_shutter","channel":0,"endpoint":""}});
        let result = call(&mut app, body.clone()).unwrap();
        assert_eq!(result["background"]["backend"], "kohdalab_shutter");
        assert_eq!(call(&mut app, body).unwrap(), result);
        let old = serde_json::to_value(&app.config.background).unwrap();
        assert!(
            call(
                &mut app,
                json!({"action":"set_background","background":{"mode":"manual"}})
            )
            .is_err()
        );
        assert!(call(&mut app,json!({"action":"set_background","lease":lease,"background":{"mode":"automatic","backend":"kohdalab_shutter","channel":256}})).is_err());
        assert_eq!(serde_json::to_value(&app.config.background).unwrap(), old);
        app.busy = true;
        assert!(
            call(
                &mut app,
                json!({"action":"set_background","lease":lease,"background":{"mode":"manual"}})
            )
            .is_err()
        );
        assert_eq!(
            call(&mut app, json!({"action":"shutter"})).unwrap()["settings"]["backend"],
            "kohdalab_shutter"
        );
    }
    #[test]
    fn manual_move_receipts_classify_typed_errors_instead_of_message_prefixes() {
        for (error, expected) in [
            (engine::RunError::Stopped("operator cancelled"), "stopped"),
            (
                engine::RunError::Failed("Stop requested; device failed".into()),
                "error",
            ),
        ] {
            let (mut app, _ctx, _dir, lease) = setup();
            app.connected[1] = true;
            let receipt = call(
                &mut app,
                json!({"action":"move","lease":lease,"position_mm":18.}),
            )
            .unwrap();
            app.api_observe(Note::from_event(&Event::ManualMoveFinished(Err(error))));
            assert_eq!(
                app.api
                    .as_ref()
                    .unwrap()
                    .session
                    .receipt(receipt["id"].as_str().unwrap())
                    .unwrap()["status"],
                expected
            );
        }
    }

    #[test]
    fn unconfirmed_cleanup_rejects_new_mutations_without_reserving_jobs() {
        let (mut app, _ctx, _dir, lease) = setup();
        app.cleanup_errors[crate::engine::CleanupCause::Shutter as usize] =
            Some("MFF closed state could not be confirmed".into());
        for action in ["start", "move", "apply_spectrometer"] {
            let id = crate::remote_gui::new_id().unwrap();
            let error = call(&mut app, json!({"action":action,"lease":lease,"request_id":id,"position_mm":18.,"settings":{"exposure_ms":1.}})).unwrap_err();
            assert_eq!(error.code, "cleanup_unconfirmed");
            assert!(app.api.as_ref().unwrap().session.receipt(&id).is_err());
            assert!(!app.busy);
        }
        // Recovery and read-only status remain available.
        assert!(call(&mut app, json!({"action":"status"})).unwrap()["cleanup_error"].is_string());
        assert_eq!(
            call(&mut app, json!({"action":"disconnect","lease":lease})).unwrap()["status"],
            "completed"
        );
        app.start();
        assert!(!app.busy);
        assert!(app.message.as_ref().unwrap().contains("recovery"));
    }
    #[test]
    fn failed_reconnect_exposes_cleanup_and_api_disconnect_recovers_retained_owner() {
        let (mut app, _ctx, dir, lease) = setup();
        let script = dir.path().join("bridge.py");
        let allow_shutdown = dir.path().join("allow-shutdown");
        std::fs::write(&script, format!(
            "import sys,json,pathlib\nfor line in sys.stdin:\n r=json.loads(line);op=r['op'];ok=op!='shutdown' or pathlib.Path({allow}).exists()\n print(json.dumps({{'id':r['id'],'ok':ok,'result':{{'settings':{{}},'shutdown':ok,'idle_confirmed':True}},'error':'shutdown not confirmed'}}),flush=True)\n if op=='shutdown' and ok:break\n",
            allow=serde_json::to_string(&allow_shutdown.to_string_lossy()).unwrap()
        )).unwrap();
        app.config.mode = Mode::Hardware;
        app.config.spectrometer.bridge_script = script.to_string_lossy().into();
        app.controller
            .commands
            .send(Command::Connect("spectrometer", app.config.clone()))
            .unwrap();
        pump_until(&mut app, |a| a.connected[0]);
        app.controller
            .commands
            .send(Command::Connect("spectrometer", app.config.clone()))
            .unwrap();
        pump_until(&mut app, |a| {
            !a.connected[0] && a.recovery_reason().is_some()
        });
        assert!(app.controller.has_spectrometer_owner());
        assert_eq!(
            call(&mut app, json!({"action":"start","lease":lease}))
                .unwrap_err()
                .code,
            "cleanup_unconfirmed"
        );
        let first = call(
            &mut app,
            json!({"action":"disconnect","lease":lease,"devices":["spectrometer"]}),
        )
        .unwrap();
        assert_eq!(
            wait_receipt(&mut app, first["id"].as_str().unwrap())["status"],
            "error"
        );
        assert!(app.controller.has_spectrometer_owner());
        std::fs::write(&allow_shutdown, "ready").unwrap();
        let recovered = call(
            &mut app,
            json!({"action":"disconnect","lease":lease,"devices":["spectrometer"]}),
        )
        .unwrap();
        assert_eq!(
            wait_receipt(&mut app, recovered["id"].as_str().unwrap())["status"],
            "completed"
        );
        assert!(!app.controller.has_spectrometer_owner());
        assert!(app.recovery_reason().is_none());
    }
    fn pump_until(app: &mut PleApp, condition: impl Fn(&PleApp) -> bool) {
        let until = Instant::now() + Duration::from_secs(5);
        while !condition(app) {
            assert!(Instant::now() < until, "phase: {}", app.state);
            app.events();
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    fn wait_receipt(app: &mut PleApp, id: &str) -> Value {
        pump_until(app, |a| {
            a.api.as_ref().unwrap().session.receipt(id).unwrap()["status"] != "running"
        });
        app.api.as_ref().unwrap().session.receipt(id).unwrap()
    }
    #[test]
    fn api_applies_connected_spectrometer_patch_once_and_records_readback() {
        let (mut app, _ctx, _dir, lease) = setup();
        let connected = call(
            &mut app,
            json!({"action":"connect","lease":lease,"devices":["spectrometer"],"request_id":crate::remote_gui::new_id().unwrap()}),
        )
        .unwrap();
        assert_eq!(
            wait_receipt(&mut app, connected["id"].as_str().unwrap())["status"],
            "completed"
        );

        let id = crate::remote_gui::new_id().unwrap();
        let request = json!({"action":"apply_spectrometer","lease":lease,"request_id":id,"settings":{
            "exposure_ms":25.0,"frames_combined":2,"center_nm":775.0,"grating":"","timeout_s":60.0
        }});
        let first = call(&mut app, request.clone()).unwrap();
        assert_eq!(call(&mut app, request.clone()).unwrap(), first);
        assert_eq!(
            call(
                &mut app,
                json!({"action":"apply_spectrometer","lease":lease,"request_id":crate::remote_gui::new_id().unwrap(),"settings":{"center_nm":776.0}}),
            )
            .unwrap_err()
            .code,
            "busy"
        );
        let receipt = wait_receipt(&mut app, &id);
        assert_eq!(receipt["status"], "completed");
        assert_eq!(
            receipt["snapshot"]["spectrometer_settings"]["exposure_ms"],
            25.0
        );
        assert_eq!(
            receipt["snapshot"]["spectrometer_settings"]["frames_combined"],
            2
        );
        assert_eq!(
            receipt["snapshot"]["spectrometer_settings"]["center_nm"],
            775.0
        );
        assert_eq!(receipt["snapshot"]["spectrometer_settings"]["grating"], "");
        assert_eq!(app.config.spectrometer.exposure_ms, 25.0);
        assert_eq!(app.config.spectrometer.frames_combined, 2);
        assert_eq!(app.config.spectrometer.center_nm, 775.0);
        assert_eq!(app.config.spectrometer.timeout_s, 60.0);
    }
    #[test]
    fn leased_api_retains_preview_and_job_retries_never_repeat_acquisition() {
        let (mut app, _ctx, dir, lease) = setup();
        let mut config = Config {
            output_dir: dir.path().join("runs").to_string_lossy().into(),
            ..Default::default()
        };
        config.background.mode = BackgroundMode::Automatic;
        config.background.backend = ShutterBackend::KohdalabShutter;
        config.background.endpoint = "missing-demo-endpoint-must-not-be-opened.json".into();
        config.background.closed_state = 7;
        config.background.open_state = 3;
        config.scan.stop_mm = 18.02;
        config.scan.repeats = 1;
        config.scan.settle_s = 0.;
        call(
            &mut app,
            json!({"action":"configure","lease":lease,"config":config}),
        )
        .unwrap();
        let id = crate::remote_gui::new_id().unwrap();
        let connect = json!({"action":"connect","lease":lease,"request_id":id});
        call(&mut app, connect.clone()).unwrap();
        assert_eq!(call(&mut app, connect).unwrap()["id"], id);
        assert_eq!(wait_receipt(&mut app, &id)["status"], "completed");
        let start_id = crate::remote_gui::new_id().unwrap();
        let start = json!({"action":"start","lease":lease,"request_id":start_id});
        call(&mut app, start.clone()).unwrap();
        call(&mut app, start.clone()).unwrap();
        assert_eq!(
            call(
                &mut app,
                json!({"action":"move","lease":lease,"position_mm":19.})
            )
            .unwrap_err()
            .code,
            "busy"
        );
        pump_until(&mut app, |a| a.awaiting == 3);
        assert_eq!(app.samples.len(), 0);
        assert_eq!(
            call(&mut app, json!({"action":"preview"})).unwrap()["kind"],
            "preview"
        );
        let confirmation = json!({"action":"select_analysis","lease":lease,"analysis":config.initial_analysis(),"request_id":crate::remote_gui::new_id().unwrap()});
        let ack = call(&mut app, confirmation.clone()).unwrap();
        assert_eq!(ack["run_job"], start_id);
        assert_eq!(call(&mut app, confirmation).unwrap(), ack);
        assert_eq!(
            call(&mut app, json!({"action":"result","id":ack["request_id"]})).unwrap(),
            ack
        );
        let finished = wait_receipt(&mut app, &start_id);
        assert_eq!(finished["status"], "completed");
        assert_eq!(finished["rows"], 2);
        assert_eq!(call(&mut app, start.clone()).unwrap(), finished);
        let mut conflict = start;
        conflict["extra"] = json!(true);
        assert_eq!(
            call(&mut app, conflict).unwrap_err().code,
            "request_id_conflict"
        );
        let (_, saved) = recording::load_run(std::path::Path::new(&app.run_dir)).unwrap();
        assert_eq!(saved.len(), 2);
        let provenance: Value = serde_json::from_slice(
            &std::fs::read(std::path::Path::new(&app.run_dir).join("automation.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(provenance["request_id"], start_id);
    }
    #[test]
    fn manual_confirmations_cannot_be_skipped_and_expired_client_is_revoked() {
        let (mut app, ctx, dir, lease) = setup();
        app.config.output_dir = dir.path().join("runs").to_string_lossy().into();
        app.config.scan.settle_s = 0.;
        let connected = call(&mut app, json!({"action":"connect","lease":lease})).unwrap();
        wait_receipt(&mut app, connected["id"].as_str().unwrap());
        let job = call(&mut app, json!({"action":"start","lease":lease})).unwrap();
        pump_until(&mut app, |a| a.awaiting == 1);
        assert_eq!(
            call(
                &mut app,
                json!({"action":"confirm_background","lease":lease,"confirmed":false})
            )
            .unwrap_err()
            .code,
            "validation"
        );
        assert_eq!(app.awaiting, 1);
        assert_eq!(
            call(
                &mut app,
                json!({"action":"confirm_signal","lease":lease,"confirmed":true})
            )
            .unwrap_err()
            .code,
            "invalid_state"
        );
        let confirmation = json!({"action":"confirm_background","lease":lease,"confirmed":true,"request_id":crate::remote_gui::new_id().unwrap()});
        let ack = call(&mut app, confirmation.clone()).unwrap();
        pump_until(&mut app, |a| a.awaiting == 2);
        assert_eq!(call(&mut app, confirmation).unwrap(), ack);
        assert_eq!(app.awaiting, 2);
        let owner = app
            .api
            .as_ref()
            .unwrap()
            .session
            .lease
            .as_ref()
            .unwrap()
            .client
            .clone();
        app.api
            .as_mut()
            .unwrap()
            .session
            .lease
            .as_mut()
            .unwrap()
            .touched = Instant::now() - Duration::from_secs(11);
        let _ = ctx.run_ui(Default::default(), |ui| app.poll_api(ui.ctx()));
        let receipt = wait_receipt(&mut app, job["id"].as_str().unwrap());
        assert_eq!(receipt["status"], "stopped");
        assert!(app.samples.is_empty());
        assert_eq!(
            call(&mut app, json!({"action":"acquire","client_id":owner}))
                .unwrap_err()
                .code,
            "ownership_lost"
        );
    }
}
