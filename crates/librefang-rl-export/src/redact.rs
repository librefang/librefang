//! Credential redaction for `toolset_metadata` before egress.
//!
//! W&B forwards the metadata blob to the run page verbatim and Tinker
//! pins it to the session's `user_metadata`. Either destination is an
//! external service the operator did not author and does not control,
//! so any credential-shaped string that slipped through a tool result
//! would leak. This module scrubs the blob in-process before serialize
//! so a tool result containing `API_KEY=sk-live-xxx` lands on the
//! upstream as `<REDACTED:CREDENTIAL>` instead.
//!
//! ## Pattern set
//!
//! The regex set mirrors `librefang_kernel::trajectory::RedactionPolicy`'s
//! default policy (`crates/librefang-kernel/src/trajectory/mod.rs`):
//! `api_key`-shaped strings, JWT tokens, and long base64 blobs. The
//! two must change together — but they are duplicated rather than
//! imported because pulling `librefang-kernel` into a leaf egress
//! crate would invert the dependency layer (the kernel must not
//! depend on `librefang-rl-export`, and a kernel dep here drags in
//! ~50 transitive crates for three regex patterns).
//!
//! ## Scope
//!
//! Only string values are rewritten — JSON keys are left intact (tool
//! input keys carry no secret material in practice and rewriting them
//! would corrupt schemas the upstream may rely on). Nested objects /
//! arrays are walked recursively so a credential inside
//! `{"tool_result": {"stdout": "API_KEY=sk-..."}}` is caught at any
//! depth.

use std::sync::OnceLock;

use regex::Regex;
use serde_json::Value;

/// Compiled-once regex set. Mirrors
/// `librefang_kernel::trajectory::CompiledPatterns` — see module
/// docs for the rationale on duplication.
struct CompiledPatterns {
    api_key: Regex,
    jwt: Regex,
    long_b64: Regex,
}

impl CompiledPatterns {
    fn get() -> &'static CompiledPatterns {
        static PATTERNS: OnceLock<CompiledPatterns> = OnceLock::new();
        PATTERNS.get_or_init(|| CompiledPatterns {
            api_key: Regex::new(
                r"(?i)\b(?:sk|api[_-]?key|key|token|secret|bearer)[_\-=:\s]+[A-Za-z0-9_\-]{16,}\b",
            )
            .expect("api_key regex must compile"),
            jwt: Regex::new(
                r"\beyJ[A-Za-z0-9_\-]{10,}\.[A-Za-z0-9_\-]{10,}\.[A-Za-z0-9_\-]{10,}\b",
            )
            .expect("jwt regex must compile"),
            long_b64: Regex::new(r"\b[A-Za-z0-9+/=]{40,}\b").expect("long_b64 regex must compile"),
        })
    }
}

/// Scrub credential-shaped substrings out of a single string. JWT is
/// matched first (most specific shape), then api-key, then the broad
/// long-base64 catch-all. Mirrors the order in
/// `librefang_kernel::trajectory::TrajectoryExporter::redact_text`.
fn redact_string(input: &str) -> String {
    let p = CompiledPatterns::get();
    let mut out = p.jwt.replace_all(input, "<REDACTED:JWT>").into_owned();
    out = p
        .api_key
        .replace_all(&out, "<REDACTED:CREDENTIAL>")
        .into_owned();
    out = p.long_b64.replace_all(&out, "<REDACTED:BLOB>").into_owned();
    out
}

/// Walk a `serde_json::Value` and rewrite every string in-place. Keys
/// are not touched (see module docs).
pub(crate) fn redact_metadata(value: &Value) -> Value {
    match value {
        Value::String(s) => Value::String(redact_string(s)),
        Value::Array(arr) => Value::Array(arr.iter().map(redact_metadata).collect()),
        Value::Object(map) => {
            let mut out = serde_json::Map::with_capacity(map.len());
            for (k, v) in map {
                out.insert(k.clone(), redact_metadata(v));
            }
            Value::Object(out)
        }
        other => other.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_api_key_in_value() {
        let v = serde_json::json!("API_KEY=sk-live-DO_NOT_LEAK_1234567890");
        let red = redact_metadata(&v);
        let s = red.as_str().expect("string value");
        assert!(!s.contains("sk-live-DO_NOT_LEAK"), "credential leaked: {s}");
        assert!(
            s.contains("<REDACTED:CREDENTIAL>"),
            "placeholder missing: {s}"
        );
    }

    #[test]
    fn redacts_jwt_in_nested_string() {
        // A realistic JWT-shaped string (three base64url segments). The
        // jwt pattern fires before api-key, so this surfaces as
        // <REDACTED:JWT> not <REDACTED:CREDENTIAL>.
        let token =
            "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NSJ9.SflKxwRJSMeKKF2QT4fwpMeJf36POk6yJV_adQssw5c";
        let v = serde_json::json!({"tool_result": {"stdout": format!("auth: {token}")}});
        let red = redact_metadata(&v);
        let rendered = red.to_string();
        assert!(!rendered.contains(token), "JWT leaked: {rendered}");
        assert!(
            rendered.contains("<REDACTED:JWT>"),
            "JWT placeholder: {rendered}"
        );
    }

    #[test]
    fn redacts_credential_at_arbitrary_depth() {
        // Deep nesting + array — the walker must descend through
        // both shapes.
        let v = serde_json::json!({
            "level1": [
                {"level2": {"level3": "secret token=ABCDEFGHIJ1234567890XYZ"}}
            ]
        });
        let red = redact_metadata(&v);
        let rendered = red.to_string();
        assert!(
            !rendered.contains("ABCDEFGHIJ1234567890XYZ"),
            "credential survived nesting: {rendered}",
        );
    }

    #[test]
    fn leaves_keys_intact() {
        // Keys are not secret in practice and rewriting them would
        // corrupt the upstream schema. Pin that they pass through.
        let v = serde_json::json!({"api_key_field_name": "harmless"});
        let red = redact_metadata(&v);
        let obj = red.as_object().expect("object");
        assert!(
            obj.contains_key("api_key_field_name"),
            "key was rewritten: {red:?}"
        );
    }

    #[test]
    fn leaves_non_credential_strings_intact() {
        // Short tool names and harmless prose must not be touched —
        // overscrubbing would corrupt the metadata operators rely on.
        let v = serde_json::json!({
            "tools": ["shell", "fetch"],
            "description": "rollout for tenant A",
        });
        let red = redact_metadata(&v);
        let rendered = red.to_string();
        assert!(rendered.contains("shell"));
        assert!(rendered.contains("rollout for tenant A"));
        assert!(
            !rendered.contains("<REDACTED"),
            "false positive: {rendered}"
        );
    }
}
