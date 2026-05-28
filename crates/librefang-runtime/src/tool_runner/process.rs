//! Persistent process tools — start / poll / write / kill / list.
//!
//! Migrated from `Result<String, String>` to `Result<String, ToolError>`
//! as part of #3576 (ToolError migration).

use super::error::{ToolError, ToolResult};

const MAX_POLL_OUTPUT_BYTES: usize = 256 * 1024;

/// Start a long-running process (REPL, server, watcher).
pub(super) async fn tool_process_start(
    input: &serde_json::Value,
    pm: Option<&crate::process_manager::ProcessManager>,
    caller_agent_id: Option<&str>,
) -> ToolResult {
    let pm = pm.ok_or(ToolError::Unavailable("Process manager"))?;
    let agent_id = caller_agent_id.ok_or(ToolError::MissingParameter("caller_agent_id"))?;
    let command = input["command"]
        .as_str()
        .ok_or(ToolError::MissingParameter("command"))?;
    let args: Vec<String> = match input["args"].as_array() {
        Some(arr) => {
            let mut out = Vec::with_capacity(arr.len());
            for (i, v) in arr.iter().enumerate() {
                match v.as_str() {
                    Some(s) => out.push(s.to_string()),
                    None => {
                        tracing::warn!(
                            index = i,
                            value = %v,
                            "Dropping non-string arg in process_start"
                        );
                    }
                }
            }
            out
        }
        None => Vec::new(),
    };

    let proc_id = pm
        .start(agent_id, command, &args)
        .await
        .map_err(ToolError::upstream_msg)?;
    Ok(serde_json::json!({
        "process_id": proc_id,
        "status": "started"
    })
    .to_string())
}

/// Read accumulated stdout/stderr from a process (non-blocking drain).
pub(super) async fn tool_process_poll(
    input: &serde_json::Value,
    pm: Option<&crate::process_manager::ProcessManager>,
) -> ToolResult {
    let pm = pm.ok_or(ToolError::Unavailable("Process manager"))?;
    let proc_id = input["process_id"]
        .as_str()
        .ok_or(ToolError::MissingParameter("process_id"))?;
    let (stdout, stderr) = pm.read(proc_id).await.map_err(ToolError::upstream_msg)?;

    let stdout_joined = join_with_cap(&stdout, MAX_POLL_OUTPUT_BYTES);
    let stderr_joined = join_with_cap(&stderr, MAX_POLL_OUTPUT_BYTES);

    let mut resp = serde_json::json!({
        "stdout": stdout_joined.text,
        "stderr": stderr_joined.text,
    });
    if stdout_joined.truncated || stderr_joined.truncated {
        resp["truncated"] = serde_json::json!(true);
    }
    Ok(resp.to_string())
}

/// Write data to a process's stdin.
pub(super) async fn tool_process_write(
    input: &serde_json::Value,
    pm: Option<&crate::process_manager::ProcessManager>,
) -> ToolResult {
    let pm = pm.ok_or(ToolError::Unavailable("Process manager"))?;
    let proc_id = input["process_id"]
        .as_str()
        .ok_or(ToolError::MissingParameter("process_id"))?;
    let data = input["data"]
        .as_str()
        .ok_or(ToolError::MissingParameter("data"))?;
    // Always append newline if not present — REPLs and line-oriented
    // interpreters expect line submission via stdin.
    let data = if data.ends_with('\n') {
        data.to_string()
    } else {
        format!("{data}\n")
    };
    pm.write(proc_id, &data)
        .await
        .map_err(ToolError::upstream_msg)?;
    Ok(serde_json::json!({
        "status": "written"
    })
    .to_string())
}

/// Terminate a process.
pub(super) async fn tool_process_kill(
    input: &serde_json::Value,
    pm: Option<&crate::process_manager::ProcessManager>,
) -> ToolResult {
    let pm = pm.ok_or(ToolError::Unavailable("Process manager"))?;
    let proc_id = input["process_id"]
        .as_str()
        .ok_or(ToolError::MissingParameter("process_id"))?;
    pm.kill(proc_id).await.map_err(ToolError::upstream_msg)?;
    Ok(serde_json::json!({
        "status": "killed"
    })
    .to_string())
}

