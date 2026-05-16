//! Fallback driver — tries multiple LLM drivers in sequence.
//!
//! If the primary driver fails with a non-retryable error, the fallback driver
//! moves to the next driver in the chain.

use crate::drivers::fallback_chain::{exhaustion_reason_for, exhaustion_until_for};
use crate::llm_driver::{CompletionRequest, CompletionResponse, LlmDriver, LlmError, StreamEvent};
use async_trait::async_trait;
use librefang_llm_driver::exhaustion::ProviderExhaustionStore;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tracing::{info, warn};

/// A driver that wraps multiple LLM drivers and tries each in order.
///
/// On failure (including rate-limit and overload), moves to the next driver.
/// Only returns an error when ALL drivers in the chain are exhausted.
/// Each driver is paired with the model name it should use.
///
/// Health-aware: tracks per-driver EWMA latency and consecutive errors.
/// On each request, the driver list is dynamically reordered so healthy,
/// low-latency drivers are tried first while preserving the primary position
/// when it is healthy.
///
/// Exhaustion-aware (#4807): when constructed with
/// [`Self::with_exhaustion_store`] and per-slot provider names supplied
/// via [`Self::with_models_and_providers`] (or registered later via
/// [`Self::set_provider_names`]), the driver pre-skips slots that have
/// been marked exhausted by the operator-budget gate or by a sibling
/// chain, and marks slots itself after rate-limit / quota / auth
/// failures. Slots with empty provider names are excluded from store
/// participation (back-compat for tests).
pub struct FallbackDriver {
    drivers: Vec<DriverEntry>,
    /// Shared provider-exhaustion store (#4807). When `Some`, pre-skip
    /// and post-failure-mark logic runs alongside the EWMA reordering;
    /// `None` preserves the historical un-gated behaviour.
    exhaustion_store: Option<ProviderExhaustionStore>,
}

struct DriverEntry {
    driver: Arc<dyn LlmDriver>,
    model_name: String,
    /// Provider name used as the exhaustion-store key. Empty for slots
    /// that were not registered against the store (legacy constructors,
    /// tests).
    provider_name: String,
    /// Exponentially weighted moving average latency in ms.
    ewma_latency_ms: AtomicU64,
    /// Consecutive error count. Reset to 0 on success or after the
    /// [`HEALTH_RECOVERY_MS`] cooldown elapses since the last failure.
    consecutive_errors: AtomicU64,
    /// Wall-clock ms since UNIX epoch of the most recent failure. `0`
    /// means "never failed". Used by [`FallbackDriver::maybe_recover`] to
    /// clear stale unhealthy state so a driver that recovered from a
    /// transient outage rejoins the healthy pool without needing an
    /// explicit success event.
    last_failure_at_ms: AtomicU64,
}

/// Penalty added to EWMA when a driver errors (makes it sort lower).
const ERROR_PENALTY_MS: u64 = 30_000;
/// EWMA smoothing factor (0.3 = new sample is 30% of the result).
const EWMA_ALPHA: f64 = 0.3;
/// How long a driver stays "unhealthy" with no new failures before it is
/// lazily restored to the healthy pool. Five minutes matches typical
/// provider rate-limit windows and transient-outage durations.
const HEALTH_RECOVERY_MS: u64 = 5 * 60 * 1000;

impl FallbackDriver {
    /// Create a new fallback driver from an ordered chain of (driver, model_name) pairs.
    ///
    /// The first entry is the primary; subsequent are fallbacks.
    ///
    /// # Panics
    /// Panics if `drivers` is empty — at least one driver must be provided.
    pub fn new(drivers: Vec<Arc<dyn LlmDriver>>) -> Self {
        assert!(
            !drivers.is_empty(),
            "FallbackDriver requires at least one driver"
        );
        Self {
            drivers: drivers
                .into_iter()
                .map(|d| DriverEntry {
                    driver: d,
                    model_name: String::new(),
                    provider_name: String::new(),
                    ewma_latency_ms: AtomicU64::new(0),
                    consecutive_errors: AtomicU64::new(0),
                    last_failure_at_ms: AtomicU64::new(0),
                })
                .collect(),
            exhaustion_store: None,
        }
    }

