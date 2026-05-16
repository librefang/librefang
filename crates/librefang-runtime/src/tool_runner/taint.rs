//! Outbound-direction taint sinks for tool dispatch.
//!
//! These helpers implement the `TaintSink` checks described in
//! `SECURITY.md` for shell command execution, network fetches, and
//! free-form payloads (agent messages, channel webhook bodies).

use librefang_types::taint::{TaintLabel, TaintSink, TaintedValue};
use std::collections::HashSet;
use tracing::warn;

/// Check if a shell command should be blocked by taint tracking.
///
/// Layer 1: Shell metacharacter injection (backticks, `$(`, `${`, etc.)
/// Layer 2: Heuristic patterns for injected external data (piped curl, base64, eval)
///
/// This implements the TaintSink::shell_exec() policy from SOTA 2.
pub(super) fn check_taint_shell_exec(command: &str) -> Option<String> {
    // Layer 1: Block shell metacharacters that enable command injection.
    // Uses the same validator as subprocess_sandbox and docker_sandbox.
    if let Some(reason) = crate::subprocess_sandbox::contains_shell_metacharacters(command) {
        return Some(format!("Shell metacharacter injection blocked: {reason}"));
    }

    // Layer 2: Heuristic patterns for injected external URLs / base64 payloads
    let suspicious_patterns = ["curl ", "wget ", "| sh", "| bash", "base64 -d", "eval "];
    for pattern in &suspicious_patterns {
        if command.contains(pattern) {
            let mut labels = HashSet::new();
            labels.insert(TaintLabel::ExternalNetwork);
            let tainted = TaintedValue::new(command, labels, "llm_tool_call");
            if let Err(violation) = tainted.check_sink(&TaintSink::shell_exec()) {
                warn!(command = crate::str_utils::safe_truncate_str(command, 80), %violation, "Shell taint check failed");
                return Some(violation.to_string());
            }
        }
    }
    None
}

/// Check if a URL should be blocked by taint tracking before network fetch.
///
/// Blocks URLs that appear to contain API keys, tokens, or other secrets
/// in query parameters (potential data exfiltration). Implements TaintSink::net_fetch().
///
/// Both the raw URL and its percent-decoded query parameter names are
/// checked — an attacker can otherwise bypass the filter with encoding
/// tricks such as `api%5Fkey=secret` (the server decodes `%5F` to `_`
/// and receives the real `api_key=secret`).
pub(super) fn check_taint_net_fetch(url: &str) -> Option<String> {
    const SECRET_KEYS: &[&str] = &["api_key", "apikey", "token", "secret", "password"];

    // Scan 1: raw URL literal for `<key>=` and the Authorization header prefix.
    let url_lower = url.to_lowercase();
    let mut hit = url_lower.contains("authorization:");
    if !hit {
        hit = SECRET_KEYS
            .iter()
            .any(|k| url_lower.contains(&format!("{k}=")));
    }

    // Scan 2: percent-decoded query parameter names. Parsing via
    // `url::Url` decodes each name so `api%5Fkey` becomes `api_key`.
    if !hit {
        if let Ok(parsed) = url::Url::parse(url) {
            for (name, _value) in parsed.query_pairs() {
                let name_lower = name.to_lowercase();
                if SECRET_KEYS.iter().any(|k| name_lower == *k) {
                    hit = true;
                    break;
                }
            }
        }
    }

    if hit {
        let mut labels = HashSet::new();
        labels.insert(TaintLabel::Secret);
        let tainted = TaintedValue::new(url, labels, "llm_tool_call");
        if let Err(violation) = tainted.check_sink(&TaintSink::net_fetch()) {
            warn!(url = crate::str_utils::safe_truncate_str(url, 80), %violation, "Net fetch taint check failed");
            return Some(violation.to_string());
        }
    }
    None
}

/// Check if a free-form string carries an obvious secret shape. Used by
/// exfiltration sinks that don't have a URL query-string structure to
/// parse — `web_fetch` request bodies, `agent_send` message payloads,
/// and (via shared helper) outbound channel / webhook bodies.
///
/// The check is a best-effort denylist: it trips when the text contains
/// an `<assignment-style-key>=<value>` fragment using one of the common
/// secret parameter names (`api_key`, `token`, `secret`, `password`,
/// …), or when it carries an `Authorization:` header prefix, or when it
/// looks like a long contiguous token (e.g. a raw bearer token dropped
/// in as the whole body). Hits are wrapped in a `TaintedValue` and run
/// through the given sink so the rejection message stays consistent
/// with the URL-side checks.
///
/// This is the same "two-sink pattern match" shape described in the
/// SECURITY.md taint section — it is **not** a full information-flow
/// tracker, and copy-pasted obfuscation will still bypass it. The goal
/// is to catch the obvious "the LLM is stuffing OPENAI_API_KEY into an
/// agent_send" shape on the way out, not to prove a data-flow theorem.
const SECRET_KEYS: &[&str] = &[
    "api_key",
    "apikey",
    "api-key",
    "authorization",
    "proxy-authorization",
    "access_token",
    "refresh_token",
    "token",
    "secret",
    "password",
    "passwd",
    "bearer",
    "x-api-key",
];

