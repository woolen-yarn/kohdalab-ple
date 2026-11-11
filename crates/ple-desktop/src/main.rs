#![forbid(unsafe_code)]
#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]
mod api;
mod api_ui;
mod backend;
mod engine;
mod file_dialog;
mod font_selection;
mod live_monitor;
mod precision;
mod presentation;
mod remote_gui;
mod remote_stage;
mod shutter_client;
mod ui;
mod visuals;
use eframe::egui::{self, RichText};

use engine::{Command, Controller, Event};
use ple_core::{
    Acquisition, AnalysisConfig, BackgroundMode, Config, ExcitationMethod, HC_EV_NM, Mode,
    PeakAnchor, PowerWavelengthMode, Sample, ShutterBackend, Spectrum, StageProtocol, recording,
};
use std::{
    collections::VecDeque,
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

#[derive(Default)]
struct Launch {
    auto_demo: bool,
    review_demo: bool,
    screenshot: Option<PathBuf>,
    config: Option<PathBuf>,
    energy: bool,
    normalize: bool,
    gui_api: bool,
    check_config: bool,
    api_dir: Option<PathBuf>,
    api_server: Option<remote_gui::Server>,
    loaded_config: Option<Config>,
    startup_error: Option<String>,
}
struct HeatmapCache {
    key: (usize, usize, bool),
    rows: Vec<ple_core::display::HeatRow>,
    low: f64,
    high: f64,
    signal_magnitude: f64,
    raster: Option<presentation::HeatmapRaster>,
}
struct PleApp {
    controller: Controller,
    api: Option<api_ui::Runtime>,
    api_cleanup: bool,
    cleanup_errors: [Option<String>; 3],
    plot_cache: Option<presentation::PlotDataCache>,
    config: Config,
    connected: [bool; 3],
    busy: bool,
    manual_moving: bool,
    spectrometer_configuring: bool,
    power_sensor_wavelength: Option<f64>,
    manual_target_mm: f64,
    state: String,
    samples: Vec<Arc<Sample>>,
    background: Option<Spectrum>,
    preview: Option<Arc<Acquisition>>,
    analysis_draft: AnalysisConfig,
    analysis_confirm_pending: bool,
    spectrometer_settings: serde_json::Value,
    power_hardware: serde_json::Value,
    shutter_state: serde_json::Value,
    logs: VecDeque<String>,
    stage_history: live_monitor::History,
    power_history: live_monitor::History,
    stage_value: Option<f64>,
    power_value: Option<f64>,
    telemetry_epoch: f64,

    energy: bool,
    normalize: bool,
    selected: Option<usize>,
    awaiting: u8,
    run_dir: String,
    config_path: String,
    load_path: String,
    message: Option<String>,
    started: Option<Instant>,
    auto_demo: bool,
    review_demo: bool,
    auto_started: bool,
    screenshot: Option<PathBuf>,
    screenshot_requested: bool,
    closing: bool,
    heatmap_cache: Option<HeatmapCache>,
    geometry: precision::Geometry,
    left_collapsed: bool,
    rails_initialized: bool,
    right_collapsed: bool,
    right_was_maximized: bool,
    device_expanded: [bool; 3],
    pending_connections: [bool; 3],
    advanced: Option<ui::Advanced>,
    dialog: Option<ui::PendingDialog>,
    plot_epoch: u64,
    finished_elapsed: Option<f64>,
    run_count: usize,
    first_saved: Option<Instant>,
    last_saved: Option<Instant>,
}
impl PleApp {
    fn new(cc: &eframe::CreationContext<'_>, launch: Launch) -> Self {
        Self::with_context(&cc.egui_ctx, launch)
    }
    fn with_context(ctx: &egui::Context, mut launch: Launch) -> Self {
        ctx.set_visuals(egui::Visuals::dark());
        precision::configure(ctx);
        let mut config = Config::default();
        if let Ok(paths) = kohdalab_control_protocol::paths::AppPaths::for_app("ple") {
            config.output_dir = paths.results_dir().to_string_lossy().into();
        }
        let mut message = launch.startup_error.take();
        if let Some(c) = launch.loaded_config.take() {
            config = c;
        } else if message.is_none()
            && let Some(p) = &launch.config
        {
            match read_profile(p) {
                Ok(c) => config = c,
                Err(e) => message = Some(e),
            }
        }
        if launch.auto_demo {
            config.mode = Mode::Demo;
            config.output_dir = "output/demo".into();
            config.scan.settle_s = 0.08;
        }
        let config_path = if let Some(path) = &launch.config {
            path.to_string_lossy().into_owned()
        } else {
            default_config_path().to_string_lossy().into_owned()
        };
        let analysis_draft = config.initial_analysis();
        let controller = Controller::spawn();
        let mut app = Self {
            controller,
            api: launch.api_server.map(api_ui::Runtime::new),
            api_cleanup: false,
            cleanup_errors: Default::default(),
            plot_cache: None,
            config,
            connected: [false; 3],
            busy: false,
            manual_moving: false,
            spectrometer_configuring: false,
            power_sensor_wavelength: None,
            manual_target_mm: 18.,
            state: "Idle".into(),
            samples: Vec::new(),
            background: None,
            preview: None,
            analysis_draft,
            analysis_confirm_pending: false,
            spectrometer_settings: serde_json::Value::Null,
            power_hardware: serde_json::Value::Null,
            shutter_state: serde_json::Value::Null,
            logs: VecDeque::new(),
            stage_history: Default::default(),
            power_history: Default::default(),
            stage_value: None,
            power_value: None,
            telemetry_epoch: recording::unix_s(),

            energy: launch.energy,
            normalize: launch.normalize,
            selected: None,
            awaiting: 0,
            run_dir: String::new(),
            config_path,
            load_path: String::new(),
            message,
            started: None,
            auto_demo: launch.auto_demo,
            review_demo: launch.review_demo,
            auto_started: false,
            screenshot: launch.screenshot,
            screenshot_requested: false,
            closing: false,
            heatmap_cache: None,
            geometry: Default::default(),
            left_collapsed: false,
            rails_initialized: false,
            right_collapsed: false,
            right_was_maximized: false,
            device_expanded: [false; 3],
            pending_connections: [false; 3],
            advanced: None,
            dialog: None,
            plot_epoch: 0,
            finished_elapsed: None,
            run_count: 0,
            first_saved: None,
            last_saved: None,
        };
        if app.auto_demo {
            app.pending_connections = [true; 3];
            for device in ["spectrometer", "stage", "power"] {
                let _ = app
                    .controller
                    .commands
                    .send(Command::Connect(device, app.config.clone()));
            }
        }
        app
    }
    fn log(&mut self, text: impl Into<String>) {
        let elapsed = recording::unix_s() - self.telemetry_epoch;
        self.logs
            .push_back(format!("{:07.1}s  {}", elapsed, text.into()));
        while self.logs.len() > 500 {
            self.logs.pop_front();
        }
    }
    fn send(&mut self, command: Command) {
        if let Err(e) = self.controller.commands.send(command) {
            self.message = Some(e.to_string());
        }
    }
    fn start(&mut self) {
        self.start_for(None);
    }
    fn cleanup_error(&self) -> Option<String> {
        self.cleanup_errors.iter().flatten().next().cloned()
    }
    fn recovery_reason(&self) -> Option<String> {
        let (pending, error) = self.controller.stop_status();
        if pending {
            Some("Waiting for Stop and device readback".into())
        } else {
            error.or_else(|| self.cleanup_error())
        }
    }
    fn start_for(&mut self, external_id: Option<String>) {
        if let Some(reason) = self.recovery_reason() {
            self.message = Some(format!("Confirm device recovery before starting: {reason}"));
            return;
        }
        match self.config.plan() {
            Ok(_) => {
                self.busy = true;
                self.awaiting = 0;
                self.samples.clear();
                self.background = None;
                self.preview = None;
                self.analysis_draft = self.config.initial_analysis();
                self.analysis_confirm_pending = false;
                self.selected = None;
                self.run_dir.clear();
                self.started = Some(Instant::now());
                self.finished_elapsed = None;
                self.run_count = self.config.plan().map(|p| p.acquisition_count).unwrap_or(0);
                self.plot_epoch = self.plot_epoch.wrapping_add(1);
                self.heatmap_cache = None;
                self.first_saved = None;
                self.last_saved = None;
                self.message = None;
                let submitted = match external_id {
                    Some(id) => self.controller.begin_external(self.config.clone(), id),
                    None => self.controller.begin(self.config.clone()),
                };
                if let Err(e) = submitted {
                    self.message = Some(e);
                    self.busy = false;
                }
            }
            Err(e) => self.message = Some(e),
        }
    }
    fn confirm_analysis(&mut self) {
        let Some(a) = &self.preview else {
            return;
        };
        if let Err(e) = self.analysis_draft.validate(&a.spectrum.wavelength_nm) {
            self.message = Some(e);
            return;
        }
        let mut c = self.config.clone();
        c.analysis = Some(self.analysis_draft.clone());
        self.analysis_confirm_pending = true;
        self.message = None;
        self.send(Command::SelectAnalysis(Box::new(c)));
    }
    fn events(&mut self) {
        while let Ok(event) = self.controller.events.try_recv() {
            let note = api_ui::Note::from_event(&event);
            match event {
                Event::Log(s) => self.log(s),
                Event::Device { device, connected } => {
                    let i = match device {
                        "spectrometer" => 0,
                        "stage" => 1,
                        _ => 2,
                    };
                    let previous = self.connected[i];
                    self.connected[i] = connected;
                    if i == 0 && !connected {
                        self.spectrometer_settings = serde_json::Value::Null;
                        self.shutter_state = serde_json::Value::Null;
                    }
                    self.pending_connections[i] = false;
                    if i == 1 {
                        self.stage_history.connection_changed(previous, connected);
                        if !connected {
                            self.stage_value = None;
                        }
                    }
                    if i == 2 {
                        self.power_history.connection_changed(previous, connected);
                        if !connected {
                            self.power_hardware = serde_json::Value::Null;
                            self.power_value = None;
                            self.power_sensor_wavelength = None;
                        }
                    }
                }
                Event::Telemetry {
                    device,
                    value,
                    timestamp,
                } => {
                    let index = if device == "stage" { 1 } else { 2 };
                    if !self.connected[index] {
                        continue;
                    }
                    let origin = if self.config.mode == Mode::Demo {
                        live_monitor::Origin::Demo
                    } else {
                        live_monitor::Origin::Hardware
                    };
                    if device == "stage" {
                        self.stage_value = Some(value);
                        self.stage_history.push(value, origin);
                    } else {
                        self.power_value = Some(value);
                        self.power_history.push(value, origin);
                    }
                    let _ = timestamp;
                }

                Event::State(s) => {
                    self.log(&s);
                    self.state = s;
                }
                Event::ManualMoveFinished(result) => {
                    self.manual_moving = false;
                    match result {
                        Ok(p) => {
                            self.state = format!("CONEX ready at {p:.6} mm");
                            self.log(self.state.clone());
                        }
                        Err(e) => {
                            self.state = "Manual move stopped / failed".into();
                            self.log(&e);
                            self.message = Some(e);
                        }
                    }
                }
                Event::AwaitBackground => {
                    self.awaiting = 1;
                    if self.auto_demo {
                        self.send(Command::Continue);
                        self.awaiting = 0;
                    }
                }
                Event::AwaitSignal => {
                    self.awaiting = 2;
                    if self.auto_demo {
                        self.send(Command::Continue);
                        self.awaiting = 0;
                    }
                }
                Event::Preview(a) => {
                    self.preview = Some(a);
                    self.awaiting = 3;
                    self.analysis_confirm_pending = false;
                    self.plot_epoch = self.plot_epoch.wrapping_add(1);
                    if self.auto_demo && !self.review_demo {
                        self.confirm_analysis();
                    }
                }
                Event::AnalysisRejected(e) => {
                    self.analysis_confirm_pending = false;
                    self.message = Some(e);
                }
                Event::AnalysisAccepted(c) => {
                    self.message = None;
                    self.config = *c;
                    self.analysis_draft = self.config.initial_analysis();
                    self.awaiting = 0;
                    self.analysis_confirm_pending = false;
                }
                Event::SpectrometerSettings(v) => {
                    if self.config.spectrometer.grating.is_empty()
                        && let Some(grating) = v["grating"].as_str()
                    {
                        self.config.spectrometer.grating = grating.into();
                    }
                    self.spectrometer_settings = v;
                }
                Event::PowerHardware(v) => {
                    if self.power_hardware != v {
                        self.log("Power meter / sensor identification read back");
                    }
                    self.power_hardware = v;
                }
                Event::ShutterState(state) => self.shutter_state = state,
                Event::PowerWavelength(nm) => self.power_sensor_wavelength = Some(nm),
                Event::SpectrometerConfigured(result) => {
                    self.spectrometer_configuring = false;
                    match result {
                        Ok(v) => {
                            self.spectrometer_settings = v;
                            self.message = None;
                            self.cleanup_errors[engine::CleanupCause::SpectrometerStop as usize] =
                                None;
                            self.log("LightField settings applied and read back");
                        }
                        Err(e) => {
                            self.log(&e);
                            self.cleanup_errors[engine::CleanupCause::SpectrometerStop as usize] =
                                Some(format!("Spectrometer configuration was not confirmed: {e}"));
                            self.message = Some(e);
                        }
                    }
                }
                Event::Cleanup { cause, result } => {
                    self.cleanup_errors[cause as usize] = result.err();
                }
                Event::Background(bg) => {
                    self.log(format!("BG saved: {} bins", bg.counts.len()));
                    self.background = Some(bg);
                }
                Event::Sample(s) => {
                    let now = Instant::now();
                    if self.first_saved.is_none() {
                        self.first_saved = Some(now);
                    }
                    self.last_saved = Some(now);
                    self.preview = None;
                    self.samples.push(s);
                }
                Event::RunCreated(path) => {
                    self.load_path = path.clone();
                    self.run_dir = path;
                }
                Event::Finished { state, error } => {
                    self.busy = false;
                    self.finished_elapsed = self.started.map(|t| t.elapsed().as_secs_f64());
                    self.awaiting = 0;
                    self.state = state.label().into();
                    self.log(state.label());
                    if let Some(e) = error {
                        self.log(&e);
                        self.message = Some(e);
                    }
                }
            }
            self.api_observe(note);
        }
    }
}
fn default_config_path() -> PathBuf {
    kohdalab_control_protocol::paths::AppPaths::for_app("ple")
        .map(|paths| paths.config_file())
        .unwrap_or_else(|_| std::env::temp_dir().join(".kohdalab/ple/config/config.json"))
}

/// Create the per-user profile once on a normal GUI launch.  A hard link from
/// a fully synced temporary file provides no-clobber creation even if another
/// process starts at the same time; an existing profile is never replaced.
fn create_default_profile(path: &std::path::Path) -> Result<(), String> {
    if path.try_exists().map_err(|e| e.to_string())? {
        return Ok(());
    }
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(std::path::Path::new("."));
    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let app_root = path
        .parent()
        .and_then(|directory| directory.parent())
        .ok_or("Default configuration requires an application config directory")?;
    let config = Config {
        output_dir: app_root.join("results").display().to_string(),
        ..Default::default()
    };
    config.plan()?;
    let mut value = serde_json::to_value(config).map_err(|e| e.to_string())?;
    value["application"] = serde_json::json!("kohdalab-ple");
    let bytes = serde_json::to_vec_pretty(&value).map_err(|e| e.to_string())?;
    use std::io::Write;
    let temporary = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
    let mut file = temporary.reopen().map_err(|e| e.to_string())?;
    file.write_all(&bytes).map_err(|e| e.to_string())?;
    file.sync_all().map_err(|e| e.to_string())?;
    match temporary.persist_noclobber(path) {
        Ok(_) => Ok(()),
        Err(e) if e.error.kind() == std::io::ErrorKind::AlreadyExists => Ok(()),
        Err(e) => Err(e.error.to_string()),
    }
}

fn read_profile(path: &std::path::Path) -> Result<Config, String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    let value: serde_json::Value = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    if value.get("scan").is_none() || value.get("spectrometer").is_none() {
        return Err("Not a PLE configuration: scan and spectrometer are required".into());
    }
    parse_profile(value)
}
fn parse_profile(mut value: serde_json::Value) -> Result<Config, String> {
    let object = value
        .as_object_mut()
        .ok_or("Configuration must be a JSON object")?;
    if let Some(application) = object.remove("application")
        && application.as_str() != Some("kohdalab-ple")
    {
        return Err("This configuration belongs to another application".into());
    }
    let config: Config = serde_json::from_value(value).map_err(|e| e.to_string())?;
    config.plan()?;
    Ok(config)
}
fn save_profile(path: &std::path::Path, config: &Config) -> Result<(), String> {
    if path.try_exists().map_err(|e| e.to_string())? {
        read_profile(path)?;
    }
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    config.plan()?;
    let mut value = serde_json::to_value(config).map_err(|e| e.to_string())?;
    value["application"] = serde_json::json!("kohdalab-ple");
    use std::io::Write;
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(std::path::Path::new("."));
    let mut temporary = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
    temporary
        .write_all(&serde_json::to_vec_pretty(&value).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    temporary.as_file().sync_all().map_err(|e| e.to_string())?;
    temporary.persist(path).map_err(|e| e.to_string())?;
    Ok(())
}
impl eframe::App for PleApp {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.events();
        self.poll_api(ctx);
        if ctx.input(|i| i.viewport().close_requested())
            && (self.busy
                || self.manual_moving
                || self.spectrometer_configuring
                || self.pending_connections.iter().any(|x| *x))
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.closing = true;
            self.controller.stop();
            self.state = "Closing: waiting for current device operation".into();
        }
        if self.closing
            && !self.busy
            && !self.manual_moving
            && !self.spectrometer_configuring
            && !self.pending_connections.iter().any(|x| *x)
            && !self.controller.stop_status().0
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        if self.auto_demo && !self.auto_started && self.connected.iter().all(|x| *x) {
            self.auto_started = true;
            self.start();
        }
        self.poll_dialog();
        ctx.request_repaint_after(Duration::from_millis(
            if self.connected.iter().any(|x| *x) || self.busy {
                16
            } else {
                75
            },
        ));
    }

    fn ui(&mut self, root: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = &root.ctx().clone();
        self.draw(root);
        if self.screenshot.is_some()
            && self.auto_started
            && ((!self.busy && !self.samples.is_empty())
                || (self.review_demo && self.awaiting == 3))
            && !self.screenshot_requested
        {
            self.screenshot_requested = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
        }
        let capture = ctx.input(|i| {
            i.events.iter().find_map(|e| {
                if let egui::Event::Screenshot { image, .. } = e {
                    Some(image.clone())
                } else {
                    None
                }
            })
        });
        if let Some(image) = capture
            && let Some(path) = &self.screenshot
        {
            let bytes: Vec<u8> = image.pixels.iter().flat_map(|c| c.to_array()).collect();
            let result = image::save_buffer(
                path,
                &bytes,
                image.width() as u32,
                image.height() as u32,
                image::ColorType::Rgba8,
            );
            if let Err(e) = result {
                eprintln!("Screenshot: {e}");
            }
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        ctx.request_repaint_after(Duration::from_millis(
            if self.connected.iter().any(|x| *x) || self.busy {
                16
            } else {
                75
            },
        ));
    }
}
fn required_arg(args: &mut impl Iterator<Item = String>, option: &str) -> eframe::Result<String> {
    args.next().ok_or_else(|| {
        eframe::Error::AppCreation(Box::new(std::io::Error::other(format!(
            "{option} needs a value"
        ))))
    })
}
fn main() -> eframe::Result {
    let mut launch = Launch {
        gui_api: true,
        ..Default::default()
    };
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--auto-demo" => launch.auto_demo = true,
            "--review-demo" => {
                launch.auto_demo = true;
                launch.review_demo = true;
            }
            "--check-config" => launch.check_config = true,
            "--gui-api" => launch.gui_api = true,
            "--no-gui-api" => launch.gui_api = false,
            "--api-dir" => {
                launch.api_dir = Some(PathBuf::from(required_arg(&mut args, "--api-dir")?))
            }
            "--energy" => launch.energy = true,
            "--power-normalize" => launch.normalize = true,
            "--screenshot" => {
                launch.screenshot = Some(PathBuf::from(required_arg(&mut args, "--screenshot")?))
            }
            "--config" => launch.config = Some(PathBuf::from(required_arg(&mut args, "--config")?)),
            "--version" => {
                println!("kohdalab-ple {}", env!("CARGO_PKG_VERSION"));
                return Ok(());
            }
            "--help" => {
                println!(
                    "kohdalab-ple [--config profile.json] [--check-config] [--version] [--gui-api | --no-gui-api] [--api-dir directory] [--auto-demo] [--energy] [--power-normalize] [--screenshot image.png]\nHardware connections and motion always require explicit actions. --auto-demo is device-free."
                );
                return Ok(());
            }
            _ => {
                return Err(eframe::Error::AppCreation(Box::new(std::io::Error::other(
                    format!("Unknown argument: {a}; use --help"),
                ))));
            }
        }
    }
    let implicit_default_profile = launch.config.is_none()
        && !launch.auto_demo
        && !launch.check_config
        && launch.screenshot.is_none();
    if implicit_default_profile {
        let path = default_config_path();
        launch.config = Some(path.clone());
        match path.try_exists() {
            Ok(true) => {}
            Ok(false) => {
                if let Err(error) = create_default_profile(&path) {
                    launch.startup_error = Some(format!(
                        "Cannot create default profile {}: {error}",
                        path.display()
                    ));
                }
            }
            Err(error) => {
                launch.startup_error = Some(format!(
                    "Cannot access default profile {}: {error}",
                    path.display()
                ));
            }
        }
    }
    if let Some(path) = &launch.config {
        let checked = read_profile(path);
        match checked {
            Ok(c) => launch.loaded_config = Some(c),
            Err(e) => {
                if implicit_default_profile {
                    if launch.startup_error.is_none() {
                        launch.startup_error =
                            Some(format!("Invalid default profile {}: {e}", path.display()));
                    }
                } else {
                    return Err(eframe::Error::AppCreation(Box::new(std::io::Error::other(
                        format!("Invalid explicit profile {}: {e}", path.display()),
                    ))));
                }
            }
        }
    }
    if launch.check_config {
        let c = launch.loaded_config.as_ref().ok_or_else(|| {
            eframe::Error::AppCreation(Box::new(std::io::Error::other(
                "--check-config requires --config FILE",
            )))
        })?;
        println!(
            "{}",
            serde_json::json!({"config_valid":true,"application_version":env!("CARGO_PKG_VERSION"),"mode":c.mode,"plan":c.plan().map_err(|e|eframe::Error::AppCreation(Box::new(std::io::Error::other(e))))?})
        );
        return Ok(());
    }
    if launch.gui_api {
        let directory = match launch.api_dir.take() {
            Some(p) => p,
            None => kohdalab_control_protocol::paths::AppPaths::for_app("ple")
                .map(|paths| paths.run_dir())
                .map_err(|e| eframe::Error::AppCreation(Box::new(std::io::Error::other(e))))?,
        };
        let server = remote_gui::Server::start(directory)
            .map_err(|e| eframe::Error::AppCreation(Box::new(std::io::Error::other(e))))?;
        eprintln!("PLE GUI API endpoint: {}", server.descriptor.display());
        launch.api_server = Some(server);
    }
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1566., 900.])
            .with_min_inner_size([1566., 700.]),
        ..Default::default()
    };
    eframe::run_native(
        "KohdaLab PLE",
        options,
        Box::new(move |cc| Ok(Box::new(PleApp::new(cc, launch)))),
    )
}