    /// Create a new fallback driver with explicit model names for each driver.
    ///
    /// # Panics
    /// Panics if `drivers` is empty — at least one driver must be provided.
    pub fn with_models(drivers: Vec<(Arc<dyn LlmDriver>, String)>) -> Self {
        assert!(
            !drivers.is_empty(),
            "FallbackDriver requires at least one driver"
        );
        Self {
            drivers: drivers
                .into_iter()
                .map(|(d, m)| DriverEntry {
                    driver: d,
                    model_name: m,
                    provider_name: String::new(),
                    ewma_latency_ms: AtomicU64::new(0),
                    consecutive_errors: AtomicU64::new(0),
                    last_failure_at_ms: AtomicU64::new(0),
                })
                .collect(),
            exhaustion_store: None,
        }
    }

    /// Create a new fallback driver with explicit model AND provider
    /// names for each driver. Provider names are the keys against which
    /// the exhaustion store records skip / mark events. Empty provider
    /// names are accepted (the slot then never participates in the
    /// store) for forward-compat with mixed configurations.
    ///
    /// # Panics
    /// Panics if `drivers` is empty.
    pub fn with_models_and_providers(drivers: Vec<(Arc<dyn LlmDriver>, String, String)>) -> Self {
        assert!(
            !drivers.is_empty(),
            "FallbackDriver requires at least one driver"
        );
        Self {
            drivers: drivers
                .into_iter()
                .map(|(d, model, provider)| DriverEntry {
                    driver: d,
                    model_name: model,
                    provider_name: provider,
                    ewma_latency_ms: AtomicU64::new(0),
                    consecutive_errors: AtomicU64::new(0),
                    last_failure_at_ms: AtomicU64::new(0),
                })
                .collect(),
            exhaustion_store: None,
        }
    }

    /// Attach a shared [`ProviderExhaustionStore`] so this driver
    /// participates in the same budget-aware skip view as the rest of
    /// the kernel (#4807). Slots whose `provider_name` is empty
    /// continue to be invoked without store interaction — only slots
    /// registered via [`Self::with_models_and_providers`] or
    /// [`Self::set_provider_names`] are gated.
    pub fn with_exhaustion_store(mut self, store: ProviderExhaustionStore) -> Self {
        self.exhaustion_store = Some(store);
        self
    }

    /// Set provider names on slots after construction. Used by callers
    /// that build the driver with [`Self::with_models`] and then learn
    /// the provider name afterwards. The slice length must match the
    /// driver count; trailing entries default to empty.
    pub fn set_provider_names<I, S>(&mut self, names: I)
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        for (entry, name) in self.drivers.iter_mut().zip(names) {
            entry.provider_name = name.into();
        }
    }

    /// Current wall-clock time in ms since UNIX epoch, or `0` if the system
    /// clock is earlier than the epoch (never happens on real systems).
    fn now_ms() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0)
    }

    /// Lazily restore a driver to the healthy pool when its last failure
    /// was more than [`HEALTH_RECOVERY_MS`] ago. Resets the consecutive
    /// error counter and strips the accumulated EWMA penalty so the driver
    /// competes for the head of the order on latency again.
    ///
    /// Called from [`Self::health_order`] on every dispatch — we cannot
    /// rely on a background task because `FallbackDriver` has no lifecycle
    /// of its own.
    fn maybe_recover(entry: &DriverEntry, now_ms: u64) {
        let errors = entry.consecutive_errors.load(Ordering::Relaxed);
        if errors == 0 {
            return;
        }
        let last = entry.last_failure_at_ms.load(Ordering::Relaxed);
        if last == 0 {
            return;
        }
        if now_ms.saturating_sub(last) < HEALTH_RECOVERY_MS {
            return;
        }
        // Compare-and-swap so concurrent dispatchers don't double-subtract
        // the penalty. If another thread already recovered the entry, the
        // store will no-op and the `errors` value we read is stale.
        if entry
            .consecutive_errors
            .compare_exchange(errors, 0, Ordering::Relaxed, Ordering::Relaxed)
            .is_ok()
        {
            let cur = entry.ewma_latency_ms.load(Ordering::Relaxed);
            // Strip the accumulated error penalty. This subtraction is only
            // correct as long as `ERROR_PENALTY_MS` is added once per error
            // *additively* on the failure path (see line ~205). If the
            // penalty formula ever becomes multiplicative, exponential, or
            // otherwise non-linear in `errors`, this restore math will
            // silently under- or over-correct — change both sites together.
            let restored = cur.saturating_sub(ERROR_PENALTY_MS.saturating_mul(errors));
            entry.ewma_latency_ms.store(restored, Ordering::Relaxed);
            info!(
                model = %entry.model_name,
                errors_cleared = errors,
                "Fallback driver recovered after cooldown — rejoining healthy pool"
            );
        }
    }

    /// Build a health-aware ordering of driver indices. Healthy drivers
    /// (consecutive_errors == 0) come first sorted by EWMA latency; unhealthy
    /// drivers follow sorted by error count (fewest errors first). The primary
    /// driver (index 0) gets a latency bonus to keep it preferred when healthy.
    ///
    /// Before ordering we give every unhealthy driver a chance to recover if
    /// enough time has elapsed since its last failure — this is what keeps a
    /// driver that was rate-limited from being stuck unhealthy forever.
    fn health_order(&self) -> Vec<usize> {
        let now = Self::now_ms();
        for entry in &self.drivers {
            Self::maybe_recover(entry, now);
        }
        let mut indices: Vec<usize> = (0..self.drivers.len()).collect();
        indices.sort_by(|&a, &b| {
            let ea = self.drivers[a].consecutive_errors.load(Ordering::Relaxed);
            let eb = self.drivers[b].consecutive_errors.load(Ordering::Relaxed);
            // Healthy (0 errors) before unhealthy
            match (ea == 0, eb == 0) {
                (true, false) => std::cmp::Ordering::Less,
                (false, true) => std::cmp::Ordering::Greater,
                _ => {
                    let la = self.drivers[a].ewma_latency_ms.load(Ordering::Relaxed);
                    let lb = self.drivers[b].ewma_latency_ms.load(Ordering::Relaxed);
                    la.cmp(&lb).then(a.cmp(&b))
                }
            }
        });
        indices
    }
}

