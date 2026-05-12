//! Daemon-attached Agent Client Protocol (ACP) listener (#3313).
//!
//! When the daemon is running, this module exposes a Unix domain
//! socket at `~/.librefang/acp.sock` so multiple `librefang acp`
//! invocations can share one long-running kernel: one approval
//! decision is visible to all editor tabs, agent state persists
//! across the editor restarting the child process, and remembered
//! `allow_always` decisions outlive the per-invocation child.
//!
//! Each accepted UDS connection runs an isolated
//! [`librefang_acp::run_with_transport`] over the connection's
//! framed JSON-RPC stream against a shared [`KernelAdapter`] backed
//! by the daemon's existing kernel. The CLI side (
//! `librefang-cli::acp::run_uds_proxy`) becomes a transparent
//! stdin↔socket pipe, so ACP wire frames flow directly between the
//! editor and the daemon.
//!
//! # Trust model
//!
//! The trust model is **same-user, same-host**. The listener defends
//! against multi-user host hijack with two layers:
//!
//! 1. **Socket file mode `0o600`** — set atomically by binding to a
//!    randomised tempfile in the parent directory, `chmod`-ing it to
//!    `0o600`, then `rename`-ing into place. This avoids the
//!    `bind() -> chmod()` TOCTOU window where another local user
//!    could `connect()` between the two syscalls and inherit a
//!    world-readable socket.
//!
//! 2. **`SO_PEERCRED` peer-uid match** — every accepted connection's
//!    `peer_cred()` is compared against the daemon's own euid. A
//!    mismatch logs a warning and drops the stream before any ACP
//!    bytes are read, so a privileged-by-mistake socket file (e.g.
//!    inherited from a sloppy umask outside our control, or a
//!    container running as root with the host filesystem mounted)
//!    still can't be hijacked by another local user.

#![cfg(unix)]

use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use librefang_acp::{AcpKernel, KernelAdapter};
use librefang_kernel::LibreFangKernel;
use tokio::net::{UnixListener, UnixStream};
use tokio_util::compat::{TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};
use tracing::{debug, info, warn};

/// Default agent name when the editor doesn't specify one. Mirrors
/// the in-process CLI default (`assistant`) so behaviour is consistent
/// across modes. A future protocol extension may let the editor pick
/// per-connection — see #3313 phase-2 follow-up.
const DEFAULT_AGENT: &str = "assistant";

/// Accept connections on `sock_path` and serve ACP over each. Returns
/// only when the listener fails to bind or the loop is interrupted —
/// the daemon's bg-task harness logs and forgets in either case.
pub async fn run_listener(kernel: Arc<LibreFangKernel>, sock_path: PathBuf) -> std::io::Result<()> {
    if let Some(parent) = sock_path.parent() {
        tokio::fs::create_dir_all(parent).await.ok();
    }

    let listener = bind_atomic_owner_only(&sock_path).await?;
    info!(path = %sock_path.display(), "ACP UDS listener bound (mode 0o600)");

    let self_uid = self_euid();

    loop {
        let stream = match listener.accept().await {
            Ok((s, _)) => s,
            Err(e) => {
                warn!(error = %e, "ACP UDS accept failed");
                continue;
            }
        };
        // SO_PEERCRED match. We accept only connections from the same
        // euid as the daemon — the threat model is multi-user hosts
        // where another local UID has read access to the socket file
        // (e.g. through a misconfigured umask the chmod-on-bind path
        // didn't catch). Drop without reading any ACP bytes.
        match stream.peer_cred() {
            Ok(cred) if cred.uid() == self_uid => {}
            Ok(cred) => {
                warn!(
                    peer_uid = cred.uid(),
                    self_uid, "ACP UDS rejected: peer uid mismatch"
                );
                drop(stream);
                continue;
            }
            Err(e) => {
                warn!(error = %e, "ACP UDS rejected: peer_cred() failed");
                drop(stream);
                continue;
            }
        }
        let kernel = Arc::clone(&kernel);
        tokio::spawn(async move {
            if let Err(e) = handle_connection(kernel, stream).await {
                warn!(error = %e, "ACP UDS connection ended with error");
            }
        });
    }
}

