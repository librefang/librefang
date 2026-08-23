use super::*;
use async_trait::async_trait;

// ============================================================================
// 2. MemoryAccess — per-agent key/value memory + per-user RBAC ACL resolution
//
// DESIGN NOTE: Internal kernel subsystems (messaging, agent_execution,
// prompt_context, goal_control) write to the shared namespace via
// `shared_memory_agent_id()`. LLM-facing tools use per-agent scoping
// (`agent_id: Some(caller_uuid)`). The `None` fallback exists for backward
// compatibility and internal kernel callers, not for agent tools.
// ============================================================================

#[async_trait]
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

    // ------------------------------------------------------------------
    // Semantic (vector) memory — the `memories` table, NOT `kv_store` (#7808)
    // ------------------------------------------------------------------
    //
    // The three methods above are exact-key KV against `kv_store`; nothing on
    // this trait used to reach the embedding-backed `memories` table, so no
    // agent tool could reach semantic memory even in principle.
    // The methods below close that seam.
    //
    // Each takes `sender_id` / `channel` rather than a resolved ACL because the
    // kernel — not the runtime — owns the `UserMemoryAccess` resolver, and the
    // `ProactiveMemoryStore::*_with_guard` wrappers it forwards to also apply
    // PII redaction and the `delete_allowed` flag.
    // Resolving the guard in the runtime instead (the shape `enforce_memory_acl`
    // uses for the KV tools) would give namespace gating but silently skip
    // redaction, handing PII-tagged fragments to an LLM for a user whose policy
    // forbids it.
    //
    // Default impls return `Unavailable` so every existing test stub and
    // embedder keeps compiling; the kernel overrides them only when the
    // proactive store is constructed.

    /// Semantic search over the agent's own memories, ranked by embedding
    /// cosine similarity when an embedding driver is configured and degrading
    /// to a `content LIKE` scan when it is not.
    ///
    /// `agent_id` must be the caller's own UUID — this is deliberately
    /// per-agent, never cross-agent.
    /// `min_confidence` drops fragments whose stored confidence has decayed
    /// below the floor; `None` keeps everything the ranker returned.
    async fn memory_semantic_search(
        &self,
        query: &str,
        agent_id: &str,
        limit: usize,
        min_confidence: Option<f32>,
        sender_id: Option<&str>,
        channel: Option<&str>,
    ) -> KernelResult<Vec<librefang_types::memory::MemoryItem>> {
        let _ = (query, agent_id, limit, min_confidence, sender_id, channel);
        Err(KernelOpError::unavailable("memory_semantic_search"))
    }

    /// Deliberately record `content` in the agent's semantic memory.
    ///
    /// Routes through the same extraction pipeline as `POST /api/memory`, so
    /// the extractor may distil `content` into one or more facts, merge it into
    /// an existing memory, or decline to store it at all.
    /// The returned vector is exactly what was persisted — an empty vector is a
    /// truthful "nothing was stored", not a silent success.
    async fn memory_semantic_add(
        &self,
        content: &str,
        agent_id: &str,
        sender_id: Option<&str>,
        channel: Option<&str>,
    ) -> KernelResult<Vec<librefang_types::memory::MemoryItem>> {
        let _ = (content, agent_id, sender_id, channel);
        Err(KernelOpError::unavailable("memory_semantic_add"))
    }

    /// Retract one semantic memory by id. `Ok(false)` means no such memory
    /// belonged to `agent_id`.
    ///
    /// Gated on the user policy's `delete_allowed` flag in addition to write
    /// access on the `proactive` namespace.
    async fn memory_semantic_forget(
        &self,
        memory_id: &str,
        agent_id: &str,
        sender_id: Option<&str>,
        channel: Option<&str>,
    ) -> KernelResult<bool> {
        let _ = (memory_id, agent_id, sender_id, channel);
        Err(KernelOpError::unavailable("memory_semantic_forget"))
    }

    /// Counts and subsystem flags for the agent's own semantic memory.
    ///
    /// Returns a JSON object; `categories` is a key-sorted object so the
    /// rendered tool result is byte-identical across processes (#3298) — a
    /// `HashMap` here would reorder the tool result between turns and
    /// invalidate the provider prompt cache for the rest of the conversation.
    async fn memory_semantic_stats(
        &self,
        agent_id: &str,
        sender_id: Option<&str>,
        channel: Option<&str>,
    ) -> KernelResult<serde_json::Value> {
        let _ = (agent_id, sender_id, channel);
        Err(KernelOpError::unavailable("memory_semantic_stats"))
    }
}
