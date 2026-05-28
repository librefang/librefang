//! WIT-typed Component Model host bindings for librefang plugins.
//!
//! This module is the thin shim that wires the bindgen-generated
//! `Host` traits to the existing `host_functions::dispatch` JSON-RPC
//! surface. Phase-5 D1 hygiene: a NEW file living alongside (not
//! modifying) `host_functions.rs` and `sandbox.rs`. The dispatch
//! function remains the single source of truth for every capability
//! check and side effect — this layer only marshals typed Component
//! Model arguments into the JSON shape `dispatch` expects, and the
//! `{"ok":...} / {"error":"..."}` JSON result back into typed Result
//! returns.
//!
//! NOTE on bindgen invocation: the actual `wasmtime::component::bindgen!`
//! macro call and the generated `Host` trait impls land in C-004
//! (`sandbox_component.rs`) where they live next to the
//! `ComponentLinker` setup that consumes them. C-003 lands the
//! shim foundation:
//!
//!   * pure conversion helpers (`parse_dispatch_result_*`) that turn
//!     `serde_json::Value` returns from `dispatch` into typed Rust
//!     `Result<T, HostErrorRepr>` values
//!   * a typed error mirror (`HostErrorRepr`) matching the WIT
//!     `host-error` variant exactly so C-004 can `.into()` it onto
//!     the bindgen-generated `HostError` without redefining the
//!     classification logic
//!   * an aux `dispatch_param_builder` that constructs the JSON
//!     params shape each `host_functions::host_*` expects
//!
//! Tests live below; they exercise the helpers against
//! hand-crafted JSON, with NO wasmtime instantiation required.

use crate::host_functions::dispatch;
use crate::sandbox::GuestState;
use serde_json::{json, Value};

// ---------------------------------------------------------------------------
// Typed error mirror — alignment with WIT `host-error` variant
// ---------------------------------------------------------------------------

/// Rust mirror of the WIT `host-error` variant. C-004's bindgen output
/// produces an equivalent type under the generated module tree; the
/// `From` impl in C-004 will translate `HostErrorRepr` -> the
/// generated `HostError`. Defining it here lets us classify dispatch
/// errors once and reuse the classification in every interface impl.
///
/// Variants MUST stay in lock-step with `librefang:plugin/host-types`
/// in `crates/librefang-skills/wit/host.wit`. Adding a variant here
/// without adding it to the WIT is a silent contract break.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostErrorRepr {
    /// Plugin's manifest did not declare this capability.
    CapabilityDenied(String),
    /// Path argument escapes the sandbox root or hits a denied prefix.
    PathDenied(String),
    /// URL resolved to a blocked SSRF target.
    SsrfDenied(String),
    /// Operating-system-level I/O failure.
    Io(String),
    /// Argument was structurally invalid.
    InvalidArgument(String),
    /// Operation timed out before the host could complete it.
    Timeout,
    /// Catch-all for host-internal failures.
    Internal(String),
}

impl HostErrorRepr {
    /// Classify a `{"error": "..."}` string from `dispatch` into a
    /// `HostErrorRepr` variant. The dispatch layer doesn't emit
    /// structured error types — it returns a free-form string. We
    /// pattern-match on well-known prefixes the existing
    /// `host_functions.rs` uses to recover structure here without
    /// touching the dispatch implementation.
    ///
    /// Unknown shapes default to `Internal` — preserving the original
    /// message for debugging while not pretending the host knows more
    /// than it does.
    pub fn from_dispatch_message(msg: &str) -> Self {
        let lower = msg.to_ascii_lowercase();

        // host_functions uses "Capability denied" / "denied" phrasing
        // in `check_capability` returns.
        if lower.contains("capability") && lower.contains("den") {
            return HostErrorRepr::CapabilityDenied(msg.to_owned());
        }
        // safe_resolve_path / safe_resolve_parent error returns
        // mention path canonicalization failures.
        if lower.contains("path")
            && (lower.contains("traversal")
                || lower.contains("escape")
                || lower.contains("invalid"))
        {
            return HostErrorRepr::PathDenied(msg.to_owned());
        }
        // is_ssrf_target rejections mention the URL or "SSRF".
        if lower.contains("ssrf") || lower.contains("loopback") || lower.contains("link-local") {
            return HostErrorRepr::SsrfDenied(msg.to_owned());
        }
        // Common io::Error Display prefixes propagated by host_fs_*.
        if lower.contains("io error")
            || lower.contains("failed to read")
            || lower.contains("failed to write")
            || lower.contains("no such file")
        {
            return HostErrorRepr::Io(msg.to_owned());
        }
        // dispatch returns "Missing '<field>' parameter" / "Invalid ..." from arg parse.
        if lower.starts_with("missing '")
            || lower.starts_with("invalid ")
            || lower.contains("must be")
        {
            return HostErrorRepr::InvalidArgument(msg.to_owned());
        }
        if lower.contains("timed out") || lower.contains("timeout") {
            return HostErrorRepr::Timeout;
        }
        HostErrorRepr::Internal(msg.to_owned())
    }
}

