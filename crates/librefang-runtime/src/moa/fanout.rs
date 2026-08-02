//! Parallel advisor fan-out engine.
//!
//! Resolves each [`MoaSlot`] to a concrete driver via the public driver
//! factory, then runs all advisors concurrently against the flattened advisory
//! view. Advisors are blind (no tools) and bounded by a semaphore; each is
//! wrapped in a timeout so one slow advisor cannot stall the turn.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;

use librefang_llm_driver::{CompletionRequest, CompletionResponse, DriverConfig, LlmDriver};
use librefang_types::config::{KernelConfig, MoaSlot};
use librefang_types::message::{Message, TokenUsage};
use tokio::sync::{broadcast, Semaphore};
use tracing::warn;

use crate::moa::progress::MoaProgressEvent;
use crate::moa::ADVISORY_SYSTEM_PROMPT;
use crate::model_catalog::ModelCatalog;

/// Maximum number of advisors dispatched concurrently.
const MAX_CONCURRENT_ADVISORS: usize = 8;

/// Default per-advisor timeout when the preset does not override it.
const DEFAULT_REFERENCE_TIMEOUT_SECS: u64 = 900;

/// Default advisor sampling temperature.
const DEFAULT_REFERENCE_TEMPERATURE: f32 = 0.7;

/// Safety fraction reserved below the advisor's context window.
///
/// The chars/4 estimator UNDER-counts on code/JSON-heavy transcripts (those
/// tokenize less favorably than plain prose), so the budget is deliberately
/// pulled 10% below the true window rather than trusting the estimate.
const CONTEXT_SAFETY_FRACTION: f64 = 0.10;

/// Output reserve when the preset sets no `reference_max_tokens`.
const DEFAULT_OUTPUT_RESERVE: usize = 8192;

/// Rough characters-per-token ratio used by the trimming estimator.
const CHARS_PER_TOKEN: usize = 4;

/// The outcome of a single advisor call.
#[derive(Debug, Clone)]
pub struct AdvisorResult {
    /// Human-readable slot label (`provider/model`).
    pub label: String,
    /// Provider name.
    pub provider: String,
    /// Model identifier.
    pub model: String,
    /// Temperature used for the call.
    pub temperature: f32,
    /// Number of messages in the advisory view sent to this advisor.
    pub input_messages: usize,
    /// The advisor's text output (or a failure sentinel).
    pub text: String,
    /// Token usage reported by the provider.
    pub usage: TokenUsage,
    /// Estimated cost in USD (0.0 when pricing is unknown).
    pub cost: f64,
    /// Whether this advisor failed (timeout, driver error, empty output).
    pub failed: bool,
}

impl AdvisorResult {
    /// Whether this result carries usable advice.
    pub fn is_success(&self) -> bool {
        !self.failed && !self.text.trim().is_empty()
    }
}

/// Human-readable label for a slot.
pub fn slot_label(slot: &MoaSlot) -> String {
    format!("{}/{}", slot.provider, slot.model)
}

/// Resolve a slot to a concrete driver using the public driver factory.
///
/// Credentials resolve from the slot's `api_key_env` override, then the
/// config's `provider_api_keys` mapping, then the `{PROVIDER}_API_KEY`
/// convention. Returns `None` only if the factory rejects the config.
pub fn resolve_slot_driver(
    slot: &MoaSlot,
    config: &KernelConfig,
    catalog: Option<&ModelCatalog>,
) -> Option<Arc<dyn LlmDriver>> {
    let api_key = if let Some(env_var) = slot.api_key_env.as_deref().filter(|s| !s.is_empty()) {
        std::env::var(env_var).ok()
    } else {
        let env_var = config.resolve_api_key_env(&slot.provider);
        std::env::var(&env_var).ok()
    };

    let driver_config = DriverConfig {
        provider: slot.provider.clone(),
        api_key,
        base_url: slot
            .base_url
            .clone()
            .or_else(|| config.provider_urls.get(&slot.provider).cloned())
            // Custom providers registered via the dashboard carry their base_url
            // in the model catalog, not in `[provider_urls]` (which is the
            // boot-time snapshot). Mirror the kernel's `lookup_provider_url`
            // fallback so a MoA slot pointing at a runtime-added provider (e.g.
            // a self-hosted OpenAI-compatible endpoint) resolves the same way a
            // directly-configured agent model does — otherwise `create_driver`
            // rejects the slot for lack of a base_url.
            .or_else(|| {
                catalog
                    .and_then(|c| c.get_provider(&slot.provider))
                    .map(|p| p.base_url.clone())
                    .filter(|u| !u.is_empty())
            }),
        proxy_url: config.provider_proxy_urls.get(&slot.provider).cloned(),
        request_timeout_secs: config
            .provider_request_timeout_secs
            .get(&slot.provider)
            .copied(),
        emit_caller_trace_headers: config.telemetry.emit_caller_trace_headers,
        ..DriverConfig::default()
    };

    match librefang_llm_drivers::drivers::create_driver(&driver_config) {
        Ok(driver) => Some(driver),
        Err(e) => {
            warn!(
                provider = %slot.provider,
                model = %slot.model,
                error = %e,
                "MoA advisor driver init failed"
            );
            None
        }
    }
}

