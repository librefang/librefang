//! Single-source [`surreal_memory::SurrealStorage`] factory shared by both
//! Surreal-backed memory backends.
//!
//! ## Why this exists
//!
//! `SurrealStorage::new()` internally invokes `surrealdb::engine::any::connect`,
//! which (in embedded SurrealKV mode) acquires an **exclusive** OS-level file
//! lock on the underlying SurrealKV directory.  SurrealKV only permits one opener
//! per process per path.
//!
//! Prior to this module the kernel boot sequence built two independent
//! `SurrealStorage` instances back-to-back — one for the proactive memory
//! backend and one for the semantic backend — each issuing its own
//! `connect("surrealkv://librefang-memory.surreal")` call.  The second call lost
//! the lock race and the kernel failed to boot.
//!
//! All `SurrealStorage::new()` calls in the workspace are now funnelled
//! through [`open_shared_memory_storage`].
//! A caller that wires both backends opens it once and shares the resulting `Arc<SurrealStorage>` between them; the `open_with_storage` factories on each backend type also delegate here.
//! The kernel reaches it through [`crate::SurrealSemanticBackend::open_with_storage`] when `[memory] vector_backend` resolves to `"surreal"`.
//!
//! ## Embedding dimensions
//!
//! `SurrealStorage` embeds every memory it writes and every query it searches with the embedding service it was opened with, and sizes its HNSW indexes from that service's `dimensions()`.
//! The caller therefore supplies the same embedding driver the rest of the system uses, together with its dimension (`EmbeddingDriver::dimensions()`, which honours `[memory] embedding_dimensions` and otherwise infers it from `embedding_model`).
//! There is no embedding-less mode: a zero dimension is rejected before any connection is made.

#[cfg(feature = "surreal-backend")]
use std::sync::Arc;

#[cfg(feature = "surreal-backend")]
use crate::proactive::EmbeddingFn;

/// Build a [`librefang_types::error::LibreFangError::Memory`] that keeps the whole cause chain.
///
/// surreal-memory reports failures as `anyhow` chains whose outermost context is often generic ("add_memory failed") while the cause that names the defect sits at the root (a schema rejection, a parse error).
/// `to_string()` keeps only the outermost layer, so the message is rendered with `{:#}` and the error itself stays on the `source()` chain.
#[cfg(feature = "surreal-backend")]
pub(crate) fn memory_error(
    context: &str,
    error: impl Into<anyhow::Error>,
) -> librefang_types::error::LibreFangError {
    let error: anyhow::Error = error.into();
    librefang_types::error::LibreFangError::Memory {
        message: format!("{context}: {error:#}"),
        source: Some(error.into()),
    }
}

/// Adapts librefang's [`EmbeddingFn`] to surreal-memory's `EmbeddingService` with the dimension the caller declared for it.
#[cfg(feature = "surreal-backend")]
struct EmbeddingBridge {
    driver: Arc<dyn EmbeddingFn>,
    dimensions: usize,
}

#[cfg(feature = "surreal-backend")]
#[async_trait::async_trait]
impl surreal_memory::EmbeddingService for EmbeddingBridge {
    async fn embed(&self, text: &str) -> anyhow::Result<surreal_memory::embeddings::Embedding> {
        let embedding = self
            .driver
            .embed_one(text)
            .await
            .map_err(|e| anyhow::anyhow!("embedding driver failed: {e}"))?;
        // A vector of the wrong length would be rejected by the HNSW index, or worse, compared against vectors of another model; fail with the cause instead.
        if embedding.len() != self.dimensions {
            anyhow::bail!(
                "embedding driver returned a {}-dimensional vector, but the shared memory storage was opened for {} dimensions",
                embedding.len(),
                self.dimensions
            );
        }
        Ok(embedding)
    }

    async fn embed_batch(
        &self,
        texts: Vec<String>,
    ) -> anyhow::Result<Vec<surreal_memory::embeddings::Embedding>> {
        let mut embeddings = Vec::with_capacity(texts.len());
        for text in &texts {
            embeddings.push(self.embed(text).await?);
        }
        Ok(embeddings)
    }

    fn dimensions(&self) -> usize {
        self.dimensions
    }
}

