//! `shell_exec` — run a single command inside the agent's workspace with
//! a sandboxed env, capture stdout/stderr/exit-code, honor session
//! interrupts, and enforce a deadline.
//!
//! Security gating (taint sinks, RO-workspace verb classification) lives
//! upstream in the dispatcher and `tool_runner::{taint, shell_safety}`;
//! by the time we reach this function the command has already been
//! cleared for execution.
//!
//! Migrated from `Result<String, String>` to `Result<String, ToolError>`
//! (#3576). Command-parse failures -> `InvalidParameter`; the two `io::Error`
//! sites (spawn / collect) -> `ToolError::Upstream` keeping the prefix message
//! AND the source; the interrupt / timeout control strings -> `upstream_msg`
//! so their exact wire text (`[interrupted]`, `Command timed out …`) is
//! preserved.

use super::error::{ToolError, ToolResult};
use std::path::Path;
use tracing::warn;

fn resolve_timeout(
    input: &serde_json::Value,
    exec_policy: Option<&librefang_types::config::ExecPolicy>,
) -> u64 {
    let policy_timeout = exec_policy.map(|p| p.timeout_secs).unwrap_or(30);
    input["timeout_seconds"].as_u64().unwrap_or(policy_timeout)
}

/// Kill the shell child process tree and log if cleanup fails.
/// `reason` is included in the log for observability (e.g. "interrupted",
/// "timed_out").
async fn kill_child_tree(pid: Option<u32>, reason: &str) {
    let Some(pid) = pid else { return };
    if let Err(e) = crate::subprocess_sandbox::kill_process_tree(pid, 0).await {
        warn!(pid, reason, error = %e, "failed to kill shell process tree");
    }
}