// ---------------------------------------------------------------------------
// Dispatch parameter builders
// ---------------------------------------------------------------------------

/// Construct the JSON `params` value the existing
/// `host_functions::host_fs_read` expects: `{"path": "<path>"}`.
pub fn params_fs_read(path: &str) -> Value {
    json!({ "path": path })
}

/// Params for `host_fs_write`: `{"path": "...", "content": "<utf8>"}`.
/// The existing dispatch only accepts UTF-8 content — Component
/// Model `list<u8>` is converted via `String::from_utf8_lossy` here so
/// non-UTF-8 byte sequences degrade rather than fail. A future
/// dispatch upgrade can accept base64 to round-trip arbitrary bytes
/// faithfully.
pub fn params_fs_write(path: &str, body: &[u8]) -> Value {
    json!({
        "path": path,
        "content": String::from_utf8_lossy(body).into_owned(),
    })
}

/// Params for `host_fs_list` (dispatch method `"fs_list"`).
pub fn params_fs_list(path: &str) -> Value {
    json!({ "path": path })
}

/// Params for `host_net_fetch`. Mirrors the wit `http-request` record
/// shape into JSON; the existing dispatch reads `url`, `method`,
/// `headers` (object), `body` (string).
pub fn params_net_fetch(
    method: &str,
    url: &str,
    headers: &[(String, String)],
    body: Option<&[u8]>,
) -> Value {
    let headers_obj: serde_json::Map<String, Value> = headers
        .iter()
        .map(|(k, v)| (k.clone(), Value::String(v.clone())))
        .collect();
    let mut out = json!({
        "method": method,
        "url": url,
        "headers": Value::Object(headers_obj),
    });
    if let Some(b) = body {
        out["body"] = Value::String(String::from_utf8_lossy(b).into_owned());
    }
    out
}

pub fn params_kv_get(key: &str) -> Value {
    json!({ "key": key })
}

pub fn params_kv_set(key: &str, value: &str) -> Value {
    json!({ "key": key, "value": value })
}

pub fn params_agent_send(target_agent: &str, body: &str) -> Value {
    json!({ "agent": target_agent, "message": body })
}

pub fn params_agent_spawn(manifest_ref: &str) -> Value {
    json!({ "manifest": manifest_ref })
}

pub fn params_env_read(name: &str) -> Value {
    json!({ "name": name })
}

// ---------------------------------------------------------------------------
// Dispatch result parsers — shared "ok/error" envelope
// ---------------------------------------------------------------------------

/// Pull a string out of `{"ok": "<s>"}`. Anything else (including
/// non-string ok, missing ok, present error) maps to a `HostErrorRepr`.
pub fn parse_dispatch_result_string(val: Value) -> Result<String, HostErrorRepr> {
    if let Some(err) = val.get("error").and_then(|e| e.as_str()) {
        return Err(HostErrorRepr::from_dispatch_message(err));
    }
    val.get("ok")
        .and_then(|v| v.as_str().map(str::to_owned))
        .ok_or_else(|| HostErrorRepr::Internal(format!("dispatch returned non-string ok: {val}")))
}

/// Pull a UTF-8 string out of `{"ok": "<s>"}` and return it as bytes.
/// Matches the current dispatch shape for `fs_read` which always
/// stringifies file contents.
pub fn parse_dispatch_result_bytes(val: Value) -> Result<Vec<u8>, HostErrorRepr> {
    parse_dispatch_result_string(val).map(String::into_bytes)
}

