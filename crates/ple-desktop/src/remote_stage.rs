//! Client for the deliberately small, remote-first CONEX bridge protocol.
//!
//! Each operation opens its own connection.  This is intentional: a status or
//! emergency stop is not queued behind a slow move request.  Name resolution is
//! provided by the standard library and therefore has no separately configurable
//! timeout; the client never retries a failed request automatically.

use ple_core::StageConfig;
use serde_json::{Value, json};
use std::{
    env,
    io::{BufRead, BufReader, Read, Write},
    net::{TcpStream, ToSocketAddrs},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

const REVISION: u64 = 1;
const MAX_RESPONSE_LINE: usize = 64 * 1024;
const READY_STATES: &[&str] = &["32", "33", "34"];

#[derive(Clone, Debug, PartialEq)]
pub struct RemoteInfo {
    pub position_mm: f64,
    pub min_mm: f64,
    pub max_mm: f64,
    pub state: String,
}

#[derive(Clone)]
pub struct RemoteStage {
    endpoint: Endpoint,
    info: RemoteInfo,
}

#[derive(Clone)]
struct Endpoint {
    host: String,
    port: u16,
    token_env: Option<String>,
    tolerance_mm: f64,
    move_timeout_s: f64,
    user_min_mm: f64,
    user_max_mm: f64,
    next_id: Arc<AtomicU64>,
}

impl RemoteStage {
    /// Connect without changing the controller state.  Only `hello` and
    /// `connect` are sent here; this method never homes, enables, or moves.
    pub fn connect(config: &StageConfig) -> Result<Self, String> {
        validate_config(config)?;
        let token_env = if config.token_env.trim().is_empty() {
            None
        } else {
            Some(config.token_env.clone())
        };
        let endpoint = Endpoint {
            host: config.host.clone(),
            port: config.port,
            token_env,
            tolerance_mm: config.tolerance_mm,
            move_timeout_s: config.timeout_s,
            user_min_mm: config.min_mm,
            user_max_mm: config.max_mm,
            next_id: Arc::new(AtomicU64::new(1)),
        };
        Self::establish(endpoint, &config.com_port)
    }

    fn establish(endpoint: Endpoint, com_port: &str) -> Result<Self, String> {
        let hello = endpoint.request("hello", json!({}), Duration::from_secs(5))?;
        expect_hello(&hello)?;

        let connected = endpoint.request(
            "connect",
            json!({ "com_port": com_port }),
            Duration::from_secs(5),
        )?;
        let info = parse_info(&connected, "connect", true)?;
        if endpoint.user_min_mm < info.min_mm || endpoint.user_max_mm > info.max_mm {
            return Err(format!(
                "configured stage limits {:.6}–{:.6} mm exceed remote physical limits {:.6}–{:.6} mm",
                endpoint.user_min_mm, endpoint.user_max_mm, info.min_mm, info.max_mm
            ));
        }
        if !within(info.position_mm, endpoint.user_min_mm, endpoint.user_max_mm) {
            return Err("remote stage position is outside configured software limits".into());
        }
        Ok(Self { endpoint, info })
    }

    pub fn info(&self) -> &RemoteInfo {
        &self.info
    }

    /// Readback is permitted while the controller reports its moving state.
    pub fn position(&self) -> Result<f64, String> {
        let response = self
            .endpoint
            .request("status", json!({}), Duration::from_secs(5))?;
        let info = parse_info(&response, "status", false)?;
        self.validate_readback(&info)?;
        Ok(info.position_mm)
    }

    pub fn move_to(&self, target: f64) -> Result<f64, String> {
        self.validate_target(target)?;
        let timeout = seconds(self.endpoint.move_timeout_s + 5.0, "move timeout")?;
        let response = self.endpoint.request(
            "move",
            json!({
                "position_mm": target,
                "tolerance_mm": self.endpoint.tolerance_mm,
                "timeout_s": self.endpoint.move_timeout_s,
            }),
            timeout,
        )?;
        let info = parse_info(&response, "move", true)?;
        self.validate_readback(&info)?;
        if (info.position_mm - target).abs() > self.endpoint.tolerance_mm {
            return Err(format!(
                "move readback {:.6} mm differs from target {:.6} mm by more than tolerance {:.6} mm",
                info.position_mm, target, self.endpoint.tolerance_mm
            ));
        }
        Ok(info.position_mm)
    }

    pub fn stop(&self) -> Result<f64, String> {
        let response =
            self.endpoint
                .request("stop", json!({ "timeout_s": 5.0 }), Duration::from_secs(10))?;
        let info = parse_info(&response, "stop", true)?;
        self.validate_readback(&info)?;
        Ok(info.position_mm)
    }

    fn validate_target(&self, target: f64) -> Result<(), String> {
        finite(target, "target position")?;
        if !within(target, self.info.min_mm, self.info.max_mm)
            || !within(target, self.endpoint.user_min_mm, self.endpoint.user_max_mm)
        {
            return Err(format!(
                "target {:.6} mm is outside allowed limits {:.6}–{:.6} mm",
                target, self.endpoint.user_min_mm, self.endpoint.user_max_mm
            ));
        }
        Ok(())
    }

    fn validate_readback(&self, info: &RemoteInfo) -> Result<(), String> {
        if (info.min_mm - self.info.min_mm).abs() > f64::EPSILON
            || (info.max_mm - self.info.max_mm).abs() > f64::EPSILON
        {
            return Err("remote physical stage limits changed during this session".into());
        }
        if !within(info.position_mm, self.info.min_mm, self.info.max_mm)
            || !within(
                info.position_mm,
                self.endpoint.user_min_mm,
                self.endpoint.user_max_mm,
            )
        {
            return Err("remote stage readback is outside configured limits".into());
        }
        Ok(())
    }
}

impl Endpoint {
    fn request(&self, op: &str, args: Value, timeout: Duration) -> Result<Value, String> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let mut request = json!({
            "revision": REVISION,
            "id": id,
            "op": op,
            "args": args,
        });
        if let Some(token_env) = &self.token_env
            && let Ok(token) = env::var(token_env)
            && !token.is_empty()
        {
            request["token"] = Value::String(token);
        }
        let mut payload = serde_json::to_vec(&request)
            .map_err(|error| format!("could not encode remote {op} request: {error}"))?;
        payload.push(b'\n');

        // Resolve once and use the first address: failed requests are never retried.
        let address = (self.host.as_str(), self.port)
            .to_socket_addrs()
            .map_err(|error| format!("could not resolve remote CONEX endpoint: {error}"))?
            .next()
            .ok_or_else(|| "remote CONEX endpoint resolved to no addresses".to_string())?;
        let mut stream = TcpStream::connect_timeout(&address, timeout)
            .map_err(|error| format!("remote CONEX {op} connection failed: {error}"))?;
        stream
            .set_read_timeout(Some(timeout))
            .map_err(|error| format!("could not set remote CONEX read timeout: {error}"))?;
        stream
            .set_write_timeout(Some(timeout))
            .map_err(|error| format!("could not set remote CONEX write timeout: {error}"))?;
        stream
            .write_all(&payload)
            .and_then(|_| stream.flush())
            .map_err(|error| format!("remote CONEX {op} request failed: {error}"))?;

        let mut response = Vec::new();
        let mut reader = BufReader::new(stream.take((MAX_RESPONSE_LINE + 1) as u64));
        reader
            .read_until(b'\n', &mut response)
            .map_err(|error| format!("remote CONEX {op} response failed: {error}"))?;
        if response.len() > MAX_RESPONSE_LINE || response.last() != Some(&b'\n') {
            return Err(format!(
                "remote CONEX {op} response is missing a bounded newline"
            ));
        }
        response.pop();
        let reply: Value = serde_json::from_slice(&response)
            .map_err(|error| format!("remote CONEX {op} response is not valid JSON: {error}"))?;
        let response_id = reply
            .get("id")
            .and_then(Value::as_u64)
            .ok_or_else(|| format!("remote CONEX {op} response has no numeric id"))?;
        if response_id != id {
            return Err(format!(
                "remote CONEX {op} response id does not match request"
            ));
        }
        if !reply.get("ok").and_then(Value::as_bool).unwrap_or(false) {
            let error = reply
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("remote server rejected request");
            return Err(format!("remote CONEX {op} failed: {error}"));
        }
        reply
            .get("result")
            .cloned()
            .ok_or_else(|| format!("remote CONEX {op} response has no result"))
    }
}

