//! Cron pre-check script execution + atomic agent.toml persistence.
//!
//! `cron_script_wake_gate` runs an author-supplied script in a hardened
//! sandbox (env stripped, cwd pinned, stdout capped, 30 s timeout) and
//! parses its final stdout line as a JSON wake gate. `atomic_write_toml`
//! is the safe-write primitive used wherever the daemon updates an
//! agent.toml from multiple call sites concurrently.

use std::path::Path;

/// Run a cron job's pre-check script and parse the wake gate from its output.
///
/// Returns `true` if the agent should be woken (normal path), `false` to skip.
///
/// Rules (mirrors Hermes `_parse_wake_gate`):
/// - Script must exit 0; on any error we default to waking the agent.
/// - Find the last non-empty stdout line and try to parse it as JSON.
/// - If the parsed object has `"wakeAgent": false` (strict bool), return false.
/// - Everything else (non-JSON, missing key, null, 0, "") → return true.
///
/// # Security hardening
///
/// `pre_check_script` used to inherit the full daemon environment, allowing
/// it to read API keys and other secrets from env vars.  It also had no
/// working-directory restriction and no stdout size limit.
///
/// This implementation now:
/// * Clears the inherited environment with `env_clear()` so daemon secrets
///   are not leaked to the child process.
/// * Passes only `PATH` and `HOME` so the script can still locate standard
///   binaries without receiving application-layer credentials.
/// * Sets `current_dir` to the agent workspace when one is available,
///   otherwise falls back to a system temp directory.
/// * Caps stdout (and stderr) at 64 KiB to prevent a misbehaving script
///   from filling daemon memory.
pub(super) async fn cron_script_wake_gate(
    job_name: &str,
    script_path: &str,
    agent_workspace: Option<&std::path::Path>,
) -> bool {
    use std::process::Stdio;
    use tokio::io::AsyncReadExt;
    use tokio::process::Command;

    /// Maximum bytes we read from stdout before truncating.
    const MAX_OUTPUT: usize = 64 * 1024; // 64 KiB

    // Resolve a safe working directory for the child process.
    // Preference order: agent workspace → system temp → current dir.
    let cwd = agent_workspace
        .map(|p| p.to_path_buf())
        .unwrap_or_else(std::env::temp_dir);

    // Build the command with a stripped-down environment.
    // `env_clear` prevents all inherited daemon env vars (API keys, secrets,
    // socket paths, etc.) from reaching the child.  We selectively restore
    // the two vars that most scripts need to function correctly.
    let mut cmd = Command::new(script_path);
    cmd.env_clear();
    if let Ok(path_val) = std::env::var("PATH") {
        cmd.env("PATH", path_val);
    }
    if let Ok(home_val) = std::env::var("HOME") {
        cmd.env("HOME", home_val);
    }
    cmd.current_dir(&cwd);
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());
    cmd.kill_on_drop(true);

    // Hard cap: pre-check scripts must complete within 30 s.
    // A hung script would otherwise block the cron dispatcher indefinitely.
    let run = async {
        let child = cmd.spawn();
        match child {
            Err(e) => Err(e),
            Ok(mut child) => {
                // Cap stdout at MAX_OUTPUT bytes.
                let mut stdout_buf = Vec::with_capacity(MAX_OUTPUT.min(4096));
                if let Some(stdout) = child.stdout.take() {
                    let _ = stdout
                        .take(MAX_OUTPUT as u64)
                        .read_to_end(&mut stdout_buf)
                        .await;
                }
                // Drain stderr (up to the same cap) to avoid blocking the child.
                if let Some(stderr) = child.stderr.take() {
                    let mut _discard = Vec::new();
                    let _ = stderr
                        .take(MAX_OUTPUT as u64)
                        .read_to_end(&mut _discard)
                        .await;
                }
                let status = child.wait().await?;
                Ok((status, stdout_buf))
            }
        }
    };

    let (status, raw_stdout) =
        match tokio::time::timeout(std::time::Duration::from_secs(30), run).await {
            Err(_elapsed) => {
                tracing::warn!(
                    job = %job_name,
                    script = %script_path,
                    "cron: pre-check script timed out after 30s, waking agent"
                );
                return true;
            }
            Ok(Err(e)) => {
                tracing::warn!(
                    job = %job_name,
                    script = %script_path,
                    error = %e,
                    "cron: pre-check script failed to launch, waking agent"
                );
                return true;
            }
            Ok(Ok(pair)) => pair,
        };

    if !status.success() {
        tracing::warn!(
            job = %job_name,
            script = %script_path,
            code = ?status.code(),
            "cron: pre-check script exited non-zero, waking agent"
        );
        return true;
    }

    let stdout = String::from_utf8_lossy(&raw_stdout);
    parse_wake_gate(&stdout)
}

/// Atomically write a TOML file by staging the new content in a sibling
/// `.tmp` file and renaming it over the destination.
///
/// SECURITY / CORRECTNESS: a plain `fs::write` is non-atomic. Two
/// concurrent persisters (e.g. `patch_agent` + `set_agent_model`) can
/// truncate each other's output mid-flight, and a process crash at the
/// wrong moment leaves a half-written file that fails to parse on next
/// boot. `rename` is atomic on POSIX filesystems and effectively atomic
/// on Windows for files on the same volume; if the rename fails we
/// clean up the staging file.
///
/// We also `sync_all` the temp file before rename so the bytes hit the
/// disk before the directory entry is swapped — without that, a power
/// loss could leave the renamed file pointing at empty/stale data even
/// though the rename succeeded.
pub(super) fn atomic_write_toml(path: &Path, content: &str) -> std::io::Result<()> {
    use std::io::Write;
    use std::sync::atomic::{AtomicU64, Ordering};
    // Per-call counter so two threads in the same process never share
    // a tmp filename — otherwise concurrent writers can clobber each
    // other's staging file before rename, defeating the atomicity.
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);

    // Same-directory tmp path keeps rename on the same filesystem so
    // it's a true atomic in-place swap rather than a cross-volume copy.
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
        f.write_all(content.as_bytes())?;
        // fsync so the bytes hit disk before we publish via rename;
        // without this a power loss between rename and flush would
        // leave the renamed file pointing at empty/garbage data.
        f.sync_all()?;
        Ok(())
    })();

    if let Err(e) = write_result {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }

    // POSIX `rename` is atomic. Windows `MoveFileEx` with
    // REPLACE_EXISTING (which Rust's std uses) is effectively atomic
    // for files on the same volume, though there is a brief window
    // where readers may see ERROR_SHARING_VIOLATION on contention.
    if let Err(e) = std::fs::rename(&tmp, path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    Ok(())
}

/// Parse the wake gate from script stdout.
///
/// Finds the last non-empty line, tries JSON-decode, checks `wakeAgent`.
/// Returns `true` (wake) unless `wakeAgent` is strictly `false`.
fn parse_wake_gate(script_output: &str) -> bool {
    let last_line = script_output.lines().rfind(|l| !l.trim().is_empty());

    let last_line = match last_line {
        Some(l) => l.trim(),
        None => return true,
    };

    let value: serde_json::Value = match serde_json::from_str(last_line) {
        Ok(v) => v,
        Err(_) => return true,
    };

    // Only `{"wakeAgent": false}` (strict bool false) skips the agent.
    value.get("wakeAgent") != Some(&serde_json::Value::Bool(false))
}
