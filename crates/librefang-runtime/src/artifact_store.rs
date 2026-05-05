//! Artifact store — spill large tool results to disk.
//!
//! When a tool returns a payload larger than `spill_threshold_bytes` the
//! runtime writes the raw bytes here and hands the agent a compact stub:
//!
//! ```text
//! [tool_result: web_fetch | sha256:abc… | 82 340 bytes | preview:]
//! <first 1 024 bytes>
//! -- truncated. Use read_artifact("sha256:abc…", offset, length) to fetch the rest.
//! ```
//!
//! Storage layout:  `<data_dir>/artifacts/<sha256hex>.bin`
//!
//! Writes are atomic (temp-file + rename) and idempotent (existing hash files
//! are not overwritten).  Age-based GC ([`gc_evict_older_than`]) runs once
//! per daemon boot via [`run_startup_gc_once`] (#3347 4/N).

use sha2::{Digest, Sha256};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{Duration, SystemTime};
use tracing::{debug, info, warn};

/// Maximum bytes that `read_artifact` will return per call (64 KiB).
pub const MAX_READ_LENGTH: usize = 64 * 1024;

/// Number of bytes shown in the spill stub preview.
const PREVIEW_BYTES: usize = 1024;

/// Opaque handle prefix that identifies an artifact.
const HANDLE_PREFIX: &str = "sha256:";

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Resolved location of the artifact store directory.
///
/// Priority: `LIBREFANG_HOME` env var > `~/.librefang`, then `artifacts/`
/// sub-directory under `data_dir`.  Callers should pass `config.data_dir`.
pub fn artifact_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("artifacts")
}

/// A handle that uniquely identifies a stored artifact.
///
/// The string form is `sha256:<64-hex-chars>` which is safe to embed in
/// tool-result text and to pass back to `read_artifact`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtifactHandle(String);

impl ArtifactHandle {
    /// The opaque string the agent uses in `read_artifact` calls.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Parse and validate a handle string.
    ///
    /// Returns `Err` when the format does not match `sha256:<64 hex chars>`.
    pub fn parse(s: &str) -> Result<Self, String> {
        let hex = s
            .strip_prefix(HANDLE_PREFIX)
            .ok_or_else(|| format!("invalid artifact handle: must start with '{HANDLE_PREFIX}'"))?;
        if hex.len() != 64 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(format!(
                "invalid artifact handle: expected 64 hex chars after prefix, got '{hex}'"
            ));
        }
        Ok(Self(s.to_string()))
    }

    fn hex(&self) -> &str {
        &self.0[HANDLE_PREFIX.len()..]
    }

    fn file_path(&self, artifact_dir: &Path) -> PathBuf {
        artifact_dir.join(format!("{}.bin", self.hex()))
    }
}

impl std::fmt::Display for ArtifactHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Default per-artifact size cap for spill writes (64 MiB).
pub const DEFAULT_MAX_ARTIFACT_BYTES: u64 = 64 * 1024 * 1024;

