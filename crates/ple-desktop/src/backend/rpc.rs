use super::StopBarrier;
use crate::engine::{CleanupCause, Event};
use ple_core::Config;
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    io::{BufRead, BufReader, Read, Write},
    path::{Path, PathBuf},
    process::{Child, ChildStdin, Command, Stdio},
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicU64, Ordering},
        mpsc,
    },
    thread,
    time::Duration,
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
            .env("PYTHONIOENCODING", "utf-8")
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
            log_bridge_stderr(BufReader::new(stderr), &events);
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
                cause: CleanupCause::SpectrometerStop,
                result: result.map(|_| ()),
            });
            drop(state);
            ready.notify_all();
        });
    }
}
const BRIDGE_STDERR_LIMIT: usize = 8192;

fn log_bridge_stderr(mut reader: impl BufRead, events: &mpsc::Sender<Event>) {
    let mut line = Vec::with_capacity(BRIDGE_STDERR_LIMIT + 1);
    loop {
        line.clear();
        match reader
            .by_ref()
            .take((BRIDGE_STDERR_LIMIT + 1) as u64)
            .read_until(b'\n', &mut line)
        {
            Ok(0) | Err(_) => break,
            Ok(_) => {}
        }
        let complete = line.last() == Some(&b'\n');
        let truncated = line.len() > BRIDGE_STDERR_LIMIT && !complete;
        if complete {
            line.pop();
            if line.last() == Some(&b'\r') {
                line.pop();
            }
        }
        line.truncate(BRIDGE_STDERR_LIMIT);
        let suffix = if truncated { " [truncated]" } else { "" };
        let _ = events.send(Event::Log(format!(
            "Bridge: {}{suffix}",
            String::from_utf8_lossy(&line)
        )));
        if truncated && reader.skip_until(b'\n').is_err() {
            break;
        }
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
    fn bridge_stderr_replaces_invalid_bytes_and_recovers_after_long_lines() {
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("stderr_bridge.py");
        std::fs::write(&script, format!(
            "import os,sys\nsys.stderr.buffer.write(b'cp932: \\x82\\xa0\\r\\n' + b'x' * {} + b'\\n' + os.environ['PYTHONIOENCODING'].encode() + b'\\nlast')\n",
            BRIDGE_STDERR_LIMIT * 2
        )).unwrap();
        let mut config = Config::default();
        config.spectrometer.bridge_script = script.to_string_lossy().into();
        let (tx, rx) = mpsc::channel();
        let _rpc = RpcClient::start(&config, tx).unwrap();
        let next_log = || match rx.recv_timeout(Duration::from_secs(3)).unwrap() {
            Event::Log(line) => line,
            other => panic!("unexpected bridge event: {other:?}"),
        };
        assert_eq!(next_log(), "Bridge: cp932: ��");
        assert_eq!(
            next_log(),
            format!("Bridge: {} [truncated]", "x".repeat(BRIDGE_STDERR_LIMIT))
        );
        assert_eq!(next_log(), "Bridge: utf-8");
        assert_eq!(next_log(), "Bridge: last");
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
}
