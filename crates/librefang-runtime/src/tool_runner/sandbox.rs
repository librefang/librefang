//! `docker_exec` sandbox tool — run an LLM-supplied command inside a Docker
//! container scoped to the agent's workspace.
//!
//! Migrated from `Result<String, String>` to `Result<String, ToolError>`
//! (#3576). The "subsystem not wired" cases (docker not configured / disabled
//! / not installed / no workspace context) map to `ToolError::Unavailable`,
//! the documented category for them (see the variant doc: "docker exec
//! disabled" is an Unavailable case → HTTP 503). The container create / exec
//! failures (`Result<_, String>`) map to `ToolError::upstream_msg`.
//!
//! Container lifetime is decided by `[docker] scope`, not by this call: the
//! container is taken from (and returned to) the process-wide
//! [`crate::docker_sandbox::ContainerPool`] under the key
//! [`crate::docker_sandbox::PoolKey::derive`] builds for the configured scope.
//! Only a call that derives no key — `scope = "session"` with no session to pin
//! the container to — keeps the historical create-exec-destroy shape.

use super::error::{ToolError, ToolResult};
use crate::docker_sandbox::{ContainerPool, PoolKey, SandboxContainer};
use std::path::Path;
use std::sync::Arc;
use tracing::warn;

/// Owns the container for the duration of one `docker_exec` call.
///
/// On drop the container either goes back to the pool — `key` is `Some`, so the scope says
/// some later call may legitimately reuse it — or is destroyed. Only the destroy branch has
/// to block; releasing to the pool is a synchronous map insert.
struct SandboxGuard {
    container: Option<SandboxContainer>,
    key: Option<PoolKey>,
    pool: Arc<ContainerPool>,
    /// The agent this call runs as, recorded on release so `scope = "shared"` can tell an
    /// agent picking its own container back up from a handover to a different one.
    agent_id: String,
}

impl SandboxGuard {
    fn new(
        container: SandboxContainer,
        key: Option<PoolKey>,
        pool: Arc<ContainerPool>,
        agent_id: &str,
    ) -> Self {
        Self {
            container: Some(container),
            key,
            pool,
            agent_id: agent_id.to_string(),
        }
    }

    /// The container this call is running against.
    fn container(&self) -> &SandboxContainer {
        self.container
            .as_ref()
            .expect("SandboxGuard always holds a container between new() and drop()")
    }

    /// Take the container out of the guard so the caller can dispose of it itself.
    /// The guard becomes inert until [`SandboxGuard::replace`] refills it.
    fn take(&mut self) -> Option<SandboxContainer> {
        self.container.take()
    }

    fn replace(&mut self, container: SandboxContainer) {
        self.container = Some(container);
    }

    /// Mark the container unfit for reuse, so the guard destroys it on drop rather than
    /// releasing it to the pool.
    fn poison(&mut self) {
        self.key = None;
    }
}

impl Drop for SandboxGuard {
    fn drop(&mut self) {
        let Some(container) = self.container.take() else {
            return;
        };
        match self.key.take() {
            Some(key) => self.pool.release(container, key, &self.agent_id),
            None => {
                let container_id = container.container_id.clone();
                tokio::task::block_in_place(|| {
                    let rt = tokio::runtime::Handle::current();
                    rt.block_on(async {
                        if let Err(e) = crate::docker_sandbox::destroy_sandbox(&container).await {
                            warn!("Failed to destroy Docker sandbox {container_id}: {e}");
                        }
                    });
                });
            }
        }
    }
}

