//! Memory Provider plugin system for LibreFang.
//!
//! Mirrors the Python `MemoryManager` from Hermes-Agent:
//! - A built-in provider is always present and cannot be removed.
//! - At most **one** external (non-builtin) provider may be registered at a
//!   time; a second registration is rejected with an error.
//! - Failures in any single provider are isolated: they are logged at `warn`
//!   level and do not propagate to callers or block other providers.
//!
//! # Example
//!
//! ```rust
//! use std::sync::Arc;
//! use librefang_memory::provider::{MemoryManager, MemoryProvider, NullMemoryProvider};
//!
//! let builtin = Arc::new(NullMemoryProvider::new("builtin", true));
//! let manager = MemoryManager::new(builtin);
//!
//! // Register an external provider (only one allowed)
//! let external = Arc::new(NullMemoryProvider::new("vector-db", false));
//! manager.register_external(external).unwrap();
//!
//! // Attempt to register a second external provider is rejected
//! let another = Arc::new(NullMemoryProvider::new("another", false));
//! assert!(manager.register_external(another).is_err());
//! ```

use async_trait::async_trait;
use std::sync::{Arc, RwLock};
use thiserror::Error;
use tracing::warn;

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

/// Errors that can be returned by a [`MemoryProvider`] or [`MemoryManager`].
#[derive(Debug, Error)]
pub enum MemoryError {
    /// A provider already has an external provider registered.
    #[error("External memory provider '{existing}' is already registered; rejected '{rejected}'")]
    ExternalProviderAlreadyRegistered {
        /// Name of the already-registered provider.
        existing: String,
        /// Name of the provider that was rejected.
        rejected: String,
    },

    /// A provider-level operation failed.
    #[error("Memory provider '{provider}' error: {reason}")]
    ProviderError {
        /// The provider that raised the error.
        provider: String,
        /// Human-readable description of the failure.
        reason: String,
    },
}

impl MemoryError {
    /// Convenience constructor for a provider-level error.
    pub fn provider(provider: impl Into<String>, reason: impl Into<String>) -> Self {
        Self::ProviderError {
            provider: provider.into(),
            reason: reason.into(),
        }
    }
}

// ---------------------------------------------------------------------------
// MemoryProvider trait
// ---------------------------------------------------------------------------

/// A pluggable memory backend that participates in the agent turn lifecycle.
///
/// Implementors must be `Send + Sync` so they can be held behind `Arc` and
/// called from async contexts on any thread.
#[async_trait]
pub trait MemoryProvider: Send + Sync {
    /// A short, stable identifier for this provider (e.g. `"builtin"`, `"qdrant"`).
    fn name(&self) -> &str;

    /// Returns `true` for the single built-in provider that is always present.
    ///
    /// Defaults to `false` (i.e. external) for custom implementations.
    fn is_builtin(&self) -> bool {
        false
    }

    /// Returns a block of text to inject into the system prompt for the
    /// current session, or `None` if this provider has nothing to contribute.
    async fn system_prompt_block(&self, session_id: &str) -> Option<String>;

    /// Prefetch relevant context for `query` prior to an agent turn.
    ///
    /// On success returns a (possibly empty) text snippet to inject into the
    /// conversation context.  On failure returns a [`MemoryError`]; the
    /// [`MemoryManager`] will log the error at `warn` level and continue with
    /// the remaining providers.
    async fn prefetch(&self, query: &str, session_id: &str) -> Result<String, MemoryError>;

    /// Called after an agent turn completes so the provider can index or sync
    /// the turn summary.
    ///
    /// Failures are non-fatal from the manager's perspective.
    async fn on_turn_complete(
        &self,
        session_id: &str,
        turn_summary: &str,
    ) -> Result<(), MemoryError>;
}

// ---------------------------------------------------------------------------
// MemoryManager
// ---------------------------------------------------------------------------

/// Orchestrates the built-in memory provider plus at most one external plugin
/// provider.
///
/// * The **built-in** provider is injected at construction and cannot be
///   replaced.
/// * Only **one** external provider may be registered; [`register_external`]
///   returns [`MemoryError::ExternalProviderAlreadyRegistered`] if called a
///   second time.
/// * All multi-provider operations apply *error isolation*: a failure in one
///   provider is logged at `warn` level and does not affect the result from
///   other providers.
/// * The external slot is guarded by a `RwLock`, so `register_external` and
///   `remove_external` can be called through a shared `Arc<MemoryManager>`
///   without requiring `&mut self`.
///
/// [`register_external`]: MemoryManager::register_external
pub struct MemoryManager {
    builtin: Arc<dyn MemoryProvider>,
    /// RwLock allows hot-swap of the external provider through `Arc<MemoryManager>`.
    external: RwLock<Option<Arc<dyn MemoryProvider>>>,
}

