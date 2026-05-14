//! Gateway-level safety-net compression (#4972).
//!
//! A cheap, deterministic, non-LLM pass that runs at the very top of the
//! agent loop — *before* the first prompt build, before any LLM call, and
//! before the LLM-based [`crate::context_compressor`] / `compactor` would
//! ever get a chance to run.
//!
//! ## Why a second compression seam exists
//!
//! The LLM-based agent-level compactor (`context_compressor.rs`) runs
//! *inside* the agent loop, only after the loop has decided to do work.
//! That's the right home for proper history summarisation, but it leaves a
//! gap: sessions that grow *between* turns (overnight channel backlog,
//! cron-job output piling up, parallel `agent_send` fan-in) can already
//! exceed the model's context window by the time the next turn starts.
//! The first LLM call then 400s with `context too long` and the
//! agent-level compactor never gets to fire.
//!
//! ## What this pass does (and does not) do
//!
//! - Cheap rough-token estimation via [`crate::compactor::estimate_token_count`].
//! - At 85 % of the model's context window (configurable), stub any tool
//!   result longer than `max_tool_result_chars` (default 200), preserving
//!   `tool_use_id` pairing so the assistant ↔ tool-result chain stays
//!   well-formed for the provider. Stubbed bytes are replaced with
//!   `[Gateway-pruned tool result — N chars elided]`.
//! - If the session is still over threshold after stubbing, drop the
//!   oldest non-pinned, non-system messages, in pairs that respect
//!   tool-use ↔ tool-result chains, until either (a) the estimate falls
//!   below threshold, or (b) only the last `keep_recent_messages` (default
//!   5) non-pinned messages remain.
//! - **Never** call the LLM. **Never** allocate via [`std::time::Instant`]
//!   or [`HashMap`] iteration. Same inputs → same outputs every call —
//!   prompt-cache stability depends on this.
//!
//! ## What is deliberately deferred to the agent-level compactor
//!
//! LLM summarisation, semantic chunk grouping, retention rules per memory
//! tier — all of that lives in `compactor.rs` / `context_compressor.rs`
//! and is out of scope here. The gateway pass aims only to drop the
//! estimate below ~0.80 so the agent-level compactor can run normally.
//!
//! ## Pinning policy
//!
//! Messages with `Message::pinned = true` are never dropped and never
//! mutated (see #2067 / #3563 — pinned typically means delegation results
//! that downstream code relies on by index). They still count toward the
//! token estimate. If pinned messages alone push the session over the
//! threshold, the gateway pass logs and gives up — the agent-level
//! compactor's LLM summarisation is the only remedy.

use librefang_types::config::GatewayCompressionConfig;
use librefang_types::message::{ContentBlock, Message, MessageContent, Role};

use crate::compactor::estimate_token_count;

/// Summary of what `apply_if_needed` mutated, for logging.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GatewayCompressionReport {
    /// True when the pass actually fired (estimate exceeded threshold).
    pub fired: bool,
    /// True when the pass was skipped because gateway compression is disabled.
    pub disabled: bool,
    /// Estimated tokens before pruning.
    pub tokens_before: usize,
    /// Estimated tokens after pruning. Equal to `tokens_before` when nothing
    /// changed (either disabled or below threshold).
    pub tokens_after: usize,
    /// Number of tool-result blocks that were stubbed.
    pub tool_results_stubbed: usize,
    /// Bytes of tool-result content elided (sum of `content.len()` before
    /// stubbing, minus stub length).
    pub tool_result_bytes_elided: usize,
    /// Number of (whole) messages dropped from the front of history.
    pub messages_dropped: usize,
}

impl GatewayCompressionReport {
    /// Returns true when any visible mutation happened. Used by the caller
    /// to decide between `info!` (something changed) and `debug!` (no-op).
    pub fn mutated(&self) -> bool {
        self.tool_results_stubbed > 0 || self.messages_dropped > 0
    }
}

