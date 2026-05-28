//! `docker_exec` sandbox tool — run an LLM-supplied command inside a
//! disposable Docker container scoped to the agent's workspace.
//!
//! Migrated from `Result<String, String>` to `Result<String, ToolError>`
//! (#3576). The "subsystem not wired" cases (docker not configured / disabled
//! / not installed / no workspace context) map to `ToolError::Unavailable`,
//! the documented category for them (see the variant doc: "docker exec
//! disabled" is an Unavailable case → HTTP 503). The container create / exec
//! failures (`Result<_, String>`) map to `ToolError::upstream_msg`.

use super::error::{ToolError, ToolResult};
use std::path::Path;
use tracing::warn;

pub(super) async fn tool_docker_exec(
    input: &serde_json::Value,
    docker_config: Option<&librefang_types::config::DockerSandboxConfig>,
    workspace_root: Option<&Path>,
    caller_agent_id: Option<&str>,
) -> ToolResult {
    let config = docker_config.ok_or(ToolError::Unavailable("Docker sandbox"))?;

    // `disabled` and `not-installed` carry an operator-actionable hint, so keep
    // the full pre-#3576 message (via upstream_msg) rather than the bare
    // `Unavailable` category — the hint is the point.
    if !config.enabled {
        return Err(ToolError::upstream_msg(
            "Docker sandbox is disabled. Set docker.enabled=true in config.",
        ));
    }

    let command = input["command"]
        .as_str()
        .ok_or(ToolError::MissingParameter("command"))?;

    let workspace = workspace_root.ok_or(ToolError::Unavailable("workspace directory"))?;
    let agent_id = caller_agent_id.unwrap_or("default");

    // Check Docker availability
    if !crate::docker_sandbox::is_docker_available().await {
        return Err(ToolError::upstream_msg(
            "Docker is not available on this system. Install Docker to use docker_exec.",
        ));
    }

    // Create sandbox container
    let container = crate::docker_sandbox::create_sandbox(config, agent_id, workspace)
        .await
        .map_err(ToolError::upstream_msg)?;

    // Execute command with timeout
    let timeout = std::time::Duration::from_secs(config.timeout_secs);
    let result = crate::docker_sandbox::exec_in_sandbox(&container, command, timeout).await;

    // Always destroy the container after execution
    if let Err(e) = crate::docker_sandbox::destroy_sandbox(&container).await {
        warn!("Failed to destroy Docker sandbox: {e}");
    }

    let exec_result = result.map_err(ToolError::upstream_msg)?;

    let response = serde_json::json!({
        "exit_code": exec_result.exit_code,
        "stdout": exec_result.stdout,
        "stderr": exec_result.stderr,
        "container_id": container.container_id,
    });

    Ok(serde_json::to_string_pretty(&response)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn docker_exec_without_config_is_unavailable() {
        let r = tool_docker_exec(&serde_json::json!({"command": "ls"}), None, None, None).await;
        assert!(matches!(r, Err(ToolError::Unavailable("Docker sandbox"))));
    }

    #[tokio::test]
    async fn docker_exec_disabled_returns_actionable_message() {
        let cfg = librefang_types::config::DockerSandboxConfig {
            enabled: false,
            ..Default::default()
        };
        let r = tool_docker_exec(
            &serde_json::json!({"command": "ls"}),
            Some(&cfg),
            None,
            None,
        )
        .await;
        // Disabled carries the actionable hint rather than a bare Unavailable.
        let msg = r.unwrap_err().to_string();
        assert!(msg.contains("docker.enabled=true"), "got: {msg}");
    }
}