fn expect_hello(value: &Value) -> Result<(), String> {
    if value.get("protocol").and_then(Value::as_str) != Some("kohdalab-conex")
        || value.get("revision").and_then(Value::as_u64) != Some(REVISION)
        || value.get("can_stop").and_then(Value::as_bool) != Some(true)
        || value.get("can_home").and_then(Value::as_bool) != Some(false)
    {
        return Err("remote CONEX hello response is incompatible with protocol revision 1".into());
    }
    Ok(())
}

fn parse_info(value: &Value, operation: &str, require_ready: bool) -> Result<RemoteInfo, String> {
    let position_mm = number(value, "position_mm", operation)?;
    let min_mm = number(value, "min_mm", operation)?;
    let max_mm = number(value, "max_mm", operation)?;
    if min_mm > max_mm {
        return Err(format!(
            "remote CONEX {operation} returned inverted physical limits"
        ));
    }
    if !within(position_mm, min_mm, max_mm) {
        return Err(format!(
            "remote CONEX {operation} readback is outside physical limits"
        ));
    }
    if value
        .get("positioner_error")
        .and_then(Value::as_str)
        .is_some_and(|e| !e.is_empty())
    {
        return Err(format!(
            "remote CONEX {operation} reports a positioner error"
        ));
    }
    if require_ready && value.get("ready").and_then(Value::as_bool) == Some(false) {
        return Err(format!("remote CONEX {operation} reports not ready"));
    }
    let state = value
        .get("state")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("remote CONEX {operation} response has no state"))?
        .to_owned();
    if require_ready && !READY_STATES.contains(&state.as_str()) {
        return Err(format!(
            "remote CONEX {operation} did not confirm a ready state (got {state})"
        ));
    }
    Ok(RemoteInfo {
        position_mm,
        min_mm,
        max_mm,
        state,
    })
}

