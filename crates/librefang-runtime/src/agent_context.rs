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
use std::sync::{Mutex, MutexGuard, OnceLock};
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

fn lock_cache() -> MutexGuard<'static, HashMap<PathBuf, String>> {
    cache().lock().unwrap_or_else(|poisoned| {
        warn!("Agent context cache lock poisoned; recovering cached context");
        // `into_inner()` only unwraps this guard; it does not reset the mutex's poison flag, so without `clear_poison()` every future access re-enters this branch and re-logs forever.
        cache().clear_poison();
        poisoned.into_inner()
    })
}

/// Resolve which `context.md` to read for the workspace.
///
/// Prefers `{workspace}/.identity/context.md` (new layout) and falls back to `{workspace}/context.md` (legacy / unmigrated workspaces).
/// The first candidate wins only when it is a regular file.
/// Symlinks and other special entries fall through to the legacy file instead of shadowing it.
fn resolve_context_path(workspace: &Path) -> PathBuf {
    let identity_dir = workspace.join(".identity");
    let identity_path = identity_dir.join(CONTEXT_FILENAME);
    let identity_dir_meta = fs::symlink_metadata(&identity_dir);
    let identity_meta = fs::symlink_metadata(&identity_path);
    let regular_identity = matches!(&identity_dir_meta, Ok(meta) if meta.is_dir())
        && matches!(&identity_meta, Ok(meta) if meta.is_file());
    let unsafe_identity = identity_dir_meta.is_ok()
        && (!matches!(&identity_dir_meta, Ok(meta) if meta.is_dir())
            || matches!(&identity_meta, Ok(meta) if !meta.is_file()));
    if regular_identity || (unsafe_identity && get_cached(&identity_path).is_some()) {
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

/// Async variant of [`load_context_md`] that performs the per-turn disk read off the Tokio worker thread via `spawn_blocking`.
/// Use this from any `async fn` running on the runtime — it matches the sync version's behaviour byte-for-byte (same cache, same symlink rejection, same UTF-8 trim and size cap) but never parks the executor on the read.
///
/// The sync [`load_context_md`] is retained for the streaming entry
/// point (`send_message_streaming_with_sender_and_opts`) which is itself
/// a non-async wrapper that returns a `JoinHandle` — async-ifying that
/// call site requires lifting an entire kernel entry path to async and
/// is tracked as a follow-up under #3579.
pub async fn load_context_md_async(workspace: &Path, cache_context: bool) -> Option<String> {
    let path = resolve_context_path_async(workspace).await;

    if cache_context {
        if let Some(cached) = get_cached(&path) {
            return Some(cached);
        }
    }

    match read_capped_async(&path).await {
        Ok(Some(content)) => {
            store_cached(&path, &content);
            Some(content)
        }
        Ok(None) => {
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

async fn resolve_context_path_async(workspace: &Path) -> PathBuf {
    let identity_dir = workspace.join(".identity");
    let identity_path = identity_dir.join(CONTEXT_FILENAME);
    let identity_dir_meta = tokio::fs::symlink_metadata(&identity_dir).await;
    let identity_meta = tokio::fs::symlink_metadata(&identity_path).await;
    let regular_identity = matches!(&identity_dir_meta, Ok(meta) if meta.is_dir())
        && matches!(&identity_meta, Ok(meta) if meta.is_file());
    let unsafe_identity = identity_dir_meta.is_ok()
        && (!matches!(&identity_dir_meta, Ok(meta) if meta.is_dir())
            || matches!(&identity_meta, Ok(meta) if !meta.is_file()));
    if regular_identity || (unsafe_identity && get_cached(&identity_path).is_some()) {
        return identity_path;
    }
    workspace.join(CONTEXT_FILENAME)
}

/// Async mirror of [`read_capped`].
/// The blocking-pool hop keeps filesystem I/O off Tokio workers while reusing the exact same no-follow open primitive.
async fn read_capped_async(path: &Path) -> io::Result<Option<String>> {
    let path = path.to_path_buf();
    match tokio::task::spawn_blocking(move || read_capped(&path)).await {
        Ok(result) => result,
        Err(error) => Err(io::Error::other(format!(
            "context.md blocking read task failed: {error}"
        ))),
    }
}

fn get_cached(path: &Path) -> Option<String> {
    lock_cache().get(path).cloned()
}

fn store_cached(path: &Path, content: &str) {
    lock_cache().insert(path.to_path_buf(), content.to_string());
}

#[cfg(any(windows, test))]
fn windows_path_is_beneath(root: &[u16], candidate: &[u16]) -> bool {
    if candidate.len() <= root.len() || !candidate.starts_with(root) {
        return false;
    }
    // Compare the normalized handle paths exactly.
    // Windows can enable per-directory case sensitivity, where `Foo` and `foo` are distinct siblings; a case-insensitive prefix check would escape the root.
    root.last()
        .is_some_and(|c| *c == b'\\' as u16 || *c == b'/' as u16)
        || matches!(candidate.get(root.len()), Some(c) if *c == b'\\' as u16 || *c == b'/' as u16)
}

#[cfg(unix)]
fn open_context_file(path: &Path) -> io::Result<fs::File> {
    use std::ffi::{CString, OsStr};
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
    use std::os::unix::ffi::OsStrExt;

    fn open_dir(path: &Path) -> io::Result<OwnedFd> {
        let path = CString::new(path.as_os_str().as_bytes()).map_err(|_| {
            io::Error::new(io::ErrorKind::InvalidInput, "context path contains NUL")
        })?;
        // SAFETY: `path` is a live NUL-terminated C string; no mode argument is needed because O_CREAT is absent.
        // The returned fd is owned below.
        let fd = unsafe {
            libc::open(
                path.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
            )
        };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: `fd` is a fresh successful `open` result and ownership is transferred exactly once into OwnedFd.
        Ok(unsafe { OwnedFd::from_raw_fd(fd) })
    }

    fn open_at(dir: &OwnedFd, name: &OsStr, directory: bool) -> io::Result<OwnedFd> {
        let name = CString::new(name.as_bytes()).map_err(|_| {
            io::Error::new(io::ErrorKind::InvalidInput, "context path contains NUL")
        })?;
        let mut flags = libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NOFOLLOW;
        if directory {
            flags |= libc::O_DIRECTORY;
        } else {
            // Opening a FIFO read-only blocks before we can reject it via fstat.
            // Regular files ignore O_NONBLOCK.
            flags |= libc::O_NONBLOCK;
        }
        // SAFETY: `dir` remains open for the call and `name` is a live NUL-terminated relative component.
        // The returned fd is owned below.
        let fd = unsafe { libc::openat(dir.as_raw_fd(), name.as_ptr(), flags) };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: `fd` is a fresh successful `openat` result and ownership is transferred exactly once into OwnedFd.
        Ok(unsafe { OwnedFd::from_raw_fd(fd) })
    }

    let parent = path
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "context path has no parent"))?;
    let (workspace, identity_relative) = if parent.file_name() == Some(OsStr::new(".identity")) {
        (
            parent.parent().ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "identity path has no workspace",
                )
            })?,
            true,
        )
    } else {
        (parent, false)
    };
    let workspace = open_dir(workspace)?;
    let context_dir = if identity_relative {
        open_at(&workspace, OsStr::new(".identity"), true)?
    } else {
        workspace
    };
    let filename = path.file_name().ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidInput, "context path has no filename")
    })?;
    let file = open_at(&context_dir, filename, false)?;
    Ok(fs::File::from(file))
}

