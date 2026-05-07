//! LLM subsystem — driver registry, model catalog, embedding fallback.
//!
//! Bundles every LLM-side handle that previously sat as a flat field on
//! `LibreFangKernel`:
//!   * `default_driver` — concrete primary driver from the kernel config.
//!   * `aux_client` — `ArcSwap`-wrapped cheap-tier fallback chain
//!     (#3314).
//!   * `embedding_driver` — optional vector-similarity backend.
//!   * `driver_cache` — lazy per-(provider, key, url) HTTP client cache.
//!   * `model_catalog` — `ArcSwap` model registry (#3384) read on the
//!     hot path.
//!   * `default_model_override` — hot-reloadable overlay applied at
//!     agent spawn.

use std::sync::{Arc, RwLock};

use arc_swap::ArcSwap;
use librefang_llm_driver::LlmDriver;
use librefang_runtime::aux_client::AuxClient;
use librefang_runtime::drivers::DriverCache;
use librefang_runtime::embedding::EmbeddingDriver;
use librefang_runtime::model_catalog::ModelCatalog;
use librefang_types::config::DefaultModelConfig;

/// Focused LLM API. Generic mutators (`catalog_update`) stay as
/// inherent methods on `LlmSubsystem`.
pub trait LlmSubsystemApi: Send + Sync {
    /// `ArcSwap`-backed model catalog handle.
    fn catalog_swap(&self) -> &ArcSwap<ModelCatalog>;
    /// Cheap atomic snapshot of the model catalog.
    fn catalog_load(&self) -> arc_swap::Guard<Arc<ModelCatalog>>;
    /// Drop every cached driver client.
    fn clear_driver_cache(&self);
    /// Optional embedding driver handle.
    fn embedding(&self) -> Option<&Arc<dyn EmbeddingDriver + Send + Sync>>;
    /// Default-model override lock.
    fn default_model_override_ref(&self) -> &RwLock<Option<DefaultModelConfig>>;
}

/// LLM driver + model-catalog cluster — see module docs.
pub struct LlmSubsystem {
    /// Default LLM driver (from kernel config).
    pub(crate) default_driver: Arc<dyn LlmDriver>,
    /// Auxiliary LLM client — resolves cheap-tier fallback chains for
    /// side tasks (compression, titles, search summarisation, vision
    /// captioning). Wrapped in `ArcSwap` so config hot-reload can rebuild
    /// the chains without restarting the kernel. See issue #3314 and
    /// `librefang_runtime::aux_client`.
    pub(crate) aux_client: ArcSwap<AuxClient>,
    /// Embedding driver for vector similarity search (None = text
    /// fallback).
    pub(crate) embedding_driver: Option<Arc<dyn EmbeddingDriver + Send + Sync>>,
    /// Lazy-loading driver cache — avoids recreating HTTP clients for
    /// the same provider/key/url combination on every agent message.
    pub(crate) driver_cache: DriverCache,
    /// Model catalog registry. `ArcSwap` (#3384) so the hot
    /// `send_message_full` path can read the snapshot atomically — was
    /// previously `std::sync::RwLock`, which forced 5+ lock acquisitions
    /// per request. Writes use the RCU pattern (`model_catalog_update`).
    pub(crate) model_catalog: ArcSwap<ModelCatalog>,
    /// Hot-reloadable default model override (set via config hot-reload,
    /// read at agent spawn).
    pub(crate) default_model_override: RwLock<Option<DefaultModelConfig>>,
}

impl LlmSubsystem {
    pub(crate) fn new(
        default_driver: Arc<dyn LlmDriver>,
        aux_client: AuxClient,
        embedding_driver: Option<Arc<dyn EmbeddingDriver + Send + Sync>>,
        model_catalog: ModelCatalog,
    ) -> Self {
        Self {
            default_driver,
            aux_client: ArcSwap::from_pointee(aux_client),
            embedding_driver,
            driver_cache: DriverCache::new(),
            model_catalog: ArcSwap::from_pointee(model_catalog),
            default_model_override: RwLock::new(None),
        }
    }

    /// Atomically mutate the model catalog using the RCU pattern. The
    /// closure may run more than once under contention; the last
    /// return value is yielded to the caller.
    pub fn catalog_update<F, R>(&self, mut f: F) -> R
    where
        F: FnMut(&mut ModelCatalog) -> R,
    {
        let mut result: Option<R> = None;
        self.model_catalog.rcu(|cat| {
            let mut next = (**cat).clone();
            result = Some(f(&mut next));
            Arc::new(next)
        });
        result.expect("rcu closure runs at least once")
    }
}

impl LlmSubsystemApi for LlmSubsystem {
    #[inline]
    fn catalog_swap(&self) -> &ArcSwap<ModelCatalog> {
        &self.model_catalog
    }

    #[inline]
    fn catalog_load(&self) -> arc_swap::Guard<Arc<ModelCatalog>> {
        self.model_catalog.load()
    }

    #[inline]
    fn clear_driver_cache(&self) {
        self.driver_cache.clear();
    }

    #[inline]
    fn embedding(&self) -> Option<&Arc<dyn EmbeddingDriver + Send + Sync>> {
        self.embedding_driver.as_ref()
    }

    #[inline]
    fn default_model_override_ref(&self) -> &RwLock<Option<DefaultModelConfig>> {
        &self.default_model_override
    }
}
