//! `shell_exec` — run a single command inside the agent's workspace with
//! a sandboxed env, capture stdout/stderr/exit-code, honor session
//! interrupts, and enforce a deadline.
//!
//! Security gating (taint sinks, RO-workspace verb classification) lives
//! upstream in the dispatcher and `tool_runner::{taint, shell_safety}`;
//! by the time we reach this function the command has already been
//! cleared for execution.

use std::path::Path;

pub(super) async fn tool_shell_exec(
    input: &serde_json::Value,
    allowed_env: &[String],
    workspace_root: Option<&Path>,
    exec_policy: Option<&librefang_types::config::ExecPolicy>,
    interrupt: Option<crate::interrupt::SessionInterrupt>,
) -> Result<String, String> {
    let command = input["command"]
        .as_str()
        .ok_or("Missing 'command' parameter")?;
    // Use LLM-specified timeout, or fall back to exec policy timeout, or default 30s
    let policy_timeout = exec_policy.map(|p| p.timeout_secs).unwrap_or(30);
    let timeout_secs = input["timeout_seconds"].as_u64().unwrap_or(policy_timeout);

    // SECURITY: Determine execution strategy based on exec policy.
    //
    // In Allowlist mode (default): Use direct execution via shlex argv splitting.
    // This avoids invoking a shell interpreter, which eliminates an entire class
    // of injection attacks (encoding tricks, $IFS, glob expansion, etc.).
    //
    // In Full mode: User explicitly opted into unrestricted shell access,
    // so we use sh -c / cmd /C as before.
    let use_direct_exec = exec_policy
        .map(|p| p.mode == librefang_types::config::ExecSecurityMode::Allowlist)
        .unwrap_or(true); // Default to safe mode

    let mut cmd = if use_direct_exec {
        // SAFE PATH: Split command into argv using POSIX shell lexer rules,
        // then execute the binary directly — no shell interpreter involved.
        let argv = shlex::split(command).ok_or_else(|| {
            "Command contains unmatched quotes or invalid shell syntax".to_string()
        })?;
        if argv.is_empty() {
            return Err("Empty command after parsing".to_string());
        }
        let mut c = tokio::process::Command::new(&argv[0]);
        if argv.len() > 1 {
            c.args(&argv[1..]);
        }
        c
    } else {
        // UNSAFE PATH: Full mode — user explicitly opted in to shell interpretation.
        // Shell resolution: prefer sh (Git Bash/MSYS2) on Windows.
        #[cfg(windows)]
        let git_sh: Option<&str> = {
            const SH_PATHS: &[&str] = &[
                "C:\\Program Files\\Git\\usr\\bin\\sh.exe",
                "C:\\Program Files (x86)\\Git\\usr\\bin\\sh.exe",
            ];
            SH_PATHS
                .iter()
                .copied()
                .find(|p| std::path::Path::new(p).exists())
        };
        let (shell, shell_arg) = if cfg!(windows) {
            #[cfg(windows)]
            {
                if let Some(sh) = git_sh {
                    (sh, "-c")
                } else {
                    ("cmd", "/C")
                }
            }
            #[cfg(not(windows))]
            {
                ("sh", "-c")
            }
        } else {
            ("sh", "-c")
        };
        let mut c = tokio::process::Command::new(shell);
        c.arg(shell_arg).arg(command);
        c
    };

    // Set working directory to agent workspace so files are created there
    if let Some(ws) = workspace_root {
        cmd.current_dir(ws);
    }

    // SECURITY: Isolate environment to prevent credential leakage.
    // Hand settings may grant access to specific provider API keys.
    crate::subprocess_sandbox::sandbox_command(&mut cmd, allowed_env);

    // Ensure UTF-8 output on Windows
    #[cfg(windows)]
    cmd.env("PYTHONIOENCODING", "utf-8");

    // Prevent child from inheriting stdin (avoids blocking on Windows)
    cmd.stdin(std::process::Stdio::null());

    // Check for interrupt before we even launch the subprocess — the user may
    // have hit /stop while approval was pending or while a prior tool was running.
    if interrupt.as_ref().is_some_and(|i| i.is_cancelled()) {
        return Err("[interrupted before execution]".to_string());
    }

    // Capture piped output so we can collect it after the process exits.
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());

    // Ensure the child is terminated when the Child handle is dropped (e.g.
    // on timeout or session cancellation) rather than becoming an orphan.
    cmd.kill_on_drop(true);

    // Spawn the child process so we hold a handle that can be killed if the
    // session interrupt fires while the command is running.  Using `output()`
    // instead would block until the process *completes*, meaning cancel() would
    // never be observed mid-execution — the whole point of this feature.
    let child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => return Err(format!("Failed to execute command: {e}")),
    };

    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(timeout_secs);

    // Drive `wait_with_output()` directly: it owns the stdout/stderr pipes and
    // drains them concurrently with reaping the child. The previous
    // `try_wait`-with-50 ms-sleep poll loop did NOT drain pipes — so any child
    // that wrote more than the OS pipe buffer (often 8–16 KB on container
    // kernels) would deadlock on `write()`, never reach `try_wait → Some`, and
    // the loop would burn the full timeout. Confirmed reproducer:
    // `yes hello | head -c 30000` deadlocks at the 8 KB pipe boundary on this
    // box.
    //
    // Cancel-cascade preserved by select-ing the wait future against a 100 ms
    // periodic interrupt poll. If interrupt fires (or the deadline lapses),
    // dropping the wait future cancels the underlying child handle —
    // `kill_on_drop(true)` set above ensures the OS process is reaped.
    let interrupt_clone = interrupt.clone();
    let mut interrupt_tick = tokio::time::interval(std::time::Duration::from_millis(100));
    interrupt_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let wait_fut = child.wait_with_output();
    tokio::pin!(wait_fut);

    let output = loop {
        tokio::select! {
            biased;
            // Process exited (with pipes drained — that's the bug fix). Take the result.
            res = &mut wait_fut => break res.map_err(|e| format!("Failed to collect output: {e}")),
            // Periodic interrupt + deadline check. We drop wait_fut on either,
            // which kills the child via kill_on_drop.
            _ = interrupt_tick.tick() => {
                if interrupt_clone.as_ref().is_some_and(|i| i.is_cancelled()) {
                    return Err("[interrupted]".to_string());
                }
                if tokio::time::Instant::now() >= deadline {
                    return Err(format!("Command timed out after {timeout_secs}s"));
                }
            }
        }
    };

    match output {
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            let exit_code = output.status.code().unwrap_or(-1);

            // Truncate very long outputs to prevent memory issues
            let max_output = 100_000;
            let stdout_str = if stdout.len() > max_output {
                format!(
                    "{}...\n[truncated, {} total bytes]",
                    crate::str_utils::safe_truncate_str(&stdout, max_output),
                    stdout.len()
                )
            } else {
                stdout.to_string()
            };
            let stderr_str = if stderr.len() > max_output {
                format!(
                    "{}...\n[truncated, {} total bytes]",
                    crate::str_utils::safe_truncate_str(&stderr, max_output),
                    stderr.len()
                )
            } else {
                stderr.to_string()
            };

            Ok(format!(
                "Exit code: {exit_code}\n\nSTDOUT:\n{stdout_str}\nSTDERR:\n{stderr_str}"
            ))
        }
        Err(e) => Err(e),
    }
}