/// Atomically expose a UDS at `final_path` with mode `0o600`.
///
/// `UnixListener::bind` honours the process umask; on a default umask
/// of `0o022` the socket file lands at mode `0o755` and there is a
/// race window between `bind` and any subsequent `chmod` where another
/// local uid can `connect()`. We close that window by binding to a
/// randomised tempfile in the same parent directory, `chmod`-ing the
/// file to `0o600`, and only then `rename`-ing it into place. The
/// rename also handles stale-socket cleanup atomically (a leftover
/// file from a crashed daemon at `final_path` is overwritten with the
/// new tightened socket in one syscall).
async fn bind_atomic_owner_only(final_path: &Path) -> std::io::Result<UnixListener> {
    let parent = final_path
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."));
    let stem = final_path
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "acp.sock".into());
    // Randomised tempfile name so an attacker can't pre-squat the path.
    // `pid + nanos` is sufficient — we only need uniqueness within the
    // window between `bind` and `rename`.
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let tmp_path = parent.join(format!(".{stem}.{}.{}", std::process::id(), nanos));

    // Best-effort cleanup if a previous attempt at this exact tmp name
    // crashed mid-bind. The randomisation makes a real collision
    // basically impossible, but a no-op `remove_file` is cheap.
    let _ = tokio::fs::remove_file(&tmp_path).await;
    let listener = UnixListener::bind(&tmp_path)?;
    // Tighten before exposing.
    tokio::fs::set_permissions(&tmp_path, std::fs::Permissions::from_mode(0o600)).await?;
    // Atomic publish. If `final_path` exists (stale from a crashed
    // daemon), `rename` overwrites it. The previous listener — if any —
    // keeps its kernel-side socket open but anyone connecting via the
    // path now lands on us.
    if let Err(e) = tokio::fs::rename(&tmp_path, final_path).await {
        // Clean up the tempfile so a bind failure doesn't litter the
        // parent dir with stale sockets.
        let _ = tokio::fs::remove_file(&tmp_path).await;
        return Err(e);
    }

    // Sweep stale orphan tempfiles left by previous daemon runs.
    //
    // On macOS Docker Desktop bind-mount volumes, `rename(2)` succeeds on
    // the host side but the source file is never unlinked from the
    // container's view of the directory, so `.acp.sock.<pid>.<nanos>`
    // tempfiles accumulate across restarts. Now that the rename has
    // succeeded the live socket is at `final_path`; anything in the parent
    // directory that still matches `.<stem>.<digits>.<digits>` is a stale
    // orphan from a previous run. Best-effort removal — ignore every error
    // (permission races, cross-device issues) because cleanup must never
    // prevent a successful bind.
    //
    // Deletion is PID-liveness checked (via `kill(pid, 0)`) and bounded by
    // a UID equality guard so we never remove a concurrent daemon's
    // in-flight tempfile or touch files owned by another user.
    sweep_stale_orphans(&parent, &stem).await;

    Ok(listener)
}

