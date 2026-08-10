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
//! `api_key`-shaped strings, JWT tokens, and long base64 blobs. The exporter
//! adds well-known short credential formats that do not meet the baseline
//! blob threshold. The shared baseline patterns must change together, but
//! they are duplicated rather than
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
//! arrays are walked recursively up to 128 container levels so a credential inside
//! `{"tool_result": {"stdout": "API_KEY=sk-..."}}` is caught at any
//! normal depth. A deeper container is replaced wholesale with
//! `<REDACTED:TOO_DEEP>` instead of risking a stack overflow.

use std::sync::OnceLock;

use regex::Regex;
use serde_json::Value;

const MAX_METADATA_DEPTH: usize = 128;
const TOO_DEEP_SENTINEL: &str = "<REDACTED:TOO_DEEP>";

// Pattern source strings. Lifted to module-level `const`s so the
// parity-snapshot test (`tests::regex_set_matches_kernel_snapshot`)
// can compare them byte-for-byte against
// `tests/fixtures/kernel_redaction_patterns.txt`, which mirrors
// `librefang_kernel::trajectory::CompiledPatterns`. Drift on either
// side fails CI loudly — see module docs for why we duplicate the
// patterns rather than depending on `librefang-kernel`.

/// JWT-shaped tokens — three base64url segments separated by dots.
/// Mirrors `librefang_kernel::trajectory::CompiledPatterns::jwt`.
const JWT_PATTERN: &str = r"\beyJ[A-Za-z0-9_\-]{10,}\.[A-Za-z0-9_\-]{10,}\.[A-Za-z0-9_\-]{10,}\b";

/// `sk_live_…`, `api-key=…`, `token: …`, etc. Case-insensitive.
/// Mirrors `librefang_kernel::trajectory::CompiledPatterns::api_key`.
const API_KEY_PATTERN: &str =
    r"(?i)\b(?:sk|api[_-]?key|key|token|secret|bearer)[_\-=:\s]+[A-Za-z0-9_\-]{16,}\b";

/// Well-known credential formats whose complete tokens are shorter than the
/// broad opaque-blob threshold.
const KNOWN_CREDENTIAL_PATTERN: &str = r"\b(?:AKIA[0-9A-Z]{16}|gh[pousr]_[A-Za-z0-9]{36,}|xox[baprs]-[A-Za-z0-9-]{10,}|[psr]k_(?:live|test)_[A-Za-z0-9]{16,})\b";

/// Long opaque base64 blobs (>= 40 chars). Word-bounded.
/// Mirrors `librefang_kernel::trajectory::CompiledPatterns::long_b64`.
const LONG_B64_PATTERN: &str = r"\b[A-Za-z0-9+/=]{40,}\b";

/// Compiled-once regex set. Mirrors
/// `librefang_kernel::trajectory::CompiledPatterns` — see module
/// docs for the rationale on duplication.
struct CompiledPatterns {
    known_credential: Regex,
    api_key: Regex,
    jwt: Regex,
    long_b64: Regex,
}