fn number(value: &Value, key: &str, operation: &str) -> Result<f64, String> {
    let number = value
        .get(key)
        .and_then(Value::as_f64)
        .ok_or_else(|| format!("remote CONEX {operation} response has no numeric {key}"))?;
    finite(number, key)?;
    Ok(number)
}

fn validate_config(config: &StageConfig) -> Result<(), String> {
    config.validate_limits()?;
    if config.host.trim().is_empty() {
        return Err("remote CONEX host is empty".into());
    }
    if config.com_port.trim().is_empty() {
        return Err("remote CONEX COM port is empty".into());
    }
    finite(config.min_mm, "stage minimum")?;
    finite(config.max_mm, "stage maximum")?;
    finite(config.tolerance_mm, "stage tolerance")?;
    finite(config.timeout_s, "stage timeout")?;
    if config.min_mm > config.max_mm {
        return Err("stage minimum must not exceed stage maximum".into());
    }
    if config.tolerance_mm <= 0.0 || config.timeout_s <= 0.0 {
        return Err("stage tolerance and timeout must be positive".into());
    }
    Ok(())
}

fn seconds(value: f64, label: &str) -> Result<Duration, String> {
    finite(value, label)?;
    if value <= 0.0 {
        return Err(format!("{label} must be positive"));
    }
    Ok(Duration::from_secs_f64(value))
}

fn finite(value: f64, label: &str) -> Result<(), String> {
    if value.is_finite() {
        Ok(())
    } else {
        Err(format!("{label} must be finite"))
    }
}