pub(super) async fn tool_docker_exec(
    input: &serde_json::Value,
    docker_config: Option<&librefang_types::config::DockerSandboxConfig>,
    workspace_root: Option<&Path>,
    caller_agent_id: Option<&str>,
    session_id: Option<librefang_types::agent::SessionId>,
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

    // Reject an unsafe command before any container is involved. `exec_in_sandbox` checks it
    // again, but the stale-container retry below would otherwise spend a freshly created
    // container re-learning that the command was never going to run.
    crate::docker_sandbox::validate_command(command).map_err(ToolError::upstream_msg)?;

    // Check Docker availability before creating the sandbox — surfaces an
    // actionable "Install Docker" hint instead of a raw spawn error.
    if !crate::docker_sandbox::is_docker_available().await {
        return Err(ToolError::upstream_msg(
            "Docker is not available on this system. Install Docker to use docker_exec.",
        ));
    }

    let pool = crate::docker_sandbox::global_pool();
    let session = session_id.map(|s| s.to_string());
    let key = PoolKey::derive(config, agent_id, session.as_deref(), workspace);

    let reused = key
        .as_ref()
        .and_then(|k| pool.acquire(k, agent_id, config.reuse_cool_secs));
    let from_pool = reused.is_some();
    let container = match reused {
        Some(c) => c,
        None => crate::docker_sandbox::create_sandbox(config, agent_id, workspace)
            .await
            .map_err(ToolError::upstream_msg)?,
    };

    let timeout = std::time::Duration::from_secs(config.timeout_secs);
    let mut guard = SandboxGuard::new(container, key, pool, agent_id);
    let mut exec_result =
        crate::docker_sandbox::exec_in_sandbox(guard.container(), command, timeout).await;

    // A pooled container can have been removed out from under us — a Docker daemon restart,
    // an operator's `docker rm`, an out-of-disk eviction. That is not a tool failure, but
    // without this retry the dead container would be handed straight back to the pool and
    // every later call for the same key would fail the same way.
    if exec_result.is_err() && from_pool {
        if let Some(dead) = guard.take() {
            warn!(
                container = %dead.container_id,
                "Pooled Docker sandbox failed; discarding it and retrying on a fresh container"
            );
            if let Err(e) = crate::docker_sandbox::destroy_sandbox(&dead).await {
                warn!("Failed to destroy stale pooled Docker sandbox: {e}");
            }
        }
        let fresh = crate::docker_sandbox::create_sandbox(config, agent_id, workspace)
            .await
            .map_err(ToolError::upstream_msg)?;
        guard.replace(fresh);
        exec_result =
            crate::docker_sandbox::exec_in_sandbox(guard.container(), command, timeout).await;
    }

    if exec_result.is_err() {
        // A failed exec leaves the container unfit for reuse. On a timeout `exec_in_sandbox`
        // only SIGKILLs the host-side `docker exec` client — the in-container workload keeps
        // running, and destroying the container is the only thing that bounds it. Anything
        // else that fails here (spawn, pipe, wait) tells us as little about what state the
        // container is in. Dropping the pool key makes the guard tear it down rather than
        // hand it on.
        guard.poison();
    }
    let exec_result = exec_result.map_err(ToolError::upstream_msg)?;

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
    use librefang_types::config::DockerScope;

    #[tokio::test]
    async fn docker_exec_without_config_is_unavailable() {
        let r = tool_docker_exec(
            &serde_json::json!({"command": "ls"}),
            None,
            None,
            None,
            None,
        )
        .await;
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
            None,
        )
        .await;
        // Disabled carries the actionable hint rather than a bare Unavailable.
        let msg = r.unwrap_err().to_string();
        assert!(msg.contains("docker.enabled=true"), "got: {msg}");
    }

    /// An unsafe command is rejected before Docker is touched at all, so the check holds on a
    /// host with no daemon. Before the pool landed this validation happened inside
    /// `exec_in_sandbox`, i.e. only after a container had been created.
    #[tokio::test]
    async fn docker_exec_rejects_unsafe_command_before_touching_docker() {
        let cfg = librefang_types::config::DockerSandboxConfig {
            enabled: true,
            ..Default::default()
        };
        let r = tool_docker_exec(
            &serde_json::json!({"command": "ls; rm -rf /"}),
            Some(&cfg),
            Some(std::path::Path::new("/tmp")),
            Some("agent-1"),
            None,
        )
        .await;
        let msg = r.unwrap_err().to_string();
        assert!(msg.contains("Command blocked"), "got: {msg}");
    }

    /// The guard hands a scope-keyed container back to the pool instead of destroying it —
    /// that is what makes `scope = "agent"` / `"shared"` mean anything. Exercised with a
    /// synthetic container so no Docker daemon is needed: the release path never shells out.
    #[tokio::test(flavor = "multi_thread")]
    async fn guard_releases_keyed_container_to_the_pool() {
        let pool = Arc::new(ContainerPool::new());
        let cfg = librefang_types::config::DockerSandboxConfig {
            enabled: true,
            scope: DockerScope::Agent,
            ..Default::default()
        };
        let key = PoolKey::derive(&cfg, "agent-1", None, std::path::Path::new("/tmp"))
            .expect("agent scope always yields a key");
        let container = SandboxContainer {
            container_id: "guard-release-1".to_string(),
            agent_id: "agent-1".to_string(),
            created_at: chrono::Utc::now(),
        };

        let guard = SandboxGuard::new(container, Some(key.clone()), Arc::clone(&pool), "agent-1");
        drop(guard);

        assert_eq!(
            pool.acquire(&key, "agent-1", cfg.reuse_cool_secs)
                .map(|c| c.container_id),
            Some("guard-release-1".to_string()),
            "a scope-keyed container must go back to the pool on drop"
        );
    }

    /// A container whose exec failed must not go back to the pool. On a timeout the
    /// in-container workload survives — `exec_in_sandbox` only kills the host-side
    /// `docker exec` client — so reusing the container would hand the next call a sandbox with
    /// someone else's runaway process still in it.
    #[tokio::test(flavor = "multi_thread")]
    async fn poisoned_guard_does_not_return_the_container_to_the_pool() {
        let pool = Arc::new(ContainerPool::new());
        let cfg = librefang_types::config::DockerSandboxConfig {
            enabled: true,
            scope: DockerScope::Agent,
            ..Default::default()
        };
        let key = PoolKey::derive(&cfg, "agent-1", None, std::path::Path::new("/tmp"))
            .expect("agent scope always yields a key");
        let container = SandboxContainer {
            container_id: "guard-poison-1".to_string(),
            agent_id: "agent-1".to_string(),
            created_at: chrono::Utc::now(),
        };

        let mut guard = SandboxGuard::new(container, Some(key), Arc::clone(&pool), "agent-1");
        guard.poison();
        drop(guard);

        assert!(
            pool.is_empty(),
            "a poisoned container must be destroyed, not pooled"
        );
    }

    /// `SandboxGuard::drop` must not panic when dropped on a multi-thread
    /// tokio runtime. The unpooled drop path uses `block_in_place` +
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
                // `key: None` is the unpooled shape — the guard destroys the container.
                let guard =
                    SandboxGuard::new(c, None, crate::docker_sandbox::global_pool(), "test-agent");
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
