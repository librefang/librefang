//! HTTP/WebSocket API server for the LibreFang Agent OS daemon.
//!
//! Exposes agent management, status, and chat via JSON REST endpoints.
//! The kernel runs in-process; the CLI connects over HTTP.

/// Decode percent-encoded strings (e.g. `%2B` -> `+`).
///
/// Used to normalise `?token=` values without using
/// `application/x-www-form-urlencoded` semantics — i.e. literal `+` characters
/// are preserved (not turned into spaces). This matters for base64-derived API
/// keys / session tokens that contain `+`, `/`, or `=`.
///
/// # Timing-side-channel mitigation
///
/// This function is on the WS auth-token decode path
/// ([`crate::ws`]) and the request middleware allowlist path
/// ([`crate::middleware`]). Both feed the decoded value into
/// constant-time comparators (`subtle::ConstantTimeEq` /
/// `matches_any`), so the comparator itself does not leak token
/// content via timing.
///
/// `percent_decode` is **not** itself constant-time: the loop branches
/// on whether each byte is `%`, and on whether the following two bytes
/// are valid hex. An attacker who can probe arbitrary `?token=` values
/// could in theory measure the cost difference between encoded and
/// raw segments. The mitigations layered here are best-effort:
///
/// 1. The output `String::from_utf8` and `Vec` writes touch every
///    byte regardless of branch outcome, so the dominant work is
///    proportional to input length, not match position.
/// 2. We force `std::hint::black_box` over the result so the compiler
///    can't optimise away parts of the computation when the caller
///    happens to discard the value early.
/// 3. The real defense is the per-IP rate limiter sitting in front of
///    the WS handshake (see `rate_limiter.rs`) — it caps how many
///    timing samples an attacker can collect.
pub(crate) fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(hi), Some(lo)) = (hex_val(bytes[i + 1]), hex_val(bytes[i + 2])) {
                out.push(hi << 4 | lo);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    let decoded = String::from_utf8(out).unwrap_or_else(|_| input.to_string());
    // black_box prevents the optimiser from skipping work for the
    // common "all-ASCII, no escapes" path when the caller's downstream
    // use is dead-code-eliminable. Best-effort timing isolation only;
    // the rate limiter is the real defence (see doc above).
    std::hint::black_box(decoded)
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

pub mod channel_bridge;
pub mod middleware;
pub mod oauth;
pub mod openai_compat;
pub mod openapi;
pub mod password_hash;
pub mod rate_limiter;
pub mod routes;
pub mod server;
pub mod stream_chunker;
pub mod stream_dedup;
pub mod terminal;
pub mod terminal_tmux;
pub mod types;
pub mod validation;
pub mod versioning;
pub mod webchat;
pub mod webhook_store;
pub mod ws;

#[cfg(feature = "telemetry")]
pub mod telemetry;
