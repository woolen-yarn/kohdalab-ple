//! Safe attachment to the GUI-owned KohdaLab shutter API (protocol 2).
//!
//! This module deliberately does not expose `discover` or `connect`: the
//! shutter GUI is the only program allowed to own the Pico connection.  The
//! state reported below is the GUI/controller's `STATE` feedback, not an
//! optical sensor measurement.
use serde_json::{Map, Value, json};
use std::{
    fs,
    io::{Read, Write},
    net::{IpAddr, Ipv4Addr, SocketAddr, TcpStream},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

const MAX_MESSAGE: usize = 64 * 1024;
const HEARTBEAT: Duration = Duration::from_secs(2);

#[derive(Clone)]
struct Endpoint {
    port: u16,
    token: String,
}

#[derive(Clone)]
struct Pending {
    request_id: String,
    desired: &'static str,
}

#[derive(Default)]
struct State {
    controller: Value,
    pending: Option<Pending>,
    close_pending: bool,
    close_error: Option<String>,
    heartbeat_error: Option<String>,
}

/// An exclusive PLE lease of an already connected KohdaLab Shutter GUI.
pub struct ExternalShutter {
    endpoint: Endpoint,
    client: String,
    lease: Mutex<Option<String>>,
    channel: usize,
    timeout: Duration,
    cancel: Arc<AtomicBool>,
    transition: Mutex<()>,
    state: Mutex<State>,
    cancel_hook: Mutex<Option<Arc<dyn Fn() + Send + Sync>>>,
    shutdown_started: AtomicBool,
    heartbeat_stop: AtomicBool,
    heartbeat: Mutex<Option<thread::JoinHandle<()>>>,
}

impl ExternalShutter {
    pub fn connect(
        endpoint: Option<PathBuf>,
        channel: usize,
        timeout_s: f64,
        cancel: Arc<AtomicBool>,
    ) -> Result<Arc<Self>, String> {
        if !timeout_s.is_finite() || timeout_s <= 0.0 {
            return Err("KohdaLab shutter timeout must be a positive finite number".into());
        }
        let timeout = Duration::from_secs_f64(timeout_s.clamp(0.05, 120.0));
        let endpoint = select_endpoint(endpoint, timeout)?;
        let client = crate::remote_gui::new_id()?;
        let status = request(&endpoint, timeout, "status", Map::new())?;
        validate_status(&status, channel)?;
        if global_busy(&status)? {
            return Err("KohdaLab Shutter GUI is busy; wait for its current command before PLE acquires control".into());
        }
        let mut acquire = Map::new();
        acquire.insert("client".into(), Value::String(client.clone()));
        acquire.insert(
            "label".into(),
            Value::String("KohdaLab PLE automatic BG".into()),
        );
        let acquired = request(&endpoint, timeout, "acquire", acquire)?;
        let lease = acquired
            .get("lease")
            .and_then(Value::as_str)
            .filter(|v| !v.is_empty())
            .ok_or("KohdaLab shutter GUI returned an invalid lease")?
            .to_owned();
        let shutter = Arc::new(Self {
            endpoint,
            client,
            lease: Mutex::new(Some(lease)),
            channel,
            timeout,
            cancel,
            transition: Mutex::new(()),
            state: Mutex::new(State {
                controller: status,
                ..State::default()
            }),
            cancel_hook: Mutex::new(None),
            shutdown_started: AtomicBool::new(false),
            heartbeat_stop: AtomicBool::new(false),
            heartbeat: Mutex::new(None),
        });
        if let Err(error) = shutter.start_heartbeat() {
            let cleanup = shutter.shutdown();
            return Err(match cleanup {
                Ok(()) => format!(
                    "start shutter heartbeat: {error}; shutter was closed and lease released"
                ),
                Err(cleanup) => {
                    format!("start shutter heartbeat: {error}; cleanup is unconfirmed: {cleanup}")
                }
            });
        }
        Ok(shutter)
    }

    /// Install the non-blocking instrument-stop action used if the renewable
    /// GUI lease is lost while an acquisition is in progress.
    pub fn set_cancel_hook(&self, hook: Arc<dyn Fn() + Send + Sync>) {
        if let Ok(mut stored) = self.cancel_hook.lock() {
            *stored = Some(hook);
        }
    }

    /// Set beam-blocked (`true` => Close) and wait for controller STATE.
    pub fn set_closed(&self, closed: bool) -> Result<Value, String> {
        if !closed {
            self.ensure_open_permitted()?;
        }
        let _transition = self
            .transition
            .lock()
            .map_err(|_| "shutter transition lock poisoned")?;
        if !closed {
            self.ensure_open_permitted()?;
        }
        self.resolve_pending()?;
        if !closed {
            self.ensure_open_permitted()?;
        }
        let desired = if closed { "Close" } else { "Open" };
        let current = self.wait_idle()?;
        // A Stop can occur while resolving an earlier action or waiting for
        // controller idle.  Check immediately before any new Open mutation.
        if !closed {
            self.ensure_open_permitted()?;
        }
        if channel_state(&current, self.channel)? == desired {
            return Ok(self.snapshot());
        }
        self.mutate_and_confirm(if closed { "close" } else { "open" }, desired)?;
        Ok(self.snapshot())
    }

    /// A redacted controller snapshot suitable for run metadata and the UI.
    pub fn snapshot(&self) -> Value {
        let (controller, close_pending, close_error, heartbeat_error) = self
            .state
            .lock()
            .map(|v| {
                (
                    redact(&v.controller),
                    v.close_pending,
                    v.close_error.clone(),
                    v.heartbeat_error.clone(),
                )
            })
            .unwrap_or((Value::Null, false, None, None));
        json!({
            "backend": "kohdalab-shutter-gui",
            "connected": controller.get("connected").and_then(Value::as_bool).unwrap_or(false),
            "channel": self.channel,
            "feedback": {
                "kind": "controller_STATE",
                "physical_optical_sensor": false,
                "description": "GUI/Pico software-commanded state; it is not confirmation from an optical sensor"
            },
            "controller": controller,
            "close_pending": close_pending,
            "close_error": close_error,
            "heartbeat_error": heartbeat_error,
            "lease_retained": self.lease.lock().map(|v| v.is_some()).unwrap_or(false),
        })
    }

    /// Request Close in a detached worker.  It never issues Open after Stop.
    pub fn request_close(self: &Arc<Self>) {
        let mut state = match self.state.lock() {
            Ok(v) => v,
            Err(_) => return,
        };
        if state.close_pending {
            return;
        }
        state.close_pending = true;
        state.close_error = None;
        drop(state);
        let this = Arc::clone(self);
        let worker = thread::Builder::new()
            .name("ple-shutter-close".into())
            .spawn(move || {
                let result = this.set_closed(true).map(|_| ());
                if let Ok(mut state) = this.state.lock() {
                    state.close_pending = false;
                    state.close_error = result.err();
                }
            });
        if let Err(error) = worker
            && let Ok(mut state) = self.state.lock()
        {
            state.close_pending = false;
            state.close_error = Some(format!(
                "could not start shutter close worker; closure is unconfirmed: {error}"
            ));
        }
    }

    /// `(close_is_pending, last_close_or_heartbeat_error)`.
    pub fn stop_status(&self) -> (bool, Option<String>) {
        self.state
            .lock()
            .map(|s| {
                (
                    s.close_pending,
                    s.close_error.clone().or_else(|| s.heartbeat_error.clone()),
                )
            })
            .unwrap_or((true, Some("shutter state lock poisoned".into())))
    }

    /// Close with controller STATE confirmation, then release the GUI lease.
    pub fn shutdown(&self) -> Result<(), String> {
        self.shutdown_started.store(true, Ordering::Release);
        self.set_closed(true)?;
        // Keep the lease (and heartbeat) until the release acknowledgement.
        // A transient transport failure can then be retried by another
        // shutdown/cleanup call without reopening the shutter.
        let lease = self
            .lease
            .lock()
            .map_err(|_| "shutter lease lock poisoned")?
            .clone();
        if let Some(lease) = lease {
            let mut body = Map::new();
            body.insert("client".into(), Value::String(self.client.clone()));
            body.insert("lease".into(), Value::String(lease));
            match self.call("release", body) {
                Ok(_) => self.clear_lease()?,
                Err(error) => {
                    // The GUI removes ownership before it writes the response.
                    // A lost ACK is therefore recoverable only from a fresh,
                    // settled controller state that proves nobody owns it.
                    let status = self.status().map_err(|probe| format!("shutter release was not acknowledged ({error}); recovery status failed: {probe}"))?;
                    if status.get("owner") != Some(&Value::Null)
                        || global_busy(&status)?
                        || channel_state(&status, self.channel)? != "Close"
                        || channel_busy(&status, self.channel)?
                    {
                        return Err(format!(
                            "shutter release was not acknowledged and safe recovery is not proven: {error}"
                        ));
                    }
                    self.clear_lease()?;
                }
            }
        }
        self.heartbeat_stop.store(true, Ordering::Release);
        if let Some(worker) = self
            .heartbeat
            .lock()
            .map_err(|_| "heartbeat lock poisoned")?
            .take()
            && worker.thread().id() != thread::current().id()
        {
            let _ = worker.join();
        }
        Ok(())
    }

    fn start_heartbeat(self: &Arc<Self>) -> Result<(), String> {
        let weak = Arc::downgrade(self);
        let handle = thread::Builder::new()
            .name("ple-shutter-heartbeat".into())
            .spawn(move || {
                loop {
                    thread::sleep(HEARTBEAT);
                    let Some(this) = weak.upgrade() else { return };
                    if this.heartbeat_stop.load(Ordering::Acquire) {
                        return;
                    }
                    let lease = this.lease.lock().ok().and_then(|v| v.clone());
                    let Some(lease) = lease else { return };
                    let mut body = Map::new();
                    body.insert("client".into(), Value::String(this.client.clone()));
                    body.insert("lease".into(), Value::String(lease));
                    if let Err(error) = this.call("heartbeat", body) {
                        let first_loss = !this.cancel.swap(true, Ordering::AcqRel);
                        let hook = if first_loss {
                            this.cancel_hook.lock().ok().and_then(|v| v.clone())
                        } else {
                            None
                        };
                        if let Ok(mut state) = this.state.lock() {
                            state.heartbeat_error = Some(format!(
                                "Shutter lease heartbeat lost; closure is unconfirmed: {error}"
                            ));
                        }
                        // Retain the last known lease.  It may still be valid
                        // during the GUI's ten-second expiry window, allowing a
                        // best-effort Close or Release after transient loss.
                        if let Some(hook) = hook {
                            hook();
                        }
                        return;
                    }
                }
            })
            .map_err(|e| format!("start shutter heartbeat: {e}"))?;
        *self
            .heartbeat
            .lock()
            .map_err(|_| "heartbeat lock poisoned")? = Some(handle);
        Ok(())
    }

    fn resolve_pending(&self) -> Result<(), String> {
        let pending = self
            .state
            .lock()
            .map_err(|_| "shutter state lock poisoned")?
            .pending
            .clone();
        let Some(pending) = pending else {
            return Ok(());
        };
        // Do not create another mutation while an accepted/lost one is unresolved.
        let deadline = Instant::now() + self.timeout;
        loop {
            let mut body = Map::new();
            body.insert("id".into(), Value::String(pending.request_id.clone()));
            let receipt = self.call("result", body).map_err(|e| {
                format!(
                    "previous shutter {} is unconfirmed ({e}); refusing a new transition",
                    pending.desired
                )
            })?;
            match receipt.get("status").and_then(Value::as_str) {
                Some("completed") | Some("error") => break,
                Some("running") if Instant::now() < deadline => {
                    thread::sleep(Duration::from_millis(50))
                }
                _ => {
                    return Err(format!(
                        "previous shutter {} is still pending; refusing a new transition",
                        pending.desired
                    ));
                }
            }
        }
        // A fresh non-busy STATE is required even after a receipt, so a late Open
        // can never be hidden behind a subsequently acknowledged Close.
        let _ = self.wait_idle()?;
        self.state
            .lock()
            .map_err(|_| "shutter state lock poisoned")?
            .pending = None;
        Ok(())
    }

    fn mutate_and_confirm(&self, method: &str, desired: &'static str) -> Result<(), String> {
        let request_id = crate::remote_gui::new_id()?;
        let lease = self
            .lease
            .lock()
            .map_err(|_| "shutter lease lock poisoned")?
            .clone()
            .ok_or("KohdaLab shutter lease is no longer valid; closure is unconfirmed")?;
        let mut body = Map::new();
        body.insert("channel".into(), json!(self.channel));
        body.insert("client".into(), Value::String(self.client.clone()));
        body.insert("lease".into(), Value::String(lease));
        body.insert("request_id".into(), Value::String(request_id.clone()));
        let receipt = match self.call(method, body) {
            Ok(v) => v,
            Err(e) => return self.mark_unconfirmed(request_id, desired, e),
        };
        match receipt.get("status").and_then(Value::as_str) {
            Some("error") => return Err(receipt_error(&receipt)),
            Some("completed") => {}
            Some("running") => {
                if let Err(error) = self.wait_receipt(&request_id, desired) {
                    return self.mark_unconfirmed(request_id, desired, error);
                }
            }
            _ => {
                return self.mark_unconfirmed(
                    request_id,
                    desired,
                    "invalid shutter receipt".into(),
                );
            }
        }
        if let Err(error) = self.wait_target(desired) {
            return self.mark_unconfirmed(request_id, desired, error);
        }
        Ok(())
    }

    fn wait_receipt(&self, request_id: &str, desired: &'static str) -> Result<(), String> {
        let deadline = Instant::now() + self.timeout;
        loop {
            let mut body = Map::new();
            body.insert("id".into(), Value::String(request_id.into()));
            let receipt = self.call("result", body)?;
            match receipt.get("status").and_then(Value::as_str) {
                Some("completed") => return Ok(()),
                Some("error") => return Err(receipt_error(&receipt)),
                Some("running") if Instant::now() < deadline => {
                    thread::sleep(Duration::from_millis(50))
                }
                _ => {
                    return Err(format!(
                        "shutter {desired} receipt did not complete before timeout"
                    ));
                }
            }
        }
    }

    fn wait_target(&self, desired: &str) -> Result<(), String> {
        let deadline = Instant::now() + self.timeout;
        loop {
            let status = self.status()?;
            if !global_busy(&status)?
                && channel_state(&status, self.channel)? == desired
                && !channel_busy(&status, self.channel)?
            {
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err(format!("timed out waiting for controller STATE {desired}"));
            }
            thread::sleep(Duration::from_millis(50));
        }
    }

    fn wait_idle(&self) -> Result<Value, String> {
        let deadline = Instant::now() + self.timeout;
        loop {
            let status = self.status()?;
            if !global_busy(&status)? && !channel_busy(&status, self.channel)? {
                return Ok(status);
            }
            if Instant::now() >= deadline {
                return Err("timed out waiting for shutter controller to become idle".into());
            }
            thread::sleep(Duration::from_millis(50));
        }
    }

    fn status(&self) -> Result<Value, String> {
        let status = self.call("status", Map::new())?;
        validate_status(&status, self.channel)?;
        self.state
            .lock()
            .map_err(|_| "shutter state lock poisoned")?
            .controller = status.clone();
        Ok(status)
    }

    fn mark_unconfirmed<T>(
        &self,
        request_id: String,
        desired: &'static str,
        error: String,
    ) -> Result<T, String> {
        let message = format!(
            "shutter {desired} is unconfirmed; retained request {request_id} and blocked further transitions: {error}"
        );
        if let Ok(mut state) = self.state.lock() {
            state.pending = Some(Pending {
                request_id,
                desired,
            });
        }
        Err(message)
    }

    fn call(&self, method: &str, body: Map<String, Value>) -> Result<Value, String> {
        request(&self.endpoint, self.timeout, method, body)
    }

    fn ensure_open_permitted(&self) -> Result<(), String> {
        if self.cancel.load(Ordering::Acquire) || self.shutdown_started.load(Ordering::Acquire) {
            Err("PLE is stopping; refusing to issue a new shutter Open".into())
        } else {
            Ok(())
        }
    }

    fn clear_lease(&self) -> Result<(), String> {
        *self
            .lease
            .lock()
            .map_err(|_| "shutter lease lock poisoned")? = None;
        Ok(())
    }
}

fn select_endpoint(explicit: Option<PathBuf>, timeout: Duration) -> Result<Endpoint, String> {
    if let Some(path) = explicit {
        if !path.is_file() {
            return Err(format!(
                "shutter GUI endpoint must be a descriptor file: {}",
                path.display()
            ));
        }
        let endpoint = load_endpoint(&path)?;
        let _ = request(&endpoint, timeout, "capabilities", Map::new())?;
        return Ok(endpoint);
    }
    let base = control_directory().ok_or("cannot determine the shutter GUI control directory")?;
    let mut active = Vec::new();
    for entry in fs::read_dir(&base).map_err(|_| "no active KohdaLab Shutter GUI API was found")? {
        let Ok(entry) = entry else { continue };
        let path = entry.path();
        let name = path
            .file_name()
            .and_then(|v| v.to_str())
            .unwrap_or_default();
        if !name.starts_with("shutter-") || !name.ends_with(".json") {
            continue;
        }
        let Ok(candidate) = load_endpoint(&path) else {
            continue;
        };
        if request(
            &candidate,
            timeout.min(Duration::from_secs(1)),
            "capabilities",
            Map::new(),
        )
        .is_ok()
        {
            active.push(candidate);
        }
    }
    match active.len() {
        0 => Err("no active KohdaLab Shutter GUI API was found".into()),
        1 => Ok(active.remove(0)),
        _ => Err(
            "multiple active KohdaLab Shutter GUI APIs found; configure an explicit endpoint"
                .into(),
        ),
    }
}

fn control_directory() -> Option<PathBuf> {
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
        .map(|p| p.join(".kohdalab").join("control"))
}

fn load_endpoint(path: &Path) -> Result<Endpoint, String> {
    let value: Value = serde_json::from_slice(
        &fs::read(path)
            .map_err(|_| format!("cannot read shutter GUI endpoint: {}", path.display()))?,
    )
    .map_err(|_| format!("invalid shutter GUI endpoint: {}", path.display()))?;
    if value.get("app").and_then(Value::as_str) != Some("kohdalab-shutter")
        || value.get("protocol").and_then(Value::as_u64) != Some(2)
    {
        return Err(format!("invalid shutter GUI endpoint: {}", path.display()));
    }
    if value.get("host").and_then(Value::as_str) != Some("127.0.0.1") {
        return Err("shutter GUI endpoint is not loopback".into());
    }
    let port = value
        .get("port")
        .and_then(Value::as_u64)
        .filter(|v| (1..=65535).contains(v))
        .ok_or("invalid shutter GUI endpoint port")? as u16;
    let token = value
        .get("token")
        .and_then(Value::as_str)
        .filter(|v| !v.is_empty())
        .ok_or("invalid shutter GUI endpoint token")?
        .to_owned();
    if value
        .get("instance_id")
        .and_then(Value::as_str)
        .filter(|v| !v.is_empty())
        .is_none()
    {
        return Err("invalid shutter GUI endpoint instance identifier".into());
    }
    Ok(Endpoint { port, token })
}

fn request(
    endpoint: &Endpoint,
    timeout: Duration,
    method: &str,
    mut body: Map<String, Value>,
) -> Result<Value, String> {
    // The upstream service applies a two-second absolute deadline.  Match it
    // locally: a peer that trickles one byte at a time cannot keep this socket
    // alive indefinitely.
    let deadline = Instant::now() + timeout.min(Duration::from_secs(2));
    body.insert("token".into(), Value::String(endpoint.token.clone()));
    body.insert("method".into(), Value::String(method.into()));
    let encoded = serde_json::to_vec(&Value::Object(body))
        .map_err(|_| "cannot encode shutter GUI request")?;
    if encoded.len() + 1 > MAX_MESSAGE {
        return Err("shutter GUI request is too large".into());
    }
    let address = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), endpoint.port);
    let mut stream = TcpStream::connect_timeout(&address, remaining(deadline)?)
        .map_err(|_| format!("could not reach KohdaLab Shutter GUI for {method}"))?;
    stream
        .set_read_timeout(Some(remaining(deadline)?))
        .map_err(|e| e.to_string())?;
    let mut wire = encoded;
    wire.push(b'\n');
    let mut written = 0;
    while written < wire.len() {
        stream
            .set_write_timeout(Some(remaining(deadline)?))
            .map_err(|e| e.to_string())?;
        let n = stream
            .write(&wire[written..])
            .map_err(|_| format!("could not send {method} to KohdaLab Shutter GUI"))?;
        if n == 0 {
            return Err(format!("could not send {method} to KohdaLab Shutter GUI"));
        }
        written += n;
    }
    let mut response = Vec::new();
    let mut byte = [0; 1];
    loop {
        stream
            .set_read_timeout(Some(remaining(deadline)?))
            .map_err(|e| e.to_string())?;
        let n = stream
            .read(&mut byte)
            .map_err(|_| format!("KohdaLab Shutter GUI did not reply to {method}"))?;
        if n == 0 {
            return Err(format!(
                "KohdaLab Shutter GUI closed without replying to {method}"
            ));
        }
        if byte[0] == b'\n' {
            break;
        }
        response.push(byte[0]);
        if response.len() > MAX_MESSAGE {
            return Err("shutter GUI response is too large".into());
        }
    }
    let envelope: Value =
        serde_json::from_slice(&response).map_err(|_| "invalid shutter GUI response")?;
    let ok = envelope
        .get("ok")
        .and_then(Value::as_bool)
        .ok_or("invalid shutter GUI response envelope")?;
    if !ok {
        let code = envelope
            .get("error_code")
            .and_then(Value::as_str)
            .unwrap_or("rejected");
        let error = envelope
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or("GUI rejected request");
        return Err(format!("shutter GUI {method} rejected ({code}): {error}"));
    }
    envelope
        .get("result")
        .cloned()
        .ok_or("shutter GUI response omitted result".into())
}

