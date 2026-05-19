//! Retry helper for transient upstream failures.
//!
//! For a long-horizon RL rollout the worst failure mode is "30 minutes
//! of compute, one TCP RST, trajectory dropped on the floor". The
//! W&B / Tinker / Atropos exporters all retry transient classes
//! (network drops, 5xx, 429) with exponential backoff capped at three
//! attempts — long enough to cover a routine cloud-side hiccup, short
//! enough that a genuinely broken upstream surfaces quickly.
//!
//! Permanent errors (auth, 4xx other than 429, `InvalidConfig`) are
//! returned to the caller on the first attempt — retrying them is
//! pointless and would mask the misconfiguration.
//!
//! ## Why a local helper rather than `librefang-runtime`'s retry loop
//!
//! `crates/librefang-runtime/src/agent_loop.rs` has a sophisticated
//! retry path (`call_with_retry`) but it is welded to `LlmError` and
//! the provider-cooldown circuit breaker — neither concept exists at
//! this layer. Pulling that helper in would also drag `librefang-
//! runtime`'s entire dependency tree (tower middleware, channel
//! adapters, tokenizer pipelines) into a leaf egress crate. The local
//! shape below is ~25 lines and unit-testable in isolation.
//!
//! ## Deliberate divergence from `agent_loop.rs:108-111`
//!
//! The *transient classification* matches the workspace standard
//! (network drops + 5xx + 429); the *timing* does not. `agent_loop`
//! uses `MAX_RETRIES = 3` over `0..=MAX_RETRIES` (4 attempts) with
//! `BASE_RETRY_DELAY_MS = 1000`. This module uses 3 attempts at 200ms
//! base — chosen for the exporter latency profile:
//!
//! - W&B / Tinker / Atropos rate-limit windows are seconds, not
//!   minutes, so a 200ms-then-400ms backoff covers the common
//!   transient blip without the 1s wake-up tax.
//! - Sub-second retry keeps the operator-visible failure latency low
//!   when an upstream is genuinely down (3 attempts at 200ms + 400ms
//!   ≈ 600ms wall + per-call timeouts, vs ~3s for the agent-loop
//!   shape).
//! - Trajectories are post-rollout; the caller is not blocking a user
//!   on the retry budget, so the workspace's longer LLM-call retry
//!   shape would burn time for no recovery benefit.
//!
//! Do NOT "fix" this back to 1000ms/4 attempts to "match the
//! workspace" — the divergence is intentional. If you are tuning
//! retries, update the constants below AND this docstring together.

use std::future::Future;
use std::time::Duration;

use crate::error::ExportError;

/// Maximum number of attempts (including the first).
///
/// Diverges from `librefang_runtime::agent_loop::MAX_RETRIES` (3 retries
/// over 4 attempts) — see module-level "Deliberate divergence" note.
const MAX_ATTEMPTS: u32 = 3;

/// Base delay for the exponential backoff. Attempt N waits
/// `BASE_DELAY_MS * 2^(N-1)` ms before retrying (i.e. 200ms, 400ms).
///
/// Diverges from `librefang_runtime::agent_loop::BASE_RETRY_DELAY_MS`
/// (1000ms) — see module-level "Deliberate divergence" note for the
/// exporter-latency rationale.
const BASE_DELAY_MS: u64 = 200;

/// Run `op` up to [`MAX_ATTEMPTS`] times, retrying transient
/// [`ExportError`]s with exponential backoff. Returns the first
/// non-transient error verbatim, or the final transient error after
/// exhausting attempts.
///
/// `label` identifies the call site in retry log lines (e.g.
/// `"wandb.create_run"`) so an operator can correlate retries against
/// the specific upstream call that flaked.
///
/// `op` is invoked fresh on every attempt — the caller is responsible
/// for any per-attempt cloning (the upload body, headers, …). This
/// shape matches how `reqwest::RequestBuilder` consumes its body and
/// avoids leaking the consumed state across retries.
///
/// Timing differs from the workspace's LLM retry loop
/// (`librefang_runtime::agent_loop::call_with_retry`, 1000ms / 4
/// attempts) by design — 200ms / 3 attempts is chosen for the
/// exporter latency profile (sub-second retry preferred for fast
/// feedback on transient W&B / Tinker / Atropos failures). See the
/// module-level "Deliberate divergence" note before tuning.
pub(crate) async fn retry_upload<F, Fut, T>(
    label: &'static str,
    mut op: F,
) -> Result<T, ExportError>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, ExportError>>,
{
    let mut last_err: Option<ExportError> = None;
    for attempt in 1..=MAX_ATTEMPTS {
        match op().await {
            Ok(v) => return Ok(v),
            Err(err) => {
                if !is_transient(&err) || attempt == MAX_ATTEMPTS {
                    tracing::debug!(
                        target = "librefang_rl_export::retry",
                        call = label,
                        attempt,
                        ?err,
                        "giving up — non-transient or attempts exhausted",
                    );
                    return Err(err);
                }
                let delay_ms = BASE_DELAY_MS * 2u64.pow(attempt - 1);
                tracing::warn!(
                    target = "librefang_rl_export::retry",
                    call = label,
                    attempt,
                    delay_ms,
                    ?err,
                    "transient upstream error — retrying",
                );
                last_err = Some(err);
                tokio::time::sleep(Duration::from_millis(delay_ms)).await;
            }
        }
    }
    // Unreachable under MAX_ATTEMPTS >= 1 — the loop body always
    // returns or breaks on the final attempt. Kept as a defensive
    // expression so a future bump in MAX_ATTEMPTS to 0 doesn't panic
    // with a use-of-moved-value compile error.
    Err(last_err.unwrap_or_else(|| {
        ExportError::NetworkError("retry helper exited without an attempt".to_string())
    }))
}