/// Estimate the USD cost of a call from catalog pricing. Returns 0.0 when the
/// model has no known pricing.
fn estimate_cost(catalog: Option<&ModelCatalog>, model: &str, usage: &TokenUsage) -> f64 {
    let Some(catalog) = catalog else {
        return 0.0;
    };
    let Some((input_per_m, output_per_m)) = catalog.pricing(model) else {
        return 0.0;
    };
    let input_cost = usage.input_tokens as f64 / 1_000_000.0 * input_per_m;
    let output_cost = usage.output_tokens as f64 / 1_000_000.0 * output_per_m;
    input_cost + output_cost
}

/// Estimate the token count of a set of frames plus the advisory system
/// prompt, using the rough chars/4 heuristic.
fn estimate_tokens(messages: &[Message]) -> usize {
    let body: usize = messages
        .iter()
        .map(|m| m.content.text_content().chars().count())
        .sum();
    (body + ADVISORY_SYSTEM_PROMPT.chars().count()).div_ceil(CHARS_PER_TOKEN)
}

/// Token budget for an advisor, or `None` when the window is unknown or the
/// reserve swallows it whole (in which case the view is sent unchanged).
fn context_budget(window: usize, max_tokens: Option<u32>) -> Option<usize> {
    if window == 0 {
        return None;
    }
    let reserve = max_tokens.map_or(DEFAULT_OUTPUT_RESERVE, |t| t as usize);
    let usable = (window as f64 * (1.0 - CONTEXT_SAFETY_FRACTION)).floor() as usize;
    usable.checked_sub(reserve).filter(|b| *b > 0)
}

/// Trim the advisory view to fit `budget` tokens.
///
/// Drops the OLDEST frames first while preserving the invariants the advisory
/// view guarantees: the result still starts on a user frame (an assistant
/// frame exposed by a drop is dropped too), and the final two frames — the
/// trailing synthetic marker plus at least one preceding turn — always
/// survive, even if that leaves the view over budget.
fn trim_to_budget(view: &[Message], budget: usize) -> Vec<Message> {
    let mut frames: Vec<Message> = view.to_vec();
    while frames.len() > 2 && estimate_tokens(&frames) > budget {
        frames.remove(0);
        // Preserve user-first ordering: never lead with an assistant turn.
        while frames.len() > 2 && frames[0].role == librefang_types::message::Role::Assistant {
            frames.remove(0);
        }
    }
    frames
}

/// Resolve an advisor's context window, memoized per `(provider, model)`.
///
/// Failures are cached as `None` ("unknown") so a missing catalog entry is not
/// re-probed once per advisor per loop iteration.
fn resolve_window(
    cache: &Mutex<HashMap<(String, String), Option<usize>>>,
    catalog: Option<&ModelCatalog>,
    slot: &MoaSlot,
) -> Option<usize> {
    let key = (slot.provider.clone(), slot.model.clone());
    if let Some(hit) = cache.lock().get(&key) {
        return *hit;
    }
    let resolved = catalog
        .and_then(|c| c.find_model(&slot.model))
        .map(|m| m.context_window as usize)
        .filter(|w| *w > 0);
    cache.lock().insert(key, resolved);
    resolved
}