/// Pull `Option<String>` out of `{"ok": "<s>"}` or `{"ok": null}`.
pub fn parse_dispatch_result_option_string(val: Value) -> Result<Option<String>, HostErrorRepr> {
    if let Some(err) = val.get("error").and_then(|e| e.as_str()) {
        return Err(HostErrorRepr::from_dispatch_message(err));
    }
    match val.get("ok") {
        Some(Value::Null) | None => Ok(None),
        Some(Value::String(s)) => Ok(Some(s.clone())),
        Some(other) => Err(HostErrorRepr::Internal(format!(
            "dispatch returned non-string ok: {other}"
        ))),
    }
}

/// Pull `Vec<String>` out of `{"ok": [...]}`. Matches `fs_list`.
pub fn parse_dispatch_result_list_string(val: Value) -> Result<Vec<String>, HostErrorRepr> {
    if let Some(err) = val.get("error").and_then(|e| e.as_str()) {
        return Err(HostErrorRepr::from_dispatch_message(err));
    }
    val.get("ok")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(str::to_owned))
                .collect()
        })
        .ok_or_else(|| HostErrorRepr::Internal(format!("dispatch returned non-array ok: {val}")))
}

/// Pull a `u64` out of `{"ok": <num>}`. Matches `time_now`.
pub fn parse_dispatch_result_u64(val: Value) -> Result<u64, HostErrorRepr> {
    if let Some(err) = val.get("error").and_then(|e| e.as_str()) {
        return Err(HostErrorRepr::from_dispatch_message(err));
    }
    val.get("ok")
        .and_then(|v| v.as_u64())
        .ok_or_else(|| HostErrorRepr::Internal(format!("dispatch returned non-u64 ok: {val}")))
}

// ---------------------------------------------------------------------------
// Convenience: combined call-and-parse for the canonical pattern.
// C-004's bindgen-generated trait impls collapse to one-liners using
// these helpers.
// ---------------------------------------------------------------------------

/// `dispatch(state, method, &params)` and parse as a string.
pub fn call_string(
    state: &GuestState,
    method: &str,
    params: &Value,
) -> Result<String, HostErrorRepr> {
    parse_dispatch_result_string(dispatch(state, method, params))
}

/// `dispatch(state, method, &params)` and parse as a `Vec<u8>` via
/// `String::into_bytes` (matches current `fs_read` shape).
pub fn call_bytes(
    state: &GuestState,
    method: &str,
    params: &Value,
) -> Result<Vec<u8>, HostErrorRepr> {
    parse_dispatch_result_bytes(dispatch(state, method, params))
}

/// `dispatch` and parse as `Option<String>`. Use for `kv_get`,
/// `env_read`.
pub fn call_option_string(
    state: &GuestState,
    method: &str,
    params: &Value,
) -> Result<Option<String>, HostErrorRepr> {
    parse_dispatch_result_option_string(dispatch(state, method, params))
}

/// `dispatch` and parse as `Vec<String>`. Use for `fs_list`.
pub fn call_list_string(
    state: &GuestState,
    method: &str,
    params: &Value,
) -> Result<Vec<String>, HostErrorRepr> {
    parse_dispatch_result_list_string(dispatch(state, method, params))
}

/// `dispatch` and parse as `u64`. Use for `time_now`.
pub fn call_u64(state: &GuestState, method: &str, params: &Value) -> Result<u64, HostErrorRepr> {
    parse_dispatch_result_u64(dispatch(state, method, params))
}

/// `dispatch` for a unit-returning op and parse `{"ok": ...}` (any
/// shape) as success. Use for `fs_write`, `kv_set`.
pub fn call_unit(state: &GuestState, method: &str, params: &Value) -> Result<(), HostErrorRepr> {
    let val = dispatch(state, method, params);
    if let Some(err) = val.get("error").and_then(|e| e.as_str()) {
        return Err(HostErrorRepr::from_dispatch_message(err));
    }
    if val.get("ok").is_some() {
        Ok(())
    } else {
        Err(HostErrorRepr::Internal(format!(
            "dispatch returned neither ok nor error: {val}"
        )))
    }
}