/// Write `content` to the artifact store.
///
/// Returns the handle on success.  The write is idempotent: if a file with
/// the same SHA-256 already exists it is not rewritten.
///
/// Returns `Err` when `content` exceeds `max_artifact_bytes` so the caller
/// can fall back to truncation instead of writing an oversized artifact.
///
/// The `artifact_dir` path is created if it does not exist.
pub fn write(
    content: &[u8],
    artifact_dir: &Path,
    max_artifact_bytes: u64,
) -> Result<ArtifactHandle, String> {
    // Per-artifact size cap: refuse the write so the caller falls back to
    // the existing byte-cap truncation path.
    if content.len() as u64 > max_artifact_bytes {
        return Err(format!(
            "artifact too large: {} bytes exceeds max_artifact_bytes ({max_artifact_bytes})",
            content.len()
        ));
    }

    let hash = hex_sha256(content);
    let handle = ArtifactHandle(format!("{HANDLE_PREFIX}{hash}"));
    let dest = handle.file_path(artifact_dir);

    if dest.exists() {
        debug!(
            handle = handle.as_str(),
            "artifact already exists, skipping write"
        );
        return Ok(handle);
    }

    std::fs::create_dir_all(artifact_dir).map_err(|e| {
        format!(
            "failed to create artifact dir {}: {e}",
            artifact_dir.display()
        )
    })?;

    // Atomic write via temp file + rename.
    // Use a unique temp name (hash + pid + nanos) to avoid TOCTOU collisions
    // on Windows where rename fails if the destination already exists and a
    // concurrent writer is racing.  If rename fails and the destination now
    // exists (same hash → same content), treat it as a success.
    let tmp_path = artifact_dir.join(format!(
        "{hash}.{pid}.{nanos}.tmp",
        pid = std::process::id(),
        nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .subsec_nanos(),
    ));
    std::fs::write(&tmp_path, content)
        .map_err(|e| format!("failed to write artifact tmp file: {e}"))?;
    if let Err(e) = std::fs::rename(&tmp_path, &dest) {
        // On Windows rename errors when dest exists; a concurrent writer with
        // the same hash already finished — that's a valid idempotent outcome.
        if dest.exists() {
            // Clean up the orphaned tmp file; ignore any cleanup error.
            let _ = std::fs::remove_file(&tmp_path);
            debug!(
                handle = handle.as_str(),
                "artifact already written by concurrent writer, skipping"
            );
        } else {
            // Genuine rename failure: try to clean up and propagate.
            let _ = std::fs::remove_file(&tmp_path);
            return Err(format!("failed to rename artifact file: {e}"));
        }
    }

    debug!(
        handle = handle.as_str(),
        bytes = content.len(),
        "artifact written"
    );
    Ok(handle)
}

/// Read up to `length` bytes from artifact `handle` starting at `offset`.
///
/// `length` is capped at [`MAX_READ_LENGTH`] silently.
/// Returns `Err` for unknown handles, bad format, or I/O failures.
pub fn read(
    handle: &str,
    offset: usize,
    length: usize,
    artifact_dir: &Path,
) -> Result<Vec<u8>, String> {
    let handle = ArtifactHandle::parse(handle)?;
    let path = handle.file_path(artifact_dir);
    if !path.exists() {
        return Err(format!(
            "artifact not found: {} (it may have been evicted)",
            handle.as_str()
        ));
    }

    let capped_length = length.min(MAX_READ_LENGTH);

    let mut file =
        std::fs::File::open(&path).map_err(|e| format!("failed to open artifact: {e}"))?;

    let file_len = file
        .metadata()
        .map(|m| m.len() as usize)
        .unwrap_or(usize::MAX);

    if offset >= file_len {
        return Ok(Vec::new());
    }

    file.seek(SeekFrom::Start(offset as u64))
        .map_err(|e| format!("seek failed: {e}"))?;

    let to_read = capped_length.min(file_len.saturating_sub(offset));
    let mut buf = vec![0u8; to_read];
    let n = file
        .read(&mut buf)
        .map_err(|e| format!("read failed: {e}"))?;
    buf.truncate(n);
    Ok(buf)
}

// ---------------------------------------------------------------------------
// Spill helper
// ---------------------------------------------------------------------------

/// Build the spill stub that replaces a large tool result in the agent's
/// message history.
///
/// Format:
/// ```text
/// [tool_result: <tool_name> | sha256:<hash> | <total_bytes> bytes | preview:]
/// <first PREVIEW_BYTES bytes (lossily decoded as UTF-8)>
/// -- truncated. Use read_artifact("sha256:<hash>", offset, length) to fetch the rest.
/// ```
pub fn build_spill_stub(tool_name: &str, handle: &ArtifactHandle, content: &[u8]) -> String {
    let preview_len = PREVIEW_BYTES.min(content.len());
    let preview = String::from_utf8_lossy(&content[..preview_len]);
    format!(
        "[tool_result: {tool_name} | {handle} | {} bytes | preview:]\n{preview}\n-- truncated. Use read_artifact(\"{handle}\", offset, length) to fetch the rest.",
        content.len(),
    )
}