#[cfg(test)]
mod config_storage_tests {
    use super::*;
    #[test]
    fn hidden_gui_logic_receives_motion_completion_without_drawing() {
        use eframe::App as _;
        let ctx = egui::Context::default();
        let mut app = PleApp::with_context(&ctx, Launch::default());
        let (sender, receiver) = std::sync::mpsc::channel();
        app.controller.events = receiver;
        app.manual_moving = true;
        sender.send(Event::ManualMoveFinished(Ok(12.0))).unwrap();
        app.logic(&ctx, &mut eframe::Frame::_new_kittest());
        assert!(!app.manual_moving);
        assert_eq!(app.state, "CONEX ready at 12.000000 mm");
    }

    #[test]
    fn native_profile_round_trip_and_foreign_file_protection() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("ple/config/config.json");
        save_profile(&path, &Config::default()).unwrap();
        let value: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(value["application"], "kohdalab-ple");
        assert!(read_profile(&path).is_ok());
        for bytes in [
            br#"{"application":"kohdalab-iv"}"#.as_slice(),
            b"{}",
            b"broken",
        ] {
            std::fs::write(&path, bytes).unwrap();
            assert!(read_profile(&path).is_err());
            assert!(save_profile(&path, &Config::default()).is_err());
            assert_eq!(std::fs::read(&path).unwrap(), bytes);
        }
    }
    #[test]
    fn default_path_has_no_preview_name() {
        assert!(default_config_path().ends_with(".kohdalab/ple/config/config.json"));
    }

    #[test]
    fn first_launch_creates_a_valid_profile_without_replacing_existing_bytes() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("ple/config/config.json");
        create_default_profile(&path).unwrap();
        assert!(path.is_file());
        let first = std::fs::read(&path).unwrap();
        assert_eq!(
            PathBuf::from(read_profile(&path).unwrap().output_dir),
            directory.path().join("ple").join("results")
        );
        create_default_profile(&path).unwrap();
        assert_eq!(first, std::fs::read(&path).unwrap());
    }

    #[test]
    fn first_launch_preserves_foreign_or_invalid_profile_for_error_reporting() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("ple/config/config.json");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let bytes = br#"{"application":"kohdalab-iv"}"#;
        std::fs::write(&path, bytes).unwrap();
        create_default_profile(&path).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        assert!(read_profile(&path).is_err());
    }

    #[test]
    fn concurrent_first_launches_publish_one_complete_json() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("ple/config/config.json");
        let mut workers = Vec::new();
        for _ in 0..8 {
            let path = path.clone();
            workers.push(std::thread::spawn(move || create_default_profile(&path)));
        }
        for worker in workers {
            assert!(worker.join().unwrap().is_ok());
        }
        assert!(read_profile(&path).is_ok());
    }

    #[test]
    fn unusable_profile_parent_is_reported_without_partial_file() {
        let directory = tempfile::tempdir().unwrap();
        let parent = directory.path().join("ple");
        std::fs::write(&parent, b"occupied").unwrap();
        let path = parent.join("config.json");
        let error = create_default_profile(&path).unwrap_err();
        assert!(!error.is_empty());
        assert!(!path.exists());
    }

    #[test]
    fn future_profile_is_preserved_for_startup_error_reporting() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("ple/config/config.json");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let bytes = br#"{"application":"kohdalab-ple","schema_version":999}"#;
        std::fs::write(&path, bytes).unwrap();
        create_default_profile(&path).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        assert!(read_profile(&path).is_err());
    }

    #[test]
    fn implicit_startup_error_is_kept_for_the_gui_message_area() {
        let context = egui::Context::default();
        let launch = Launch {
            startup_error: Some("Invalid default profile; repair it in Settings".into()),
            ..Default::default()
        };
        let app = PleApp::with_context(&context, launch);
        assert_eq!(
            app.message.as_deref(),
            Some("Invalid default profile; repair it in Settings")
        );
        assert!(app.connected.iter().all(|connected| !connected));
    }
}