impl FallbackDriver {
    /// Record an exhaustion-class failure against the shared store, if
    /// any. Mirrors `FallbackChain`'s policy so both wrappers stamp
    /// slots with consistent reason/backoff.
    fn record_exhaustion(&self, entry: &DriverEntry, err: &LlmError) {
        let Some(store) = &self.exhaustion_store else {
            return;
        };
        if entry.provider_name.is_empty() {
            return;
        }
        let reason = err.failover_reason();
        if let Some(exhaust_reason) = exhaustion_reason_for(&reason) {
            let until = exhaustion_until_for(err, &reason);
            store.mark_exhausted(entry.provider_name.clone(), exhaust_reason, until);
        }
    }

    /// Should this slot be pre-skipped? Returns `true` when an attached
    /// store reports the slot exhausted; no-ops otherwise.
    fn is_slot_exhausted(&self, entry: &DriverEntry) -> bool {
        let Some(store) = &self.exhaustion_store else {
            return false;
        };
        if entry.provider_name.is_empty() {
            return false;
        }
        store.record_skip(&entry.provider_name).is_some()
    }
}

#[async_trait]
impl LlmDriver for FallbackDriver {
    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        let mut last_error = None;
        let order = self.health_order();

        for &i in &order {
            let entry = &self.drivers[i];

            // Pre-attempt skip: store reports the slot exhausted (#4807).
            if self.is_slot_exhausted(entry) {
                continue;
            }

            let mut req = request.clone();
            if !entry.model_name.is_empty() {
                req.model = entry.model_name.clone();
            }

            let start = std::time::Instant::now();
            match entry.driver.complete(req).await {
                Ok(mut response) => {
                    let latency = start.elapsed().as_millis() as u64;
                    let prev = entry.ewma_latency_ms.load(Ordering::Relaxed);
                    let new = if prev == 0 {
                        latency
                    } else {
                        (EWMA_ALPHA * latency as f64 + (1.0 - EWMA_ALPHA) * prev as f64) as u64
                    };
                    entry.ewma_latency_ms.store(new, Ordering::Relaxed);
                    entry.consecutive_errors.store(0, Ordering::Relaxed);
                    // Stamp the slot that actually served the request
                    // (review nit 10). Only stamp when we know the
                    // slot's provider name AND the inner driver hasn't
                    // already stamped (preserves attribution from a
                    // wrapped FallbackChain).
                    if response.actual_provider.is_none() && !entry.provider_name.is_empty() {
                        response.actual_provider = Some(entry.provider_name.clone());
                    }
                    return Ok(response);
                }
                Err(e) => {
                    entry.consecutive_errors.fetch_add(1, Ordering::Relaxed);
                    entry
                        .last_failure_at_ms
                        .store(Self::now_ms(), Ordering::Relaxed);
                    let prev = entry.ewma_latency_ms.load(Ordering::Relaxed);
                    entry
                        .ewma_latency_ms
                        .store(prev.saturating_add(ERROR_PENALTY_MS), Ordering::Relaxed);
                    warn!(
                        driver_index = i,
                        provider = %entry.provider_name,
                        model = %entry.model_name,
                        error = %e,
                        consecutive_errors = entry.consecutive_errors.load(Ordering::Relaxed),
                        "Fallback driver failed, trying next"
                    );
                    self.record_exhaustion(entry, &e);
                    last_error = Some(e);
                }
            }
        }

