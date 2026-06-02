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
    async fn knowledge_add_entity(
        &self,
        entity: &librefang_types::memory::Entity,
    ) -> Result<String, KernelOpError>;

    /// Add a relation to the knowledge graph.
    ///
    /// Takes `relation` by reference for the same reason as
    /// [`knowledge_add_entity`](Self::knowledge_add_entity). See #3553.
    async fn knowledge_add_relation(
        &self,
        relation: &librefang_types::memory::Relation,
    ) -> Result<String, KernelOpError>;

    /// Query the knowledge graph with a pattern.
    async fn knowledge_query(
        &self,
        pattern: librefang_types::memory::GraphPattern,
    ) -> Result<Vec<librefang_types::memory::GraphMatch>, KernelOpError>;
}
