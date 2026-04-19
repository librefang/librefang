//! Consolidation lock for auto-dream.
//!
//! The lock file serves a dual purpose:
//!   * Its **mtime** is the source of truth for `last_consolidated_at`. We
//!     never store the timestamp anywhere else — `stat` is the one read per
//!     tick, which is cheap.
//!   * Its **body** is the PID of the process currently holding the lock.
//!     If that PID is still alive we must not fire a second consolidation.
//!
//! The mtime trick (lifted from libre-code's `consolidationLock.ts`) means a
//! failed fork can be rolled back by rewinding the mtime to its pre-acquire
//! value — the next time-gate check will pass again and we'll retry.
//!
//! A stuck-but-crashed process is handled via a stale-window: after
//! `HOLDER_STALE_MS` we reclaim the lock regardless of PID liveness, as a
//! guard against PID reuse.

use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use std::time::{SystemTime, UNIX_EPOCH};

use dashmap::DashSet;
use librefang_types::error::{LibreFangError, LibreFangResult};
use tokio::fs;

/// A PID that has held the lock longer than this is presumed dead regardless
/// of whether the OS still shows it running (guards against PID reuse).
const HOLDER_STALE_MS: u64 = 60 * 60 * 1000;

/// Process-local claims close the same-daemon race that the on-disk token
/// alone cannot eliminate: one caller can write+verify before another caller
/// overwrites the file, allowing both to "win" in sequence. We still need
/// the filesystem lock for cross-process coherence; this only serializes
/// concurrent acquirers targeting the same path inside one process.
static IN_PROCESS_CLAIMS: LazyLock<DashSet<PathBuf>> = LazyLock::new(DashSet::new);

/// Handle onto the lock file. Stateless — every operation re-reads from disk
/// so multiple processes racing on the same path stay coherent.
#[derive(Debug, Clone)]
pub struct ConsolidationLock {
    path: PathBuf,
}

impl ConsolidationLock {
    /// Create a handle. Does not touch the filesystem.
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    /// Path of the backing lock file (for logging/diagnostics only).
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Read the mtime of the lock file, interpreted as "last consolidated at"
    /// in milliseconds since the Unix epoch. Returns `0` if the file does not
    /// exist (no consolidation has ever run on this host).
    pub async fn read_last_consolidated_at(&self) -> LibreFangResult<u64> {
        match fs::metadata(&self.path).await {
            Ok(meta) => Ok(mtime_ms(&meta)?),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(0),
            Err(e) => Err(LibreFangError::Internal(format!(
                "auto_dream: stat lock file failed: {e}"
            ))),
        }
    }

    /// Try to acquire the lock. On success, returns the pre-acquire mtime so
    /// the caller can rewind it via [`rollback`] after a failure.
    ///
    /// Returns `Ok(None)` if the lock is held by a live process (not an
    /// error — a competing process already owns the turn). Returns `Err` for
    /// genuine filesystem problems.
    ///
    /// Body format is `"<pid>:<uuid>"`. The PID drives the liveness check
    /// (`kill(0)`); the UUID is a per-acquire unique token so the
    /// last-writer-wins race check can distinguish two concurrent
    /// acquirers in the *same* process (e.g. two rapid manual triggers).
    /// Writing only the PID would leave same-daemon races unprotected —
    /// both writers write the same string and both "win" verification.
    pub async fn try_acquire(&self) -> LibreFangResult<Option<u64>> {
        // Probe existing holder, if any.
        let (prior_mtime, holder_pid) = match fs::metadata(&self.path).await {
            Ok(meta) => {
                let mtime = mtime_ms(&meta)?;
                let body = fs::read_to_string(&self.path).await.unwrap_or_default();
                let pid = parse_pid_from_body(&body);
                (Some(mtime), pid)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (None, None),
            Err(e) => {
                return Err(LibreFangError::Internal(format!(
                    "auto_dream: stat lock file failed: {e}"
                )));
            }
        };

        if let Some(mtime) = prior_mtime {
            let now = now_ms();
            let within_stale_window = now.saturating_sub(mtime) < HOLDER_STALE_MS;
            let live_holder = holder_pid.is_some_and(is_process_running);
            if within_stale_window && live_holder {
                if let Some(pid) = holder_pid {
                    tracing::debug!(
                        holder_pid = pid,
                        age_s = (now - mtime) / 1000,
                        "auto_dream: lock held by live PID"
                    );
                }
                return Ok(None);
            }
            // Stale: either PID dead or the holder exceeded the stale window.
            // Fall through and reclaim.
        }

        // Ensure parent dir exists (data_dir may be fresh).
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).await.map_err(|e| {
                LibreFangError::Internal(format!("auto_dream: create lock parent dir failed: {e}"))
            })?;
        }

