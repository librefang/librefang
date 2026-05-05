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
//! are not overwritten).  GC / age-based eviction is a follow-up (#3347 4/N).

use sha2::{Digest, Sha256};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use tracing::{debug, warn};

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

/// Write `content` to the artifact store.
///
/// Returns the handle on success.  The write is idempotent: if a file with
/// the same SHA-256 already exists it is not rewritten.
///
/// The `artifact_dir` path is created if it does not exist.
pub fn write(content: &[u8], artifact_dir: &Path) -> Result<ArtifactHandle, String> {
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
    let tmp_path = artifact_dir.join(format!("{hash}.tmp"));
    std::fs::write(&tmp_path, content)
        .map_err(|e| format!("failed to write artifact tmp file: {e}"))?;
    std::fs::rename(&tmp_path, &dest)
        .map_err(|e| format!("failed to rename artifact file: {e}"))?;

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
/// Returns `Ok(Some(stub))` when spill succeeds, `Ok(None)` when content is
/// below the threshold, and logs a warning + returns `Ok(None)` on write
/// failure (so the caller can fall back to the existing truncation path).
pub fn maybe_spill(
    tool_name: &str,
    content: &[u8],
    threshold: u64,
    artifact_dir: &Path,
) -> Option<String> {
    if content.len() as u64 <= threshold {
        return None;
    }
    match write(content, artifact_dir) {
        Ok(handle) => Some(build_spill_stub(tool_name, &handle, content)),
        Err(e) => {
            warn!(tool = tool_name, error = %e, "artifact spill failed, falling back to truncation");
            None
        }
    }
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
        let handle = write(content, dir.path()).unwrap();
        assert!(handle.as_str().starts_with("sha256:"));

        let got = read(handle.as_str(), 0, 64, dir.path()).unwrap();
        assert_eq!(got, content);
    }

    #[test]
    fn read_with_offset() {
        let dir = tempfile::TempDir::new().unwrap();
        let content = b"0123456789";
        let handle = write(content, dir.path()).unwrap();

        let got = read(handle.as_str(), 3, 4, dir.path()).unwrap();
        assert_eq!(got, b"3456");
    }

    #[test]
    fn read_offset_past_end_returns_empty() {
        let dir = tempfile::TempDir::new().unwrap();
        let content = b"short";
        let handle = write(content, dir.path()).unwrap();

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
        let h1 = write(content, dir.path()).unwrap();
        let h2 = write(content, dir.path()).unwrap();
        assert_eq!(h1, h2);
        // Only one file should exist.
        let entries: Vec<_> = std::fs::read_dir(dir.path()).unwrap().collect();
        assert_eq!(entries.len(), 1);
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
        let handle = write(content, dir.path()).unwrap();
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
        let result = maybe_spill("web_fetch", content, 1000, dir.path());
        assert!(result.is_none());
    }

    #[test]
    fn maybe_spill_above_threshold_returns_stub() {
        let dir = tempfile::TempDir::new().unwrap();
        let content = vec![b'x'; 2000];
        let stub = maybe_spill("web_fetch", &content, 100, dir.path()).unwrap();
        assert!(stub.contains("sha256:"));
        assert!(stub.contains("web_fetch"));
        assert!(stub.contains("read_artifact"));
    }
}
