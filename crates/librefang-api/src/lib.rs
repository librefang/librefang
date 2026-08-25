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

fn lock_a2a_agents<T>(mutex: &std::sync::Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poisoned| {
        tracing::warn!("API A2A agent registry lock poisoned; recovering trusted agent state");
        mutex.clear_poison();
        poisoned.into_inner()
    })
}

#[cfg(test)]
mod a2a_agent_lock_tests {
    use super::lock_a2a_agents;
    use std::sync::Mutex;

    #[test]
    fn poisoned_a2a_lock_recovers_preserved_agents_and_remains_usable() {
        let agents = Mutex::new(vec!["trusted"]);
        let poison = std::thread::scope(|scope| {
            scope
                .spawn(|| {
                    let mut state = agents.lock().unwrap();
                    state.push("discovered");
                    panic!("poison API A2A registry lock");
                })
                .join()
        });
        assert!(poison.is_err());
        assert!(agents.is_poisoned());
        assert_eq!(&*lock_a2a_agents(&agents), &["trusted", "discovered"]);
        assert!(!agents.is_poisoned());

        lock_a2a_agents(&agents).push("later");
        assert_eq!(agents.lock().unwrap().len(), 3);
    }
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
/// The temp file receives a unique name derived from the process ID and a per-process monotonic counter so concurrent writers never share a staging file.
/// The file is `sync_all`-ed before the rename.
/// On Unix, the parent directory is synced after the rename so the new directory entry is durable.
pub(crate) fn atomic_write(path: &std::path::Path, content: &[u8]) -> std::io::Result<()> {
    atomic_write_detailed(path, content).map_err(AtomicWriteError::into_io_error)
}

#[derive(Debug)]
pub(crate) enum AtomicWriteError {
    BeforeCommit(std::io::Error),
    AfterCommit(std::io::Error),
}

impl AtomicWriteError {
    fn into_io_error(self) -> std::io::Error {
        match self {
            Self::BeforeCommit(error) | Self::AfterCommit(error) => error,
        }
    }
}

pub(crate) fn atomic_write_detailed(
    path: &std::path::Path,
    content: &[u8],
) -> Result<(), AtomicWriteError> {
    atomic_write_detailed_with_parent_sync(path, content, |parent| {
        #[cfg(unix)]
        std::fs::File::open(parent)?.sync_all()?;
        #[cfg(not(unix))]
        let _ = parent;
        Ok(())
    })
}

fn atomic_write_detailed_with_parent_sync(
    path: &std::path::Path,
    content: &[u8],
    sync_parent: impl FnOnce(&std::path::Path) -> std::io::Result<()>,
) -> Result<(), AtomicWriteError> {
    use std::io::Write;
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);

    let mut tmp = path.to_path_buf();
    let file_name = path
        .file_name()
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidInput, "missing filename"))
        .map_err(AtomicWriteError::BeforeCommit)?
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
        return Err(AtomicWriteError::BeforeCommit(e));
    }

    if let Err(e) = std::fs::rename(&tmp, path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(AtomicWriteError::BeforeCommit(e));
    }

    // `Path::parent()` returns `Some("")` (not `None`) for a bare
    // relative filename like `config.toml` — `None` only happens for
    // `/` or an empty path itself. Map that empty-but-present case to
    // `.` so we still fsync the actual containing directory instead of
    // failing `File::open("")` with ENOENT after the rename already
    // succeeded.
    let parent = match path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p,
        _ => std::path::Path::new("."),
    };
    sync_parent(parent).map_err(AtomicWriteError::AfterCommit)?;

    Ok(())
}

#[cfg(test)]
mod atomic_write_tests {
    use super::{atomic_write, atomic_write_detailed_with_parent_sync, AtomicWriteError};

    #[test]
    fn replaces_existing_content_and_leaves_no_staging_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("config.toml");
        std::fs::write(&path, b"old").expect("seed file");

        atomic_write(&path, b"new content").expect("atomic write");

        assert_eq!(std::fs::read(&path).expect("read result"), b"new content");
        let entries: Vec<_> = std::fs::read_dir(dir.path())
            .expect("read parent")
            .map(|entry| entry.expect("directory entry").file_name())
            .collect();
        assert_eq!(entries, vec![std::ffi::OsString::from("config.toml")]);
    }

    #[test]
    fn distinguishes_parent_sync_failure_after_replacement() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("config.toml");
        std::fs::write(&path, b"old").expect("seed file");

        let error = atomic_write_detailed_with_parent_sync(&path, b"new", |_| {
            Err(std::io::Error::other("injected parent sync failure"))
        })
        .expect_err("parent sync should fail");

        assert!(matches!(error, AtomicWriteError::AfterCommit(_)));
        assert_eq!(std::fs::read(&path).expect("read result"), b"new");
    }
}

#[cfg(windows)]
pub mod acp_pipe;
#[cfg(unix)]
pub mod acp_uds;
pub mod approval;
pub mod channel_bridge;
pub mod client_ip;
pub mod error;
pub mod everyapi_catalog;
pub mod extensions;
pub mod extractors;
pub mod idempotency;
pub mod mcp_oauth;
pub mod middleware;
pub mod oauth;
pub mod openai_compat;
pub mod openapi;
pub mod openrouter_catalog;
pub mod passkey;
pub mod password_hash;
pub mod rate_limiter;
pub mod routes;
pub mod secrets_env;
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