        // Same-process racers must lose before touching the file. The
        // token-based verify below only proves "my write was visible when I
        // re-read", not "nobody overwrote me later".
        if !IN_PROCESS_CLAIMS.insert(self.path.clone()) {
            return Ok(None);
        }

        // Per-acquire token — makes "did our write win?" a real check even
        // when two acquirers are in the same process and thus share a PID.
        let token = format!("{}:{}", std::process::id(), uuid::Uuid::new_v4());
        fs::write(&self.path, token.as_bytes()).await.map_err(|e| {
            IN_PROCESS_CLAIMS.remove(&self.path);
            LibreFangError::Internal(format!("auto_dream: write lock file failed: {e}"))
        })?;

        // Verify our write won the race. Last-writer-wins semantics mean
        // the loser sees a body that is not their unique token — they bail
        // and retry next tick. Without the UUID this check would falsely
        // succeed for same-PID racers because their bodies would be
        // identical.
        let verify = fs::read_to_string(&self.path).await.map_err(|e| {
            IN_PROCESS_CLAIMS.remove(&self.path);
            LibreFangError::Internal(format!("auto_dream: verify lock file failed: {e}"))
        })?;
        if verify.trim() != token {
            IN_PROCESS_CLAIMS.remove(&self.path);
            return Ok(None);
        }