/// Run all advisors concurrently against the advisory view.
///
/// Each advisor gets the same flattened `advisory_view` with the advisory
/// system prompt. Failures (driver init, timeout, empty output) are captured
/// as `failed` results rather than aborting the fan-out.
///
/// When `progress` is `Some`, each advisor completion emits a
/// [`MoaProgressEvent::Progress`] (running `done`/`total` + the advisor's
/// label) as it lands, so frontends see live fan-out progress instead of a
/// single batch at the end. Send failures (no live receiver) are ignored.
pub async fn run_fanout(
    advisors: &[(MoaSlot, Arc<dyn LlmDriver>)],
    advisory_view: &[Message],
    temperature: f32,
    timeout_secs: u64,
    max_tokens: Option<u32>,
    catalog: Option<&ModelCatalog>,
    progress: Option<&broadcast::Sender<MoaProgressEvent>>,
) -> Vec<AdvisorResult> {
    let semaphore = Arc::new(Semaphore::new(MAX_CONCURRENT_ADVISORS));
    let full_view = Arc::new(advisory_view.to_vec());
    let total = advisors.len();
    // Context windows are resolved once per `(provider, model)` for the whole
    // fan-out; unknown results are memoized too so a missing catalog entry is
    // not re-probed per advisor per iteration.
    let windows: Mutex<HashMap<(String, String), Option<usize>>> = Mutex::new(HashMap::new());

    let full_view_tokens = estimate_tokens(&full_view);

    let mut handles = Vec::with_capacity(advisors.len());

    for (slot, driver) in advisors {
        let permit = Arc::clone(&semaphore);
        // Each advisor gets a view trimmed to its OWN context window; slots
        // with an unknown window or a non-positive budget send it unchanged.
        let messages = match resolve_window(&windows, catalog, slot)
            .and_then(|w| context_budget(w, max_tokens))
        {
            Some(budget) if full_view_tokens > budget => {
                Arc::new(trim_to_budget(&full_view, budget))
            }
            _ => Arc::clone(&full_view),
        };
        let input_messages = messages.len();
        let slot = slot.clone();
        let driver = Arc::clone(driver);

        handles.push(tokio::spawn(async move {
            // Hold a permit for the duration of the call to bound concurrency.
            let _permit = permit.acquire_owned().await;
            call_advisor(
                &slot,
                driver.as_ref(),
                &messages,
                temperature,
                timeout_secs,
                max_tokens,
                input_messages,
            )
            .await
        }));
    }

    let mut results = Vec::with_capacity(handles.len());
    let mut done = 0usize;
    for handle in handles {
        match handle.await {
            Ok(mut result) => {
                result.cost = estimate_cost(catalog, &result.model, &result.usage);
                done += 1;
                if let Some(tx) = progress {
                    let _ = tx.send(MoaProgressEvent::Progress {
                        done,
                        total,
                        label: result.label.clone(),
                    });
                }
                results.push(result);
            }
            Err(e) => {
                warn!(error = %e, "MoA advisor task panicked");
                done += 1;
                if let Some(tx) = progress {
                    let _ = tx.send(MoaProgressEvent::Progress {
                        done,
                        total,
                        label: "[panicked]".to_string(),
                    });
                }
            }
        }
    }
    results
}

/// Call a single advisor with a timeout, returning an [`AdvisorResult`].
async fn call_advisor(
    slot: &MoaSlot,
    driver: &dyn LlmDriver,
    messages: &[Message],
    temperature: f32,
    timeout_secs: u64,
    max_tokens: Option<u32>,
    input_messages: usize,
) -> AdvisorResult {
    let label = slot_label(slot);
    let request = CompletionRequest {
        model: slot.model.clone(),
        messages: Arc::new(messages.to_vec()),
        tools: Arc::new(Vec::new()),
        max_tokens: max_tokens.unwrap_or(4096),
        temperature,
        system: Some(ADVISORY_SYSTEM_PROMPT.to_string()),
        ..Default::default()
    };

    let outcome = tokio::time::timeout(
        Duration::from_secs(timeout_secs.max(1)),
        driver.complete(request),
    )
    .await;

    match outcome {
        Ok(Ok(response)) => {
            let text = response_text(&response);
            let usage = response.usage;
            let failed = text.trim().is_empty();
            AdvisorResult {
                label,
                provider: slot.provider.clone(),
                model: slot.model.clone(),
                temperature,
                input_messages,
                text: if failed {
                    "[failed: empty response]".into()
                } else {
                    text
                },
                usage,
                cost: 0.0,
                failed,
            }
        }
        Ok(Err(e)) => AdvisorResult {
            label,
            provider: slot.provider.clone(),
            model: slot.model.clone(),
            temperature,
            input_messages,
            text: format!("[failed: {e}]"),
            usage: TokenUsage::default(),
            cost: 0.0,
            failed: true,
        },
        Err(_) => AdvisorResult {
            label,
            provider: slot.provider.clone(),
            model: slot.model.clone(),
            temperature,
            input_messages,
            text: format!("[failed: timeout after {timeout_secs}s]"),
            usage: TokenUsage::default(),
            cost: 0.0,
            failed: true,
        },
    }
}

