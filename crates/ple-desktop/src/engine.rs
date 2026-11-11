//! Single run coordinator; independent device actors keep monitoring live.
use crate::backend::{PowerClient, RpcClient, Spectrometer, StageClient};
use ple_core::{
    Acquisition, BackgroundMode, Config, Sample, Spectrum, acquisition_name,
    recording::{Recording, unix_s},
    subtract_background,
};
use serde_json::json;
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};
#[derive(Clone, Copy, Debug)]
pub enum CleanupCause {
    SpectrometerStop,
    Shutter,
    Stage,
}

#[derive(Clone, Copy, Debug)]
pub enum RunState {
    Completed,
    Stopped,
    InterruptedUnconfirmed,
    Failed,
}

impl RunState {
    pub fn label(self) -> &'static str {
        match self {
            Self::Completed => "Completed",
            Self::Stopped => "Stopped",
            Self::InterruptedUnconfirmed => "Interrupted — device state unconfirmed",
            Self::Failed => "Failed",
        }
    }

    fn recorded_status(self) -> &'static str {
        match self {
            Self::Completed => "completed",
            Self::Stopped => "stopped",
            Self::InterruptedUnconfirmed => "interrupted_unconfirmed",
            Self::Failed => "failed",
        }
    }
}

#[derive(Clone, Debug)]
pub enum RunError {
    Stopped(&'static str),
    Failed(String),
}

impl From<String> for RunError {
    fn from(error: String) -> Self {
        Self::Failed(error)
    }
}

impl From<&str> for RunError {
    fn from(error: &str) -> Self {
        Self::Failed(error.into())
    }
}

impl std::fmt::Display for RunError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Stopped(message) => f.write_str(message),
            Self::Failed(message) => f.write_str(message),
        }
    }
}

