//! Persistent process tools — start / poll / write / kill / list.
//!
//! Migrated from `Result<String, String>` to `Result<String, ToolError>`
//! (#3576). A missing `ProcessManager` -> `Unavailable("Process manager")`;
//! missing params -> `MissingParameter`; the `ProcessManager` operations (all
//! `Result<_, String>`) -> `upstream_msg`. The JSON status payloads are built
//! infallibly and unchanged.

use super::error::{ToolError, ToolResult};

/// Start a long-running process (REPL, server, watcher).
pub(super) async fn tool_process_start(
    input: &serde_json::Value,
    pm: Option<&crate::process_manager::ProcessManager>,
    caller_agent_id: Option<&str>,
) -> ToolResult {
    let pm = pm.ok_or(ToolError::Unavailable("Process manager"))?;
    let agent_id = caller_agent_id.unwrap_or("default");
    let command = input["command"]
        .as_str()
        .ok_or(ToolError::MissingParameter("command"))?;
    let args: Vec<String> = input["args"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

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
    Ok(serde_json::json!({
        "stdout": stdout,
        "stderr": stderr,
    })
    .to_string())
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
    // Always append newline if not present (common expectation for REPLs)
    let data = if data.ends_with('\n') {
        data.to_string()
    } else {
        format!("{data}\n")
    };
    pm.write(proc_id, &data)
        .await
        .map_err(ToolError::upstream_msg)?;
    Ok(r#"{"status": "written"}"#.to_string())
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
    Ok(r#"{"status": "killed"}"#.to_string())
}

/// List processes for the current agent.
pub(super) async fn tool_process_list(
    pm: Option<&crate::process_manager::ProcessManager>,
    caller_agent_id: Option<&str>,
) -> ToolResult {
    let pm = pm.ok_or(ToolError::Unavailable("Process manager"))?;
    let agent_id = caller_agent_id.unwrap_or("default");
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
}
