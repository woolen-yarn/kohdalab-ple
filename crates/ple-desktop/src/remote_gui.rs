//! Opt-in, loopback-only GUI control transport.
//!
//! The listener never touches instruments.  It forwards authenticated requests
//! to the desktop event loop through a bounded channel, where the existing
//! controller remains the only owner of device connections.
use serde_json::{Value, json};
use std::{
    fs,
    io::{Read, Write},
    net::{Ipv4Addr, TcpListener, TcpStream},
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

pub const API_VERSION: u64 = 1;
const MAX_REQUEST: usize = 1024 * 1024;
const MAX_CONNECTIONS: usize = 8;
const TIMEOUT: Duration = Duration::from_secs(2);

pub struct Request {
    pub body: Value,
    pub deadline: Instant,
    pub reply: mpsc::SyncSender<Value>,
}

pub struct Server {
    pub requests: mpsc::Receiver<Request>,
    pub descriptor: PathBuf,
    stopped: Arc<AtomicBool>,
}

impl Server {
    /// Start on an ephemeral loopback port and publish one private descriptor.
    /// The caller explicitly chooses the directory so tests and launched GUI
    /// instances cannot accidentally attach by process recency.
    pub fn start(directory: PathBuf) -> Result<Self, String> {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .map_err(|e| format!("GUI API bind: {e}"))?;
        listener.set_nonblocking(true).map_err(|e| e.to_string())?;
        let port = listener.local_addr().map_err(|e| e.to_string())?.port();
        let token = new_id()?;
        let instance_id = new_id()?;
        #[cfg(unix)]
        let created_directory = !directory.exists();
        fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if created_directory {
                fs::set_permissions(&directory, fs::Permissions::from_mode(0o700))
                    .map_err(|e| e.to_string())?;
            }
        }
        let descriptor =
            directory.join(format!("ple-gui-{}-{instance_id}.json", std::process::id()));
        // tempfile creates a private file; write/sync before atomically
        // publishing a complete discovery record.
        let mut temporary =
            tempfile::NamedTempFile::new_in(&directory).map_err(|e| e.to_string())?;
        serde_json::to_writer(
            &mut temporary,
            &json!({
                "revision": 1, "api_version": API_VERSION,
                "app": "kohdalab-ple", "host": "127.0.0.1", "port": port,
                "token": token, "instance_id": instance_id, "pid": std::process::id(),
            }),
        )
        .map_err(|e| e.to_string())?;
        temporary
            .as_file_mut()
            .sync_all()
            .map_err(|e| e.to_string())?;
        temporary
            .persist_noclobber(&descriptor)
            .map_err(|e| e.to_string())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&descriptor, fs::Permissions::from_mode(0o600))
                .map_err(|e| e.to_string())?;
        }
        let (send, requests) = mpsc::sync_channel(8);
        let stopped = Arc::new(AtomicBool::new(false));
        let stop = stopped.clone();
        let active = Arc::new(AtomicUsize::new(0));
        thread::Builder::new()
            .name("ple-gui-api-listener".into())
            .spawn(move || {
                while !stop.load(Ordering::Acquire) {
                    match listener.accept() {
                        Ok((stream, peer)) if peer.ip().is_loopback() => {
                            if let Some(permit) = ConnectionPermit::acquire(&active) {
                                let token = token.clone();
                                let send = send.clone();
                                let _ = thread::Builder::new()
                                    .name("ple-gui-api-peer".into())
                                    .spawn(move || {
                                        let _permit = permit;
                                        let _ = serve(stream, &token, &send);
                                    });
                            }
                        }
                        Ok(_) => {}
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            thread::sleep(Duration::from_millis(10))
                        }
                        Err(_) => break,
                    }
                }
            })
            .map_err(|e| e.to_string())?;
        Ok(Self {
            requests,
            descriptor,
            stopped,
        })
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::Release);
        let _ = fs::remove_file(&self.descriptor);
    }
}

/// A random RFC 4122 version-4 identifier without a UUID dependency.
pub fn new_id() -> Result<String, String> {
    let mut bytes = [0_u8; 16];
    getrandom::fill(&mut bytes).map_err(|e| format!("random API id: {e}"))?;
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    Ok(format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        bytes[0],
        bytes[1],
        bytes[2],
        bytes[3],
        bytes[4],
        bytes[5],
        bytes[6],
        bytes[7],
        bytes[8],
        bytes[9],
        bytes[10],
        bytes[11],
        bytes[12],
        bytes[13],
        bytes[14],
        bytes[15]
    ))
}