impl CompiledPatterns {
    fn get() -> &'static CompiledPatterns {
        static PATTERNS: OnceLock<CompiledPatterns> = OnceLock::new();
        PATTERNS.get_or_init(|| CompiledPatterns {
            known_credential: Regex::new(KNOWN_CREDENTIAL_PATTERN)
                .expect("known credential regex must compile"),
            api_key: Regex::new(API_KEY_PATTERN).expect("api_key regex must compile"),
            jwt: Regex::new(JWT_PATTERN).expect("jwt regex must compile"),
            long_b64: Regex::new(LONG_B64_PATTERN).expect("long_b64 regex must compile"),
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
        .known_credential
        .replace_all(&out, "<REDACTED:CREDENTIAL>")
        .into_owned();
    out = p
        .api_key
        .replace_all(&out, "<REDACTED:CREDENTIAL>")
        .into_owned();
    out = p.long_b64.replace_all(&out, "<REDACTED:BLOB>").into_owned();
    out
}

/// Consume a `serde_json::Value` and rewrite every string. Keys are not
/// touched (see module docs). An explicit work stack avoids recursive walk
/// and drop glue on adversarially deep caller-constructed values.
pub(crate) fn redact_metadata(value: Value) -> Value {
    enum Work {
        Visit(Value, usize),
        FinishArray(usize),
        FinishObject(Vec<String>),
    }

    let mut work = vec![Work::Visit(value, 0)];
    let mut output = Vec::new();
    while let Some(item) = work.pop() {
        match item {
            Work::Visit(Value::String(value), _) => {
                output.push(Value::String(redact_string(&value)));
            }
            Work::Visit(value @ (Value::Array(_) | Value::Object(_)), depth)
                if depth >= MAX_METADATA_DEPTH =>
            {
                drop_value_iteratively(value);
                output.push(Value::String(TOO_DEEP_SENTINEL.to_string()));
            }
            Work::Visit(Value::Array(values), depth) => {
                let len = values.len();
                work.push(Work::FinishArray(len));
                work.extend(
                    values
                        .into_iter()
                        .rev()
                        .map(|value| Work::Visit(value, depth + 1)),
                );
            }
            Work::Visit(Value::Object(map), depth) => {
                let (keys, values): (Vec<_>, Vec<_>) = map.into_iter().unzip();
                work.push(Work::FinishObject(keys));
                work.extend(
                    values
                        .into_iter()
                        .rev()
                        .map(|value| Work::Visit(value, depth + 1)),
                );
            }
            Work::Visit(value, _) => output.push(value),
            Work::FinishArray(len) => {
                let values = output.split_off(output.len() - len);
                output.push(Value::Array(values));
            }
            Work::FinishObject(keys) => {
                let values = output.split_off(output.len() - keys.len());
                output.push(Value::Object(keys.into_iter().zip(values).collect()));
            }
        }
    }
    output.pop().expect("redaction always produces one value")
}

fn drop_value_iteratively(value: Value) {
    let mut pending = vec![value];
    while let Some(value) = pending.pop() {
        match value {
            Value::Array(values) => pending.extend(values),
            Value::Object(map) => pending.extend(map.into_values()),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_api_key_in_value() {
        let v = serde_json::json!("API_KEY=sk-live-DO_NOT_LEAK_1234567890");
        let red = redact_metadata(v);
        let s = red.as_str().expect("string value");
        assert!(!s.contains("sk-live-DO_NOT_LEAK"), "credential leaked: {s}");
        assert!(
            s.contains("<REDACTED:CREDENTIAL>"),
            "placeholder missing: {s}"
        );
    }

    #[test]
    fn redacts_well_known_credential_formats_below_blob_threshold() {
        let credentials = [
            ("AK", "IAIOSFODNN7EXAMPLE"),
            ("gh", "p_abcdefghijklmnopqrstuvwxyz0123456789"),
            ("gh", "s_abcdefghijklmnopqrstuvwxyz0123456789"),
            ("xo", "xb-123456789012-123456789012-abcdefghijklmnop"),
            ("xo", "xp-123456789012-123456789012-abcdefghijklmnop"),
            ("rk", "_live_abcdefghijklmnopqrstuvwx"),
            ("pk", "_live_abcdefghijklmnopqrstuvwx"),
        ];
        for (prefix, suffix) in credentials {
            let credential = format!("{prefix}{suffix}");
            let redacted = redact_metadata(Value::String(format!("credential={credential}")));
            let rendered = redacted.as_str().unwrap();
            assert!(
                !rendered.contains(&credential),
                "known credential format leaked: {rendered}"
            );
            assert!(
                rendered.contains("<REDACTED:CREDENTIAL>"),
                "credential placeholder missing: {rendered}"
            );
        }
    }

    #[test]
    fn leaves_near_miss_credential_shapes_intact() {
        let near_misses = [
            ("AK", "IAIOSFODNN7EXAMPL"),
            ("gh", "p_abcdefghijklmnopqrstuvwxyz012345678"),
            ("xo", "xb-123456789"),
            ("pk", "_live_abcdefghijklmno"),
            ("prefixAK", "IAIOSFODNN7EXAMPLE"),
        ];
        for (prefix, suffix) in near_misses {
            let input = format!("{prefix}{suffix}");
            let redacted = redact_metadata(Value::String(input.clone()));
            assert_eq!(redacted, Value::String(input));
        }
    }

    #[test]
    fn redacts_jwt_in_nested_string() {
        // A realistic JWT-shaped string (three base64url segments). The
        // jwt pattern fires before api-key, so this surfaces as
        // <REDACTED:JWT> not <REDACTED:CREDENTIAL>.
        let token =
            "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NSJ9.SflKxwRJSMeKKF2QT4fwpMeJf36POk6yJV_adQssw5c";
        let v = serde_json::json!({"tool_result": {"stdout": format!("auth: {token}")}});
        let red = redact_metadata(v);
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
        let red = redact_metadata(v);
        let rendered = red.to_string();
        assert!(
            !rendered.contains("ABCDEFGHIJ1234567890XYZ"),
            "credential survived nesting: {rendered}",
        );
    }

    #[test]
    fn replaces_metadata_beyond_the_depth_budget() {
        let mut within_budget = Value::String(["token=", "ABCDEFGHIJKLMNOP"].concat());
        for _ in 0..MAX_METADATA_DEPTH {
            within_budget = Value::Array(vec![within_budget]);
        }
        let within_budget = redact_metadata(within_budget).to_string();
        assert!(within_budget.contains("<REDACTED:CREDENTIAL>"));
        assert!(!within_budget.contains("ABCDEFGHIJKLMNOP"));

        let mut value = Value::String("leaf".to_string());
        for _ in 0..50_000 {
            value = Value::Array(vec![value]);
        }

        let redacted = redact_metadata(value);
        assert!(
            redacted.to_string().contains("<REDACTED:TOO_DEEP>"),
            "over-depth metadata must be replaced with a sentinel"
        );
    }

    #[test]
    fn leaves_keys_intact() {
        // Keys are not secret in practice and rewriting them would
        // corrupt the upstream schema. Pin that they pass through.
        let v = serde_json::json!({"api_key_field_name": "harmless"});
        let red = redact_metadata(v);
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
        let red = redact_metadata(v);
        let rendered = red.to_string();
        assert!(rendered.contains("shell"));
        assert!(rendered.contains("rollout for tenant A"));
        assert!(
            !rendered.contains("<REDACTED"),
            "false positive: {rendered}"
        );
    }

    /// Snapshot of the kernel's `RedactionPolicy` regex source strings,
    /// embedded at compile time. See the fixture header for the
    /// sync-on-change contract.
    const KERNEL_FIXTURE: &str = include_str!("../tests/fixtures/kernel_redaction_patterns.txt");

    /// Parse the fixture into `(label, pattern)` rows, skipping comment
    /// and blank lines. The fixture format is documented in the file
    /// header (`# Format:` block).
    fn parse_fixture(raw: &str) -> Vec<(&str, &str)> {
        raw.lines()
            .filter(|l| !l.trim().is_empty() && !l.trim_start().starts_with('#'))
            .map(|l| {
                let (label, pat) = l
                    .split_once('\t')
                    .unwrap_or_else(|| panic!("fixture row missing TAB separator: {l:?}"));
                (label, pat)
            })
            .collect()
    }

    #[test]
    fn regex_set_matches_kernel_snapshot() {
        // Parity-snapshot test. The kernel's `RedactionPolicy` patterns
        // are checked in at `tests/fixtures/kernel_redaction_patterns.txt`
        // (see fixture header). This test fails loudly when either side
        // drifts so the operator must consciously resync rather than
        // discover the gap in production (W&B / Tinker would silently
        // upload an unredacted credential).
        //
        // To resync: edit the fixture to match the kernel's current
        // pattern strings AND update the `*_PATTERN` consts above, or
        // vice versa. The expected resolution is "kernel changed, mirror
        // it here" — the egress crate must never weaken the kernel's
        // policy.
        let fixture = parse_fixture(KERNEL_FIXTURE);
        let local: Vec<(&str, &str)> = vec![
            ("jwt", JWT_PATTERN),
            ("api_key", API_KEY_PATTERN),
            ("long_b64", LONG_B64_PATTERN),
        ];
        assert_eq!(
            fixture, local,
            "redaction-pattern drift between rl-export and kernel snapshot — \
             see tests/fixtures/kernel_redaction_patterns.txt header for resync steps",
        );
    }
}
