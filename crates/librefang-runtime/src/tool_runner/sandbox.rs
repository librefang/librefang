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

struct SandboxGuard {
    container: crate::docker_sandbox::SandboxContainer,
}

impl Drop for SandboxGuard {
    fn drop(&mut self) {
        let container_id = self.container.container_id.clone();
        tokio::task::block_in_place(|| {
            let rt = tokio::runtime::Handle::current();
            rt.block_on(async {
                if let Err(e) = crate::docker_sandbox::destroy_sandbox(&self.container).await {
                    warn!("Failed to destroy Docker sandbox {container_id}: {e}");
                }
            });
        });
    }
}

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

    // Check Docker availability before creating the sandbox — surfaces an
    // actionable "Install Docker" hint instead of a raw spawn error.
    if !crate::docker_sandbox::is_docker_available().await {
        return Err(ToolError::upstream_msg(
            "Docker is not available on this system. Install Docker to use docker_exec.",
        ));
    }

    let container = crate::docker_sandbox::create_sandbox(config, agent_id, workspace)
        .await
        .map_err(ToolError::upstream_msg)?;

    let _guard = SandboxGuard { container };

    let timeout = std::time::Duration::from_secs(config.timeout_secs);
    let exec_result = crate::docker_sandbox::exec_in_sandbox(&_guard.container, command, timeout)
        .await
        .map_err(ToolError::upstream_msg)?;

    let response = serde_json::json!({
        "exit_code": exec_result.exit_code,
        "stdout": exec_result.stdout,
        "stderr": exec_result.stderr,
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

    /// `SandboxGuard::drop` must not panic when dropped on a multi-thread
    /// tokio runtime. The drop path uses `block_in_place` +
    /// `Handle::current().block_on` which panics if called outside a runtime
    /// context or on a current-thread runtime during shutdown.
    #[tokio::test(flavor = "multi_thread")]
    async fn sandbox_guard_drop_does_not_panic() {
        // We cannot easily spin up a real Docker container in a unit test, so
        // exercise the guard construction + drop with a mock container id.
        // The real container would fail `destroy_sandbox`, but the guard's
        // Drop impl logs and swallows that error — the important invariant
        // is that the block_in_place + Handle::current() path does not panic.
        //
        // If `is_docker_available()` returns false in CI we skip gracefully.
        if !crate::docker_sandbox::is_docker_available().await {
            return;
        }

        let cfg = librefang_types::config::DockerSandboxConfig {
            enabled: true,
            ..Default::default()
        };
        let workspace = std::path::Path::new("/tmp");
        let container = crate::docker_sandbox::create_sandbox(&cfg, "test-agent", workspace).await;
        match container {
            Ok(c) => {
                let guard = super::SandboxGuard { container: c };
                // Guard dropped here — Drop impl runs block_in_place.
                drop(guard);
            }
            Err(_) => {
                // Docker present but sandbox creation failed (e.g. image pull).
                // Not a test failure — the drop path is what we're exercising.
            }
        }
    }
}
