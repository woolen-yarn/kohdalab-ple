//! Device boundaries. No GUI thread performs instrument I/O.
use crate::{engine::Event, remote_stage::RemoteStage};
use ple_core::{Config, Mode, Spectrum, StageConfig, StageProtocol};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    io::{BufRead, BufReader, Read, Write},
    net::{TcpStream, ToSocketAddrs},
    path::{Path, PathBuf},
    process::{Child, ChildStdin, Command, Stdio},
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};
type Reply = mpsc::Sender<Result<Value, String>>;
struct RpcInner {
    input: Mutex<ChildStdin>,
    pending: Mutex<HashMap<u64, Reply>>,
    next: AtomicU64,
    child: Mutex<Child>,
    events: mpsc::Sender<Event>,
    stop_barrier: (Mutex<StopBarrier>, Condvar),
}
impl Drop for RpcInner {
    fn drop(&mut self) {
        if let Ok(child) = self.child.get_mut() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}
#[derive(Clone)]
pub struct RpcClient(Arc<RpcInner>);
impl RpcClient {
    pub fn start(c: &Config, events: mpsc::Sender<Event>) -> Result<Self, String> {
        let script = Path::new(&c.spectrometer.bridge_script);
        let script: PathBuf = if script.is_absolute() {
            script.into()
        } else {
            resolve_bridge_script(script)
        };
        let mut command = Command::new(&c.spectrometer.python);
        command
            .arg("-u")
            .arg(script)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000000);
        }
        let mut child = command
            .spawn()
            .map_err(|e| format!("Cannot start Python bridge: {e}"))?;
        let input = child.stdin.take().ok_or("No bridge stdin")?;
        let stdout = child.stdout.take().ok_or("No bridge stdout")?;
        let stderr = child.stderr.take().ok_or("No bridge stderr")?;
        let inner = Arc::new(RpcInner {
            input: Mutex::new(input),
            pending: Mutex::new(HashMap::new()),
            next: AtomicU64::new(1),
            child: Mutex::new(child),
            events: events.clone(),
            stop_barrier: (Mutex::new(StopBarrier::default()), Condvar::new()),
        });
        let weak = Arc::downgrade(&inner);
        thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            loop {
                let line = read_bounded_line(&mut reader, 64 * 1024 * 1024);
                if matches!(line, Ok(None)) {
                    break;
                }
                let Some(inner) = weak.upgrade() else {
                    return;
                };
                let response = line.and_then(|s| {
                    serde_json::from_slice::<Value>(&s.unwrap())
                        .map_err(|e| format!("Invalid bridge JSON: {e}"))
                });
                match response {
                    Ok(v) => {
                        if let Some(id) = v["id"].as_u64()
                            && let Some(reply) = inner.pending.lock().unwrap().remove(&id)
                        {
                            let result = if v["ok"].as_bool() == Some(true) {
                                Ok(v["result"].clone())
                            } else {
                                Err(v
                                    .get("error")
                                    .map(|v| {
                                        if let Some(s) = v.as_str() {
                                            s.into()
                                        } else {
                                            v.to_string()
                                        }
                                    })
                                    .unwrap_or("Bridge operation failed".into()))
                            };
                            let _ = reply.send(result);
                        }
                    }
                    Err(e) => {
                        for (_, reply) in inner.pending.lock().unwrap().drain() {
                            let _ = reply.send(Err(e.clone()));
                        }
                        break;
                    }
                }
            }
            if let Some(inner) = weak.upgrade() {
                for (_, reply) in inner.pending.lock().unwrap().drain() {
                    let _ = reply.send(Err(
                        "Python bridge closed; hardware state may be unknown".into()
                    ));
                }
            }
        });
        thread::spawn(move || {
            for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                let _ = events.send(Event::Log(format!("Bridge: {line}")));
            }
        });
        Ok(Self(inner))
    }
    pub fn request(&self, op: &str, args: Value, timeout: Duration) -> Result<Value, String> {
        if matches!(
            op,
            "configure_spectrometer"
                | "connect_spectrometer"
                | "acquire"
                | "shutter_connect"
                | "shutter_set"
                | "shutdown"
                | "disconnect_spectrometer"
        ) {
            let recovery_operation = matches!(op, "shutdown" | "disconnect_spectrometer")
                || (op == "shutter_set" && args["closed"].as_bool() == Some(true));
            self.wait_for_stop(!recovery_operation)?;
        }
        let id = self.0.next.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = mpsc::channel();
        self.0.pending.lock().unwrap().insert(id, tx);
        let send = (|| {
            let mut input = self
                .0
                .input
                .lock()
                .map_err(|_| "Bridge input lock failed")?;
            serde_json::to_writer(
                &mut *input,
                &json!({"revision":1,"id":id,"op":op,"args":args}),
            )
            .map_err(|e| e.to_string())?;
            input
                .write_all(b"\n")
                .and_then(|_| input.flush())
                .map_err(|e| e.to_string())
        })();
        if let Err(e) = send {
            self.0.pending.lock().unwrap().remove(&id);
            if Self::requires_idle_after_failure(op) {
                self.stop();
            }
            return Err(e);
        }
        let result = match rx.recv_timeout(timeout) {
            Ok(r) => r,
            Err(_) => {
                self.0.pending.lock().unwrap().remove(&id);
                Err(format!(
                    "{op} timed out; request was not retried; device state may be unknown"
                ))
            }
        };
        // A failed reply or a lost acknowledgement does not prove that the
        // SDK operation stopped. Reserve the Stop barrier before returning.
        if result.is_err() && Self::requires_idle_after_failure(op) {
            self.stop();
        }
        result
    }
    fn requires_idle_after_failure(op: &str) -> bool {
        matches!(
            op,
            "configure_spectrometer"
                | "connect_spectrometer"
                | "acquire"
                | "shutter_connect"
                | "shutter_set"
        )
    }
    pub fn wait_for_stop(&self, require_confirmed: bool) -> Result<(), String> {
        let (lock, ready) = &self.0.stop_barrier;
        let state = lock.lock().unwrap();
        let (state, _) = ready
            .wait_timeout_while(state, Duration::from_secs(30), |s| s.pending)
            .map_err(|_| "Bridge stop synchronization failed")?;
        if state.pending {
            return Err("Prior bridge Stop is pending; operation was not started".into());
        }
        if require_confirmed && let Some(e) = &state.unconfirmed {
            return Err(format!(
                "Prior bridge Stop unconfirmed: {e}; retry Stop or disconnect after checking the equipment"
            ));
        }
        Ok(())
    }
    pub fn stop_status(&self) -> (bool, Option<String>) {
        let s = self.0.stop_barrier.0.lock().unwrap();
        (s.pending, s.unconfirmed.clone())
    }
    pub fn stop(&self) {
        let mut state = self.0.stop_barrier.0.lock().unwrap();
        if state.pending {
            return;
        }
        state.pending = true;
        drop(state);
        let client = self.clone();
        thread::spawn(move || {
            let result = client
                .request("stop", json!({}), Duration::from_secs(5))
                .and_then(|_| client.request("ensure_idle", json!({}), Duration::from_secs(20)))
                .and_then(|v| {
                    if v["idle_confirmed"].as_bool() == Some(true) {
                        Ok(v)
                    } else {
                        Err("LightField did not confirm idle".into())
                    }
                });
            let (lock, ready) = &client.0.stop_barrier;
            let mut state = lock.lock().unwrap();
            state.pending = false;
            state.unconfirmed = result.as_ref().err().cloned();
            let _ = client.0.events.send(Event::Cleanup {
                confirmed: result.is_ok(),
                error: result.err(),
            });
            drop(state);
            ready.notify_all();
        });
    }
}
fn read_bounded_line(reader: &mut impl BufRead, limit: usize) -> Result<Option<Vec<u8>>, String> {
    let mut bytes = Vec::new();
    let n = reader
        .take(limit as u64 + 1)
        .read_until(b'\n', &mut bytes)
        .map_err(|e| e.to_string())?;
    if n == 0 {
        return Ok(None);
    }
    if n > limit || bytes.last() != Some(&b'\n') {
        return Err("Bridge reply exceeded limit or was not a complete JSON line".into());
    }
    Ok(Some(bytes))
}
fn timeout(s: f64) -> Duration {
    Duration::from_secs_f64(s.clamp(0.1, 86400.))
}
pub struct Spectrometer {
    rpc: Option<RpcClient>,
    mode: Mode,
    config: Config,
    acquisitions: usize,
    actual_settings: Value,
    shutter_attached: bool,
}
impl Drop for Spectrometer {
    fn drop(&mut self) {
        if let Some(rpc) = &self.rpc {
            let _ = rpc.request("shutdown", json!({}), Duration::from_secs(5));
        }
    }
}
impl Spectrometer {
    pub fn connect(
        c: &Config,
        events: mpsc::Sender<Event>,
        stop_handle: &Mutex<Option<RpcClient>>,
    ) -> Result<Self, String> {
        let mut actual_settings = json!({"demo":true,"exposure_ms":c.spectrometer.exposure_ms,"frames_combined":c.spectrometer.frames_combined,"center_nm":c.spectrometer.center_nm,"grating":c.spectrometer.grating});
        let rpc = if c.mode == Mode::Hardware {
            let client = RpcClient::start(c, events)?;
            // Retain ownership even if the connect acknowledgement fails.
            *stop_handle.lock().unwrap() = Some(client.clone());
            let state = client.request(
                "connect_spectrometer",
                serde_json::to_value(&c.spectrometer).map_err(|e| e.to_string())?,
                Duration::from_secs(90),
            )?;
            actual_settings = state["settings"].clone();
            Some(client)
        } else {
            None
        };
        Ok(Self {
            rpc,
            mode: c.mode,
            config: c.clone(),
            acquisitions: 0,
            actual_settings,
            shutter_attached: false,
        })
    }
    pub fn disconnect(&mut self) -> Result<(), String> {
        if self.shutter_attached {
            self.set_shutter(true)?;
        }
        if let Some(rpc) = &self.rpc {
            let result = rpc.request("shutdown", json!({}), Duration::from_secs(25))?;
            if result["shutdown"].as_bool() != Some(true) {
                return Err("Bridge shutdown was not confirmed".into());
            }
        }
        self.shutter_attached = false;
        self.rpc = None;
        Ok(())
    }
    pub fn cancel_handle(&self) -> Option<RpcClient> {
        self.rpc.clone()
    }
    pub fn configure(&mut self, c: &Config) -> Result<(), String> {
        if self.mode != c.mode {
            return Err("Reconnect all devices after changing mode".into());
        }
        if let Some(rpc) = &self.rpc {
            self.actual_settings = rpc.request(
                "configure_spectrometer",
                serde_json::to_value(&c.spectrometer).map_err(|e| e.to_string())?,
                Duration::from_secs(30),
            )?;
        }
        if self.rpc.is_none() {
            self.actual_settings = json!({"demo":true,"exposure_ms":c.spectrometer.exposure_ms,"frames_combined":c.spectrometer.frames_combined,"center_nm":c.spectrometer.center_nm,"grating":c.spectrometer.grating});
        }
        self.config = c.clone();
        Ok(())
    }
    pub fn settings(&self) -> Value {
        self.actual_settings.clone()
    }
    pub fn prepare_shutter(&mut self, c: &Config) -> Result<(), String> {
        if self.shutter_attached {
            self.set_shutter(true)?;
            if let Some(rpc) = &self.rpc {
                rpc.request("disconnect_shutter", json!({}), Duration::from_secs(10))?;
            }
            self.shutter_attached = false;
        }
        if c.background.mode == ple_core::BackgroundMode::Automatic {
            if let Some(rpc) = &self.rpc {
                rpc.request(
                    "shutter_connect",
                    json!({"serial":c.background.shutter_serial,"closed_state":c.background.closed_state,"open_state":c.background.open_state,"timeout_s":c.background.timeout_s}),
                    timeout(c.background.timeout_s + 5.),
                )?;
            }
            self.shutter_attached = true;
        }
        Ok(())
    }
    pub fn set_shutter(&self, closed: bool) -> Result<(), String> {
        if !self.shutter_attached {
            return Err("Automatic shutter is not connected".into());
        }
        if let Some(rpc) = &self.rpc {
            let r = rpc.request(
                "shutter_set",
                json!({"closed":closed,"timeout_s":self.config.background.timeout_s}),
                timeout(self.config.background.timeout_s + 5.),
            )?;
            if r["closed"].as_bool() != Some(closed) {
                return Err("Shutter readback did not confirm requested optical state".into());
            }
        }
        Ok(())
    }
    pub fn acquire(
        &mut self,
        path: &str,
        background: bool,
        position: f64,
    ) -> Result<Spectrum, String> {
        if let Some(rpc) = &self.rpc {
            let v=rpc.request("acquire",json!({"path_base":path,"background":background,"timeout_s":self.config.spectrometer.timeout_s}),timeout(self.config.spectrometer.timeout_s+10.))?;
            let spectrum: Spectrum =
                serde_json::from_value(v).map_err(|e| format!("Invalid spectrum response: {e}"))?;
            spectrum.validate()?;
            let path = spectrum
                .spe_path
                .as_ref()
                .ok_or("LightField did not report a raw SPE file")?;
            if !Path::new(path).is_file() {
                return Err("Raw SPE file is not available locally".into());
            }
            Ok(spectrum)
        } else {
            thread::sleep(Duration::from_millis(80));
            self.acquisitions += 1;
            let wavelength_nm: Vec<_> = (0..641).map(|i| 640. + i as f64 * 0.375).collect();
            let exc = 700. + (position - 18.) * 50.;
            let response = 0.5 + 1.4 * (-((exc - 707.) / 5.).powi(2)).exp();
            let counts = wavelength_nm
                .iter()
                .map(|w| {
                    let bg = 70. + 0.04 * (w - 640.);
                    if background {
                        bg
                    } else {
                        let pl = 900.
                            * response
                            * (-0.5 * ((w - (780. + (exc - 700.) * 0.1)) / 7.).powi(2)).exp();
                        let laser = 1800. * (-0.5 * ((w - exc) / 0.8).powi(2)).exp();
                        let noise = 2. * ((w * 13. + self.acquisitions as f64).sin());
                        bg + pl + laser + noise
                    }
                })
                .collect();
            Ok(Spectrum {
                wavelength_nm,
                counts,
                spe_path: None,
                settings: self.actual_settings.clone(),
            })
        }
    }
}
pub enum StageCommand {
    Connect(Box<Config>, mpsc::Sender<Result<(), String>>),
    Disconnect,
    InvalidateRemote(u64),
    Move(f64, mpsc::Sender<Result<f64, String>>),
    Shutdown,
}
#[derive(Clone)]
pub struct StageClient {
    tx: mpsc::Sender<StageCommand>,
    connected: Arc<AtomicBool>,
    remote: Arc<Mutex<Option<(u64, RemoteStage)>>>,
    epoch: Arc<AtomicU64>,
    stop_barrier: Arc<(Mutex<StopBarrier>, Condvar)>,
    events: mpsc::Sender<Event>,
}
#[derive(Default)]
struct StopBarrier {
    pending: bool,
    unconfirmed: Option<String>,
}
impl StageClient {
    pub fn spawn(events: mpsc::Sender<Event>) -> Self {
        let (tx, rx) = mpsc::channel();
        let connected = Arc::new(AtomicBool::new(false));
        let worker_connected = connected.clone();
        let remote = Arc::new(Mutex::new(None));
        let worker_remote = remote.clone();
        let epoch = Arc::new(AtomicU64::new(0));
        let worker_epoch = epoch.clone();
        let stop_barrier = Arc::new((Mutex::new(StopBarrier::default()), Condvar::new()));
        let worker_events = events.clone();
        thread::spawn(move || {
            let mut stage: Option<Stage> = None;
            let mut stage_epoch = None;
            loop {
                match rx.recv_timeout(Duration::from_millis(400)) {
                    Ok(StageCommand::Connect(c, reply)) => {
                        let result = Stage::connect(c.stage, c.mode);
                        match result {
                            Ok(s) => {
                                let generation = worker_epoch.fetch_add(1, Ordering::AcqRel) + 1;
                                *worker_remote.lock().unwrap() =
                                    s.remote.clone().map(|remote| (generation, remote));
                                stage = Some(s);
                                stage_epoch = Some(generation);
                                worker_connected.store(true, Ordering::Release);
                                let _ = reply.send(Ok(()));
                            }
                            Err(e) => {
                                stage = None;
                                stage_epoch = None;
                                *worker_remote.lock().unwrap() = None;
                                worker_connected.store(false, Ordering::Release);
                                let _ = reply.send(Err(e));
                            }
                        }
                    }
                    Ok(StageCommand::Disconnect) => {
                        stage = None;
                        stage_epoch = None;
                        worker_epoch.fetch_add(1, Ordering::AcqRel);
                        *worker_remote.lock().unwrap() = None;
                        worker_connected.store(false, Ordering::Release);
                    }
                    Ok(StageCommand::Move(p, reply)) => {
                        let result = stage
                            .as_mut()
                            .ok_or("CONEX not connected".into())
                            .and_then(|s| s.move_to(p));
                        if result.is_err() {
                            worker_connected.store(false, Ordering::Release);
                            stage = None;
                            stage_epoch = None;
                            worker_epoch.fetch_add(1, Ordering::AcqRel);
                            *worker_remote.lock().unwrap() = None;
                            let _ = events.send(Event::Device {
                                device: "stage",
                                connected: false,
                            });
                        }
                        let _ = reply.send(result);
                    }
                    Ok(StageCommand::InvalidateRemote(generation)) => {
                        if stage_epoch == Some(generation) {
                            stage = None;
                            stage_epoch = None;
                            worker_epoch.fetch_add(1, Ordering::AcqRel);
                            *worker_remote.lock().unwrap() = None;
                            worker_connected.store(false, Ordering::Release);
                        }
                    }
                    Ok(StageCommand::Shutdown) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    Err(mpsc::RecvTimeoutError::Timeout) => {
                        if let Some(s) = &mut stage {
                            match s.position() {
                                Ok(p) => {
                                    let _ = events.send(Event::Telemetry {
                                        device: "stage",
                                        value: p,
                                        timestamp: ple_core::recording::unix_s(),
                                    });
                                }
                                Err(e) => {
                                    let _ =
                                        events.send(Event::Log(format!("CONEX telemetry: {e}")));
                                    let _ = events.send(Event::Device {
                                        device: "stage",
                                        connected: false,
                                    });
                                    stage = None;
                                    stage_epoch = None;
                                    worker_epoch.fetch_add(1, Ordering::AcqRel);
                                    *worker_remote.lock().unwrap() = None;
                                    worker_connected.store(false, Ordering::Release);
                                }
                            }
                        }
                    }
                }
            }
        });
        Self {
            tx,
            connected,
            remote,
            epoch,
            stop_barrier,
            events: worker_events,
        }
    }
    pub fn stop_status(&self) -> (bool, Option<String>) {
        let state = self.stop_barrier.0.lock().unwrap();
        (state.pending, state.unconfirmed.clone())
    }
    pub fn is_connected(&self) -> bool {
        self.connected.load(Ordering::Acquire)
    }
    pub fn connect(&self, c: Config) -> Result<(), String> {
        self.wait_for_stop()?;
        let (tx, rx) = mpsc::channel();
        self.tx
            .send(StageCommand::Connect(Box::new(c), tx))
            .map_err(|e| e.to_string())?;
        rx.recv().map_err(|e| e.to_string())?
    }
    pub fn move_to(&self, p: f64, cancelled: &AtomicBool) -> Result<f64, String> {
        self.wait_for_stop()?;
        if cancelled.load(Ordering::Acquire) {
            return Err("Stop requested; CONEX move was not started".into());
        }
        let remote = { self.remote.lock().unwrap().clone() };
        if let Some((generation, remote)) = remote {
            let result = remote.move_to(p);
            if let Err(error) = &result {
                self.schedule_stop(generation, remote.clone());
                self.clear_remote(generation, format!("CONEX remote move: {error}"));
            }
            return result;
        }
        let (tx, rx) = mpsc::channel();
        self.tx
            .send(StageCommand::Move(p, tx))
            .map_err(|e| e.to_string())?;
        rx.recv().map_err(|e| e.to_string())?
    }
    pub fn disconnect(&self) {
        self.connected.store(false, Ordering::Release);
        self.epoch.fetch_add(1, Ordering::AcqRel);
        *self.remote.lock().unwrap() = None;
        let _ = self.tx.send(StageCommand::Disconnect);
    }
    /// A remote stop opens an independent short-lived connection, so it can run
    /// while a remote move call is waiting for controller confirmation.
    pub fn stop(&self) {
        let remote = { self.remote.lock().unwrap().clone() };
        let Some((generation, remote)) = remote else {
            return;
        };
        self.schedule_stop(generation, remote);
    }
    fn schedule_stop(&self, generation: u64, remote: RemoteStage) {
        let (lock, _) = &*self.stop_barrier;
        let mut barrier = lock.lock().unwrap();
        if barrier.pending {
            return;
        }
        barrier.pending = true;
        drop(barrier);
        let connected = self.connected.clone();
        let shared_remote = self.remote.clone();
        let events = self.events.clone();
        let tx = self.tx.clone();
        let epoch = self.epoch.clone();
        let stop_barrier = self.stop_barrier.clone();
        thread::spawn(move || {
            let result = remote.stop();
            let unconfirmed = result.as_ref().err().cloned();
            match result {
                Ok(position) => {
                    let _ = events.send(Event::Log(format!(
                        "CONEX Stop confirmed ready at {position:.6} mm"
                    )));
                    let _ = events.send(Event::Telemetry {
                        device: "stage",
                        value: position,
                        timestamp: ple_core::recording::unix_s(),
                    });
                }
                Err(error) => {
                    let current_generation =
                        { shared_remote.lock().unwrap().as_ref().map(|(id, _)| *id) };
                    if epoch.load(Ordering::Acquire) == generation
                        && current_generation == Some(generation)
                    {
                        *shared_remote.lock().unwrap() = None;
                        let _ = epoch.compare_exchange(
                            generation,
                            generation + 1,
                            Ordering::AcqRel,
                            Ordering::Acquire,
                        );
                        connected.store(false, Ordering::Release);
                        // Invalidate the actor-owned Stage as well.  Otherwise its
                        // telemetry loop could resume publishing a stale connection.
                        // This is queued after the in-flight operation and is safe to
                        // send from this independent emergency-stop thread.
                        let _ = tx.send(StageCommand::InvalidateRemote(generation));
                        let _ = events.send(Event::Log(format!("CONEX remote stop: {error}")));
                        let _ = events.send(Event::Device {
                            device: "stage",
                            connected: false,
                        });
                    }
                }
            }
            let (lock, ready) = &*stop_barrier;
            let mut barrier = lock.lock().unwrap();
            barrier.pending = false;
            if let Some(error) = unconfirmed {
                barrier.unconfirmed = Some(error);
            }
            drop(barrier);
            ready.notify_all();
        });
    }
    fn wait_for_stop(&self) -> Result<(), String> {
        let (lock, ready) = &*self.stop_barrier;
        let barrier = lock.lock().unwrap();
        let (barrier, waited) = ready
            .wait_timeout_while(barrier, Duration::from_secs(12), |state| state.pending)
            .map_err(|_| "CONEX stop synchronization failed".to_string())?;
        if barrier.pending || waited.timed_out() {
            return Err(
                "previous CONEX stop is still unconfirmed; reconnect/move is blocked".into(),
            );
        }
        if let Some(error) = &barrier.unconfirmed {
            return Err(format!(
                "previous CONEX stop was not confirmed ({error}); verify or restart the remote CONEX service, then reopen the app before moving or reconnecting"
            ));
        }
        Ok(())
    }
    fn clear_remote(&self, generation: u64, message: String) {
        if self.epoch.load(Ordering::Acquire) != generation {
            return;
        }
        let mut remote = self.remote.lock().unwrap();
        if remote.as_ref().map(|(id, _)| *id) != Some(generation) {
            return;
        }
        *remote = None;
        drop(remote);
        let _ = self.epoch.compare_exchange(
            generation,
            generation + 1,
            Ordering::AcqRel,
            Ordering::Acquire,
        );
        self.connected.store(false, Ordering::Release);
        let _ = self.tx.send(StageCommand::InvalidateRemote(generation));
        let _ = self.events.send(Event::Log(message));
        let _ = self.events.send(Event::Device {
            device: "stage",
            connected: false,
        });
    }
    pub fn shutdown(&self) {
        *self.remote.lock().unwrap() = None;
        self.epoch.fetch_add(1, Ordering::AcqRel);
        let _ = self.tx.send(StageCommand::Shutdown);
    }
}
struct Stage {
    config: StageConfig,
    mode: Mode,
    position: f64,
    remote: Option<RemoteStage>,
}
impl Stage {
    fn connect(config: StageConfig, mode: Mode) -> Result<Self, String> {
        config.validate_limits()?;
        let mut s = Self {
            config,
            mode,
            position: 18.,
            remote: None,
        };
        if mode == Mode::Hardware {
            match s.config.protocol {
                StageProtocol::RemoteJson => {
                    let remote = RemoteStage::connect(&s.config)?;
                    s.position = remote.info().position_mm;
                    s.remote = Some(remote);
                }
                StageProtocol::LegacyTcp => match s.request("GET").and_then(|v| parse_finite(&v)) {
                    Ok(p) => s.position = p,
                    Err(e) => {
                        if !s.config.allow_legacy_initialize {
                            return Err(format!(
                                "Existing server is not initialized ({e}). CHECK may home the stage; enable explicit legacy initialization only after checking the setup."
                            ));
                        }
                        let r = s.request(&format!("CHECK {}", s.config.com_port))?;
                        if r != "OK" {
                            return Err(format!("CONEX CHECK: {r}"));
                        }
                        s.position = s.read()?;
                    }
                },
            }
        }
        Ok(s)
    }
    fn request(&self, command: &str) -> Result<String, String> {
        let mut last = String::new();
        let addresses = (self.config.host.as_str(), self.config.port)
            .to_socket_addrs()
            .map_err(|e| e.to_string())?;
        let mut socket = None;
        for addr in addresses {
            match TcpStream::connect_timeout(&addr, Duration::from_secs(3)) {
                Ok(s) => {
                    socket = Some(s);
                    break;
                }
                Err(e) => last = e.to_string(),
            }
        }
        let mut socket = socket.ok_or(format!("Cannot connect to CONEX server: {last}"))?;
        socket
            .set_read_timeout(Some(timeout(self.config.timeout_s)))
            .map_err(|e| e.to_string())?;
        socket
            .set_write_timeout(Some(Duration::from_secs(3)))
            .map_err(|e| e.to_string())?;
        socket
            .write_all(format!("{command}\n").as_bytes())
            .map_err(|e| e.to_string())?;
        let mut reader = BufReader::new(socket).take(8192);
        let mut response = String::new();
        reader.read_line(&mut response).map_err(|e| e.to_string())?;
        Ok(response.trim().into())
    }
    fn read(&self) -> Result<f64, String> {
        parse_finite(&self.request("GET")?)
    }
    fn position(&mut self) -> Result<f64, String> {
        if self.mode == Mode::Hardware {
            self.position = if let Some(remote) = &self.remote {
                remote.position()?
            } else {
                self.read()?
            };
        }
        Ok(self.position)
    }
    fn move_to(&mut self, target: f64) -> Result<f64, String> {
        if !target.is_finite() || target < self.config.min_mm || target > self.config.max_mm {
            return Err("CONEX target outside limits".into());
        }
        if self.mode == Mode::Hardware {
            if let Some(remote) = &self.remote {
                self.position = remote.move_to(target)?;
                return Ok(self.position);
            }
            let result = self.request(&format!("MOVE {target:0.8}"))?;
            if !result.starts_with("MOVED ") {
                return Err(format!("CONEX MOVE: {result}"));
            }
            let actual = parse_finite(result.trim_start_matches("MOVED "))?;
            let readback = self.read()?;
            if (actual - target).abs() > self.config.tolerance_mm
                || (readback - target).abs() > self.config.tolerance_mm
            {
                return Err(format!(
                    "CONEX position mismatch: requested {target:0.6}, read {readback:0.6}"
                ));
            }
            self.position = readback;
        } else {
            thread::sleep(Duration::from_millis(40));
            self.position = target;
        }
        Ok(self.position)
    }
}
fn parse_finite(s: &str) -> Result<f64, String> {
    let p = s
        .trim()
        .parse::<f64>()
        .map_err(|_| format!("Invalid CONEX response: {s}"))?;
    if p.is_finite() {
        Ok(p)
    } else {
        Err("Non-finite device readback".into())
    }
}
type PowerReading = (f64, f64, f64, Value);
pub enum PowerCommand {
    Connect(Box<Config>, mpsc::Sender<Result<(), String>>),
    Disconnect,
    Read(mpsc::Sender<Result<PowerReading, String>>),
    Inventory(mpsc::Sender<Result<Value, String>>),
    ReadAt(f64, mpsc::Sender<Result<PowerReading, String>>),
    Shutdown(mpsc::Sender<()>),
}
#[derive(Clone)]
pub struct PowerClient {
    tx: mpsc::Sender<PowerCommand>,
    connected: Arc<AtomicBool>,
}
impl PowerClient {
    pub fn spawn(events: mpsc::Sender<Event>) -> Self {
        let (tx, rx) = mpsc::channel();
        let connected = Arc::new(AtomicBool::new(false));
        let worker_connected = connected.clone();
        thread::spawn(move || {
            let mut power: Option<Power> = None;
            loop {
                match rx.recv_timeout(Duration::from_millis(500)) {
                    Ok(PowerCommand::Connect(c, reply)) => {
                        let result = Power::connect(&c, events.clone());
                        match result {
                            Ok(p) => {
                                let _ = events.send(Event::PowerWavelength(p.wavelength_nm));
                                power = Some(p);
                                worker_connected.store(true, Ordering::Release);
                                let _ = reply.send(Ok(()));
                            }
                            Err(e) => {
                                power = None;
                                worker_connected.store(false, Ordering::Release);
                                let _ = reply.send(Err(e));
                            }
                        }
                    }
                    Ok(PowerCommand::Disconnect) => {
                        power = None;
                        worker_connected.store(false, Ordering::Release);
                    }
                    Ok(PowerCommand::Inventory(reply)) => {
                        let _ = reply.send(
                            power
                                .as_ref()
                                .map(|p| p.hardware.clone())
                                .ok_or("Power meter not connected".into()),
                        );
                    }
                    Ok(PowerCommand::ReadAt(wavelength, reply)) => {
                        let result = power
                            .as_mut()
                            .ok_or("Power meter not connected".into())
                            .and_then(|p| {
                                p.read_at(wavelength)
                                    .map(|(w, t, nm)| (w, t, nm, p.hardware.clone()))
                            });
                        match &result {
                            Ok((value, timestamp, actual, _)) => {
                                let _ = events.send(Event::PowerWavelength(*actual));
                                let _ = events.send(Event::Telemetry {
                                    device: "power",
                                    value: *value,
                                    timestamp: *timestamp,
                                });
                            }
                            Err(_) => {
                                power = None;
                                worker_connected.store(false, Ordering::Release);
                                let _ = events.send(Event::Device {
                                    device: "power",
                                    connected: false,
                                });
                            }
                        }
                        let _ = reply.send(result);
                    }
                    Ok(PowerCommand::Read(reply)) => {
                        let result = power
                            .as_mut()
                            .ok_or("Power meter not connected".into())
                            .and_then(|p| {
                                p.read()
                                    .map(|(w, t)| (w, t, p.wavelength_nm, p.hardware.clone()))
                            });
                        if let Ok((value, timestamp, actual, _)) = &result {
                            let _ = events.send(Event::PowerWavelength(*actual));
                            let _ = events.send(Event::Telemetry {
                                device: "power",
                                value: *value,
                                timestamp: *timestamp,
                            });
                        } else {
                            worker_connected.store(false, Ordering::Release);
                            power = None;
                            let _ = events.send(Event::Device {
                                device: "power",
                                connected: false,
                            });
                        }
                        let _ = reply.send(result);
                    }
                    Ok(PowerCommand::Shutdown(done)) => {
                        drop(power.take());
                        let _ = done.send(());
                        break;
                    }
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    Err(mpsc::RecvTimeoutError::Timeout) => {
                        if let Some(p) = &mut power {
                            match p.read() {
                                Ok((w, timestamp)) => {
                                    let _ = events.send(Event::Telemetry {
                                        device: "power",
                                        value: w,
                                        timestamp,
                                    });
                                }
                                Err(e) => {
                                    let _ =
                                        events.send(Event::Log(format!("Power telemetry: {e}")));
                                    let _ = events.send(Event::Device {
                                        device: "power",
                                        connected: false,
                                    });
                                    power = None;
                                    worker_connected.store(false, Ordering::Release);
                                }
                            }
                        }
                    }
                }
            }
        });
        Self { tx, connected }
    }
    pub fn is_connected(&self) -> bool {
        self.connected.load(Ordering::Acquire)
    }
    pub fn connect(&self, c: Config) -> Result<(), String> {
        let (tx, rx) = mpsc::channel();
        self.tx
            .send(PowerCommand::Connect(Box::new(c), tx))
            .map_err(|e| e.to_string())?;
        rx.recv().map_err(|e| e.to_string())?
    }
    pub fn read_at(&self, wavelength: f64) -> Result<PowerReading, String> {
        if !wavelength.is_finite() || wavelength <= 0. {
            return Err("Invalid power sensor wavelength".into());
        }
        let (tx, rx) = mpsc::channel();
        self.tx
            .send(PowerCommand::ReadAt(wavelength, tx))
            .map_err(|e| e.to_string())?;
        rx.recv().map_err(|e| e.to_string())?
    }
    pub fn hardware(&self) -> Result<Value, String> {
        let (tx, rx) = mpsc::channel();
        self.tx
            .send(PowerCommand::Inventory(tx))
            .map_err(|e| e.to_string())?;
        rx.recv().map_err(|e| e.to_string())?
    }
    pub fn read_with_wavelength(&self) -> Result<PowerReading, String> {
        let (tx, rx) = mpsc::channel();
        self.tx
            .send(PowerCommand::Read(tx))
            .map_err(|e| e.to_string())?;
        rx.recv().map_err(|e| e.to_string())?
    }
    pub fn disconnect(&self) {
        self.connected.store(false, Ordering::Release);
        let _ = self.tx.send(PowerCommand::Disconnect);
    }
    pub fn shutdown(&self) {
        self.connected.store(false, Ordering::Release);
        let (tx, rx) = mpsc::channel();
        if self.tx.send(PowerCommand::Shutdown(tx)).is_ok() {
            let _ = rx.recv_timeout(Duration::from_secs(21));
        }
    }
}
struct Power {
    hardware: Value,
    events: mpsc::Sender<Event>,
    wavelength_tolerance_nm: f64,
    wavelength_nm: f64,
    rpc: Option<RpcClient>,
    start: Instant,
}
impl Drop for Power {
    fn drop(&mut self) {
        if let Some(rpc) = &self.rpc {
            let _ = rpc.request("shutdown", json!({}), Duration::from_secs(5));
        }
    }
}
impl Power {
    fn connect(c: &Config, events: mpsc::Sender<Event>) -> Result<Self, String> {
        let mut hardware = json!({"demo":true,"source":"simulated"});
        let mut actual_wavelength = c.power.wavelength_nm;
        let rpc = if c.mode == Mode::Hardware {
            let rpc = RpcClient::start(c, events.clone())?;
            let state = rpc.request(
                "connect_power",
                serde_json::to_value(&c.power).map_err(|e| e.to_string())?,
                Duration::from_secs(30),
            )?;
            hardware = state.get("hardware").cloned().unwrap_or_else(
                || json!({"available":false,"errors":["Bridge did not provide power inventory"]}),
            );
            actual_wavelength = state["wavelength_nm"]
                .as_f64()
                .ok_or("Missing power wavelength readback")?;
            if !actual_wavelength.is_finite()
                || (actual_wavelength - c.power.wavelength_nm).abs()
                    > c.power.wavelength_tolerance_nm
            {
                return Err("Power sensor wavelength was not accepted".into());
            }
            Some(rpc)
        } else {
            None
        };
        let _ = events.send(Event::PowerHardware(hardware.clone()));
        Ok(Self {
            hardware,
            events,
            wavelength_nm: actual_wavelength,
            wavelength_tolerance_nm: c.power.wavelength_tolerance_nm,
            rpc,
            start: Instant::now(),
        })
    }
    fn read_at(&mut self, wavelength: f64) -> Result<(f64, f64, f64), String> {
        let actual = if let Some(rpc) = &self.rpc {
            let r = rpc.request(
                "power_set_wavelength",
                json!({"wavelength_nm":wavelength,"wavelength_tolerance_nm":self.wavelength_tolerance_nm}),
                Duration::from_secs(10),
            )?;
            r["wavelength_nm"]
                .as_f64()
                .ok_or("Missing power wavelength readback")?
        } else {
            wavelength
        };
        if !actual.is_finite() || (actual - wavelength).abs() > self.wavelength_tolerance_nm {
            return Err("Power sensor rejected the requested wavelength".into());
        }
        self.wavelength_nm = actual;
        let (value, time) = self.read()?;
        Ok((value, time, actual))
    }
    fn read(&mut self) -> Result<(f64, f64), String> {
        let value = if let Some(rpc) = &self.rpc {
            let r = rpc.request("power_read", json!({}), Duration::from_secs(15))?;
            if let Some(hardware) = r.get("hardware")
                && *hardware != self.hardware
            {
                self.hardware = hardware.clone();
                let _ = self
                    .events
                    .send(Event::PowerHardware(self.hardware.clone()));
            }
            let readback = r["wavelength_nm"]
                .as_f64()
                .ok_or("Missing actual power wavelength")?;
            if !readback.is_finite() || (readback - self.wavelength_nm).abs() > 1e-6 {
                return Err("Power sensor wavelength changed outside the application".into());
            }
            r["power_w"]
                .as_f64()
                .ok_or("Power read did not contain power_w")?
        } else {
            20e-6 * (1. + 0.02 * (self.start.elapsed().as_secs_f64() * 0.4).sin())
        };
        if !value.is_finite() {
            return Err("Non-finite power reading".into());
        }
        Ok((value, ple_core::recording::unix_s()))
    }
}

