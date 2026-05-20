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
    /// Classification policy (issue #4965 error decision matrix):
    /// - `RateLimit` (429): mark exhausted — 1h cooldown (caller already
    ///   retried once for `complete()`; `stream()` has no retry).
    /// - `CreditExhausted` (402): mark credit-exhausted — 24h cooldown
    ///   (quota refresh windows are typically daily).
    /// - `AuthError` (401/403/bad key): mark **permanently** exhausted — the
    ///   key is invalid and must be replaced outside the pool.
    /// - `HttpError` (other 4xx/5xx): mark exhausted — treat any provider-
    ///   side error as a reason to rotate.
    /// - `ModelUnavailable` / `Timeout`: don't mark the key — these are
    ///   provider-side issues, not key-specific.
    /// - `ContextTooLong` / `Unknown` / `ChainExhausted`: propagate without
    ///   marking — none of these classify the credential itself.
    fn handle_driver_error(&self, api_key: &str, error: &LlmError) {
        use librefang_llm_driver::llm_errors::FailoverReason;
        match error.failover_reason() {
            FailoverReason::RateLimit(_) => {
                self.pool.mark_exhausted(api_key);
            }
            FailoverReason::CreditExhausted => {
                // 402 — quota / credits depleted. Issue #4965 spec: 24h
                // cooldown to ride out the typical daily quota window.
                self.pool.mark_credit_exhausted(api_key);
            }
            FailoverReason::AuthError => {
                self.pool.mark_permanent(api_key);
            }
            FailoverReason::HttpError => {
                self.pool.mark_exhausted(api_key);
            }
            FailoverReason::ModelUnavailable | FailoverReason::Timeout => {}
            // ChainExhausted classifies the fallback chain as a whole, not
            // this credential — leave the key untouched. Reaching this arm
            // from a PooledDriver should be vanishingly rare (the wrapped
            // driver is a concrete provider, not a FallbackChain), but the
            // match must remain exhaustive.
            FailoverReason::ContextTooLong
            | FailoverReason::Unknown
            | FailoverReason::ChainExhausted => {}
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

#[cfg(test)]
mod tests {
    //! Behaviour tests for the `handle_driver_error` classification matrix —
    //! exercises the issue #4965 error → rotation decision table directly
    //! against a real `CredentialPool`, without needing to spin up a fake
    //! HTTP server. We use a no-op driver constructor (the actual driver
    //! handles aren't invoked here — only the error-classifier path).
    //!
    //! Full end-to-end coverage of the retry-on-429 + rotation flow happens
    //! in the `librefang-llm-drivers::credential_pool::tests` module (which
    //! is provider-agnostic) and in the `credential_pools_routes_test`
    //! integration test (which exercises the HTTP surface).
    use librefang_llm_driver::llm_errors::FailoverReason;
    use librefang_llm_driver::LlmError;
    use librefang_llm_drivers::{new_arc_pool, PoolStrategy};
    use librefang_runtime::drivers::DriverCache;
    use std::sync::Arc;
    use std::time::Duration;

    fn make_pooled() -> super::PooledDriver {
        let pool = new_arc_pool(
            vec![
                ("key-a".to_string(), 10),
                ("key-b".to_string(), 5),
            ],
            PoolStrategy::FillFirst,
        );
        let base_config = librefang_llm_driver::DriverConfig {
            provider: "test-provider".to_string(),
            api_key: None,
            base_url: None,
            ..Default::default()
        };
        super::PooledDriver::new(pool, Arc::new(DriverCache::new()), base_config)
    }

    /// 429 marks the key exhausted with the standard (1h) cooldown. Issue #4965
    /// row 1: a 429 the kernel sees after retry-once also flips the key into
    /// cooldown so FillFirst rolls forward to the next priority.
    #[test]
    fn rate_limit_marks_exhausted_short_cooldown() {
        let p = make_pooled();
        let err = LlmError::Api {
            status: 429,
            message: "Too many requests".into(),
            code: None,
        };
        assert!(matches!(err.failover_reason(), FailoverReason::RateLimit(_)));
        p.handle_driver_error("key-a", &err);
        // FillFirst now picks key-b (priority 5) because key-a is in cooldown.
        let snap = p.pool.snapshot();
        assert!(snap[0].is_exhausted, "key-a should be exhausted");
        let cooldown = snap[0].cooldown_remaining_secs.expect("cooldown set");
        // 1h ≈ 3600s; allow generous lower bound for test jitter.
        assert!(
            (3500..=3600).contains(&cooldown),
            "expected ~1h cooldown for 429, got {cooldown}s"
        );
    }

    /// 402 marks the key exhausted with the long (24h) credit cooldown.
    /// Issue #4965 row 2.
    #[test]
    fn credit_exhausted_marks_long_cooldown() {
        let p = make_pooled();
        let err = LlmError::Api {
            status: 402,
            message: "Insufficient credits".into(),
            code: None,
        };
        assert!(matches!(err.failover_reason(), FailoverReason::CreditExhausted));
        p.handle_driver_error("key-a", &err);
        let snap = p.pool.snapshot();
        assert!(snap[0].is_exhausted);
        let cooldown = snap[0].cooldown_remaining_secs.expect("cooldown set");
        // 24h ≈ 86_400s; assert it's clearly > 1h so we know we picked the
        // right code path (and not the 429 branch).
        assert!(
            cooldown > Duration::from_secs(60 * 60 * 2).as_secs(),
            "402 cooldown should exceed 2h to distinguish from 429 path, got {cooldown}s"
        );
        assert!(
            cooldown <= 86_400,
            "402 cooldown should not exceed 24h, got {cooldown}s"
        );
    }

    /// 401 marks the key permanently invalid (sentinel = u64::MAX).
    /// Issue #4965 row 3.
    #[test]
    fn auth_error_marks_permanent() {
        let p = make_pooled();
        let err = LlmError::Api {
            status: 401,
            message: "Invalid API key".into(),
            code: None,
        };
        assert!(matches!(err.failover_reason(), FailoverReason::AuthError));
        p.handle_driver_error("key-a", &err);
        let snap = p.pool.snapshot();
        assert_eq!(
            snap[0].cooldown_remaining_secs,
            Some(u64::MAX),
            "401 should mark key permanently invalid"
        );
    }

    /// 500/503/etc. mark the key exhausted (treated as a temporary fault).
    /// Issue #4965 row 4.
    #[test]
    fn http_error_marks_exhausted() {
        let p = make_pooled();
        let err = LlmError::Api {
            status: 500,
            message: "Internal server error".into(),
            code: None,
        };
        // 500 maps to HttpError (general HTTP failure path).
        assert!(matches!(err.failover_reason(), FailoverReason::HttpError));
        p.handle_driver_error("key-a", &err);
        let snap = p.pool.snapshot();
        assert!(snap[0].is_exhausted, "5xx should mark the key exhausted");
    }

    /// Timeouts and `ModelUnavailable` are provider-level conditions, not
    /// key-level — they must NOT mark the key. Issue #4965 row 5.
    #[test]
    fn timeout_does_not_mark_key() {
        let p = make_pooled();
        let err = LlmError::TimedOut {
            inactivity_secs: 30,
            partial_text: None,
            partial_text_len: 0,
            last_activity: "test".into(),
        };
        assert!(matches!(err.failover_reason(), FailoverReason::Timeout));
        p.handle_driver_error("key-a", &err);
        let snap = p.pool.snapshot();
        assert!(!snap[0].is_exhausted, "timeout must not mark the key");
        assert_eq!(snap[0].cooldown_remaining_secs, None);
    }

    /// Issue #4965 acceptance: when every key in the pool is in cooldown,
    /// `acquire()` must surface a 503-shape error so the surrounding
    /// `FallbackChain` can roll forward to the next `[[fallback_providers]]`
    /// entry (status 503 maps to `FailoverReason::ModelUnavailable` per
    /// `LlmError::failover_reason` for `code: None`).
    #[test]
    fn all_keys_exhausted_returns_503_for_fallback_chain() {
        let p = make_pooled();
        // Drain both keys via the 24h credit-exhausted path so no jitter
        // window can let one of them flip back to available between marks.
        p.pool.mark_credit_exhausted("key-a");
        p.pool.mark_credit_exhausted("key-b");
        assert_eq!(
            p.pool.available_count(),
            0,
            "preconditions: both keys must be in cooldown"
        );

        let err = p.acquire().expect_err("acquire must fail when no keys remain");
        match err {
            LlmError::Api { status, ref message, code } => {
                assert_eq!(status, 503, "must be 503 so FallbackChain rolls forward");
                assert!(
                    message.contains("exhausted"),
                    "diagnostic must mention exhaustion, got: {message}"
                );
                assert!(code.is_none(), "no provider-typed code expected");
            }
            other => panic!("expected LlmError::Api {{ status: 503, .. }}, got {other:?}"),
        }
        // And the failover classification: 503 with code=None maps to
        // ModelUnavailable → FallbackChain skips to the next provider entry.
        assert!(matches!(
            err.failover_reason(),
            FailoverReason::ModelUnavailable
        ));
    }
}