pub(super) async fn tool_shell_exec(
    input: &serde_json::Value,
    allowed_env: &[String],
    workspace_root: Option<&Path>,
    exec_policy: Option<&librefang_types::config::ExecPolicy>,
    interrupt: Option<crate::interrupt::SessionInterrupt>,
    process_registry: Option<&crate::process_registry::ProcessRegistry>,
    session_id: Option<String>,
) -> ToolResult {
    let command = input["command"]
        .as_str()
        .ok_or(ToolError::MissingParameter("command"))?;

    let timeout_secs = resolve_timeout(input, exec_policy);

    let max_output = exec_policy.map(|p| p.max_output_bytes).unwrap_or(100_000);

    let use_direct_exec = exec_policy
        .map(|p| p.mode == librefang_types::config::ExecSecurityMode::Allowlist)
        .unwrap_or(true);

    let mut cmd = if use_direct_exec {
        let argv = if cfg!(windows) {
            #[cfg(windows)]
            {
                windows_argv_split(command).ok_or(ToolError::InvalidParameter {
                    name: "command",
                    reason: "Command contains unmatched quotes or invalid shell syntax".to_string(),
                })?
            }
            #[cfg(not(windows))]
            {
                shlex::split(command).ok_or(ToolError::InvalidParameter {
                    name: "command",
                    reason: "Command contains unmatched quotes or invalid shell syntax".to_string(),
                })?
            }
        } else {
            shlex::split(command).ok_or(ToolError::InvalidParameter {
                name: "command",
                reason: "Command contains unmatched quotes or invalid shell syntax".to_string(),
            })?
        };
        if argv.is_empty() {
            return Err(ToolError::InvalidParameter {
                name: "command",
                reason: "Empty command after parsing".to_string(),
            });
        }
        let mut c = tokio::process::Command::new(&argv[0]);
        if argv.len() > 1 {
            c.args(&argv[1..]);
        }
        c
    } else {
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

    if let Some(ws) = workspace_root {
        cmd.current_dir(ws);
    }

    crate::subprocess_sandbox::sandbox_command(&mut cmd, allowed_env);

    #[cfg(windows)]
    cmd.env("PYTHONIOENCODING", "utf-8");

    cmd.stdin(std::process::Stdio::null());

    if interrupt.as_ref().is_some_and(|i| i.is_cancelled()) {
        return Err(ToolError::upstream_msg("[interrupted before execution]"));
    }

    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());

    cmd.kill_on_drop(true);

    #[cfg(unix)]
    cmd.process_group(0);

    let child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            return Err(ToolError::Upstream {
                message: format!("Failed to execute command: {e}"),
                source: Some(Box::new(e)),
            })
        }
    };

    let child_pid = child.id();
    if let (Some(reg), Some(pid)) = (process_registry, child_pid) {
        reg.register(pid, command.to_string(), session_id);
    }

    let deadline = tokio::time::Instant::now()
        .checked_add(std::time::Duration::from_secs(timeout_secs))
        .unwrap_or_else(|| tokio::time::Instant::now() + std::time::Duration::from_secs(3600));

    let interrupt_clone = interrupt.clone();
    let mut interrupt_tick = tokio::time::interval(std::time::Duration::from_millis(100));
    interrupt_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let wait_fut = child.wait_with_output();
    tokio::pin!(wait_fut);

    let output = loop {
        tokio::select! {
            biased;
            res = &mut wait_fut => break res.map_err(|e| ToolError::Upstream {
                message: format!("Failed to collect output: {e}"),
                source: Some(Box::new(e)),
            }),
            _ = interrupt_tick.tick() => {
                if interrupt_clone.as_ref().is_some_and(|i| i.is_cancelled()) {
                    kill_child_tree(child_pid, "interrupted").await;
                    return Err(ToolError::upstream_msg("[interrupted]"));
                }
                if tokio::time::Instant::now() >= deadline {
                    kill_child_tree(child_pid, "timed_out").await;
                    return Err(ToolError::upstream_msg(format!(
                        "Command timed out after {timeout_secs}s"
                    )));
                }
            }
        }
    };

    match output {
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            let exit_code = output.status.code().unwrap_or(-1);

            if let (Some(reg), Some(pid)) = (process_registry, child_pid) {
                reg.mark_finished(pid, exit_code);
            }

            let stdout_str = super::spill::spill_or_passthrough(
                "shell_exec",
                stdout.to_string(),
                max_output as u64,
                crate::artifact_store::DEFAULT_MAX_ARTIFACT_BYTES,
            );
            let stderr_str = super::spill::spill_or_passthrough(
                "shell_exec",
                stderr.to_string(),
                max_output as u64,
                crate::artifact_store::DEFAULT_MAX_ARTIFACT_BYTES,
            );

            Ok(format!(
                "Exit code: {exit_code}\n\nSTDOUT:\n{stdout_str}\nSTDERR:\n{stderr_str}"
            ))
        }
        Err(e) => Err(e),
    }
}

