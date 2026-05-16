//! LLM call retry, cooldown circuit-breaker, and streaming-with-retry
//! support for the main agent loop.
//!
//! Splits the per-call retry policy (`call_with_retry`) and the per-stream
//! retry / stop-fan-in (`stream_with_retry`) out of `agent_loop/mod.rs`. The
//! process-global `LLM_CONCURRENCY` semaphore and the retry constants
//! (`MAX_RETRIES`, `BASE_RETRY_DELAY_MS`, `DEFAULT_DEFER_MS`,
//! `MAX_CONCURRENT_LLM_CALLS`) live with the retry path that consumes them.

use crate::auth_cooldown::{CooldownVerdict, ProviderCooldown};
use crate::llm_driver::{CompletionRequest, LlmDriver, LlmError, StreamEvent};
use crate::llm_errors;
use librefang_types::error::{LibreFangError, LibreFangResult};
use librefang_types::message::{StopReason, TokenUsage};
use std::time::Duration;
use tokio::sync::mpsc;
use tracing::{debug, instrument, warn};

use super::TIMEOUT_PARTIAL_OUTPUT_MARKER;

/// Maximum retries for rate-limited or overloaded API calls.
pub(super) const MAX_RETRIES: u32 = 3;

/// Base delay for exponential backoff (milliseconds).
pub(super) const BASE_RETRY_DELAY_MS: u64 = 1000;

/// Maximum number of concurrent LLM calls across all agents.
///
/// Each in-flight LLM call holds the full request + response body in RAM.
/// On a 256 MB deployment with many hand-agents firing simultaneously this
/// is the dominant memory spike.  Callers queue (`.await`) rather than
/// fail when the limit is reached; the existing per-call timeout still fires.
const MAX_CONCURRENT_LLM_CALLS: usize = 5;

/// Process-global semaphore that caps simultaneous LLM HTTP calls.
static LLM_CONCURRENCY: std::sync::LazyLock<tokio::sync::Semaphore> =
    std::sync::LazyLock::new(|| tokio::sync::Semaphore::new(MAX_CONCURRENT_LLM_CALLS));

/// Call an LLM driver with automatic retry on rate-limit and overload errors.
///
/// Uses the `llm_errors` classifier for smart error handling and the
/// `ProviderCooldown` circuit breaker to prevent request storms.
fn check_retry_cooldown(
    provider: Option<&str>,
    cooldown: Option<&ProviderCooldown>,
    allow_probe_log_message: &str,
) -> LibreFangResult<()> {
    if let (Some(provider), Some(cooldown)) = (provider, cooldown) {
        match cooldown.check(provider) {
            CooldownVerdict::Reject {
                reason,
                retry_after_secs,
            } => {
                return Err(LibreFangError::llm_driver_msg(format!(
                    "Provider '{provider}' is in cooldown ({reason}). Retry in {retry_after_secs}s."
                )));
            }
            CooldownVerdict::AllowProbe => {
                debug!(provider, "{allow_probe_log_message}");
            }
            CooldownVerdict::Allow => {}
        }
    }

    Ok(())
}

fn record_retry_success(provider: Option<&str>, cooldown: Option<&ProviderCooldown>) {
    if let (Some(provider), Some(cooldown)) = (provider, cooldown) {
        cooldown.record_success(provider);
    }
}

fn record_retry_failure(
    provider: Option<&str>,
    cooldown: Option<&ProviderCooldown>,
    is_billing: bool,
) {
    if let (Some(provider), Some(cooldown)) = (provider, cooldown) {
        cooldown.record_failure(provider, is_billing);
    }
}

/// Conservative defer window when a provider exhausts in-loop retries
/// without giving us a structured `retry_after_ms` hint. 5 minutes is short
/// enough that quota-clearing windows (claude.ai 5h hard caps, OpenAI 1m
/// per-token windows) usually open well before this re-fires, and long
/// enough that a tight ticker doesn't burn a fresh quota the moment it
/// resets.
const DEFAULT_DEFER_MS: u64 = 5 * 60 * 1000;