impl MemoryManager {
    /// Create a new manager with the given built-in provider.
    pub fn new(builtin: Arc<dyn MemoryProvider>) -> Self {
        Self {
            builtin,
            external: RwLock::new(None),
        }
    }

    /// Register an external (non-builtin) provider.
    ///
    /// Returns `Err` if an external provider is already registered.
    ///
    /// This method takes `&self` so it can be called through `Arc<MemoryManager>`.
    pub fn register_external(&self, provider: Arc<dyn MemoryProvider>) -> Result<(), MemoryError> {
        let mut slot = self
            .external
            .write()
            .expect("MemoryManager external lock poisoned");
        if let Some(existing) = slot.as_ref() {
            return Err(MemoryError::ExternalProviderAlreadyRegistered {
                existing: existing.name().to_owned(),
                rejected: provider.name().to_owned(),
            });
        }
        *slot = Some(provider);
        Ok(())
    }

    /// Remove the current external provider, if any.
    ///
    /// Returns the removed provider so the caller can perform any teardown.
    ///
    /// This method takes `&self` so it can be called through `Arc<MemoryManager>`.
    pub fn remove_external(&self) -> Option<Arc<dyn MemoryProvider>> {
        self.external
            .write()
            .expect("MemoryManager external lock poisoned")
            .take()
    }

    /// Returns a clone of the external provider `Arc`, if one is registered.
    pub fn external(&self) -> Option<Arc<dyn MemoryProvider>> {
        self.external
            .read()
            .expect("MemoryManager external lock poisoned")
            .clone()
    }

    // -- Multi-provider helpers ---------------------------------------------

    /// Snapshot all providers: builtin first, then external (if present).
    ///
    /// Returns owned `Arc`s so callers can `.await` without holding the lock.
    fn snapshot_providers(&self) -> Vec<Arc<dyn MemoryProvider>> {
        let mut providers: Vec<Arc<dyn MemoryProvider>> = vec![Arc::clone(&self.builtin)];
        if let Some(ext) = self.external() {
            providers.push(ext);
        }
        providers
    }

    // -- Public async API ---------------------------------------------------

    /// Collect system-prompt blocks from every provider.
    ///
    /// Providers that return `None` are skipped.
    /// Providers that return an empty string after trimming are skipped.
    pub async fn collect_system_blocks(&self, session_id: &str) -> Vec<String> {
        let mut blocks = Vec::new();
        for provider in self.snapshot_providers() {
            match provider.system_prompt_block(session_id).await {
                Some(block) if !block.trim().is_empty() => {
                    blocks.push(block);
                }
                Some(_) => {} // empty block — skip
                None => {}    // provider has nothing to contribute
            }
        }
        blocks
    }

    /// Prefetch context from every provider and merge the results.
    ///
    /// Providers that fail are logged at `warn` level and skipped; their
    /// failure does not affect the output from other providers.
    ///
    /// Returns merged context text (non-empty provider results joined with
    /// `"\n\n"`), or an empty string if no provider returns content.
    pub async fn prefetch_all(&self, query: &str, session_id: &str) -> String {
        let mut parts: Vec<String> = Vec::new();
        for provider in self.snapshot_providers() {
            match provider.prefetch(query, session_id).await {
                Ok(result) if !result.trim().is_empty() => {
                    parts.push(result);
                }
                Ok(_) => {} // empty result — skip
                Err(err) => {
                    warn!(
                        provider = provider.name(),
                        error = %err,
                        "Memory provider prefetch failed (non-fatal)"
                    );
                }
            }
        }
        parts.join("\n\n")
    }