fn within(value: f64, min: f64, max: f64) -> bool {
    value >= min && value <= max
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::BufRead,
        net::{TcpListener, TcpStream},
        sync::{Mutex, mpsc},
        thread,
    };

    fn endpoint(port: u16) -> Endpoint {
        Endpoint {
            host: "127.0.0.1".into(),
            port,
            token_env: None,
            tolerance_mm: 0.01,
            move_timeout_s: 1.0,
            user_min_mm: 0.0,
            user_max_mm: 25.0,
            next_id: Arc::new(AtomicU64::new(1)),
        }
    }

    fn reply(request: &Value, result: Value) -> String {
        format!(
            "{}\n",
            json!({"id": request["id"], "ok": true, "result": result})
        )
    }

    fn read_request(stream: &TcpStream) -> Value {
        let mut bytes = Vec::new();
        BufReader::new(stream)
            .read_until(b'\n', &mut bytes)
            .unwrap();
        serde_json::from_slice(bytes.strip_suffix(b"\n").unwrap()).unwrap()
    }

    #[test]
    fn error_readback_cannot_be_used_as_ready() {
        let mut v = json!({"position_mm":18.,"min_mm":0.,"max_mm":25.,"state":"32","positioner_error":"fault"});
        assert!(parse_info(&v, "status", false).is_err());
        v["positioner_error"] = json!("");
        v["ready"] = json!(false);
        assert!(parse_info(&v, "move", true).is_err());
    }
    #[test]
    fn connect_uses_newline_json_and_never_moves() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let (seen_tx, seen_rx) = mpsc::channel();
        let server = thread::spawn(move || {
            for _ in 0..2 {
                let stream = listener.accept().unwrap().0;
                let request = read_request(&stream);
                seen_tx.send(request.clone()).unwrap();
                let result = if request["op"] == "hello" {
                    json!({"protocol":"kohdalab-conex","revision":1,"can_stop":true,"can_home":false})
                } else {
                    json!({"position_mm":18.0,"min_mm":0.0,"max_mm":25.0,"state":"32"})
                };
                (&stream)
                    .write_all(reply(&request, result).as_bytes())
                    .unwrap();
            }
        });
        let stage = RemoteStage::establish(endpoint(port), "COM4").unwrap();
        assert_eq!(stage.info().position_mm, 18.0);
        let seen = [seen_rx.recv().unwrap(), seen_rx.recv().unwrap()];
        assert_eq!(seen[0]["op"], "hello");
        assert_eq!(seen[1]["op"], "connect");
        assert!(seen.iter().all(|request| request["revision"] == 1));
        server.join().unwrap();
    }

    #[test]
    fn malformed_and_wrong_id_replies_are_rejected() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = thread::spawn(move || {
            let stream = listener.accept().unwrap().0;
            let _ = read_request(&stream);
            (&stream).write_all(b"not-json\n").unwrap();
        });
        assert!(
            endpoint(port)
                .request("status", json!({}), Duration::from_secs(1))
                .is_err()
        );
        server.join().unwrap();

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = thread::spawn(move || {
            let stream = listener.accept().unwrap().0;
            let request = read_request(&stream);
            let bad = json!({"id": request["id"].as_u64().unwrap() + 1, "ok": true, "result": {}});
            (&stream).write_all(format!("{bad}\n").as_bytes()).unwrap();
        });
        assert!(
            endpoint(port)
                .request("status", json!({}), Duration::from_secs(1))
                .is_err()
        );
        server.join().unwrap();
    }

    #[test]
    fn stop_and_status_are_not_blocked_by_move_waiter() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let stage = RemoteStage {
            endpoint: endpoint(port),
            info: RemoteInfo {
                position_mm: 18.0,
                min_mm: 0.0,
                max_mm: 25.0,
                state: "32".into(),
            },
        };
        let (move_started_tx, move_started_rx) = mpsc::channel();
        let (release_move_tx, release_move_rx) = mpsc::channel();
        let release_move_rx = Arc::new(Mutex::new(Some(release_move_rx)));
        let server = thread::spawn(move || {
            let mut handlers = Vec::new();
            for _ in 0..3 {
                let stream = listener.accept().unwrap().0;
                let started = move_started_tx.clone();
                let release = release_move_rx.clone();
                handlers.push(thread::spawn(move || {
                    let request = read_request(&stream);
                    match request["op"].as_str().unwrap() {
                        "move" => {
                            started.send(()).unwrap();
                            release.lock().unwrap().take().unwrap().recv().unwrap();
                        }
                        "status" | "stop" => {}
                        other => panic!("unexpected operation {other}"),
                    }
                    (&stream)
                        .write_all(
                            reply(
                                &request,
                                json!({"position_mm":18.0,"min_mm":0.0,"max_mm":25.0,"state":"32"}),
                            )
                            .as_bytes(),
                        )
                        .unwrap();
                }));
            }
            for handler in handlers {
                handler.join().unwrap();
            }
        });
        let mover = stage.clone();
        let move_waiter = thread::spawn(move || mover.move_to(18.0));
        move_started_rx
            .recv_timeout(Duration::from_secs(1))
            .unwrap();
        assert_eq!(stage.position().unwrap(), 18.0);
        assert_eq!(stage.stop().unwrap(), 18.0);
        release_move_tx.send(()).unwrap();
        assert_eq!(move_waiter.join().unwrap().unwrap(), 18.0);
        server.join().unwrap();
    }
}
