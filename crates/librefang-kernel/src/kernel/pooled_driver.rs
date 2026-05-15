//! PooledDriver — wraps an LLM driver with credential pool rotation.
//!
//! On each `complete()` / `stream()` call the wrapper acquires an API key from
//! the configured credential pool, builds (or reuses) the inner driver with
//! that key, and reports success / exhaustion back to the pool so the pool
//! can rotate to the next available key on error.

use async_trait::async_trait;
use librefang_llm_driver::{
    CompletionRequest, CompletionResponse, LlmDriver, LlmError, StreamEvent,
};
use librefang_llm_drivers::credential_pool::ArcCredentialPool;
use librefang_runtime::drivers::DriverCache;
use std::sync::Arc;

/// Driver wrapper that acquires a fresh API key from a [`CredentialPool`] on
/// every invocation and reports errors back to the pool for automatic key
/// rotation.
///
/// When all pool keys are exhausted the wrapper returns a 503-style error so
/// that a wrapping [`FallbackDriver`] can fall through to the next provider.
pub(crate) struct PooledDriver {
    pool: ArcCredentialPool,
    driver_cache: Arc<DriverCache>,
    /// Base driver config *without* the API key. Cloned and patched with the
    /// acquired key on each call.
    base_config: librefang_llm_driver::DriverConfig,
}

impl PooledDriver {
    pub(crate) fn new(
        pool: ArcCredentialPool,
        driver_cache: Arc<DriverCache>,
        base_config: librefang_llm_driver::DriverConfig,
    ) -> Self {
        Self {
            pool,
            driver_cache,
            base_config,
        }
    }
}

#[async_trait]
impl LlmDriver for PooledDriver {
    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        let api_key = self.pool.acquire().ok_or_else(|| LlmError::Api {
            status: 503,
            message:
                "All credential pool keys exhausted — no available credentials for this provider"
                    .into(),
            code: None,
        })?;

        let mut config = self.base_config.clone();
        config.api_key = Some(api_key.clone());

        let driver = self.driver_cache.get_or_create(&config)?;

        match driver.complete(request).await {
            Ok(response) => {
                self.pool.mark_success(&api_key);
                Ok(response)
            }
            Err(e) => {
                self.handle_driver_error(&api_key, &e);
                Err(e)
            }
        }
    }

    async fn stream(
        &self,
        request: CompletionRequest,
        tx: tokio::sync::mpsc::Sender<StreamEvent>,
    ) -> Result<CompletionResponse, LlmError> {
        let api_key = self.pool.acquire().ok_or_else(|| LlmError::Api {
            status: 503,
            message:
                "All credential pool keys exhausted — no available credentials for this provider"
                    .into(),
            code: None,
        })?;

        let mut config = self.base_config.clone();
        config.api_key = Some(api_key.clone());

        let driver = self.driver_cache.get_or_create(&config)?;

        match driver.stream(request, tx).await {
            Ok(response) => {
                self.pool.mark_success(&api_key);
                Ok(response)
            }
            Err(e) => {
                self.handle_driver_error(&api_key, &e);
                Err(e)
            }
        }
    }
}

impl PooledDriver {
    /// Classify a driver error and report it to the credential pool so
    /// exhausted / invalid keys are rotated out.
    fn handle_driver_error(&self, api_key: &str, error: &LlmError) {
        use librefang_llm_driver::llm_errors::FailoverReason;
        match error.failover_reason() {
            // Rate-limited: mark exhausted so the pool rotates to another key.
            // The key becomes available again after the pool's default cooldown
            // (1 hour).
            FailoverReason::RateLimit(_) => {
                self.pool.mark_exhausted(api_key);
            }
            // Billing / credit exhausted: mark exhausted with a cooldown so
            // the key isn't tried again until the billing cycle resets.
            FailoverReason::CreditExhausted => {
                self.pool.mark_exhausted(api_key);
            }
            // Auth failure: mark exhausted permanently (the pool's cooldown
            // will prevent reuse for the default TTL; subsequent success
            // from another path would clear it via mark_success).
            FailoverReason::AuthError => {
                self.pool.mark_exhausted(api_key);
            }
            // Model unavailable / timeout / http error: don't mark the key as
            // exhausted — these are provider-side issues, not key-specific.
            FailoverReason::ModelUnavailable
            | FailoverReason::Timeout
            | FailoverReason::HttpError => {}
            // Context too long / unknown: propagate without marking the key.
            FailoverReason::ContextTooLong | FailoverReason::Unknown => {}
        }
    }
}