async fn handle_retryable_llm_error(
    attempt: u32,
    retry_after_ms: u64,
    exhausted_message: String,
    retry_log_message: &str,
    last_error_label: &'static str,
    provider: Option<&str>,
    cooldown: Option<&ProviderCooldown>,
) -> Result<String, LibreFangError> {
    if attempt == MAX_RETRIES {
        record_retry_failure(provider, cooldown, false);
        // Append the defer marker so the channel bridge can route this
        // entry to `JournalStatus::Deferred` (re-dispatched on a ticker
        // once the quota window resets) instead of `Failed` (one-shot).
        // Floor the hint at DEFAULT_DEFER_MS — providers that returned no
        // structured retry-after still need a usable delay.
        let defer_ms = retry_after_ms.max(DEFAULT_DEFER_MS);
        return Err(LibreFangError::llm_driver_msg(format!(
            "{exhausted_message} {marker}={defer_ms}",
            marker = librefang_channels::message_journal::RATE_LIMIT_DEFER_MARKER,
        )));
    }

    let delay = std::cmp::max(retry_after_ms, BASE_RETRY_DELAY_MS * 2u64.pow(attempt));
    warn!(attempt, delay_ms = delay, "{retry_log_message}");
    tokio::time::sleep(Duration::from_millis(delay)).await;
    Ok(last_error_label.to_string())
}

fn build_user_facing_llm_error(
    error: &LlmError,
    classification_log_message: &str,
) -> (bool, LibreFangError) {
    let raw_error = error.to_string();
    let status = match error {
        LlmError::Api { status, .. } => Some(*status),
        _ => None,
    };
    let classified = llm_errors::classify_error(&raw_error, status);
    warn!(
        category = ?classified.category,
        retryable = classified.is_retryable,
        raw = %raw_error,
        "{classification_log_message}: {}",
        classified.sanitized_message
    );

    let user_msg = if classified.category == llm_errors::LlmErrorCategory::Format {
        format!("{} — raw: {}", classified.sanitized_message, raw_error)
    } else {
        classified.sanitized_message
    };

    (
        classified.is_billing,
        LibreFangError::llm_driver_msg(user_msg),
    )
}

#[instrument(
    skip_all,
    fields(
        llm.provider = provider.unwrap_or("unknown"),
        llm.model = %request.model,
        llm.messages = request.messages.len(),
        llm.tools = request.tools.len(),
    ),
)]
pub(super) async fn call_with_retry(
    driver: &dyn LlmDriver,
    request: CompletionRequest,
    provider: Option<&str>,
    cooldown: Option<&ProviderCooldown>,
) -> LibreFangResult<crate::llm_driver::CompletionResponse> {
    check_retry_cooldown(
        provider,
        cooldown,
        "Allowing probe request through circuit breaker",
    )?;

    let mut last_error = None;

    for attempt in 0..=MAX_RETRIES {
        // Acquire the permit inside the retry loop so it is held only during
        // the actual HTTP round-trip and released before any backoff sleep.
        // Holding it across retries would block a slot for the full backoff
        // duration (up to minutes on rate-limit), starving other agents.
        let _permit = LLM_CONCURRENCY
            .acquire()
            .await
            .expect("LLM_CONCURRENCY semaphore closed");
        match driver.complete(request.clone()).await {
            Ok(response) => {
                record_retry_success(provider, cooldown);
                return Ok(response);
            }
            Err(LlmError::RateLimited { retry_after_ms, .. }) => {
                last_error = Some(
                    handle_retryable_llm_error(
                        attempt,
                        retry_after_ms,
                        format!("Rate limited after {} retries", MAX_RETRIES),
                        "Rate limited, retrying after delay",
                        "Rate limited",
                        provider,
                        cooldown,
                    )
                    .await?,
                );
            }
            Err(LlmError::Overloaded { retry_after_ms }) => {
                last_error = Some(
                    handle_retryable_llm_error(
                        attempt,
                        retry_after_ms,
                        format!("Model overloaded after {} retries", MAX_RETRIES),
                        "Model overloaded, retrying after delay",
                        "Overloaded",
                        provider,
                        cooldown,
                    )
                    .await?,
                );
            }
            Err(e) => {
                let (is_billing, err) = build_user_facing_llm_error(&e, "LLM error classified");
                record_retry_failure(provider, cooldown, is_billing);
                return Err(err);
            }
        }
    }

    Err(LibreFangError::llm_driver_msg(
        last_error.unwrap_or_else(|| "Unknown error".to_string()),
    ))
}