/// Split a command string into argv following Windows CRT rules.
///
/// Handles `\"` (escaped quote), `\\` (escaped backslash), and empty
/// quoted strings (`""` produces an empty arg). Returns `None` on
/// unterminated quotes.
#[cfg(any(windows, test))]
fn windows_argv_split(s: &str) -> Option<Vec<String>> {
    let mut args = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;

    enum State {
        LeadingWhitespace,
        Token,
        Delimited,
    }
    let mut state = State::LeadingWhitespace;
    let mut chars = s.chars().peekable();

    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                let mut count = 1;
                while chars.peek() == Some(&'\\') {
                    chars.next();
                    count += 1;
                }
                if chars.peek() == Some(&'"') {
                    chars.next();
                    let slashes = count / 2;
                    for _ in 0..slashes {
                        current.push('\\');
                    }
                    if count % 2 == 1 {
                        current.push('"');
                    } else {
                        in_quotes = !in_quotes;
                    }
                } else {
                    for _ in 0..count {
                        current.push('\\');
                    }
                }
                state = State::Token;
            }
            '"' => {
                in_quotes = !in_quotes;
                state = State::Token;
            }
            ' ' | '\t' if !in_quotes => {
                if let State::Token = state {
                    args.push(std::mem::take(&mut current));
                    state = State::Delimited;
                }
            }
            _ => {
                current.push(c);
                state = State::Token;
            }
        }
    }

    if in_quotes {
        return None;
    }
    if let State::Token = state {
        args.push(current);
    }
    if args.is_empty() {
        return None;
    }
    Some(args)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn shell_exec_missing_command_is_missing_parameter() {
        let r = tool_shell_exec(&json!({}), &[], None, None, None, None, None).await;
        assert!(matches!(r, Err(ToolError::MissingParameter("command"))));
    }

    #[tokio::test]
    async fn shell_exec_unmatched_quotes_is_invalid_parameter() {
        let r = tool_shell_exec(
            &json!({"command": "echo \"unterminated"}),
            &[],
            None,
            None,
            None,
            None,
            None,
        )
        .await;
        assert!(matches!(
            r,
            Err(ToolError::InvalidParameter {
                name: "command",
                ..
            })
        ));
    }

    #[test]
    fn argv_simple_space_separated() {
        assert_eq!(
            windows_argv_split("cmd a b").unwrap(),
            vec!["cmd", "a", "b"],
        );
    }

    #[test]
    fn argv_quoted_arg_preserves_spaces() {
        assert_eq!(
            windows_argv_split(r#"cmd "a b""#).unwrap(),
            vec!["cmd", "a b"],
        );
    }

    #[test]
    fn argv_escaped_quote_inside_quoted() {
        assert_eq!(
            windows_argv_split(r#"cmd "a\"b""#).unwrap(),
            vec!["cmd", r#"a"b"#],
        );
    }

    #[test]
    fn argv_escaped_quote_outside_quotes() {
        assert_eq!(
            windows_argv_split(r#"cmd \"escaped\""#).unwrap(),
            vec!["cmd", r#""escaped""#],
        );
    }

    #[test]
    fn argv_escaped_quote_with_space_splits() {
        assert_eq!(
            windows_argv_split(r#"cmd \"a\" \"b\""#).unwrap(),
            vec!["cmd", r#""a""#, r#""b""#],
        );
    }

    #[test]
    fn argv_empty_quoted_string() {
        assert_eq!(
            windows_argv_split(r#"cmd "" arg"#).unwrap(),
            vec!["cmd", "", "arg"],
        );
    }

    #[test]
    fn argv_double_backslash_before_quote() {
        assert_eq!(
            windows_argv_split(r#"cmd "a\\" b"#).unwrap(),
            vec!["cmd", r#"a\"#, "b"],
        );
    }

    #[test]
    fn argv_triple_backslash_before_quote() {
        assert_eq!(
            windows_argv_split(r#"cmd "a\\\" b""#).unwrap(),
            vec!["cmd", r#"a\" b"#],
        );
    }

    #[test]
    fn argv_unterminated_quote_is_none() {
        assert!(windows_argv_split(r#"cmd "unterminated"#).is_none());
    }

    #[test]
    fn argv_empty_input_is_none() {
        assert!(windows_argv_split("").is_none());
    }

    #[test]
    fn argv_whitespace_only_is_none() {
        assert!(windows_argv_split("   ").is_none());
    }

    #[test]
    fn argv_multiple_spaces_no_empty_args() {
        assert_eq!(windows_argv_split("cmd   a").unwrap(), vec!["cmd", "a"],);
    }

    #[test]
    fn timeout_default_uses_policy_value() {
        let policy = librefang_types::config::ExecPolicy::default();
        assert_eq!(resolve_timeout(&json!({}), Some(&policy)), 30);
    }

    #[test]
    fn timeout_override_up_is_not_clamped() {
        let policy = librefang_types::config::ExecPolicy::default();
        assert_eq!(
            resolve_timeout(&json!({"timeout_seconds": 300}), Some(&policy)),
            300,
        );
    }

    #[test]
    fn timeout_override_down_is_honored() {
        let policy = librefang_types::config::ExecPolicy::default();
        assert_eq!(
            resolve_timeout(&json!({"timeout_seconds": 5}), Some(&policy)),
            5,
        );
    }

    #[test]
    fn timeout_no_policy_uses_hardcoded_default() {
        assert_eq!(resolve_timeout(&json!({}), None), 30);
    }

    #[test]
    fn timeout_no_policy_with_input_override() {
        assert_eq!(resolve_timeout(&json!({"timeout_seconds": 120}), None), 120,);
    }
}