        Ok(Some(prior_mtime.unwrap_or(0)))
    }

    /// Rewind the mtime to its pre-acquire value after a failed consolidation.
    ///
    /// Also clears the PID body — otherwise our still-running process would
    /// look like it's holding the lock on the next tick. When `prior_mtime`
    /// is `0` (no lock existed before), the file is unlinked to restore the
    /// "never consolidated" state.
    pub async fn rollback(&self, prior_mtime: u64) -> LibreFangResult<()> {
        IN_PROCESS_CLAIMS.remove(&self.path);
        if prior_mtime == 0 {
            match fs::remove_file(&self.path).await {
                Ok(()) => return Ok(()),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
                Err(e) => {
                    return Err(LibreFangError::Internal(format!(
                        "auto_dream: unlink lock on rollback failed: {e}"
                    )));
                }
            }
        }
        fs::write(&self.path, b"").await.map_err(|e| {
            LibreFangError::Internal(format!(
                "auto_dream: clear lock body on rollback failed: {e}"
            ))
        })?;
        set_mtime_ms(&self.path, prior_mtime).map_err(|e| {
            LibreFangError::Internal(format!("auto_dream: rewind lock mtime failed: {e}"))
        })?;
        Ok(())
    }

    /// Record a successful consolidation by touching the lock's mtime to now.
    /// This is the "happy path" counterpart to [`rollback`] — after the dream
    /// agent finishes, we leave the mtime where `try_acquire` set it, and
    /// this function is only needed for the manual-trigger path which
    /// doesn't go through try_acquire.
    pub async fn record_now(&self) -> LibreFangResult<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).await.map_err(|e| {
                LibreFangError::Internal(format!("auto_dream: create lock parent dir failed: {e}"))
            })?;
        }
        let token = format!("{}:{}", std::process::id(), uuid::Uuid::new_v4());
        fs::write(&self.path, token.as_bytes())
            .await
            .map_err(|e| LibreFangError::Internal(format!("auto_dream: touch lock failed: {e}")))?;
        Ok(())
    }

    /// Release the lock after a successful dream. Clears the PID body so the
    /// next `try_acquire` doesn't see a live holder, and refreshes mtime to
    /// now so `last_consolidated_at` reflects completion time (not acquire
    /// time). Without this, a completed dream keeps its own PID in the body
    /// and every subsequent acquire within `HOLDER_STALE_MS` sees a live
    /// holder, breaking `min_hours < 1` configs and blocking quick re-dreams.
    pub async fn release(&self) -> LibreFangResult<()> {
        IN_PROCESS_CLAIMS.remove(&self.path);
        fs::write(&self.path, b"").await.map_err(|e| {
            LibreFangError::Internal(format!("auto_dream: release lock failed: {e}"))
        })?;
        // Explicitly refresh mtime — coarse-resolution filesystems may leave
        // it unchanged if the previous write was within the same tick.
        set_mtime_ms(&self.path, now_ms()).map_err(|e| {
            LibreFangError::Internal(format!("auto_dream: refresh mtime on release failed: {e}"))
        })?;
        Ok(())
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn mtime_ms(meta: &std::fs::Metadata) -> LibreFangResult<u64> {
    let t = meta.modified().map_err(|e| {
        LibreFangError::Internal(format!("auto_dream: unsupported mtime on platform: {e}"))
    })?;
    let d = t
        .duration_since(UNIX_EPOCH)
        .map_err(|e| LibreFangError::Internal(format!("auto_dream: mtime before epoch: {e}")))?;
    Ok(d.as_millis() as u64)
}

fn set_mtime_ms(path: &Path, mtime_ms: u64) -> std::io::Result<()> {
    // filetime crate isn't a dep; use raw libc on Unix, best-effort on others.
    #[cfg(unix)]
    {
        use std::ffi::CString;
        use std::os::unix::ffi::OsStrExt;
        let c = CString::new(path.as_os_str().as_bytes())
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;
        let secs = (mtime_ms / 1000) as libc::time_t;
        let usecs = ((mtime_ms % 1000) * 1000) as libc::suseconds_t;
        let times = [
            libc::timeval {
                tv_sec: secs,
                tv_usec: usecs,
            },
            libc::timeval {
                tv_sec: secs,
                tv_usec: usecs,
            },
        ];
        // SAFETY: `c` is a valid NUL-terminated C string and `times` is a
        // valid 2-element timeval array per utimes(2)'s contract.
        let rc = unsafe { libc::utimes(c.as_ptr(), times.as_ptr()) };
        if rc != 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(())
    }
    #[cfg(not(unix))]
    {
        // On Windows we can't easily rewind mtime without an extra crate.
        // The fallback costs us: after a failed fork the time gate won't
        // re-open until min_hours, which is acceptable for MVP.
        let _ = (path, mtime_ms);
        Ok(())
    }
}

/// Extract the PID from a lock body. Current bodies look like `"<pid>:<uuid>"`;
/// older daemons wrote bare `"<pid>"`, so we fall back to parsing the entire
/// trimmed string. Returns `None` for empty / unparseable bodies (treated as
/// "no holder" upstream — e.g. after `release()` clears the body on success).
fn parse_pid_from_body(body: &str) -> Option<u32> {
    let trimmed = body.trim();
    if trimmed.is_empty() {
        return None;
    }
    let head = trimmed.split(':').next().unwrap_or(trimmed);
    head.parse::<u32>().ok()
}

fn is_process_running(pid: u32) -> bool {
    #[cfg(unix)]
    {
        // kill(pid, 0) — signal 0 asks "can I send a signal?" without sending
        // one. Returns 0 if the process exists and we have permission, -1
        // with ESRCH if it's dead, -1 with EPERM if it exists under another
        // user (still alive from our perspective).
        // SAFETY: `libc::kill` is a thin FFI wrapper; pid and signal 0 are
        // always valid arguments.
        let rc = unsafe { libc::kill(pid as libc::pid_t, 0) };
        if rc == 0 {
            return true;
        }
        // EPERM means the process exists but we don't have permission to
        // signal it — still alive from a liveness-probe perspective.
        std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
    }
    #[cfg(not(unix))]
    {
        // No cheap check without extra deps; treat as running and rely on
        // the HOLDER_STALE_MS window to reclaim after the grace period.
        let _ = pid;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn tmpfile(name: &str) -> PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!(
            "librefang-auto-dream-test-{}-{}-{}",
            name,
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        p
    }

    #[tokio::test]
    async fn read_last_consolidated_at_returns_zero_when_absent() {
        let lock = ConsolidationLock::new(tmpfile("absent"));
        assert_eq!(lock.read_last_consolidated_at().await.unwrap(), 0);
    }

    #[tokio::test]
    async fn acquire_fresh_returns_zero_prior() {
        let path = tmpfile("fresh");
        let lock = ConsolidationLock::new(path.clone());
        let prior = lock.try_acquire().await.unwrap();
        assert_eq!(prior, Some(0));
        let body = tokio::fs::read_to_string(&path).await.unwrap();
        // Body is "<pid>:<uuid>" so same-process racers don't both "win"
        // verification. Check the PID prefix rather than the full token.
        assert_eq!(
            parse_pid_from_body(&body),
            Some(std::process::id()),
            "body should start with our PID",
        );
        assert!(
            body.trim().contains(':'),
            "body should include uuid suffix, got {body:?}",
        );
        let _ = tokio::fs::remove_file(&path).await;
    }

    #[tokio::test]
    async fn parse_pid_from_body_handles_legacy_and_new_formats() {
        // Legacy bare-PID format — daemons built before the token change.
        assert_eq!(parse_pid_from_body("4242"), Some(4242));
        // Current "<pid>:<uuid>" format.
        assert_eq!(
            parse_pid_from_body("4242:123e4567-e89b-12d3-a456-426614174000"),
            Some(4242)
        );
        // Empty / whitespace / garbage bodies ⇒ no holder.
        assert_eq!(parse_pid_from_body(""), None);
        assert_eq!(parse_pid_from_body("   "), None);
        assert_eq!(parse_pid_from_body(":abc"), None);
    }

    #[tokio::test]
    async fn acquire_blocked_when_live_pid_holds() {
        let path = tmpfile("blocked");
        let lock = ConsolidationLock::new(path.clone());
        // Our own PID is definitely alive.
        lock.try_acquire().await.unwrap();
        let second = lock.try_acquire().await.unwrap();
        assert!(second.is_none(), "expected None when live holder owns lock");
        let _ = tokio::fs::remove_file(&path).await;
    }

    #[tokio::test]
    async fn rollback_to_zero_unlinks() {
        let path = tmpfile("rollback-zero");
        let lock = ConsolidationLock::new(path.clone());
        lock.try_acquire().await.unwrap();
        assert!(path.exists());
        lock.rollback(0).await.unwrap();
        assert!(!path.exists());
    }

    // Unix-only: `set_mtime_ms` is a no-op on Windows (documented fallback in
    // lock.rs: rewinding mtime without an extra crate isn't cheap on Windows,
    // and production relies on the HOLDER_STALE_MS window to recover after a
    // failed fork). The assertion below exercises Unix-specific semantics.
    #[cfg(unix)]
    #[tokio::test]
    async fn rollback_rewinds_mtime() {
        let path = tmpfile("rollback-rewind");
        let lock = ConsolidationLock::new(path.clone());
        lock.try_acquire().await.unwrap();
        let now = now_ms();
        // Rewind to 2h ago.
        let target = now - 2 * 60 * 60 * 1000;
        lock.rollback(target).await.unwrap();
        let after = lock.read_last_consolidated_at().await.unwrap();
        // Allow a 2s slop for fs resolution.
        assert!(
            after.abs_diff(target) < 2000,
            "expected mtime ~{target}, got {after}"
        );
        let _ = tokio::fs::remove_file(&path).await;
    }

    #[tokio::test]
    async fn release_clears_body_so_next_acquire_succeeds() {
        let path = tmpfile("release");
        let lock = ConsolidationLock::new(path.clone());
        // First dream: acquire, then release on success.
        lock.try_acquire().await.unwrap();
        let body = tokio::fs::read_to_string(&path).await.unwrap();
        assert_eq!(
            parse_pid_from_body(&body),
            Some(std::process::id()),
            "acquire should stamp our PID",
        );
        lock.release().await.unwrap();
        assert_eq!(
            tokio::fs::read_to_string(&path).await.unwrap().trim(),
            "",
            "release should clear the PID body"
        );
        // Second dream should be able to acquire immediately — the previous
        // PID is gone so `try_acquire` does not see a live holder.
        let prior = lock.try_acquire().await.unwrap();
        assert!(
            prior.is_some(),
            "expected Some after release (live-holder gate should not trip)",
        );
        let _ = tokio::fs::remove_file(&path).await;
    }

    // Unix-only: `is_process_running` always returns true on Windows
    // (documented fallback in lock.rs — no cheap liveness probe without an
    // extra crate, so production relies on HOLDER_STALE_MS to reclaim stale
    // holders after an hour). This test exercises the immediate-reclaim path
    // which is Unix-specific.
    #[cfg(unix)]
    #[tokio::test]
    async fn acquire_reclaims_stale_pid() {
        let path = tmpfile("stale-pid");
        // Write a definitely-dead PID (process 1 may be alive on unix; use a
        // clearly absurd value instead — on Linux, PIDs are capped at
        // /proc/sys/kernel/pid_max which is typically 4M).
        let fake_pid = u32::MAX - 1;
        tokio::fs::write(&path, fake_pid.to_string().as_bytes())
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(5)).await;
        let lock = ConsolidationLock::new(path.clone());
        let prior = lock.try_acquire().await.unwrap();
        assert!(prior.is_some(), "should reclaim from dead PID");
        let body = tokio::fs::read_to_string(&path).await.unwrap();
        assert_eq!(
            parse_pid_from_body(&body),
            Some(std::process::id()),
            "reclaim should stamp our PID",
        );
        let _ = tokio::fs::remove_file(&path).await;
    }

    #[tokio::test]
    async fn same_process_concurrent_acquire_single_winner() {
        // Two in-process acquirers share the same PID, so the legacy
        // bare-PID body would let both verification checks pass. The
        // UUID-tagged body should give exactly one winner.
        let path = tmpfile("same-process-race");
        let lock_a = ConsolidationLock::new(path.clone());
        let lock_b = ConsolidationLock::new(path.clone());
        let (ra, rb) = tokio::join!(lock_a.try_acquire(), lock_b.try_acquire());
        let winners = [ra.unwrap(), rb.unwrap()]
            .iter()
            .filter(|r| r.is_some())
            .count();
        assert_eq!(winners, 1, "exactly one racer should win the lock");
        let _ = tokio::fs::remove_file(&path).await;
    }
}
