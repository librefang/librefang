//! Message-history trim cap: default, floor, ceiling, and per-loop resolution.
//!
//! Splits the small history-config cluster (`DEFAULT_MAX_HISTORY_MESSAGES`,
//! `MIN_HISTORY_MESSAGES`, `MAX_HISTORY_MESSAGES`, `resolve_max_history`,
//! `clamp_max_history`) out
//! of `agent_loop/mod.rs`. None of these helpers touch the agent loop
//! state directly; they only read `AgentManifest.max_history_messages` /
//! `LoopOptions.max_history_messages` and clamp the result against the
//! safe-trim floor and hard ceiling.

use librefang_types::agent::AgentManifest;
use tracing::warn;

use super::LoopOptions;

/// Default cap for message-history auto-trimming (value: 60).
///
/// 60 gives ~7–10 real turns for typical workflows while keeping the
/// prompt-cache prefix stable. Override per-agent via
/// `AgentManifest.max_history_messages` or globally via
/// `KernelConfig.max_history_messages`.
pub const DEFAULT_MAX_HISTORY_MESSAGES: usize = 60;

/// Hard ceiling for message-history auto-trimming (value: 500).
///
/// Operator and manifest overrides above this value are clamped down with a
/// warning log before the cap reaches `safe_trim_messages`.
pub const MAX_HISTORY_MESSAGES: usize = 500;

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
#[must_use]
pub(super) fn resolve_max_history(manifest: &AgentManifest, opts: &LoopOptions) -> usize {
    let raw = manifest
        .max_history_messages
        .or(opts.max_history_messages)
        .unwrap_or(DEFAULT_MAX_HISTORY_MESSAGES);
    clamp_max_history(raw, &manifest.name)
}

/// Clamp a requested cap into the supported range, logging a warning when the
/// requested value is outside it. Returning silently for values inside the
/// range keeps logs quiet for the common path.
#[must_use]
#[inline]
fn clamp_max_history(requested: usize, agent_name: &str) -> usize {
    if requested > MAX_HISTORY_MESSAGES {
        warn!(
            agent = %agent_name,
            requested,
            applied = MAX_HISTORY_MESSAGES,
            "max_history_messages above hard upper limit; clamping"
        );
        return MAX_HISTORY_MESSAGES;
    }
    if requested < MIN_HISTORY_MESSAGES {
        warn!(
            agent = %agent_name,
            requested,
            applied = MIN_HISTORY_MESSAGES,
            "max_history_messages below floor; clamping"
        );
        MIN_HISTORY_MESSAGES
    } else {
        requested
    }
}
