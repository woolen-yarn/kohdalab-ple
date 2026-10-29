//! Transport-independent ownership and job receipts. Hardware stays in engine.
use serde_json::{Value, json};
use std::{
    collections::{HashSet, VecDeque},
    time::{Duration, Instant},
};
pub const LEASE_TIMEOUT: Duration = Duration::from_secs(10);
pub const HISTORY_LIMIT: usize = 32;
pub type Result<T> = std::result::Result<T, Error>;
#[derive(Debug)]
pub struct Error {
    pub code: &'static str,
    pub message: String,
}
impl Error {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
    pub fn response(self) -> Value {
        json!({"ok":false,"error":self.message,"error_code":self.code})
    }
}
pub fn valid_id(id: &str) -> bool {
    id.len() == 36
        && id.bytes().enumerate().all(|(i, b)| {
            if [8, 13, 18, 23].contains(&i) {
                b == b'-'
            } else {
                b.is_ascii_hexdigit()
            }
        })
}
pub fn success(value: Value) -> Value {
    json!({"ok":true,"result":value})
}
pub struct Lease {
    pub id: String,
    pub client: String,
    pub touched: Instant,
}
pub struct Receipt {
    pub id: String,
    pub fingerprint: Value,
    pub value: Value,
}
#[derive(Default)]
pub struct Session {
    pub lease: Option<Lease>,
    revoked: HashSet<String>,
    seen: HashSet<String>,
    receipts: VecDeque<Receipt>,
    command_results: VecDeque<Receipt>,
}
impl Session {
    pub fn claim(&mut self, client: &str, busy: bool) -> Result<Value> {
        if !valid_id(client) {
            return Err(Error::new("validation", "client_id must be a UUID"));
        }
        if self.revoked.contains(client) {
            return Err(Error::new(
                "ownership_lost",
                "This client was revoked; create a new client",
            ));
        }
        if let Some(lease) = &mut self.lease {
            if lease.client != client {
                return Err(Error::new("ownership", "Another client owns control"));
            }
            lease.touched = Instant::now();
            return Ok(json!({"lease":lease.id,"timeout_s":10}));
        }
        if busy {
            return Err(Error::new(
                "busy",
                "Wait for the current operation or dialog before acquiring control",
            ));
        }
        if self.revoked.len() >= 100_000 {
            return Err(Error::new(
                "capacity",
                "Ownership history limit reached; restart the application",
            ));
        }
        let id = crate::remote_gui::new_id().map_err(|e| Error::new("internal", e))?;
        self.lease = Some(Lease {
            id: id.clone(),
            client: client.into(),
            touched: Instant::now(),
        });
        Ok(json!({"lease":id,"timeout_s":10}))
    }
    pub fn claim_common(&mut self, client: &str, lease: Option<&str>, busy: bool) -> Result<Value> {
        if let Some(current) = &self.lease {
            if current.client != client {
                return Err(Error::new("ownership", "Another client owns control"));
            }
            if lease != Some(current.id.as_str()) {
                return Err(Error::new(
                    "invalid_lease",
                    "The opaque lease is required for common reacquisition",
                ));
            }
        } else if lease.is_some() {
            return Err(Error::new("invalid_lease", "No active common lease exists"));
        }
        self.claim(client, busy)
    }
    pub fn require(&mut self, body: &Value) -> Result<()> {
        let Some(lease) = &mut self.lease else {
            return Err(Error::new(
                "ownership_lost",
                "Acquire control before modifying the application",
            ));
        };
        if body["lease"].as_str() != Some(lease.id.as_str()) {
            return Err(Error::new(
                "ownership_lost",
                "The lease is invalid or was revoked",
            ));
        }
        lease.touched = Instant::now();
        Ok(())
    }
    pub fn require_common(&mut self, body: &Value) -> Result<()> {
        let client = body
            .get("client_id")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::new("ownership_lost", "Common mutations require client_id"))?;
        if self
            .lease
            .as_ref()
            .is_none_or(|lease| lease.client != client)
        {
            return Err(Error::new(
                "ownership_lost",
                "The client does not own this lease",
            ));
        }
        self.require(body)
    }
    pub fn expired(&self) -> bool {
        self.lease
            .as_ref()
            .is_some_and(|l| l.touched.elapsed() >= LEASE_TIMEOUT)
    }
    pub fn revoke(&mut self) {
        if let Some(l) = self.lease.take() {
            self.revoked.insert(l.client);
        }
    }
    pub fn previous(&self, id: &str, fingerprint: &Value) -> Result<Option<Value>> {
        if !valid_id(id) {
            return Err(Error::new("validation", "request_id must be a UUID"));
        }
        let id = id.to_ascii_lowercase();
        if let Some(r) = self
            .receipts
            .iter()
            .chain(self.command_results.iter())
            .find(|r| r.id == id)
        {
            if &r.fingerprint != fingerprint {
                return Err(Error::new(
                    "request_id_conflict",
                    "request_id was used for another operation or payload",
                ));
            }
            return Ok(Some(r.value.clone()));
        }
        if self.seen.contains(&id) {
            return Err(Error::new(
                "request_id_expired",
                "This request_id is retired; inspect saved evidence and use a new UUID",
            ));
        }
        Ok(None)
    }
    pub fn remember_command(
        &mut self,
        id: String,
        kind: &str,
        fingerprint: Value,
        result: Value,
        run_job: Option<&str>,
    ) -> Result<Value> {
        let id = id.to_ascii_lowercase();
        if self.seen.len() >= 100_000 {
            return Err(Error::new(
                "capacity",
                "Command ID limit reached; restart the application",
            ));
        }
        let mut ack = result;
        if !ack.is_object() {
            ack = json!({"value":ack});
        }
        ack["request_id"] = json!(id);
        ack["kind"] = json!(kind);
        ack["run_job"] = json!(run_job);
        self.seen.insert(id.clone());
        self.command_results.push_back(Receipt {
            id,
            fingerprint,
            value: ack.clone(),
        });
        while self.command_results.len() > HISTORY_LIMIT {
            self.command_results.pop_front();
        }
        Ok(ack)
    }
    pub fn can_remember(&self) -> Result<()> {
        if self.seen.len() >= 100_000 {
            Err(Error::new(
                "capacity",
                "Request ID capacity exhausted; restart the application",
            ))
        } else {
            Ok(())
        }
    }
    pub fn reserve(&mut self, id: String, kind: &str, fingerprint: Value) -> Result<Value> {
        let id = id.to_ascii_lowercase();
        if self.seen.len() >= 100_000 {
            return Err(Error::new(
                "capacity",
                "Job ID limit reached; restart the application",
            ));
        }
        let value = json!({"id":id,"kind":kind,"status":"running","rows":0,"output":null,"error":null,"error_code":null,"snapshot":null});
        self.seen.insert(id.clone());
        self.receipts.push_back(Receipt {
            id,
            fingerprint,
            value: value.clone(),
        });
        while self.receipts.len() > HISTORY_LIMIT {
            self.receipts.pop_front();
        }
        Ok(value)
    }
    pub fn receipt(&self, id: &str) -> Result<Value> {
        let id = id.to_ascii_lowercase();
        self.receipts
            .iter()
            .chain(self.command_results.iter())
            .find(|r| r.id == id)
            .map(|r| r.value.clone())
            .ok_or_else(|| Error::new("unknown_job", "No retained receipt for that ID"))
    }
    pub fn update(
        &mut self,
        id: &str,
        status: &str,
        snapshot: Value,
        error: Option<String>,
        code: Option<&str>,
    ) {
        if let Some(r) = self.receipts.iter_mut().find(|r| r.id == id) {
            r.value["status"] = json!(status);
            let measured = r.value["kind"] == "start";
            r.value["rows"] = if measured {
                snapshot["rows"].clone()
            } else {
                json!(0)
            };
            r.value["output"] = if measured {
                snapshot["run_dir"].clone()
            } else {
                Value::Null
            };
            r.value["snapshot"] = snapshot;
            r.value["error"] = json!(error);
            r.value["error_code"] = json!(code);
        }
    }
}
pub fn fingerprint(method: &str, body: &Value) -> Value {
    let mut args = body.clone();
    if let Some(m) = args.as_object_mut() {
        for key in [
            "token",
            "lease",
            "client_id",
            "action",
            "method",
            "request_id",
        ] {
            m.remove(key);
        }
    }
    json!({"method":method,"args":args})
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn leases_are_exclusive_and_revocation_is_permanent() {
        let mut s = Session::default();
        let a = crate::remote_gui::new_id().unwrap();
        let b = crate::remote_gui::new_id().unwrap();
        let lease = s.claim(&a, false).unwrap();
        assert!(s.claim(&b, false).is_err());
        assert!(s.require(&json!({"lease":lease["lease"]})).is_ok());
        s.revoke();
        assert!(s.claim(&a, false).is_err());
        assert!(s.claim(&b, false).is_ok());
    }
    #[test]
    fn job_receipts_are_idempotent_and_retired_ids_cannot_execute_again() {
        let mut s = Session::default();
        let id = crate::remote_gui::new_id().unwrap();
        let f = fingerprint("start", &json!({"lease":"a","request_id":id}));
        let r = s.reserve(id.clone(), "start", f.clone()).unwrap();
        assert_eq!(s.previous(&id.to_uppercase(), &f).unwrap(), Some(r));
        assert!(s.previous(&id, &json!({})).is_err());
        for _ in 0..HISTORY_LIMIT {
            s.reserve(crate::remote_gui::new_id().unwrap(), "start", json!({}))
                .unwrap();
        }
        assert_eq!(s.previous(&id, &f).unwrap_err().code, "request_id_expired");
    }
}