/// Attempt to spill `content` to the artifact store.
///
/// Returns `Some(stub)` when spill succeeds, `None` when content is below
/// the threshold or when the write fails (so the caller can fall back to the
/// existing byte-cap truncation path).
///
/// `max_artifact_bytes` caps how large a single artifact may be.  When
/// `content` exceeds this cap the spill is skipped and the caller falls back
/// to truncation.  Pass [`DEFAULT_MAX_ARTIFACT_BYTES`] for the default.
pub fn maybe_spill(
    tool_name: &str,
    content: &[u8],
    threshold: u64,
    max_artifact_bytes: u64,
    artifact_dir: &Path,
) -> Option<String> {
    if content.len() as u64 <= threshold {
        return None;
    }
    match write(content, artifact_dir, max_artifact_bytes) {
        Ok(handle) => Some(build_spill_stub(tool_name, &handle, content)),
        Err(e) => {
            warn!(tool = tool_name, error = %e, "artifact spill failed, falling back to truncation");
            None
        }
    }
}

// ---------------------------------------------------------------------------
// Garbage collection (#3347 4/N)
// ---------------------------------------------------------------------------

/// Outcome of a single [`gc_evict_older_than`] pass.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct GcReport {
    /// Number of files inspected (artifacts plus orphan tmp files).
    pub scanned: usize,
    /// Number of files actually removed.
    pub evicted: usize,
    /// Total bytes freed across all removed files.
    pub bytes_freed: u64,
    /// Per-file errors (stat or remove failures).  Non-fatal: any file the
    /// pass could not handle is left in place and recorded here.
    pub errors: Vec<String>,
}

/// Evict artifacts older than `max_age` from `artifact_dir`.
///
/// Uses each file's modification time.  Both `<hash>.bin` artifacts and
/// orphan `<hash>.<pid>.<nanos>.tmp` files left over from interrupted
/// writes are subject to eviction; other files in the directory are
/// ignored so the function is safe to point at unrelated paths.
///
/// Never returns `Err`: the artifact store is best-effort persistent
/// cache, and a failed GC pass must not crash callers.  Errors are
/// surfaced via the returned [`GcReport`] for logging.
pub fn gc_evict_older_than(artifact_dir: &Path, max_age: Duration) -> GcReport {
    gc_evict_older_than_with_now(artifact_dir, max_age, SystemTime::now())
}

/// Test-friendly variant of [`gc_evict_older_than`] that takes the
/// reference "now" explicitly.  Production callers go through the public
/// wrapper; tests use this to fast-forward time without manipulating
/// filesystem mtimes (which would require a `filetime` dev-dep).
pub(crate) fn gc_evict_older_than_with_now(
    artifact_dir: &Path,
    max_age: Duration,
    now: SystemTime,
) -> GcReport {
    let mut report = GcReport::default();
    let entries = match std::fs::read_dir(artifact_dir) {
        Ok(e) => e,
        Err(e) => {
            // Missing dir is the steady-state on a fresh install; only
            // surface real I/O errors.
            if e.kind() != std::io::ErrorKind::NotFound {
                report
                    .errors
                    .push(format!("read_dir {}: {e}", artifact_dir.display()));
            }
            return report;
        }
    };

    for entry in entries.flatten() {
        let path = entry.path();
        let file_name = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) => n.to_string(),
            None => continue,
        };
        // Only touch artifact-store-owned files.  Skipping non-matching
        // names keeps the GC safe even if the dir is shared by mistake.
        if !file_name.ends_with(".bin") && !file_name.ends_with(".tmp") {
            continue;
        }
        report.scanned += 1;

        let metadata = match entry.metadata() {
            Ok(m) => m,
            Err(e) => {
                report.errors.push(format!("stat {file_name}: {e}"));
                continue;
            }
        };
        // Files without a readable mtime are skipped rather than evicted —
        // we have no evidence they are stale.
        let mtime = match metadata.modified() {
            Ok(t) => t,
            Err(e) => {
                report.errors.push(format!("mtime {file_name}: {e}"));
                continue;
            }
        };
        // `duration_since` errors when mtime is in the future (clock skew);
        // treat that as age-zero so we don't evict freshly-written files.
        let age = now.duration_since(mtime).unwrap_or_default();
        if age <= max_age {
            continue;
        }

        let len = metadata.len();
        match std::fs::remove_file(&path) {
            Ok(()) => {
                report.evicted += 1;
                report.bytes_freed += len;
            }
            Err(e) => {
                report.errors.push(format!("remove {file_name}: {e}"));
            }
        }
    }
    report
}