/// Run the gateway compression pass if the session is over threshold.
///
/// `history` is mutated in place. Returns a report describing what (if
/// anything) was pruned. Callers should log at `info!` when
/// `report.mutated()` and at `debug!` otherwise.
///
/// Pure function: no I/O, no async, no LLM. Safe to call from any context.
pub fn apply_if_needed(
    history: &mut Vec<Message>,
    ctx_window: u32,
    cfg: &GatewayCompressionConfig,
) -> GatewayCompressionReport {
    let mut report = GatewayCompressionReport::default();

    if !cfg.enabled {
        report.disabled = true;
        return report;
    }

    // ctx_window == 0 means "model context window unknown" (the kernel
    // upstream couldn't resolve the model in the catalog). We can't make a
    // ratio judgement without it, so no-op rather than guess.
    if ctx_window == 0 {
        return report;
    }

    let threshold_ratio = clamp_ratio(cfg.threshold_ratio);
    let threshold = (ctx_window as f64 * threshold_ratio as f64) as usize;

    let tokens_before = estimate_token_count(history, None, None);
    report.tokens_before = tokens_before;
    report.tokens_after = tokens_before;

    if tokens_before <= threshold {
        // Under threshold — agent-level compactor handles its own decision.
        return report;
    }

    report.fired = true;

    // Phase 1: stub oversized tool results in place.
    let (stubbed, bytes_elided) = stub_large_tool_results(history, cfg.max_tool_result_chars);
    report.tool_results_stubbed = stubbed;
    report.tool_result_bytes_elided = bytes_elided;

    let after_phase1 = estimate_token_count(history, None, None);
    report.tokens_after = after_phase1;

    if after_phase1 <= threshold {
        return report;
    }

    // Phase 2: drop oldest non-pinned messages until either the estimate
    // falls below threshold or only `keep_recent_messages` non-pinned
    // remain. We walk forward and remove whole tool_use ↔ tool_result
    // pairs together so the chain stays well-formed.
    let dropped = drop_oldest_until_under(history, threshold, cfg.keep_recent_messages);
    report.messages_dropped = dropped;
    report.tokens_after = estimate_token_count(history, None, None);

    report
}

fn clamp_ratio(r: f32) -> f32 {
    // Allow operators to push the gateway threshold high (e.g. 0.95) but
    // not above 1.0 (free pass) or below the agent-compactor default
    // (0.70) — otherwise we'd permanently steal the LLM compactor's job.
    r.clamp(0.70, 0.99)
}

/// Walk `history` in place and stub any `ToolResult.content` longer than
/// `max_chars`. Preserves `tool_use_id` so the assistant ↔ tool-result
/// chain stays well-formed for the provider. Returns
/// `(count_stubbed, total_bytes_elided)`.
fn stub_large_tool_results(history: &mut [Message], max_chars: usize) -> (usize, usize) {
    let mut count = 0usize;
    let mut bytes_elided = 0usize;

    for msg in history.iter_mut() {
        if msg.pinned {
            continue;
        }
        let MessageContent::Blocks(blocks) = &mut msg.content else {
            continue;
        };
        for block in blocks.iter_mut() {
            if let ContentBlock::ToolResult { content, .. } = block {
                let original_len = content.len();
                if original_len > max_chars {
                    let stub =
                        format!("[Gateway-pruned tool result — {original_len} chars elided]");
                    bytes_elided += original_len.saturating_sub(stub.len());
                    *content = stub;
                    count += 1;
                }
            }
        }
    }

    (count, bytes_elided)
}

