use async_trait::async_trait;

use super::*;

/// Agent info returned by list and discovery operations.
#[derive(Debug, Clone)]
pub struct AgentInfo {
    pub id: String,
    pub name: String,
    pub state: String,
    pub model_provider: String,
    pub model_name: String,
    pub description: String,
    pub tags: Vec<String>,
    pub tools: Vec<String>,
}

// ============================================================================
// 1. AgentControl — agent lifecycle, inter-agent send, listing, heartbeats,
//    forked one-shot calls, plus a couple of agent-scoped config queries
//    (`max_agent_call_depth`, `fire_agent_step`).
// ============================================================================

#[async_trait]
pub trait AgentControl: Send + Sync {
    /// Spawn a new agent from a TOML manifest string.
    /// `parent_id` is the UUID string of the spawning agent (for lineage tracking).
    /// Returns (agent_id, agent_name) on success.
    async fn spawn_agent(
        &self,
        manifest_toml: &str,
        parent_id: Option<&str>,
    ) -> Result<(String, String), KernelOpError>;

    /// Spawn an agent with capability inheritance enforcement.
    /// `parent_caps` are the parent's granted capabilities. The kernel MUST verify
    /// that every capability in the child manifest is covered by `parent_caps`.
    async fn spawn_agent_checked(
        &self,
        manifest_toml: &str,
        parent_id: Option<&str>,
        parent_caps: &[librefang_types::capability::Capability],
    ) -> Result<(String, String), KernelOpError> {
        // Default: delegate to spawn_agent (no enforcement)
        // The kernel MUST override this with real enforcement
        let _ = parent_caps;
        self.spawn_agent(manifest_toml, parent_id).await
    }

    /// Send a message to another agent and get the response.
    async fn send_to_agent(&self, agent_id: &str, message: &str) -> Result<String, KernelOpError>;

    /// Like [`send_to_agent`](Self::send_to_agent), but records that the
    /// call was made on behalf of `parent_agent_id`, so a `/stop` issued to
    /// the parent cascades into the callee's loop (issue #3044). Defaults
    /// to the plain `send_to_agent` behavior for implementations that
    /// don't support cancel cascading — a trace log flags the fallthrough
    /// so operators can tell a non-standard handle is in play.
    async fn send_to_agent_as(
        &self,
        agent_id: &str,
        message: &str,
        parent_agent_id: &str,
    ) -> Result<String, KernelOpError> {
        tracing::trace!(
            agent = %agent_id,
            parent = %parent_agent_id,
            "send_to_agent_as: default impl — cancel cascade not supported by this handle"
        );
        self.send_to_agent(agent_id, message).await
    }

    /// Like [`send_to_agent`](Self::send_to_agent), but pins the callee to a
    /// deterministic session derived from `conversation_key`. The kernel maps
    /// the key to `SessionId::for_channel(target, "agent_send:<key>")`, so
    /// the same key always resolves to the same session (history preserved)
    /// and a different key always resolves to a distinct session. Defaults to
    /// the plain `send_to_agent` behaviour for implementations that do not
    /// support session pinning.
    async fn send_to_agent_with_key(
        &self,
        agent_id: &str,
        message: &str,
        conversation_key: &str,
    ) -> Result<String, KernelOpError> {
        let _ = conversation_key;
        self.send_to_agent(agent_id, message).await
    }

    /// Like [`send_to_agent_as`](Self::send_to_agent_as), but also pins the
    /// callee session via `conversation_key` (see
    /// [`send_to_agent_with_key`](Self::send_to_agent_with_key)). Explicit
    /// `conversation_key` takes precedence over the target manifest
    /// `session_mode`. Defaults to `send_to_agent_as` for implementations
    /// that do not support session pinning.
    async fn send_to_agent_as_with_key(
        &self,
        agent_id: &str,
        message: &str,
        parent_agent_id: &str,
        conversation_key: &str,
    ) -> Result<String, KernelOpError> {
        let _ = conversation_key;
        self.send_to_agent_as(agent_id, message, parent_agent_id)
            .await
    }

    /// List all running agents.
    fn list_agents(&self) -> Vec<AgentInfo>;

    /// Kill an agent by ID.
    fn kill_agent(&self, agent_id: &str) -> Result<(), KernelOpError>;

    /// Find agents by query (matches on name substring, tag, or tool name; case-insensitive).
    fn find_agents(&self, query: &str) -> Vec<AgentInfo>;

    /// Touch the agent's `last_active` timestamp to prevent heartbeat false-positives
    /// during long-running operations (e.g., LLM calls).
    fn touch_heartbeat(&self, agent_id: &str) {
        let _ = agent_id;
    }

    /// Fire an `agent:step` external hook event.
    /// Called by the runtime at the start of each agent loop iteration.
    fn fire_agent_step(&self, _agent_id: &str, _step: u32) {}

    /// Run a forked agent turn that collapses to a single text response —
    /// the "structured-output via forked call" primitive. Used by the
    /// proactive memory extractor so its LLM call shares the parent
    /// turn's `(system + tools + messages)` prefix for Anthropic prompt
    /// cache alignment, instead of issuing a standalone `driver.complete()`
    /// that always starts cold.
    ///
    /// Internally: spawn `run_forked_agent_streaming`, drain to completion,
    /// return the final assistant text. Fork semantics apply — the call's
    /// messages do NOT persist into the agent's canonical session, and the
    /// turn-end hook fires with `is_fork: true` so auto-dream won't
    /// recurse.
    ///
    /// `allowed_tools = Some(vec![])` keeps the fork single-turn (no tool
    /// calls permitted — model returns text). Pass a larger allowlist only
    /// when the caller actually expects tool use (e.g. future extractors
    /// that want the fork to call `memory_store` directly).
    ///
    /// Default: error. The real kernel overrides; tests / stubs that
    /// don't implement the full streaming path just fall back to a
    /// standalone driver call through the extractor's own path.
    async fn run_forked_agent_oneshot(
        &self,
        _agent_id: &str,
        _prompt: &str,
        _allowed_tools: Option<Vec<String>>,
    ) -> Result<String, KernelOpError> {
        Err(KernelOpError::unavailable("run_forked_agent_oneshot"))
    }

    /// Maximum inter-agent call depth (from config). Default: 5.
    fn max_agent_call_depth(&self) -> u32 {
        5
    }
}