/// Header names whose mere presence implies the value is a credential,
/// regardless of what the value looks like. `Authorization: Bearer sk-…`
/// has a space between the scheme and the token, which would otherwise
/// defeat the contiguous-token heuristic in `check_taint_outbound_text`.
const SECRET_HEADER_NAMES: &[&str] = &[
    "authorization",
    "proxy-authorization",
    "x-api-key",
    "api-key",
    "apikey",
    "x-auth-token",
    "cookie",
    "set-cookie",
];

/// Check if an HTTP header (name + value) should be blocked. Headers
/// whose name identifies them as credential carriers are rejected
/// unconditionally; everything else falls through to the text-level
/// scanner used for bodies.
pub(super) fn check_taint_outbound_header(
    name: &str,
    value: &str,
    sink: &TaintSink,
) -> Option<String> {
    let name_lower = name.to_ascii_lowercase();
    if SECRET_HEADER_NAMES.iter().any(|h| *h == name_lower)
        || SECRET_KEYS.iter().any(|k| *k == name_lower)
    {
        let mut labels = HashSet::new();
        labels.insert(TaintLabel::Secret);
        let tainted = TaintedValue::new(value, labels, "llm_tool_call");
        if let Err(violation) = tainted.check_sink(sink) {
            warn!(
                sink = %sink.name,
                header = %name_lower,
                value_len = value.len(),
                %violation,
                "Outbound taint check failed (credential header)"
            );
            return Some(violation.to_string());
        }
    }
    // Fall through to the regular body-level scan so e.g. a custom
    // `X-Forwarded-Debug: api_key=sk-…` still gets caught.
    check_taint_outbound_text(value, sink)
}

/// Decide whether a contiguous string "smells like" a raw secret token.
/// Returns false for pure-hex / pure-decimal / single-case alnum blobs
/// so that git commit SHAs, UUIDs-without-dashes, and sha256 digests —
/// which agents legitimately exchange — don't trip the filter. Genuine
/// API tokens tend to include mixed case and/or punctuation
/// (`sk-…`, `ghp_…`, base64 with `+/=`).
fn looks_like_opaque_token(trimmed: &str) -> bool {
    if trimmed.len() < 32 || trimmed.chars().any(char::is_whitespace) {
        return false;
    }
    let charset_ok = trimmed.chars().all(|c| {
        c.is_ascii_alphanumeric()
            || c == '-'
            || c == '_'
            || c == '.'
            || c == '/'
            || c == '+'
            || c == '='
    });
    if !charset_ok {
        return false;
    }
    // Require mixed character classes: either (a) at least one
    // uppercase AND one lowercase letter, or (b) at least one of the
    // token-ish punctuation characters. Pure hex (git SHAs, sha256),
    // pure decimal, and pure single-case alphanumeric all fail this.
    let has_upper = trimmed.chars().any(|c| c.is_ascii_uppercase());
    let has_lower = trimmed.chars().any(|c| c.is_ascii_lowercase());
    let has_punct = trimmed
        .chars()
        .any(|c| matches!(c, '-' | '_' | '.' | '/' | '+' | '='));
    (has_upper && has_lower) || has_punct
}

pub(super) fn check_taint_outbound_text(payload: &str, sink: &TaintSink) -> Option<String> {
    let lower = payload.to_lowercase();

    // Fast path 1: `Authorization:` header literal — unambiguous
    // signal that the LLM is trying to ship credentials in-band.
    let mut hit = lower.contains("authorization:");

    // Fast path 2: `key=value` / `key: value` / `key":` / `'key':`
    // shapes. We match on the key name plus one of a handful of
    // assignment separators so plain prose ("a token of appreciation")
    // doesn't trip the filter.
    if !hit {
        let normalized = lower
            .replace(" = ", "=")
            .replace(" =", "=")
            .replace("= ", "=")
            .replace(" : ", ":")
            .replace(" :", ":")
            .replace(": ", ":");
        for k in SECRET_KEYS {
            for sep in ["=", ":", "\":", "':"] {
                if normalized.contains(&format!("{k}{sep}")) {
                    hit = true;
                    break;
                }
            }
            if hit {
                break;
            }
        }
    }

    // Fast path 3: the payload *is* a long opaque token. Covers the
    // case where the LLM shoves a raw credential into the message
    // without any key/value framing. Matches conservatively — long
    // strings with only base64/hex characters and no whitespace, so
    // natural-language messages don't false-positive. Well-known
    // prefixes (`sk-`, `ghp_`, `xoxp-`) are also flagged regardless
    // of length.
    if !hit {
        let trimmed = payload.trim();
        let well_known_prefix = trimmed.starts_with("sk-")
            || trimmed.starts_with("ghp_")
            || trimmed.starts_with("github_pat_")
            || trimmed.starts_with("xoxp-")
            || trimmed.starts_with("xoxb-")
            || trimmed.starts_with("AKIA")
            || trimmed.starts_with("AIza");
        if looks_like_opaque_token(trimmed) || well_known_prefix {
            hit = true;
        }
    }

    if hit {
        let mut labels = HashSet::new();
        labels.insert(TaintLabel::Secret);
        let tainted = TaintedValue::new(payload, labels, "llm_tool_call");
        if let Err(violation) = tainted.check_sink(sink) {
            // Never log the payload itself: if the heuristic fired, the
            // payload IS the secret we are trying to contain.
            warn!(
                sink = %sink.name,
                payload_len = payload.len(),
                %violation,
                "Outbound taint check failed"
            );
            return Some(violation.to_string());
        }
    }
    None
}
