use super::{StopBarrier, timeout};
use crate::{
    engine::{CleanupCause, Event, RunError},
    remote_stage::RemoteStage,
};
use ple_core::{Config, Mode, StageConfig, StageProtocol};
use std::{
    io::{BufRead, BufReader, Read, Write},
    net::{TcpStream, ToSocketAddrs},
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc,
    },
    thread,
    time::Duration,
};
const STAGE_MONITOR_FAILURE_LIMIT: usize = 3;

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
    remote_motion: Arc<Mutex<RemoteMotion>>,
    stop_barrier: Arc<(Mutex<StopBarrier>, Condvar)>,
    events: mpsc::Sender<Event>,
}
#[derive(Default)]
struct RemoteMotion {
    active: usize,
    revision: u64,
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
        let remote_motion = Arc::new(Mutex::new(RemoteMotion::default()));
        let worker_motion = remote_motion.clone();
        let stop_barrier = Arc::new((Mutex::new(StopBarrier::default()), Condvar::new()));
        let worker_events = events.clone();
        thread::spawn(move || {
            let mut stage: Option<Stage> = None;
            let mut stage_epoch = None;
            let mut monitor_failures = 0;
            let mut last_motion_revision = 0;
            loop {
                match rx.recv_timeout(Duration::from_millis(400)) {
                    Ok(StageCommand::Connect(c, reply)) => {
                        monitor_failures = 0;
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
                        monitor_failures = 0;
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
                            let revision = worker_motion.lock().unwrap().revision;
                            let position = s.position();
                            let motion = worker_motion.lock().unwrap();
                            if last_motion_revision != motion.revision {
                                monitor_failures = 0;
                                last_motion_revision = motion.revision;
                            }
                            // An overlapping remote move owns the connection outcome.
                            if revision != motion.revision
                                || stage_epoch != Some(worker_epoch.load(Ordering::Acquire))
                            {
                                continue;
                            }
                            match position {
                                Ok(p) => {
                                    monitor_failures = 0;
                                    let _ = events.send(Event::Telemetry {
                                        device: "stage",
                                        value: p,
                                        timestamp: ple_core::recording::unix_s(),
                                    });
                                }
                                Err(e) => {
                                    let _ =
                                        events.send(Event::Log(format!("CONEX telemetry: {e}")));
                                    if motion.active > 0 {
                                        continue;
                                    }
                                    monitor_failures += 1;
                                    if monitor_failures < STAGE_MONITOR_FAILURE_LIMIT {
                                        continue;
                                    }
                                    stage = None;
                                    stage_epoch = None;
                                    worker_epoch.fetch_add(1, Ordering::AcqRel);
                                    *worker_remote.lock().unwrap() = None;
                                    worker_connected.store(false, Ordering::Release);
                                    let _ = events.send(Event::Device {
                                        device: "stage",
                                        connected: false,
                                    });
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
            remote_motion,
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
    pub fn move_to(&self, p: f64, cancelled: &AtomicBool) -> Result<f64, RunError> {
        self.wait_for_stop()?;
        if cancelled.load(Ordering::Acquire) {
            return Err(RunError::Stopped(
                "Stop requested; CONEX move was not started",
            ));
        }
        let remote = {
            let mut motion = self.remote_motion.lock().unwrap();
            let remote = self.remote.lock().unwrap().clone();
            if remote.is_some() {
                motion.active += 1;
                motion.revision += 1;
            }
            remote
        };
        if let Some((generation, remote)) = remote {
            let result = remote.move_to(p);
            if let Err(error) = &result {
                self.schedule_stop(generation, remote.clone());
                self.clear_remote(generation, format!("CONEX remote move: {error}"));
            }
            let mut motion = self.remote_motion.lock().unwrap();
            motion.active -= 1;
            motion.revision += 1;
            return result.map_err(RunError::from);
        }
        let (tx, rx) = mpsc::channel();
        self.tx
            .send(StageCommand::Move(p, tx))
            .map_err(|e| e.to_string())?;
        rx.recv()
            .map_err(|e| e.to_string())?
            .map_err(RunError::from)
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
            if unconfirmed.is_some() {
                barrier.unconfirmed = unconfirmed.clone();
            }
            let _ = events.send(Event::Cleanup {
                cause: CleanupCause::Stage,
                result: unconfirmed.map_or(Ok(()), Err),
            });
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};
    use std::time::Instant;

    #[test]
    fn stage_monitor_tolerates_one_timeout_and_disconnects_after_three_consecutive_failures() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = thread::spawn(move || {
            for (expected, response) in [
                ("GET", Some("18.0\n")),
                ("GET", None),
                ("MOVE 18.10000000", Some("MOVED 18.1\n")),
                ("GET", Some("18.1\n")),
                ("GET", Some("bad readback\n")),
                ("GET", Some("bad readback\n")),
                ("GET", Some("18.1\n")),
                ("GET", Some("bad readback\n")),
                ("GET", Some("bad readback\n")),
                ("GET", Some("bad readback\n")),
            ] {
                let (mut stream, _) = listener.accept().unwrap();
                let mut line = String::new();
                BufReader::new(stream.try_clone().unwrap())
                    .read_line(&mut line)
                    .unwrap();
                assert_eq!(line.trim_end(), expected);
                if let Some(response) = response {
                    stream.write_all(response.as_bytes()).unwrap();
                } else {
                    // Keep the connection open until the client read times out.
                    stream
                        .set_read_timeout(Some(Duration::from_secs(2)))
                        .unwrap();
                    assert_eq!(stream.read(&mut [0]).unwrap(), 0);
                }
            }
        });
        let (tx, rx) = mpsc::channel();
        let stage = StageClient::spawn(tx);
        stage
            .connect(Config {
                mode: Mode::Hardware,
                stage: StageConfig {
                    host: "127.0.0.1".into(),
                    port,
                    timeout_s: 0.1,
                    ..Default::default()
                },
                ..Default::default()
            })
            .unwrap();
        let next = || rx.recv_timeout(Duration::from_secs(3)).unwrap();
        assert!(matches!(next(), Event::Log(msg) if msg.starts_with("CONEX telemetry:")));
        assert!(stage.is_connected());
        assert_eq!(stage.move_to(18.1, &AtomicBool::new(false)).unwrap(), 18.1);
        for _ in 0..2 {
            assert!(matches!(next(), Event::Log(_)));
        }
        assert!(matches!(
            next(),
            Event::Telemetry {
                device: "stage",
                ..
            }
        ));
        for _ in 0..3 {
            assert!(matches!(next(), Event::Log(_)));
        }
        assert!(matches!(
            next(),
            Event::Device {
                device: "stage",
                connected: false
            }
        ));
        assert!(!stage.is_connected());
        stage.shutdown();
        server.join().unwrap();
    }

    #[test]
    fn remote_move_result_takes_priority_over_overlapping_monitor_failure() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let (polling, poll_entered) = mpsc::channel();
        let (moved, move_finished) = mpsc::channel();
        let server = thread::spawn(move || {
            let accept = || {
                let (stream, _) = listener.accept().unwrap();
                let mut line = String::new();
                BufReader::new(stream.try_clone().unwrap())
                    .read_line(&mut line)
                    .unwrap();
                (stream, serde_json::from_str::<Value>(&line).unwrap())
            };
            let reply = |mut stream: TcpStream, req: &Value, result: Value, ok| {
                writeln!(
                    stream,
                    "{}",
                    json!({"id":req["id"],"ok":ok,"result":result,"error":"mock telemetry failure"})
                )
                .unwrap();
            };
            let (stream, req) = accept();
            assert_eq!(req["op"], "hello");
            reply(
                stream,
                &req,
                json!({"protocol":"kohdalab-conex","revision":1,"can_stop":true,"can_home":false}),
                true,
            );
            let (stream, req) = accept();
            assert_eq!(req["op"], "connect");
            let position = |mm| json!({"position_mm":mm,"min_mm":0.,"max_mm":25.,"state":"32"});
            reply(stream, &req, position(18.), true);
            for _ in 0..2 {
                let (stream, req) = accept();
                assert_eq!(req["op"], "status");
                reply(stream, &req, Value::Null, false);
            }
            let (poll_stream, poll_req) = accept();
            assert_eq!(poll_req["op"], "status");
            polling.send(()).unwrap();
            let (stream, req) = accept();
            assert_eq!(req["op"], "move");
            reply(stream, &req, position(18.1), true);
            move_finished.recv_timeout(Duration::from_secs(3)).unwrap();
            reply(poll_stream, &poll_req, Value::Null, false);
            let (stream, req) = accept();
            assert_eq!(req["op"], "status");
            reply(stream, &req, position(18.1), true);
        });
        let (tx, rx) = mpsc::channel();
        let stage = StageClient::spawn(tx);
        stage
            .connect(Config {
                mode: Mode::Hardware,
                stage: StageConfig {
                    protocol: StageProtocol::RemoteJson,
                    host: "127.0.0.1".into(),
                    port,
                    token_env: String::new(),
                    ..Default::default()
                },
                ..Default::default()
            })
            .unwrap();
        let next = || rx.recv_timeout(Duration::from_secs(3)).unwrap();
        assert!(matches!(next(), Event::Log(_)));
        assert!(matches!(next(), Event::Log(_)));
        poll_entered.recv_timeout(Duration::from_secs(3)).unwrap();
        assert_eq!(stage.move_to(18.1, &AtomicBool::new(false)).unwrap(), 18.1);
        moved.send(()).unwrap();
        assert!(matches!(next(), Event::Telemetry { device: "stage", value, .. } if value == 18.1));
        assert!(stage.is_connected());
        stage.shutdown();
        server.join().unwrap();
    }

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
        let (tx, rx) = mpsc::channel();
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
        assert!(rx.try_iter().any(|event| matches!(
            event,
            Event::Cleanup {
                cause: CleanupCause::Stage,
                result: Ok(())
            }
        )));
        assert!(!stage.is_connected());
        stage.shutdown();
        server.join().unwrap();
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
                        .to_string()
                        .contains("reopen")
                );
            }
            join.join().unwrap();
            stage.shutdown();
            server.join().unwrap();
        }
    }
    #[test]
    fn successful_retry_stop_does_not_erase_prior_unconfirmed_stop() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = thread::spawn(move || {
            let mut stops = 0;
            for _ in 0..6 {
                let (mut stream, _) = listener.accept().unwrap();
                let mut line = String::new();
                BufReader::new(stream.try_clone().unwrap())
                    .read_line(&mut line)
                    .unwrap();
                let request: Value = serde_json::from_str(&line).unwrap();
                let op = request["op"].as_str().unwrap();
                let result = match op {
                    "hello" => {
                        json!({"protocol":"kohdalab-conex","revision":1,"can_stop":true,"can_home":false})
                    }
                    "connect" | "stop" => {
                        json!({"position_mm":18.,"min_mm":0.,"max_mm":25.,"state":"32"})
                    }
                    other => panic!("Unexpected operation {other}"),
                };
                let ok = if op == "stop" {
                    stops += 1;
                    stops > 1
                } else {
                    true
                };
                writeln!(stream,"{}",json!({"id":request["id"],"ok":ok,"result":result,"error":if ok{Value::Null}else{json!("unconfirmed stop")}})).unwrap();
            }
        });
        let config = StageConfig {
            host: "127.0.0.1".into(),
            port,
            token_env: String::new(),
            ..Default::default()
        };
        let first = RemoteStage::connect(&config).unwrap();
        let (tx, _rx) = mpsc::channel();
        let stage = StageClient::spawn(tx);
        stage.epoch.store(1, Ordering::Release);
        *stage.remote.lock().unwrap() = Some((1, first));
        stage.connected.store(true, Ordering::Release);
        stage.stop();
        let deadline = Instant::now() + Duration::from_secs(5);
        while stage.stop_barrier.0.lock().unwrap().pending {
            assert!(Instant::now() < deadline, "Stop did not finish");
            thread::sleep(Duration::from_millis(5));
        }
        assert!(stage.stop_barrier.0.lock().unwrap().unconfirmed.is_some());

        let retry = RemoteStage::connect(&config).unwrap();
        stage.epoch.store(2, Ordering::Release);
        *stage.remote.lock().unwrap() = Some((2, retry));
        stage.connected.store(true, Ordering::Release);
        stage.stop();
        let deadline = Instant::now() + Duration::from_secs(5);
        while stage.stop_barrier.0.lock().unwrap().pending {
            assert!(Instant::now() < deadline, "Stop did not finish");
            thread::sleep(Duration::from_millis(5));
        }
        assert!(stage.stop_barrier.0.lock().unwrap().unconfirmed.is_some());
        stage.shutdown();
        server.join().unwrap();
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
