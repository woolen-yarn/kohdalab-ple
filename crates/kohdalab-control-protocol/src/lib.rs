//! KohdaLab Control Protocol 1 boundary helpers. No sockets or device operations.
//! The identical vendored crate is tested with each product; policy stays in its engine.
use serde_json::{Value, json};
use std::time::{SystemTime, UNIX_EPOCH};

pub const VERSION: u64 = 1;
pub const PROTOCOL: &str = "kohdalab-control";
pub const DISCOVERY_DIRECTORY: &str = ".kohdalab/control";

/// Add the canonical descriptor without removing a product's compatibility fields.
pub fn descriptor(mut legacy: Value, application: &str, api_version: u64) -> Value {
    if let Some(address) = legacy.get("address").and_then(Value::as_str)
        && let Ok(address) = address.parse::<std::net::SocketAddr>()
    {
        legacy["host"] = json!(address.ip().to_string());
        legacy["port"] = json!(address.port());
    }
    legacy["application"] = json!(application);
    legacy["app"] = json!(application);
    legacy["control_protocol"] = json!(VERSION);
    legacy["discovery_version"] = json!(1);
    legacy["api_version"] = json!(api_version);
    legacy["transport"] = json!("tcp-jsonl");
    legacy["started_unix_s"] = json!(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_secs())
    );
    legacy
}

pub fn is_common(body: &Value) -> bool {
    body.get("control_protocol").is_some()
        || body
            .get("method")
            .or_else(|| body.get("action"))
            .and_then(Value::as_str)
            .is_some_and(|s| s.starts_with("control."))
}

/// Normalize only the new namespace. Native requests and safety defaults are untouched.
/// `_control_method` is reserved, locally derived metadata, not caller authority.
pub fn prepare(body: &Value) -> Result<Value, Value> {
    if !is_common(body) {
        let mut native = body.clone();
        if let Some(object) = native.as_object_mut() {
            object.remove("_control_method");
        }
        return Ok(native);
    }
    let fail =
        |code, message| response(body, json!({"ok":false,"error_code":code,"error":message}));
    if body
        .get("control_protocol")
        .is_some_and(|v| v.as_u64() != Some(VERSION))
    {
        return Err(fail(
            "unsupported_protocol",
            "Only KohdaLab Control Protocol 1 is supported",
        ));
    }
    if let (Some(method), Some(action)) = (body.get("method"), body.get("action"))
        && method != action
    {
        return Err(fail(
            "invalid_request",
            "method and action must not disagree",
        ));
    }
    let Some(method) = body
        .get("method")
        .or_else(|| body.get("action"))
        .and_then(Value::as_str)
        .and_then(|s| s.strip_prefix("control."))
    else {
        return Err(fail(
            "invalid_request",
            "Common requests require a control.* method",
        ));
    };
    if method.is_empty()
        || method.len() > 64
        || !method.bytes().all(|c| c.is_ascii_lowercase() || c == b'_')
    {
        return Err(fail("invalid_request", "Invalid control method"));
    }
    if let Some(id) = body.get("request_id")
        && !id.is_null()
        && (!id.is_string() || id.as_str().is_some_and(|s| s.len() > 128))
    {
        return Err(fail(
            "invalid_request",
            "request_id must be a bounded string",
        ));
    }
    if let (Some(a), Some(b)) = (body.get("client_id"), body.get("client"))
        && a != b
    {
        return Err(fail(
            "invalid_request",
            "client_id and client must not disagree",
        ));
    }
    let mut native = body.clone();
    let mapped = match method {
        "describe" => "control_describe",
        "status" => "control_status",
        "result" => "control_result",
        other => other,
    };
    native["method"] = json!(mapped);
    native["action"] = json!(mapped);
    native["_control_method"] = json!(method);
    if let Some(client) = body.get("client_id").or_else(|| body.get("client")) {
        native["client_id"] = client.clone();
        native["client"] = client.clone();
    }
    if let Some(id) = body.get("job_id") {
        for key in ["run_id", "id"] {
            if let Some(other) = body.get(key)
                && other != id
            {
                return Err(fail("invalid_request", "job_id aliases must not disagree"));
            }
            native[key] = id.clone();
        }
    }
    Ok(native)
}

pub fn category(code: &str) -> &'static str {
    match code {
        "authentication" | "unauthorized" => "authentication",
        "ownership" | "ownership_lost" | "invalid_lease" | "revoked" => "ownership",
        "busy" | "capacity" => "busy",
        "timeout" => "timeout",
        "transport" | "disconnected" | "connection" => "transport",
        "device" => "device",
        "protection" | "safety" | "limit" => "safety",
        "storage" | "io" => "storage",
        "cleanup_unconfirmed" => "cleanup_unconfirmed",
        "unknown_job" => "unknown_job",
        "request_id_conflict" => "request_id_conflict",
        "unsupported" | "unsupported_protocol" => "unsupported",
        "invalid_request" | "validation" | "range" => "validation",
        "stopped" | "cancelled" => "cancelled",
        _ => "application",
    }
}

pub fn fault(code: &str, message: &str) -> Value {
    // A timeout never implies that a mutation did not execute. No automatic replay.
    json!({"code":category(code),"application_code":code,"message":message,"retryable":false})
}