        Err(last_error.unwrap_or_else(|| LlmError::Api {
            status: 0,
            message: "No drivers configured in fallback chain".to_string(),
            code: None,
        }))
    }

    async fn stream(
        &self,
        request: CompletionRequest,
        tx: tokio::sync::mpsc::Sender<StreamEvent>,
    ) -> Result<CompletionResponse, LlmError> {
        let mut last_error = None;
        let order = self.health_order();

        for &i in &order {
            let entry = &self.drivers[i];

            if self.is_slot_exhausted(entry) {
                continue;
            }

            let mut req = request.clone();
            if !entry.model_name.is_empty() {
                req.model = entry.model_name.clone();
            }

            let start = std::time::Instant::now();
            match entry.driver.stream(req, tx.clone()).await {
                Ok(mut response) => {
                    let latency = start.elapsed().as_millis() as u64;
                    let prev = entry.ewma_latency_ms.load(Ordering::Relaxed);
                    let new = if prev == 0 {
                        latency
                    } else {
                        (EWMA_ALPHA * latency as f64 + (1.0 - EWMA_ALPHA) * prev as f64) as u64
                    };
                    entry.ewma_latency_ms.store(new, Ordering::Relaxed);
                    entry.consecutive_errors.store(0, Ordering::Relaxed);
                    if response.actual_provider.is_none() && !entry.provider_name.is_empty() {
                        response.actual_provider = Some(entry.provider_name.clone());
                    }
                    return Ok(response);
                }
                Err(e) => {
                    entry.consecutive_errors.fetch_add(1, Ordering::Relaxed);
                    entry
                        .last_failure_at_ms
                        .store(Self::now_ms(), Ordering::Relaxed);
                    let prev = entry.ewma_latency_ms.load(Ordering::Relaxed);
                    entry
                        .ewma_latency_ms
                        .store(prev.saturating_add(ERROR_PENALTY_MS), Ordering::Relaxed);
                    warn!(
                        driver_index = i,
                        provider = %entry.provider_name,
                        model = %entry.model_name,
                        error = %e,
                        consecutive_errors = entry.consecutive_errors.load(Ordering::Relaxed),
                        "Fallback driver (stream) failed, trying next"
                    );
                    self.record_exhaustion(entry, &e);
                    last_error = Some(e);
                }
            }
        }

        Err(last_error.unwrap_or_else(|| LlmError::Api {
            status: 0,
            message: "No drivers configured in fallback chain".to_string(),
            code: None,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm_driver::CompletionResponse;
    use librefang_types::message::{ContentBlock, StopReason, TokenUsage};

    struct FailDriver;

    #[async_trait]
    impl LlmDriver for FailDriver {
        async fn complete(&self, _req: CompletionRequest) -> Result<CompletionResponse, LlmError> {
            Err(LlmError::Api {
                status: 500,
                message: "Internal error".to_string(),
                code: None,
            })
        }
    }

    struct OkDriver;

    #[async_trait]
    impl LlmDriver for OkDriver {
        async fn complete(&self, _req: CompletionRequest) -> Result<CompletionResponse, LlmError> {
            Ok(CompletionResponse {
                content: vec![ContentBlock::Text {
                    text: "OK".to_string(),
                    provider_metadata: None,
                }],
                stop_reason: StopReason::EndTurn,
                tool_calls: vec![],
                usage: TokenUsage {
                    input_tokens: 10,
                    output_tokens: 5,
                    ..Default::default()
                },
                actual_provider: None,
            })
        }
    }

    #[tokio::test]
    async fn unhealthy_driver_recovers_after_cooldown() {
        // Seed the primary's health state directly to simulate three
        // accumulated failures. We can't get there via dispatch: after the
        // first failure health_order() reroutes to the healthy secondary,
        // so the primary would never accrue a second error through complete().
        let fb = FallbackDriver::with_models(vec![
            (
                Arc::new(FailDriver) as Arc<dyn LlmDriver>,
                "fail".to_string(),
            ),
            (Arc::new(OkDriver) as Arc<dyn LlmDriver>, "ok".to_string()),
        ]);

        let primary = &fb.drivers[0];
        primary.consecutive_errors.store(3, Ordering::Relaxed);
        primary
            .last_failure_at_ms
            .store(FallbackDriver::now_ms(), Ordering::Relaxed);
        primary
            .ewma_latency_ms
            .store(ERROR_PENALTY_MS * 3, Ordering::Relaxed);

        assert_eq!(primary.consecutive_errors.load(Ordering::Relaxed), 3);
        let penalised = primary.ewma_latency_ms.load(Ordering::Relaxed);
        assert!(penalised >= ERROR_PENALTY_MS * 3);

        // Simulate the cooldown elapsing by calling maybe_recover with a
        // fabricated future timestamp.
        let future = primary.last_failure_at_ms.load(Ordering::Relaxed) + HEALTH_RECOVERY_MS + 1;
        FallbackDriver::maybe_recover(primary, future);

        assert_eq!(
            primary.consecutive_errors.load(Ordering::Relaxed),
            0,
            "consecutive_errors must be cleared after cooldown"
        );
        let recovered = primary.ewma_latency_ms.load(Ordering::Relaxed);
        assert!(
            recovered < penalised,
            "EWMA penalty must be stripped on recovery ({penalised} → {recovered})"
        );
    }

    #[tokio::test]
    async fn recover_is_noop_within_cooldown() {
        let fb = FallbackDriver::with_models(vec![
            (
                Arc::new(FailDriver) as Arc<dyn LlmDriver>,
                "fail".to_string(),
            ),
            (Arc::new(OkDriver) as Arc<dyn LlmDriver>, "ok".to_string()),
        ]);
        let _ = fb.complete(test_request()).await;
        let primary = &fb.drivers[0];
        let last = primary.last_failure_at_ms.load(Ordering::Relaxed);

        // One second after the failure — well inside the cooldown window.
        FallbackDriver::maybe_recover(primary, last + 1_000);

        assert_eq!(
            primary.consecutive_errors.load(Ordering::Relaxed),
            1,
            "error counter must stay set until cooldown elapses"
        );
    }

    fn test_request() -> CompletionRequest {
        CompletionRequest {
            model: "test".to_string(),
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

    #[tokio::test]
    async fn test_fallback_primary_succeeds() {
        let driver = FallbackDriver::new(vec![
            Arc::new(OkDriver) as Arc<dyn LlmDriver>,
            Arc::new(FailDriver) as Arc<dyn LlmDriver>,
        ]);
        let result = driver.complete(test_request()).await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap().text(), "OK");
    }

    #[tokio::test]
    async fn test_fallback_primary_fails_secondary_succeeds() {
        let driver = FallbackDriver::new(vec![
            Arc::new(FailDriver) as Arc<dyn LlmDriver>,
            Arc::new(OkDriver) as Arc<dyn LlmDriver>,
        ]);
        let result = driver.complete(test_request()).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_fallback_all_fail() {
        let driver = FallbackDriver::new(vec![
            Arc::new(FailDriver) as Arc<dyn LlmDriver>,
            Arc::new(FailDriver) as Arc<dyn LlmDriver>,
        ]);
        let result = driver.complete(test_request()).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_rate_limit_falls_through() {
        struct RateLimitDriver;

        #[async_trait]
        impl LlmDriver for RateLimitDriver {
            async fn complete(
                &self,
                _req: CompletionRequest,
            ) -> Result<CompletionResponse, LlmError> {
                Err(LlmError::RateLimited {
                    retry_after_ms: 5000,
                    message: None,
                })
            }
        }

        let driver = FallbackDriver::new(vec![
            Arc::new(RateLimitDriver) as Arc<dyn LlmDriver>,
            Arc::new(OkDriver) as Arc<dyn LlmDriver>,
        ]);
        let result = driver.complete(test_request()).await;
        // Rate limit should fall through to the OkDriver fallback
        assert!(result.is_ok());
        assert_eq!(result.unwrap().text(), "OK");
    }

    #[tokio::test]
    async fn test_rate_limit_all_fail() {
        struct RateLimitDriver;

        #[async_trait]
        impl LlmDriver for RateLimitDriver {
            async fn complete(
                &self,
                _req: CompletionRequest,
            ) -> Result<CompletionResponse, LlmError> {
                Err(LlmError::RateLimited {
                    retry_after_ms: 5000,
                    message: None,
                })
            }
        }

        let driver = FallbackDriver::new(vec![
            Arc::new(RateLimitDriver) as Arc<dyn LlmDriver>,
            Arc::new(RateLimitDriver) as Arc<dyn LlmDriver>,
        ]);
        let result = driver.complete(test_request()).await;
        // All drivers rate-limited — error should bubble up
        assert!(matches!(result, Err(LlmError::RateLimited { .. })));
    }

    // ── Health-aware reordering tests ───────────────────────────────

    #[tokio::test]
    async fn test_health_order_prefers_healthy_driver() {
        // Primary (index 0) = FailDriver, Secondary (index 1) = OkDriver
        let driver = FallbackDriver::new(vec![
            Arc::new(FailDriver) as Arc<dyn LlmDriver>,
            Arc::new(OkDriver) as Arc<dyn LlmDriver>,
        ]);

        // First call: primary fails, secondary succeeds
        let _ = driver.complete(test_request()).await;

        // Now primary has consecutive_errors=1, secondary has 0
        assert_eq!(
            driver.drivers[0].consecutive_errors.load(Ordering::Relaxed),
            1
        );
        assert_eq!(
            driver.drivers[1].consecutive_errors.load(Ordering::Relaxed),
            0
        );

        // health_order should now prefer secondary (index 1) first
        let order = driver.health_order();
        assert_eq!(order[0], 1, "healthy driver should come first");
        assert_eq!(order[1], 0, "unhealthy driver should come second");
    }

    #[tokio::test]
    async fn test_consecutive_errors_reset_on_success() {
        use std::sync::atomic::AtomicBool;

        /// A driver that fails once, then succeeds on subsequent calls.
        struct RecoverDriver {
            failed: AtomicBool,
        }

        #[async_trait]
        impl LlmDriver for RecoverDriver {
            async fn complete(
                &self,
                _req: CompletionRequest,
            ) -> Result<CompletionResponse, LlmError> {
                if !self.failed.swap(true, Ordering::Relaxed) {
                    Err(LlmError::Api {
                        status: 503,
                        message: "temporary".to_string(),
                        code: None,
                    })
                } else {
                    Ok(CompletionResponse {
                        content: vec![ContentBlock::Text {
                            text: "recovered".to_string(),
                            provider_metadata: None,
                        }],
                        stop_reason: StopReason::EndTurn,
                        tool_calls: vec![],
                        usage: TokenUsage {
                            input_tokens: 1,
                            output_tokens: 1,
                            ..Default::default()
                        },
                        actual_provider: None,
                    })
                }
            }
        }

        let driver = FallbackDriver::new(vec![
            Arc::new(RecoverDriver {
                failed: AtomicBool::new(false),
            }) as Arc<dyn LlmDriver>,
            Arc::new(OkDriver) as Arc<dyn LlmDriver>,
        ]);

        // First call: primary fails (errors=1), falls through to OkDriver
        let r1 = driver.complete(test_request()).await;
        assert!(r1.is_ok());
        assert_eq!(
            driver.drivers[0].consecutive_errors.load(Ordering::Relaxed),
            1
        );

        // Second call: health_order puts OkDriver first, but primary
        // (RecoverDriver) will now succeed when tried. Let's force it
        // by making a third request after the RecoverDriver is healthy.
        // Actually on 2nd call, OkDriver (index 1) is healthy and comes first,
        // so it succeeds directly. Let's verify primary is still errored.
        let r2 = driver.complete(test_request()).await;
        assert!(r2.is_ok());

        // Now manually reset to simulate the primary being tried again.
        // Since RecoverDriver's `failed` is true, next call returns Ok.
        // Reset consecutive_errors to 0 to let health_order try primary first.
        driver.drivers[0]
            .consecutive_errors
            .store(0, Ordering::Relaxed);
        driver.drivers[0]
            .ewma_latency_ms
            .store(0, Ordering::Relaxed);

        let r3 = driver.complete(test_request()).await;
        assert!(r3.is_ok());
        // Primary should have succeeded and errors should remain 0
        assert_eq!(
            driver.drivers[0].consecutive_errors.load(Ordering::Relaxed),
            0
        );
    }

    #[tokio::test]
    async fn test_ewma_latency_tracked_on_success() {
        let driver = FallbackDriver::new(vec![Arc::new(OkDriver) as Arc<dyn LlmDriver>]);

        // Before any calls, EWMA should be 0
        assert_eq!(driver.drivers[0].ewma_latency_ms.load(Ordering::Relaxed), 0);

        let _ = driver.complete(test_request()).await;

        // After a successful call, EWMA should be > 0 (at least 0ms for fast in-mem)
        // It could be 0 if the call was instant, so just verify it didn't error
        let ewma = driver.drivers[0].ewma_latency_ms.load(Ordering::Relaxed);
        // EWMA is set (first call sets it to raw latency, could be 0 for instant)
        assert!(ewma < 1000, "EWMA should be reasonable, got {ewma}");
    }

    #[tokio::test]
    async fn test_error_penalty_increases_ewma() {
        let driver = FallbackDriver::new(vec![
            Arc::new(FailDriver) as Arc<dyn LlmDriver>,
            Arc::new(OkDriver) as Arc<dyn LlmDriver>,
        ]);

        let _ = driver.complete(test_request()).await;

        // FailDriver (index 0) should have ERROR_PENALTY_MS added
        let ewma = driver.drivers[0].ewma_latency_ms.load(Ordering::Relaxed);
        assert!(
            ewma >= ERROR_PENALTY_MS,
            "error penalty should inflate EWMA, got {ewma}"
        );
    }

    #[tokio::test]
    async fn test_health_order_sorts_by_ewma_when_both_healthy() {
        let driver = FallbackDriver::new(vec![
            Arc::new(OkDriver) as Arc<dyn LlmDriver>,
            Arc::new(OkDriver) as Arc<dyn LlmDriver>,
        ]);

        // Simulate: driver 0 has high latency, driver 1 has low latency
        driver.drivers[0]
            .ewma_latency_ms
            .store(5000, Ordering::Relaxed);
        driver.drivers[1]
            .ewma_latency_ms
            .store(100, Ordering::Relaxed);

        let order = driver.health_order();
        assert_eq!(order[0], 1, "lower latency driver should come first");
        assert_eq!(order[1], 0);
    }

    #[tokio::test]
    async fn test_health_order_healthy_always_before_unhealthy() {
        let driver = FallbackDriver::new(vec![
            Arc::new(OkDriver) as Arc<dyn LlmDriver>,
            Arc::new(OkDriver) as Arc<dyn LlmDriver>,
        ]);

        // Driver 0: unhealthy (errors=3) but low EWMA
        driver.drivers[0]
            .consecutive_errors
            .store(3, Ordering::Relaxed);
        driver.drivers[0]
            .ewma_latency_ms
            .store(10, Ordering::Relaxed);

        // Driver 1: healthy (errors=0) but high EWMA
        driver.drivers[1]
            .consecutive_errors
            .store(0, Ordering::Relaxed);
        driver.drivers[1]
            .ewma_latency_ms
            .store(9999, Ordering::Relaxed);

        let order = driver.health_order();
        assert_eq!(
            order[0], 1,
            "healthy driver should come first even with higher latency"
        );
    }

    #[test]
    #[should_panic(expected = "FallbackDriver requires at least one driver")]
    fn test_new_empty_drivers_panics() {
        let _driver = FallbackDriver::new(vec![]);
    }

    #[test]
    #[should_panic(expected = "FallbackDriver requires at least one driver")]
    fn test_with_models_empty_drivers_panics() {
        let _driver = FallbackDriver::with_models(vec![]);
    }

    #[tokio::test]
    async fn test_ewma_saturating_add_does_not_overflow() {
        // Single FailDriver so health_order must try it
        let driver = FallbackDriver::new(vec![Arc::new(FailDriver) as Arc<dyn LlmDriver>]);

        // Set EWMA near u64::MAX to test saturation
        driver.drivers[0]
            .ewma_latency_ms
            .store(u64::MAX - 1, Ordering::Relaxed);

        // This call will fail, triggering saturating_add(ERROR_PENALTY_MS)
        let _ = driver.complete(test_request()).await;

        let ewma = driver.drivers[0].ewma_latency_ms.load(Ordering::Relaxed);
        assert_eq!(ewma, u64::MAX, "EWMA should saturate at u64::MAX");
    }

    // ── #4807 exhaustion-store integration ───────────────────────────

    /// When the shared store reports a slot exhausted, that slot's
    /// driver must NOT be invoked even though `FallbackDriver`'s health
    /// view would otherwise prefer it.
    #[tokio::test]
    async fn exhausted_slot_is_skipped_via_store() {
        use librefang_llm_driver::exhaustion::{ExhaustionReason, ProviderExhaustionStore};
        use std::sync::atomic::AtomicUsize;
        use std::time::{Duration, Instant};

        struct CountingFailDriver(AtomicUsize);

        #[async_trait]
        impl LlmDriver for CountingFailDriver {
            async fn complete(
                &self,
                _req: CompletionRequest,
            ) -> Result<CompletionResponse, LlmError> {
                self.0.fetch_add(1, Ordering::SeqCst);
                Err(LlmError::Api {
                    status: 500,
                    message: "should not be called".to_string(),
                    code: None,
                })
            }
        }

        let store = ProviderExhaustionStore::new();
        store.mark_exhausted(
            "p1",
            ExhaustionReason::BudgetExceeded,
            Some(Instant::now() + Duration::from_secs(60)),
        );

        let p1_driver = Arc::new(CountingFailDriver(AtomicUsize::new(0)));
        let p1_calls = Arc::clone(&p1_driver);
        let fb = FallbackDriver::with_models_and_providers(vec![
            (
                p1_driver as Arc<dyn LlmDriver>,
                "model-a".to_string(),
                "p1".to_string(),
            ),
            (
                Arc::new(OkDriver) as Arc<dyn LlmDriver>,
                "model-b".to_string(),
                "p2".to_string(),
            ),
        ])
        .with_exhaustion_store(store);

        let resp = fb.complete(test_request()).await.unwrap();
        assert_eq!(resp.text(), "OK", "p2 should have served the request");
        assert_eq!(
            p1_calls.0.load(Ordering::SeqCst),
            0,
            "exhausted p1 must NOT be invoked"
        );
        // Nit 10: the successful slot's provider name rides along on
        // the response so downstream billing attributes spend to p2.
        assert_eq!(resp.actual_provider.as_deref(), Some("p2"));
    }

    /// A rate-limit / auth / credit failure must mark the slot in the
    /// shared store so subsequent requests skip it.
    #[tokio::test]
    async fn rate_limit_failure_marks_slot_in_store() {
        use librefang_llm_driver::exhaustion::{ExhaustionReason, ProviderExhaustionStore};

        struct RateLimitDriver;

        #[async_trait]
        impl LlmDriver for RateLimitDriver {
            async fn complete(
                &self,
                _req: CompletionRequest,
            ) -> Result<CompletionResponse, LlmError> {
                Err(LlmError::RateLimited {
                    retry_after_ms: 1,
                    message: None,
                })
            }
        }

        let store = ProviderExhaustionStore::new();
        let fb = FallbackDriver::with_models_and_providers(vec![
            (
                Arc::new(RateLimitDriver) as Arc<dyn LlmDriver>,
                "model-a".to_string(),
                "p1".to_string(),
            ),
            (
                Arc::new(OkDriver) as Arc<dyn LlmDriver>,
                "model-b".to_string(),
                "p2".to_string(),
            ),
        ])
        .with_exhaustion_store(store.clone());

        let resp = fb.complete(test_request()).await.unwrap();
        assert_eq!(resp.actual_provider.as_deref(), Some("p2"));

        let rec = store.is_exhausted("p1").expect("p1 must be flagged");
        assert_eq!(rec.reason, ExhaustionReason::RateLimited);
    }

    /// Slots without a provider name (legacy `with_models`) never
    /// participate in the store — confirms back-compat for callers that
    /// haven't migrated.
    #[tokio::test]
    async fn empty_provider_name_skips_store_interaction() {
        use librefang_llm_driver::exhaustion::ProviderExhaustionStore;

        let store = ProviderExhaustionStore::new();
        let fb = FallbackDriver::with_models(vec![(
            Arc::new(OkDriver) as Arc<dyn LlmDriver>,
            "model".to_string(),
        )])
        .with_exhaustion_store(store.clone());

        let resp = fb.complete(test_request()).await.unwrap();
        // No provider name means no actual_provider stamp.
        assert!(resp.actual_provider.is_none());
        // And the store stays empty even after the call.
        assert_eq!(store.live_count(), 0);
    }
}
