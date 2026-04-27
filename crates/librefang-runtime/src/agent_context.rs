//! Per-turn agent context loader for external `context.md` files.
//!
//! Some agents depend on a `context.md` file updated by external tools (e.g. a
//! cron job that writes live market data, or a script that refreshes project
//! state). Before this change the file was read once when the session started
//! and then cached in `CachedWorkspaceMetadata` for the lifetime of the
//! conversation, so external updates never reached the LLM.
//!
//! The default behaviour is now a small disk read per turn when the prompt is
//! assembled. Agents that depend on the old behaviour can opt back in via the
//! `cache_context` flag on their manifest.
//!
//! This module intentionally does not participate in per-token streaming — it
//! is called once per agent turn, right before the system prompt is built.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::{fs, io};

use tracing::{debug, warn};

/// Maximum size of `context.md` to inject into the prompt (32 KB).
///
/// Matches the cap used by the kernel's identity-file reader so a runaway file
/// cannot blow up the prompt.
const MAX_CONTEXT_BYTES: u64 = 32_768;

/// Filename that agents use for per-turn refreshable context.
pub const CONTEXT_FILENAME: &str = "context.md";

/// In-memory cache of the last successful read for each resolved path.
///
/// Used for two purposes:
/// 1. When `cache_context = true`, the first successful read is returned on
///    every subsequent call.
/// 2. When `cache_context = false` and a re-read fails on disk (e.g. the file
///    was temporarily replaced by an external writer), we fall back to the
///    previous content instead of dropping context mid-conversation.
fn cache() -> &'static Mutex<HashMap<PathBuf, String>> {
    static CACHE: OnceLock<Mutex<HashMap<PathBuf, String>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Resolve which `context.md` to read for the workspace.
///
/// Prefers `{workspace}/.identity/context.md` (new layout) and falls back to
/// `{workspace}/context.md` (legacy / unmigrated workspaces). The first
/// candidate that exists on disk wins — even if it is empty or unreadable —
/// so callers can detect and report the canonical location's failures rather
/// than silently picking up a stale legacy file.
fn resolve_context_path(workspace: &Path) -> PathBuf {
    let identity_path = workspace.join(".identity").join(CONTEXT_FILENAME);
    if identity_path.exists() {
        return identity_path;
    }
    workspace.join(CONTEXT_FILENAME)
}

/// Load the agent's `context.md` for this turn.
///
/// Returns the current on-disk content, or — if the read fails after a
/// previous success — the cached content with a warning. Returns `None` when
/// no context.md has ever been seen for this workspace.
///
/// When `cache_context` is true the first successful read is stored and
/// returned verbatim on every future call. Callers pass the flag straight from
/// `AgentManifest::cache_context`.
pub fn load_context_md(workspace: &Path, cache_context: bool) -> Option<String> {
    let path = resolve_context_path(workspace);

    if cache_context {
        if let Some(cached) = get_cached(&path) {
            return Some(cached);
        }
    }

    match read_capped(&path) {
        Ok(Some(content)) => {
            store_cached(&path, &content);
            Some(content)
        }
        Ok(None) => {
            // File is absent or empty — do not serve a stale cache for a
            // deleted file unless the caller explicitly opted into caching.
            if cache_context {
                get_cached(&path)
            } else {
                None
            }
        }
        Err(e) => {
            if let Some(prev) = get_cached(&path) {
                warn!(
                    path = %path.display(),
                    error = %e,
                    "Failed to re-read context.md; falling back to cached content"
                );
                Some(prev)
            } else {
                debug!(path = %path.display(), error = %e, "context.md unreadable and no cache");
                None
            }
        }
    }
}

fn get_cached(path: &Path) -> Option<String> {
    cache()
        .lock()
        .ok()
        .and_then(|guard| guard.get(path).cloned())
}

fn store_cached(path: &Path, content: &str) {
    if let Ok(mut guard) = cache().lock() {
        guard.insert(path.to_path_buf(), content.to_string());
    }
}