#[cfg(windows)]
fn open_context_file(path: &Path) -> io::Result<fs::File> {
    use std::ffi::c_void;
    use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
    use std::os::windows::io::AsRawHandle;

    type Handle = *mut c_void;

    #[link(name = "kernel32")]
    extern "system" {
        fn GetFinalPathNameByHandleW(
            file: Handle,
            path: *mut u16,
            path_len: u32,
            flags: u32,
        ) -> u32;
    }

    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0000_0400;
    const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
    const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
    const FILE_NAME_NORMALIZED: u32 = 0;
    const VOLUME_NAME_DOS: u32 = 0;

    fn final_path(file: &fs::File) -> io::Result<Vec<u16>> {
        let mut buffer = vec![0_u16; 512];
        loop {
            // SAFETY: the file handle remains live and `buffer` provides the writable capacity reported to the Windows API.
            let len = unsafe {
                GetFinalPathNameByHandleW(
                    file.as_raw_handle().cast(),
                    buffer.as_mut_ptr(),
                    buffer.len().try_into().unwrap_or(u32::MAX),
                    FILE_NAME_NORMALIZED | VOLUME_NAME_DOS,
                )
            };
            if len == 0 {
                return Err(io::Error::last_os_error());
            }
            let len = len as usize;
            if len < buffer.len() {
                buffer.truncate(len);
                return Ok(buffer);
            }
            buffer.resize(len.saturating_add(1), 0);
        }
    }

    fn open_reparse_point(path: &Path, directory: bool) -> io::Result<fs::File> {
        let mut options = fs::OpenOptions::new();
        options.read(true);
        let mut flags = FILE_FLAG_OPEN_REPARSE_POINT;
        if directory {
            flags |= FILE_FLAG_BACKUP_SEMANTICS;
        }
        options.custom_flags(flags).open(path)
    }

    let parent = path
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "context path has no parent"))?;
    let workspace = if parent.file_name() == Some(std::ffi::OsStr::new(".identity")) {
        parent.parent().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "identity path has no workspace",
            )
        })?
    } else {
        parent
    };

    let workspace_handle = open_reparse_point(workspace, true)?;
    let workspace_meta = workspace_handle.metadata()?;
    if !workspace_meta.is_dir()
        || workspace_meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "context workspace is not a regular directory",
        ));
    }

    let file = open_reparse_point(path, false)?;
    let file_meta = file.metadata()?;
    if file_meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "context.md is a reparse point",
        ));
    }

    // The final path comes from the already-opened handle.
    // If an intermediate directory was swapped for a junction, this resolves outside the opened workspace and is rejected before any bytes are read.
    let workspace_path = final_path(&workspace_handle)?;
    let file_path = final_path(&file)?;
    if !windows_path_is_beneath(&workspace_path, &file_path) {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "context.md resolved outside its workspace",
        ));
    }

    Ok(file)
}