fn resolve_bridge_script(script: &Path) -> PathBuf {
    if script.is_file() {
        return script.to_owned();
    }
    if let Ok(exe) = std::env::current_exe()
        && let Some(parent) = exe.parent()
    {
        for candidate in [
            parent.join(script),
            parent.join("../Resources").join(script),
        ] {
            if candidate.is_file() {
                return candidate;
            }
        }
    }
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(script)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn failed_remote_move_keeps_a_stop_path_even_after_connection_invalidation() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = thread::spawn(move || {
            let until = Instant::now() + Duration::from_secs(3);
            loop {
                assert!(Instant::now() < until, "Stop was never sent");
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        // Accepted sockets inherit nonblocking mode on Windows.
                        // This mock reads full request lines with a deadline.
                        stream.set_nonblocking(false).unwrap();
                        stream
                            .set_read_timeout(Some(Duration::from_secs(1)))
                            .unwrap();
                        let mut line = String::new();
                        BufReader::new(stream.try_clone().unwrap())
                            .read_line(&mut line)
                            .unwrap();
                        let req: Value = serde_json::from_str(&line).unwrap();
                        let op = req["op"].as_str().unwrap();
                        let result = if op == "hello" {
                            json!({"protocol":"kohdalab-conex","revision":1,"can_stop":true,"can_home":false})
                        } else {
                            json!({"position_mm":18.,"min_mm":0.,"max_mm":25.,"state":"32"})
                        };
                        writeln!(stream,"{}",json!({"id":req["id"],"ok":op!="move","result":result,"error":if op=="move"{json!("uncertain move")}else{Value::Null}})).unwrap();
                        if op == "stop" {
                            break;
                        }
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(2))
                    }
                    Err(e) => panic!("{e}"),
                }
            }
        });
        let (tx, _rx) = mpsc::channel();
        let stage = StageClient::spawn(tx);
        let c = Config {
            mode: Mode::Hardware,
            stage: StageConfig {
                protocol: StageProtocol::RemoteJson,
                host: "127.0.0.1".into(),
                port,
                token_env: String::new(),
                ..Default::default()
            },
            ..Default::default()
        };
        stage.connect(c).unwrap();
        assert!(stage.move_to(19., &AtomicBool::new(false)).is_err());
        stage.wait_for_stop().unwrap();
        assert!(!stage.is_connected());
        stage.shutdown();
        server.join().unwrap();
    }
    #[test]
    fn bounded_bridge_framing_rejects_partial_and_oversized_replies() {
        assert_eq!(
            read_bounded_line(&mut std::io::Cursor::new(b"{}\n"), 8)
                .unwrap()
                .unwrap(),
            b"{}\n"
        );
        assert!(read_bounded_line(&mut std::io::Cursor::new(b"{}"), 8).is_err());
        assert!(read_bounded_line(&mut std::io::Cursor::new(b"123456789\n"), 8).is_err());
    }
    #[test]
    fn failed_or_timed_out_acquisition_verifies_idle_before_another_operation() {
        for timed_out in [false, true] {
            for idle_confirmed in [false, true] {
                let dir = tempfile::tempdir().unwrap();
                let script = dir.path().join("fault_bridge.py");
                std::fs::write(&script, format!(
                    "import sys,json,time\nseen=[]\nfor line in sys.stdin:\n r=json.loads(line);op=r['op'];seen.append(op)\n if op=='acquire' and {timed_out}: time.sleep(0.15)\n if op=='stop': time.sleep(0.05)\n print(json.dumps({{'id':r['id'],'ok':op!='acquire','result':{{'seen':seen,'idle_confirmed':{idle_confirmed}}},'error':'SDK acquisition error'}}),flush=True)\n",
                    timed_out=if timed_out {"True"} else {"False"},
                    idle_confirmed=if idle_confirmed {"True"} else {"False"}
                )).unwrap();
                let mut c = Config::default();
                c.spectrometer.bridge_script = script.to_string_lossy().into();
                let (tx, _rx) = mpsc::channel();
                let rpc = RpcClient::start(&c, tx).unwrap();
                let error = rpc
                    .request(
                        "acquire",
                        json!({}),
                        if timed_out {
                            Duration::from_millis(40)
                        } else {
                            Duration::from_secs(2)
                        },
                    )
                    .unwrap_err();
                assert_eq!(error.contains("timed out"), timed_out);
                let next = rpc.request("configure_spectrometer", json!({}), Duration::from_secs(2));
                if idle_confirmed {
                    assert_eq!(
                        next.unwrap()["seen"],
                        json!(["acquire", "stop", "ensure_idle", "configure_spectrometer"])
                    );
                    assert_eq!(rpc.stop_status(), (false, None));
                } else {
                    assert!(next.unwrap_err().contains("unconfirmed"));
                    assert!(rpc.stop_status().1.is_some());
                }
            }
        }
    }
    #[test]
    fn bridge_stop_ack_precedes_new_configuration_and_failure_stays_latched() {
        for successful in [true, false] {
            let dir = tempfile::tempdir().unwrap();
            let script = dir.path().join("bridge.py");
            std::fs::write(&script,format!("import sys,json,time\nseen=[]\nfor line in sys.stdin:\n r=json.loads(line);seen.append(r['op'])\n if r['op']=='stop': time.sleep(0.1)\n ok=r['op']!='stop' or {}\n print(json.dumps({{'id':r['id'],'ok':ok,'result':{{'seen':seen,'idle_confirmed':True}},'error':'stop failed' if not ok else None}}),flush=True)\n",if successful{"True"}else{"False"})).unwrap();
            let mut c = Config::default();
            c.spectrometer.bridge_script = script.to_string_lossy().into();
            let (tx, _rx) = mpsc::channel();
            let rpc = RpcClient::start(&c, tx).unwrap();
            rpc.stop();
            let result = rpc.request("configure_spectrometer", json!({}), Duration::from_secs(2));
            if successful {
                let seen = result.unwrap();
                assert_eq!(
                    seen["seen"],
                    json!(["stop", "ensure_idle", "configure_spectrometer"])
                );
            } else {
                assert!(result.unwrap_err().contains("unconfirmed"));
                assert!(rpc.stop_status().1.is_some());
            }
        }
    }
    #[test]
    fn delayed_stop_blocks_reconnect_and_stale_failure_latches_recovery() {
        for success in [true, false] {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let port = listener.local_addr().unwrap().port();
            let (entered_tx, entered_rx) = mpsc::channel();
            let (release_tx, release_rx) = mpsc::channel();
            let server = thread::spawn(move || {
                for _ in 0..3 {
                    let (mut stream, _) = listener.accept().unwrap();
                    let mut line = String::new();
                    BufReader::new(stream.try_clone().unwrap())
                        .read_line(&mut line)
                        .unwrap();
                    let req: Value = serde_json::from_str(&line).unwrap();
                    let result = match req["op"].as_str().unwrap() {
                        "hello" => {
                            json!({"protocol":"kohdalab-conex","revision":1,"can_stop":true,"can_home":false})
                        }
                        "connect" => {
                            json!({"position_mm":18.,"min_mm":0.,"max_mm":25.,"state":"32"})
                        }
                        "stop" => {
                            entered_tx.send(()).unwrap();
                            release_rx.recv().unwrap();
                            json!({"position_mm":18.,"min_mm":0.,"max_mm":25.,"state":"32"})
                        }
                        other => panic!("Unexpected operation {other}"),
                    };
                    let ok = req["op"] != "stop" || success;
                    writeln!(stream,"{}",json!({"id":req["id"],"ok":ok,"result":result,"error":if ok{Value::Null}else{json!("unconfirmed stop")}})).unwrap();
                }
            });
            let config = StageConfig {
                host: "127.0.0.1".into(),
                port,
                token_env: String::new(),
                ..Default::default()
            };
            let remote = RemoteStage::connect(&config).unwrap();
            let (tx, _rx) = mpsc::channel();
            let stage = StageClient::spawn(tx);
            stage.epoch.store(1, Ordering::Release);
            *stage.remote.lock().unwrap() = Some((1, remote));
            stage.connected.store(true, Ordering::Release);
            stage.stop();
            assert!(stage.stop_barrier.0.lock().unwrap().pending);
            entered_rx.recv_timeout(Duration::from_secs(1)).unwrap();
            stage.disconnect();
            let (reconnected_tx, reconnected_rx) = mpsc::channel();
            let next = stage.clone();
            let join = thread::spawn(move || {
                reconnected_tx
                    .send(next.connect(Config::default()))
                    .unwrap();
            });
            assert!(
                reconnected_rx
                    .recv_timeout(Duration::from_millis(75))
                    .is_err()
            );
            release_tx.send(()).unwrap();
            let result = reconnected_rx.recv_timeout(Duration::from_secs(2)).unwrap();
            assert_eq!(result.is_ok(), success);
            if success {
                assert!(stage.move_to(18.1, &AtomicBool::new(false)).is_ok());
            } else {
                assert!(!stage.stop_barrier.0.lock().unwrap().pending);
                assert!(
                    stage
                        .move_to(18.1, &AtomicBool::new(false))
                        .unwrap_err()
                        .contains("reopen")
                );
            }
            join.join().unwrap();
            stage.shutdown();
            server.join().unwrap();
        }
    }
    #[test]
    fn legacy_protocol_reads_newline_without_waiting_for_eof() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut line = String::new();
            BufReader::new(stream.try_clone().unwrap())
                .read_line(&mut line)
                .unwrap();
            assert_eq!(line, "GET\n");
            stream.write_all(b"18.1250\n").unwrap();
            thread::sleep(Duration::from_millis(500));
        });
        let stage = Stage {
            config: StageConfig {
                host: "127.0.0.1".into(),
                port,
                ..Default::default()
            },
            mode: Mode::Hardware,
            position: 0.,
            remote: None,
        };
        let start = Instant::now();
        assert_eq!(stage.read().unwrap(), 18.125);
        assert!(start.elapsed() < Duration::from_millis(450));
        server.join().unwrap();
    }
    #[test]
    fn explicit_power_reads_update_live_trace_and_connection_state() {
        let (tx, rx) = mpsc::channel();
        let power = PowerClient::spawn(tx);
        power.connect(Config::default()).unwrap();
        assert!(power.is_connected());
        let (w, _, _, _) = power.read_with_wavelength().unwrap();
        assert!(w > 0.);
        let inventory = rx.recv_timeout(Duration::from_secs(1)).unwrap();
        assert!(matches!(inventory, Event::PowerHardware(v) if v["source"] == "simulated"));
        let wavelength = rx.recv_timeout(Duration::from_secs(1)).unwrap();
        assert!(
            matches!(wavelength, Event::PowerWavelength(nm) if nm == Config::default().power.wavelength_nm)
        );
        let read_wavelength = rx.recv_timeout(Duration::from_secs(1)).unwrap();
        assert!(
            matches!(read_wavelength, Event::PowerWavelength(nm) if nm == Config::default().power.wavelength_nm)
        );
        let event = rx.recv_timeout(Duration::from_secs(1)).unwrap();
        assert!(matches!(
            event,
            Event::Telemetry {
                device: "power",
                ..
            }
        ));
        power.disconnect();
        assert!(!power.is_connected());
        power.shutdown();
    }
    #[test]
    fn failed_move_invalidates_stage_connection() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = thread::spawn(move || {
            for response in ["18.0000\n", "ERROR: move failed\n"] {
                let (mut stream, _) = listener.accept().unwrap();
                let mut line = String::new();
                BufReader::new(stream.try_clone().unwrap())
                    .read_line(&mut line)
                    .unwrap();
                stream.write_all(response.as_bytes()).unwrap();
            }
        });
        let (tx, rx) = mpsc::channel();
        let stage = StageClient::spawn(tx);
        let config = Config {
            mode: Mode::Hardware,
            stage: StageConfig {
                host: "127.0.0.1".into(),
                port,
                ..Default::default()
            },
            ..Default::default()
        };
        stage.connect(config).unwrap();
        assert!(stage.is_connected());
        assert!(stage.move_to(18.1, &AtomicBool::new(false)).is_err());
        assert!(!stage.is_connected());
        assert!(matches!(
            rx.recv_timeout(Duration::from_secs(1)).unwrap(),
            Event::Device {
                device: "stage",
                connected: false
            }
        ));
        stage.shutdown();
        server.join().unwrap();
    }
    #[test]
    fn no_motion_on_connect_and_limits_are_enforced() {
        let mut s = Stage::connect(StageConfig::default(), Mode::Demo).unwrap();
        assert_eq!(s.position().unwrap(), 18.);
        assert!(s.move_to(30.).is_err());
        assert!(s.move_to(f64::NAN).is_err());
        assert_eq!(s.position().unwrap(), 18.);
    }
}
