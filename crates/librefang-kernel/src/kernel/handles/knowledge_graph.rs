//! [`kernel_handle::KnowledgeGraph`] — entity / relation insert and graph
//! pattern query against the substrate's knowledge-graph store. Pure
//! delegation; the substrate owns the values so the trait takes refs and
//! we clone here to keep the call sites simple (#3553).

use librefang_runtime::kernel_handle;
use librefang_types::memory::Memory;

use super::super::LibreFangKernel;

#[async_trait::async_trait]
impl kernel_handle::KnowledgeGraph for LibreFangKernel {
    async fn knowledge_add_entity(
        &self,
        entity: &librefang_types::memory::Entity,
    ) -> Result<String, kernel_handle::KernelOpError> {
        // The substrate owns the value (it moves into spawn_blocking).
        // Clone here so the trait can take `&Entity` and avoid forcing
        // every caller to give up ownership. See #3553.
        self.memory
            .substrate
            .add_entity(entity.clone())
            .await
            .map_err(|e| {
                kernel_handle::KernelOpError::Internal(format!("Knowledge add entity failed: {e}"))
            })
    }

    async fn knowledge_add_relation(
        &self,
        relation: &librefang_types::memory::Relation,
    ) -> Result<String, kernel_handle::KernelOpError> {
        self.memory
            .substrate
            .add_relation(relation.clone())
            .await
            .map_err(|e| {
                kernel_handle::KernelOpError::Internal(format!(
                    "Knowledge add relation failed: {e}"
                ))
            })
    }

    async fn knowledge_query(
        &self,
        pattern: librefang_types::memory::GraphPattern,
    ) -> Result<Vec<librefang_types::memory::GraphMatch>, kernel_handle::KernelOpError> {
        self.memory
            .substrate
            .query_graph(pattern)
            .await
            .map_err(|e| {
                kernel_handle::KernelOpError::Internal(format!("Knowledge query failed: {e}"))
            })
    }
}