/// Is `err` worth a retry? Mirrors the standard "transport drop / 5xx
/// / 429" set used elsewhere in the workspace (see
/// `librefang_llm_driver::llm_errors::is_transient`). Auth failures,
/// other 4xx, decode failures, invalid config, and the
/// `TrainerNotReady` sentinel are all permanent.
pub(crate) fn is_transient(err: &ExportError) -> bool {
    match err {
        ExportError::NetworkError(_) => true,
        ExportError::UpstreamRejected { status, .. } => {
            *status == 429 || (500..600).contains(status)
        }
        ExportError::AuthError
        | ExportError::MalformedResponse(_)
        | ExportError::InvalidConfig(_)
        | ExportError::TrainerNotReady { .. } => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;

    #[tokio::test]
    async fn returns_immediately_on_success() {
        let counter = Arc::new(AtomicU32::new(0));
        let c = counter.clone();
        let out: Result<u32, ExportError> = retry_upload("test.ok", || {
            let c = c.clone();
            async move {
                c.fetch_add(1, Ordering::SeqCst);
                Ok(42)
            }
        })
        .await;
        assert_eq!(out.expect("ok"), 42);
        assert_eq!(
            counter.load(Ordering::SeqCst),
            1,
            "must not retry on success"
        );
    }

    #[tokio::test]
    async fn returns_immediately_on_permanent_error() {
        // AuthError must NOT be retried — refreshing credentials needs
        // operator action and retrying 3x just amplifies the 401.
        let counter = Arc::new(AtomicU32::new(0));
        let c = counter.clone();
        let out: Result<(), ExportError> = retry_upload("test.permanent", || {
            let c = c.clone();
            async move {
                c.fetch_add(1, Ordering::SeqCst);
                Err(ExportError::AuthError)
            }
        })
        .await;
        assert!(matches!(out, Err(ExportError::AuthError)));
        assert_eq!(
            counter.load(Ordering::SeqCst),
            1,
            "auth errors must not be retried"
        );
    }

    #[tokio::test]
    async fn retries_5xx_then_succeeds() {
        // First two attempts return 503; third succeeds. Pins the
        // recovery-after-blip behaviour.
        let counter = Arc::new(AtomicU32::new(0));
        let c = counter.clone();
        let out: Result<u32, ExportError> = retry_upload("test.5xx", || {
            let c = c.clone();
            async move {
                let n = c.fetch_add(1, Ordering::SeqCst);
                if n < 2 {
                    Err(ExportError::UpstreamRejected {
                        status: 503,
                        body: "overloaded".to_string(),
                    })
                } else {
                    Ok(7)
                }
            }
        })
        .await;
        assert_eq!(out.expect("ok"), 7);
        assert_eq!(counter.load(Ordering::SeqCst), 3, "expected 3 attempts");
    }

    #[tokio::test]
    async fn retries_429_then_succeeds() {
        // Rate-limit responses must retry as well — both W&B and
        // Tinker rate-limit.
        let counter = Arc::new(AtomicU32::new(0));
        let c = counter.clone();
        let out: Result<u32, ExportError> = retry_upload("test.429", || {
            let c = c.clone();
            async move {
                let n = c.fetch_add(1, Ordering::SeqCst);
                if n == 0 {
                    Err(ExportError::UpstreamRejected {
                        status: 429,
                        body: "rate limited".to_string(),
                    })
                } else {
                    Ok(11)
                }
            }
        })
        .await;
        assert_eq!(out.expect("ok"), 11);
        assert_eq!(counter.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn does_not_retry_non_429_4xx() {
        // A 404 ("project not found") is operator-fixable — retry would
        // just amplify the misconfiguration.
        let counter = Arc::new(AtomicU32::new(0));
        let c = counter.clone();
        let out: Result<(), ExportError> = retry_upload("test.4xx", || {
            let c = c.clone();
            async move {
                c.fetch_add(1, Ordering::SeqCst);
                Err(ExportError::UpstreamRejected {
                    status: 404,
                    body: "missing".to_string(),
                })
            }
        })
        .await;
        assert!(matches!(
            out,
            Err(ExportError::UpstreamRejected { status: 404, .. })
        ));
        assert_eq!(counter.load(Ordering::SeqCst), 1, "4xx must not retry");
    }

    #[tokio::test]
    async fn surfaces_final_error_after_exhausting_attempts() {
        let counter = Arc::new(AtomicU32::new(0));
        let c = counter.clone();
        let out: Result<(), ExportError> = retry_upload("test.exhaust", || {
            let c = c.clone();
            async move {
                c.fetch_add(1, Ordering::SeqCst);
                Err(ExportError::NetworkError("connection reset".to_string()))
            }
        })
        .await;
        assert!(matches!(out, Err(ExportError::NetworkError(_))));
        assert_eq!(
            counter.load(Ordering::SeqCst),
            MAX_ATTEMPTS,
            "should attempt MAX_ATTEMPTS times then give up"
        );
    }
}