/// Canonical response for control.*; keep legacy envelopes unchanged otherwise.
pub fn response(original: &Value, native: Value) -> Value {
    if !is_common(original) {
        return native;
    }
    if native.get("control_protocol").and_then(Value::as_u64) == Some(VERSION)
        && native.get("ok").is_some_and(Value::is_boolean)
    {
        return native;
    }
    let request_id = original.get("request_id").cloned().unwrap_or(Value::Null);
    if native.get("ok").and_then(Value::as_bool) == Some(true) {
        json!({"control_protocol":VERSION,"ok":true,"request_id":request_id,"result":native.get("result").or_else(||native.get("data")).cloned().unwrap_or(Value::Null),"error":null})
    } else {
        let code = native
            .get("error_code")
            .and_then(Value::as_str)
            .unwrap_or("application");
        let message = native
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or("The application rejected the request");
        json!({"control_protocol":VERSION,"ok":false,"request_id":request_id,"result":null,"error":fault(code,message)})
    }
}

/// App declares only implemented operations and its actual, unchanged safety policy.
pub fn describe(
    application: &str,
    version: &str,
    api_version: u64,
    methods: &[&str],
    safety: Value,
    native: Value,
) -> Value {
    json!({"protocol":PROTOCOL,"control_protocol":VERSION,"application":application,"application_version":version,"api_version":api_version,"methods":methods,"safety":safety,"native":native})
}

pub fn status(phase: &str, busy: bool, ownership: Value, cleanup: &str, native: Value) -> Value {
    json!({"phase":phase,"busy":busy,"ownership":ownership,"cleanup":cleanup,"native":native})
}

/// Standard receipt: retain the original payload and distinguish an ACK from its job.
pub fn receipt(
    id: &str,
    kind: &str,
    state: &str,
    cleanup: &str,
    linked_job_id: Option<&str>,
    native: Value,
) -> Value {
    let terminal = matches!(state, "completed" | "stopped" | "error");
    json!({"job_id":id,"kind":kind,"status":state,"terminal":terminal,"cleanup":cleanup,"linked_job_id":linked_job_id,"native":native})
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn namespace_normalizes_identity_and_id_but_not_policy() {
        let body = json!({"method":"control.result","control_protocol":1,"client_id":"abc","job_id":"job","stop_on_disconnect":false});
        let p = prepare(&body).unwrap();
        assert_eq!(p["method"], "control_result");
        assert_eq!(p["action"], "control_result");
        assert_eq!(p["client"], "abc");
        assert_eq!(p["run_id"], "job");
        assert_eq!(p["stop_on_disconnect"], false);
    }
    #[test]
    fn conflicts_and_future_versions_cannot_dispatch() {
        for body in [
            json!({"method":"control.status","action":"control.start"}),
            json!({"method":"control.status","control_protocol":2}),
            json!({"method":"control.status","client":"x","client_id":"y"}),
            json!({"method":"control.result","job_id":"a","id":"b"}),
        ] {
            let e = prepare(&body).unwrap_err();
            assert_eq!(e["ok"], false);
            assert_eq!(e["control_protocol"], 1);
        }
    }
    #[test]
    fn reserved_marker_cannot_be_forged_in_native_requests() {
        let p = prepare(&json!({"method":"start","_control_method":"status"})).unwrap();
        assert!(p.get("_control_method").is_none());
    }
    #[test]
    fn native_envelope_and_standard_stop_semantics_are_not_rewritten() {
        let b = json!({"method":"stop"});
        let v = json!({"ok":true,"data":{"stopping":true}});
        assert_eq!(response(&b, v.clone()), v);
        assert_eq!(
            receipt("a", "stop", "running", "pending", None, json!({}))["terminal"],
            false
        );
    }
    #[test]
    fn common_errors_preserve_native_code_and_never_authorize_replay() {
        let e = response(
            &json!({"method":"control.start","request_id":"id"}),
            json!({"ok":false,"error_code":"timeout","error":"Outcome unknown"}),
        );
        assert_eq!(e["error"]["code"], "timeout");
        assert_eq!(e["error"]["retryable"], false);
        assert_eq!(e["request_id"], "id");
    }
    #[test]
    fn common_success_does_not_duplicate_large_result_under_data() {
        let r = response(
            &json!({"method":"control.status"}),
            json!({"ok":true,"result":{"samples":[1,2,3]},"data":{"samples":[1,2,3]}}),
        );
        assert!(r.get("data").is_none());
        assert_eq!(r["result"]["samples"].as_array().unwrap().len(), 3);
    }
    #[test]
    fn descriptors_keep_native_fields_and_add_canonical_identity() {
        let d = descriptor(
            json!({"protocol":2,"address":"127.0.0.1:1234","token":"secret","instance_id":"id","pid":5}),
            "kohdalab-iv",
            2,
        );
        assert_eq!(d["control_protocol"], 1);
        assert_eq!(d["protocol"], 2);
        assert_eq!(d["application"], "kohdalab-iv");
        assert_eq!(d["host"], "127.0.0.1");
        assert_eq!(d["port"], 1234);
    }
}