#[cfg(not(any(unix, windows)))]
fn open_context_file(_path: &Path) -> io::Result<fs::File> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "secure no-follow context reads are unsupported on this platform",
    ))
}

/// Read the file, returning `Ok(None)` if it is missing or empty, and
/// `Ok(Some(...))` if it has usable content. Oversized files are truncated to
/// [`MAX_CONTEXT_BYTES`] so prompt size remains bounded.
///
/// The read itself is capped — a multi-GB file will not be slurped into
/// memory just to be truncated afterwards.
fn read_capped(path: &Path) -> io::Result<Option<String>> {
    use std::io::Read;

    // SECURITY: the no-follow flag and metadata check apply to the same open handle.
    // A path swap cannot redirect this read to a symlink target.
    let file = match open_context_file(path) {
        Ok(file) => file,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
    };
    let meta = file.metadata()?;
    if !meta.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "context.md is not a regular file",
        ));
    }

    // Cap the read at MAX_CONTEXT_BYTES + 4 (max UTF-8 char length) so we
    // never load more than the cap into memory. The +4 slop lets us trim
    // back to the last valid UTF-8 boundary if the cap landed mid-codepoint.
    let cap = (MAX_CONTEXT_BYTES as usize).saturating_add(4);
    let mut bytes = Vec::with_capacity(cap.min((meta.len() as usize).saturating_add(1)));
    file.take(cap as u64).read_to_end(&mut bytes)?;

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

    /// Serializes the tests that touch the process-global context cache.
    ///
    /// [`poisoned_cache_lock_recovers_cached_context`] asserts on `cache().is_poisoned()`, and *any* cache access clears that flag by design — so a cache-touching test running concurrently on another libtest thread fails that assertion for reasons that have nothing to do with the behaviour under test.
    /// Under `cargo nextest`, which CI runs, every test owns its own process and the question never arises; under `cargo test` they share one, so the tests take this guard rather than depending on the scheduler.
    static CACHE_SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// Take [`CACHE_SERIAL`], recovering it when a previous test panicked while holding it.
    fn cache_serial() -> std::sync::MutexGuard<'static, ()> {
        CACHE_SERIAL.lock().unwrap_or_else(|e| e.into_inner())
    }

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
    fn poisoned_cache_lock_recovers_cached_context() {
        let _serial = cache_serial();
        let poison = std::thread::spawn(|| {
            let _guard = cache().lock().unwrap();
            panic!("poison agent context cache lock");
        })
        .join();
        assert!(poison.is_err());
        assert!(cache().is_poisoned());

        let path = fresh_workspace("poison_recovery").join(CONTEXT_FILENAME);
        store_cached(&path, "first cached value");
        assert_eq!(get_cached(&path).as_deref(), Some("first cached value"));
        // The first post-panic access must clear the poison flag, not just unwrap around it — otherwise every subsequent access re-triggers the recovery branch (and its `warn!`) for the rest of the process.
        assert!(!cache().is_poisoned());

        store_cached(&path, "updated cached value");
        assert_eq!(get_cached(&path).as_deref(), Some("updated cached value"));
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn reread_picks_up_external_update() {
        let _serial = cache_serial();
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
        let _serial = cache_serial();
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
        let _serial = cache_serial();
        let ws = fresh_workspace("missing");
        assert!(load_context_md(&ws, false).is_none());
        assert!(load_context_md(&ws, true).is_none());
        let _ = fs::remove_dir_all(&ws);
    }

    #[test]
    fn read_failure_falls_back_to_cache() {
        let _serial = cache_serial();
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
        let _serial = cache_serial();
        let ws = fresh_workspace("empty");
        let path = ws.join(CONTEXT_FILENAME);
        fs::write(&path, "   \n\n  ").unwrap();
        assert!(load_context_md(&ws, false).is_none());
        let _ = fs::remove_dir_all(&ws);
    }

    #[test]
    fn identity_dir_takes_precedence_over_root() {
        let _serial = cache_serial();
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
        let _serial = cache_serial();
        let ws = fresh_workspace("rootonly");
        fs::write(ws.join(CONTEXT_FILENAME), "root only payload").unwrap();

        let loaded = load_context_md(&ws, false).unwrap();
        assert!(loaded.contains("root only payload"));

        let _ = fs::remove_dir_all(&ws);
    }

    /// Regression test for the prompt-injection exfil vector caught in review: a symlinked context.md must NOT be followed, even when the target is a regular readable file.
    /// Without a no-follow open bound to the subsequent handle checks, an attacker who can drop a symlink into the agent workspace could point context.md at /etc/passwd and have its contents injected into the LLM prompt.
    #[cfg(unix)]
    #[test]
    fn rejects_symlink_context_file() {
        let _serial = cache_serial();
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

    #[cfg(unix)]
    #[test]
    fn context_file_opener_does_not_follow_symlinks() {
        let _serial = cache_serial();
        let ws = fresh_workspace("nofollow_open");
        let real = ws.join("real.md");
        let link = ws.join(CONTEXT_FILENAME);
        fs::write(&real, "secret target").unwrap();
        std::os::unix::fs::symlink(&real, &link).unwrap();

        assert!(
            open_context_file(&link).is_err(),
            "the file open operation itself must reject a symlink"
        );
        let _ = fs::remove_dir_all(&ws);
    }

    #[test]
    fn windows_handle_path_containment_is_exact_and_component_bounded() {
        let _serial = cache_serial();
        let root: Vec<u16> = r"\\?\C:\work\Foo".encode_utf16().collect();
        let child: Vec<u16> = r"\\?\C:\work\Foo\context.md".encode_utf16().collect();
        let case_distinct_sibling: Vec<u16> =
            r"\\?\C:\work\foo\context.md".encode_utf16().collect();
        let prefix_sibling: Vec<u16> = r"\\?\C:\work\Foobar\context.md".encode_utf16().collect();

        assert!(windows_path_is_beneath(&root, &child));
        assert!(!windows_path_is_beneath(&root, &case_distinct_sibling));
        assert!(!windows_path_is_beneath(&root, &prefix_sibling));
    }

    #[cfg(unix)]
    #[test]
    fn context_reader_does_not_block_on_fifo() {
        let _serial = cache_serial();
        use std::ffi::CString;
        use std::os::unix::ffi::OsStrExt;
        use std::sync::mpsc;
        use std::time::Duration;

        let ws = fresh_workspace("nofollow_fifo");
        let fifo = ws.join(CONTEXT_FILENAME);
        let fifo_c = CString::new(fifo.as_os_str().as_bytes()).unwrap();
        // SAFETY: `fifo_c` is a live NUL-terminated path and the mode contains only ordinary permission bits.
        assert_eq!(unsafe { libc::mkfifo(fifo_c.as_ptr(), 0o600) }, 0);

        let (tx, rx) = mpsc::channel();
        let worker = std::thread::spawn(move || {
            let _ = tx.send(read_capped(&fifo).map(|_| ()));
        });
        // Wait for the actual completion event.
        // The deadline is only a failure bound proving that a FIFO open did not block waiting for a writer.
        let result = rx
            .recv_timeout(Duration::from_secs(2))
            .expect("opening a FIFO must not wait for a writer");
        worker.join().unwrap();
        assert!(result.is_err(), "a FIFO must not be accepted as context");
        let _ = fs::remove_dir_all(&ws);
    }

    #[cfg(windows)]
    #[test]
    fn windows_context_file_opener_rejects_file_symlink() {
        let _serial = cache_serial();
        let ws = fresh_workspace("windows_file_symlink");
        let target = ws.join("target.md");
        let link = ws.join(CONTEXT_FILENAME);
        fs::write(&target, "must not load").unwrap();
        if let Err(error) = std::os::windows::fs::symlink_file(&target, &link) {
            // Windows requires Developer Mode or SeCreateSymbolicLinkPrivilege for this fixture.
            // The production check remains compiled even on hosts where the test account cannot create reparse points.
            if error.kind() == io::ErrorKind::PermissionDenied {
                let _ = fs::remove_dir_all(&ws);
                return;
            }
            panic!("failed to create file symlink fixture: {error}");
        }

        assert!(open_context_file(&link).is_err());
        let _ = fs::remove_dir_all(&ws);
    }

    #[cfg(windows)]
    #[test]
    fn windows_context_file_opener_rejects_symlinked_identity_directory() {
        let _serial = cache_serial();
        let ws = fresh_workspace("windows_identity_dir_symlink");
        let outside = fresh_workspace("windows_identity_dir_target");
        fs::write(outside.join(CONTEXT_FILENAME), "must not load").unwrap();
        if let Err(error) = std::os::windows::fs::symlink_dir(&outside, ws.join(".identity")) {
            if error.kind() == io::ErrorKind::PermissionDenied {
                let _ = fs::remove_dir_all(&ws);
                let _ = fs::remove_dir_all(&outside);
                return;
            }
            panic!("failed to create directory symlink fixture: {error}");
        }

        assert!(
            open_context_file(&ws.join(".identity").join(CONTEXT_FILENAME)).is_err(),
            "the opened file handle must be rejected when an intermediate reparse point escapes the workspace"
        );
        let _ = fs::remove_dir_all(&ws);
        let _ = fs::remove_dir_all(&outside);
    }

    #[cfg(unix)]
    #[test]
    fn identity_symlink_falls_back_to_regular_root_context() {
        let _serial = cache_serial();
        let ws = fresh_workspace("identity_symlink_fallback");
        let identity_dir = ws.join(".identity");
        fs::create_dir_all(&identity_dir).unwrap();
        let target = ws.join("untrusted-target.md");
        fs::write(&target, "must not load").unwrap();
        std::os::unix::fs::symlink(&target, identity_dir.join(CONTEXT_FILENAME)).unwrap();
        fs::write(ws.join(CONTEXT_FILENAME), "trusted root context").unwrap();

        assert_eq!(
            load_context_md(&ws, false).as_deref(),
            Some("trusted root context")
        );
        let _ = fs::remove_dir_all(&ws);
    }

    #[cfg(unix)]
    #[test]
    fn identity_directory_symlink_falls_back_to_regular_root_context() {
        let _serial = cache_serial();
        let ws = fresh_workspace("identity_dir_symlink_fallback");
        let target_dir = ws.join("untrusted-identity");
        fs::create_dir_all(&target_dir).unwrap();
        fs::write(target_dir.join(CONTEXT_FILENAME), "must not load").unwrap();
        std::os::unix::fs::symlink(&target_dir, ws.join(".identity")).unwrap();
        fs::write(ws.join(CONTEXT_FILENAME), "trusted root context").unwrap();

        assert_eq!(
            load_context_md(&ws, false).as_deref(),
            Some("trusted root context")
        );
        let _ = fs::remove_dir_all(&ws);
    }

    #[cfg(unix)]
    #[test]
    fn symlink_replacement_falls_back_to_cached_context() {
        let _serial = cache_serial();
        let ws = fresh_workspace("symlink_cache_fallback");
        let context = ws.join(CONTEXT_FILENAME);
        let target = ws.join("untrusted-target.md");
        fs::write(&context, "cached good context").unwrap();
        assert_eq!(
            load_context_md(&ws, false).as_deref(),
            Some("cached good context")
        );
        fs::write(&target, "must not load").unwrap();
        fs::remove_file(&context).unwrap();
        std::os::unix::fs::symlink(&target, &context).unwrap();

        assert_eq!(
            load_context_md(&ws, false).as_deref(),
            Some("cached good context")
        );
        let _ = fs::remove_dir_all(&ws);
    }

    #[cfg(unix)]
    #[test]
    fn identity_symlink_replacement_falls_back_to_cached_context() {
        let _serial = cache_serial();
        let ws = fresh_workspace("identity_symlink_cache_fallback");
        let identity_dir = ws.join(".identity");
        let context = identity_dir.join(CONTEXT_FILENAME);
        let target = ws.join("untrusted-target.md");
        fs::create_dir_all(&identity_dir).unwrap();
        fs::write(&context, "cached good identity context").unwrap();
        assert_eq!(
            load_context_md(&ws, false).as_deref(),
            Some("cached good identity context")
        );
        fs::write(&target, "must not load").unwrap();
        fs::remove_file(&context).unwrap();
        std::os::unix::fs::symlink(&target, &context).unwrap();

        assert_eq!(
            load_context_md(&ws, false).as_deref(),
            Some("cached good identity context")
        );
        let _ = fs::remove_dir_all(&ws);
    }

    /// Async variant must yield identical content for the standard read
    /// path. This pins the byte-for-byte equivalence with the sync API
    /// — if a future refactor diverges, this test will catch it.
    // The guard is held across the awaits below on purpose: serializing these tests against the sync ones is the whole point of it, and `#[tokio::test]` drives them on a current-thread runtime, so there is no other task to block.
    #[allow(clippy::await_holding_lock)]
    #[tokio::test]
    async fn async_variant_matches_sync_for_basic_read() {
        let _serial = cache_serial();
        let ws = fresh_workspace("async_basic");
        fs::write(ws.join(CONTEXT_FILENAME), "async-ok payload").unwrap();

        let loaded = load_context_md_async(&ws, false).await.unwrap();
        assert!(loaded.contains("async-ok payload"));

        let _ = fs::remove_dir_all(&ws);
    }

    /// Async API must honour the symlink rejection identically to sync.
    #[cfg(unix)]
    // The guard is held across the awaits below on purpose: serializing these tests against the sync ones is the whole point of it, and `#[tokio::test]` drives them on a current-thread runtime, so there is no other task to block.
    #[allow(clippy::await_holding_lock)]
    #[tokio::test]
    async fn async_variant_rejects_symlink_context_file() {
        let _serial = cache_serial();
        let ws = fresh_workspace("async_symlink");
        let real = ws.join("real.md");
        fs::write(&real, "would-be-leaked content").unwrap();
        std::os::unix::fs::symlink(&real, ws.join(CONTEXT_FILENAME)).unwrap();

        let loaded = load_context_md_async(&ws, false).await;
        assert!(
            loaded.is_none(),
            "async symlinked context.md must be refused, got {loaded:?}"
        );

        let _ = fs::remove_dir_all(&ws);
    }

    /// Async API picks up `.identity/context.md` over the legacy root
    /// fallback — same precedence rule as the sync version.
    // The guard is held across the awaits below on purpose: serializing these tests against the sync ones is the whole point of it, and `#[tokio::test]` drives them on a current-thread runtime, so there is no other task to block.
    #[allow(clippy::await_holding_lock)]
    #[tokio::test]
    async fn async_variant_identity_dir_takes_precedence() {
        let _serial = cache_serial();
        let ws = fresh_workspace("async_identity");
        let identity_dir = ws.join(".identity");
        fs::create_dir_all(&identity_dir).unwrap();
        fs::write(ws.join(CONTEXT_FILENAME), "root payload").unwrap();
        fs::write(identity_dir.join(CONTEXT_FILENAME), "identity payload").unwrap();

        let loaded = load_context_md_async(&ws, false).await.unwrap();
        assert!(loaded.contains("identity payload"));
        assert!(!loaded.contains("root payload"));

        let _ = fs::remove_dir_all(&ws);
    }
}
