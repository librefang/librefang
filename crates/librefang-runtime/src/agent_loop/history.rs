//! Message-history trim cap: default, floor, and per-loop resolution.
//!
//! Splits the small history-config cluster (`DEFAULT_MAX_HISTORY_MESSAGES`,
//! `MIN_HISTORY_MESSAGES`, `resolve_max_history`, `clamp_max_history`) out
//! of `agent_loop/mod.rs`. None of these helpers touch the agent loop
//! state directly; they only read `AgentManifest.max_history_messages` /
//! `LoopOptions.max_history_messages` and clamp the result against the
//! safe-trim floor.

use librefang_types::agent::AgentManifest;
use tracing::warn;

use super::LoopOptions;

/// Default ceiling for message history before auto-trimming.
///
/// The earlier value of 40 assumed "tool calls consume 4–6 messages per
/// user turn → 7–10 conversation turns", which held for chat-style
/// agents but underestimated coordinator hands. In production logs we
/// observed `creator:creator-hand` (polling `video_status` every 15-20s)
/// trimming every turn at total_messages=41 with `hit_ratio=0.0` — the
/// trim was invalidating the prompt-cache prefix on every turn because
/// the cap was too tight for the actual workflow length. Survey of the
/// librefang-registry hands shows `max_iterations` of 50–80 is the
/// norm; at ~8 messages per real turn that fills the bucket in 5
/// turns, far less than the 7–10 the comment claimed.
///
/// 60 gives ~7–10 real turns for those heavier workflows and leaves
/// the prompt-cache prefix stable across normal back-and-forth.
/// Long-workflow hands (researcher / creator / devops / predictor)
/// still set explicit per-hand `max_history_messages` overrides in the
/// registry to push their headroom higher.
///
/// Override per-agent via `AgentManifest.max_history_messages` or
/// globally via `KernelConfig.max_history_messages`; resolved at loop
/// entry by `resolve_max_history`. Values below `MIN_HISTORY_MESSAGES`
/// are clamped up at runtime.
pub const DEFAULT_MAX_HISTORY_MESSAGES: usize = 60;

/// Floor for the message-history cap. Values below this are clamped up
/// with a warning log: `safe_trim_messages` re-validates the trimmed
/// window via `validate_and_repair` and synthesizes a minimal user
/// message when fewer than 2 messages survive, so caps under one full
/// tool round-trip (user → tool_use → tool_result → assistant text)
/// defeat the safe-trim heuristic entirely.
pub(super) const MIN_HISTORY_MESSAGES: usize = 4;

/// Resolve the effective message-history trim cap for an agent loop entry.
///
/// Resolution order:
/// 1. `manifest.max_history_messages` (per-agent override)
/// 2. `opts.max_history_messages` (operator / kernel config override)
/// 3. `DEFAULT_MAX_HISTORY_MESSAGES` (compiled-in fallback)
///
/// The resolved value is then clamped up to `MIN_HISTORY_MESSAGES` if it
/// would otherwise defeat the safe-trim heuristic.
pub(super) fn resolve_max_history(manifest: &AgentManifest, opts: &LoopOptions) -> usize {
    let raw = manifest
        .max_history_messages
        .or(opts.max_history_messages)
        .unwrap_or(DEFAULT_MAX_HISTORY_MESSAGES);
    clamp_max_history(raw, &manifest.name)
}

/// Clamp a requested cap up to `MIN_HISTORY_MESSAGES`, logging a warning
/// when the requested value is too low. Returning silently for values at
/// or above the floor keeps logs quiet for the common path.
fn clamp_max_history(requested: usize, agent: &str) -> usize {
    if requested < MIN_HISTORY_MESSAGES {
        warn!(
            agent = %agent,
            requested,
            applied = MIN_HISTORY_MESSAGES,
            "max_history_messages below floor; clamping"
        );
        MIN_HISTORY_MESSAGES
    } else {
        requested
    }
}
