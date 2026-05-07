//! Error-classification-aware fallback chain for multi-provider LLM routing.
//!
//! [`FallbackChain`] differs from [`super::fallback::FallbackDriver`] in that it
//! uses [`FailoverReason`] to choose a *targeted* recovery strategy per error
//! class rather than applying a uniform health-penalty model:
//!
//! | `FailoverReason`    | Strategy                                              |
//! |---------------------|-------------------------------------------------------|
//! | `RateLimit`         | sleep `retry_delay_ms`, retry same provider ≤2 times  |
//! | `CreditExhausted`   | skip immediately to next provider                     |
//! | `ModelUnavailable`  | skip immediately to next provider                     |
//! | `Timeout`           | skip immediately to next provider                     |
//! | `HttpError`         | skip immediately to next provider                     |
//! | `AuthError`         | skip immediately to next provider                     |
//! | `ContextTooLong`    | propagate — caller must compress the context          |
//! | `Unknown`           | propagate — do not waste attempts on opaque errors    |
//!
//! The chain is ordered: index 0 is the primary provider, higher indices are
//! fallbacks.  Each element carries an optional model-name override so a single
//! `FallbackChain` can span heterogeneous providers that expose different model
//! slugs.

use crate::llm_driver::{CompletionRequest, CompletionResponse, LlmDriver, LlmError, StreamEvent};
use async_trait::async_trait;
use librefang_llm_driver::FailoverReason;
use std::sync::Arc;
use tokio::sync::watch;
use tracing::warn;

/// Default sleep duration when a provider returns a rate-limit error without a
/// `Retry-After` hint.  Kept short (2 s) so the chain does not stall the agent
/// loop for long; two retries means up to 4 s of backoff before failover.
const DEFAULT_RATE_LIMIT_SLEEP_MS: u64 = 2_000;

/// Maximum number of rate-limit retries on a single provider before skipping
/// to the next one in the chain.
const MAX_RATE_LIMIT_RETRIES: usize = 2;

// ---------------------------------------------------------------------------
// Entry type
// ---------------------------------------------------------------------------

/// A single slot in the fallback chain: a driver plus an optional model override.
pub struct ChainEntry {
    /// The LLM driver for this provider.
    pub driver: Arc<dyn LlmDriver>,
    /// When non-empty, overrides `CompletionRequest::model` for this provider.
    pub model_override: String,
    /// Human-readable provider label used in log messages.
    pub provider_name: String,
}

// ---------------------------------------------------------------------------
// FallbackChain
// ---------------------------------------------------------------------------

/// An ordered list of LLM drivers with error-classification-aware failover.
///
/// # Example
///
/// ```rust,ignore
/// let chain = FallbackChain::new(vec![
///     ChainEntry { driver: anthropic_driver, model_override: "claude-3-5-sonnet-20241022".into(), provider_name: "anthropic".into() },
///     ChainEntry { driver: openai_driver,    model_override: "gpt-4o".into(),                    provider_name: "openai".into() },
/// ]);
/// let response = chain.complete(request).await?;
/// ```
pub struct FallbackChain {
    entries: Vec<ChainEntry>,
    /// Sleep duration (ms) to use when a rate-limit error carries no
    /// `Retry-After` hint.
    rate_limit_sleep_ms: u64,
}

impl FallbackChain {
    /// Build a chain from an ordered list of entries.
    ///
    /// # Panics
    /// Panics when `entries` is empty — at least one provider is required.
    pub fn new(entries: Vec<ChainEntry>) -> Self {
        assert!(
            !entries.is_empty(),
            "FallbackChain requires at least one entry"
        );
        Self {
            entries,
            rate_limit_sleep_ms: DEFAULT_RATE_LIMIT_SLEEP_MS,
        }
    }

    /// Override the default rate-limit sleep duration (milliseconds).
    pub fn with_rate_limit_sleep_ms(mut self, ms: u64) -> Self {
        self.rate_limit_sleep_ms = ms;
        self
    }

    /// Build a `ChainEntry` slice from parallel `(driver, model, name)` tuples.
    pub fn from_tuples(tuples: Vec<(Arc<dyn LlmDriver>, String, String)>) -> Self {
        let entries = tuples
            .into_iter()
            .map(|(driver, model_override, provider_name)| ChainEntry {
                driver,
                model_override,
                provider_name,
            })
            .collect();
        Self::new(entries)
    }

