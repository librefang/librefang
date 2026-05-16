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
use librefang_llm_driver::exhaustion::{
    ExhaustionReason, ProviderExhaustionStore, DEFAULT_LONG_BACKOFF,
};
use librefang_llm_driver::{FailoverReason, ProviderExhaustionDetail};
use std::sync::Arc;
use std::time::{Duration, Instant};
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
    /// Shared exhaustion store (#4807). When `Some`, each `complete` /
    /// `stream` call pre-checks every slot via
    /// [`ProviderExhaustionStore::is_exhausted`] and skips exhausted
    /// slots without invoking the underlying driver. Failed attempts
    /// classified as rate-limit / quota / auth mark the slot exhausted
    /// so subsequent requests within the backoff window also skip it.
    /// `None` preserves the historical un-gated behaviour for callers
    /// that have not opted in (tests, ad-hoc one-shot chains).
    exhaustion_store: Option<ProviderExhaustionStore>,
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
            exhaustion_store: None,
        }
    }

    /// Override the default rate-limit sleep duration (milliseconds).
    pub fn with_rate_limit_sleep_ms(mut self, ms: u64) -> Self {
        self.rate_limit_sleep_ms = ms;
        self
    }

    /// Attach a shared exhaustion store (#4807). When attached, the chain
    /// skips slots that have been marked exhausted (rate-limit, quota,
    /// budget, auth) without invoking the underlying driver — and marks
    /// slots exhausted itself after a failing attempt.
    ///
    /// The store is reference-counted internally; cloning is cheap. The
    /// same store should be passed to every `FallbackChain` constructed
    /// against the same set of provider slots so all chains see a
    /// coherent exhaustion view.
    pub fn with_exhaustion_store(mut self, store: ProviderExhaustionStore) -> Self {
        self.exhaustion_store = Some(store);
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

                    let retryable = matches!(
                        reason,
                        FailoverReason::RateLimit(_)
                            | FailoverReason::HttpError
                            | FailoverReason::Timeout
                    );

                    if retryable && attempts < MAX_RATE_LIMIT_RETRIES {
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
                            "FallbackChain: transient error, retrying before failover"
                        );

                        tokio::time::sleep(std::time::Duration::from_millis(sleep_ms)).await;
                        attempts += 1;
                        continue;
                    }

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
        // Tracks why each slot was skipped, for `AllProvidersExhausted`
        // when nothing in the chain succeeds. Keyed by provider_name so
        // duplicate provider entries collapse — the most recent reason
        // wins (consistent with `ProviderExhaustionStore::mark_exhausted`).
        let mut skip_reasons: std::collections::BTreeMap<String, ExhaustionReason> =
            std::collections::BTreeMap::new();

        for entry in &self.entries {
            // Pre-check: if a shared exhaustion store says this slot is
            // out, skip it without invoking the driver. This is the
            // primary win of #4807 — we don't burn latency re-asking a
            // provider we know is rate-limited or out of credit.
            if let Some(store) = &self.exhaustion_store {
                if let Some(rec) = store.record_skip(&entry.provider_name) {
                    skip_reasons.insert(entry.provider_name.clone(), rec.reason);
                    continue;
                }
            }

            match self.try_entry(entry, request.clone()).await {
                Ok(mut resp) => {
                    // Stamp the slot that actually served the request
                    // so the metering layer attributes spend correctly
                    // (review nit 10). Preserve any stamp set by a
                    // wrapped FallbackChain / FallbackDriver lower in
                    // the stack.
                    if resp.actual_provider.is_none() {
                        resp.actual_provider = Some(entry.provider_name.clone());
                    }
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

                    // Record the slot in the shared exhaustion store so
                    // subsequent requests within the backoff window skip
                    // it (#4807).
                    if let Some(store) = &self.exhaustion_store {
                        if let Some(exhaust_reason) = exhaustion_reason_for(&reason) {
                            let until = exhaustion_until_for(&e, &reason);
                            store.mark_exhausted(
                                entry.provider_name.clone(),
                                exhaust_reason,
                                until,
                            );
                            skip_reasons.insert(entry.provider_name.clone(), exhaust_reason);
                        }
                    }

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
                        // Propagate immediately. `ChainExhausted`
                        // means a *nested* FallbackChain (e.g. an
                        // aux client inside the primary chain)
                        // already exhausted its own slots — there's
                        // no point retrying the same wrapped chain.
                        FailoverReason::ContextTooLong
                        | FailoverReason::ChainExhausted
                        | FailoverReason::Unknown => {
                            return Err(e);
                        }
                    }
                }
            }
        }

        // Every entry refused or was pre-skipped. With a store
        // attached and *every* slot accounted for as an exhaustion-
        // class outcome (pre-skipped via the store OR attempted and
        // failed with a rate-limit / quota / auth reason), synthesize
        // the typed `AllProvidersExhausted` variant so callers can
        // react to "chain is dry" without parsing a generic Api
        // message (#4807). The most recent underlying provider error
        // (if any slot was actually attempted) rides along on
        // `cause`, which `thiserror`'s `#[source]` attribute exposes
        // through `std::error::Error::source` — preserving the
        // upstream chain per the trait-crate's source-chain rule
        // (#3745).
        //
        // Critically, when at least one slot failed with a NON-
        // exhaustion reason (e.g. genuine HTTP 500, transport error),
        // we propagate that last error verbatim instead of wrapping
        // it in `AllProvidersExhausted`. The chain isn't actually
        // "dry" in that case — the slot is broken, not exhausted —
        // and callers depend on the distinction to route the failure
        // (a 500 reaches the human; an exhaustion event reaches the
        // operator). Review blocking issue #5.
        if self.exhaustion_store.is_some() && skip_reasons.len() == self.entries.len() {
            return Err(LlmError::AllProvidersExhausted {
                details: skip_reasons
                    .into_iter()
                    .map(|(provider_id, reason)| ProviderExhaustionDetail {
                        provider_id,
                        reason,
                    })
                    .collect(),
                cause: last_error.map(Box::new),
            });
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
        let mut skip_reasons: std::collections::BTreeMap<String, ExhaustionReason> =
            std::collections::BTreeMap::new();

        for entry in &self.entries {
            // Pre-check the exhaustion store before opening the upstream
            // stream — exhaustion-aware fallback applies equally to
            // streaming and non-streaming paths (#4807).
            if let Some(store) = &self.exhaustion_store {
                if let Some(rec) = store.record_skip(&entry.provider_name) {
                    skip_reasons.insert(entry.provider_name.clone(), rec.reason);
                    continue;
                }
            }

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
                    // Wait for the relay to drain all buffered events so they
                    // are not silently dropped when the handle is discarded.
                    let _ = relay_handle.await;
                    // Stamp the slot that actually served the stream
                    // (review nit 10). Preserve any stamp from a nested
                    // wrapper.
                    if resp.actual_provider.is_none() {
                        resp.actual_provider = Some(entry.provider_name.clone());
                    }
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

                    // Mark the slot exhausted so subsequent calls skip it
                    // within the backoff window (#4807).
                    if let Some(store) = &self.exhaustion_store {
                        if let Some(exhaust_reason) = exhaustion_reason_for(&reason) {
                            let until = exhaustion_until_for(&e, &reason);
                            store.mark_exhausted(
                                entry.provider_name.clone(),
                                exhaust_reason,
                                until,
                            );
                            skip_reasons.insert(entry.provider_name.clone(), exhaust_reason);
                        }
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
                        FailoverReason::ContextTooLong
                        | FailoverReason::ChainExhausted
                        | FailoverReason::Unknown => {
                            return Err(e);
                        }
                    }
                }
            }
        }

        // Streaming path mirrors the non-streaming policy above
        // (#4807, #3745, review blocking #5): synthesize the typed
        // `AllProvidersExhausted` variant only when *every* slot was
        // accounted for as an exhaustion-class outcome. A non-
        // exhaustion failure (e.g. genuine 500) propagates verbatim
        // so the caller can distinguish "chain dry" from "slot
        // broken".
        if self.exhaustion_store.is_some() && skip_reasons.len() == self.entries.len() {
            return Err(LlmError::AllProvidersExhausted {
                details: skip_reasons
                    .into_iter()
                    .map(|(provider_id, reason)| ProviderExhaustionDetail {
                        provider_id,
                        reason,
                    })
                    .collect(),
                cause: last_error.map(Box::new),
            });
        }

        Err(last_error.unwrap_or_else(|| LlmError::Api {
            status: 0,
            message: "FallbackChain(stream): all providers exhausted".to_string(),
            code: None,
        }))
    }
}