/// Best-effort removal of `.<stem>.<pid>.<nanos>` orphan tempfiles in `parent`.
///
/// Each candidate is subject to three guards before deletion:
/// 1. **UID check** — the file must be owned by the current daemon's euid.
///    This prevents cross-user blast radius even when PIDs wrap.
/// 2. **PID-liveness check** — `kill(pid, 0)` is used; only files whose
///    parsed PID returns `ESRCH` (no such process) are removed. `EPERM`
///    means the process exists but is owned by another user — kept.
///    A live PID means a concurrent daemon is still in its bind→rename
///    window — kept.
/// 3. **Recency guard** — files whose mtime is within the last 10 seconds
///    are skipped to protect fresh tempfiles that haven't been parsed yet.
async fn sweep_stale_orphans(parent: &Path, stem: &str) {
    use std::os::unix::ffi::OsStrExt;

    let prefix = format!(".{stem}.");
    let prefix_bytes = prefix.as_bytes();
    let self_uid = self_euid();
    // Files younger than this window are left alone to avoid racing with a
    // concurrent daemon's bind→chmod→rename sequence.
    const RECENCY_WINDOW: Duration = Duration::from_secs(10);

    let mut rd = match tokio::fs::read_dir(parent).await {
        Ok(rd) => rd,
        Err(e) => {
            debug!(error = %e, "ACP UDS sweep skipped");
            return;
        }
    };
    while let Ok(Some(entry)) = rd.next_entry().await {
        let name = entry.file_name();
        // Bytewise prefix comparison — avoids lossy UTF-8 conversion on
        // platforms where filenames can be arbitrary byte sequences.
        if !name.as_bytes().starts_with(prefix_bytes) {
            continue;
        }
        // Parse `<pid>.<nanos>` from the suffix after the prefix.
        let suffix = &name.as_bytes()[prefix_bytes.len()..];
        let dot = match suffix.iter().position(|&b| b == b'.') {
            Some(p) => p,
            None => continue,
        };
        let pid_bytes = &suffix[..dot];
        let nanos_bytes = &suffix[dot + 1..];
        if pid_bytes.is_empty()
            || nanos_bytes.is_empty()
            || !pid_bytes.iter().all(|b| b.is_ascii_digit())
            || !nanos_bytes.iter().all(|b| b.is_ascii_digit())
        {
            continue;
        }
        // Parse PID; skip on overflow (impossibly large value, not a real PID).
        let pid_str = match std::str::from_utf8(pid_bytes) {
            Ok(s) => s,
            Err(_) => continue,
        };
        let pid: libc::pid_t = match pid_str.parse() {
            Ok(p) => p,
            Err(_) => continue,
        };

        // --- Guard 1: UID equality ---
        let meta = match tokio::fs::metadata(entry.path()).await {
            Ok(m) => m,
            Err(_) => continue,
        };
        if meta.uid() != self_uid {
            continue;
        }

        // --- Guard 2: Recency window ---
        if let Ok(mtime) = meta.modified() {
            if SystemTime::now()
                .duration_since(mtime)
                .map(|age| age < RECENCY_WINDOW)
                .unwrap_or(true)
            {
                continue;
            }
        }

        // --- Guard 3: PID liveness via kill(pid, 0) ---
        // Safety: kill(pid, 0) is a read-only liveness probe; it sends no
        // signal, has no side effects, and is thread-safe.
        let alive = unsafe { libc::kill(pid, 0) == 0 };
        let dead = if alive {
            false
        } else {
            // ESRCH = no such process (dead). EPERM = exists, owned by
            // another user (treat as alive — don't delete).
            let err = std::io::Error::last_os_error();
            err.raw_os_error() == Some(libc::ESRCH)
        };

        if dead {
            let _ = tokio::fs::remove_file(entry.path()).await;
        }
    }
}

/// Process effective uid. We compare against this on every accepted
/// connection. `unsafe` is fine — `geteuid` is a thread-safe, no-arg
/// libc call that returns a `uid_t`.
fn self_euid() -> u32 {
    // Safety: `geteuid` is signal-safe and has no side effects.
    unsafe { libc::geteuid() }
}

