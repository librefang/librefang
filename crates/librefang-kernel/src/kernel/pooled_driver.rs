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

    /// Build a DriverConfig patched with the given API key, then get or create
    /// the inner driver from the cache.
    fn inner_driver(&self, api_key: &str) -> Result<Arc<dyn LlmDriver>, LlmError> {
        let mut config = self.base_config.clone();
        config.api_key = Some(api_key.to_string());
        self.driver_cache.get_or_create(&config)
    }

    /// Acquire a key from the pool or return a 503-style error.
    fn acquire(&self) -> Result<String, LlmError> {
        self.pool.acquire().ok_or_else(|| LlmError::Api {
            status: 503,
            message:
                "All credential pool keys exhausted — no available credentials for this provider"
                    .into(),
            code: None,
        })
    }

    /// Classify a driver error and report it to the credential pool.
    ///
    /// Classification policy:
    /// - `RateLimit` (429): mark exhausted (caller already retried once for
    ///   `complete()`; `stream()` has no retry).
    /// - `CreditExhausted` (402): mark exhausted (1h cooldown).
    /// - `AuthError` (401/bad key): mark **permanently** exhausted — the key
    ///   is invalid and must be replaced outside the pool.
    /// - `HttpError` (other 4xx/5xx including 403): mark exhausted — treat
    ///   any provider-side error as a reason to rotate.
    /// - `ModelUnavailable` / `Timeout`: don't mark the key — these are
    ///   provider-side issues, not key-specific.
    /// - `ContextTooLong` / `Unknown`: propagate without marking.
    fn handle_driver_error(&self, api_key: &str, error: &LlmError) {
        use librefang_llm_driver::llm_errors::FailoverReason;
        match error.failover_reason() {
            FailoverReason::RateLimit(_) => {
                self.pool.mark_exhausted(api_key);
            }
            FailoverReason::CreditExhausted => {
                self.pool.mark_exhausted(api_key);
            }
            FailoverReason::AuthError => {
                self.pool.mark_permanent(api_key);
            }
            FailoverReason::HttpError => {
                self.pool.mark_exhausted(api_key);
            }
            FailoverReason::ModelUnavailable | FailoverReason::Timeout => {}
            FailoverReason::ContextTooLong | FailoverReason::Unknown => {}
        }
    }
}

#[async_trait]
impl LlmDriver for PooledDriver {
    /// Complete a non-streaming request with automatic 429 retry-once.
    ///
    /// If the first attempt returns a rate-limit error, the request is retried
    /// once with the same key. If the retry also fails (any error, including
    /// a second 429), the key is marked exhausted and the error is propagated.
    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        let api_key = self.acquire()?;
        let driver = self.inner_driver(&api_key)?;

        // Clone before first attempt so the request is still owned for the
        // potential retry. CompletionRequest wraps messages/tools in Arc, so
        // clone is cheap (refcount bump, not deep copy).
        let retry_request = request.clone();

        // First attempt.
        match driver.complete(request).await {
            Ok(response) => {
                self.pool.mark_success(&api_key);
                return Ok(response);
            }
            Err(first_err) => {
                // Retry once on rate-limit, propagate all other errors.
                let reason = first_err.failover_reason();
                if !matches!(
                    reason,
                    librefang_llm_driver::llm_errors::FailoverReason::RateLimit(_)
                ) {
                    self.handle_driver_error(&api_key, &first_err);
                    return Err(first_err);
                }
            }
        }

        // Retry with the same key.
        let driver = self.inner_driver(&api_key)?;
        match driver.complete(retry_request).await {
            Ok(response) => {
                self.pool.mark_success(&api_key);
                Ok(response)
            }
            Err(retry_err) => {
                self.handle_driver_error(&api_key, &retry_err);
                Err(retry_err)
            }
        }
    }

    /// Stream a response. Does not retry on 429 (partial stream events cannot
    /// be replayed), but still marks the key exhausted so the next call picks
    /// a fresh credential.
    async fn stream(
        &self,
        request: CompletionRequest,
        tx: tokio::sync::mpsc::Sender<StreamEvent>,
    ) -> Result<CompletionResponse, LlmError> {
        let api_key = self.acquire()?;
        let driver = self.inner_driver(&api_key)?;

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
