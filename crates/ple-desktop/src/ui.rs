//! PLE bindings for the measured TRKR desktop shell; no instrument I/O here.
use super::*;
use crate::{
    precision::{self, Form, rect},
    visuals::*,
};
use egui::{Align, Align2, FontId, Layout, Rect, Stroke, Vec2};
use std::sync::mpsc::{Receiver, TryRecvError};

#[derive(Clone, Copy, PartialEq)]
pub(super) enum Advanced {
    Spectrometer,
    Stage,
    Power,
    Excitation,
}
#[derive(Clone, Copy)]
pub(super) enum DialogTarget {
    Config,
    Output,
    Results,
    Driver,
}
pub(super) struct PendingDialog {
    target: DialogTarget,
    result: Receiver<Result<Option<PathBuf>, String>>,
}
impl PleApp {
    fn locked(&self) -> bool {
        self.api_owned()
            || self.api_cleanup
            || self.controller.stop_status().0
            || self.busy
            || self.manual_moving
            || self.spectrometer_configuring
            || self.pending_connections.iter().any(|v| *v)
            || self.dialog.is_some()
    }
    fn device_action(&mut self, index: usize, connect: bool) {
        if self.locked() {
            return;
        }
        let device = ["spectrometer", "stage", "power"][index];
        self.pending_connections[index] = true;
        if connect {
            self.send(Command::Connect(device, self.config.clone()));
        } else {
            self.send(Command::Disconnect(device));
        }
    }
    fn all_devices(&mut self, connect: bool) {
        if self.locked() {
            return;
        }
        for (i, device) in ["spectrometer", "stage", "power"].into_iter().enumerate() {
            if self.connected[i] != connect
                || (!connect && i == 0 && self.controller.has_spectrometer_owner())
            {
                self.pending_connections[i] = true;
                self.send(if connect {
                    Command::Connect(device, self.config.clone())
                } else {
                    Command::Disconnect(device)
                });
            }
        }
    }
    fn choose(&mut self, target: DialogTarget) {
        if self.locked() {
            return;
        }
        let (kind, initial) = match target {
            DialogTarget::Config => (file_dialog::Kind::Config, &self.config_path),
            DialogTarget::Output => (file_dialog::Kind::Directory, &self.config.output_dir),
            DialogTarget::Results => (file_dialog::Kind::Results, &self.load_path),
            DialogTarget::Driver => (file_dialog::Kind::Directory, &self.config.power.driver_path),
        };
        self.dialog = Some(PendingDialog {
            target,
            result: file_dialog::open(kind, initial),
        });
    }
    pub(super) fn poll_dialog(&mut self) {
        let result = self
            .dialog
            .as_ref()
            .and_then(|d| match d.result.try_recv() {
                Ok(r) => Some((d.target, r)),
                Err(TryRecvError::Empty) => None,
                Err(TryRecvError::Disconnected) => {
                    Some((d.target, Err("Native dialog closed unexpectedly".into())))
                }
            });
        if let Some((target, result)) = result {
            self.dialog = None;
            match result {
                Ok(Some(path)) => {
                    let path = path.to_string_lossy().into_owned();
                    match target {
                        DialogTarget::Config => self.config_path = path,
                        DialogTarget::Output => self.config.output_dir = path,
                        DialogTarget::Results => {
                            self.load_path = path;
                            self.open_results();
                        }
                        DialogTarget::Driver => self.config.power.driver_path = path,
                    }
                }
                Ok(None) => {}
                Err(e) => {
                    self.log(&e);
                    self.message = Some(e);
                }
            }
        }
    }
    fn load_profile(&mut self) {
        if self.connected.iter().any(|v| *v) {
            self.message = Some("Disconnect all devices before loading a profile".into());
            return;
        }
        let result = std::fs::read_to_string(&self.config_path)
            .map_err(|e| e.to_string())
            .and_then(|s| serde_json::from_str::<Config>(&s).map_err(|e| e.to_string()))
            .and_then(|c| {
                c.plan()?;
                Ok(c)
            });
        match result {
            Ok(c) => {
                self.analysis_draft = c.initial_analysis();
                self.preview = None;
                self.config = c;
                self.stage_history.clear();
                self.power_history.clear();
                self.log("Configuration loaded");
                self.message = None;
            }
            Err(e) => {
                self.log(&e);
                self.message = Some(e);
            }
        }
    }
    fn open_results(&mut self) {
        if self.busy || self.connected.iter().any(|v| *v) {
            self.message = Some("Disconnect all devices before opening a saved run".into());
            return;
        }
        match recording::load_run(std::path::Path::new(&self.load_path)).and_then(|(c, samples)| {
            let path = std::path::Path::new(&self.load_path).join("metadata/background.json");
            let bg: Spectrum =
                serde_json::from_reader(std::fs::File::open(path).map_err(|e| e.to_string())?)
                    .map_err(|e| e.to_string())?;
            bg.validate()?;
            if samples.iter().any(|s| {
                s.spectrum.wavelength_nm.len() != bg.wavelength_nm.len()
                    || s.spectrum
                        .wavelength_nm
                        .iter()
                        .zip(&bg.wavelength_nm)
                        .any(|(a, b)| (a - b).abs() > 1e-7)
            }) {
                return Err("Saved spectra do not share the recorded BG wavelength grid".into());
            }
            Ok((c, samples, bg))
        }) {
            Ok((c, samples, bg)) => {
                self.background = Some(bg);
                self.first_saved = None;
                self.last_saved = None;
                self.analysis_draft = c.initial_analysis();
                self.preview = None;
                self.config = c;
                self.samples = samples.into_iter().map(Arc::new).collect();
                self.run_dir = self.load_path.clone();
                self.run_count = self
                    .config
                    .plan()
                    .map(|p| p.acquisition_count)
                    .unwrap_or(self.samples.len());
                self.state = "Recorded results".into();
                self.selected = None;
                self.started = None;
                self.finished_elapsed = None;
                self.heatmap_cache = None;
                self.plot_epoch = self.plot_epoch.wrapping_add(1);
                self.message = None;
                self.log("Recorded results opened; no hardware actions");
            }
            Err(e) => {
                self.log(&e);
                self.message = Some(e);
            }
        }
    }
    fn config_card(&mut self, ui: &mut egui::Ui, area: Rect) {
        let locked = self.locked();
        let (browse, load, save, connect, disconnect);
        {
            let mut f = Form::new(ui, &mut self.geometry, area.min);
            f.enabled = !locked;
            let w = area.width();
            let inner = w - 22.;
            let half = (inner - 8.) * 0.5;
            f.group("session_group", rect(0., 0., w, 128.), "Config");
            f.text(
                "config_path",
                rect(11., 27., inner - 78., 26.),
                &mut self.config_path,
            );
            browse = f.button(
                "browse_button",
                rect(w - 81., 27., 70., 26.),
                "Browse",
                true,
            );
            load = f.button("load_button", rect(11., 59., half, 26.), "Load", true);
            save = f.button(
                "save_button",
                rect(19. + half, 59., half, 26.),
                "Save",
                true,
            );
            connect = f.button(
                "all_connect_button",
                rect(11., 91., half, 26.),
                "All Connect",
                true,
            );
            disconnect = f.button(
                "all_disconnect_button",
                rect(19. + half, 91., half, 26.),
                "All Disconnect",
                true,
            );
        }
        let mut demo = self.config.mode == Mode::Demo;
        ui.interact(
            area,
            egui::Id::new("session-mode-menu"),
            egui::Sense::hover(),
        )
        .context_menu(|ui| {
            ui.add_enabled_ui(!locked && !self.connected.iter().any(|v| *v), |ui| {
                if ui.checkbox(&mut demo, "Demo mode").changed() {
                    self.config.mode = if demo { Mode::Demo } else { Mode::Hardware };
                    self.stage_history.clear();
                    self.power_history.clear();
                    self.stage_value = None;
                    self.power_value = None;
                }
            });
            ui.label("Mode is selected when Connect is pressed.");
        });
        if browse {
            self.choose(DialogTarget::Config);
        }
        if load {
            self.load_profile();
        }
        if save {
            match save_profile(std::path::Path::new(&self.config_path), &self.config) {
                Ok(()) => {
                    self.message = None;
                    self.log("Configuration saved");
                }
                Err(e) => {
                    self.log(&e);
                    self.message = Some(e);
                }
            }
        }
        if connect {
            self.all_devices(true);
        }
        if disconnect {
            self.all_devices(false);
        }
    }
    fn device_height(&self, index: usize) -> f32 {
        match (index, self.device_expanded[index]) {
            (0, false) => 158.,
            (0, true) => 396.,
            (_, false) => 234.,
            (1, true) => 498.,
            (_, true) => 426.,
        }
    }
    fn device_card(&mut self, ui: &mut egui::Ui, index: usize, area: Rect) {
        let locked = self.locked();
        let recovery_ready = self.recovery_reason().is_none();
        let connected = self.connected[index];
        let owned = index == 0 && self.controller.has_spectrometer_owner();
        let w = area.width();
        let expanded = self.device_expanded[index];
        let mut connect = false;
        let mut disconnect = false;
        let mut settings = false;
        let mut apply = false;
        let mut manual_move = false;
        let mut manual_stop = false;
        {
            let mut f = Form::new(ui, &mut self.geometry, area.min);
            f.group(
                &format!("device_{index}_group"),
                rect(0., 0., w, area.height()),
                ["Spectrometer", "CONEX CC", "Power meter"][index],
            );
            f.enabled = !self.busy && !self.manual_moving;
            f.toggle(
                &format!("device_{index}_toggle"),
                rect(11., 27., 71., 24.),
                &mut self.device_expanded[index],
                "Device",
            );
            let caption = if self.pending_connections[index] {
                if connected {
                    "Disconnecting…"
                } else {
                    "Connecting…"
                }
            } else if connected {
                ["LightField", "CONEX-CC", "Thorlabs TLPM"][index]
            } else {
                "----"
            };
            f.value(
                &format!("device_{index}_name"),
                rect(102., 27., w - 113., 24.),
                caption,
                if self.pending_connections[index] {
                    TEAL
                } else {
                    TEXT
                },
            );
            f.ui.painter().circle_filled(
                area.min + Vec2::new(93., 39.),
                3.,
                if connected {
                    GREEN
                } else {
                    egui::Color32::GRAY
                },
            );
            if expanded {
                f.enabled = !locked && !connected;
                match index {
                    0 => {
                        f.enabled = !locked && !connected;
                        f.label(
                            "lf_experiment_label",
                            rect(20., 66., 73., 26.),
                            "Experiment",
                        );
                        f.text(
                            "lf_experiment",
                            rect(98., 66., w - 118., 26.),
                            &mut self.config.spectrometer.experiment,
                        );
                        f.enabled = !locked;
                        f.label("lf_center_label", rect(20., 98., 73., 26.), "Center (nm)");
                        f.number(
                            "lf_center",
                            rect(98., 98., w - 118., 26.),
                            &mut self.config.spectrometer.center_nm,
                            0.1,
                            3,
                            true,
                        );
                        f.label("lf_grating_label", rect(20., 130., 73., 26.), "Grating");
                        f.text(
                            "lf_grating",
                            rect(98., 130., w - 118., 26.),
                            &mut self.config.spectrometer.grating,
                        );
                        let presets = [
                            ("Template", ""),
                            ("300/1µm", "[1.0um,300][2][0]"),
                            ("300/500", "[500nm,300][1][0]"),
                            ("1200/500", "[500nm,1200][0][0]"),
                        ];
                        for (i, (name, value)) in presets.iter().enumerate() {
                            if f.button(
                                &format!("grating_{i}"),
                                rect(20. + i as f32 * 73., 162., 70., 24.),
                                name,
                                true,
                            ) {
                                self.config.spectrometer.grating = (*value).into();
                            }
                        }
                        f.label("lf_exposure_label", rect(20., 194., 73., 26.), "Exp. (ms)");
                        f.number(
                            "lf_exposure",
                            rect(98., 194., w - 118., 26.),
                            &mut self.config.spectrometer.exposure_ms,
                            10.,
                            3,
                            true,
                        );
                        let mut frames = self.config.spectrometer.frames_combined as f64;
                        f.label("lf_frames_label", rect(20., 226., 73., 26.), "Frames");
                        f.number(
                            "lf_frames",
                            rect(98., 226., w - 118., 26.),
                            &mut frames,
                            1.,
                            0,
                            true,
                        );
                        self.config.spectrometer.frames_combined =
                            frames.round().clamp(1., 10000.) as u32;
                        apply = f.button(
                            "lf_apply",
                            rect(20., 258., w - 40., 26.),
                            "Apply / read back",
                            connected && recovery_ready,
                        );
                    }
                    1 => {
                        if f.button(
                            "protocol_json",
                            rect(20., 66., 140., 24.),
                            "Remote JSON",
                            true,
                        ) {
                            self.config.stage.protocol = StageProtocol::RemoteJson;
                        }
                        if f.button(
                            "protocol_legacy",
                            rect(168., 66., 140., 24.),
                            "Legacy TCP",
                            true,
                        ) {
                            self.config.stage.protocol = StageProtocol::LegacyTcp;
                        }

                        f.label("stage_host_label", rect(20., 98., 73., 26.), "Host");
                        f.text(
                            "stage_host",
                            rect(98., 98., w - 118., 26.),
                            &mut self.config.stage.host,
                        );
                        let mut port = self.config.stage.port as f64;
                        f.label("stage_port_label", rect(20., 130., 73., 26.), "Port / COM");
                        f.number(
                            "stage_port",
                            rect(98., 130., 72., 26.),
                            &mut port,
                            1.,
                            0,
                            true,
                        );
                        self.config.stage.port = port.round().clamp(1., 65535.) as u16;
                        f.text(
                            "stage_com",
                            rect(178., 130., w - 198., 26.),
                            &mut self.config.stage.com_port,
                        );
                        f.label("stage_min_label", rect(20., 162., 73., 26.), "Soft min");
                        f.number(
                            "stage_min",
                            rect(98., 162., w - 118., 26.),
                            &mut self.config.stage.min_mm,
                            0.1,
                            3,
                            true,
                        );
                        f.label("stage_max_label", rect(20., 194., 73., 26.), "Soft max");
                        f.number(
                            "stage_max",
                            rect(98., 194., w - 118., 26.),
                            &mut self.config.stage.max_mm,
                            0.1,
                            3,
                            true,
                        );
                    }
                    _ => {
                        f.label("power_resource_label", rect(20., 66., 73., 26.), "Resource");
                        f.text(
                            "power_resource",
                            rect(98., 66., w - 118., 26.),
                            &mut self.config.power.resource,
                        );
                        f.label(
                            "power_lambda_label",
                            rect(20., 98., 73., 26.),
                            "Sensor (nm)",
                        );
                        f.number(
                            "power_lambda",
                            rect(98., 98., w - 118., 26.),
                            &mut self.config.power.wavelength_nm,
                            0.1,
                            3,
                            true,
                        );
                        f.label(
                            "power_factor_label",
                            rect(20., 130., 73., 26.),
                            "Sample factor",
                        );
                        f.number(
                            "power_factor",
                            rect(98., 130., w - 118., 26.),
                            &mut self.config.power.correction_factor,
                            0.001,
                            6,
                            true,
                        );
                        f.label(
                            "power_average_label",
                            rect(20., 162., 73., 26.),
                            "Average (s)",
                        );
                        f.number(
                            "power_average",
                            rect(98., 162., w - 118., 26.),
                            &mut self.config.power.average_s,
                            0.1,
                            3,
                            true,
                        );
                    }
                }
                f.enabled = !locked;
                let bw = (w - 36.) / 3.;
                connect = f.button(
                    &format!("connect_{index}"),
                    rect(
                        11.,
                        if index == 0 {
                            294.
                        } else if index == 1 {
                            226.
                        } else {
                            194.
                        },
                        bw,
                        26.,
                    ),
                    "Connect",
                    !connected,
                );
                disconnect = f.button(
                    &format!("disconnect_{index}"),
                    rect(
                        18. + bw,
                        if index == 0 {
                            294.
                        } else if index == 1 {
                            226.
                        } else {
                            194.
                        },
                        bw,
                        26.,
                    ),
                    "Disconnect",
                    connected || owned,
                );
                settings = f.button(
                    &format!("settings_{index}"),
                    rect(
                        25. + bw * 2.,
                        if index == 0 {
                            294.
                        } else if index == 1 {
                            226.
                        } else {
                            194.
                        },
                        bw,
                        26.,
                    ),
                    "Settings",
                    true,
                );
            }
            if expanded && index == 1 {
                f.enabled = !locked && connected && recovery_ready;
                f.label(
                    "manual_target_label",
                    rect(20., 262., 73., 26.),
                    "Go to (mm)",
                );
                f.number(
                    "manual_target",
                    rect(98., 262., 104., 26.),
                    &mut self.manual_target_mm,
                    0.01,
                    6,
                    true,
                );
                manual_move = f.button(
                    "manual_move",
                    rect(210., 262., 48., 26.),
                    "Move",
                    self.manual_target_mm.is_finite()
                        && self.manual_target_mm >= self.config.stage.min_mm
                        && self.manual_target_mm <= self.config.stage.max_mm,
                );
                f.value(
                    "stage_travel",
                    rect(20., 464., w - 40., 20.),
                    "Travel: 0-25 mm / manual move while idle",
                    MUTED,
                );
                f.enabled = true;
                manual_stop = f.button(
                    "manual_stop",
                    rect(266., 262., 48., 26.),
                    "Stop",
                    self.manual_moving,
                );
            }
            if index == 0 {
                let y = if expanded { 330. } else { 66. };
                f.value(
                    "exposure_summary",
                    rect(20., y, w - 40., 22.),
                    format!(
                        "Exposure (ms)     {:0.3}",
                        self.spectrometer_settings["exposure_ms"]
                            .as_f64()
                            .unwrap_or(self.config.spectrometer.exposure_ms)
                    ),
                    TEXT,
                );
                f.value(
                    "center_summary",
                    rect(20., y + 26., w - 40., 22.),
                    format!(
                        "Center (nm)         {:0.3}",
                        self.spectrometer_settings["center_nm"]
                            .as_f64()
                            .unwrap_or(self.config.spectrometer.center_nm)
                    ),
                    TEXT,
                );
                if !expanded {
                    f.value(
                        "spectrometer_status",
                        rect(20., y + 52., w - 40., 22.),
                        if self.busy {
                            "Status     Acquiring / waiting"
                        } else if !recovery_ready {
                            "Status     Recovery required"
                        } else if connected {
                            "Status     Ready"
                        } else {
                            "Status     Disconnected"
                        },
                        MUTED,
                    );
                }
            }
        }
        if index > 0 {
            let y = if expanded {
                if index == 1 { 304. } else { 232. }
            } else {
                65.
            };
            let monitor = rect(area.left() + 11., area.top() + y, w - 22., 156.);
            let (history, channel) = if index == 1 {
                (&self.stage_history, live_monitor::STAGE_CHANNEL)
            } else {
                (&self.power_history, live_monitor::POWER_CHANNEL)
            };
            precision::record(&mut self.geometry, channel.id, monitor);
            live_monitor::draw(ui, monitor, history, channel, connected);
        }
        if manual_move && self.recovery_reason().is_none() {
            match self.controller.manual_move(self.manual_target_mm) {
                Ok(()) => {
                    self.manual_moving = true;
                    self.message = None;
                }
                Err(e) => self.message = Some(e),
            }
        }
        if manual_stop {
            self.controller.stop();
            self.state = "Manual Stop requested; waiting for readback".into();
        }
        if apply && self.recovery_reason().is_none() {
            self.spectrometer_configuring = true;
            self.send(Command::ConfigureSpectrometer(Box::new(
                self.config.clone(),
            )));
        }
        if connect {
            self.device_action(index, true);
        }
        if disconnect {
            self.device_action(index, false);
        }
        if settings {
            self.advanced = Some([Advanced::Spectrometer, Advanced::Stage, Advanced::Power][index]);
        }
    }
    fn measurement(&mut self, ui: &mut egui::Ui, area: Rect) {
        let locked = self.locked();
        let recovery_ready = self.recovery_reason().is_none();
        let externally_owned = self.api_owned();
        let (start, stop, open, browse, excitation);
        let tab = rect(area.left(), area.top(), 65., 24.);
        precision::record(&mut self.geometry, "tab_ple", tab);
        ui.put(
            tab,
            egui::Button::new("PLE")
                .selected(true)
                .min_size(tab.size())
                .corner_radius(6),
        );
        let page = rect(
            area.left() + 2.,
            area.top() + 26.,
            area.width() - 6.,
            area.height() - 30.,
        );
        ui.painter().rect(
            page,
            10.,
            PANEL,
            Stroke::new(1_f32, BORDER),
            egui::StrokeKind::Inside,
        );
        {
            let mut f = Form::new(ui, &mut self.geometry, page.min);
            f.enabled = !locked;
            let right = page.width() - 260.;
            let col = 270.;

            for (name, y, label, value, step, digits) in [
                (
                    "scan_min",
                    11.,
                    "Min (mm)",
                    &mut self.config.scan.start_mm,
                    0.001,
                    6,
                ),
                (
                    "scan_max",
                    43.,
                    "Max (mm)",
                    &mut self.config.scan.stop_mm,
                    0.001,
                    6,
                ),
                (
                    "scan_step",
                    75.,
                    "Step (mm)",
                    &mut self.config.scan.step_mm,
                    0.001,
                    6,
                ),
                (
                    "scan_wait",
                    107.,
                    "Wait (s)",
                    &mut self.config.scan.settle_s,
                    0.1,
                    3,
                ),
            ] {
                f.label(&format!("{name}_label"), rect(10., y, 82., 26.), label);
                f.number(name, rect(94., y, 100., 26.), value, step, digits, true);
            }
            f.value(
                "scan_min_hint",
                rect(200., 11., 66., 26.),
                format!("≥ {:0.2}", self.config.stage.min_mm),
                MUTED,
            );
            f.value(
                "scan_max_hint",
                rect(200., 43., 66., 26.),
                format!("≤ {:0.2}", self.config.stage.max_mm),
                MUTED,
            );
            if let Ok(p) = self.config.plan() {
                f.value(
                    "scan_points_hint",
                    rect(200., 75., 66., 26.),
                    format!("{} pts", p.positions_mm.len()),
                    MUTED,
                );
            }
            f.enabled = self.awaiting == 3 && !self.analysis_confirm_pending && !externally_owned;
            for (name, y, label, value) in [
                (
                    "pl_peak",
                    11.,
                    "Peak (nm)",
                    &mut self.analysis_draft.peak_nm,
                ),
                (
                    "integral_min",
                    43.,
                    "Integral min",
                    &mut self.analysis_draft.integral_min_nm,
                ),
                (
                    "integral_max",
                    75.,
                    "Integral max",
                    &mut self.analysis_draft.integral_max_nm,
                ),
            ] {
                f.label(&format!("{name}_label"), rect(col, y, 88., 26.), label);
                f.number(name, rect(col + 90., y, 112., 26.), value, 0.1, 3, true);
            }
            f.enabled = !locked;
            let mut repeats = self.config.scan.repeats as f64;
            f.label("repeats_label", rect(col, 107., 88., 26.), "Repeats");
            f.number(
                "repeats",
                rect(col + 90., 107., 112., 26.),
                &mut repeats,
                1.,
                0,
                true,
            );
            self.config.scan.repeats = repeats.round().clamp(1., 20000.) as u32;
            f.value(
                "bg_policy",
                rect(10., 141., 245., 20.),
                if self.awaiting == 3 {
                    "Review first PL · then Continue"
                } else {
                    "BG > first PL > choose analysis > scan"
                },
                MUTED,
            );
            f.enabled = !locked || self.awaiting == 3;
            excitation = f.button(
                "excitation_settings",
                rect(col, 141., 202., 24.),
                "Analysis / BG settings",
                true,
            );
            match self.config.plan() {
                Ok(plan) => f.value(
                    "plan_hint",
                    rect(10., 170., right - 20., 18.),
                    format!(
                        "{} acquisitions · last target {:.6} mm",
                        plan.acquisition_count,
                        plan.positions_mm.last().copied().unwrap_or(0.)
                    ),
                    MUTED,
                ),
                Err(error) => f.value("plan_hint", rect(10., 170., right - 20., 18.), error, RED),
            }
            f.enabled = !locked;
            f.group("output_group", rect(right, 11., 248., 94.), "Output");
            f.label("output_dir_label", rect(right + 11., 37., 29., 26.), "Dir");
            f.text(
                "output_dir",
                rect(right + 45., 37., 151., 26.),
                &mut self.config.output_dir,
            );
            browse = f.button(
                "output_browse",
                rect(right + 204., 37., 33., 26.),
                "...",
                true,
            );
            f.label("sample_label", rect(right + 11., 69., 36., 26.), "Sample");
            f.text(
                "sample",
                rect(right + 54., 69., 183., 26.),
                &mut self.config.sample,
            );
            f.group("run_group", rect(right, 117., 248., 63.), "Run");
            f.enabled = true;
            start = f.button(
                "run_start",
                rect(right + 11., 143., 70., 26.),
                if self.awaiting == 3 {
                    "Continue"
                } else {
                    "Start"
                },
                if self.awaiting == 3 {
                    !self.analysis_confirm_pending && !externally_owned
                } else {
                    !locked && recovery_ready && self.connected.iter().all(|v| *v)
                },
            );
            stop = f.button(
                "run_stop",
                rect(right + 89., 143., 70., 26.),
                "Stop",
                self.busy || self.manual_moving || self.spectrometer_configuring,
            );
            open = f.button(
                "run_open",
                rect(right + 167., 143., 70., 26.),
                "Open",
                !locked && !self.connected.iter().any(|v| *v),
            );
        }
        if start {
            if self.awaiting == 3 {
                self.confirm_analysis();
            } else {
                self.start();
            }
        }
        if stop {
            self.controller.stop();
            self.state = "Stop requested — waiting for device confirmation".into();
            self.awaiting = 0;
        }
        if browse {
            self.choose(DialogTarget::Output);
        }
        if open {
            self.choose(DialogTarget::Results);
        }
        if excitation {
            self.advanced = Some(Advanced::Excitation);
        }
    }
    fn toolbar(&mut self, ui: &mut egui::Ui, area: Rect) {
        let total = if self.run_count > 0 {
            self.run_count
        } else {
            self.config.plan().map(|p| p.acquisition_count).unwrap_or(0)
        };
        let done = self.samples.len();
        let eta = if self.busy && done >= 2 && done < total {
            match (self.first_saved, self.last_saved) {
                (Some(first), Some(last)) => {
                    let remaining = last.duration_since(first).as_secs_f64() / (done - 1) as f64
                        * (total - done) as f64
                        - last.elapsed().as_secs_f64();
                    if remaining > 0. {
                        format!("~ {:.0} s", remaining)
                    } else {
                        "-".into()
                    }
                }
                _ => "-".into(),
            }
        } else {
            "-".into()
        };
        let mut f = Form::new(ui, &mut self.geometry, area.min);
        f.label("plot_label", rect(0., 0., 28., 20.), "Plot");
        f.label("step_label", rect(48., 0., 28., 20.), "Step");
        f.label(
            "step_value",
            rect(86., 0., 62., 20.),
            if total > 0 {
                format!("{done}/{total}")
            } else {
                "-".into()
            },
        );
        f.label("eta_title", rect(154., 0., 28., 20.), "ETA");
        f.label("eta_value", rect(192., 0., 80., 20.), eta);
        if self.config.mode == Mode::Demo {
            f.value(
                "demo_badge",
                rect(275., 0., 165., 20.),
                "DEMO · NO HARDWARE",
                RED,
            );
        }
        let x = area.width() - 345.;
        f.label("display_x_label", rect(x, 0., 12., 20.), "X");
        f.combo(
            "display_x",
            rect(x + 18., 0., 145., 22.),
            &mut self.energy,
            &[(false, "Wavelength (nm)"), (true, "Energy (eV)")],
        );
        f.label("display_y_label", rect(x + 172., 0., 12., 20.), "Y");
        f.combo(
            "display_y",
            rect(x + 191., 0., 154., 22.),
            &mut self.normalize,
            &[(false, "Counts"), (true, "Counts / W")],
        );
        ui.interact(area, egui::Id::new("plot_options"), egui::Sense::hover())
            .context_menu(|ui| {
                if ui.button("Reset plot view").clicked() {
                    self.plot_epoch = self.plot_epoch.wrapping_add(1);
                    ui.close();
                }
                if ui.button("Follow latest acquisition").clicked() {
                    self.selected = None;
                    ui.close();
                }
                ui.label("Display changes never modify saved spectra.");
            });
    }
    fn snapshot_fields(&self) -> Vec<(&'static str, String)> {
        let s = self
            .selected
            .and_then(|i| self.samples.get(i))
            .or_else(|| self.samples.last());
        let v = |n: Option<f64>| n.map(|n| format!("{n:0.6}")).unwrap_or("-".into());
        vec![
            ("measurement", "ple".into()),
            (
                "mode",
                if self.config.mode == Mode::Demo {
                    "demo"
                } else {
                    "hardware"
                }
                .into(),
            ),
            ("phase", self.state.clone()),
            ("spectrometer", self.connected[0].to_string()),
            ("conex_cc", self.connected[1].to_string()),
            ("power_meter", self.connected[2].to_string()),
            (
                "power_sensor_nm",
                v(self
                    .power_sensor_wavelength
                    .or_else(|| s.and_then(|s| s.power_sensor_wavelength_nm))),
            ),
            (
                "point",
                format!("{}/{}", self.samples.len(), self.run_count),
            ),
            ("target_mm", v(s.map(|s| s.target_mm))),
            ("position_mm", v(s.map(|s| s.actual_mm))),
            ("excitation_nm", v(s.map(|s| s.excitation_nm))),
            ("excitation_eV", v(s.map(|s| HC_EV_NM / s.excitation_nm))),
            ("PL_peak_nm", v(s.and_then(|s| s.pl_peak_nm))),
            ("PL_area", v(s.map(|s| s.pl_integral_counts_nm))),
            (
                "power_W",
                s.map(|s| format!("{:0.6e}", s.power_w))
                    .unwrap_or("-".into()),
            ),
            (
                "sample_power_W",
                s.map(|s| format!("{:0.6e}", s.sample_power_w))
                    .unwrap_or("-".into()),
            ),
            ("exposure_ms", v(Some(self.config.spectrometer.exposure_ms))),
            ("repeats", self.config.scan.repeats.to_string()),
            (
                "LF_frames",
                self.config.spectrometer.frames_combined.to_string(),
            ),
            (
                "BG",
                if self.background.is_some() {
                    "acquired / subtracted"
                } else {
                    "not acquired"
                }
                .into(),
            ),
            (
                "elapsed_s",
                v(self
                    .finished_elapsed
                    .or_else(|| self.started.map(|t| t.elapsed().as_secs_f64()))),
            ),
            ("run_dir", self.run_dir.clone()),
        ]
    }
    fn details(&mut self, ui: &mut egui::Ui, columns: precision::Columns) {
        ui.painter().rect(
            columns.log,
            8.,
            BG,
            Stroke::new(1_f32, BORDER),
            egui::StrokeKind::Inside,
        );
        ui.scope_builder(
            egui::UiBuilder::new()
                .max_rect(columns.log.shrink(4.))
                .layout(Layout::top_down(Align::Min)),
            |ui| {
                ui.set_clip_rect(ui.clip_rect().intersect(columns.log.shrink(4.)));
                egui::ScrollArea::vertical()
                    .id_salt("log_scroll")
                    .stick_to_bottom(true)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        if self.config.mode == Mode::Demo {
                            ui.label("DEMO — synthetic data · no hardware");
                        }
                        for line in &self.logs {
                            ui.add(egui::Label::new(line).wrap());
                        }
                        if let Some(error) = &self.message {
                            ui.add(egui::Label::new(RichText::new(error).color(RED)).wrap());
                        }
                    });
            },
        );
        let area = columns.snapshot;
        let inner = area.shrink(2.);
        let split = 101.;
        let header = rect(inner.left(), inner.top(), inner.width() - 12., 24.);
        ui.painter().rect_stroke(
            area,
            0.,
            Stroke::new(1_f32, BORDER),
            egui::StrokeKind::Inside,
        );
        ui.painter()
            .rect_filled(header, 0., egui::Color32::from_rgb(45, 46, 52));
        ui.painter().line_segment(
            [
                egui::pos2(inner.left() + split, inner.top()),
                egui::pos2(inner.left() + split, inner.bottom()),
            ],
            Stroke::new(1_f32, BORDER),
        );
        for (x, w, title) in [
            (inner.left(), split, "Field"),
            (inner.left() + split, header.width() - split, "Value"),
        ] {
            ui.painter().text(
                egui::pos2(x + w * 0.5, header.center().y),
                Align2::CENTER_CENTER,
                title,
                FontId::proportional(12.),
                TEXT,
            );
        }
        let body = Rect::from_min_max(egui::pos2(inner.left(), header.bottom()), inner.max);
        let fields = self.snapshot_fields();
        ui.scope_builder(
            egui::UiBuilder::new()
                .max_rect(body)
                .layout(Layout::top_down(Align::Min)),
            |ui| {
                ui.set_clip_rect(ui.clip_rect().intersect(body));
                egui::ScrollArea::vertical()
                    .id_salt("snapshot_table")
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.spacing_mut().item_spacing.y = 0.;
                        for (key, value) in fields {
                            let (row, _) = ui.allocate_exact_size(
                                Vec2::new(body.width() - 12., 30.),
                                egui::Sense::hover(),
                            );
                            for (cell, text) in [
                                (rect(row.left(), row.top(), split, 30.), key.to_owned()),
                                (
                                    rect(row.left() + split, row.top(), row.width() - split, 30.),
                                    value,
                                ),
                            ] {
                                ui.painter()
                                    .with_clip_rect(cell.intersect(ui.clip_rect()))
                                    .text(
                                        cell.left_center() + Vec2::new(4., 0.),
                                        Align2::LEFT_CENTER,
                                        &text,
                                        FontId::proportional(12.),
                                        TEXT,
                                    );
                                ui.interact(
                                    cell,
                                    ui.id().with((key, cell.left().to_bits())),
                                    egui::Sense::hover(),
                                )
                                .on_hover_text(&text);
                            }
                            ui.painter().line_segment(
                                [row.left_bottom(), row.right_bottom()],
                                Stroke::new(0.5_f32, BORDER.gamma_multiply(0.25)),
                            );
                        }
                    });
            },
        );
    }
    fn toggle_right(&mut self, ctx: &egui::Context, viewport: Rect) {
        if !self.right_collapsed {
            self.right_was_maximized = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
            if self.right_was_maximized {
                ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(false));
            }
        }
        let extra = precision::Columns::RIGHT_WIDTH - 27.;
        let width = viewport.width() + if self.right_collapsed { extra } else { -extra };
        self.right_collapsed = !self.right_collapsed;
        ctx.send_viewport_cmd(egui::ViewportCommand::MinInnerSize(Vec2::new(
            precision::Columns::minimum_width(self.right_collapsed, self.left_collapsed),
            700.,
        )));
        ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(Vec2::new(
            width,
            viewport.height(),
        )));
        if !self.right_collapsed && self.right_was_maximized {
            ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(true));
            self.right_was_maximized = false;
        }
    }
    pub(super) fn draw(&mut self, ctx: &egui::Context) {
        self.geometry.clear();
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(BG))
            .show(ctx, |ui| {
                let viewport = ui.max_rect();
                let minimum =
                    precision::Columns::minimum_width(self.right_collapsed, self.left_collapsed);
                ctx.send_viewport_cmd(egui::ViewportCommand::MinInnerSize(Vec2::new(
                    minimum, 700.,
                )));
                if viewport.width() < minimum - 1. {
                    ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(Vec2::new(
                        minimum,
                        viewport.height(),
                    )));
                }
                let columns =
                    precision::Columns::new(viewport, self.right_collapsed, self.left_collapsed);
                columns.record(&mut self.geometry);
                if !self.left_collapsed {
                    let content = Rect::from_min_max(
                        columns.left.min + Vec2::splat(2.),
                        egui::pos2(columns.left_toggle.left() - 4., columns.left.bottom() - 2.),
                    );
                    let origin = content.min + Vec2::splat(9.);
                    let card_w = content.width() - 18.;
                    self.config_card(ui, rect(origin.x, origin.y, card_w, 128.));
                    let device_area = Rect::from_min_max(
                        egui::pos2(content.left(), origin.y + 136.),
                        content.max,
                    );
                    precision::record(&mut self.geometry, "device_scroll", device_area);
                    ui.scope_builder(
                        egui::UiBuilder::new()
                            .max_rect(device_area)
                            .layout(Layout::top_down(Align::Min)),
                        |ui| {
                            ui.set_clip_rect(ui.clip_rect().intersect(device_area));
                            ui.spacing_mut().scroll = egui::style::ScrollStyle::floating();
                            egui::ScrollArea::vertical()
                                .id_salt("devices-scroll")
                                .auto_shrink([false, false])
                                .max_height(device_area.height())
                                .show(ui, |ui| {
                                    let origin = ui.max_rect().min + Vec2::new(9., 0.);
                                    let mut y = origin.y;
                                    for i in 0..3 {
                                        let h = self.device_height(i);
                                        self.device_card(ui, i, rect(origin.x, y, card_w, h));
                                        y += h + 8.;
                                    }
                                    ui.set_min_size(Vec2::new(device_area.width(), y - origin.y));
                                });
                        },
                    );
                }
                ui.painter().line_segment(
                    [
                        columns.left.right_top() + Vec2::new(4., 0.),
                        columns.left.right_bottom() + Vec2::new(4., 0.),
                    ],
                    Stroke::new(1_f32, BORDER),
                );
                if ui
                    .put(
                        columns.left_toggle,
                        egui::Button::new(if self.left_collapsed { ">" } else { "<" })
                            .min_size(columns.left_toggle.size())
                            .corner_radius(4),
                    )
                    .on_hover_text(if self.left_collapsed {
                        "Show devices"
                    } else {
                        "Hide devices"
                    })
                    .clicked()
                {
                    self.left_collapsed = !self.left_collapsed;
                }
                if ui
                    .put(
                        columns.toggle,
                        egui::Button::new(if self.right_collapsed { "<" } else { ">" })
                            .min_size(columns.toggle.size())
                            .corner_radius(4),
                    )
                    .on_hover_text("Show / hide log and values")
                    .clicked()
                {
                    self.toggle_right(ctx, viewport);
                }
                if !self.right_collapsed {
                    self.details(ui, columns);
                }
                self.measurement(ui, columns.measurement);
                self.toolbar(ui, columns.toolbar);
                self.plots(ui, columns.plots);
            });
        self.advanced_window(ctx);
        self.background_confirmation(ctx);
        if let Some(reason) = self.recovery_reason() {
            let pending = self.controller.stop_status().0;
            egui::Window::new("Device recovery")
                .id(egui::Id::new("device_recovery"))
                .anchor(egui::Align2::CENTER_CENTER, [0., 0.])
                .collapsible(false)
                .resizable(false)
                .show(ctx, |ui| {
                    ui.set_width(420.);
                    ui.colored_label(ORANGE, reason);
                    ui.label("Start, Move and Apply wait for confirmed device state. Check the equipment, then retry Stop or disconnect and reconnect.");
                    if ui.add_enabled(!pending, egui::Button::new("Retry Stop / verify idle")).clicked() {
                        self.controller.stop();
                    }
                });
        }
    }
    fn background_confirmation(&mut self, ctx: &egui::Context) {
        if self.awaiting == 0 || self.awaiting == 3 || self.api_owned() || self.api_cleanup {
            return;
        }
        let bg = self.awaiting == 1;
        egui::Window::new(if bg {
            "Acquire background"
        } else {
            "Restore signal path"
        })
        .id(egui::Id::new("background_confirmation"))
        .collapsible(false)
        .resizable(false)
        .anchor(Align2::CENTER_CENTER, [0., 0.])
        .show(ctx, |ui| {
            ui.set_width(410.);
            ui.label(if bg {
                "Block the excitation beam before acquiring the background."
            } else {
                "Restore the excitation beam and confirm the signal optical path."
            });
            ui.label("The raw BG is preserved. Every PL spectrum is BG-subtracted.");
            ui.horizontal(|ui| {
                if ui
                    .button(if bg {
                        "Beam blocked · Acquire BG"
                    } else {
                        "Signal ready · Acquire first PL"
                    })
                    .clicked()
                {
                    self.awaiting = 0;
                    self.send(Command::Continue);
                }
                if ui.button("Stop").clicked() {
                    self.awaiting = 0;
                    self.controller.stop();
                }
            });
        });
    }
    fn advanced_window(&mut self, ctx: &egui::Context) {
        let Some(panel) = self.advanced else {
            return;
        };
        let mut opened = true;
        let mut browse = false;
        let locked = self.locked();
        let index = match panel {
            Advanced::Spectrometer => 0,
            Advanced::Stage => 1,
            Advanced::Power => 2,
            Advanced::Excitation => 3,
        };
        egui::Window::new(["LightField settings","CONEX CC settings","Power meter settings","Analysis / BG settings"][index]).id(egui::Id::new("advanced_settings")).open(&mut opened).collapsible(false).resizable(false).show(ctx,|ui|{ui.set_width(480.);ui.add_enabled_ui(!self.api_owned()&&((!locked&&(index==0||index>=3||!self.connected[index]))||(index==3&&self.awaiting==3&&!self.analysis_confirm_pending)),|ui|{match panel{
  Advanced::Spectrometer=>{ui.label(&self.config.apparatus.spectrometer_model);ui.label("Read back from LightField:");ui.monospace(serde_json::to_string_pretty(&self.spectrometer_settings).unwrap_or_default());ui.add_enabled_ui(!self.connected[0],|ui|{label_text(ui,"Python executable",&mut self.config.spectrometer.python);label_text(ui,"Bridge script",&mut self.config.spectrometer.bridge_script);ui.checkbox(&mut self.config.spectrometer.visible,"Show LightField");label_number(ui,"Acquisition timeout (s)",&mut self.config.spectrometer.timeout_s,1.);});},
  Advanced::Stage=>{ui.label(format!("Port defaults · X: {} / Y: {}",self.config.apparatus.conex_x_default_port,self.config.apparatus.conex_y_default_port));ui.label("The COM port in Device is the active connection; these defaults do not switch axes.");ui.horizontal(|ui|{ui.selectable_value(&mut self.config.stage.protocol,StageProtocol::RemoteJson,"Remote JSON");ui.selectable_value(&mut self.config.stage.protocol,StageProtocol::LegacyTcp,"Legacy TCP");});label_text(ui,"Token environment variable",&mut self.config.stage.token_env);ui.label("Host / port address the remote PC. COM is the port on that PC.");label_number(ui,"Position tolerance (mm)",&mut self.config.stage.tolerance_mm,0.0001);label_number(ui,"Move timeout (s)",&mut self.config.stage.timeout_s,1.);if self.config.stage.protocol==StageProtocol::LegacyTcp{ui.checkbox(&mut self.config.stage.allow_legacy_initialize,"Allow legacy CHECK initialization");ui.label("CHECK may home or enable the stage. Default connection only queries GET.");ui.label("Legacy MOVE cannot be interrupted; Stop waits for completion / timeout.");}else{ui.label("Remote JSON keeps position monitoring and Stop available during movement.");}},
  Advanced::Power=>{ui.label(format!("Candidate models: {}",self.config.apparatus.power_meter_candidates.join(" / ")));ui.horizontal(|ui|{ui.selectable_value(&mut self.config.power.wavelength_mode,PowerWavelengthMode::Fixed,"Fixed sensor wavelength");ui.selectable_value(&mut self.config.power.wavelength_mode,PowerWavelengthMode::PositionCalibration,"Follow position calibration");});label_number(ui,"Excitation agreement tolerance (nm)",&mut self.config.power.wavelength_tolerance_nm,0.1);ui.label("Follow mode sets and verifies sensor wavelength before each power read. Enter position/wavelength anchors in Analysis settings.");label_text(ui,"Legacy driver folder",&mut self.config.power.driver_path);browse=ui.button("Browse").clicked();ui.label("Folder containing ThorlabsPowerMeter.py and Thorlabs_DotNet_dll.");},
  Advanced::Excitation=>{ui.label(format!("Laser: {}",self.config.apparatus.laser_model));ui.horizontal(|ui|{ui.selectable_value(&mut self.config.excitation_method,ExcitationMethod::SpectrumPeak,"Spectrum peak");ui.selectable_value(&mut self.config.excitation_method,ExcitationMethod::Calibration,"Position calibration");});if self.config.excitation_method==ExcitationMethod::SpectrumPeak{label_number(ui,"Excitation search min (nm)",&mut self.config.excitation_min_nm,0.1);label_number(ui,"Excitation search max (nm)",&mut self.config.excitation_max_nm,0.1);label_number(ui,"Minimum peak SNR",&mut self.config.excitation_min_snr,0.5);ui.label("Separate from the PL analysis peak. Maximum-bin detection; median / MAD screening.");}
if self.config.excitation_method==ExcitationMethod::Calibration||self.config.power.wavelength_mode==PowerWavelengthMode::PositionCalibration{let mut remove=None;egui::Grid::new("excitation_anchors").show(ui,|ui|{ui.label("CONEX (mm)");ui.label("Excitation (nm)");ui.end_row();for(i,a)in self.config.excitation_peaks.iter_mut().enumerate(){ui.add(egui::DragValue::new(&mut a.position_mm).speed(0.001));ui.add(egui::DragValue::new(&mut a.wavelength_nm).speed(0.1));if ui.button("Remove").clicked(){remove=Some(i);}ui.end_row();}});if let Some(i)=remove{self.config.excitation_peaks.remove(i);}
if ui.button("Add anchor").clicked(){self.config.excitation_peaks.push(PeakAnchor{position_mm:self.config.scan.stop_mm,wavelength_nm:720.});}}label_number(ui,"Peak search ±nm",&mut self.analysis_draft.peak_half_width_nm,0.1);
ui.add_enabled_ui(!locked,|ui|{ui.horizontal(|ui|{ui.selectable_value(&mut self.config.background.mode,BackgroundMode::Manual,"Manual BG");ui.selectable_value(&mut self.config.background.mode,BackgroundMode::Automatic,"Automatic shutter");});if self.config.background.mode==BackgroundMode::Automatic{label_text(ui,"MFF shutter serial",&mut self.config.background.shutter_serial);let mut closed=self.config.background.closed_state as f64;let mut open=self.config.background.open_state as f64;label_number(ui,"Beam blocked state",&mut closed,1.);label_number(ui,"Beam open state",&mut open,1.);self.config.background.closed_state=closed.round().clamp(0.,255.)as u8;self.config.background.open_state=open.round().clamp(0.,255.)as u8;label_number(ui,"Shutter timeout (s)",&mut self.config.background.timeout_s,1.);ui.label("Check the actual optical path: state numbers depend on the setup. Automatic mode closes after the run or stop.");}});
}
 }});if let Err(e)=self.config.plan(){ui.colored_label(RED,e);}});
        if browse {
            self.choose(DialogTarget::Driver);
        }
        if !opened {
            self.advanced = None;
        }
    }
}
fn label_text(ui: &mut egui::Ui, label: &str, value: &mut String) {
    ui.label(label);
    ui.add_sized(
        [ui.available_width(), 26.],
        egui::TextEdit::singleline(value),
    );
}
fn label_number(ui: &mut egui::Ui, label: &str, value: &mut f64, step: f64) {
    ui.horizontal(|ui| {
        ui.add_sized([240., 26.], egui::Label::new(label));
        ui.add_sized([160., 26.], egui::DragValue::new(value).speed(step));
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    fn frame(app: &mut PleApp, ctx: &egui::Context, events: Vec<egui::Event>) {
        let _ = ctx.run(
            egui::RawInput {
                screen_rect: Some(rect(0., 0., 1566., 900.)),
                events,
                ..Default::default()
            },
            |ctx| app.draw(ctx),
        );
    }
    fn click(app: &mut PleApp, ctx: &egui::Context, key: &str) {
        let g = app.geometry[key];
        let p = egui::pos2(g[0] + g[2] * 0.5, g[1] + g[3] * 0.5);
        frame(
            app,
            ctx,
            vec![
                egui::Event::PointerMoved(p),
                egui::Event::PointerButton {
                    pos: p,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: Default::default(),
                },
            ],
        );
        frame(
            app,
            ctx,
            vec![egui::Event::PointerButton {
                pos: p,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: Default::default(),
            }],
        );
    }
    #[test]
    fn fixed_shell_keeps_controls_and_four_plots_inside_center() {
        let ctx = egui::Context::default();
        let mut app = PleApp::with_context(&ctx, Launch::default());
        frame(&mut app, &ctx, vec![]);
        let to_rect = |g: [f32; 4]| rect(g[0], g[1], g[2], g[3]);
        let center = to_rect(app.geometry["center_panel"]);
        for key in [
            "tab_ple",
            "scan_min",
            "scan_max",
            "scan_step",
            "pl_peak",
            "run_start",
            "run_stop",
            "plot_current",
            "plot_overlay",
            "plot_excitation",
            "plot_heatmap",
        ] {
            assert!(center.contains_rect(to_rect(app.geometry[key])), "{key}");
        }
        assert_eq!(app.geometry["left_panel"][2], 384.);
        assert_eq!(app.geometry["right_panel"][2], 300.);
        assert_eq!(app.geometry["ple_conex_live"][2], 306.);
        assert_eq!(app.geometry["ple_power_live"][2], 306.);
    }
    #[test]
    fn recovery_blocks_run_and_manual_motion_but_can_be_resolved() {
        let ctx = egui::Context::default();
        let mut app = PleApp::with_context(&ctx, Launch::default());
        app.connected = [true; 3];
        app.api_cleanup_error = Some("Closed state unconfirmed".into());
        app.device_expanded[1] = true;
        frame(&mut app, &ctx, vec![]);
        click(&mut app, &ctx, "run_start");
        assert!(!app.busy);
        click(&mut app, &ctx, "manual_move");
        assert!(!app.manual_moving);
        assert!(!app.locked(), "device reconnect must remain available");
        app.api_cleanup_error = None;
        frame(&mut app, &ctx, vec![]);
        click(&mut app, &ctx, "run_start");
        assert!(app.busy);
    }
    #[test]
    fn device_toggle_exposes_connection_settings_without_moving_other_columns() {
        let ctx = egui::Context::default();
        let mut app = PleApp::with_context(&ctx, Launch::default());
        frame(&mut app, &ctx, vec![]);
        let plots = app.geometry["plot_current"];
        assert!(!app.geometry.contains_key("stage_host"));
        click(&mut app, &ctx, "device_1_toggle");
        frame(&mut app, &ctx, vec![]);
        assert!(app.device_expanded[1]);
        assert!(app.geometry.contains_key("stage_host"));
        assert_eq!(app.geometry["plot_current"], plots);
        assert!(!app.connected.iter().any(|v| *v));
    }
    #[test]
    fn focused_scan_field_cannot_change_with_arrow_keys_while_running() {
        let ctx = egui::Context::default();
        let mut app = PleApp::with_context(&ctx, Launch::default());
        frame(&mut app, &ctx, vec![]);
        click(&mut app, &ctx, "scan_min");
        let original = app.config.scan.start_mm;
        app.busy = true;
        frame(
            &mut app,
            &ctx,
            vec![egui::Event::Key {
                key: egui::Key::ArrowUp,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Default::default(),
            }],
        );
        assert_eq!(app.config.scan.start_mm, original);
    }
    #[test]
    fn review_enables_peak_selection_while_scan_stays_locked() {
        let ctx = egui::Context::default();
        let mut app = PleApp::with_context(&ctx, Launch::default());
        app.busy = true;
        app.awaiting = 3;
        frame(&mut app, &ctx, vec![]);
        click(&mut app, &ctx, "pl_peak");
        let before = app.analysis_draft.peak_nm;
        let scan = app.config.scan.start_mm;
        frame(
            &mut app,
            &ctx,
            vec![egui::Event::Key {
                key: egui::Key::ArrowUp,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Default::default(),
            }],
        );
        assert!((app.analysis_draft.peak_nm - before - 0.1).abs() < 1e-6);
        assert_eq!(app.config.scan.start_mm, scan);
    }
    #[test]
    fn expanded_spectrometer_settings_fit_inside_device_card() {
        let ctx = egui::Context::default();
        let mut app = PleApp::with_context(&ctx, Launch::default());
        app.device_expanded[0] = true;
        frame(&mut app, &ctx, vec![]);
        let g = app.geometry["device_0_group"];
        let card = rect(g[0], g[1], g[2], g[3]);
        for key in [
            "lf_grating",
            "lf_exposure",
            "lf_frames",
            "lf_apply",
            "connect_0",
            "center_summary",
        ] {
            let g = app.geometry[key];
            assert!(card.contains_rect(rect(g[0], g[1], g[2], g[3])), "{key}");
        }
    }
    #[test]
    fn manual_target_controls_fit_and_move_is_locked_during_scan() {
        let ctx = egui::Context::default();
        let mut app = PleApp::with_context(&ctx, Launch::default());
        app.device_expanded[1] = true;
        app.connected[1] = true;
        app.busy = true;
        frame(&mut app, &ctx, vec![]);
        let g = app.geometry["device_1_group"];
        let card = rect(g[0], g[1], g[2], g[3]);
        for key in ["manual_target", "manual_move", "manual_stop"] {
            let g = app.geometry[key];
            assert!(card.contains_rect(rect(g[0], g[1], g[2], g[3])));
        }
        click(&mut app, &ctx, "manual_move");
        assert!(!app.manual_moving);
    }
    #[test]
    fn stop_remains_available_while_configuration_is_locked() {
        let ctx = egui::Context::default();
        let mut app = PleApp::with_context(&ctx, Launch::default());
        app.busy = true;
        app.state = "Acquiring".into();
        frame(&mut app, &ctx, vec![]);
        click(&mut app, &ctx, "run_stop");
        assert!(app.state.starts_with("Stop requested"));
        assert_eq!(app.config.scan.start_mm, 18.);
    }
}