#[derive(Clone, Debug)]
pub enum Event {
    Log(String),
    Device {
        device: &'static str,
        connected: bool,
    },
    // Unlike Device (which also reports retained ownership on failure), this
    // event confirms that the requested connect/disconnect operation succeeded.
    DeviceConfirmed {
        device: &'static str,
        connected: bool,
    },
    Telemetry {
        device: &'static str,
        value: f64,
        timestamp: f64,
    },
    State(String),
    ManualMoveFinished(Result<f64, RunError>),
    AwaitBackground,
    AwaitSignal,
    Preview(Arc<Acquisition>),
    AnalysisRejected(String),
    AnalysisAccepted(Box<Config>),
    SpectrometerSettings(serde_json::Value),
    PowerWavelength(f64),
    PowerHardware(serde_json::Value),
    ShutterState(serde_json::Value),
    SpectrometerConfigured(Result<serde_json::Value, String>),
    Cleanup {
        cause: CleanupCause,
        result: Result<(), String>,
    },
    Background(Spectrum),
    Sample(Arc<Sample>),
    RunCreated(String),
    Finished {
        state: RunState,
        error: Option<String>,
    },
}
pub enum Command {
    Connect(&'static str, Config),
    Disconnect(&'static str),
    Start(Config, Option<String>),
    ManualMove(f64),
    Continue,
    SelectAnalysis(Box<Config>),
    ConfigureSpectrometer(Box<Config>),
    Shutdown,
}
pub struct Controller {
    pub commands: mpsc::Sender<Command>,
    pub events: mpsc::Receiver<Event>,
    cancel: Arc<AtomicBool>,
    spec_cancel: Arc<Mutex<Option<RpcClient>>>,
    shutter: Arc<Mutex<Option<Arc<crate::shutter_client::ExternalShutter>>>>,
    shutdown: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
    stage: StageClient,
}
impl Controller {
    pub fn spawn() -> Self {
        let (tx, rx) = mpsc::channel();
        let (events_tx, events) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let spec_cancel = Arc::new(Mutex::new(None::<RpcClient>));
        let shutter = Arc::new(Mutex::new(
            None::<Arc<crate::shutter_client::ExternalShutter>>,
        ));
        let shutter_handle = shutter.clone();
        let stop = cancel.clone();
        let spec_handle = spec_cancel.clone();
        let shutdown = Arc::new(AtomicBool::new(false));
        let shutdown_worker = shutdown.clone();
        let stage = StageClient::spawn(events_tx.clone());
        let coordinator_stage = stage.clone();
        let worker = thread::spawn(move || {
            let stage = coordinator_stage;
            let power = PowerClient::spawn(events_tx.clone());
            let mut spec: Option<Spectrometer> = None;
            let mut connected_mode = None;
            while !shutdown_worker.load(Ordering::Acquire) {
                let Ok(command) = rx.recv() else {
                    break;
                };
                match command {
                    Command::Connect(device, c) => {
                        let _ = events_tx.send(Event::State(format!("Connecting {device}…")));
                        if spec.is_none() && !stage.is_connected() && !power.is_connected() {
                            connected_mode = None;
                        }
                        if connected_mode.is_some_and(|m| m != c.mode) {
                            let _ = events_tx.send(Event::Log(
                                "Disconnect all devices before changing Demo / Hardware mode"
                                    .into(),
                            ));
                            let _ = events_tx.send(Event::State("Idle".into()));
                            continue;
                        }
                        let result = match device {
                            "spectrometer" => {
                                let result = (|| {
                                    let previous_shutter = shutter_handle.lock().unwrap().clone();
                                    if spec.is_none()
                                        && let Some(shutter) = previous_shutter
                                    {
                                        shutter
                                            .shutdown()
                                            .map_err(|e| (CleanupCause::Shutter, e))?;
                                        *shutter_handle.lock().unwrap() = None;
                                        let _ = events_tx.send(Event::Cleanup {
                                            cause: CleanupCause::Shutter,
                                            result: Ok(()),
                                        });
                                    }
                                    let previous = spec_handle.lock().unwrap().clone();
                                    if let Some(previous) = &previous {
                                        previous
                                            .wait_for_stop(true)
                                            .map_err(|e| (CleanupCause::SpectrometerStop, e))?;
                                    }
                                    if let Some(current) = spec.as_mut() {
                                        current.disconnect(&events_tx)?;
                                    } else if let Some(previous) = previous {
                                        let result = previous
                                            .request("shutdown", json!({}), Duration::from_secs(25))
                                            .map_err(|e| (CleanupCause::SpectrometerStop, e))?;
                                        if result["shutdown"].as_bool() != Some(true) {
                                            return Err((
                                                CleanupCause::SpectrometerStop,
                                                "Previous LightField bridge shutdown was not confirmed"
                                                    .into(),
                                            ));
                                        }
                                    }
                                    spec = None;
                                    *spec_handle.lock().unwrap() = None;
                                    Spectrometer::connect(
                                        &c,
                                        events_tx.clone(),
                                        &spec_handle,
                                        shutter_handle.clone(),
                                        stop.clone(),
                                    )
                                    .map(|s| {
                                        *spec_handle.lock().unwrap() = s.cancel_handle();
                                        let _ = events_tx
                                            .send(Event::SpectrometerSettings(s.settings()));
                                        spec = Some(s);
                                    })
                                    .map_err(|e| (CleanupCause::SpectrometerStop, e))
                                })();
                                result.map_err(|(cause, error)| {
                                    if matches!(cause, CleanupCause::Shutter)
                                        || spec_handle.lock().unwrap().is_some()
                                    {
                                        let _ = events_tx.send(Event::Cleanup {
                                            cause,
                                            result: Err(error.clone()),
                                        });
                                    }
                                    error
                                })
                            }
                            "stage" => stage.connect(c.clone()),
                            "power" => power.connect(c.clone()),
                            _ => Err("Unknown device".into()),
                        };
                        if result.is_ok() {
                            connected_mode = Some(c.mode);
                            let _ = events_tx.send(Event::DeviceConfirmed {
                                device,
                                connected: true,
                            });
                        }
                        if device == "spectrometer" && result.is_ok() {
                            let _ = events_tx.send(Event::Cleanup {
                                cause: CleanupCause::SpectrometerStop,
                                result: Ok(()),
                            });
                        }
                        let _ = events_tx.send(Event::Device {
                            device,
                            connected: result.is_ok(),
                        });
                        let _ = events_tx.send(Event::Log(match result {
                            Ok(()) => format!("{device} connected ({:?})", c.mode),
                            Err(e) => format!("{device}: {e}"),
                        }));
                        let _ = events_tx.send(Event::State("Idle".into()));
                    }
                    Command::Disconnect(device) => {
                        let mut disconnect_failed = false;
                        match device {
                            "spectrometer" => {
                                let orphan = spec_handle.lock().unwrap().clone();
                                let result = if let Some(current) = spec.as_mut() {
                                    current.disconnect(&events_tx)
                                } else if let Some(previous) = orphan {
                                    previous
                                        .request("shutdown", json!({}), Duration::from_secs(25))
                                        .and_then(|v| {
                                            if v["shutdown"].as_bool() == Some(true) {
                                                Ok(())
                                            } else {
                                                Err("Bridge shutdown was not confirmed".into())
                                            }
                                        })
                                        .map_err(|e| (CleanupCause::SpectrometerStop, e))
                                } else {
                                    Ok(())
                                };
                                if let Err((cause, e)) = result {
                                    disconnect_failed = true;
                                    let _ = events_tx.send(Event::Cleanup {
                                        cause,
                                        result: Err(e.clone()),
                                    });
                                    let _ = events_tx.send(Event::Log(format!(
                                        "Spectrometer disconnect not confirmed: {e}"
                                    )));
                                } else {
                                    spec = None;
                                    *spec_handle.lock().unwrap() = None;
                                    let _ = events_tx.send(Event::Cleanup {
                                        cause: CleanupCause::SpectrometerStop,
                                        result: Ok(()),
                                    });
                                }
                            }
                            "stage" => {
                                stage.disconnect();
                            }
                            "power" => {
                                power.disconnect();
                            }
                            _ => {}
                        }
                        if spec.is_none() && !stage.is_connected() && !power.is_connected() {
                            connected_mode = None;
                        }
                        if !disconnect_failed {
                            let _ = events_tx.send(Event::DeviceConfirmed {
                                device,
                                connected: false,
                            });
                        }
                        let _ = events_tx.send(Event::Device {
                            device,
                            connected: disconnect_failed,
                        });
                    }
                    Command::ManualMove(target) => {
                        let _ = events_tx
                            .send(Event::State(format!("Manual CONEX move to {target:.6} mm")));
                        let result = check_stop(&stop)
                            .and_then(|_| stage.move_to(target, &stop))
                            .and_then(|p| {
                                check_stop(&stop)?;
                                Ok(p)
                            });
                        if let Ok(value) = &result {
                            let _ = events_tx.send(Event::Telemetry {
                                device: "stage",
                                value: *value,
                                timestamp: unix_s(),
                            });
                        }
                        let _ = events_tx.send(Event::ManualMoveFinished(result));
                    }
                    Command::Start(c, external_id) => {
                        let ready = spec.is_some()
                            && stage.is_connected()
                            && power.is_connected()
                            && connected_mode == Some(c.mode);
                        if !ready {
                            let _=events_tx.send(Event::Finished{state:RunState::Failed,error:Some("Connect spectrometer, CONEX CC and power meter in the same mode before starting".into())});
                            continue;
                        }
                        let plan = match c.plan() {
                            Ok(p) => p,
                            Err(e) => {
                                let _ = events_tx.send(Event::Finished {
                                    state: RunState::Failed,
                                    error: Some(e),
                                });
                                continue;
                            }
                        };
                        let mut recording = match Recording::create(&c, &plan) {
                            Ok(r) => r,
                            Err(e) => {
                                let _ = events_tx.send(Event::Finished {
                                    state: RunState::Failed,
                                    error: Some(e),
                                });
                                continue;
                            }
                        };
                        if let Some(id) = external_id {
                            let saved = ple_core::recording::atomic_json(
                                &recording.directory.join("automation.json"),
                                &json!({"api_version":1,"request_id":id,"run_id":recording.run_id}),
                            )
                            .and_then(|_| {
                                recording.event("external_start", json!({"request_id":id}))
                            });
                            if let Err(e) = saved {
                                let _ = recording.finish("failed", Some(&e));
                                let _ = events_tx.send(Event::Finished {
                                    state: RunState::Failed,
                                    error: Some(e),
                                });
                                continue;
                            }
                        }
                        let _ = events_tx.send(Event::RunCreated(
                            recording.directory.to_string_lossy().into(),
                        ));
                        let result = execute(
                            &c,
                            &plan,
                            spec.as_mut().unwrap(),
                            &stage,
                            &power,
                            &stop,
                            &rx,
                            &events_tx,
                            &mut recording,
                        );
                        let state = match &result {
                            Ok(()) => RunState::Completed,
                            Err(RunError::Stopped(_)) => RunState::Stopped,
                            Err(_) if stop.load(Ordering::Acquire) => {
                                if c.mode == ple_core::Mode::Hardware {
                                    RunState::InterruptedUnconfirmed
                                } else {
                                    RunState::Stopped
                                }
                            }
                            Err(_) => RunState::Failed,
                        };
                        let error = result.err().map(|e| e.to_string());
                        let (final_state, error) =
                            match recording.finish(state.recorded_status(), error.as_deref()) {
                                Ok(()) => (state, error),
                                Err(e) => (
                                    RunState::Failed,
                                    Some(format!(
                                        "{}; final status could not be saved: {e}",
                                        error.unwrap_or_default()
                                    )),
                                ),
                            };
                        let _ = events_tx.send(Event::Finished {
                            state: final_state,
                            error,
                        });
                    }
                    Command::ConfigureSpectrometer(c) => {
                        let result = spec
                            .as_mut()
                            .ok_or("Spectrometer is not connected".into())
                            .and_then(|s| {
                                c.plan()?;
                                s.configure(&c)?;
                                Ok(s.settings())
                            });
                        let _ = events_tx.send(Event::SpectrometerConfigured(result));
                    }
                    Command::Continue | Command::SelectAnalysis(_) => {}
                    Command::Shutdown => break,
                }
            }
            let last_handle = spec_handle.lock().unwrap().clone();
            if let Some(rpc) = &last_handle {
                rpc.stop();
            }
            let cleanup = if let Some(current) = spec.as_mut() {
                current.disconnect(&events_tx)
            } else if let Some(rpc) = &last_handle {
                rpc.request("shutdown", json!({}), Duration::from_secs(25))
                    .and_then(|v| {
                        if v["shutdown"].as_bool() == Some(true) {
                            Ok(())
                        } else {
                            Err("Bridge shutdown was not confirmed".into())
                        }
                    })
                    .map_err(|e| (CleanupCause::SpectrometerStop, e))
            } else {
                Ok(())
            };
            if let Err((cause, error)) = cleanup {
                let _ = events_tx.send(Event::Cleanup {
                    cause,
                    result: Err(error.clone()),
                });
                let _ = events_tx.send(Event::Log(format!(
                    "Final LightField cleanup unconfirmed: {error}"
                )));
            }
            *spec_handle.lock().unwrap() = None;
            drop(spec);
            stage.shutdown();
            power.shutdown();
        });
        Self {
            commands: tx,
            events,
            cancel,
            spec_cancel,
            shutter,
            shutdown,
            worker: Some(worker),
            stage,
        }
    }
    pub fn begin(&self, config: Config) -> Result<(), String> {
        self.require_recovered()?;
        self.cancel.store(false, Ordering::Release);
        self.commands
            .send(Command::Start(config, None))
            .map_err(|e| e.to_string())
    }
    pub fn begin_external(&self, config: Config, id: String) -> Result<(), String> {
        self.require_recovered()?;
        self.cancel.store(false, Ordering::Release);
        self.commands
            .send(Command::Start(config, Some(id)))
            .map_err(|e| e.to_string())
    }
    pub fn manual_move(&self, target: f64) -> Result<(), String> {
        self.require_recovered()?;
        if !target.is_finite() || !(0. ..=25.).contains(&target) {
            return Err("CONEX target must be within 0–25 mm".into());
        }
        self.cancel.store(false, Ordering::Release);
        self.commands
            .send(Command::ManualMove(target))
            .map_err(|e| e.to_string())
    }
    pub fn stop_status(&self) -> (bool, Option<String>) {
        let (stage_pending, stage_error) = self.stage.stop_status();
        let (spec_pending, spec_error) = self
            .spec_cancel
            .lock()
            .unwrap()
            .as_ref()
            .map_or((false, None), RpcClient::stop_status);
        let (shutter_pending, shutter_error) = self
            .shutter
            .lock()
            .unwrap()
            .as_ref()
            .map_or((false, None), |s| s.stop_status());
        (
            stage_pending || spec_pending || shutter_pending,
            stage_error.or(spec_error).or(shutter_error),
        )
    }
    pub fn shutter_status(&self) -> Option<serde_json::Value> {
        let shutter = self.shutter.lock().unwrap().clone();
        shutter.map(|s| s.snapshot())
    }
    pub fn has_spectrometer_owner(&self) -> bool {
        self.spec_cancel.lock().unwrap().is_some()
    }
    fn require_recovered(&self) -> Result<(), String> {
        let (pending, error) = self.stop_status();
        if pending {
            return Err("Stop is pending; wait for device readback".into());
        }
        if let Some(error) = error {
            return Err(format!("Device recovery is unconfirmed: {error}"));
        }
        Ok(())
    }
    pub fn stop(&self) {
        self.cancel.store(true, Ordering::Release);
        self.stage.stop();
        if let Some(shutter) = self.shutter.lock().unwrap().as_ref() {
            shutter.request_close();
        }
        if let Some(rpc) = self.spec_cancel.lock().unwrap().as_ref() {
            rpc.stop();
        }
    }
}
impl Drop for Controller {
    fn drop(&mut self) {
        self.stop();
        self.shutdown.store(true, Ordering::Release);
        let _ = self.commands.send(Command::Shutdown);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
fn check_stop(stop: &AtomicBool) -> Result<(), RunError> {
    if stop.load(Ordering::Acquire) {
        Err(RunError::Stopped(
            "Stop requested; no further scan points will start",
        ))
    } else {
        Ok(())
    }
}
fn wait_confirmation(rx: &mpsc::Receiver<Command>, stop: &AtomicBool) -> Result<(), RunError> {
    loop {
        check_stop(stop)?;
        match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(Command::Continue) => return Ok(()),
            Ok(Command::Shutdown) => {
                stop.store(true, Ordering::Release);
                return Err("Application is closing".into());
            }
            Ok(_) => return Err("Unexpected command while waiting for operator".into()),
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(_) => return Err("Command channel closed".into()),
        }
    }
}
fn wait_settle(seconds: f64, stop: &AtomicBool) -> Result<(), RunError> {
    let deadline = Instant::now() + Duration::from_secs_f64(seconds);
    while Instant::now() < deadline {
        check_stop(stop)?;
        thread::sleep(
            Duration::from_millis(25).min(deadline.saturating_duration_since(Instant::now())),
        );
    }
    check_stop(stop)
}
#[allow(clippy::too_many_arguments)]
fn execute(
    c: &Config,
    plan: &ple_core::Plan,
    spec: &mut Spectrometer,
    stage: &StageClient,
    power: &PowerClient,
    stop: &AtomicBool,
    commands: &mpsc::Receiver<Command>,
    events: &mpsc::Sender<Event>,
    recording: &mut Recording,
) -> Result<(), RunError> {
    spec.configure(c)?;
    let _ = events.send(Event::SpectrometerSettings(spec.settings()));
    recording.hardware(json!({"spectrometer":spec.settings(),"power":power.hardware()?,"mode":c.mode,"source":if c.mode == ple_core::Mode::Hardware {"connected_device_readback"} else {"simulated"}}))?;
    let result = spec
        .prepare_shutter(c)
        .map_err(RunError::from)
        .and_then(|()| {
            let snapshot = spec.shutter_snapshot();
            let _ = events.send(Event::ShutterState(snapshot.clone()));
            ple_core::recording::atomic_json(
                &recording.directory.join("metadata/shutter.json"),
                &snapshot,
            )?;
            recording.event("shutter_connected", snapshot)?;
            execute_inner(
                c, plan, spec, stage, power, stop, commands, events, recording,
            )
        });
    if c.background.mode == BackgroundMode::Automatic && spec.shutter_attached() {
        let closed = spec.set_shutter(true);
        let closed_confirmed = closed.is_ok();
        let closed = closed.and_then(|()| spec.finish_shutter());
        let snapshot = spec.shutter_snapshot();
        let _ = events.send(Event::ShutterState(snapshot.clone()));
        let _ = events.send(Event::Cleanup {
            cause: CleanupCause::Shutter,
            result: closed.clone(),
        });
        recording.event(
            "shutter_end_close",
            json!({"controller_state_confirmed":closed_confirmed,"cleanup_confirmed":closed.is_ok(),"physical_optical_sensor_confirmed":false,"state":snapshot,"error":closed.as_ref().err()}),
        )?;
        if let Err(e) = closed {
            return Err(format!(
                "shutter cleanup could not be confirmed: {e}; prior: {}",
                result.err().map(|e| e.to_string()).unwrap_or_default()
            )
            .into());
        }
    }
    result
}
#[allow(clippy::too_many_arguments)]
fn execute_inner(
    c: &Config,
    plan: &ple_core::Plan,
    spec: &mut Spectrometer,
    stage: &StageClient,
    power: &PowerClient,
    stop: &AtomicBool,
    commands: &mpsc::Receiver<Command>,
    events: &mpsc::Sender<Event>,
    recording: &mut Recording,
) -> Result<(), RunError> {
    if c.background.mode == BackgroundMode::Manual {
        let _ = events.send(Event::State(
            "Awaiting BG: block the excitation beam".into(),
        ));
        let _ = events.send(Event::AwaitBackground);
        wait_confirmation(commands, stop)?;
    } else {
        check_stop(stop)?;
        let _ = events.send(Event::State("Closing BG shutter".into()));
        spec.set_shutter(true)?;
        let snapshot = spec.shutter_snapshot();
        let _ = events.send(Event::ShutterState(snapshot.clone()));
        recording.event("shutter_closed", json!({"controller_state_confirmed":true,"physical_optical_sensor_confirmed":false,"state":snapshot}))?;
    }
    check_stop(stop)?;
    let _ = events.send(Event::State("Acquiring background".into()));
    recording.event("background_started", json!({"mode":c.background.mode}))?;
    let background = spec.acquire(&recording.base_path("background"), true, c.scan.start_mm)?;
    check_stop(stop)?;
    if background
        .counts
        .len()
        .saturating_mul(plan.acquisition_count)
        > 8_000_000
    {
        return Err("Run exceeds live-view memory budget; reduce points/repeats or ROI".into());
    }
    recording.background(&background)?;
    let _ = events.send(Event::Background(background.clone()));
    if c.background.mode == BackgroundMode::Manual {
        let _ = events.send(Event::State(
            "Awaiting signal: unblock excitation beam".into(),
        ));
        let _ = events.send(Event::AwaitSignal);
        wait_confirmation(commands, stop)?;
    } else {
        check_stop(stop)?;
        let _ = events.send(Event::State("Opening signal shutter".into()));
        spec.set_shutter(false)?;
        let snapshot = spec.shutter_snapshot();
        let _ = events.send(Event::ShutterState(snapshot.clone()));
        recording.event("shutter_opened",json!({"controller_state_confirmed":true,"physical_optical_sensor_confirmed":false,"state":snapshot}))?;
    }
    recording.event("signal_path_confirmed", json!({"mode":c.background.mode,"confirmation_source":if c.background.mode==BackgroundMode::Manual {"operator"} else {"controller_state"},"physical_optical_sensor_confirmed":false}))?;
    // The first acquisition is retained, reviewed, then becomes point 0 / repeat 0.
    let actual = move_point(c, 0, plan.positions_mm[0], stage, stop, events, recording)?;
    let (first, acquisition_extrapolated) = acquire_point(
        c,
        plan,
        0,
        0,
        plan.positions_mm[0],
        actual,
        spec,
        power,
        stop,
        events,
        recording,
        &background,
    )?;
    recording.preview(&first)?;
    let _ = events.send(Event::Preview(Arc::new(first.clone())));
    let _ = events.send(Event::State(
        "Review first PL: choose peak and integration interval".into(),
    ));
    let (accepted, first_sample) = loop {
        check_stop(stop)?;
        match commands.recv_timeout(Duration::from_millis(100)) {
            Ok(Command::SelectAnalysis(selected)) => {
                let mut candidate = c.clone();
                candidate.analysis = Some(selected.initial_analysis());
                candidate.excitation_method = selected.excitation_method;
                candidate.excitation_min_nm = selected.excitation_min_nm;
                candidate.excitation_max_nm = selected.excitation_max_nm;
                candidate.excitation_min_snr = selected.excitation_min_snr;
                candidate.excitation_peaks = selected.excitation_peaks.clone();
                let checked = candidate.plan().and_then(|p| {
                    first
                        .clone()
                        .analyze(&candidate, &p, &candidate.initial_analysis())
                });
                match checked {
                    Ok(sample) => break (candidate, sample),
                    Err(e) => {
                        let _ = events.send(Event::AnalysisRejected(e));
                    }
                }
            }
            Ok(Command::Shutdown) => {
                stop.store(true, Ordering::Release);
                return Err("Application is closing".into());
            }
            Ok(_) => {
                let _ = events.send(Event::AnalysisRejected(
                    "Confirm the analysis settings or Stop".into(),
                ));
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(_) => return Err("Command channel closed".into()),
        }
    };
    let accepted_plan = accepted.plan()?;
    recording.analysis(&accepted, &accepted_plan)?;
    let _ = events.send(Event::AnalysisAccepted(Box::new(accepted.clone())));
    if acquisition_extrapolated
        || (accepted.excitation_method == ple_core::ExcitationMethod::Calibration
            && outside_excitation_calibration(&accepted_plan, actual))
    {
        recording.excitation_extrapolated(first.point_index, first.repeat_index, actual)?;
    }
    recording.sample(&first_sample, &background)?;
    let _ = events.send(Event::Sample(Arc::new(first_sample)));
    check_stop(stop)?;
    for (index, &target) in plan.positions_mm.iter().enumerate() {
        let actual = if index == 0 {
            actual
        } else {
            move_point(&accepted, index, target, stage, stop, events, recording)?
        };
        for repeat in 0..accepted.scan.repeats {
            if index == 0 && repeat == 0 {
                continue;
            }
            let (a, acquisition_extrapolated) = acquire_point(
                &accepted,
                &accepted_plan,
                index,
                repeat,
                target,
                actual,
                spec,
                power,
                stop,
                events,
                recording,
                &background,
            )?;
            let sample = a.analyze(&accepted, &accepted_plan, &accepted.initial_analysis())?;
            if acquisition_extrapolated
                || (accepted.excitation_method == ple_core::ExcitationMethod::Calibration
                    && outside_excitation_calibration(&accepted_plan, actual))
            {
                recording.excitation_extrapolated(index, repeat, actual)?;
            }
            recording.sample(&sample, &background)?;
            let _ = events.send(Event::Sample(Arc::new(sample)));
            check_stop(stop)?;
        }
    }
    Ok(())
}
fn move_point(
    c: &Config,
    index: usize,
    target: f64,
    stage: &StageClient,
    stop: &AtomicBool,
    events: &mpsc::Sender<Event>,
    recording: &mut Recording,
) -> Result<f64, RunError> {
    check_stop(stop)?;
    let _ = events.send(Event::State(format!("Moving CONEX to {target:.6} mm")));
    recording.event(
        "move_requested",
        json!({"point_index":index,"target_mm":target}),
    )?;
    let actual = stage.move_to(target, stop)?;
    check_stop(stop)?;
    recording.event("move_confirmed", json!({"actual_mm":actual}))?;
    let _ = events.send(Event::Telemetry {
        device: "stage",
        value: actual,
        timestamp: unix_s(),
    });
    let _ = events.send(Event::State("Settling".into()));
    wait_settle(c.scan.settle_s, stop)?;
    Ok(actual)
}
fn outside_excitation_calibration(plan: &ple_core::Plan, actual: f64) -> bool {
    let anchors = &plan.excitation_anchors;
    matches!((anchors.first(), anchors.last()), (Some(lo), Some(hi))
        if actual < lo.position_mm || actual > hi.position_mm)
}

#[allow(clippy::too_many_arguments)]
fn acquire_point(
    c: &Config,
    plan: &ple_core::Plan,
    index: usize,
    repeat: u32,
    target: f64,
    actual: f64,
    spec: &mut Spectrometer,
    power: &PowerClient,
    stop: &AtomicBool,
    events: &mpsc::Sender<Event>,
    recording: &mut Recording,
    bg: &Spectrum,
) -> Result<(Acquisition, bool), RunError> {
    check_stop(stop)?;
    let (power_w, power_time, sensor_wavelength, power_hardware, power_observation) =
        if c.power.wavelength_mode == ple_core::PowerWavelengthMode::PositionCalibration {
            let predicted = ple_core::interpolate_excitation_at_readback(
                &plan.excitation_anchors,
                actual,
                c.stage.tolerance_mm,
            )?;
            power.read_at(predicted)?
        } else {
            power.read_with_wavelength()?
        };
    let sample_power_w = power_w * c.power.correction_factor;
    if !sample_power_w.is_finite() || sample_power_w <= 0. {
        return Err("Power must be finite and positive for the file name and normalization".into());
    }
    check_stop(stop)?;
    let file_base = acquisition_name(actual, sample_power_w, index, repeat)?;
    let started = unix_s();
    let _ = events.send(Event::State(format!(
        "Acquiring point {}, repeat {}",
        index + 1,
        repeat + 1
    )));
    recording.event("acquisition_started",json!({"file_base":file_base,"point_index":index,"repeat_index":repeat,"power_w":power_w,"power_timestamp_unix_s":power_time,"power_hardware":power_hardware,"power_observation":power_observation}))?;
    let spectrum = spec.acquire(&recording.base_path(&file_base), false, actual)?;
    let mut acquired = Acquisition {
        file_base,
        point_index: index,
        repeat_index: repeat,
        target_mm: target,
        actual_mm: actual,
        power_w,
        sample_power_w,
        power_timestamp_unix_s: power_time,
        power_sensor_wavelength_nm: Some(sensor_wavelength),
        power_observation,
        acquisition_started_unix_s: started,
        timestamp_unix_s: unix_s(),
        spectrum,
        corrected_counts: Vec::new(),
    };
    recording.acquired(&acquired)?;
    let excitation_extrapolated = c.power.wavelength_mode
        == ple_core::PowerWavelengthMode::PositionCalibration
        && outside_excitation_calibration(plan, actual);
    if !matching_acquisition_settings(&acquired.spectrum.settings, &bg.settings) {
        return Err("LightField settings changed after BG; raw acquisition retained".into());
    }
    acquired.corrected_counts = subtract_background(&acquired.spectrum, bg)?;
    if acquired
        .corrected_counts
        .iter()
        .any(|v| !(v / sample_power_w).is_finite())
    {
        return Err("Power normalization overflow; raw acquisition retained".into());
    }
    Ok((acquired, excitation_extrapolated))
}

fn matching_acquisition_settings(a: &serde_json::Value, b: &serde_json::Value) -> bool {
    // A current bridge must provide a read-only setting snapshot.  Critical
    // fields may be explicitly unsupported, but an unknown SDK member or an
    // unreadable value is never evidence that BG and signal were the same.
    const REQUIRED_BACKGROUND_FIELDS: &[&str] = &[
        "selected_regions",
        "adc_analog_gain",
        "adc_speed",
        "adc_quality",
        "adc_bit_depth",
        "readout_control_mode",
        "sensor_temperature_setpoint",
        "online_cosmic_ray_enabled",
        "online_cosmic_ray_method",
        "online_cosmic_ray_kernel_size",
        "online_flatfield_enabled",
        "online_flatfield_reference_file",
        "online_blemish_enabled",
        "online_blemish_definition_file",
        "online_orientation_enabled",
        "online_orientation_flip_horizontal",
        "online_orientation_flip_vertical",
        "online_orientation_rotate_clockwise",
    ];
    let valid_background_snapshot = |value: &serde_json::Value| {
        let Some(settings) = value.get("background_settings") else {
            return value["demo"].as_bool() == Some(true);
        };
        settings.as_object().is_some_and(|fields| {
            REQUIRED_BACKGROUND_FIELDS
                .iter()
                .all(|key| fields.contains_key(*key))
                && fields.values().all(|field| {
                    matches!(
                        field.get("status").and_then(|v| v.as_str()),
                        Some("supported" | "unsupported")
                    )
                })
                && (!fields["online_flatfield_enabled"]
                    .get("value")
                    .is_some_and(|v| v == true)
                    || matches!(
                        fields
                            .get("online_flatfield_reference_identity")
                            .and_then(|v| v.get("status").and_then(|s| s.as_str())),
                        Some("supported")
                    ))
                && (!fields["online_blemish_enabled"]
                    .get("value")
                    .is_some_and(|v| v == true)
                    || matches!(
                        fields
                            .get("online_blemish_definition_identity")
                            .and_then(|v| v.get("status").and_then(|s| s.as_str())),
                        Some("supported")
                    ))
        })
    };
    if !valid_background_snapshot(a) || !valid_background_snapshot(b) {
        return false;
    }
    if let (Some(old), Some(new)) = (
        a.pointer("/hardware/lightfield/devices")
            .and_then(|v| v.as_array()),
        b.pointer("/hardware/lightfield/devices")
            .and_then(|v| v.as_array()),
    ) {
        // The SDK may reorder devices or add previously unavailable fields.
        // Compare known identities only when the device type is unambiguous.
        for original in old {
            let Some(kind) = original["type"].as_str().filter(|s| !s.is_empty()) else {
                continue;
            };
            if old
                .iter()
                .filter(|v| v["type"].as_str() == Some(kind))
                .count()
                != 1
            {
                continue;
            }
            let candidates: Vec<_> = new
                .iter()
                .filter(|v| v["type"].as_str() == Some(kind))
                .collect();
            if candidates.len() != 1 {
                continue;
            }
            for field in ["model", "serial_number"] {
                if let (Some(left), Some(right)) = (
                    original[field].as_str().filter(|s| !s.is_empty()),
                    candidates[0][field].as_str().filter(|s| !s.is_empty()),
                ) && left != right
                {
                    return false;
                }
            }
        }
    }
    let strip = |v: &serde_json::Value| {
        let mut v = v.clone();
        if let Some(object) = v.as_object_mut() {
            object.remove("hardware");
            // Sensor readings and stabilization status are observations, not
            // stable BG identity.  The temperature setpoint is in
            // background_settings and therefore remains compared.
            object.remove("temperature");
        }
        v
    };
    strip(a) == strip(b)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn optional_inventory_does_not_hide_changed_settings_or_camera() {
        let base = json!({"demo":true,"exposure_ms":10.,"grating":"g","hardware":{"lightfield":{"devices":[{"type":"Camera","model":"A","serial_number":"1"}]}}});
        let mut current = base.clone();
        current["hardware"]["lightfield"]["errors"] = json!(["optional metadata unavailable"]);
        assert!(matching_acquisition_settings(&base, &current));
        current["hardware"]["lightfield"]["devices"][0]["serial_number"] = serde_json::Value::Null;
        assert!(matching_acquisition_settings(&base, &current));
        assert!(matching_acquisition_settings(&current, &base));
        current = base.clone();
        current["hardware"]["lightfield"]["devices"]
            .as_array_mut()
            .unwrap()
            .push(json!({"type":"Spectrometer","model":"HRS-300","serial_number":"S1"}));
        let mut reordered = current.clone();
        reordered["hardware"]["lightfield"]["devices"]
            .as_array_mut()
            .unwrap()
            .reverse();
        assert!(matching_acquisition_settings(&current, &reordered));
        current = base.clone();
        current["hardware"]["lightfield"]["devices"][0]["serial_number"] = json!("2");
        assert!(!matching_acquisition_settings(&base, &current));
        current = base.clone();
        current["exposure_ms"] = json!(20.);
        assert!(!matching_acquisition_settings(&base, &current));
        let mut qualified = json!({"exposure_ms":10.,"grating":"g","background_settings":{"selected_regions":{"status":"supported","value":[{"X":0,"Y":0,"Width":512,"Height":1,"XBinning":1,"YBinning":1}]},"adc_analog_gain":{"status":"supported","value":"low"},"readout_control_mode":{"status":"supported","value":"full"},"sensor_temperature_setpoint":{"status":"supported","value":-70.},"online_flatfield_enabled":{"status":"supported","value":false},"online_blemish_enabled":{"status":"supported","value":false}},"temperature":{"reading":{"status":"supported","value":-69.},"status":{"status":"supported","value":"Locked"}}});
        for name in [
            "adc_speed",
            "adc_quality",
            "adc_bit_depth",
            "online_cosmic_ray_enabled",
            "online_cosmic_ray_method",
            "online_cosmic_ray_kernel_size",
            "online_flatfield_reference_file",
            "online_blemish_definition_file",
            "online_orientation_enabled",
            "online_orientation_flip_horizontal",
            "online_orientation_flip_vertical",
            "online_orientation_rotate_clockwise",
        ] {
            qualified["background_settings"]
                .as_object_mut()
                .unwrap()
                .insert(name.into(), json!({"status":"supported","value":"test"}));
        }
        let mut changed = qualified.clone();
        changed["background_settings"]["selected_regions"]["value"][0]["YBinning"] = json!(2);
        assert!(!matching_acquisition_settings(&qualified, &changed));
        let mut temperature_drift = qualified.clone();
        temperature_drift["temperature"]["reading"]["value"] = json!(-60.);
        assert!(matching_acquisition_settings(
            &qualified,
            &temperature_drift
        ));
        let mut setpoint_changed = qualified.clone();
        setpoint_changed["background_settings"]["sensor_temperature_setpoint"]["value"] =
            json!(-60.);
        assert!(!matching_acquisition_settings(
            &qualified,
            &setpoint_changed
        ));
        let mut unknown = qualified.clone();
        unknown["background_settings"]["adc_analog_gain"] = json!({"status":"unknown_sdk_member"});
        assert!(!matching_acquisition_settings(&qualified, &unknown));
        let mut unsupported = qualified.clone();
        unsupported["background_settings"]["adc_analog_gain"] = json!({"status":"unsupported"});
        assert!(!matching_acquisition_settings(&qualified, &unsupported));
        let mut both_unsupported = unsupported.clone();
        both_unsupported["background_settings"]["selected_regions"] =
            json!({"status":"unsupported"});
        let mut matching_unsupported = both_unsupported.clone();
        matching_unsupported["background_settings"]["adc_analog_gain"] =
            json!({"status":"unsupported"});
        assert!(matching_acquisition_settings(
            &both_unsupported,
            &matching_unsupported
        ));
    }
    #[test]
    fn failed_spectrometer_connect_retains_stop_owner_until_verified_recovery() {
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("bridge.py");
        let instances = dir.path().join("instances.txt");
        let allow_idle = dir.path().join("allow-idle");
        std::fs::write(&script, format!(
            "import sys,json,pathlib\np=pathlib.Path({instances});n=int(p.read_text())+1 if p.exists() else 1;p.write_text(str(n))\nfor line in sys.stdin:\n r=json.loads(line);op=r['op'];ok=not(op=='connect_spectrometer' and n==1)\n result={{'settings':{{}},'shutdown':True,'idle_confirmed':pathlib.Path({allow_idle}).exists()}}\n print(json.dumps({{'id':r['id'],'ok':ok,'result':result,'error':'connect failed'}}),flush=True)\n if op=='shutdown':break\n",
            instances=serde_json::to_string(&instances.to_string_lossy()).unwrap(),
            allow_idle=serde_json::to_string(&allow_idle.to_string_lossy()).unwrap()
        )).unwrap();
        let c = Controller::spawn();
        let mut config = Config {
            mode: ple_core::Mode::Hardware,
            ..Default::default()
        };
        config.spectrometer.bridge_script = script.to_string_lossy().into();
        let connect = |expected| {
            c.commands
                .send(Command::Connect("spectrometer", config.clone()))
                .unwrap();
            loop {
                if let Event::Device {
                    device: "spectrometer",
                    connected,
                } = c.events.recv_timeout(Duration::from_secs(3)).unwrap()
                {
                    assert_eq!(connected, expected);
                    break;
                }
            }
        };
        connect(false);
        let deadline = Instant::now() + Duration::from_secs(3);
        while c.stop_status().0 {
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(5));
        }
        assert!(
            c.stop_status().1.is_some(),
            "failed connect must retain its Stop error"
        );
        connect(false);
        assert_eq!(
            std::fs::read_to_string(&instances).unwrap(),
            "1",
            "must not launch another owner while old Stop is unconfirmed"
        );
        std::fs::write(&allow_idle, "ready").unwrap();
        c.stop();
        let deadline = Instant::now() + Duration::from_secs(3);
        while c.stop_status().0 {
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(c.stop_status(), (false, None));
        connect(true);
        assert_eq!(std::fs::read_to_string(&instances).unwrap(), "2");
    }
    fn setup() -> (Controller, Config, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let mut config = Config {
            output_dir: dir.path().to_string_lossy().into(),
            ..Default::default()
        };
        config.scan.stop_mm = 18.02;
        config.scan.settle_s = 0.;
        config.scan.repeats = 1;
        let controller = Controller::spawn();
        for d in ["spectrometer", "stage", "power"] {
            controller
                .commands
                .send(Command::Connect(d, config.clone()))
                .unwrap();
        }
        let mut connected = 0;
        while connected < 3 {
            if let Event::Device { connected: ok, .. } = controller
                .events
                .recv_timeout(Duration::from_secs(3))
                .unwrap()
            {
                assert!(ok);
                connected += 1;
            }
        }
        (controller, config, dir)
    }
    #[test]
    fn automatic_bg_with_missing_shutter_gui_returns_the_original_connection_error() {
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("fake_lightfield.py");
        std::fs::write(
            &script,
            r#"
import json, sys
for line in sys.stdin:
    request = json.loads(line)
    op = request["op"]
    assert op in ("connect_spectrometer", "configure_spectrometer", "shutdown"), op
    print(json.dumps({"id": request["id"], "ok": True, "result": {"shutdown": True}}))
"#,
        )
        .unwrap();
        let missing = dir.path().join("missing-shutter.json");
        let mut config = Config {
            mode: ple_core::Mode::Hardware,
            output_dir: dir.path().to_string_lossy().into(),
            ..Default::default()
        };
        config.spectrometer.experiment = "fake".into();
        config.spectrometer.bridge_script = script.to_string_lossy().into();
        config.background.mode = BackgroundMode::Automatic;
        config.background.backend = ple_core::ShutterBackend::KohdalabShutter;
        config.background.endpoint = missing.to_string_lossy().into();
        let (tx, rx) = mpsc::channel();
        let stop = Arc::new(AtomicBool::new(false));
        let handle = Mutex::new(None);
        let mut spec = Spectrometer::connect(
            &config,
            tx.clone(),
            &handle,
            Arc::new(Mutex::new(None)),
            stop.clone(),
        )
        .unwrap();
        let stage = StageClient::spawn(tx.clone());
        let power = PowerClient::spawn(tx.clone());
        power.connect(Config::default()).unwrap();
        let (_command_tx, commands) = mpsc::channel();
        let plan = config.plan().unwrap();
        let mut recording = Recording::create(&config, &plan).unwrap();
        let error = execute(
            &config,
            &plan,
            &mut spec,
            &stage,
            &power,
            &stop,
            &commands,
            &tx,
            &mut recording,
        )
        .unwrap_err();
        let error = error.to_string();
        assert_eq!(
            error,
            format!(
                "shutter GUI endpoint must be a descriptor file: {}",
                missing.display()
            )
        );
        let ctx = eframe::egui::Context::default();
        let mut app = crate::PleApp::with_context(&ctx, crate::Launch::default());
        tx.send(Event::Finished {
            state: RunState::Failed,
            error: Some(error.clone()),
        })
        .unwrap();
        app.controller.events = rx;
        app.events();
        assert_eq!(app.message.as_deref(), Some(error.as_str()));
        assert!(app.recovery_reason().is_none());
        assert!(!recording.directory.join("metadata/shutter.json").exists());
        stage.shutdown();
        power.shutdown();
    }

    #[test]
    fn run_continues_and_logs_once_for_readback_below_first_calibration_anchor() {
        use std::io::{BufRead, BufReader, Write};

        let dir = tempfile::tempdir().unwrap();
        let mut config = Config {
            output_dir: dir.path().to_string_lossy().into(),
            excitation_method: ple_core::ExcitationMethod::Calibration,
            ..Default::default()
        };
        config.background.mode = BackgroundMode::Automatic;
        config.scan.stop_mm = 18.;
        config.scan.settle_s = 0.;
        config.scan.repeats = 1;
        let actual = 18. - config.stage.tolerance_mm / 2.;
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let stage_config = Config {
            mode: ple_core::Mode::Hardware,
            stage: ple_core::StageConfig {
                host: "127.0.0.1".into(),
                port: listener.local_addr().unwrap().port(),
                ..config.stage.clone()
            },
            ..config.clone()
        };
        let server = thread::spawn(move || {
            let mut moves_remaining = 2;
            loop {
                let (mut stream, _) = listener.accept().unwrap();
                let mut line = String::new();
                BufReader::new(stream.try_clone().unwrap())
                    .read_line(&mut line)
                    .unwrap();
                match line.trim_end() {
                    "MOVE 18.00000000" => {
                        moves_remaining -= 1;
                        writeln!(stream, "MOVED {actual}").unwrap();
                    }
                    "GET" => {
                        writeln!(stream, "{actual}").unwrap();
                        if moves_remaining == 0 {
                            break;
                        }
                    }
                    other => panic!("unexpected mock stage command: {other}"),
                }
            }
        });
        let (tx, rx) = mpsc::channel();
        let stage = StageClient::spawn(tx.clone());
        stage.connect(stage_config).unwrap();
        let power = PowerClient::spawn(tx.clone());
        power.connect(config.clone()).unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let mut spec = Spectrometer::connect(
            &config,
            tx.clone(),
            &Mutex::new(None),
            Arc::new(Mutex::new(None)),
            stop.clone(),
        )
        .unwrap();
        for mode in [
            ple_core::PowerWavelengthMode::Fixed,
            ple_core::PowerWavelengthMode::PositionCalibration,
        ] {
            config.power.wavelength_mode = mode;
            let plan = config.plan().unwrap();
            let mut recording = Recording::create(&config, &plan).unwrap();
            let (command_tx, commands) = mpsc::channel();
            command_tx
                .send(Command::SelectAnalysis(Box::new(config.clone())))
                .unwrap();
            execute(
                &config,
                &plan,
                &mut spec,
                &stage,
                &power,
                &stop,
                &commands,
                &tx,
                &mut recording,
            )
            .unwrap();
            let log = std::fs::read_to_string(recording.directory.join("analysis.log")).unwrap();
            assert_eq!(
                log.lines()
                    .filter(|line| line.contains("\texcitation_extrapolated\t"))
                    .count(),
                1
            );
            let sample = rx
                .try_iter()
                .find_map(|event| match event {
                    Event::Sample(sample) => Some(sample),
                    _ => None,
                })
                .unwrap();
            assert_eq!(sample.actual_mm, actual);
            assert!((sample.excitation_nm - 699.975).abs() < 1e-9);
            if mode == ple_core::PowerWavelengthMode::PositionCalibration {
                assert_eq!(
                    sample.power_sensor_wavelength_nm,
                    Some(sample.excitation_nm)
                );
            }
        }
        stage.shutdown();
        power.shutdown();
        server.join().unwrap();
    }

    #[test]
    fn power_calibration_tracks_before_exposure_and_mismatch_retains_raw() {
        for (tolerance, success) in [(1., true), (0.01, false)] {
            let (controller, mut config, _dir) = setup();
            config.background.mode = BackgroundMode::Automatic;
            config.power.wavelength_mode = ple_core::PowerWavelengthMode::PositionCalibration;
            config.power.wavelength_tolerance_nm = tolerance;
            controller.begin(config.clone()).unwrap();
            let mut path = None;
            let mut count = 0;
            loop {
                match controller
                    .events
                    .recv_timeout(Duration::from_secs(5))
                    .unwrap()
                {
                    Event::RunCreated(p) => path = Some(p),
                    Event::Preview(a) => {
                        assert_eq!(a.power_sensor_wavelength_nm, Some(700.));
                        controller
                            .commands
                            .send(Command::SelectAnalysis(Box::new(config.clone())))
                            .unwrap();
                    }
                    Event::Sample(s) => {
                        count += 1;
                        let predicted = ple_core::interpolate_excitation_at_readback(
                            &config.plan().unwrap().excitation_anchors,
                            s.actual_mm,
                            config.stage.tolerance_mm,
                        )
                        .unwrap();
                        assert_eq!(s.power_sensor_wavelength_nm, Some(predicted));
                        assert!(s.power_timestamp_unix_s <= s.acquisition_started_unix_s);
                    }
                    Event::Finished { state, error } => {
                        assert_eq!(
                            state.label(),
                            if success { "Completed" } else { "Failed" },
                            "{error:?}"
                        );
                        if !success {
                            assert!(error.unwrap().contains("disagrees"));
                        }
                        break;
                    }
                    _ => {}
                }
            }
            let path = std::path::PathBuf::from(path.unwrap());
            assert_eq!(count, if success { 2 } else { 1 });
            assert_eq!(
                std::fs::read_dir(path.join("raw"))
                    .unwrap()
                    .filter_map(Result::ok)
                    .filter(|e| e.file_name().to_string_lossy().starts_with("Y_"))
                    .count(),
                2
            );
            assert!(path.join("metadata/acquisition_00001_000.json").exists());
        }
    }
    #[test]
    fn manual_stage_moves_without_spectrometer_or_power_and_rejects_outside_travel() {
        let controller = Controller::spawn();
        controller
            .commands
            .send(Command::Connect("stage", Config::default()))
            .unwrap();
        loop {
            if let Event::Device {
                device: "stage",
                connected,
            } = controller
                .events
                .recv_timeout(Duration::from_secs(2))
                .unwrap()
            {
                assert!(connected);
                break;
            }
        }
        for target in [0., 25.] {
            controller.manual_move(target).unwrap();
            loop {
                if let Event::ManualMoveFinished(result) = controller
                    .events
                    .recv_timeout(Duration::from_secs(2))
                    .unwrap()
                {
                    assert_eq!(result.unwrap(), target);
                    break;
                }
            }
        }
        for target in [-0.001, 25.001, f64::NAN] {
            assert!(controller.manual_move(target).is_err());
        }
    }
    #[test]
    fn demo_run_saves_raw_bg_corrected_and_json() {
        let (controller, config, _dir) = setup();
        let planned = config.plan().unwrap().acquisition_count;
        controller.begin(config.clone()).unwrap();
        let mut path = None;
        let mut count = 0;
        loop {
            match controller
                .events
                .recv_timeout(Duration::from_secs(5))
                .unwrap()
            {
                Event::RunCreated(p) => path = Some(p),
                Event::AwaitBackground | Event::AwaitSignal => {
                    controller.commands.send(Command::Continue).unwrap()
                }
                Event::Preview(_) => {
                    controller
                        .commands
                        .send(Command::SelectAnalysis(Box::new(config.clone())))
                        .unwrap();
                }
                Event::Sample(s) => {
                    count += 1;
                    assert!(s.excitation_nm >= 680. && s.excitation_nm <= 740.);
                    assert!(s.pl_integral_counts_nm > 0.);
                    assert_eq!(s.corrected_counts.len(), 641);
                }
                Event::Finished { state, error } => {
                    assert_eq!(state.label(), "Completed", "{error:?}");
                    assert!(error.is_none());
                    break;
                }
                _ => {}
            }
        }
        assert_eq!(count, planned);
        let path = std::path::PathBuf::from(path.unwrap());
        let (_, samples) = ple_core::recording::load_run(&path).unwrap();
        assert_eq!(samples.len(), planned);
        assert!(path.join("raw/background.csv").exists());
        assert_eq!(
            std::fs::read_dir(path.join("processed")).unwrap().count(),
            planned
        );
        assert!(path.join("analysis.json").exists());
        assert!(path.join("accepted_settings.json").exists());
        assert!(path.join("settings.json").exists());
        let inventory: serde_json::Value =
            serde_json::from_slice(&std::fs::read(path.join("hardware.json")).unwrap()).unwrap();
        assert_eq!(inventory["hardware"]["source"], "simulated");
        let events = std::fs::read_to_string(path.join("events.jsonl")).unwrap();
        let readings: Vec<serde_json::Value> = events
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .filter(|v: &serde_json::Value| v["kind"] == "acquisition_started")
            .collect();
        assert_eq!(readings.len(), planned);
        assert!(
            readings
                .iter()
                .all(|v| v["data"]["power_hardware"]["source"] == "simulated")
        );
        assert!(!path.join("raw/background.spe").exists());
    }
    #[test]
    fn review_is_paused_invalid_selection_can_be_corrected_and_first_raw_is_reused() {
        let (controller, mut config, _dir) = setup();
        config.background.mode = BackgroundMode::Automatic;
        controller.begin(config.clone()).unwrap();
        let mut preview = None;
        let mut path = None;
        let mut rejected = false;
        let mut first = false;
        loop {
            match controller
                .events
                .recv_timeout(Duration::from_secs(5))
                .unwrap()
            {
                Event::RunCreated(p) => path = Some(p),
                Event::AwaitBackground | Event::AwaitSignal => {
                    panic!("Automatic shutter requested manual confirmation")
                }
                Event::Preview(a) => {
                    assert_eq!(a.point_index, 0);
                    preview = Some(a.clone());
                    // No sample may be committed before the user accepts analysis.
                    let until = Instant::now() + Duration::from_millis(250);
                    while Instant::now() < until {
                        if let Ok(e) = controller.events.recv_timeout(Duration::from_millis(25)) {
                            assert!(!matches!(e, Event::Sample(_) | Event::Finished { .. }));
                        }
                    }
                    let mut invalid = config.clone();
                    let mut analysis = config.initial_analysis();
                    analysis.integral_min_nm = 900.;
                    invalid.analysis = Some(analysis);
                    controller
                        .commands
                        .send(Command::SelectAnalysis(Box::new(invalid)))
                        .unwrap();
                }
                Event::AnalysisRejected(_) => {
                    rejected = true;
                    controller
                        .commands
                        .send(Command::SelectAnalysis(Box::new(config.clone())))
                        .unwrap();
                }
                Event::Sample(s) => {
                    if !first {
                        let p = preview.as_ref().unwrap();
                        assert_eq!(s.spectrum.counts, p.spectrum.counts);
                        assert_eq!(s.file_base, p.file_base);
                        assert_eq!(s.acquisition_started_unix_s, p.acquisition_started_unix_s);
                        first = true;
                    }
                }
                Event::Finished { state, error } => {
                    assert_eq!(state.label(), "Completed", "{error:?}");
                    break;
                }
                _ => {}
            }
        }
        assert!(rejected && first);
        let path = std::path::PathBuf::from(path.unwrap());
        let status: serde_json::Value =
            serde_json::from_slice(&std::fs::read(path.join("status.json")).unwrap()).unwrap();
        assert_eq!(status["analysis_confirmed"], true);
        let events = std::fs::read_to_string(path.join("events.jsonl")).unwrap();
        assert!(events.contains("shutter_end_close"));
    }
    #[test]
    fn stopping_during_review_retains_preview_without_committing_analysis() {
        let (controller, mut config, _dir) = setup();
        config.background.mode = BackgroundMode::Automatic;
        controller.begin(config).unwrap();
        let mut path = None;
        let mut basename = None;
        loop {
            match controller
                .events
                .recv_timeout(Duration::from_secs(5))
                .unwrap()
            {
                Event::RunCreated(p) => path = Some(p),
                Event::Preview(a) => {
                    basename = Some(a.file_base.clone());
                    controller.stop();
                }
                Event::Sample(_) => panic!("Review was not accepted"),
                Event::Finished { state, .. } => {
                    assert_eq!(state.label(), "Stopped");
                    break;
                }
                _ => {}
            }
        }
        let p = std::path::PathBuf::from(path.unwrap());
        assert!(
            p.join("raw")
                .join(format!("{}.csv", basename.unwrap()))
                .exists()
        );
        assert!(p.join("metadata/preview.json").exists());
        assert!(!p.join("analysis.json").exists());
    }
    #[test]
    fn immediate_stop_is_not_lost_and_retains_run() {
        let (controller, config, _dir) = setup();
        controller.begin(config).unwrap();
        controller.stop();
        let mut path = None;
        loop {
            match controller
                .events
                .recv_timeout(Duration::from_secs(5))
                .unwrap()
            {
                Event::RunCreated(p) => path = Some(p),
                Event::Sample(_) => panic!("Stopped before BG confirmation but acquired a signal"),
                Event::Finished { state, .. } => {
                    assert_eq!(state.label(), "Stopped");
                    break;
                }
                _ => {}
            }
        }
        let status: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(std::path::Path::new(&path.unwrap()).join("status.json"))
                .unwrap(),
        )
        .unwrap();
        assert_eq!(status["completed_acquisitions"], 0);
    }
}