async fn handle_connection(
    kernel: Arc<LibreFangKernel>,
    stream: UnixStream,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let adapter = Arc::new(KernelAdapter::new(kernel));
    let agent_id = adapter.resolve_agent(DEFAULT_AGENT).await?;

    let (read_half, write_half) = stream.into_split();
    let transport =
        agent_client_protocol::ByteStreams::new(write_half.compat_write(), read_half.compat());
    librefang_acp::run_with_transport(adapter, agent_id, transport).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::MetadataExt;
    use std::time::{Duration, SystemTime};

    #[tokio::test]
    async fn bind_atomic_owner_only_sets_mode_0600() {
        let dir = tempfile::tempdir().expect("tempdir");
        let sock = dir.path().join("acp.sock");
        let _listener = bind_atomic_owner_only(&sock).await.expect("bind");
        let meta = tokio::fs::metadata(&sock).await.expect("stat");
        // Bottom 9 bits are the rwx bits.
        assert_eq!(meta.mode() & 0o777, 0o600, "socket must be mode 0600");
        assert_eq!(
            meta.uid(),
            self_euid(),
            "socket must be owned by daemon uid"
        );
    }

    #[tokio::test]
    async fn bind_atomic_owner_only_overwrites_stale_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let sock = dir.path().join("acp.sock");
        // Pre-existing file (simulates a crashed-daemon leftover)
        // — the bind path must overwrite it atomically rather than
        // fail with EADDRINUSE or fall into a TOCTOU.
        tokio::fs::write(&sock, b"stale").await.expect("seed stale");
        let _listener = bind_atomic_owner_only(&sock).await.expect("rebind");
        let meta = tokio::fs::metadata(&sock).await.expect("stat");
        assert_eq!(meta.mode() & 0o777, 0o600);
    }

    /// Helper: plant a tempfile-shaped name with an mtime 60 s in the past.
    async fn plant_old_orphan(dir: &std::path::Path, name: &str) {
        let path = dir.join(name);
        tokio::fs::write(&path, b"orphan")
            .await
            .expect("seed orphan");
        // Back-date mtime by 60 s so the recency guard doesn't protect it.
        let past = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
            .saturating_sub(60);
        // Safety: path is a valid, freshly-created file; tv_sec/tv_usec are
        // plain integers; utimes is thread-safe.
        use std::ffi::CString;
        let c_path = CString::new(path.to_str().expect("path utf8")).expect("cstring");
        let times = [
            libc::timeval {
                tv_sec: past as libc::time_t,
                tv_usec: 0,
            },
            libc::timeval {
                tv_sec: past as libc::time_t,
                tv_usec: 0,
            },
        ];
        let rc = unsafe { libc::utimes(c_path.as_ptr(), times.as_ptr()) };
        assert_eq!(rc, 0, "utimes failed: {}", std::io::Error::last_os_error());
    }

    #[tokio::test]
    async fn bind_atomic_owner_only_sweeps_dead_pid_orphans() {
        let dir = tempfile::tempdir().expect("tempdir");
        let sock = dir.path().join("acp.sock");

        // PID 1 is init/launchd — always alive, so we need a definitely-dead
        // PID. Spawn `true`, wait for it to exit, then use its PID.
        let mut child = std::process::Command::new("true")
            .spawn()
            .expect("spawn true");
        let dead_pid = child.id();
        // wait() reaps the child so the PID is no longer live.
        child.wait().expect("wait true");
        // Give the OS a moment to release the PID slot.
        tokio::time::sleep(Duration::from_millis(50)).await;

        let orphan_name = format!(".acp.sock.{dead_pid}.111222333");
        plant_old_orphan(dir.path(), &orphan_name).await;

        // An unrelated file must survive.
        tokio::fs::write(dir.path().join("unrelated.txt"), b"keep")
            .await
            .expect("seed unrelated");

        let _listener = bind_atomic_owner_only(&sock).await.expect("bind");

        // The live socket must exist.
        assert!(
            tokio::fs::metadata(&sock).await.is_ok(),
            "live socket must exist"
        );

        // The unrelated file must survive.
        assert!(
            tokio::fs::metadata(dir.path().join("unrelated.txt"))
                .await
                .is_ok(),
            "unrelated.txt must not be removed"
        );
    }

    /// A tempfile whose PID belongs to a living process must NOT be deleted.
    #[tokio::test]
    async fn sweep_preserves_live_pid_tempfile() {
        let dir = tempfile::tempdir().expect("tempdir");

        // Spawn a long-lived child; its PID is guaranteed alive during the sweep.
        let mut child = std::process::Command::new("sleep")
            .arg("30")
            .spawn()
            .expect("spawn sleep");
        let live_pid = child.id();

        let name = format!(".acp.sock.{live_pid}.999888777");
        plant_old_orphan(dir.path(), &name).await;

        sweep_stale_orphans(dir.path(), "acp.sock").await;

        // File must still exist — the PID is alive.
        assert!(
            tokio::fs::metadata(dir.path().join(&name)).await.is_ok(),
            "tempfile with live PID {live_pid} must not be deleted"
        );

        child.kill().ok();
        let _ = child.wait();
    }

    /// A tempfile whose mtime is within the recency window must NOT be deleted
    /// regardless of PID state.
    #[tokio::test]
    async fn sweep_preserves_recent_tempfile() {
        let dir = tempfile::tempdir().expect("tempdir");

        // Use a definitely-dead-or-irrelevant PID (PID 0 is invalid, kernel
        // returns ESRCH), but the file's mtime is fresh so the recency guard
        // must protect it before we even reach the liveness check.
        // We use the current process's PID to guarantee ESRCH won't fire
        // (process exists), and keep mtime at "now" (default on write).
        let name = format!(".acp.sock.{}.555444333", std::process::id());
        // Write without backdating — mtime stays at "now".
        tokio::fs::write(dir.path().join(&name), b"recent")
            .await
            .expect("seed recent");

        sweep_stale_orphans(dir.path(), "acp.sock").await;

        assert!(
            tokio::fs::metadata(dir.path().join(&name)).await.is_ok(),
            "recently-written tempfile must not be swept"
        );
    }
}