/// Drop oldest non-pinned, non-system messages until either the token
/// estimate falls below `threshold` or only `keep_recent` non-pinned
/// messages remain in the history. A `tool_use` assistant message is
/// dropped together with its paired `tool_result` user message so the
/// chain stays well-formed.
///
/// Returns the number of messages removed. Deterministic: same input
/// always produces the same removal sequence.
fn drop_oldest_until_under(
    history: &mut Vec<Message>,
    threshold: usize,
    keep_recent: usize,
) -> usize {
    let mut dropped = 0usize;

    // Termination is guaranteed by two explicit breaks below:
    //   (1) `non_pinned_count <= keep_recent` once enough drops have happened, and
    //   (2) `next_droppable_index` returning `None` when every remaining
    //       non-pinned candidate is locked by a pinned paired partner.
    // Each iteration removes at least one message (1 for solo, 2 for a
    // tool_use/tool_result pair), so the non-pinned count is strictly
    // monotonically decreasing — `loop {}` cannot spin.
    loop {
        // Non-pinned, non-system count. When it hits keep_recent we stop.
        let non_pinned_count = history
            .iter()
            .filter(|m| !m.pinned && m.role != Role::System)
            .count();
        if non_pinned_count <= keep_recent {
            break;
        }

        if estimate_token_count(history, None, None) <= threshold {
            break;
        }

        // Find the oldest droppable candidate. A `tool_use` whose paired
        // `tool_result` at `idx+1` is pinned is untouchable — dropping
        // the `tool_use` alone would orphan the pinned `tool_result` and
        // the provider would 400 on the resulting payload. Skip past
        // such stuck pairs and keep searching forward.
        let Some(idx) = next_droppable_index(history) else {
            // Every remaining non-pinned candidate is locked by a pinned
            // paired partner. No further progress possible; bail rather
            // than spin.
            break;
        };

        // If this message holds a tool_use, its paired tool_result lives in
        // the *next* message (assistant tool_use → user tool_result is the
        // canonical layout). Drop both together so the chain stays whole.
        // `next_droppable_index` guarantees the paired `tool_result`, if
        // present, is itself non-pinned.
        let removed_now = if message_contains_tool_use(&history[idx])
            && idx + 1 < history.len()
            && message_contains_tool_result(&history[idx + 1])
            && !history[idx + 1].pinned
        {
            history.remove(idx);
            history.remove(idx); // shifted up — same index
            2
        } else if message_contains_tool_result(&history[idx]) {
            // Lone orphaned tool_result (preceding tool_use already gone
            // or pinned). Drop it solo — it would otherwise be rejected
            // by the provider as unmatched.
            history.remove(idx);
            1
        } else {
            history.remove(idx);
            1
        };

        dropped += removed_now;
    }

    dropped
}

/// Return the index of the oldest non-pinned, non-system message that is
/// safe to drop. A `tool_use` followed by a pinned `tool_result` is NOT
/// safe (dropping the tool_use would orphan the pinned tool_result), so
/// we step past it and keep looking. Returns `None` when no droppable
/// candidate exists.
fn next_droppable_index(history: &[Message]) -> Option<usize> {
    let mut i = 0;
    while i < history.len() {
        let msg = &history[i];
        if msg.pinned || msg.role == Role::System {
            i += 1;
            continue;
        }
        // A tool_use whose paired tool_result is pinned is untouchable —
        // skip past both to look for a later candidate.
        if message_contains_tool_use(msg)
            && i + 1 < history.len()
            && message_contains_tool_result(&history[i + 1])
            && history[i + 1].pinned
        {
            i += 2;
            continue;
        }
        return Some(i);
    }
    None
}

fn message_contains_tool_use(msg: &Message) -> bool {
    if let MessageContent::Blocks(blocks) = &msg.content {
        blocks
            .iter()
            .any(|b| matches!(b, ContentBlock::ToolUse { .. }))
    } else {
        false
    }
}

