//! Mixture-of-Agents (MoA) support.
//!
//! Implements the MoA execution model: a set of blind *advisor* models reason
//! over a flattened, text-only view of the conversation in parallel, and a
//! single *aggregator* model receives their private advice plus the full
//! conversation (and tools) to take the actual action.
//!
//! The public entry point is [`driver::MoaDriver`], which implements
//! [`librefang_llm_driver::LlmDriver`] and is substituted by the kernel when a
//! manifest's provider is `"moa"`.

pub mod advisory_view;
pub mod driver;
pub mod fanout;
pub mod guidance;
pub mod privacy;
pub mod progress;
pub mod trace;

/// The single system message every advisor receives.
///
/// Advisors are deliberately blind: they cannot call tools, run commands, or
/// access files/URLs. This prompt forbids them from claiming to have executed
/// anything and frames their output as private guidance for the aggregator.
pub const ADVISORY_SYSTEM_PROMPT: &str = "You are a reference advisor in a Mixture of Agents process. \
You are NOT the acting agent and do NOT execute anything: you cannot call tools, run commands, browse, \
or access files, repositories, or URLs. A separate aggregator/orchestrator model holds those capabilities \
and will take the actual actions. You must NEVER claim or imply that you have executed a command, \
downloaded a file, or accessed a URL. Give your most intelligent analysis of the state: the goal, the \
problem, the best approach, concrete next steps and tool-use strategy, likely pitfalls, and anything the \
acting agent missed. Respond with your advice directly — no preamble, no disclaimers. Your response is \
private guidance handed to the aggregator, not shown to the user.";

/// Bridge a [`MoaDriver`](driver::MoaDriver)'s progress broadcast into the
/// agent loop's phase callback.
///
/// The kernel resolves the driver *before* the loop builds its
/// [`PhaseCallback`](crate::agent_loop::PhaseCallback), so the driver cannot
/// receive the callback at construction. Instead it emits
/// [`MoaProgressEvent`](progress::MoaProgressEvent)s into a kernel-owned
/// broadcast channel; this helper subscribes and spawns a turn-bound task that
/// maps each event onto the matching [`LoopPhase`](crate::agent_loop::LoopPhase)
/// variant and invokes `on_phase`.
///
/// Returns the task handle (abort it when the turn ends) or `None` when the
/// driver is not a `MoaDriver` or has no progress channel.
pub fn spawn_moa_progress_relay(
    driver: &std::sync::Arc<dyn librefang_llm_driver::LlmDriver>,
    on_phase: crate::agent_loop::PhaseCallback,
) -> Option<tokio::task::JoinHandle<()>> {
    use crate::agent_loop::LoopPhase;
    use crate::moa::progress::MoaProgressEvent;

    let moa = driver.as_any().downcast_ref::<driver::MoaDriver>()?;
    let mut rx = moa.progress_receiver()?;

    Some(tokio::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(MoaProgressEvent::FanoutStart { total }) => {
                    on_phase(LoopPhase::MoaProgress {
                        done: 0,
                        total,
                        label: String::new(),
                    });
                }
                Ok(MoaProgressEvent::Progress { done, total, label }) => {
                    on_phase(LoopPhase::MoaProgress { done, total, label });
                }
                Ok(MoaProgressEvent::Reference {
                    index,
                    count,
                    label,
                }) => {
                    // Advisor text is internal reasoning (PII-redacted before
                    // the aggregator) — never broadcast; surface the label only.
                    on_phase(LoopPhase::MoaReference {
                        index,
                        count,
                        label,
                    });
                }
                Ok(MoaProgressEvent::Aggregating {
                    aggregator,
                    ref_count,
                }) => {
                    on_phase(LoopPhase::MoaAggregating {
                        aggregator,
                        ref_count,
                    });
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    tracing::warn!(
                        lagged = n,
                        "MoA progress relay: consumer lagged, {n} event(s) dropped"
                    );
                    continue;
                }
            }
        }
    }))
}