// ---------------------------------------------------------------------------
// Exhaustion reason / until helpers (#4807)
// ---------------------------------------------------------------------------

/// Map a [`FailoverReason`] (per-error structural classification) onto the
/// store's [`ExhaustionReason`]. Returns `None` for failure modes that
/// should NOT mark the slot — context-too-long (caller error, slot is
/// fine), unknown (could not classify), and the transient
/// `ModelUnavailable` / `HttpError` / `Timeout` paths where a single
/// failure isn't evidence the slot is durably down.
///
/// `pub(crate)` so the sibling [`super::fallback::FallbackDriver`] can
/// share the same exhaustion-classification policy without duplicating
/// the mapping (#4807, Blocker 2).
pub(crate) fn exhaustion_reason_for(failover: &FailoverReason) -> Option<ExhaustionReason> {
    match failover {
        FailoverReason::RateLimit(_) => Some(ExhaustionReason::RateLimited),
        FailoverReason::CreditExhausted => Some(ExhaustionReason::QuotaExceeded),
        FailoverReason::AuthError => Some(ExhaustionReason::AuthFailed),
        // ModelUnavailable / Timeout / HttpError can be transient (network
        // hiccup, single 503). One failure shouldn't park the slot for an
        // hour — let the next call re-attempt. If the slot truly is down,
        // it will keep failing and any cumulative health-penalty layer
        // (FallbackDriver) handles that orthogonally.
        FailoverReason::ModelUnavailable
        | FailoverReason::Timeout
        | FailoverReason::HttpError
        | FailoverReason::ContextTooLong
        | FailoverReason::ChainExhausted
        | FailoverReason::Unknown => None,
    }
}