    /// Notify all providers that an agent turn has completed.
    ///
    /// Failures are logged at `warn` level but do not propagate.
    pub async fn notify_turn_complete(&self, session_id: &str, turn_summary: &str) {
        for provider in self.snapshot_providers() {
            if let Err(err) = provider.on_turn_complete(session_id, turn_summary).await {
                warn!(
                    provider = provider.name(),
                    error = %err,
                    "Memory provider on_turn_complete failed (non-fatal)"
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------
// NullMemoryProvider — no-op implementation for testing
// ---------------------------------------------------------------------------

/// A no-op [`MemoryProvider`] that always returns empty results.
///
/// Useful as a placeholder in tests or when a provider slot is intentionally
/// left empty.
pub struct NullMemoryProvider {
    name: String,
    builtin: bool,
}

impl NullMemoryProvider {
    /// Create a null provider with the given name.
    ///
    /// Set `builtin` to `true` when using this as the mandatory built-in slot.
    pub fn new(name: impl Into<String>, builtin: bool) -> Self {
        Self {
            name: name.into(),
            builtin,
        }
    }
}

#[async_trait]
impl MemoryProvider for NullMemoryProvider {
    fn name(&self) -> &str {
        &self.name
    }

    fn is_builtin(&self) -> bool {
        self.builtin
    }

    async fn system_prompt_block(&self, _session_id: &str) -> Option<String> {
        None
    }

    async fn prefetch(&self, _query: &str, _session_id: &str) -> Result<String, MemoryError> {
        Ok(String::new())
    }

    async fn on_turn_complete(
        &self,
        _session_id: &str,
        _turn_summary: &str,
    ) -> Result<(), MemoryError> {
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn null_builtin() -> Arc<dyn MemoryProvider> {
        Arc::new(NullMemoryProvider::new("builtin", true))
    }

    fn null_external(name: &str) -> Arc<dyn MemoryProvider> {
        Arc::new(NullMemoryProvider::new(name, false))
    }

    #[test]
    fn register_external_once_succeeds() {
        let mgr = MemoryManager::new(null_builtin());
        mgr.register_external(null_external("ext1")).unwrap();
    }

    #[test]
    fn register_external_twice_fails() {
        let mgr = MemoryManager::new(null_builtin());
        mgr.register_external(null_external("ext1")).unwrap();
        let err = mgr.register_external(null_external("ext2")).unwrap_err();
        match err {
            MemoryError::ExternalProviderAlreadyRegistered { existing, rejected } => {
                assert_eq!(existing, "ext1");
                assert_eq!(rejected, "ext2");
            }
            other => panic!("unexpected error: {other}"),
        }
    }

    #[test]
    fn remove_external_clears_slot() {
        let mgr = MemoryManager::new(null_builtin());
        mgr.register_external(null_external("ext1")).unwrap();
        let removed = mgr.remove_external();
        assert!(removed.is_some());
        // Can register a new one after removal
        mgr.register_external(null_external("ext2")).unwrap();
    }

    #[test]
    fn register_and_remove_through_arc() {
        // Verify that hot-swap works through Arc<MemoryManager>
        let mgr = Arc::new(MemoryManager::new(null_builtin()));
        mgr.register_external(null_external("ext1")).unwrap();
        let removed = mgr.remove_external();
        assert!(removed.is_some());
        mgr.register_external(null_external("ext2")).unwrap();
    }

    #[tokio::test]
    async fn prefetch_all_returns_empty_for_null_providers() {
        let mgr = MemoryManager::new(null_builtin());
        mgr.register_external(null_external("ext1")).unwrap();
        let result = mgr.prefetch_all("test query", "session-1").await;
        assert!(result.is_empty());
    }

    #[tokio::test]
    async fn collect_system_blocks_returns_empty_for_null_providers() {
        let mgr = MemoryManager::new(null_builtin());
        let blocks = mgr.collect_system_blocks("session-1").await;
        assert!(blocks.is_empty());
    }

    /// A provider that always fails prefetch — used to verify error isolation.
    struct FailingProvider;

    #[async_trait]
    impl MemoryProvider for FailingProvider {
        fn name(&self) -> &str {
            "failing"
        }

        async fn system_prompt_block(&self, _session_id: &str) -> Option<String> {
            None
        }

        async fn prefetch(&self, _query: &str, _session_id: &str) -> Result<String, MemoryError> {
            Err(MemoryError::provider("failing", "simulated failure"))
        }

        async fn on_turn_complete(
            &self,
            _session_id: &str,
            _turn_summary: &str,
        ) -> Result<(), MemoryError> {
            Err(MemoryError::provider("failing", "simulated failure"))
        }
    }

    #[tokio::test]
    async fn prefetch_error_is_isolated() {
        let mgr = MemoryManager::new(null_builtin());
        mgr.register_external(Arc::new(FailingProvider)).unwrap();
        // Should not panic; failing provider's error is swallowed
        let result = mgr.prefetch_all("query", "sid").await;
        assert!(result.is_empty());
    }

    #[tokio::test]
    async fn notify_turn_complete_error_is_isolated() {
        let mgr = MemoryManager::new(null_builtin());
        mgr.register_external(Arc::new(FailingProvider)).unwrap();
        // Should not panic
        mgr.notify_turn_complete("sid", "summary").await;
    }
}
