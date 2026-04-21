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
/// ]).unwrap();
/// let response = chain.complete(request).await?;
/// ```
pub struct FallbackChain {
    entries: Vec<ChainEntry>,
    /// Sleep duration (ms) to use when a rate-limit error carries no
    /// `Retry-After` hint.
    rate_limit_sleep_ms: u64,
    /// Maximum number of rate-limit retries on a single provider before
    /// skipping to the next one in the chain.
    max_rate_limit_retries: usize,
}

// ---------------------------------------------------------------------------
// Error types
// ---------------------------------------------------------------------------

/// Error returned when creating a [`FallbackChain`] with no entries.
#[derive(Debug)]
pub struct EmptyChainError;

impl std::fmt::Display for EmptyChainError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "FallbackChain requires at least one entry")
    }
}

impl std::error::Error for EmptyChainError {}

// ---------------------------------------------------------------------------
// FallbackChain
// ---------------------------------------------------------------------------

impl FallbackChain {
    /// Build a chain from an ordered list of entries.
    ///
    /// # Errors
    /// Returns an error if `entries` is empty — at least one provider is required.
    pub fn new(entries: Vec<ChainEntry>) -> Result<Self, EmptyChainError> {
        if entries.is_empty() {
            return Err(EmptyChainError);
        }
        Ok(Self {
            entries,
            rate_limit_sleep_ms: DEFAULT_RATE_LIMIT_SLEEP_MS,
            max_rate_limit_retries: MAX_RATE_LIMIT_RETRIES,
        })
    }

    /// Override the default rate-limit sleep duration (milliseconds).
    pub fn with_rate_limit_sleep_ms(mut self, ms: u64) -> Self {
        self.rate_limit_sleep_ms = ms;
        self
    }