/// Call an LLM driver in streaming mode with automatic retry on rate-limit and overload errors.
///
/// Uses the `llm_errors` classifier and `ProviderCooldown` circuit breaker.
/// Result of a `stream_with_retry` call.
pub(super) struct StreamWithRetryResult {
    pub(super) response: crate::llm_driver::CompletionResponse,
    /// True when the incremental cascade-leak guard fired mid-stream:
    /// TextDelta forwarding was stopped early and the partial response
    /// must be treated as a silent drop (system-prompt regurgitation).
    pub(super) cascade_leak_aborted: bool,
}

/// Stream an LLM completion with retry logic and an incremental cascade-leak
/// guard.
///
/// A proxy channel sits between the driver's raw `TextDelta` events and the
/// outer `tx`. Each delta is appended to an in-memory accumulator and
/// `is_cascade_leak` is run against it. On the first hit:
/// - Forwarding of further `TextDelta` events stops immediately (no more
///   tokens reach the wire).
/// - All other event types (`ToolUseStart`, `ContentComplete`, …) are still
///   forwarded so the driver loop terminates cleanly.
/// - `cascade_leak_aborted = true` is returned to the caller.
pub(super) async fn stream_with_retry(
    driver: &dyn LlmDriver,
    request: CompletionRequest,
    tx: mpsc::Sender<StreamEvent>,
    provider: Option<&str>,
    cooldown: Option<&ProviderCooldown>,
) -> LibreFangResult<StreamWithRetryResult> {
    check_retry_cooldown(
        provider,
        cooldown,
        "Allowing probe request through circuit breaker (stream)",
    )?;

    let mut last_error = None;
    // Sticky flag: once a cascade-leak fires in any attempt, all subsequent
    // retry attempts must be short-circuited to a silent drop. Without this,
    // a RateLimited or Overloaded retry would start a fresh accumulator and
    // give the leaking model a "do over" — defeating the guard entirely.
    let mut leak_fired_sticky = false;

    for attempt in 0..=MAX_RETRIES {
        // If a previous attempt already triggered the leak guard, do not
        // invoke the driver again — return immediately with cascade_leak_aborted.
        if leak_fired_sticky {
            use crate::llm_driver::CompletionResponse;
            return Ok(StreamWithRetryResult {
                response: CompletionResponse {
                    content: vec![],
                    stop_reason: StopReason::EndTurn,
                    tool_calls: vec![],
                    usage: TokenUsage::default(),
                },
                cascade_leak_aborted: true,
            });
        }

        // Same rationale as call_with_retry: acquire inside the loop so
        // the permit is not held during backoff sleeps between retries.
        let _permit = LLM_CONCURRENCY
            .acquire()
            .await
            .expect("LLM_CONCURRENCY semaphore closed");

        // Proxy channel: driver writes to `proxy_tx`; we forward events to
        // `tx` with incremental cascade-leak scanning on TextDelta.
        let (proxy_tx, mut proxy_rx) = mpsc::channel::<StreamEvent>(64);
        let outer_tx = tx.clone();

        // Spawn a forwarding task that accumulates text and checks for leaks.
        // Cap the accumulator at 128 KB so a pathologically long stream cannot
        // grow the leak-detection buffer unboundedly (the rolling suffix kept
        // after the cap still covers any marker that could span a delta boundary).
        const ACCUMULATED_CAP: usize = 128 * 1024;
        let forward_task = tokio::spawn(async move {
            let mut accumulated = String::new();
            let mut leak_fired = false;
            while let Some(event) = proxy_rx.recv().await {
                match &event {
                    StreamEvent::TextDelta { text } if !leak_fired => {
                        // Rolling-window: once we exceed the cap, discard the
                        // oldest bytes and keep only the tail that is large
                        // enough to overlap any multi-token marker.  The longest
                        // marker in STRUCTURAL_TURN_FRAMES / ENVELOPE_* is
                        // ~30 chars; 512 bytes of overlap is a comfortable
                        // margin.
                        if accumulated.len() + text.len() > ACCUMULATED_CAP {
                            const OVERLAP: usize = 512;
                            let keep_from = accumulated.len().saturating_sub(OVERLAP);
                            // Walk to a valid UTF-8 boundary.
                            let keep_from = (keep_from..=accumulated.len())
                                .find(|&i| accumulated.is_char_boundary(i))
                                .unwrap_or(accumulated.len());
                            accumulated.drain(..keep_from);
                        }
                        accumulated.push_str(text);
                        if crate::silent_response::is_cascade_leak(&accumulated) {
                            leak_fired = true;
                            // Stop forwarding TextDelta — do not send this
                            // token to the wire. Other event types continue.
                            continue;
                        }
                        // Forward the delta; ignore send errors (client gone).
                        let _ = outer_tx
                            .send(StreamEvent::TextDelta { text: text.clone() })
                            .await;
                    }
                    StreamEvent::TextDelta { .. } => {
                        // leak_fired: swallow remaining text tokens silently.
                    }
                    other => {
                        let _ = outer_tx.send(other.clone()).await;
                    }
                }
            }
            leak_fired
        });

        // Drive the LLM stream, then join the forwarding task exactly once.
        // The join handle is consumed here; each match arm either returns or
        // continues, so there is exactly one await site per control-flow path.
        let driver_result = driver.stream(request.clone(), proxy_tx).await;
        // proxy_tx is dropped when driver returns (moved into driver.stream).
        // forward_task drains the proxy channel and finishes.
        let cascade_leak_aborted = forward_task.await.unwrap_or(false);
        // Propagate to the sticky flag so any retry iteration short-circuits.
        if cascade_leak_aborted {
            leak_fired_sticky = true;
        }

        match driver_result {
            Ok(response) => {
                record_retry_success(provider, cooldown);
                return Ok(StreamWithRetryResult {
                    response,
                    cascade_leak_aborted,
                });
            }
            Err(LlmError::RateLimited { retry_after_ms, .. }) => {
                last_error = Some(
                    handle_retryable_llm_error(
                        attempt,
                        retry_after_ms,
                        format!("Rate limited after {} retries", MAX_RETRIES),
                        "Rate limited (stream), retrying after delay",
                        "Rate limited",
                        provider,
                        cooldown,
                    )
                    .await?,
                );
            }
            Err(LlmError::Overloaded { retry_after_ms }) => {
                last_error = Some(
                    handle_retryable_llm_error(
                        attempt,
                        retry_after_ms,
                        format!("Model overloaded after {} retries", MAX_RETRIES),
                        "Model overloaded (stream), retrying after delay",
                        "Overloaded",
                        provider,
                        cooldown,
                    )
                    .await?,
                );
            }
            Err(LlmError::TimedOut {
                inactivity_secs,
                partial_text,
                partial_text_len,
                last_activity,
            }) => {
                warn!(
                    inactivity_secs,
                    partial_text_len, last_activity, "LLM stream timed out with partial output"
                );
                // #3552: `partial_text` is `Option<Arc<str>>` — copy the body
                // into the owned `String` that `TextDelta` requires only when
                // we actually have one to forward. Most consumers (failover
                // classification, log lines, error stringification through
                // `LibreFangError::LlmDriver(e.to_string())`) only ever read
                // `partial_text_len` and pay nothing for the body.
                if let Some(body) = partial_text.as_deref() {
                    if !body.is_empty() {
                        let _ = tx
                            .send(StreamEvent::TextDelta {
                                text: body.to_string(),
                            })
                            .await;
                    }
                }
                return Err(LibreFangError::llm_driver_msg(format!(
                    "Task timed out after {inactivity_secs}s of inactivity \
                     (last: {last_activity}). \
                     {partial_text_len} chars of partial output were delivered. \
                     {TIMEOUT_PARTIAL_OUTPUT_MARKER}"
                )));
            }
            Err(e) => {
                let err_str = e.to_string();
                if llm_errors::is_transient(&err_str) && attempt < MAX_RETRIES {
                    warn!(
                        attempt,
                        error = %err_str,
                        "LLM stream died with transient error, retrying"
                    );
                    last_error = Some("Transient stream error".to_string());
                    tokio::time::sleep(Duration::from_millis(
                        BASE_RETRY_DELAY_MS * 2u64.pow(attempt),
                    ))
                    .await;
                    continue;
                }
                let (is_billing, err) =
                    build_user_facing_llm_error(&e, "LLM stream error classified");
                record_retry_failure(provider, cooldown, is_billing);
                return Err(err);
            }
        }
    }

    Err(LibreFangError::llm_driver_msg(
        last_error.unwrap_or_else(|| "Unknown error".to_string()),
    ))
}