    /// Attempt a `complete` call on a single entry, applying rate-limit retry
    /// logic for up to `MAX_RATE_LIMIT_RETRIES` before giving up on that entry.
    ///
    /// Returns:
    /// - `Ok(response)` on success.
    /// - `Err(e)` with the last error when all retries are exhausted.
    async fn try_entry(
        &self,
        entry: &ChainEntry,
        request: CompletionRequest,
    ) -> Result<CompletionResponse, LlmError> {
        let mut attempts = 0usize;

        loop {
            let mut req = request.clone();
            if !entry.model_override.is_empty() {
                req.model = entry.model_override.clone();
            }

            match entry.driver.complete(req).await {
                Ok(mut resp) => {
                    resp.actual_provider = Some(entry.provider_name.clone());
                    return Ok(resp);
                }
                Err(e) => {
                    let reason = e.failover_reason();

                    if matches!(reason, FailoverReason::RateLimit(_))
                        && attempts < MAX_RATE_LIMIT_RETRIES
                    {
                        // Extract the suggested retry delay from RateLimited or
                        // Overloaded variants (both now classify as RateLimit);
                        // fall back to the configured default when the hint is
                        // absent or zero.
                        let sleep_ms = match &e {
                            LlmError::RateLimited { retry_after_ms, .. } if *retry_after_ms > 0 => {
                                *retry_after_ms
                            }
                            LlmError::Overloaded { retry_after_ms } if *retry_after_ms > 0 => {
                                *retry_after_ms
                            }
                            _ => self.rate_limit_sleep_ms,
                        };

                        warn!(
                            provider = %entry.provider_name,
                            model = %entry.model_override,
                            attempt = attempts + 1,
                            sleep_ms,
                            reason = ?reason,
                            "FallbackChain: rate-limited, sleeping before retry"
                        );

                        tokio::time::sleep(std::time::Duration::from_millis(sleep_ms)).await;
                        attempts += 1;
                        continue;
                    }

                    // Any other reason (or rate-limit retries exhausted): return error
                    // to the outer loop which will decide whether to skip or propagate.
                    return Err(e);
                }
            }
        }
    }
}

#[async_trait]
impl LlmDriver for FallbackChain {
    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        let mut last_error: Option<LlmError> = None;

        for entry in &self.entries {
            match self.try_entry(entry, request.clone()).await {
                Ok(mut resp) => {
                    resp.actual_provider = Some(entry.provider_name.clone());
                    return Ok(resp);
                }
                Err(e) => {
                    let reason = e.failover_reason();
                    warn!(
                        provider = %entry.provider_name,
                        model = %entry.model_override,
                        error = %e,
                        reason = ?reason,
                        "FallbackChain: provider exhausted, trying next"
                    );

                    match reason {
                        // Skip to next provider.
                        FailoverReason::CreditExhausted
                        | FailoverReason::ModelUnavailable
                        | FailoverReason::Timeout
                        | FailoverReason::HttpError
                        | FailoverReason::AuthError
                        | FailoverReason::RateLimit(_) => {
                            last_error = Some(e);
                            continue;
                        }
                        // Propagate immediately.
                        FailoverReason::ContextTooLong | FailoverReason::Unknown => {
                            return Err(e);
                        }
                    }
                }
            }
        }