    /// Override the maximum number of rate-limit retries on a single provider.
    pub fn with_max_rate_limit_retries(mut self, n: usize) -> Self {
        self.max_rate_limit_retries = n;
        self
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
                Ok(resp) => return Ok(resp),
                Err(e) => {
                    let reason = e.failover_reason();

                    if matches!(reason, FailoverReason::RateLimit(_))
                        && attempts < self.max_rate_limit_retries
                    {
                        // Extract suggested delay from the LlmError, fall back to default.
                        // Both RateLimited and Overloaded variants may carry a hint.
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
                Ok(resp) => return Ok(resp),
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

            // Wrap the sender so we can detect whether any TextDelta events
            // have already been forwarded to the caller.  If so, we cannot
            // safely fall through to the next provider — the caller would
            // receive a corrupted concatenation of this provider's partial
            // output and a fresh response from provider B.
            let (text_emit_tx, text_emit_rx) = watch::channel(false);
            let (wrap_tx, mut wrap_rx) = tokio::sync::mpsc::channel::<StreamEvent>(32);

            // Relay loop: forward events to the real sender, signal on any
            // content-bearing event (not just TextDelta) so that tool-use and
            // thinking streams are also detected before any failover decision.
            let tx_for_relay = tx.clone();
            let text_emit_flag = text_emit_tx.clone();
            let relay_handle = tokio::spawn(async move {
                while let Some(event) = wrap_rx.recv().await {
                    // Set the flag for any event that represents actual content
                    // output from the provider.  Metadata-only events such as
                    // RateLimitInfo are NOT listed here because they don't
                    // constitute observable output to the caller.
                    let is_content = matches!(
                        &event,
                        StreamEvent::TextDelta { .. }
                            | StreamEvent::ToolUseStart { .. }
                            | StreamEvent::ToolInputDelta { .. }
                            | StreamEvent::ThinkingDelta { .. }
                            | StreamEvent::ContentComplete { .. }
                    );
                    if is_content {
                        let _ = text_emit_flag.send(true);
                    }
                    let _ = tx_for_relay.send(event).await;
                }
            });

            // Stream does not get rate-limit retry (streaming mid-response retry
            // is not supported); any error here triggers the skip/propagate logic.
            match entry.driver.stream(req, wrap_tx).await {
                Ok(resp) => {
                    // Provider succeeded — wait for the relay task to drain all
                    // buffered events to the caller before returning, so events
                    // are not silently dropped when the relay_handle is dropped.
                    let _ = relay_handle.await;
                    return Ok(resp);
                }
                Err(e) => {
                    // Drop wrap_tx is implicit here (moved into stream above and
                    // returned by stream).  Wait for the relay task to drain all
                    // buffered events before reading the text_emit flag, otherwise
                    // we race against TextDelta events still sitting in the mpsc
                    // buffer (TOCTOU: flag could read false while events are
                    // in-flight).
                    let _ = relay_handle.await;

                    let reason = e.failover_reason();
                    warn!(
                        provider = %entry.provider_name,
                        model = %entry.model_override,
                        error = %e,
                        reason = ?reason,
                        "FallbackChain(stream): provider exhausted, trying next"
                    );

                    match reason {
                        // If any text was emitted before the error, we cannot
                        // safely fall through — bail out with the error rather
                        // than let the caller receive concatenated garbage.
                        _ if *text_emit_rx.borrow() => {
                            return Err(e);
                        }
                        FailoverReason::CreditExhausted
                        | FailoverReason::ModelUnavailable
                        | FailoverReason::Timeout
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
        }
    }

    fn test_request() -> CompletionRequest {
        CompletionRequest {
            model: "test-model".to_string(),
            messages: vec![],
            tools: vec![],
            max_tokens: 100,
            temperature: 0.0,
            system: None,
            thinking: None,
            prompt_caching: false,
            response_format: None,
            timeout_secs: None,
            extra_body: None,
            agent_id: None,
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
        let chain = FallbackChain::new(vec![entry(Arc::new(OkDriver("primary")), "p1")]).unwrap();
        let r = chain.complete(test_request()).await.unwrap();
        assert_eq!(r.text(), "primary");
    }

    #[tokio::test]
    async fn credit_exhausted_falls_to_next() {
        let chain = FallbackChain::new(vec![
            entry(Arc::new(CreditExhaustedDriver), "p1"),
            entry(Arc::new(OkDriver("fallback")), "p2"),
        ])
        .unwrap();
        let r = chain.complete(test_request()).await.unwrap();
        assert_eq!(r.text(), "fallback");
    }

    #[tokio::test]
    async fn model_unavailable_falls_to_next() {
        let chain = FallbackChain::new(vec![
            entry(Arc::new(ModelUnavailableDriver), "p1"),
            entry(Arc::new(OkDriver("fallback")), "p2"),
        ])
        .unwrap();
        let r = chain.complete(test_request()).await.unwrap();
        assert_eq!(r.text(), "fallback");
    }

    #[tokio::test]
    async fn context_too_long_propagates_immediately() {
        let chain = FallbackChain::new(vec![
            entry(Arc::new(ContextTooLongDriver), "p1"),
            entry(Arc::new(OkDriver("should-not-reach")), "p2"),
        ])
        .unwrap();
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
        .unwrap()
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
        ])
        .unwrap();
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
        }])
        .unwrap();
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
        };
        assert_eq!(e.failover_reason(), FailoverReason::RateLimit(None));
    }

    #[test]
    fn failover_reason_402_credit() {
        let e = LlmError::Api {
            status: 402,
            message: "Insufficient credit balance".to_string(),
        };
        assert_eq!(e.failover_reason(), FailoverReason::CreditExhausted);
    }

    #[test]
    fn failover_reason_413_context() {
        let e = LlmError::Api {
            status: 413,
            message: "Payload too large".to_string(),
        };
        assert_eq!(e.failover_reason(), FailoverReason::ContextTooLong);
    }

    #[test]
    fn failover_reason_503() {
        let e = LlmError::Api {
            status: 503,
            message: "Service unavailable".to_string(),
        };
        assert_eq!(e.failover_reason(), FailoverReason::ModelUnavailable);
    }

    #[test]
    fn failover_reason_overloaded_variant() {
        let e = LlmError::Overloaded {
            retry_after_ms: 1000,
        };
        assert_eq!(e.failover_reason(), FailoverReason::RateLimit(Some(1000)));
    }

    #[test]
    fn failover_reason_timed_out_variant() {
        let e = LlmError::TimedOut {
            inactivity_secs: 30,
            partial_text: String::new(),
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
    fn failover_reason_auth_model_unavailable() {
        let e = LlmError::AuthenticationFailed("bad key".to_string());
        assert_eq!(e.failover_reason(), FailoverReason::ModelUnavailable);
    }

    #[test]
    fn failover_reason_missing_api_key_model_unavailable() {
        let e = LlmError::MissingApiKey("ANTHROPIC_API_KEY".to_string());
        assert_eq!(e.failover_reason(), FailoverReason::ModelUnavailable);
    }

    #[test]
    fn failover_reason_parse_unknown() {
        let e = LlmError::Parse("unexpected token".to_string());
        assert_eq!(e.failover_reason(), FailoverReason::Unknown);
    }

    #[test]
    fn failover_reason_404_generic_not_found_is_unknown() {
        // A 404 whose message does NOT reference the model (e.g. wrong base
        // URL) should be Unknown, not ModelUnavailable.
        let e = LlmError::Api {
            status: 404,
            message: "Not found".to_string(),
        };
        assert_eq!(e.failover_reason(), FailoverReason::Unknown);
    }

    #[test]
    fn failover_reason_404_model_not_found_is_model_unavailable() {
        // A 404 whose message explicitly references the model should still
        // be classified as ModelUnavailable.
        let e = LlmError::Api {
            status: 404,
            message: "model not found: claude-4-opus".to_string(),
        };
        assert_eq!(e.failover_reason(), FailoverReason::ModelUnavailable);
    }

    #[test]
    fn failover_reason_overloaded_uses_retry_hint() {
        let e = LlmError::Overloaded {
            retry_after_ms: 3000,
        };
        assert_eq!(e.failover_reason(), FailoverReason::RateLimit(Some(3000)));
    }
}
