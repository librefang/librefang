use async_trait::async_trait;

use super::*;

// ============================================================================
// 5. KnowledgeGraph — entity/relation insert + pattern query
// ============================================================================

#[async_trait]
pub trait KnowledgeGraph: Send + Sync {
    /// Add an entity to the knowledge graph.
    ///
    /// Takes `entity` by reference so callers that already hold an owned
    /// value (e.g. proactive memory extractors that may retry the call)
    /// avoid forced moves and downstream `.clone()` chains. The kernel
    /// implementation clones into the underlying store when it actually
    /// needs ownership; total clone count is unchanged but the choice
    /// moves from caller to callee. See issue #3553.
    /// `peer_id` scopes the entity to a single user on a multi-user agent
    /// (#6494); `None` writes a shared/unscoped entity.
    /// `agent_id` scopes the entity to its owning agent so the agent-scoped relations read and `delete_by_agent` see it (the empty string is the shared/unscoped sentinel).
    /// Callers thread the turn's `caller_agent_id`.
    async fn knowledge_add_entity(
        &self,
        entity: &librefang_types::memory::Entity,
        agent_id: &str,
        peer_id: Option<&str>,
    ) -> Result<String, KernelOpError>;

    /// Add a relation to the knowledge graph.
    ///
    /// Takes `relation` by reference for the same reason as
    /// [`knowledge_add_entity`](Self::knowledge_add_entity). See #3553.
    /// `peer_id` scopes the relation to a single user (#6494); `None` writes a
    /// shared/unscoped relation.
    /// `agent_id` scopes the relation to its owning agent (see [`knowledge_add_entity`](Self::knowledge_add_entity)).
    async fn knowledge_add_relation(
        &self,
        relation: &librefang_types::memory::Relation,
        agent_id: &str,
        peer_id: Option<&str>,
    ) -> Result<String, KernelOpError>;

    /// Query the knowledge graph with a pattern, optionally scoped to a user.
    ///
    /// `peer_id` restricts the read to that user's triples (#6494); `None` is
    /// an unscoped read returning every peer's rows.
    async fn knowledge_query(
        &self,
        pattern: librefang_types::memory::GraphPattern,
        peer_id: Option<&str>,
    ) -> Result<Vec<librefang_types::memory::GraphMatch>, KernelOpError>;
}
