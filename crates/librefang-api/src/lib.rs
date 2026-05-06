//! HTTP/WebSocket API server for the LibreFang Agent OS daemon.
//!
//! Exposes agent management, status, and chat via JSON REST endpoints.
//! The kernel runs in-process; the CLI connects over HTTP.

// `routes::config::ui_sections_overlay()` builds a 32-entry
// `serde_json::json!([...])` literal that exceeds the default 128-token macro
// recursion limit. Lift it to 256 — well below `rustc`'s practical ceiling
// and headroom for future sections without re-touching this attribute.
#![recursion_limit = "256"]

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

/// Write `content` to `path` atomically via a sibling temp file + rename.
///
/// The temp file receives a unique name derived from the process ID and a
/// per-process monotonic counter so concurrent writers never share a staging
/// file.  The file is `sync_all`-ed before the rename so a power loss between
/// the two syscalls does not leave a zero-byte file in place of the original.
pub(crate) fn atomic_write(path: &std::path::Path, content: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);

    let mut tmp = path.to_path_buf();
    let file_name = path
        .file_name()
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidInput, "missing filename"))?
        .to_os_string();
    let mut tmp_name = file_name;
    tmp_name.push(format!(".{}.{seq}.tmp", std::process::id()));
    tmp.set_file_name(tmp_name);

    let write_result = (|| -> std::io::Result<()> {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(content)?;
        f.sync_all()?;
        Ok(())
    })();

    if let Err(e) = write_result {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }

    if let Err(e) = std::fs::rename(&tmp, path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    Ok(())
}

pub mod approval;
pub mod channel_bridge;
pub mod client_ip;
pub mod error;
pub mod extensions;
pub mod extractors;
pub mod idempotency;
pub mod mcp_oauth;
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
pub mod trajectory;
pub mod triggers;
pub mod types;
pub mod validation;
pub mod versioning;
pub mod webchat;
pub mod webhook_store;
pub mod workflow;
pub mod ws;

#[cfg(feature = "telemetry")]
pub mod telemetry;