/// Extract the concatenated text from a completion response.
fn response_text(response: &CompletionResponse) -> String {
    use librefang_types::message::ContentBlock;
    let mut parts = Vec::new();
    for block in &response.content {
        if let ContentBlock::Text { text, .. } = block {
            parts.push(text.clone());
        }
    }
    parts.join("\n")
}

/// Resolve the effective advisor temperature from a preset override.
pub fn effective_reference_temperature(override_temp: Option<f32>) -> f32 {
    override_temp.unwrap_or(DEFAULT_REFERENCE_TEMPERATURE)
}

/// Resolve the effective advisor timeout from a preset override.
pub fn effective_reference_timeout(override_secs: Option<u64>) -> u64 {
    override_secs.unwrap_or(DEFAULT_REFERENCE_TIMEOUT_SECS)
}

#[cfg(test)]
mod tests {
    use super::*;
    use librefang_types::message::Role;

    /// A user frame whose text is `n` characters long.
    fn user(n: usize) -> Message {
        Message::user("u".repeat(n))
    }

    fn assistant(n: usize) -> Message {
        Message::assistant("a".repeat(n))
    }

    #[test]
    fn budget_subtracts_reserve_and_safety_margin() {
        // 100_000 window, 10% safety -> 90_000 usable, minus an 8_000 reserve.
        assert_eq!(context_budget(100_000, Some(8_000)), Some(82_000));
    }

    #[test]
    fn budget_uses_default_reserve_when_unset() {
        assert_eq!(
            context_budget(100_000, None),
            Some(90_000 - DEFAULT_OUTPUT_RESERVE)
        );
    }

    #[test]
    fn budget_none_when_reserve_swallows_window() {
        // Reserve exceeds the usable window: no budget, so the caller sends
        // the view untrimmed rather than shipping an empty transcript.
        assert_eq!(context_budget(4_000, Some(8_000)), None);
        assert_eq!(context_budget(0, Some(10)), None);
    }

    #[test]
    fn trim_drops_oldest_first() {
        let view = vec![user(4_000), assistant(4_000), user(4_000), user(40)];
        let trimmed = trim_to_budget(&view, 1_100);
        // Newest frames survive; the trailing marker is always last.
        assert!(trimmed.len() < view.len());
        assert_eq!(
            trimmed.last().unwrap().content.text_content(),
            view.last().unwrap().content.text_content()
        );
    }

    #[test]
    fn trim_never_leads_with_assistant_frame() {
        // Dropping frame 0 would expose an assistant turn; the advisory view
        // must still start on a user frame after trimming.
        let view = vec![user(8_000), assistant(8_000), user(8_000), user(40)];
        let trimmed = trim_to_budget(&view, 500);
        assert_eq!(trimmed[0].role, Role::User);
    }

    #[test]
    fn trim_keeps_final_two_frames_even_over_budget() {
        // A budget no suffix can satisfy must not empty the view: the last
        // turn plus the trailing marker always survive.
        let view = vec![user(8_000), user(8_000), user(8_000)];
        let trimmed = trim_to_budget(&view, 1);
        assert_eq!(trimmed.len(), 2);
        assert!(estimate_tokens(&trimmed) > 1);
    }

    #[test]
    fn trim_is_noop_when_already_within_budget() {
        let view = vec![user(40), user(40)];
        let trimmed = trim_to_budget(&view, 1_000_000);
        assert_eq!(trimmed.len(), view.len());
    }

    #[test]
    fn estimate_counts_system_prompt_overhead() {
        // The advisory system prompt rides along on every call, so the
        // estimate must include it or the budget is systematically optimistic.
        let empty = estimate_tokens(&[]);
        assert!(empty >= ADVISORY_SYSTEM_PROMPT.chars().count() / CHARS_PER_TOKEN);
    }
}