fn message_contains_tool_result(msg: &Message) -> bool {
    if let MessageContent::Blocks(blocks) = &msg.content {
        blocks
            .iter()
            .any(|b| matches!(b, ContentBlock::ToolResult { .. }))
    } else {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use librefang_types::message::{ContentBlock, Message, MessageContent, Role};
    use librefang_types::tool::ToolExecutionStatus;

    fn small_user(text: &str) -> Message {
        Message::user(text.to_string())
    }

    fn small_assistant(text: &str) -> Message {
        Message::assistant(text.to_string())
    }

    fn tool_use_msg(id: &str, name: &str) -> Message {
        Message {
            role: Role::Assistant,
            content: MessageContent::Blocks(vec![ContentBlock::ToolUse {
                id: id.to_string(),
                name: name.to_string(),
                input: serde_json::json!({}),
                provider_metadata: None,
            }]),
            pinned: false,
            timestamp: None,
        }
    }

    fn tool_result_msg(id: &str, content: &str) -> Message {
        Message {
            role: Role::User,
            content: MessageContent::Blocks(vec![ContentBlock::ToolResult {
                tool_use_id: id.to_string(),
                tool_name: "demo".to_string(),
                content: content.to_string(),
                is_error: false,
                status: ToolExecutionStatus::default(),
                approval_request_id: None,
            }]),
            pinned: false,
            timestamp: None,
        }
    }

    fn default_cfg() -> GatewayCompressionConfig {
        GatewayCompressionConfig::default()
    }

    #[test]
    fn under_threshold_is_noop() {
        let mut history = vec![small_user("hi"), small_assistant("hello")];
        let original = history.clone();
        let report = apply_if_needed(&mut history, 200_000, &default_cfg());
        assert!(!report.fired, "report = {report:?}");
        assert_eq!(report.messages_dropped, 0);
        assert_eq!(report.tool_results_stubbed, 0);
        assert_eq!(history.len(), original.len());
        assert!(!report.mutated());
    }

    #[test]
    fn disabled_is_noop_even_when_huge() {
        // Bloated history but config disabled.
        let mut history: Vec<Message> = (0..200)
            .map(|i| small_user(&format!("padding-{i}-{}", "x".repeat(1000))))
            .collect();
        let cfg = GatewayCompressionConfig {
            enabled: false,
            ..GatewayCompressionConfig::default()
        };
        let before = history.len();
        let report = apply_if_needed(&mut history, 1_000, &cfg);
        assert!(report.disabled);
        assert!(!report.fired);
        assert_eq!(history.len(), before);
    }

    #[test]
    fn unknown_ctx_window_is_noop() {
        let mut history = vec![small_user(&"x".repeat(10_000))];
        let before = history.len();
        let report = apply_if_needed(&mut history, 0, &default_cfg());
        assert!(!report.fired);
        assert_eq!(history.len(), before);
    }

    #[test]
    fn stubs_oversized_tool_results_first() {
        // One big tool result + an assistant tool_use referencing it.
        // Context window deliberately tiny so a single big result puts us
        // over threshold without needing message drops.
        let big = "y".repeat(20_000);
        let mut history = vec![
            small_user("please run the tool"),
            tool_use_msg("call-1", "demo"),
            tool_result_msg("call-1", &big),
            small_user("thanks"),
        ];
        // Roughly: 20_000 chars / 4 = 5_000 tokens. Threshold at 0.85 *
        // 1_000 = 850. So firmly over.
        let report = apply_if_needed(&mut history, 1_000, &default_cfg());
        assert!(report.fired);
        assert_eq!(report.tool_results_stubbed, 1);
        assert!(report.tool_result_bytes_elided > 0);

        // Verify the stub message replaced the content and tool_use_id
        // was preserved.
        match &history[2].content {
            MessageContent::Blocks(blocks) => match &blocks[0] {
                ContentBlock::ToolResult {
                    tool_use_id,
                    content,
                    ..
                } => {
                    assert_eq!(tool_use_id, "call-1");
                    assert!(content.starts_with("[Gateway-pruned tool result"));
                }
                other => panic!("expected ToolResult, got {other:?}"),
            },
            other => panic!("expected Blocks, got {other:?}"),
        }
        // tool_use side is structurally intact.
        assert!(message_contains_tool_use(&history[1]));
    }

    #[test]
    fn pinned_messages_never_pruned_or_mutated() {
        // A pinned message with a huge tool result should NOT be stubbed,
        // even when the gateway pass fires.
        let big = "z".repeat(20_000);
        let mut pinned_huge_result = tool_result_msg("call-pinned", &big);
        pinned_huge_result.pinned = true;
        let mut history = vec![
            pinned_huge_result.clone(),
            small_user("a"),
            small_user("b"),
            small_user("c"),
        ];
        let _report = apply_if_needed(&mut history, 1_000, &default_cfg());
        // First message is still there, still pinned, still has its big
        // content (untouched).
        assert!(history[0].pinned);
        match &history[0].content {
            MessageContent::Blocks(blocks) => match &blocks[0] {
                ContentBlock::ToolResult { content, .. } => {
                    assert_eq!(content.len(), 20_000);
                }
                _ => panic!("expected ToolResult"),
            },
            _ => panic!("expected Blocks"),
        }
    }

    #[test]
    fn drops_oldest_until_under_keep_recent_floor() {
        // Many medium-sized messages, none pinned, no tool results to stub.
        // Forces the message-drop phase.
        let mut history: Vec<Message> = (0..40)
            .map(|i| small_user(&format!("msg-{i}-{}", "x".repeat(500))))
            .collect();

        let cfg = GatewayCompressionConfig {
            keep_recent_messages: 5,
            ..GatewayCompressionConfig::default()
        };
        let report = apply_if_needed(&mut history, 1_000, &cfg);
        assert!(report.fired);
        assert!(report.messages_dropped > 0);
        // keep_recent floor enforced: never drops past the last 5
        // non-pinned messages.
        assert!(history.len() >= 5);
        // The drop loop stops at whichever bound hits first — threshold or
        // keep_recent. So `n` should be in `[40 - report.messages_dropped,
        // 35]` (35 is the keep_recent floor for 40 messages). The
        // important invariant is "old messages got dropped, recent ones
        // survived" — not an exact id.
        match &history[0].content {
            MessageContent::Text(s) => {
                let n: i32 = s
                    .strip_prefix("msg-")
                    .and_then(|rest| rest.split('-').next())
                    .and_then(|n| n.parse().ok())
                    .expect("parseable id");
                assert!(n > 0, "first surviving msg id = {n} (no drops?)");
                assert!(
                    n as usize <= 40 - cfg.keep_recent_messages,
                    "first surviving msg id = {n} should not exceed keep_recent floor"
                );
            }
            _ => panic!("expected Text"),
        }
        // And the last message — the most recent one — is always preserved.
        match &history.last().expect("non-empty").content {
            MessageContent::Text(s) => assert!(s.starts_with("msg-39-")),
            _ => panic!("expected Text"),
        }
    }

    #[test]
    fn tool_use_and_result_dropped_as_pair() {
        let mut history = vec![
            tool_use_msg("call-A", "demo"),
            tool_result_msg("call-A", &"a".repeat(100)),
            small_user(&format!("padding {}", "y".repeat(2_000))),
            small_user(&format!("padding2 {}", "z".repeat(2_000))),
            small_user(&format!("padding3 {}", "q".repeat(2_000))),
            small_user(&format!("padding4 {}", "w".repeat(2_000))),
            small_user("recent"),
        ];
        let cfg = GatewayCompressionConfig {
            // Small chunk size irrelevant — tool result content is short.
            max_tool_result_chars: 200,
            keep_recent_messages: 3,
            ..GatewayCompressionConfig::default()
        };
        let report = apply_if_needed(&mut history, 1_000, &cfg);
        assert!(report.fired);
        // tool_use and its paired tool_result were both dropped, OR neither
        // was. Verify the chain is still well-formed — for every
        // tool_result remaining there is a preceding tool_use with the
        // same id.
        for (i, msg) in history.iter().enumerate() {
            if let MessageContent::Blocks(blocks) = &msg.content {
                for b in blocks {
                    if let ContentBlock::ToolResult { tool_use_id, .. } = b {
                        // Search prior messages for the matching tool_use.
                        let mut found = false;
                        for prior in history.iter().take(i) {
                            if let MessageContent::Blocks(pblocks) = &prior.content {
                                for pb in pblocks {
                                    if let ContentBlock::ToolUse { id, .. } = pb {
                                        if id == tool_use_id {
                                            found = true;
                                            break;
                                        }
                                    }
                                }
                            }
                            if found {
                                break;
                            }
                        }
                        assert!(
                            found,
                            "orphan tool_result at index {i} with id {tool_use_id}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn most_recent_messages_are_preserved() {
        let mut history: Vec<Message> = (0..30)
            .map(|i| small_user(&format!("msg-{i}-{}", "x".repeat(800))))
            .collect();
        let cfg = GatewayCompressionConfig {
            keep_recent_messages: 5,
            ..GatewayCompressionConfig::default()
        };
        let _report = apply_if_needed(&mut history, 1_000, &cfg);
        // The very last message ("msg-29-…") is always preserved.
        let last = history.last().expect("non-empty after prune");
        match &last.content {
            MessageContent::Text(s) => assert!(s.starts_with("msg-29-")),
            _ => panic!("expected Text"),
        }
    }

    #[test]
    fn deterministic_across_calls() {
        // Same input → same output, twice in a row, byte-for-byte.
        // Strip wall-clock timestamps (set inside `Message::user/assistant`
        // via `Utc::now()`) so the JSON comparison reflects only the
        // pruning algorithm, not construction-time clock drift.
        let build = || -> Vec<Message> {
            let big = "p".repeat(15_000);
            let mut msgs = vec![
                small_user("intro"),
                tool_use_msg("call-x", "demo"),
                tool_result_msg("call-x", &big),
                small_assistant("ok"),
                small_user("next"),
                small_user("more"),
                small_user("final"),
            ];
            for m in msgs.iter_mut() {
                m.timestamp = None;
            }
            msgs
        };
        let mut h1 = build();
        let mut h2 = build();
        let r1 = apply_if_needed(&mut h1, 1_000, &default_cfg());
        let r2 = apply_if_needed(&mut h2, 1_000, &default_cfg());
        assert_eq!(r1, r2);
        // Compare serialised history to catch any nondeterminism in
        // ordering / content.
        let s1 = serde_json::to_string(&h1).expect("ser h1");
        let s2 = serde_json::to_string(&h2).expect("ser h2");
        assert_eq!(s1, s2);
    }

    #[test]
    fn threshold_ratio_clamped_low() {
        // A user trying to set 0.10 (below the compactor's 0.70) gets
        // clamped to 0.70 — otherwise the gateway would steal the
        // LLM-compactor's job permanently.
        assert_eq!(clamp_ratio(0.10), 0.70);
        assert_eq!(clamp_ratio(0.85), 0.85);
        assert_eq!(clamp_ratio(1.5), 0.99);
    }

    #[test]
    fn tool_use_with_pinned_paired_result_is_not_dropped() {
        // Phase-2 regression: a non-pinned tool_use whose paired
        // tool_result at idx+1 is pinned MUST stay whole. Single-dropping
        // the tool_use would orphan the pinned tool_result and providers
        // 400 on that shape. The chain has to remain well-formed.
        let mut pinned_result = tool_result_msg("call-keep", &"k".repeat(200));
        pinned_result.pinned = true;
        let mut history = vec![
            tool_use_msg("call-keep", "demo"),
            pinned_result,
            // Padding to force the gateway pass to fire and look for
            // candidates to drop.
            small_user(&format!("padding-1 {}", "y".repeat(2_000))),
            small_user(&format!("padding-2 {}", "z".repeat(2_000))),
            small_user(&format!("padding-3 {}", "q".repeat(2_000))),
            small_user("recent"),
        ];
        let cfg = GatewayCompressionConfig {
            keep_recent_messages: 2,
            ..GatewayCompressionConfig::default()
        };
        let report = apply_if_needed(&mut history, 1_000, &cfg);
        assert!(report.fired, "should have fired: {report:?}");

        // The pinned tool_result must still be present and pinned.
        let pinned_still_present = history.iter().any(|m| {
            m.pinned
                && matches!(
                    &m.content,
                    MessageContent::Blocks(b)
                        if b.iter().any(|cb| matches!(
                            cb,
                            ContentBlock::ToolResult { tool_use_id, .. }
                                if tool_use_id == "call-keep"
                        ))
                )
        });
        assert!(
            pinned_still_present,
            "pinned tool_result must survive — history = {history:?}"
        );

        // Orphan-free invariant: every remaining tool_result has a
        // preceding tool_use with the same id.
        for (i, msg) in history.iter().enumerate() {
            if let MessageContent::Blocks(blocks) = &msg.content {
                for b in blocks {
                    if let ContentBlock::ToolResult { tool_use_id, .. } = b {
                        let has_pair = history.iter().take(i).any(|prior| {
                            matches!(
                                &prior.content,
                                MessageContent::Blocks(pb)
                                    if pb.iter().any(|cb| matches!(
                                        cb,
                                        ContentBlock::ToolUse { id, .. } if id == tool_use_id
                                    ))
                            )
                        });
                        assert!(
                            has_pair,
                            "orphan tool_result at idx {i} with id {tool_use_id} — \
                             history = {history:?}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn drop_loop_terminates_when_no_candidate_can_be_dropped() {
        // Every non-pinned candidate is a tool_use locked by a pinned
        // paired tool_result. Plus a system message and a pinned message,
        // none of which are ever droppable. There must be nothing the
        // drop loop CAN drop, and it must terminate (not spin) — even
        // though we're well over threshold.
        let big = "Q".repeat(8_000);
        let mut pinned_result_1 = tool_result_msg("call-1", &big);
        pinned_result_1.pinned = true;
        let mut pinned_result_2 = tool_result_msg("call-2", &big);
        pinned_result_2.pinned = true;
        let mut history = vec![
            tool_use_msg("call-1", "demo"),
            pinned_result_1,
            tool_use_msg("call-2", "demo"),
            pinned_result_2,
        ];
        let before = history.clone();
        let report = apply_if_needed(&mut history, 1_000, &default_cfg());
        assert!(report.fired);
        // Nothing was droppable — history must be identical to the input.
        assert_eq!(history.len(), before.len(), "history length changed");
        assert_eq!(report.messages_dropped, 0);
        // Pinned tool_results both still present and still pinned.
        assert!(history[1].pinned);
        assert!(history[3].pinned);
        // tool_use ↔ pinned tool_result chains both intact.
        assert!(message_contains_tool_use(&history[0]));
        assert!(message_contains_tool_result(&history[1]));
        assert!(message_contains_tool_use(&history[2]));
        assert!(message_contains_tool_result(&history[3]));
    }

    #[test]
    fn entirely_pinned_history_gives_up_gracefully() {
        // If every message is pinned, the drop phase can't proceed and
        // must terminate rather than loop. We don't enforce a specific
        // post-condition on tokens — the LLM compactor's summarisation
        // is the only remedy in that case.
        let big = "Q".repeat(15_000);
        let mut pinned_msg = tool_result_msg("call-p", &big);
        pinned_msg.pinned = true;
        let mut history = vec![pinned_msg.clone(), pinned_msg.clone(), pinned_msg];
        let report = apply_if_needed(&mut history, 1_000, &default_cfg());
        assert!(report.fired);
        // All three still there (pinned protects them at both phases).
        assert_eq!(history.len(), 3);
    }
}
