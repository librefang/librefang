//! SurrealDB-backed [`crate::ProactiveMemoryBackend`] implementation.
//!
//! Provides decay (TTL-based eviction), consolidation, and vacuum operations
//! against the SurrealDB `memories` table managed by the `surreal-memory`
//! library migrations.
//!
//! ## Design notes
//!
//! - `run_decay` deletes memory rows whose `expires_at` field is in the past,
//!   mapping the SQLite TTL semantics onto a SurrealQL DELETE WHERE query.
//! - `consolidate` is a best-effort no-op when no embedding service is
//!   attached; it returns an empty `ConsolidationReport` instead of failing.
//!   Full LLM-assisted consolidation requires a `SurrealStorage` with an
//!   embedding service, which the kernel wires separately via
//!   `SurrealMemoryBackend::with_extended`.
//! - `vacuum_if_shrank` is always a no-op — SurrealDB manages its own
//!   compaction without the caller needing to trigger it.

use crate::backend::ProactiveMemoryBackend;
use async_trait::async_trait;
use librefang_storage::pool::SurrealSession;
use librefang_types::config::MemoryDecayConfig;
use librefang_types::error::{LibreFangError, LibreFangResult};
use librefang_types::memory::ConsolidationReport;
use serde_json::Value as JsonValue;
use std::sync::Arc;
use surrealdb::{engine::any::Any, Surreal};

/// SurrealDB-backed implementation of [`ProactiveMemoryBackend`].
pub struct SurrealProactiveMemoryBackend {
    db: Arc<Surreal<Any>>,
    /// Optional full SurrealStorage for LLM-assisted consolidation.
    /// When `None`, consolidate() returns an empty report.
    #[cfg(feature = "surreal-backend")]
    extended: Option<Arc<surreal_memory::SurrealStorage>>,
}

impl SurrealProactiveMemoryBackend {
    /// Open the proactive memory backend against an existing [`SurrealSession`].
    #[must_use]
    pub fn open(session: &SurrealSession) -> Self {
        Self {
            db: Arc::new(session.client().clone()),
            #[cfg(feature = "surreal-backend")]
            extended: None,
        }
    }

    /// Attach a full `SurrealStorage` for LLM-assisted consolidation.
    #[cfg(feature = "surreal-backend")]
    #[must_use]
    pub fn with_extended(mut self, storage: Arc<surreal_memory::SurrealStorage>) -> Self {
        self.extended = Some(storage);
        self
    }

    /// Open the proactive memory backend and wire a [`surreal_memory::SurrealStorage`]
    /// built from `storage_cfg` so that `consolidate()` → `expire_stale_memories()` is
    /// fully active.  The `SurrealStorage` uses a noop embedding service because
    /// consolidation only needs TTL eviction, not vector search.
    ///
    /// Call sites in `librefang-kernel` use this instead of calling
    /// `SurrealStorage::new()` directly (which requires `surreal-memory` to be a
    /// direct dependency of the kernel crate).
    #[cfg(feature = "surreal-backend")]
    pub async fn open_with_storage(
        session: &SurrealSession,
        storage_cfg: &librefang_storage::config::StorageConfig,
    ) -> Result<Self, String> {
        use surreal_memory::storage::surreal::{SurrealConfig, SurrealMode};

        struct NoopEmbedding;
        #[async_trait::async_trait]
        impl surreal_memory::EmbeddingService for NoopEmbedding {
            async fn embed(
                &self,
                _text: &str,
            ) -> anyhow::Result<surreal_memory::embeddings::Embedding> {
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

        let sm_config = match &storage_cfg.backend {
            librefang_storage::config::StorageBackendKind::Embedded { path } => SurrealConfig {
                mode: SurrealMode::Embedded,
                endpoint: None,
                embedded_path: Some(path.to_string_lossy().to_string()),
                username: None,
                password: None,
                namespace: storage_cfg.effective_namespace().to_string(),
                database: storage_cfg.effective_database().to_string(),
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
            .map_err(|e| format!("SurrealStorage (proactive memory consolidation): {e}"))?;

        Ok(Self::open(session).with_extended(Arc::new(storage)))
    }
}

#[async_trait]
impl ProactiveMemoryBackend for SurrealProactiveMemoryBackend {
    fn run_decay(&self, config: &MemoryDecayConfig) -> LibreFangResult<usize> {
        if !config.enabled {
            return Ok(0);
        }

        let db = self.db.clone();
        let now = chrono::Utc::now().to_rfc3339();

        // Use a block_on bridge: run_decay is a sync trait method.
        let result = tokio::runtime::Handle::try_current()
            .map(|h| tokio::task::block_in_place(|| h.block_on(do_decay(db.clone(), now.clone()))))
            .unwrap_or_else(|_| {
                tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("temporary runtime")
                    .block_on(do_decay(db, now))
            });
        result
    }

    async fn consolidate(&self) -> LibreFangResult<ConsolidationReport> {
        #[cfg(feature = "surreal-backend")]
        if let Some(ref storage) = self.extended {
            use surreal_memory::MemoryStorage;
            let expired = storage
                .expire_stale_memories()
                .await
                .map_err(|e| LibreFangError::Memory(e.to_string()))?;
            return Ok(ConsolidationReport {
                memories_merged: 0,
                memories_decayed: expired,
                duration_ms: 0,
            });
        }

        // No extended storage — return a no-op report.
        Ok(ConsolidationReport {
            memories_merged: 0,
            memories_decayed: 0,
            duration_ms: 0,
        })
    }

    fn vacuum_if_shrank(&self, _pruned_count: usize) -> LibreFangResult<()> {
        // SurrealDB manages its own compaction; no manual vacuum needed.
        Ok(())
    }
}

/// Async helper: delete rows from `memories` whose `expires_at` is past.
async fn do_decay(db: Arc<Surreal<Any>>, now: String) -> LibreFangResult<usize> {
    let rows: Vec<JsonValue> = db
        .query("DELETE memories WHERE expires_at != NONE AND expires_at < $now RETURN id")
        .bind(("now", now))
        .await
        .map_err(|e| LibreFangError::Memory(e.to_string()))?
        .take(0)
        .map_err(|e| LibreFangError::Memory(e.to_string()))?;
    Ok(rows.len())
}