/// Read the file, returning `Ok(None)` if it is missing or empty, and
/// `Ok(Some(...))` if it has usable content. Oversized files are truncated to
/// [`MAX_CONTEXT_BYTES`] so prompt size remains bounded.
///
/// The read itself is capped — a multi-GB file will not be slurped into
/// memory just to be truncated afterwards.
fn read_capped(path: &Path) -> io::Result<Option<String>> {
    use std::io::Read;

    // SECURITY: use symlink_metadata so we can refuse symlinks. Without
    // this, an attacker (e.g. via prompt injection that lets the agent
    // create files in its workspace) could point `.identity/context.md`
    // at `/etc/passwd` and have its contents injected into the LLM
    // prompt on the next turn.
    let meta = match fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
    };
    if meta.file_type().is_symlink() {
        warn!(
            path = %path.display(),
            "Refusing to read context.md: target is a symlink"
        );
        return Ok(None);
    }
    if !meta.is_file() {
        return Ok(None);
    }

    // Cap the read at MAX_CONTEXT_BYTES + 4 (max UTF-8 char length) so we
    // never load more than the cap into memory. The +4 slop lets us trim
    // back to the last valid UTF-8 boundary if the cap landed mid-codepoint.
    let cap = (MAX_CONTEXT_BYTES as usize).saturating_add(4);
    let mut bytes = Vec::with_capacity(cap.min((meta.len() as usize).saturating_add(1)));
    fs::File::open(path)?
        .take(cap as u64)
        .read_to_end(&mut bytes)?;

    // Trim to the last valid UTF-8 boundary, in case the cap split a
    // multi-byte character. Any bytes beyond that point are dropped.
    let valid_up_to = match std::str::from_utf8(&bytes) {
        Ok(_) => bytes.len(),
        Err(e) => e.valid_up_to(),
    };
    // If the file contains zero valid UTF-8 bytes (e.g. a binary blob or
    // an interrupted external write), surface this as an I/O error so the
    // caller can fall back to the cached good content rather than serve
    // an empty Live Context section.
    if valid_up_to == 0 && !bytes.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "context.md contains no valid UTF-8 prefix",
        ));
    }
    bytes.truncate(valid_up_to);
    let content = String::from_utf8(bytes).expect("trimmed to valid UTF-8 boundary above");

    if content.trim().is_empty() {
        return Ok(None);
    }

    if meta.len() > MAX_CONTEXT_BYTES {
        let truncated = crate::str_utils::safe_truncate_str(&content, MAX_CONTEXT_BYTES as usize);
        return Ok(Some(truncated.to_string()));
    }
    Ok(Some(content))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn fresh_workspace(tag: &str) -> PathBuf {
        // Unique temp dir per test to avoid cross-test cache pollution.
        let dir = std::env::temp_dir().join(format!(
            "librefang_ctx_{}_{}",
            tag,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn reread_picks_up_external_update() {
        let ws = fresh_workspace("reread");
        let path = ws.join(CONTEXT_FILENAME);

        fs::write(&path, "initial content A").unwrap();
        let first = load_context_md(&ws, false).unwrap();
        assert!(first.contains("initial content A"));

        // External writer updates the file (simulates the cron case).
        {
            let mut f = fs::File::create(&path).unwrap();
            f.write_all(b"updated content B").unwrap();
        }

        let second = load_context_md(&ws, false).unwrap();
        assert!(second.contains("updated content B"));
        assert!(!second.contains("initial content A"));

        let _ = fs::remove_dir_all(&ws);
    }

    #[test]
    fn cache_context_true_freezes_first_read() {
        let ws = fresh_workspace("cache");
        let path = ws.join(CONTEXT_FILENAME);

        fs::write(&path, "frozen A").unwrap();
        let first = load_context_md(&ws, true).unwrap();
        assert!(first.contains("frozen A"));

        fs::write(&path, "never seen B").unwrap();
        let second = load_context_md(&ws, true).unwrap();
        assert_eq!(first, second);
        assert!(!second.contains("never seen B"));

        let _ = fs::remove_dir_all(&ws);
    }

    #[test]
    fn missing_file_returns_none() {
        let ws = fresh_workspace("missing");
        assert!(load_context_md(&ws, false).is_none());
        assert!(load_context_md(&ws, true).is_none());
        let _ = fs::remove_dir_all(&ws);
    }

    #[test]
    fn read_failure_falls_back_to_cache() {
        let ws = fresh_workspace("fallback");
        let path = ws.join(CONTEXT_FILENAME);

        fs::write(&path, "cached payload").unwrap();
        let first = load_context_md(&ws, false).unwrap();
        assert!(first.contains("cached payload"));

        // Write bytes that are not valid UTF-8 so read_to_string returns an
        // IO error. This simulates a transient read failure while an external
        // writer is mid-rewrite.
        {
            let mut f = fs::File::create(&path).unwrap();
            f.write_all(&[0xff, 0xfe, 0xfd, 0x80, 0x81]).unwrap();
        }

        let second = load_context_md(&ws, false);
        assert_eq!(second.as_deref(), Some("cached payload"));

        let _ = fs::remove_dir_all(&ws);
    }

    #[test]
    fn empty_file_treated_as_absent() {
        let ws = fresh_workspace("empty");
        let path = ws.join(CONTEXT_FILENAME);
        fs::write(&path, "   \n\n  ").unwrap();
        assert!(load_context_md(&ws, false).is_none());
        let _ = fs::remove_dir_all(&ws);
    }

    #[test]
    fn identity_dir_takes_precedence_over_root() {
        let ws = fresh_workspace("identity");
        let identity_dir = ws.join(".identity");
        fs::create_dir_all(&identity_dir).unwrap();

        // Both files exist — `.identity/context.md` must win.
        fs::write(ws.join(CONTEXT_FILENAME), "root payload").unwrap();
        fs::write(identity_dir.join(CONTEXT_FILENAME), "identity payload").unwrap();

        let loaded = load_context_md(&ws, false).unwrap();
        assert!(loaded.contains("identity payload"));
        assert!(!loaded.contains("root payload"));

        let _ = fs::remove_dir_all(&ws);
    }

    #[test]
    fn falls_back_to_root_when_identity_dir_missing() {
        let ws = fresh_workspace("rootonly");
        fs::write(ws.join(CONTEXT_FILENAME), "root only payload").unwrap();

        let loaded = load_context_md(&ws, false).unwrap();
        assert!(loaded.contains("root only payload"));

        let _ = fs::remove_dir_all(&ws);
    }

    /// Regression test for the prompt-injection exfil vector caught in
    /// review: a symlinked context.md must NOT be followed, even when the
    /// target is a regular readable file. Without `symlink_metadata` +
    /// explicit refusal, an attacker who can drop a symlink into the agent
    /// workspace could point context.md at /etc/passwd and have its
    /// contents injected into the LLM prompt.
    #[cfg(unix)]
    #[test]
    fn rejects_symlink_context_file() {
        let ws = fresh_workspace("symlink");
        let real = ws.join("real.md");
        fs::write(&real, "would-be-leaked content").unwrap();
        std::os::unix::fs::symlink(&real, ws.join(CONTEXT_FILENAME)).unwrap();

        let loaded = load_context_md(&ws, false);
        assert!(
            loaded.is_none(),
            "symlinked context.md must be refused, got {loaded:?}"
        );

        let _ = fs::remove_dir_all(&ws);
    }
}