/// Minimum exhaustion backoff applied to rate-limit hints. Some providers
/// (and `try_entry`'s synthetic short hints used in tests) report
/// `Retry-After: 0` or tiny single-digit-millisecond values; honouring those
/// literally would clear the exhaustion entry before the next request even
/// gets to the store, defeating the point of the skip-on-next-call gate.
/// 30s is short enough that a rate-limit window genuinely lifting in
/// seconds is barely delayed; long enough that the store retains its value.
const MIN_RATE_LIMIT_EXHAUSTION_BACKOFF: Duration = Duration::from_secs(30);

/// Compute the auto-clear `Instant` for an exhaustion record based on the
/// underlying error. RateLimit honours the server's `Retry-After` hint
/// (parsed from `LlmError::RateLimited.retry_after_ms` / `Overloaded`)
/// floored at [`MIN_RATE_LIMIT_EXHAUSTION_BACKOFF`]; the hard-action
/// variants (quota, auth) use [`DEFAULT_LONG_BACKOFF`].
///
/// `pub(crate)` so the sibling [`super::fallback::FallbackDriver`] can
/// reuse the same back-off policy (#4807, Blocker 2).
pub(crate) fn exhaustion_until_for(err: &LlmError, failover: &FailoverReason) -> Option<Instant> {
    let now = Instant::now();
    match failover {
        FailoverReason::RateLimit(Some(ms)) if *ms > 0 => {
            let hinted = Duration::from_millis(*ms);
            Some(now + hinted.max(MIN_RATE_LIMIT_EXHAUSTION_BACKOFF))
        }
        FailoverReason::RateLimit(_) => {
            // Server didn't tell us when to retry. Pull the embedded hint
            // off RateLimited / Overloaded variants when present, else
            // fall back to the default short back-off used by `try_entry`.
            let hint_ms = match err {
                LlmError::RateLimited { retry_after_ms, .. } if *retry_after_ms > 0 => {
                    Some(*retry_after_ms)
                }
                LlmError::Overloaded { retry_after_ms } if *retry_after_ms > 0 => {
                    Some(*retry_after_ms)
                }
                _ => None,
            };
            let hinted = Duration::from_millis(hint_ms.unwrap_or(DEFAULT_RATE_LIMIT_SLEEP_MS));
            Some(now + hinted.max(MIN_RATE_LIMIT_EXHAUSTION_BACKOFF))
        }
        FailoverReason::CreditExhausted | FailoverReason::AuthError => {
            Some(now + DEFAULT_LONG_BACKOFF)
        }
        _ => None,
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
            prompt_cache_strategy: None,
            response_format: None,
            timeout_secs: None,
            extra_body: None,
            agent_id: None,
            session_id: None,
            step_id: None,
            reasoning_echo_policy: librefang_types::model_catalog::ReasoningEchoPolicy::default(),
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

    // ── #4807: exhaustion-store-aware fallback ──────────────────────────

    /// Driver that counts every call so tests can prove a slot was (or
    /// was not) invoked.
    struct CountingOkDriver {
        calls: std::sync::atomic::AtomicUsize,
        label: &'static str,
    }

    #[async_trait]
    impl LlmDriver for CountingOkDriver {
        async fn complete(&self, _req: CompletionRequest) -> Result<CompletionResponse, LlmError> {
            self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(ok_response(self.label))
        }
    }

    /// Driver that always rate-limits with a server-supplied retry hint,
    /// counting calls so we can prove the chain skips it on subsequent
    /// invocations.
    struct RateLimitedCountingDriver {
        calls: std::sync::atomic::AtomicUsize,
        retry_after_ms: u64,
    }

    #[async_trait]
    impl LlmDriver for RateLimitedCountingDriver {
        async fn complete(&self, _req: CompletionRequest) -> Result<CompletionResponse, LlmError> {
            self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Err(LlmError::RateLimited {
                retry_after_ms: self.retry_after_ms,
                message: None,
            })
        }
    }

    #[tokio::test]
    async fn exhaustion_store_skips_marked_provider() {
        // With a shared exhaustion store: pre-marked providers are skipped
        // without invoking their driver.
        let store = ProviderExhaustionStore::new();
        store.mark_exhausted(
            "p1",
            ExhaustionReason::RateLimited,
            Some(Instant::now() + Duration::from_secs(60)),
        );

        let p1_driver = Arc::new(CountingOkDriver {
            calls: std::sync::atomic::AtomicUsize::new(0),
            label: "p1",
        });
        let p1_counter = Arc::clone(&p1_driver);

        let chain = FallbackChain::new(vec![
            ChainEntry {
                driver: p1_driver as Arc<dyn LlmDriver>,
                model_override: String::new(),
                provider_name: "p1".to_string(),
            },
            entry(Arc::new(OkDriver("p2")), "p2"),
        ])
        .with_exhaustion_store(store);

        let r = chain.complete(test_request()).await.unwrap();
        assert_eq!(r.text(), "p2", "exhausted p1 must be skipped");
        assert_eq!(
            p1_counter.calls.load(std::sync::atomic::Ordering::SeqCst),
            0,
            "exhausted slot's driver must NOT be called"
        );
    }

    #[tokio::test]
    async fn rate_limit_failure_marks_slot_with_retry_after() {
        // After the chain learns p1 is rate-limited, a second invocation
        // skips p1 directly via the store — proving exhaustion state
        // persists between calls.
        let store = ProviderExhaustionStore::new();
        // `retry_after_ms = 1` keeps `try_entry`'s internal back-off short
        // (1 ms × MAX_RATE_LIMIT_RETRIES) so the test isn't stalled by the
        // embedded hint. The store-side `until` derived in
        // `exhaustion_until_for` lands ~1 ms in the future, which is more
        // than enough to be past `now` on the second pass — exactly what we
        // need: prove the second call skips even when the embedded hint
        // is short, because the store entry is fresh.
        let p1_driver = Arc::new(RateLimitedCountingDriver {
            calls: std::sync::atomic::AtomicUsize::new(0),
            retry_after_ms: 1,
        });
        let p1_counter = Arc::clone(&p1_driver);

        let p2_driver = Arc::new(CountingOkDriver {
            calls: std::sync::atomic::AtomicUsize::new(0),
            label: "p2",
        });
        let p2_counter = Arc::clone(&p2_driver);

        let chain = FallbackChain::new(vec![
            ChainEntry {
                driver: p1_driver as Arc<dyn LlmDriver>,
                model_override: String::new(),
                provider_name: "p1".to_string(),
            },
            ChainEntry {
                driver: p2_driver as Arc<dyn LlmDriver>,
                model_override: String::new(),
                provider_name: "p2".to_string(),
            },
        ])
        .with_rate_limit_sleep_ms(0)
        .with_exhaustion_store(store.clone());

        // First call: p1 fails MAX_RATE_LIMIT_RETRIES + 1 times then p2 wins.
        let _ = chain.complete(test_request()).await.unwrap();
        let calls_after_first = p1_counter.calls.load(std::sync::atomic::Ordering::SeqCst);
        assert!(
            calls_after_first > 0,
            "p1 must have been called on the first request"
        );

        // Store should now have p1 marked as RateLimited.
        let rec = store.is_exhausted("p1").expect("p1 should be marked");
        assert_eq!(rec.reason, ExhaustionReason::RateLimited);
        assert!(rec.until.is_some(), "rate-limit should carry a retry time");

        // Second call: p1 must be skipped entirely — driver call count
        // does not change.
        let _ = chain.complete(test_request()).await.unwrap();
        assert_eq!(
            p1_counter.calls.load(std::sync::atomic::Ordering::SeqCst),
            calls_after_first,
            "p1 driver must NOT be invoked on the second call (slot marked exhausted)"
        );
        assert_eq!(
            p2_counter.calls.load(std::sync::atomic::Ordering::SeqCst),
            2,
            "p2 must serve both requests"
        );
    }

    #[tokio::test]
    async fn auth_failure_marks_slot_with_long_backoff() {
        struct AuthFailDriver;

        #[async_trait]
        impl LlmDriver for AuthFailDriver {
            async fn complete(
                &self,
                _req: CompletionRequest,
            ) -> Result<CompletionResponse, LlmError> {
                Err(LlmError::AuthenticationFailed("bad key".to_string()))
            }
        }

        let store = ProviderExhaustionStore::new();
        let chain = FallbackChain::new(vec![
            entry(Arc::new(AuthFailDriver), "p1"),
            entry(Arc::new(OkDriver("p2")), "p2"),
        ])
        .with_exhaustion_store(store.clone());

        let _ = chain.complete(test_request()).await.unwrap();

        let rec = store.is_exhausted("p1").expect("p1 should be marked");
        assert_eq!(rec.reason, ExhaustionReason::AuthFailed);
        // AuthError uses DEFAULT_LONG_BACKOFF (1h). Confirm `until` is at
        // least 30 minutes in the future — generous tolerance for slow
        // test runners.
        let until = rec.until.expect("auth failure must carry a backoff");
        let remaining = until.saturating_duration_since(Instant::now());
        assert!(
            remaining > Duration::from_secs(30 * 60),
            "auth-failed backoff should be ~1h, got {remaining:?}"
        );
    }

    #[tokio::test]
    async fn all_exhausted_yields_typed_variant_when_store_present() {
        // When every slot in the chain is pre-marked exhausted, the
        // returned error is the typed `AllProvidersExhausted` variant
        // listing each slot — callers can recognise "chain is dry"
        // without parsing a generic Api message.
        let store = ProviderExhaustionStore::new();
        let until = Instant::now() + Duration::from_secs(60);
        store.mark_exhausted("p1", ExhaustionReason::RateLimited, Some(until));
        store.mark_exhausted("p2", ExhaustionReason::QuotaExceeded, Some(until));

        let chain = FallbackChain::new(vec![
            entry(Arc::new(OkDriver("should-not-reach-1")), "p1"),
            entry(Arc::new(OkDriver("should-not-reach-2")), "p2"),
        ])
        .with_exhaustion_store(store);

        let err = chain.complete(test_request()).await.unwrap_err();
        match err {
            LlmError::AllProvidersExhausted { details, cause } => {
                assert_eq!(details.len(), 2);
                // Sorted by provider_id — p1 then p2.
                assert_eq!(details[0].provider_id, "p1");
                assert_eq!(details[0].reason, ExhaustionReason::RateLimited);
                assert_eq!(details[1].provider_id, "p2");
                assert_eq!(details[1].reason, ExhaustionReason::QuotaExceeded);
                // Every slot was pre-skipped before any upstream call —
                // there is no underlying provider error to ride along.
                assert!(
                    cause.is_none(),
                    "all-pre-skipped path must not synthesize a fake cause, got {cause:?}"
                );
            }
            other => panic!("expected AllProvidersExhausted, got {other:?}"),
        }
    }

    // #4807 / #3745 — When at least one slot was attempted and failed
    // before the chain ran dry, the typed `AllProvidersExhausted`
    // variant must (a) still fire (was previously unreachable in this
    // case — review blocking #5) and (b) preserve the upstream
    // provider error via `Error::source()` so the source chain rule
    // from `librefang-llm-driver/AGENTS.md` is honoured.
    #[tokio::test]
    async fn mixed_attempt_and_skip_yields_typed_variant_with_source_chain() {
        let store = ProviderExhaustionStore::new();
        // p1 is pre-marked exhausted; p2 is fresh but its driver fails
        // with a credit-exhausted error, which the chain marks against
        // the store and treats as a skip-to-next.
        let until = Instant::now() + Duration::from_secs(60);
        store.mark_exhausted("p1", ExhaustionReason::RateLimited, Some(until));

        let chain = FallbackChain::new(vec![
            entry(Arc::new(OkDriver("should-not-reach")), "p1"),
            entry(Arc::new(CreditExhaustedDriver), "p2"),
        ])
        .with_exhaustion_store(store);

        let err = chain.complete(test_request()).await.unwrap_err();
        match &err {
            LlmError::AllProvidersExhausted { details, cause } => {
                assert_eq!(details.len(), 2);
                assert_eq!(details[0].provider_id, "p1");
                assert_eq!(details[0].reason, ExhaustionReason::RateLimited);
                assert_eq!(details[1].provider_id, "p2");
                assert_eq!(details[1].reason, ExhaustionReason::QuotaExceeded);
                // The last underlying provider error rode along on `cause`.
                let inner = cause
                    .as_deref()
                    .expect("attempted slot must contribute a cause");
                match inner {
                    LlmError::Api { status, .. } => assert_eq!(*status, 402),
                    other => panic!("expected wrapped Api(402), got {other:?}"),
                }
            }
            other => panic!("expected AllProvidersExhausted, got {other:?}"),
        }
        // `thiserror`'s `#[source]` exposes `cause` through the
        // standard `Error::source` walker — assert the chain is intact
        // (#3745 rule). The wrapped error is `Box<LlmError>`, so the
        // top-level `source()` returns `&Box<LlmError>` as `&dyn Error`;
        // walking one more level (the Box's own `source`) lands on the
        // inner `LlmError` (which is what we actually care about). Both
        // levels must report the upstream Api(402) status — the Box's
        // Display delegates to the inner error.
        let src = std::error::Error::source(&err)
            .expect("AllProvidersExhausted with a wrapped cause must expose source()");
        assert!(
            src.to_string().contains("API error (402)"),
            "source Display should surface upstream Api(402), got {src}"
        );
    }

    // Review blocking #5: when at least one slot fails with a
    // NON-exhaustion reason (genuine 500, transport error), the chain
    // must propagate THAT error verbatim instead of wrapping it in
    // `AllProvidersExhausted`. The chain isn't actually "dry" — the
    // slot is broken — and callers depend on the distinction.
    #[tokio::test]
    async fn non_exhaustion_failure_propagates_raw_error() {
        // Driver that returns a plain HTTP 500 — failover_reason
        // classifies it as HttpError, which is transient and not
        // exhaustion-class, so the chain must NOT mark the slot in
        // the store and must NOT wrap into AllProvidersExhausted.
        struct ServerErrorDriver;

        #[async_trait]
        impl LlmDriver for ServerErrorDriver {
            async fn complete(
                &self,
                _req: CompletionRequest,
            ) -> Result<CompletionResponse, LlmError> {
                Err(LlmError::Api {
                    status: 500,
                    message: "internal server error".to_string(),
                    code: None,
                })
            }
        }

        let store = ProviderExhaustionStore::new();
        let chain = FallbackChain::new(vec![
            entry(Arc::new(ServerErrorDriver), "p1"),
            entry(Arc::new(ServerErrorDriver), "p2"),
        ])
        .with_exhaustion_store(store.clone());

        let err = chain.complete(test_request()).await.unwrap_err();
        match &err {
            LlmError::Api { status, .. } => assert_eq!(*status, 500),
            other => panic!("expected raw Api(500), got {other:?}"),
        }
        // Neither slot should be marked in the store — HTTP 500 is
        // transient, the slot may serve the next request fine.
        assert!(
            store.is_exhausted("p1").is_none(),
            "p1 must not be flagged exhausted on a transient HTTP 500"
        );
        assert!(
            store.is_exhausted("p2").is_none(),
            "p2 must not be flagged exhausted on a transient HTTP 500"
        );
    }

    // Review blocking #5: when slot 1 was pre-skipped (in the store)
    // and slot 2 was attempted-and-rate-limited (exhaustion-class),
    // EVERY slot is accounted for as an exhaustion-class outcome, so
    // the chain returns `AllProvidersExhausted` with `cause` = slot 2's
    // RateLimited error.
    #[tokio::test]
    async fn skip_plus_rate_limit_yields_typed_variant_with_cause() {
        let store = ProviderExhaustionStore::new();
        // Pre-mark p1.
        let until = Instant::now() + Duration::from_secs(60);
        store.mark_exhausted("p1", ExhaustionReason::QuotaExceeded, Some(until));

        let p2_driver = Arc::new(RateLimitedCountingDriver {
            calls: std::sync::atomic::AtomicUsize::new(0),
            retry_after_ms: 1,
        });
        let chain = FallbackChain::new(vec![
            entry(Arc::new(OkDriver("should-not-reach")), "p1"),
            ChainEntry {
                driver: p2_driver as Arc<dyn LlmDriver>,
                model_override: String::new(),
                provider_name: "p2".to_string(),
            },
        ])
        .with_rate_limit_sleep_ms(0)
        .with_exhaustion_store(store);

        let err = chain.complete(test_request()).await.unwrap_err();
        match err {
            LlmError::AllProvidersExhausted { details, cause } => {
                assert_eq!(details.len(), 2);
                assert_eq!(details[0].provider_id, "p1");
                assert_eq!(details[0].reason, ExhaustionReason::QuotaExceeded);
                assert_eq!(details[1].provider_id, "p2");
                assert_eq!(details[1].reason, ExhaustionReason::RateLimited);
                let inner = cause.expect("attempted slot must contribute a cause");
                assert!(
                    matches!(*inner, LlmError::RateLimited { .. }),
                    "cause should be the rate-limit failure from p2, got {inner:?}",
                );
            }
            other => panic!("expected AllProvidersExhausted, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn legacy_no_store_chain_preserves_old_error_shape() {
        // Without a store attached: the chain falls back to the historical
        // Api { message: "...all providers exhausted" } error so callers
        // that downcast on it keep working.
        let chain = FallbackChain::new(vec![
            entry(Arc::new(CreditExhaustedDriver), "p1"),
            entry(Arc::new(ModelUnavailableDriver), "p2"),
        ]);

        let err = chain.complete(test_request()).await.unwrap_err();
        assert!(
            !matches!(err, LlmError::AllProvidersExhausted { .. }),
            "without store, must NOT synthesize AllProvidersExhausted",
        );
    }

    #[test]
    fn exhaustion_reason_for_classifies_rate_limit_quota_auth() {
        assert_eq!(
            exhaustion_reason_for(&FailoverReason::RateLimit(None)),
            Some(ExhaustionReason::RateLimited)
        );
        assert_eq!(
            exhaustion_reason_for(&FailoverReason::CreditExhausted),
            Some(ExhaustionReason::QuotaExceeded)
        );
        assert_eq!(
            exhaustion_reason_for(&FailoverReason::AuthError),
            Some(ExhaustionReason::AuthFailed)
        );
    }

    #[test]
    fn exhaustion_reason_for_skips_transient_and_caller_errors() {
        // Transient errors don't park the slot — let the next call retry.
        assert_eq!(
            exhaustion_reason_for(&FailoverReason::ModelUnavailable),
            None
        );
        assert_eq!(exhaustion_reason_for(&FailoverReason::Timeout), None);
        assert_eq!(exhaustion_reason_for(&FailoverReason::HttpError), None);
        // Caller errors (ContextTooLong) and unclassifiable errors do not
        // mark the slot — the slot is fine.
        assert_eq!(exhaustion_reason_for(&FailoverReason::ContextTooLong), None);
        assert_eq!(exhaustion_reason_for(&FailoverReason::Unknown), None);
        // ChainExhausted is a terminal classification — the caller
        // *is* the chain that already exhausted, so we do not mark
        // its own slot in the parent store.
        assert_eq!(exhaustion_reason_for(&FailoverReason::ChainExhausted), None);
    }

    #[test]
    fn exhaustion_until_honours_rate_limit_retry_after() {
        // 5-second server hint is below MIN_RATE_LIMIT_EXHAUSTION_BACKOFF
        // (30 s) — must be floored to 30s. This is intentional: a very
        // short server-reported back-off doesn't earn us a window in
        // which to skip the slot, so we extend it to a useful minimum.
        let err = LlmError::RateLimited {
            retry_after_ms: 5_000,
            message: None,
        };
        let until = exhaustion_until_for(&err, &FailoverReason::RateLimit(Some(5_000)))
            .expect("rate-limit should set an `until`");
        let delta = until.saturating_duration_since(Instant::now());
        assert!(
            delta >= Duration::from_secs(25) && delta <= Duration::from_secs(35),
            "until should be ~30s (min floor) out, got {delta:?}"
        );

        // 5-minute server hint exceeds the floor and is preserved.
        let err_long = LlmError::RateLimited {
            retry_after_ms: 300_000,
            message: None,
        };
        let until_long = exhaustion_until_for(&err_long, &FailoverReason::RateLimit(Some(300_000)))
            .expect("rate-limit should set an `until`");
        let delta_long = until_long.saturating_duration_since(Instant::now());
        assert!(
            delta_long >= Duration::from_secs(290) && delta_long <= Duration::from_secs(310),
            "until should honour large hint, got {delta_long:?}"
        );
    }

    #[test]
    fn exhaustion_until_auth_failure_uses_long_backoff() {
        let err = LlmError::AuthenticationFailed("bad".to_string());
        let until = exhaustion_until_for(&err, &FailoverReason::AuthError)
            .expect("auth failure should set an `until`");
        let delta = until.saturating_duration_since(Instant::now());
        // DEFAULT_LONG_BACKOFF is 1h — anything ≥ 30 min is fine here.
        assert!(
            delta >= Duration::from_secs(30 * 60),
            "auth failure should use long backoff, got {delta:?}"
        );
    }
}
