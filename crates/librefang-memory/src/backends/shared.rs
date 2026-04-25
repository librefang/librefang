//! Single-source [`surreal_memory::SurrealStorage`] factory shared by both
//! Surreal-backed memory backends.
//!
//! ## Why this exists
//!
//! `SurrealStorage::new()` internally invokes `surrealdb::engine::any::connect`,
//! which (in embedded RocksDB mode) acquires an **exclusive** OS-level file
//! lock on the underlying RocksDB directory.  RocksDB only permits one opener
//! per process per path.
//!
//! Prior to this module the kernel boot sequence built two independent
//! `SurrealStorage` instances back-to-back — one for the proactive memory
//! backend and one for the semantic backend — each issuing its own
//! `connect("rocksdb://librefang-memory.surreal")` call.  The second call lost
//! the lock race and the kernel failed to boot.
//!
//! All `SurrealStorage::new()` calls in the workspace are now funnelled
//! through [`open_shared_memory_storage`].  The kernel calls it exactly once
//! at boot and shares the resulting `Arc<SurrealStorage>` between both
//! backends; the legacy `open_with_storage` factories on each backend type
//! also delegate here so standalone (test-only) callers continue to work.

#[cfg(feature = "surreal-backend")]
use std::sync::Arc;

#[cfg(feature = "surreal-backend")]
struct NoopEmbedding;

#[cfg(feature = "surreal-backend")]
#[async_trait::async_trait]
impl surreal_memory::EmbeddingService for NoopEmbedding {
    async fn embed(&self, _text: &str) -> anyhow::Result<surreal_memory::embeddings::Embedding> {
        Ok(vec![])
    }
    async fn embed_batch(
        &self,
        texts: Vec<String>,
    ) -> anyhow::Result<Vec<surreal_memory::embeddings::Embedding>> {
        Ok(texts.iter().map(|_| vec![]).collect())
    }
    fn dimensions(&self) -> usize {
        0
    }
}

/// Build a single shared [`surreal_memory::SurrealStorage`] for use by both
/// the proactive and semantic memory backends.
///
/// The returned `Arc<SurrealStorage>` owns the embedded RocksDB file lock for
/// `librefang-memory.surreal` (in embedded mode) or the WebSocket session to
/// the `memory` database (in remote mode).  Callers must keep the `Arc` alive
/// for the lifetime of any backend that uses it.
///
/// Routes via [`librefang_storage::config::StorageConfig::memory_storage_config`]
/// so the operational store on `librefang.surreal` (owned by
/// `SurrealConnectionPool`) is never touched.
///
/// The internal embedding service is a [`NoopEmbedding`] — real embeddings
/// are supplied at query time by the `ContextEngine`'s `EmbeddingDriver` via
/// `SemanticBackend::recall`'s `query_embedding` parameter.
#[cfg(feature = "surreal-backend")]
pub async fn open_shared_memory_storage(
    storage_cfg: &librefang_storage::config::StorageConfig,
) -> Result<Arc<surreal_memory::SurrealStorage>, String> {
    use surreal_memory::storage::surreal::{SurrealConfig, SurrealMode};

    let mem_cfg = storage_cfg.memory_storage_config();
    let sm_config = match &mem_cfg.backend {
        librefang_storage::config::StorageBackendKind::Embedded { path } => SurrealConfig {
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
            let password = std::env::var(&remote.password_env).unwrap_or_default();
            SurrealConfig {
                mode: SurrealMode::Server,
                endpoint: Some(remote.url.clone()),
                embedded_path: None,
                username: if remote.username.is_empty() {
                    None
                } else {
                    Some(remote.username.clone())
                },
                password: if password.is_empty() {
                    None
                } else {
                    Some(password)
                },
                namespace: remote.namespace.clone(),
                database: remote.database.clone(),
                retry: surreal_memory::RetryConfig::default(),
            }
        }
    };

    let storage = surreal_memory::SurrealStorage::new(&sm_config, Arc::new(NoopEmbedding))
        .await
        .map_err(|e| format!("shared memory SurrealStorage: {e}"))?;
    Ok(Arc::new(storage))
}