/// Build a single shared [`surreal_memory::SurrealStorage`] for use by both
/// the proactive and semantic memory backends.
///
/// The returned `Arc<SurrealStorage>` owns the embedded SurrealKV file lock for
/// `librefang-memory.surreal` (in embedded mode) or the WebSocket session to
/// the `memory` database (in remote mode).  Callers must keep the `Arc` alive
/// for the lifetime of any backend that uses it.
///
/// Routes via [`librefang_storage::config::StorageConfig::memory_storage_config`]
/// so the operational store on `librefang.surreal` (owned by
/// `SurrealConnectionPool`) is never touched.
///
/// `embedding` is the embedding driver the storage uses for every write and search, and `dimensions` is its output dimension, which sizes the HNSW indexes.
/// Pass the driver the context engine uses and its `dimensions()`; surreal-memory rebuilds the indexes when the dimension changes on an empty store and refuses to open when stored vectors have a different dimension.
///
/// # Errors
///
/// Returns an error when `dimensions` is zero, or when the storage cannot be opened.
#[cfg(feature = "surreal-backend")]
pub async fn open_shared_memory_storage(
    storage_cfg: &librefang_storage::config::StorageConfig,
    embedding: Arc<dyn EmbeddingFn>,
    dimensions: usize,
) -> Result<Arc<surreal_memory::SurrealStorage>, String> {
    use surreal_memory::storage::surreal::{SurrealConfig, SurrealMode};
    use surreal_memory::SurrealAuthLevel;

    if dimensions == 0 {
        return Err(
            "shared memory SurrealStorage needs a non-zero embedding dimension; configure an embedding provider (`[memory] embedding_model` / `embedding_dimensions`)".to_string(),
        );
    }

    let mem_cfg = storage_cfg.memory_storage_config();
    let sm_config = match &mem_cfg.backend {
        librefang_storage::config::StorageBackendKind::Embedded { path } => SurrealConfig {
            // No credentials in embedded mode; Root is surreal-memory's own default.
            auth_level: SurrealAuthLevel::Root,
            mode: SurrealMode::Embedded,
            endpoint: None,
            embedded_path: Some(path.to_string_lossy().to_string()),
            username: None,
            password: None,
            namespace: mem_cfg.effective_namespace().to_string(),
            database: mem_cfg.effective_database().to_string(),
            retry: surreal_memory::RetryConfig::default(),
        },
        librefang_storage::config::StorageBackendKind::Remote(remote) => {
            // The operational pool refuses to connect without this credential; match it rather than opening an unauthenticated session that fails later on the first permission check.
            let password = std::env::var(&remote.password_env)
                .ok()
                .filter(|v| !v.is_empty())
                .ok_or_else(|| {
                    format!(
                        "shared memory SurrealStorage: missing credential env var `{}`",
                        remote.password_env
                    )
                })?;
            SurrealConfig {
                // Same level selection as the operational pool (`librefang_storage::pool`): the `root`
                // system user is defined ON ROOT and must sign in at root level, while any other user is
                // treated as namespace-scoped. Both stores read the same `[storage.backend]` credentials,
                // so they must agree or one of them fails to authenticate.
                auth_level: if remote.username == "root" {
                    SurrealAuthLevel::Root
                } else {
                    SurrealAuthLevel::Namespace
                },
                mode: SurrealMode::Server,
                endpoint: Some(remote.url.clone()),
                embedded_path: None,
                username: Some(remote.username.clone()),
                password: Some(password),
                namespace: remote.namespace.clone(),
                database: remote.database.clone(),
                retry: surreal_memory::RetryConfig::default(),
            }
        }
    };

    let storage = surreal_memory::SurrealStorage::new(
        &sm_config,
        Arc::new(EmbeddingBridge {
            driver: embedding,
            dimensions,
        }),
    )
    .await
    .map_err(|e| format!("shared memory SurrealStorage: {e:#}"))?;
    Ok(Arc::new(storage))
}

#[cfg(all(test, feature = "surreal-backend"))]
mod tests {
    use super::*;
    use librefang_types::error::LibreFangResult;
    use surreal_memory::EmbeddingService;

    struct FixedEmbedding(usize);

    #[async_trait::async_trait]
    impl EmbeddingFn for FixedEmbedding {
        async fn embed_one(&self, _text: &str) -> LibreFangResult<Vec<f32>> {
            Ok(vec![0.5; self.0])
        }
    }

    #[tokio::test]
    async fn bridge_reports_declared_dimension_and_passes_matching_vectors() {
        let bridge = EmbeddingBridge {
            driver: Arc::new(FixedEmbedding(4)),
            dimensions: 4,
        };
        assert_eq!(bridge.dimensions(), 4);
        assert_eq!(bridge.embed("x").await.unwrap().len(), 4);
        let batch = bridge
            .embed_batch(vec!["a".into(), "b".into()])
            .await
            .unwrap();
        assert_eq!(batch.len(), 2);
    }

    /// A driver whose output disagrees with the declared dimension would corrupt the HNSW index; the bridge must refuse the vector.
    #[tokio::test]
    async fn bridge_rejects_vectors_of_another_dimension() {
        let bridge = EmbeddingBridge {
            driver: Arc::new(FixedEmbedding(3)),
            dimensions: 4,
        };
        let err = bridge.embed("x").await.unwrap_err().to_string();
        assert!(err.contains("3-dimensional"), "{err}");
        assert!(err.contains("opened for 4 dimensions"), "{err}");
    }

    /// `to_string()` on an anyhow chain keeps only the outermost context; the helper must carry the root cause into the message and onto `source()`.
    #[test]
    fn memory_error_keeps_the_whole_cause_chain() {
        let root = anyhow::anyhow!("Found field 'metadata.librefang', but no such field exists");
        let err = memory_error("remember", root.context("add_memory failed"));
        let rendered = err.to_string();
        assert!(
            rendered.contains("remember: add_memory failed"),
            "{rendered}"
        );
        assert!(rendered.contains("metadata.librefang"), "{rendered}");
        assert!(std::error::Error::source(&err).is_some());
    }
}