// ---------------------------------------------------------------------------
// Tests — exercise helpers against hand-crafted JSON without wasmtime.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn classify_capability_denied() {
        let e = HostErrorRepr::from_dispatch_message("Capability denied: FileRead(/etc/passwd)");
        assert!(matches!(e, HostErrorRepr::CapabilityDenied(_)));
    }

    #[test]
    fn classify_ssrf() {
        let e = HostErrorRepr::from_dispatch_message("SSRF: loopback address blocked");
        assert!(matches!(e, HostErrorRepr::SsrfDenied(_)));
    }

    #[test]
    fn classify_path_traversal() {
        let e = HostErrorRepr::from_dispatch_message("invalid path traversal detected");
        assert!(matches!(e, HostErrorRepr::PathDenied(_)));
    }

    #[test]
    fn classify_io() {
        let e = HostErrorRepr::from_dispatch_message(
            "failed to read file: No such file or directory (os error 2)",
        );
        assert!(matches!(e, HostErrorRepr::Io(_)));
    }

    #[test]
    fn classify_invalid_argument() {
        let e = HostErrorRepr::from_dispatch_message("Missing 'path' parameter");
        assert!(matches!(e, HostErrorRepr::InvalidArgument(_)));
    }

    #[test]
    fn classify_timeout() {
        let e = HostErrorRepr::from_dispatch_message("operation timed out after 5s");
        assert!(matches!(e, HostErrorRepr::Timeout));
    }

    #[test]
    fn classify_unknown_falls_to_internal() {
        let e = HostErrorRepr::from_dispatch_message("some weird new error");
        assert!(matches!(e, HostErrorRepr::Internal(_)));
    }

    #[test]
    fn parse_string_ok() {
        let v = json!({"ok": "hello"});
        assert_eq!(parse_dispatch_result_string(v).unwrap(), "hello");
    }

    #[test]
    fn parse_string_error() {
        let v = json!({"error": "Capability denied: FileRead"});
        let err = parse_dispatch_result_string(v).unwrap_err();
        assert!(matches!(err, HostErrorRepr::CapabilityDenied(_)));
    }

    #[test]
    fn parse_string_missing_ok_is_internal() {
        let v = json!({"ok": 42});
        let err = parse_dispatch_result_string(v).unwrap_err();
        assert!(matches!(err, HostErrorRepr::Internal(_)));
    }

    #[test]
    fn parse_bytes_round_trips_utf8() {
        let v = json!({"ok": "hello-bytes"});
        assert_eq!(
            parse_dispatch_result_bytes(v).unwrap(),
            b"hello-bytes".to_vec()
        );
    }

    #[test]
    fn parse_option_string_null_is_none() {
        let v = json!({"ok": null});
        assert_eq!(parse_dispatch_result_option_string(v).unwrap(), None);
    }

    #[test]
    fn parse_option_string_present_is_some() {
        let v = json!({"ok": "value"});
        assert_eq!(
            parse_dispatch_result_option_string(v).unwrap(),
            Some("value".to_owned())
        );
    }

    #[test]
    fn parse_list_string_array() {
        let v = json!({"ok": ["a", "b", "c"]});
        assert_eq!(
            parse_dispatch_result_list_string(v).unwrap(),
            vec!["a".to_owned(), "b".to_owned(), "c".to_owned()]
        );
    }

    #[test]
    fn parse_u64_seconds_since_epoch() {
        let v = json!({"ok": 1_700_000_000u64});
        assert_eq!(parse_dispatch_result_u64(v).unwrap(), 1_700_000_000u64);
    }

    #[test]
    fn params_net_fetch_includes_body_when_present() {
        let p = params_net_fetch(
            "POST",
            "https://example.com",
            &[("x".into(), "y".into())],
            Some(b"payload"),
        );
        assert_eq!(p["method"], "POST");
        assert_eq!(p["url"], "https://example.com");
        assert_eq!(p["headers"]["x"], "y");
        assert_eq!(p["body"], "payload");
    }

    #[test]
    fn params_net_fetch_omits_body_when_none() {
        let p = params_net_fetch("GET", "https://example.com", &[], None);
        assert!(p.get("body").is_none());
    }
}