struct ConnectionPermit(Arc<AtomicUsize>);
impl ConnectionPermit {
    fn acquire(active: &Arc<AtomicUsize>) -> Option<Self> {
        let mut count = active.load(Ordering::Acquire);
        while count < MAX_CONNECTIONS {
            match active.compare_exchange_weak(
                count,
                count + 1,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => return Some(Self(active.clone())),
                Err(current) => count = current,
            }
        }
        None
    }
}
impl Drop for ConnectionPermit {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

fn serve(
    mut stream: TcpStream,
    token: &str,
    send: &mpsc::SyncSender<Request>,
) -> Result<(), String> {
    stream.set_nonblocking(false).map_err(|e| e.to_string())?;
    stream
        .set_read_timeout(Some(TIMEOUT))
        .map_err(|e| e.to_string())?;
    stream
        .set_write_timeout(Some(TIMEOUT))
        .map_err(|e| e.to_string())?;
    let response = read_line(&mut stream)
        .and_then(|line| {
            if line.len() > MAX_REQUEST || !line.ends_with('\n') {
                return Err("Request must be one JSON line smaller than 1 MiB".into());
            }
            serde_json::from_str::<Value>(&line).map_err(|_| "Invalid JSON request".to_owned())
        })
        .and_then(|body| {
            if body.get("token").and_then(Value::as_str) != Some(token) {
                return Ok(json!({"ok":false,"error":"GUI API authentication failed","error_code":"authentication"}));
            }
            let (reply, receive) = mpsc::sync_channel(1);
            send.try_send(Request {
                body,
                deadline: Instant::now() + TIMEOUT,
                reply,
            })
            .map_err(|_| "GUI request queue unavailable".to_owned())?;
            receive
                .recv_timeout(TIMEOUT)
                .map_err(|_| "GUI request timed out; check status before retrying".to_owned())
        })
        .unwrap_or_else(|error| json!({"ok":false,"error":error,"error_code":"transport"}));
    serde_json::to_writer(&mut stream, &response).map_err(|e| e.to_string())?;
    stream.write_all(b"\n").map_err(|e| e.to_string())
}

fn read_line(stream: &mut TcpStream) -> Result<String, String> {
    let deadline = Instant::now() + TIMEOUT;
    let mut bytes = Vec::new();
    loop {
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .ok_or("Request read timed out")?;
        stream
            .set_read_timeout(Some(remaining))
            .map_err(|e| e.to_string())?;
        let mut chunk = [0_u8; 4096];
        let count = stream.read(&mut chunk).map_err(|e| e.to_string())?;
        if count == 0 {
            return Err("Incomplete JSON request".into());
        }
        let end = chunk[..count]
            .iter()
            .position(|byte| *byte == b'\n')
            .map(|index| index + 1);
        bytes.extend_from_slice(&chunk[..end.unwrap_or(count)]);
        if bytes.len() > MAX_REQUEST {
            return Err("Request exceeds 1 MiB".into());
        }
        if end.is_some() {
            return String::from_utf8(bytes).map_err(|_| "Request must be UTF-8".into());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader};
    fn request(descriptor: &Value, token: &str) -> Value {
        let mut stream =
            TcpStream::connect(("127.0.0.1", descriptor["port"].as_u64().unwrap() as u16)).unwrap();
        writeln!(stream, "{}", json!({"token":token,"action":"status"})).unwrap();
        let mut response = String::new();
        BufReader::new(stream).read_line(&mut response).unwrap();
        serde_json::from_str(&response).unwrap()
    }
    #[test]
    fn ids_are_version_four() {
        let id = new_id().unwrap();
        assert_eq!(id.len(), 36);
        assert_eq!(&id[14..15], "4");
        assert!(matches!(&id[19..20], "8" | "9" | "a" | "b"));
    }
    #[test]
    fn auth_precedes_dispatch_and_descriptor_is_removed() {
        let directory = tempfile::tempdir().unwrap();
        let server = Server::start(directory.path().into()).unwrap();
        let descriptor: Value =
            serde_json::from_slice(&fs::read(&server.descriptor).unwrap()).unwrap();
        assert_eq!(descriptor["app"], "kohdalab-ple");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&server.descriptor)
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
        assert_eq!(request(&descriptor, "invalid")["ok"], false);
        assert_eq!(
            request(&descriptor, "invalid")["error_code"],
            "authentication"
        );
        assert!(server.requests.try_recv().is_err());
        let copy = descriptor.clone();
        let client = thread::spawn(move || request(&copy, copy["token"].as_str().unwrap()));
        let job = server.requests.recv_timeout(TIMEOUT).unwrap();
        assert_eq!(job.body["action"], "status");
        job.reply.send(json!({"ok":true,"result":{}})).unwrap();
        assert_eq!(client.join().unwrap()["ok"], true);
        let path = server.descriptor.clone();
        drop(server);
        assert!(!path.exists());
    }
    #[test]
    fn connection_permits_are_bounded_and_reusable() {
        let active = Arc::new(AtomicUsize::new(0));
        let permits: Vec<_> = (0..MAX_CONNECTIONS)
            .map(|_| ConnectionPermit::acquire(&active).unwrap())
            .collect();
        assert!(ConnectionPermit::acquire(&active).is_none());
        drop(permits);
        assert!(ConnectionPermit::acquire(&active).is_some());
    }
    #[test]
    fn oversize_or_idle_peers_cannot_dispatch_or_block_status() {
        let directory = tempfile::tempdir().unwrap();
        let server = Server::start(directory.path().into()).unwrap();
        let descriptor: Value =
            serde_json::from_slice(&fs::read(&server.descriptor).unwrap()).unwrap();
        let mut oversized =
            TcpStream::connect(("127.0.0.1", descriptor["port"].as_u64().unwrap() as u16)).unwrap();
        oversized.write_all(&vec![b'x'; MAX_REQUEST + 1]).unwrap();
        oversized.write_all(b"\n").unwrap();
        let mut response = String::new();
        BufReader::new(oversized).read_line(&mut response).unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(&response).unwrap()["error_code"],
            "transport"
        );
        assert!(server.requests.try_recv().is_err());
        let _idle =
            TcpStream::connect(("127.0.0.1", descriptor["port"].as_u64().unwrap() as u16)).unwrap();
        let copy = descriptor.clone();
        let client = thread::spawn(move || request(&copy, copy["token"].as_str().unwrap()));
        let job = server
            .requests
            .recv_timeout(Duration::from_secs(1))
            .unwrap();
        job.reply
            .send(json!({"ok":true,"result":{"alive":true}}))
            .unwrap();
        assert_eq!(client.join().unwrap()["result"]["alive"], true);
    }
}