fn remaining(deadline: Instant) -> Result<Duration, String> {
    deadline
        .checked_duration_since(Instant::now())
        .filter(|v| !v.is_zero())
        .ok_or("KohdaLab Shutter GUI request exceeded absolute two-second deadline".into())
}

fn validate_status(status: &Value, channel: usize) -> Result<(), String> {
    if status.get("connected").and_then(Value::as_bool) != Some(true) {
        return Err(
            "KohdaLab Shutter GUI is not connected to a shutter; PLE will not connect hardware"
                .into(),
        );
    }
    if status.get("demo").and_then(Value::as_bool) != Some(false) {
        return Err(
            "KohdaLab Shutter GUI is in demo mode; hardware PLE refuses simulated shutter feedback"
                .into(),
        );
    }
    let state = channel_state(status, channel)?;
    let _ = global_busy(status)?;
    if state == "Unknown" {
        return Err("shutter firmware did not provide STATE feedback; PLE requires calibrated Open/Close controller state".into());
    }
    if state != "Open" && state != "Close" && state != "Moving" {
        return Err("invalid shutter controller STATE".into());
    }
    Ok(())
}
fn channel(status: &Value, index: usize) -> Result<&Value, String> {
    status
        .get("channels")
        .and_then(Value::as_object)
        .and_then(|v| v.get(&index.to_string()))
        .ok_or_else(|| format!("shutter channel CH{index} is not available"))
}
fn channel_state(status: &Value, index: usize) -> Result<&str, String> {
    channel(status, index)?
        .get("state")
        .and_then(Value::as_str)
        .ok_or("shutter STATE feedback is missing".into())
}
fn channel_busy(status: &Value, index: usize) -> Result<bool, String> {
    channel(status, index)?
        .get("busy")
        .and_then(Value::as_bool)
        .ok_or("shutter busy feedback is missing".into())
}
fn global_busy(status: &Value) -> Result<bool, String> {
    status
        .get("busy")
        .and_then(Value::as_bool)
        .ok_or("shutter global busy feedback is missing".into())
}
fn receipt_error(receipt: &Value) -> String {
    format!(
        "shutter command failed: {}",
        receipt
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or("unknown GUI error")
    )
}
fn redact(value: &Value) -> Value {
    match value {
        Value::Array(v) => Value::Array(v.iter().map(redact).collect()),
        Value::Object(v) => Value::Object(
            v.iter()
                .filter(|(k, _)| {
                    !k.eq_ignore_ascii_case("token") && !k.eq_ignore_ascii_case("lease")
                })
                .map(|(k, v)| (k.clone(), redact(v)))
                .collect(),
        ),
        _ => value.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{BufRead, BufReader},
        net::TcpListener,
        sync::atomic::{AtomicUsize, Ordering},
    };

    struct Mock {
        descriptor: PathBuf,
        stop: Arc<AtomicBool>,
        worker: Option<thread::JoinHandle<()>>,
    }
    impl Drop for Mock {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::Release);
            // Wake a nonblocking listener only after a bounded polling delay.
            if let Some(worker) = self.worker.take() {
                let _ = worker.join();
            }
            let _ = fs::remove_file(&self.descriptor);
            if let Some(parent) = self.descriptor.parent() {
                let _ = fs::remove_dir(parent);
            }
        }
    }
    fn mock<F>(handler: F) -> Mock
    where
        F: Fn(Value) -> Value + Send + Sync + 'static,
    {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        listener.set_nonblocking(true).unwrap();
        let port = listener.local_addr().unwrap().port();
        let directory = std::env::temp_dir().join(format!(
            "ple-shutter-test-{}-{}",
            std::process::id(),
            crate::remote_gui::new_id().unwrap()
        ));
        fs::create_dir_all(&directory).unwrap();
        let descriptor = directory.join("shutter-test.json");
        fs::write(&descriptor, serde_json::to_vec(&json!({"app":"kohdalab-shutter","protocol":2,"host":"127.0.0.1","port":port,"token":"test-token","instance_id":"test"})).unwrap()).unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = stop.clone();
        let handler = Arc::new(handler);
        let worker = thread::spawn(move || {
            while !stopped.load(Ordering::Acquire) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        stream.set_nonblocking(false).unwrap();
                        stream
                            .set_read_timeout(Some(Duration::from_secs(2)))
                            .unwrap();
                        stream
                            .set_write_timeout(Some(Duration::from_secs(2)))
                            .unwrap();
                        let mut reader = BufReader::new(stream.try_clone().unwrap());
                        let mut line = String::new();
                        if reader.read_line(&mut line).unwrap_or(0) > 0 {
                            let response = serde_json::from_str(&line)
                                .map(|request| handler(request))
                                .unwrap_or_else(|_| json!({"ok":false,"error":"bad request"}));
                            let mut writer = stream;
                            let _ = writer
                                .write_all(serde_json::to_string(&response).unwrap().as_bytes());
                            let _ = writer.write_all(b"\n");
                        }
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(2))
                    }
                    Err(_) => return,
                }
            }
        });
        Mock {
            descriptor,
            stop,
            worker: Some(worker),
        }
    }
    fn write_descriptor(endpoint: &Endpoint) -> PathBuf {
        let directory = std::env::temp_dir().join(format!(
            "ple-shutter-test-{}-{}",
            std::process::id(),
            crate::remote_gui::new_id().unwrap()
        ));
        fs::create_dir_all(&directory).unwrap();
        let descriptor = directory.join("shutter-test.json");
        fs::write(
            &descriptor,
            serde_json::to_vec(&json!({
                "app":"kohdalab-shutter", "protocol":2, "host":"127.0.0.1",
                "port":endpoint.port, "token":endpoint.token.clone(), "instance_id":"test"
            }))
            .unwrap(),
        )
        .unwrap();
        descriptor
    }
    fn controller(state: &str, busy: bool) -> Value {
        json!({"connected":true,"demo":false,"busy":false,"owner":null,"channels":{"0":{"state":state,"busy":busy,"position":0,"target":0}}})
    }
    #[test]
    fn protocol_transition_and_feedback_metadata() {
        let state = Arc::new(Mutex::new("Close".to_owned()));
        let calls = Arc::new(Mutex::new(Vec::<String>::new()));
        let mock = mock({
            let state = state.clone();
            let calls = calls.clone();
            move |request| {
                assert_eq!(request["token"], "test-token");
                let method = request["method"].as_str().unwrap().to_owned();
                calls.lock().unwrap().push(method.clone());
                match method.as_str() {
                    "status" => {
                        json!({"ok":true,"result":controller(&state.lock().unwrap(), false)})
                    }
                    "capabilities" | "heartbeat" | "release" => json!({"ok":true,"result":{}}),
                    "acquire" => json!({"ok":true,"result":{"lease":"lease"}}),
                    "open" => {
                        *state.lock().unwrap() = "Open".into();
                        json!({"ok":true,"result":{"id":"one","status":"completed"}})
                    }
                    "close" => {
                        *state.lock().unwrap() = "Close".into();
                        json!({"ok":true,"result":{"id":"two","status":"completed"}})
                    }
                    _ => json!({"ok":false,"error":"unexpected"}),
                }
            }
        });
        let shutter = ExternalShutter::connect(
            Some(mock.descriptor.clone()),
            0,
            0.2,
            Arc::new(AtomicBool::new(false)),
        )
        .unwrap();
        shutter.set_closed(false).unwrap();
        let snapshot = shutter.snapshot();
        assert_eq!(snapshot["feedback"]["physical_optical_sensor"], false);
        assert_eq!(snapshot["controller"]["channels"]["0"]["state"], "Open");
        shutter.shutdown().unwrap();
        let calls = calls.lock().unwrap();
        assert!(calls.iter().any(|v| v == "open") && calls.iter().any(|v| v == "close"));
        assert!(!calls.iter().any(|v| v == "connect" || v == "discover"));
    }
    #[test]
    fn rejects_missing_state_feedback() {
        let mock = mock(|request| match request["method"].as_str().unwrap() {
            "capabilities" => json!({"ok":true,"result":{}}),
            "status" => {
                json!({"ok":true,"result":{"connected":true,"demo":false,"busy":false,"channels":{"0":{"busy":false}}}})
            }
            _ => json!({"ok":false,"error":"unexpected"}),
        });
        let error = ExternalShutter::connect(
            Some(mock.descriptor.clone()),
            0,
            0.1,
            Arc::new(AtomicBool::new(false)),
        )
        .err()
        .unwrap();
        assert!(error.contains("STATE"));
    }
    #[test]
    fn command_error_is_not_confirmed_as_close_or_open() {
        let mock = mock(|request| match request["method"].as_str().unwrap() {
            "capabilities" => json!({"ok":true,"result":{}}),
            "status" => json!({"ok":true,"result":controller("Close", false)}),
            "acquire" => json!({"ok":true,"result":{"lease":"lease"}}),
            "open" => {
                json!({"ok":true,"result":{"id":"one","status":"error","error":"Pico refused command"}})
            }
            _ => json!({"ok":true,"result":{}}),
        });
        let shutter = ExternalShutter::connect(
            Some(mock.descriptor.clone()),
            0,
            0.1,
            Arc::new(AtomicBool::new(false)),
        )
        .unwrap();
        assert!(
            shutter
                .set_closed(false)
                .unwrap_err()
                .contains("Pico refused")
        );
    }
    #[test]
    fn rejected_lease_prevents_attachment() {
        let mock = mock(|request| match request["method"].as_str().unwrap() {
            "capabilities" => json!({"ok":true,"result":{}}),
            "status" => json!({"ok":true,"result":controller("Close", false)}),
            "acquire" => {
                json!({"ok":false,"error_code":"lease_busy","error":"held by another controller"})
            }
            _ => json!({"ok":true,"result":{}}),
        });
        let error = ExternalShutter::connect(
            Some(mock.descriptor.clone()),
            0,
            0.1,
            Arc::new(AtomicBool::new(false)),
        )
        .err()
        .unwrap();
        assert!(error.contains("lease_busy"));
    }
    #[test]
    fn stop_during_open_never_sends_a_second_open() {
        let opens = Arc::new(AtomicUsize::new(0));
        let mock = mock({
            let opens = opens.clone();
            move |request| match request["method"].as_str().unwrap() {
                "capabilities" => json!({"ok":true,"result":{}}),
                "status" => json!({"ok":true,"result":controller("Close", false)}),
                "acquire" => json!({"ok":true,"result":{"lease":"lease"}}),
                "open" => {
                    opens.fetch_add(1, Ordering::AcqRel);
                    json!({"ok":true,"result":{"id":"one","status":"running"}})
                }
                "result" => json!({"ok":true,"result":{"id":"one","status":"running"}}),
                _ => json!({"ok":true,"result":{}}),
            }
        });
        let cancel = Arc::new(AtomicBool::new(false));
        let shutter =
            ExternalShutter::connect(Some(mock.descriptor.clone()), 0, 0.06, cancel.clone())
                .unwrap();
        assert!(shutter.set_closed(false).is_err());
        cancel.store(true, Ordering::Release);
        shutter.request_close();
        let deadline = Instant::now() + Duration::from_secs(1);
        while shutter.stop_status().0 && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(opens.load(Ordering::Acquire), 1);
        assert!(
            shutter
                .stop_status()
                .1
                .unwrap_or_default()
                .contains("pending")
        );
    }

    #[test]
    fn cancel_during_idle_poll_prevents_open() {
        let cancel = Arc::new(AtomicBool::new(false));
        let opens = Arc::new(AtomicUsize::new(0));
        let status_count = Arc::new(AtomicUsize::new(0));
        let mock = mock({
            let cancel = cancel.clone();
            let opens = opens.clone();
            let status_count = status_count.clone();
            move |request| match request["method"].as_str().unwrap() {
                "capabilities" => json!({"ok":true,"result":{}}),
                "status" => {
                    // connect() consumes the first status.  Latch Stop while
                    // the next idle poll is returning a perfectly idle Close.
                    if status_count.fetch_add(1, Ordering::AcqRel) >= 1 {
                        cancel.store(true, Ordering::Release);
                    }
                    json!({"ok":true,"result":controller("Close", false)})
                }
                "acquire" => json!({"ok":true,"result":{"lease":"lease"}}),
                "open" => {
                    opens.fetch_add(1, Ordering::AcqRel);
                    json!({"ok":true,"result":{"id":"bad","status":"completed"}})
                }
                _ => json!({"ok":true,"result":{}}),
            }
        });
        let shutter =
            ExternalShutter::connect(Some(mock.descriptor.clone()), 0, 0.1, cancel).unwrap();
        assert!(shutter.set_closed(false).unwrap_err().contains("refusing"));
        assert_eq!(opens.load(Ordering::Acquire), 0);
    }

    #[test]
    fn trickle_response_cannot_extend_absolute_deadline() {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let worker = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut request = String::new();
            let _ = reader.read_line(&mut request);
            for byte in b"{\"ok\":true,\"result\":{}}\n" {
                if stream.write_all(&[*byte]).is_err() {
                    return;
                }
                thread::sleep(Duration::from_millis(5));
            }
        });
        let start = Instant::now();
        let error = request(
            &Endpoint {
                port,
                token: "test-token".into(),
            },
            Duration::from_millis(60),
            "status",
            Map::new(),
        )
        .unwrap_err();
        assert!(
            start.elapsed() < Duration::from_millis(160),
            "request did not honor absolute deadline"
        );
        assert!(error.contains("did not reply") || error.contains("deadline"));
        let _ = worker.join();
    }

    #[test]
    fn failed_release_retains_lease_for_cleanup_retry() {
        for owner in [Some(json!("another controller")), None] {
            let releases = Arc::new(AtomicUsize::new(0));
            let mock = mock({
                let releases = releases.clone();
                move |request| match request["method"].as_str().unwrap() {
                    "capabilities" => json!({"ok":true,"result":{}}),
                    "status" => {
                        let mut status = controller("Close", false);
                        if releases.load(Ordering::Acquire) == 1 {
                            if let Some(owner) = &owner {
                                status["owner"] = owner.clone();
                            } else {
                                status.as_object_mut().unwrap().remove("owner");
                            }
                        }
                        json!({"ok":true,"result":status})
                    }
                    "acquire" => json!({"ok":true,"result":{"lease":"lease"}}),
                    "release" if releases.fetch_add(1, Ordering::AcqRel) == 0 => {
                        json!({"ok":false,"error_code":"transport","error":"temporary failure"})
                    }
                    "release" => json!({"ok":true,"result":{}}),
                    _ => json!({"ok":true,"result":{}}),
                }
            });
            let shutter = ExternalShutter::connect(
                Some(mock.descriptor.clone()),
                0,
                0.1,
                Arc::new(AtomicBool::new(false)),
            )
            .unwrap();
            assert!(shutter.shutdown().is_err());
            assert!(shutter.shutdown().is_ok());
            assert_eq!(releases.load(Ordering::Acquire), 2);
            assert!(shutter.set_closed(false).unwrap_err().contains("refusing"));
        }
    }

    #[test]
    fn global_busy_other_channel_blocks_bg_open_until_idle() {
        let statuses = Arc::new(AtomicUsize::new(0));
        let opens = Arc::new(AtomicUsize::new(0));
        let mock = mock({
            let statuses = statuses.clone();
            let opens = opens.clone();
            move |request| match request["method"].as_str().unwrap() {
                "capabilities" => json!({"ok":true,"result":{}}),
                "status" => {
                    let number = statuses.fetch_add(1, Ordering::AcqRel);
                    let mut status = controller("Close", false);
                    // The selected channel is already Close.  Another channel's
                    // command remains globally busy for this one poll.
                    if number == 1 {
                        status["busy"] = json!(true);
                    }
                    json!({"ok":true,"result":status})
                }
                "acquire" => json!({"ok":true,"result":{"lease":"lease"}}),
                "open" => {
                    assert!(statuses.load(Ordering::Acquire) >= 3);
                    opens.fetch_add(1, Ordering::AcqRel);
                    json!({"ok":true,"result":{"id":"one","status":"completed"}})
                }
                _ => json!({"ok":true,"result":{}}),
            }
        });
        let shutter = ExternalShutter::connect(
            Some(mock.descriptor.clone()),
            0,
            0.2,
            Arc::new(AtomicBool::new(false)),
        )
        .unwrap();
        // The mock leaves CH0 Close after the Open receipt, so this must fail
        // state confirmation; the assertion above proves the Open was not
        // emitted during the other-channel busy interval.
        assert!(shutter.set_closed(false).is_err());
        assert_eq!(opens.load(Ordering::Acquire), 1);
    }

    #[test]
    fn initial_global_busy_rejects_attachment() {
        let mock = mock(|request| match request["method"].as_str().unwrap() {
            "capabilities" => json!({"ok":true,"result":{}}),
            "status" => {
                json!({"ok":true,"result":{"connected":true,"demo":false,"busy":true,"channels":{"0":{"state":"Close","busy":false}}}})
            }
            _ => panic!("must not acquire while global busy"),
        });
        let error = ExternalShutter::connect(
            Some(mock.descriptor.clone()),
            0,
            0.1,
            Arc::new(AtomicBool::new(false)),
        )
        .err()
        .unwrap();
        assert!(error.contains("busy"));
    }

    #[test]
    fn heartbeat_loss_latches_cancel_hook_and_keeps_lease_visible() {
        let cancel = Arc::new(AtomicBool::new(false));
        let called = Arc::new(AtomicBool::new(false));
        let mock = mock(|request| match request["method"].as_str().unwrap() {
            "capabilities" => json!({"ok":true,"result":{}}),
            "status" => json!({"ok":true,"result":controller("Close", false)}),
            "acquire" => json!({"ok":true,"result":{"lease":"lease"}}),
            "heartbeat" => {
                json!({"ok":false,"error_code":"transport","error":"simulated link loss"})
            }
            "release" => json!({"ok":true,"result":{}}),
            _ => json!({"ok":true,"result":{}}),
        });
        let shutter =
            ExternalShutter::connect(Some(mock.descriptor.clone()), 0, 0.1, cancel.clone())
                .unwrap();
        shutter.set_cancel_hook(Arc::new({
            let called = called.clone();
            move || called.store(true, Ordering::Release)
        }));
        let deadline = Instant::now() + Duration::from_secs(3);
        while !called.load(Ordering::Acquire) && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(10));
        }
        let snapshot = shutter.snapshot();
        assert!(cancel.load(Ordering::Acquire));
        assert!(called.load(Ordering::Acquire));
        assert!(
            snapshot["heartbeat_error"]
                .as_str()
                .unwrap_or_default()
                .contains("heartbeat lost")
        );
        assert_eq!(snapshot["lease_retained"], true);
        shutter.shutdown().unwrap();
    }

    #[test]
    fn lost_release_ack_recovers_only_from_ownerless_settled_status() {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        listener.set_nonblocking(true).unwrap();
        let endpoint = Endpoint {
            port: listener.local_addr().unwrap().port(),
            token: "test-token".into(),
        };
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = stop.clone();
        let releases = Arc::new(AtomicUsize::new(0));
        let release_count = releases.clone();
        let worker = thread::spawn(move || {
            while !stopped.load(Ordering::Acquire) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        stream.set_nonblocking(false).unwrap();
                        stream
                            .set_read_timeout(Some(Duration::from_secs(2)))
                            .unwrap();
                        stream
                            .set_write_timeout(Some(Duration::from_secs(2)))
                            .unwrap();
                        let mut line = String::new();
                        let _ = BufReader::new(stream.try_clone().unwrap()).read_line(&mut line);
                        let method = serde_json::from_str::<Value>(&line).ok().and_then(|v| {
                            v.get("method").and_then(Value::as_str).map(str::to_owned)
                        });
                        if method.as_deref() == Some("release")
                            && release_count.fetch_add(1, Ordering::AcqRel) == 0
                        {
                            // GUI release has committed owner=None but this TCP ACK
                            // is lost.  A second release would now be unauthorized.
                            continue;
                        }
                        let response = match method.as_deref() {
                            Some("capabilities") => json!({"ok":true,"result":{}}),
                            Some("status") => {
                                json!({"ok":true,"result":controller("Close", false)})
                            }
                            Some("acquire") => json!({"ok":true,"result":{"lease":"lease"}}),
                            Some("release") => {
                                json!({"ok":false,"error_code":"not_owner","error":"lease was already released"})
                            }
                            _ => json!({"ok":false,"error":"unexpected"}),
                        };
                        let _ =
                            stream.write_all(serde_json::to_string(&response).unwrap().as_bytes());
                        let _ = stream.write_all(b"\n");
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(2))
                    }
                    Err(_) => return,
                }
            }
        });
        let shutter = ExternalShutter::connect(
            Some(write_descriptor(&endpoint)),
            0,
            0.1,
            Arc::new(AtomicBool::new(false)),
        )
        .unwrap();
        assert!(shutter.shutdown().is_ok());
        assert_eq!(
            releases.load(Ordering::Acquire),
            1,
            "must not retry release after ownerless ACK-loss recovery"
        );
        stop.store(true, Ordering::Release);
        let _ = worker.join();
    }
}