/// List processes for the current agent.
pub(super) async fn tool_process_list(
    pm: Option<&crate::process_manager::ProcessManager>,
    caller_agent_id: Option<&str>,
) -> ToolResult {
    let pm = pm.ok_or(ToolError::Unavailable("Process manager"))?;
    let agent_id = caller_agent_id.ok_or(ToolError::MissingParameter("caller_agent_id"))?;
    let procs = pm.list(agent_id);
    let list: Vec<serde_json::Value> = procs
        .iter()
        .map(|p| {
            serde_json::json!({
                "id": p.id,
                "command": p.command,
                "alive": p.alive,
                "uptime_secs": p.uptime_secs,
            })
        })
        .collect();
    Ok(serde_json::Value::Array(list).to_string())
}

struct CappedOutput {
    text: String,
    truncated: bool,
}

/// Join lines with a byte cap. If a single line would exceed the cap,
/// truncate it at a char boundary rather than dropping all output.
fn join_with_cap(lines: &[String], max_bytes: usize) -> CappedOutput {
    let mut buf = String::with_capacity(max_bytes.min(lines.len() * 64));
    let mut truncated = false;
    for line in lines {
        let remaining = max_bytes.saturating_sub(buf.len());
        if remaining == 0 {
            truncated = true;
            break;
        }
        if line.len() <= remaining {
            buf.push_str(line);
            if remaining - line.len() > 0 {
                buf.push('\n');
            }
        } else {
            // Line would exceed cap — truncate at a char boundary.
            truncated = true;
            let mut end = remaining.min(line.len());
            while end > 0 && !line.is_char_boundary(end) {
                end -= 1;
            }
            buf.push_str(&line[..end]);
            break;
        }
    }
    CappedOutput {
        text: buf,
        truncated,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn process_tools_without_manager_return_unavailable() {
        assert!(matches!(
            tool_process_start(&json!({}), None, None).await,
            Err(ToolError::Unavailable("Process manager"))
        ));
        assert!(matches!(
            tool_process_poll(&json!({}), None).await,
            Err(ToolError::Unavailable("Process manager"))
        ));
        assert!(matches!(
            tool_process_write(&json!({}), None).await,
            Err(ToolError::Unavailable("Process manager"))
        ));
        assert!(matches!(
            tool_process_kill(&json!({}), None).await,
            Err(ToolError::Unavailable("Process manager"))
        ));
        assert!(matches!(
            tool_process_list(None, None).await,
            Err(ToolError::Unavailable("Process manager"))
        ));
    }

    #[test]
    fn join_with_cap_truncates_within_long_line() {
        let lines = vec!["a".repeat(300_000)];
        let result = join_with_cap(&lines, 256 * 1024);
        assert!(result.truncated);
        assert!(!result.text.is_empty());
        assert!(result.text.len() <= 256 * 1024);
    }

    #[test]
    fn join_with_cap_empty_on_zero_budget() {
        let lines = vec!["hello".to_string()];
        let result = join_with_cap(&lines, 0);
        assert!(result.truncated);
        assert!(result.text.is_empty());
    }

    #[test]
    fn join_with_cap_full_line_fits() {
        let lines = vec!["hello".to_string(), "world".to_string()];
        let result = join_with_cap(&lines, 100);
        assert!(!result.truncated);
        assert_eq!(result.text, "hello\nworld\n");
    }

    #[test]
    fn join_with_cap_exact_fit_not_truncated() {
        // Line length exactly equals cap — fits, only trailing \n is dropped.
        let line = "x".repeat(100);
        let lines = vec![line];
        let result = join_with_cap(&lines, 100);
        assert!(!result.truncated);
        assert_eq!(result.text.len(), 100);
        assert!(!result.text.ends_with('\n'));
    }

    #[test]
    fn join_with_cap_respects_char_boundary() {
        // Multi-byte UTF-8 character at the truncation point.
        let line = "x".repeat(100) + "\u{1F600}"; // emoji = 4 bytes
        let lines = vec![line];
        // 100 bytes + 4-byte emoji = 104, but cap at 102 → must not split emoji
        let result = join_with_cap(&lines, 102);
        assert!(result.truncated);
        assert!(result.text.is_char_boundary(result.text.len()));
    }
}
