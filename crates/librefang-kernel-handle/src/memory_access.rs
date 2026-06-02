use super::*;

// ============================================================================
// 2. MemoryAccess — per-agent key/value memory + per-user RBAC ACL resolution
//
// DESIGN NOTE: Internal kernel subsystems (messaging, agent_execution,
// prompt_context, goal_control) write to the shared namespace via
// `shared_memory_agent_id()`. LLM-facing tools use per-agent scoping
// (`agent_id: Some(caller_uuid)`). The `None` fallback exists for backward
// compatibility and internal kernel callers, not for agent tools.
// ============================================================================

pub trait MemoryAccess: Send + Sync {
    /// Store a value in the agent's memory.
    /// When `agent_id` is `Some`, the key is scoped to that agent so each agent
    /// gets its own isolated memory namespace.
    /// When `None`, uses the shared memory namespace (backward compatible;
    /// internal kernel subsystems use this, LLM-facing tools do not).
    /// When `peer_id` is `Some`, the key is further scoped to that peer.
    fn memory_store(
        &self,
        key: &str,
        value: serde_json::Value,
        agent_id: Option<&str>,
        peer_id: Option<&str>,
    ) -> Result<(), KernelOpError>;

    /// Recall a value from the agent's memory.
    /// When `agent_id` is `Some`, only returns values stored under that agent's namespace.
    /// When `None`, uses the shared memory namespace (backward compatible;
    /// internal kernel subsystems use this, LLM-facing tools do not).
    /// When `peer_id` is `Some`, only returns values stored under that peer's namespace.
    fn memory_recall(
        &self,
        key: &str,
        agent_id: Option<&str>,
        peer_id: Option<&str>,
    ) -> Result<Option<serde_json::Value>, KernelOpError>;

    /// List all keys in the agent's memory.
    /// When `agent_id` is `Some`, only returns keys within that agent's namespace.
    /// When `None`, uses the shared memory namespace (backward compatible;
    /// internal kernel subsystems use this, LLM-facing tools do not).
    /// When `peer_id` is `Some`, only returns keys within that peer's namespace.
    fn memory_list(
        &self,
        agent_id: Option<&str>,
        peer_id: Option<&str>,
    ) -> Result<Vec<String>, KernelOpError>;

    /// Resolve the per-user memory ACL for the given sender + channel
    /// pair (RBAC M3, #3054 Phase 2). Returns the resolved
    /// `UserMemoryAccess` so the runtime can build a
    /// `MemoryNamespaceGuard` and gate proactive-memory reads.
    ///
    /// `None` means RBAC is disabled (no registered users) or the sender
    /// could not be attributed to any registered user — callers should
    /// treat this as "no per-user restriction" so the existing single-user
    /// behaviour is preserved.
    ///
    /// Default impl returns `None` so embedders / stubs that haven't
    /// wired RBAC keep the pre-M3 behaviour.
    fn memory_acl_for_sender(
        &self,
        sender_id: Option<&str>,
        channel: Option<&str>,
    ) -> Option<librefang_types::user_policy::UserMemoryAccess> {
        let _ = (sender_id, channel);
        None
    }
}