        Err(last_error.unwrap_or_else(|| LlmError::Api {
            status: 0,
            message: "FallbackChain: all providers exhausted".to_string(),
            code: None,
        }))
    }

    async fn stream(
        &self,
        request: CompletionRequest,
        tx: tokio::sync::mpsc::Sender<StreamEvent>,
    ) -> Result<CompletionResponse, LlmError> {
        let mut last_error: Option<LlmError> = None;

        for entry in &self.entries {
            let mut req = request.clone();
            if !entry.model_override.is_empty() {
                req.model = entry.model_override.clone();
            }

            // Intercept the event stream so we can detect whether any content
            // has already been forwarded to the caller before deciding whether
            // failover is safe.  If content was emitted and the provider then
            // errors, the caller has already received partial output; falling
            // through to the next provider would concatenate a second response
            // onto the partial content, producing garbage.  In that case we
            // propagate the error regardless of its FailoverReason.
            let (content_emitted_tx, content_emitted_rx) = watch::channel(false);
            let (intercept_tx, mut intercept_rx) = tokio::sync::mpsc::channel::<StreamEvent>(32);

            let tx_relay = tx.clone();
            let content_flag = content_emitted_tx.clone();
            let relay_handle = tokio::spawn(async move {
                while let Some(event) = intercept_rx.recv().await {
                    // Any event that represents observable LLM output to the
                    // caller.  PhaseChange is metadata-only and excluded.
                    let is_content = matches!(
                        &event,
                        StreamEvent::TextDelta { .. }
                            | StreamEvent::ToolUseStart { .. }
                            | StreamEvent::ToolInputDelta { .. }
                            | StreamEvent::ToolUseEnd { .. }
                            | StreamEvent::ThinkingDelta { .. }
                            | StreamEvent::ContentComplete { .. }
                            | StreamEvent::ToolExecutionResult { .. }
                    );
                    if is_content {
                        let _ = content_flag.send(true);
                    }
                    if tx_relay.send(event).await.is_err() {
                        // Downstream caller dropped the receiver. Close the
                        // relay's inbound channel so the wrapped driver's next
                        // `tx.send(...)` fails, triggering its backpressure
                        // path and aborting the upstream LLM stream (#3769).
                        tracing::debug!(
                            "FallbackChain(stream): downstream receiver dropped; cancelling inner driver"
                        );
                        intercept_rx.close();
                        break;
                    }
                }
            });

            // Stream does not get rate-limit retry (streaming mid-response retry
            // is not supported); any error here triggers the skip/propagate logic.
            match entry.driver.stream(req, intercept_tx).await {
                Ok(mut resp) => {
                    let _ = relay_handle.await;
                    resp.actual_provider = Some(entry.provider_name.clone());
                    return Ok(resp);
                }
                Err(e) => {
                    // Wait for the relay to finish draining before reading the
                    // content flag to avoid a TOCTOU race (events already in
                    // the mpsc buffer but not yet forwarded).
                    let _ = relay_handle.await;

                    let reason = e.failover_reason();
                    warn!(
                        provider = %entry.provider_name,
                        model = %entry.model_override,
                        error = %e,
                        reason = ?reason,
                        "FallbackChain(stream): provider exhausted, trying next"
                    );

                    // If the provider already forwarded content to the caller,
                    // failover would produce a corrupted concatenation — bail out.
                    if *content_emitted_rx.borrow() {
                        return Err(e);
                    }

                    match reason {
                        FailoverReason::CreditExhausted
                        | FailoverReason::ModelUnavailable
                        | FailoverReason::Timeout
                        | FailoverReason::HttpError
                        | FailoverReason::AuthError
                        | FailoverReason::RateLimit(_) => {
                            last_error = Some(e);
                            continue;
                        }
                        FailoverReason::ContextTooLong | FailoverReason::Unknown => {
                            return Err(e);
                        }
                    }
                }
            }
        }

        Err(last_error.unwrap_or_else(|| LlmError::Api {
            status: 0,
            message: "FallbackChain(stream): all providers exhausted".to_string(),
            code: None,
        }))
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm_driver::CompletionResponse;
    use librefang_types::message::{ContentBlock, StopReason, TokenUsage};

    fn ok_response(text: &str) -> CompletionResponse {
        CompletionResponse {
            content: vec![ContentBlock::Text {
                text: text.to_string(),
                provider_metadata: None,
            }],
            stop_reason: StopReason::EndTurn,
            tool_calls: vec![],
            usage: TokenUsage {
                input_tokens: 5,
                output_tokens: 3,
                ..Default::default()
            },
            actual_provider: None,
        }
    }

    fn test_request() -> CompletionRequest {
        CompletionRequest {
            model: "test-model".to_string(),
            messages: std::sync::Arc::new(vec![]),
            tools: std::sync::Arc::new(vec![]),
            max_tokens: 100,
            temperature: 0.0,
            system: None,
            thinking: None,
            prompt_caching: false,
            cache_ttl: None,
            response_format: None,
            timeout_secs: None,
            extra_body: None,
            agent_id: None,
            session_id: None,
            step_id: None,
        }
    }

    fn entry(driver: Arc<dyn LlmDriver>, name: &str) -> ChainEntry {
        ChainEntry {
            driver,
            model_override: String::new(),
            provider_name: name.to_string(),
        }
    }

    // ── Test drivers ──────────────────────────────────────────────────────

    struct OkDriver(&'static str);

    #[async_trait]
    impl LlmDriver for OkDriver {
        async fn complete(&self, _req: CompletionRequest) -> Result<CompletionResponse, LlmError> {
            Ok(ok_response(self.0))
        }
    }

    struct CreditExhaustedDriver;

    #[async_trait]
    impl LlmDriver for CreditExhaustedDriver {
        async fn complete(&self, _req: CompletionRequest) -> Result<CompletionResponse, LlmError> {
            Err(LlmError::Api {
                status: 402,
                message: "Insufficient credits in your account".to_string(),
                code: None,
            })
        }
    }

    struct ModelUnavailableDriver;

    #[async_trait]
    impl LlmDriver for ModelUnavailableDriver {
        async fn complete(&self, _req: CompletionRequest) -> Result<CompletionResponse, LlmError> {
            Err(LlmError::Api {
                status: 503,
                message: "Service unavailable".to_string(),
                code: None,
            })
        }
    }

    struct ContextTooLongDriver;

    #[async_trait]
    impl LlmDriver for ContextTooLongDriver {
        async fn complete(&self, _req: CompletionRequest) -> Result<CompletionResponse, LlmError> {
            Err(LlmError::Api {
                status: 413,
                message: "Context length exceeded".to_string(),
                code: None,
            })
        }
    }

    struct RateLimitedDriver {
        calls: std::sync::atomic::AtomicUsize,
    }

    #[async_trait]
    impl LlmDriver for RateLimitedDriver {
        async fn complete(&self, _req: CompletionRequest) -> Result<CompletionResponse, LlmError> {
            self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Err(LlmError::RateLimited {
                retry_after_ms: 1, // 1 ms so tests don't stall
                message: None,
            })
        }
    }

    // ── Tests ─────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn primary_succeeds() {
        let chain = FallbackChain::new(vec![entry(Arc::new(OkDriver("primary")), "p1")]);
        let r = chain.complete(test_request()).await.unwrap();
        assert_eq!(r.text(), "primary");
    }

    #[tokio::test]
    async fn credit_exhausted_falls_to_next() {
        let chain = FallbackChain::new(vec![
            entry(Arc::new(CreditExhaustedDriver), "p1"),
            entry(Arc::new(OkDriver("fallback")), "p2"),
        ]);
        let r = chain.complete(test_request()).await.unwrap();
        assert_eq!(r.text(), "fallback");
    }

    #[tokio::test]
    async fn model_unavailable_falls_to_next() {
        let chain = FallbackChain::new(vec![
            entry(Arc::new(ModelUnavailableDriver), "p1"),
            entry(Arc::new(OkDriver("fallback")), "p2"),
        ]);
        let r = chain.complete(test_request()).await.unwrap();
        assert_eq!(r.text(), "fallback");
    }

    #[tokio::test]
    async fn context_too_long_propagates_immediately() {
        let chain = FallbackChain::new(vec![
            entry(Arc::new(ContextTooLongDriver), "p1"),
            entry(Arc::new(OkDriver("should-not-reach")), "p2"),
        ]);
        let err = chain.complete(test_request()).await.unwrap_err();
        // ContextTooLong must propagate without reaching p2
        assert_eq!(err.failover_reason(), FailoverReason::ContextTooLong);
    }

    #[tokio::test]
    async fn rate_limited_retries_before_skip() {
        let driver = Arc::new(RateLimitedDriver {
            calls: std::sync::atomic::AtomicUsize::new(0),
        });
        let calls_ref = Arc::clone(&driver);
        let chain = FallbackChain::new(vec![
            ChainEntry {
                driver: driver as Arc<dyn LlmDriver>,
                model_override: String::new(),
                provider_name: "p1".to_string(),
            },
            entry(Arc::new(OkDriver("fallback")), "p2"),
        ])
        .with_rate_limit_sleep_ms(0); // no real sleeping in tests

        let r = chain.complete(test_request()).await.unwrap();
        assert_eq!(r.text(), "fallback");
        // MAX_RATE_LIMIT_RETRIES = 2 retries + 1 initial = 3 total calls on p1
        assert_eq!(
            calls_ref.calls.load(std::sync::atomic::Ordering::SeqCst),
            MAX_RATE_LIMIT_RETRIES + 1,
            "should attempt 1 + MAX_RATE_LIMIT_RETRIES times before skipping"
        );
    }

    #[tokio::test]
    async fn all_exhausted_returns_error() {
        let chain = FallbackChain::new(vec![
            entry(Arc::new(CreditExhaustedDriver), "p1"),
            entry(Arc::new(ModelUnavailableDriver), "p2"),
        ]);
        assert!(chain.complete(test_request()).await.is_err());
    }

    #[tokio::test]
    async fn model_override_applied() {
        struct ModelCapture;

        #[async_trait]
        impl LlmDriver for ModelCapture {
            async fn complete(
                &self,
                req: CompletionRequest,
            ) -> Result<CompletionResponse, LlmError> {
                Ok(ok_response(&req.model))
            }
        }

        let chain = FallbackChain::new(vec![ChainEntry {
            driver: Arc::new(ModelCapture),
            model_override: "custom-model-v2".to_string(),
            provider_name: "p1".to_string(),
        }]);
        let r = chain.complete(test_request()).await.unwrap();
        assert_eq!(r.text(), "custom-model-v2");
    }

    // ── failover_reason() unit tests ─────────────────────────────────────

    #[test]
    fn failover_reason_rate_limited() {
        let e = LlmError::RateLimited {
            retry_after_ms: 5000,
            message: None,
        };
        assert_eq!(e.failover_reason(), FailoverReason::RateLimit(Some(5000)));
    }

    #[test]
    fn failover_reason_429() {
        let e = LlmError::Api {
            status: 429,
            message: "Too many requests".to_string(),
            code: None,
        };
        assert_eq!(e.failover_reason(), FailoverReason::RateLimit(None));
    }

    #[test]
    fn failover_reason_402_credit() {
        let e = LlmError::Api {
            status: 402,
            message: "Insufficient credit balance".to_string(),
            code: None,
        };
        assert_eq!(e.failover_reason(), FailoverReason::CreditExhausted);
    }

    #[test]
    fn failover_reason_413_context() {
        let e = LlmError::Api {
            status: 413,
            message: "Payload too large".to_string(),
            code: None,
        };
        assert_eq!(e.failover_reason(), FailoverReason::ContextTooLong);
    }

    #[test]
    fn failover_reason_503() {
        let e = LlmError::Api {
            status: 503,
            message: "Service unavailable".to_string(),
            code: None,
        };
        assert_eq!(e.failover_reason(), FailoverReason::ModelUnavailable);
    }

    #[test]
    fn failover_reason_overloaded_is_rate_limit() {
        // Overloaded means the provider is reachable but busy — retry with
        // backoff (RateLimit), not skip to the next provider (ModelUnavailable).
        let e = LlmError::Overloaded {
            retry_after_ms: 1000,
        };
        assert_eq!(e.failover_reason(), FailoverReason::RateLimit(Some(1000)));
    }

    #[test]
    fn failover_reason_timed_out_variant() {
        let e = LlmError::TimedOut {
            inactivity_secs: 30,
            partial_text: None,
            partial_text_len: 0,
            last_activity: "none".to_string(),
        };
        assert_eq!(e.failover_reason(), FailoverReason::Timeout);
    }

    #[test]
    fn failover_reason_model_not_found() {
        let e = LlmError::ModelNotFound("gpt-5-ultra".to_string());
        assert_eq!(e.failover_reason(), FailoverReason::ModelUnavailable);
    }

    #[test]
    fn failover_reason_auth_skips_to_next_provider() {
        // A bad API key on one slot must not stop the entire chain — classify
        // as AuthError so later slots (with valid keys) can still run.
        let e = LlmError::AuthenticationFailed("bad key".to_string());
        assert_eq!(e.failover_reason(), FailoverReason::AuthError);
    }

    #[test]
    fn failover_reason_missing_key_skips_to_next_provider() {
        let e = LlmError::MissingApiKey("OPENAI_API_KEY".to_string());
        assert_eq!(e.failover_reason(), FailoverReason::AuthError);
    }

    #[test]
    fn failover_reason_401_is_auth_error() {
        let e = LlmError::Api {
            status: 401,
            message: "Unauthorized".to_string(),
            code: None,
        };
        assert_eq!(e.failover_reason(), FailoverReason::AuthError);
    }

    #[test]
    fn failover_reason_http_transport_error() {
        let e = LlmError::Http("connection refused".to_string());
        assert_eq!(e.failover_reason(), FailoverReason::HttpError);
    }

    // #3745: classification must come from the typed `code` field, not from
    // substring-matching the human-readable `message`. These tests use empty
    // / non-English / deliberately misleading messages combined with a typed
    // `ProviderErrorCode` and assert the correct `FailoverReason` is still
    // produced — proving the substring matcher is gone from the typed path.
    #[test]
    fn failover_reason_typed_code_classifies_with_empty_message() {
        use librefang_llm_driver::llm_errors::ProviderErrorCode;

        let e = LlmError::Api {
            // status 200 is nonsense for an error — pick something the
            // legacy matcher would have classified as `HttpError` to prove
            // the typed `code` is what drives the decision.
            status: 200,
            message: String::new(),
            code: Some(ProviderErrorCode::RateLimit),
        };
        assert_eq!(e.failover_reason(), FailoverReason::RateLimit(None));
    }

    #[test]
    fn failover_reason_typed_code_ignores_misleading_localized_message() {
        use librefang_llm_driver::llm_errors::ProviderErrorCode;

        // Provider rewrote the message in another language *and* the
        // English fragments don't contain any of the legacy substring
        // hooks ("rate limit", "credit", "context", "not found", …).
        let e = LlmError::Api {
            status: 403,
            message: "余额不足，请前往控制台充值".to_string(),
            code: Some(ProviderErrorCode::CreditExhausted),
        };
        assert_eq!(e.failover_reason(), FailoverReason::CreditExhausted);

        // Even an actively misleading English message must not flip the
        // verdict when the typed code says context overflow.
        let e = LlmError::Api {
            status: 400,
            message: "rate limit exceeded".to_string(),
            code: Some(ProviderErrorCode::ContextLengthExceeded),
        };
        assert_eq!(e.failover_reason(), FailoverReason::ContextTooLong);
    }

    #[test]
    fn failover_reason_typed_code_auth_error() {
        use librefang_llm_driver::llm_errors::ProviderErrorCode;

        let e = LlmError::Api {
            status: 403,
            message: String::new(),
            code: Some(ProviderErrorCode::AuthError),
        };
        assert_eq!(e.failover_reason(), FailoverReason::AuthError);
    }

    #[test]
    fn failover_reason_typed_code_model_not_found() {
        use librefang_llm_driver::llm_errors::ProviderErrorCode;

        let e = LlmError::Api {
            status: 404,
            message: "endpoint not found".to_string(),
            code: Some(ProviderErrorCode::ModelNotFound),
        };
        assert_eq!(e.failover_reason(), FailoverReason::ModelUnavailable);
    }

    #[tokio::test]
    async fn overloaded_retries_before_skip() {
        // Overloaded is a transient capacity error — the chain should retry
        // the same provider (up to MAX_RATE_LIMIT_RETRIES) before skipping.
        struct OverloadedDriver {
            calls: std::sync::atomic::AtomicUsize,
        }

        #[async_trait]
        impl LlmDriver for OverloadedDriver {
            async fn complete(
                &self,
                _req: CompletionRequest,
            ) -> Result<CompletionResponse, LlmError> {
                self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                Err(LlmError::Overloaded { retry_after_ms: 1 })
            }
        }

        let driver = Arc::new(OverloadedDriver {
            calls: std::sync::atomic::AtomicUsize::new(0),
        });
        let calls_ref = Arc::clone(&driver);
        let chain = FallbackChain::new(vec![
            ChainEntry {
                driver: driver as Arc<dyn LlmDriver>,
                model_override: String::new(),
                provider_name: "p1".to_string(),
            },
            entry(Arc::new(OkDriver("fallback")), "p2"),
        ])
        .with_rate_limit_sleep_ms(0);

        let r = chain.complete(test_request()).await.unwrap();
        assert_eq!(r.text(), "fallback");
        assert_eq!(
            calls_ref.calls.load(std::sync::atomic::Ordering::SeqCst),
            MAX_RATE_LIMIT_RETRIES + 1,
            "Overloaded should retry MAX_RATE_LIMIT_RETRIES times before skipping"
        );
    }

    #[tokio::test]
    async fn auth_failure_falls_to_next() {
        // A chain with a broken first slot (bad key) must succeed via the
        // second slot — not stop and propagate the auth error.
        struct AuthFailDriver;

        #[async_trait]
        impl LlmDriver for AuthFailDriver {
            async fn complete(
                &self,
                _req: CompletionRequest,
            ) -> Result<CompletionResponse, LlmError> {
                Err(LlmError::AuthenticationFailed(
                    "invalid api key".to_string(),
                ))
            }
        }

        let chain = FallbackChain::new(vec![
            entry(Arc::new(AuthFailDriver), "p1"),
            entry(Arc::new(OkDriver("fallback")), "p2"),
        ]);
        let r = chain.complete(test_request()).await.unwrap();
        assert_eq!(r.text(), "fallback");
    }
}