/// Process-wide guard so [`run_startup_gc_once`] fires at most one async
/// task per daemon, even if multiple subsystems try to trigger it.
static STARTUP_GC_FIRED: OnceLock<()> = OnceLock::new();

/// Spawn a one-shot artifact-store GC in the tokio runtime.
///
/// Idempotent across the lifetime of the process: subsequent calls return
/// without spawning anything.  When `max_age` is zero the call is also a
/// no-op so operators can disable eviction by setting
/// `[tool_results] artifact_max_age_days = 0`.
///
/// The GC itself runs on `spawn_blocking` to keep filesystem I/O off the
/// async reactor; the spawning task only awaits the join handle and logs.
pub fn run_startup_gc_once(artifact_dir: &Path, max_age: Duration) {
    if max_age.is_zero() {
        return;
    }
    if STARTUP_GC_FIRED.set(()).is_err() {
        return;
    }
    let dir = artifact_dir.to_path_buf();
    tokio::spawn(async move {
        let dir_for_log = dir.clone();
        let report = match tokio::task::spawn_blocking(move || gc_evict_older_than(&dir, max_age))
            .await
        {
            Ok(r) => r,
            Err(e) => {
                warn!(error = %e, dir = %dir_for_log.display(), "artifact GC: blocking task panicked");
                return;
            }
        };
        if !report.errors.is_empty() {
            for err in &report.errors {
                warn!(dir = %dir_for_log.display(), error = %err, "artifact GC: per-file error");
            }
        }
        info!(
            dir = %dir_for_log.display(),
            scanned = report.scanned,
            evicted = report.evicted,
            bytes_freed = report.bytes_freed,
            errors = report.errors.len(),
            "artifact GC: startup pass complete"
        );
    });
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

fn hex_sha256(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    format!("{:x}", hasher.finalize())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_write_read() {
        let dir = tempfile::TempDir::new().unwrap();
        let content = b"hello artifact world";
        let handle = write(content, dir.path(), DEFAULT_MAX_ARTIFACT_BYTES).unwrap();
        assert!(handle.as_str().starts_with("sha256:"));

        let got = read(handle.as_str(), 0, 64, dir.path()).unwrap();
        assert_eq!(got, content);
    }

    #[test]
    fn read_with_offset() {
        let dir = tempfile::TempDir::new().unwrap();
        let content = b"0123456789";
        let handle = write(content, dir.path(), DEFAULT_MAX_ARTIFACT_BYTES).unwrap();

        let got = read(handle.as_str(), 3, 4, dir.path()).unwrap();
        assert_eq!(got, b"3456");
    }

    #[test]
    fn read_offset_past_end_returns_empty() {
        let dir = tempfile::TempDir::new().unwrap();
        let content = b"short";
        let handle = write(content, dir.path(), DEFAULT_MAX_ARTIFACT_BYTES).unwrap();

        let got = read(handle.as_str(), 1000, 64, dir.path()).unwrap();
        assert!(got.is_empty());
    }

    #[test]
    fn read_nonexistent_handle_returns_err() {
        let dir = tempfile::TempDir::new().unwrap();
        let fake = "sha256:".to_string() + &"a".repeat(64);
        let err = read(&fake, 0, 64, dir.path()).unwrap_err();
        assert!(err.contains("not found"));
    }

    #[test]
    fn write_is_idempotent() {
        let dir = tempfile::TempDir::new().unwrap();
        let content = b"idempotent";
        let h1 = write(content, dir.path(), DEFAULT_MAX_ARTIFACT_BYTES).unwrap();
        let h2 = write(content, dir.path(), DEFAULT_MAX_ARTIFACT_BYTES).unwrap();
        assert_eq!(h1, h2);
        // Only one file should exist.
        let entries: Vec<_> = std::fs::read_dir(dir.path()).unwrap().collect();
        assert_eq!(entries.len(), 1);
    }

    #[test]
    fn write_rejects_oversized_artifact() {
        let dir = tempfile::TempDir::new().unwrap();
        let content = vec![b'x'; 1000];
        // Cap of 500 bytes — content exceeds cap.
        let err = write(&content, dir.path(), 500).unwrap_err();
        assert!(err.contains("too large"), "expected 'too large' in: {err}");
        // No file should have been written.
        let entries: Vec<_> = std::fs::read_dir(dir.path()).unwrap().collect();
        assert!(entries.is_empty());
    }

    #[test]
    fn parse_handle_rejects_bad_prefix() {
        assert!(ArtifactHandle::parse("md5:abc").is_err());
    }

    #[test]
    fn parse_handle_rejects_short_hex() {
        let bad = "sha256:".to_string() + &"a".repeat(32);
        assert!(ArtifactHandle::parse(&bad).is_err());
    }

    #[test]
    fn corrupted_artifact_returns_err() {
        let dir = tempfile::TempDir::new().unwrap();
        // Write a valid artifact, then corrupt it.
        let content = b"some data";
        let handle = write(content, dir.path(), DEFAULT_MAX_ARTIFACT_BYTES).unwrap();
        let path = ArtifactHandle::parse(handle.as_str())
            .unwrap()
            .file_path(dir.path());
        // Replace with non-seekable garbage by truncating the file.
        std::fs::write(&path, b"").unwrap();
        // read at offset 0 from an empty file should return empty vec (not error).
        let got = read(handle.as_str(), 0, 64, dir.path()).unwrap();
        assert!(got.is_empty());
    }

    #[test]
    fn maybe_spill_below_threshold_returns_none() {
        let dir = tempfile::TempDir::new().unwrap();
        let content = b"small";
        let result = maybe_spill(
            "web_fetch",
            content,
            1000,
            DEFAULT_MAX_ARTIFACT_BYTES,
            dir.path(),
        );
        assert!(result.is_none());
    }

    #[test]
    fn maybe_spill_above_threshold_returns_stub() {
        let dir = tempfile::TempDir::new().unwrap();
        let content = vec![b'x'; 2000];
        let stub = maybe_spill(
            "web_fetch",
            &content,
            100,
            DEFAULT_MAX_ARTIFACT_BYTES,
            dir.path(),
        )
        .unwrap();
        assert!(stub.contains("sha256:"));
        assert!(stub.contains("web_fetch"));
        assert!(stub.contains("read_artifact"));
    }

    #[test]
    fn maybe_spill_oversized_falls_back_to_none() {
        let dir = tempfile::TempDir::new().unwrap();
        // Content is above the spill threshold but also above the per-artifact cap.
        let content = vec![b'x'; 2000];
        let result = maybe_spill("web_fetch", &content, 100, 500, dir.path());
        // Should return None (write rejected) rather than panicking.
        assert!(result.is_none());
    }

    // -----------------------------------------------------------------------
    // GC tests (#3347 4/N)
    // -----------------------------------------------------------------------

    /// Build a "now" timestamp that pretends `extra` has elapsed since the
    /// real wall clock — equivalent to backdating every file in the dir by
    /// `extra` without touching filesystem mtimes.
    fn now_plus(extra: Duration) -> SystemTime {
        SystemTime::now() + extra
    }

    #[test]
    fn gc_missing_dir_returns_clean_report() {
        let dir = tempfile::TempDir::new().unwrap();
        let missing = dir.path().join("does-not-exist");
        let report = gc_evict_older_than(&missing, Duration::from_secs(60));
        assert_eq!(report.scanned, 0);
        assert_eq!(report.evicted, 0);
        assert_eq!(report.bytes_freed, 0);
        assert!(report.errors.is_empty(), "missing dir is not an error");
    }

    #[test]
    fn gc_does_not_evict_fresh_artifact() {
        let dir = tempfile::TempDir::new().unwrap();
        let content = b"fresh content";
        let handle = write(content, dir.path(), DEFAULT_MAX_ARTIFACT_BYTES).unwrap();
        // Pass with a 1-hour max-age — the file we just wrote is < 1s old.
        let report = gc_evict_older_than(dir.path(), Duration::from_secs(3600));
        assert_eq!(report.scanned, 1);
        assert_eq!(report.evicted, 0);
        // Round-trip read still works.
        let got = read(handle.as_str(), 0, 64, dir.path()).unwrap();
        assert_eq!(got, content);
    }

    #[test]
    fn gc_evicts_stale_artifact_and_frees_bytes() {
        let dir = tempfile::TempDir::new().unwrap();
        let content = vec![b'x'; 1234];
        let handle = write(&content, dir.path(), DEFAULT_MAX_ARTIFACT_BYTES).unwrap();
        // Simulate "now" at +2 days; the just-written file's age becomes 2 days.
        let now = now_plus(Duration::from_secs(2 * 24 * 3600));
        let report = gc_evict_older_than_with_now(dir.path(), Duration::from_secs(24 * 3600), now);
        assert_eq!(report.scanned, 1);
        assert_eq!(report.evicted, 1);
        assert_eq!(report.bytes_freed, content.len() as u64);
        assert!(report.errors.is_empty());
        // Re-read after eviction must fail with "not found".
        let err = read(handle.as_str(), 0, 64, dir.path()).unwrap_err();
        assert!(err.contains("not found"), "expected not-found, got: {err}");
    }

    #[test]
    fn gc_evicts_orphan_tmp_files() {
        let dir = tempfile::TempDir::new().unwrap();
        // Synthesize an orphan tmp file matching the writer's naming convention.
        let orphan = dir.path().join("deadbeef.999.111.tmp");
        std::fs::write(&orphan, b"orphan junk").unwrap();
        let now = now_plus(Duration::from_secs(7 * 24 * 3600));
        let report = gc_evict_older_than_with_now(dir.path(), Duration::from_secs(24 * 3600), now);
        assert_eq!(report.scanned, 1);
        assert_eq!(report.evicted, 1);
        assert!(!orphan.exists());
    }

    #[test]
    fn gc_ignores_unrelated_files() {
        let dir = tempfile::TempDir::new().unwrap();
        // A file that does not match the .bin / .tmp suffix must not be touched
        // even when stale, so pointing GC at the wrong directory is safe.
        let unrelated = dir.path().join("README.md");
        std::fs::write(&unrelated, b"important").unwrap();
        let now = now_plus(Duration::from_secs(365 * 24 * 3600));
        let report = gc_evict_older_than_with_now(dir.path(), Duration::from_secs(60), now);
        assert_eq!(report.scanned, 0);
        assert_eq!(report.evicted, 0);
        assert!(unrelated.exists(), "unrelated file must survive GC");
    }

    #[test]
    fn gc_future_mtime_is_not_evicted() {
        // Clock skew can produce an mtime in the future; the GC must treat
        // that as age zero rather than panicking on `duration_since`.
        let dir = tempfile::TempDir::new().unwrap();
        let _ = write(b"future-clock-skew", dir.path(), DEFAULT_MAX_ARTIFACT_BYTES).unwrap();
        // "Now" set far in the past — every file's mtime is "in the future"
        // relative to it.  age must clamp to zero, so nothing is evicted.
        let pretend_now = SystemTime::UNIX_EPOCH;
        let report = gc_evict_older_than_with_now(dir.path(), Duration::ZERO, pretend_now);
        assert_eq!(report.scanned, 1);
        assert_eq!(report.evicted, 0);
    }

    #[tokio::test]
    async fn run_startup_gc_zero_max_age_is_noop() {
        // With max_age = 0 the function returns immediately without firing
        // the OnceLock guard, so subsequent non-zero calls still work.
        let dir = tempfile::TempDir::new().unwrap();
        let content = vec![b'x'; 100];
        let _ = write(&content, dir.path(), DEFAULT_MAX_ARTIFACT_BYTES).unwrap();
        run_startup_gc_once(dir.path(), Duration::ZERO);
        // Yield once — if a task were spawned it would have a chance to run.
        tokio::task::yield_now().await;
        // File still present.
        let entries: Vec<_> = std::fs::read_dir(dir.path()).unwrap().collect();
        assert_eq!(entries.len(), 1, "zero-max-age must not evict anything");
    }
}
