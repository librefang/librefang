//! Tool-result history fold — mechanism 3 of #3347.
//!
//! After `history_fold_after_turns` assistant turns have elapsed, tool-result
//! messages from those older turns are "stale" and contribute noise to the
//! context window without materially helping the agent.  This module folds
//! them into compact summaries so the LLM sees the history without the raw
//! payload bulk.
//!
//! # Algorithm
//!
//! 1. Walk `messages` and count assistant turns.  Any tool-result user message
//!    that was answered before the most recent `history_fold_after_turns`
//!    assistant turns is marked stale.
//! 2. Group consecutive stale tool-result messages and ask the aux-LLM (or the
//!    primary driver when no aux chain is configured) to produce a 1–2 sentence
//!    summary per group.
//! 3. Rewrite **each `ContentBlock::ToolResult` in the stale group in place**:
//!    `tool_use_id` / `tool_name` / `is_error` / `status` are preserved, only
//!    `content` is replaced with `"[history-fold] <summary>"`.
//! 4. Pinned messages are never folded (they are protected work product).
//!
//! # tool_use ↔ tool_result pairing — why we don't replace messages
//!
//! Earlier drafts of this module replaced each stale tool-result message with
//! a single `Message::user(Text("[history-fold] ..."))` plain-text message.
//! That broke conversation invariants: provider APIs (Anthropic Messages,
//! OpenAI Responses, Gemini function_call) require every assistant
//! `tool_use` block to be answered by a `tool_result` block carrying the
//! same `tool_use_id`.  Replacing the user message with free-form text
//! left the matching assistant `tool_use` orphaned, and the next provider
//! call returned `400 invalid_request_error: messages: tool_use ids must
//! match tool_result tool_use_ids`.
//!
//! Rewriting `ToolResult.content` in place keeps the `Blocks([ToolResult{
//! tool_use_id, ...}])` shape so every original `tool_use` still has its
//! matching `tool_result` — only the payload contracts.
//!
//! # Boundary choice
//!
//! Folding runs at the **pre-LLM-call boundary** (same as context compression)
//! so that the LLM always sees the compacted history regardless of whether the
//! session was loaded from disk mid-flight.  Running at session-load would also
//! work but would require async I/O at load time, complicating the sync path.
//!
//! # Fallback
//!
//! When the aux-LLM call fails (no key configured, network error, empty
//! response), the fold falls back to a static stub:
//! `[history-fold: <count> tool result(s) summarisation unavailable]`
//! This ensures the stale payload is still removed from context even when the
//! LLM is unavailable.

use crate::aux_client::AuxClient;
use crate::llm_driver::{CompletionRequest, LlmDriver};
use librefang_types::config::AuxTask;
use librefang_types::message::{ContentBlock, Message, MessageContent, Role};
use std::sync::Arc;
use tracing::{debug, info, warn};

/// Prefix used in folded summary messages so agents and downstream code can
/// recognise that earlier tool results were compacted.
const FOLD_PREFIX: &str = "[history-fold]";

/// Number of characters shown in the per-tool preview inside the fold prompt.
const FOLD_PREVIEW_CHARS: usize = 500;

/// Result of a single fold pass.
#[derive(Debug, Default)]
pub struct FoldResult {
    /// Number of groups that were folded.
    pub groups_folded: usize,
    /// Total tool-result messages that were replaced.
    pub messages_replaced: usize,
    /// Number of groups that fell back to the static stub because the
    /// aux-LLM summarisation failed (#7 review follow-up: was previously a
    /// single-bit `used_fallback`, which lost fidelity when one group in
    /// the pass failed and others succeeded).
    pub groups_used_fallback: usize,
}

/// Fold stale tool-result messages in `messages`.
///
/// `fold_after_turns` — fold tool results older than this many assistant turns.
/// `min_batch_size` — only run a fold pass when at least this many newly-stale
/// (i.e. not already-folded) tool-result messages have accumulated. Amortises
/// the aux-LLM cost on long sessions where each new turn would otherwise drag
/// exactly one new message across the staleness boundary and trigger another
/// fold call. Set to `1` to disable the batch threshold (fold every turn);
/// `0` is treated as `1`.
/// `model` — model slug forwarded to the summariser.
/// `aux_client` — optional aux-LLM client; when `None`, fallback text is used.
/// `driver` — primary driver (used when aux chain resolves to primary).
///
/// Returns the (possibly modified) message list and a [`FoldResult`] summary.
pub async fn fold_stale_tool_results(
    mut messages: Vec<Message>,
    fold_after_turns: u32,
    min_batch_size: u32,
    model: &str,
    aux_client: Option<&AuxClient>,
    driver: Arc<dyn LlmDriver>,
) -> (Vec<Message>, FoldResult) {
    if fold_after_turns == 0 {
        return (messages, FoldResult::default());
    }

    // Walk backwards to find the stale boundary.  Count assistant turns from
    // the end; tool-result messages whose assistant-turn distance exceeds
    // `fold_after_turns` are stale.
    let stale_indices = collect_stale_indices(&messages, fold_after_turns as usize);

    if stale_indices.is_empty() {
        return (messages, FoldResult::default());
    }

    // Cost amortiser (#3 review follow-up): on a long-running session every
    // new turn pushes exactly one fresh message across the staleness
    // boundary, which would trigger an aux-LLM call per turn just to fold a
    // single message. Skip the pass until at least `min_batch_size`
    // newly-stale messages have accumulated.
    let batch_size = std::cmp::max(min_batch_size, 1) as usize;
    if stale_indices.len() < batch_size {
        debug!(
            stale_count = stale_indices.len(),
            min_batch_size = batch_size,
            "history_fold: skip — newly-stale below batch threshold"
        );
        return (messages, FoldResult::default());
    }

    debug!(
        stale_count = stale_indices.len(),
        fold_after_turns, "history_fold: folding stale tool-result messages"
    );

    // Resolve the summarisation driver (aux preferred, primary as fallback).
    let summary_driver = aux_client
        .map(|c| c.driver_for(AuxTask::Fold))
        .unwrap_or_else(|| Arc::clone(&driver));

    // Group consecutive stale indices so we produce one summary call per
    // contiguous run; the summary text is then assigned to the `content`
    // field of every `ToolResult` block in the group.
    let groups = group_consecutive(stale_indices);

    let mut result = FoldResult::default();

    // Compute a summary string for each group.
    let mut group_summaries: Vec<(Vec<usize>, String)> = Vec::with_capacity(groups.len());
    for (g_idx, group) in groups.iter().enumerate() {
        let count = group.len();
        let group_msgs: Vec<&Message> = group.iter().map(|&i| &messages[i]).collect();
        let summary = summarise_group(group_msgs.as_slice(), model, &*summary_driver, g_idx).await;
        let text = match summary {
            Ok(text) => {
                info!(
                    count,
                    "history_fold: summarised group of {count} tool-result(s)"
                );
                text
            }
            Err(e) => {
                warn!(
                    count,
                    error = %e,
                    "history_fold: summarisation failed, using fallback stub"
                );
                result.groups_used_fallback += 1;
                FALLBACK_SUMMARY.to_string()
            }
        };
        result.groups_folded += 1;
        result.messages_replaced += count;
        group_summaries.push((group.clone(), text));
    }

    // Apply the summary by rewriting `ContentBlock::ToolResult.content` in
    // place — preserving `tool_use_id` / `tool_name` / `is_error` / `status`
    // so every original assistant `tool_use` block keeps its matching
    // `tool_result` block.  This is the difference vs. the earlier draft
    // that emitted a free-text stub: provider APIs reject mismatched
    // tool_use_ids with `400 invalid_request_error`. (Pairing-preservation
    // is module-doc invariant; see the test suite below.)
    for (group, summary) in &group_summaries {
        let stub_content = format!("{FOLD_PREFIX} {summary}");
        for &i in group {
            if let MessageContent::Blocks(blocks) = &mut messages[i].content {
                for block in blocks.iter_mut() {
                    if let ContentBlock::ToolResult { content, .. } = block {
                        *content = stub_content.clone();
                    }
                }
            }
        }
    }

    (messages, result)
}

/// Static-stub text used when the aux-LLM summarisation call fails.  Kept as
/// a const so tests and call sites can spot it without string-matching.
const FALLBACK_SUMMARY: &str = "[summarisation unavailable]";

// ─────────────────────────────────────────────────────────────────────────────
// Internal helpers
// ─────────────────────────────────────────────────────────────────────────────

/// Return the indices (into `messages`) of tool-result user messages that are
/// older than `fold_after_turns` assistant turns from the end.
///
/// A message is a "tool-result message" when its content is a `Blocks` vec
/// that contains at least one `ContentBlock::ToolResult` block AND it has the
/// `User` role.  Pinned messages are never stale.
fn collect_stale_indices(messages: &[Message], fold_after_turns: usize) -> Vec<usize> {
    // Walk backwards, count assistant messages, mark the boundary index.
    let mut assistant_turns_seen = 0usize;
    let mut boundary_idx = messages.len(); // exclusive upper bound for "recent" turns

    for (i, msg) in messages.iter().enumerate().rev() {
        if msg.role == Role::Assistant {
            assistant_turns_seen += 1;
            if assistant_turns_seen == fold_after_turns {
                // Everything at index < i is from before this boundary.
                boundary_idx = i;
                break;
            }
        }
    }

    if assistant_turns_seen < fold_after_turns {
        // Not enough history yet.
        return Vec::new();
    }

    // Collect stale tool-result indices.
    // Messages that already start with FOLD_PREFIX are previously-folded stubs;
    // skip them to avoid redundant re-summarisation on every subsequent turn.
    messages
        .iter()
        .enumerate()
        .filter(|(i, msg)| {
            *i < boundary_idx
                && !msg.pinned
                && msg.role == Role::User
                && is_tool_result_message(msg)
                && !is_already_folded(msg)
        })
        .map(|(i, _)| i)
        .collect()
}

/// Returns true when `msg` is a user message whose content consists entirely
/// (or partially) of `ToolResult` blocks.
fn is_tool_result_message(msg: &Message) -> bool {
    match &msg.content {
        MessageContent::Blocks(blocks) => blocks
            .iter()
            .any(|b| matches!(b, ContentBlock::ToolResult { .. })),
        _ => false,
    }
}

/// Returns true when `msg`'s tool-result blocks have already been collapsed
/// by a prior fold pass.  Detection: any `ToolResult.content` starts with
/// `FOLD_PREFIX`.  These messages are cheap stubs and re-folding them would
/// produce summary-of-summary output AND burn another aux-LLM call.
fn is_already_folded(msg: &Message) -> bool {
    match &msg.content {
        MessageContent::Blocks(blocks) => blocks.iter().any(|b| {
            matches!(
                b,
                ContentBlock::ToolResult { content, .. } if content.starts_with(FOLD_PREFIX)
            )
        }),
        // Legacy: previous draft emitted plain-text stubs. Recognise those
        // too so an in-flight session that was folded by an older binary
        // doesn't get re-folded on first run after upgrade.
        MessageContent::Text(t) => t.starts_with(FOLD_PREFIX),
    }
}

/// Group a sorted list of indices into consecutive runs.
fn group_consecutive(mut indices: Vec<usize>) -> Vec<Vec<usize>> {
    indices.sort_unstable();
    let mut groups: Vec<Vec<usize>> = Vec::new();
    let mut current: Vec<usize> = Vec::new();

    for idx in indices {
        if current.is_empty() || idx == *current.last().unwrap() + 1 {
            current.push(idx);
        } else {
            groups.push(current);
            current = vec![idx];
        }
    }
    if !current.is_empty() {
        groups.push(current);
    }
    groups
}

/// Ask the LLM to summarise a group of tool-result messages.
async fn summarise_group(
    group: &[&Message],
    model: &str,
    driver: &dyn LlmDriver,
    group_idx: usize,
) -> Result<String, String> {
    // Render the group to a compact text block.
    let mut text = format!("Tool results group {}:\n", group_idx + 1);
    for msg in group {
        match &msg.content {
            MessageContent::Blocks(blocks) => {
                for block in blocks {
                    if let ContentBlock::ToolResult {
                        tool_name, content, ..
                    } = block
                    {
                        let preview: String = content.chars().take(FOLD_PREVIEW_CHARS).collect();
                        let has_more = content.len() > FOLD_PREVIEW_CHARS;
                        text.push_str(&format!("- {tool_name}: {preview}"));
                        if has_more {
                            text.push_str(" ...[truncated]");
                        }
                        text.push('\n');
                    }
                }
            }
            MessageContent::Text(t) => {
                let preview: String = t.chars().take(FOLD_PREVIEW_CHARS).collect();
                text.push_str(&format!("- {preview}\n"));
            }
        }
    }

    let prompt = format!(
        "Summarise the following tool execution results in 1–2 sentences. \
         Capture what each tool did and what it returned, omitting raw data. \
         Output only the summary, no preamble.\n\n{text}"
    );

    let request = CompletionRequest {
        model: model.to_string(),
        messages: Arc::new(vec![Message {
            role: Role::User,
            content: MessageContent::Blocks(vec![ContentBlock::Text {
                text: prompt,
                provider_metadata: None,
            }]),
            pinned: false,
            timestamp: None,
        }]),
        tools: Arc::new(vec![]),
        max_tokens: 256,
        temperature: 0.3,
        system: Some(
            "You are a concise summariser. Produce short factual summaries of tool outputs."
                .to_string(),
        ),
        thinking: None,
        // Each summary prompt embeds distinct tool-result previews, so
        // there is no shared prefix to amortise. Caching only adds the
        // cache-write latency / token cost without any subsequent hit.
        // (#5 review follow-up.)
        prompt_caching: false,
        cache_ttl: None,
        response_format: None,
        timeout_secs: None,
        extra_body: None,
        agent_id: None,
        session_id: None,
        step_id: None,
    };

    match driver.complete(request).await {
        Ok(resp) => {
            let summary = resp.text();
            if summary.is_empty() {
                Err("LLM returned empty summary".to_string())
            } else {
                Ok(summary)
            }
        }
        Err(e) => Err(format!("LLM call failed: {e}")),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm_driver::{CompletionRequest, CompletionResponse, LlmError};
    use librefang_types::message::{ContentBlock, Message, MessageContent, Role};

    // ── Helpers ──────────────────────────────────────────────────────────────

    fn assistant_msg(text: &str) -> Message {
        Message {
            role: Role::Assistant,
            content: MessageContent::Text(text.to_string()),
            pinned: false,
            timestamp: None,
        }
    }

    fn tool_result_msg(tool_name: &str, content: &str) -> Message {
        Message {
            role: Role::User,
            content: MessageContent::Blocks(vec![ContentBlock::ToolResult {
                tool_use_id: "id-1".to_string(),
                tool_name: tool_name.to_string(),
                content: content.to_string(),
                is_error: false,
                status: librefang_types::tool::ToolExecutionStatus::Completed,
                approval_request_id: None,
            }]),
            pinned: false,
            timestamp: None,
        }
    }

    fn user_msg(text: &str) -> Message {
        Message {
            role: Role::User,
            content: MessageContent::Text(text.to_string()),
            pinned: false,
            timestamp: None,
        }
    }

    // ── Mock drivers ─────────────────────────────────────────────────────────

    /// Driver that always returns a fixed summary string.
    struct OkDriver(String);

    #[async_trait::async_trait]
    impl LlmDriver for OkDriver {
        async fn complete(&self, _req: CompletionRequest) -> Result<CompletionResponse, LlmError> {
            Ok(CompletionResponse {
                content: vec![ContentBlock::Text {
                    text: self.0.clone(),
                    provider_metadata: None,
                }],
                tool_calls: vec![],
                stop_reason: librefang_types::message::StopReason::EndTurn,
                usage: librefang_types::message::TokenUsage::default(),
            })
        }
    }

    /// Driver that always returns an error.
    struct FailDriver;

    #[async_trait::async_trait]
    impl LlmDriver for FailDriver {
        async fn complete(&self, _req: CompletionRequest) -> Result<CompletionResponse, LlmError> {
            Err(LlmError::Http("simulated failure".to_string()))
        }
    }

    // ── Tests ─────────────────────────────────────────────────────────────

    /// Build a message list that simulates `n_turns` turns, each containing
    /// one user message, one assistant message, and one tool-result message.
    fn build_history(n_turns: usize) -> Vec<Message> {
        let mut msgs = vec![user_msg("initial question")];
        for i in 0..n_turns {
            msgs.push(assistant_msg(&format!("assistant response {i}")));
            msgs.push(tool_result_msg(
                "shell_exec",
                &format!("output of turn {i}"),
            ));
        }
        msgs
    }

    /// Returns true when any `ToolResult.content` in `msg` starts with
    /// `FOLD_PREFIX` — the post-#1-review fold-detection predicate.
    fn has_folded_tool_result(msg: &Message) -> bool {
        matches!(&msg.content, MessageContent::Blocks(blocks)
        if blocks.iter().any(|b| matches!(
            b,
            ContentBlock::ToolResult { content, .. } if content.starts_with(FOLD_PREFIX)
        )))
    }

    /// Helper: extract every `tool_use_id` from `msg`'s ToolResult blocks.
    /// Used by pairing-preservation tests to confirm fold did not drop ids.
    fn tool_use_ids_in(msg: &Message) -> Vec<String> {
        match &msg.content {
            MessageContent::Blocks(blocks) => blocks
                .iter()
                .filter_map(|b| match b {
                    ContentBlock::ToolResult { tool_use_id, .. } => Some(tool_use_id.clone()),
                    _ => None,
                })
                .collect(),
            _ => Vec::new(),
        }
    }

    #[tokio::test]
    async fn fold_after_8_folds_old_turns() {
        // 10 turns total; fold_after=8 → turns 0 and 1 are stale (oldest 2).
        let messages = build_history(10);
        let pre_ids: Vec<Vec<String>> = messages.iter().map(tool_use_ids_in).collect();
        let driver: Arc<dyn LlmDriver> = Arc::new(OkDriver("nice summary".to_string()));

        let (out, result) =
            fold_stale_tool_results(messages, 8, 1, "test-model", None, driver).await;

        assert!(
            result.groups_folded >= 1,
            "expected at least one group folded"
        );
        assert!(
            result.messages_replaced >= 1,
            "expected at least one message replaced"
        );
        // tool_use_ids must survive — pairing invariant.
        let post_ids: Vec<Vec<String>> = out.iter().map(tool_use_ids_in).collect();
        assert_eq!(
            pre_ids, post_ids,
            "fold must preserve every ToolResult.tool_use_id (pairing invariant)"
        );
        // Folded ToolResult.content must carry the FOLD_PREFIX marker.
        assert!(
            out.iter().any(has_folded_tool_result),
            "expected at least one folded ToolResult in output"
        );
    }

    #[tokio::test]
    async fn no_fold_when_not_enough_turns() {
        // Only 5 turns; fold_after=8 → nothing should be folded.
        let messages = build_history(5);
        let original_len = messages.len();
        let driver: Arc<dyn LlmDriver> = Arc::new(OkDriver("summary".to_string()));

        let (out, result) =
            fold_stale_tool_results(messages, 8, 1, "test-model", None, driver).await;

        assert_eq!(result.groups_folded, 0);
        assert_eq!(result.messages_replaced, 0);
        assert_eq!(out.len(), original_len, "history unchanged");
    }

    #[tokio::test]
    async fn fallback_stub_when_llm_unavailable() {
        // 10 turns, fold_after=8, but the LLM driver always fails.
        let messages = build_history(10);
        let driver: Arc<dyn LlmDriver> = Arc::new(FailDriver);

        let (out, result) =
            fold_stale_tool_results(messages, 8, 1, "test-model", None, driver).await;

        // Should still fold (with fallback stubs).
        assert!(
            result.groups_used_fallback >= 1,
            "expected at least one group to use the fallback stub"
        );
        assert!(result.groups_folded >= 1);
        let fallback_present = out.iter().any(|m| match &m.content {
            MessageContent::Blocks(blocks) => blocks.iter().any(|b| match b {
                ContentBlock::ToolResult { content, .. } => {
                    content.starts_with(FOLD_PREFIX)
                        && content.contains("summarisation unavailable")
                }
                _ => false,
            }),
            _ => false,
        });
        assert!(
            fallback_present,
            "expected fallback summary in a folded ToolResult.content"
        );
    }

    /// Pairing invariant explicit check: feed a turn whose assistant message
    /// has a `ToolUse{id="abc"}` and a corresponding user `ToolResult{
    /// tool_use_id="abc"}`, fold, and confirm the tool_use_id survives.
    #[tokio::test]
    async fn fold_preserves_tool_use_id_pairing() {
        let mut msgs: Vec<Message> = Vec::new();
        msgs.push(user_msg("user question"));
        // Stale turn (fold_after=2 means turns >= 2 from end are stale).
        msgs.push(Message {
            role: Role::Assistant,
            content: MessageContent::Blocks(vec![ContentBlock::ToolUse {
                id: "tid_stale".to_string(),
                name: "shell".to_string(),
                input: serde_json::json!({"cmd": "ls"}),
                provider_metadata: None,
            }]),
            pinned: false,
            timestamp: None,
        });
        msgs.push(Message {
            role: Role::User,
            content: MessageContent::Blocks(vec![ContentBlock::ToolResult {
                tool_use_id: "tid_stale".to_string(),
                tool_name: "shell".to_string(),
                content: "<original 50KB stale output>".to_string(),
                is_error: false,
                status: librefang_types::tool::ToolExecutionStatus::Completed,
                approval_request_id: None,
            }]),
            pinned: false,
            timestamp: None,
        });
        // Two recent turns to push the stale one across the boundary.
        for i in 0..3 {
            msgs.push(assistant_msg(&format!("recent {i}")));
            msgs.push(tool_result_msg("recent_tool", &format!("recent {i}")));
        }
        let driver: Arc<dyn LlmDriver> = Arc::new(OkDriver("compact summary".to_string()));
        let (out, _result) = fold_stale_tool_results(msgs, 2, 1, "test-model", None, driver).await;

        // The stale ToolResult must still exist with its tool_use_id intact,
        // only `content` rewritten — without this the assistant's ToolUse{
        // id="tid_stale"} would be orphaned and the next provider call would
        // 400 with "tool_use ids must match tool_result tool_use_ids".
        let stale_tr_block = out.iter().find_map(|m| match &m.content {
            MessageContent::Blocks(blocks) => blocks.iter().find_map(|b| match b {
                ContentBlock::ToolResult {
                    tool_use_id,
                    content,
                    ..
                } if tool_use_id == "tid_stale" => Some(content.clone()),
                _ => None,
            }),
            _ => None,
        });
        let content = stale_tr_block.expect(
            "stale ToolResult{tool_use_id=tid_stale} must survive fold to keep \
             tool_use/tool_result pairing intact",
        );
        assert!(
            content.starts_with(FOLD_PREFIX),
            "stale ToolResult.content must be rewritten to a fold stub, got: {content:?}"
        );
    }

    /// Cost amortiser (#3): when `min_batch_size > stale_count` the fold
    /// pass exits early without calling the aux-LLM.
    #[tokio::test]
    async fn min_batch_size_skips_fold_when_below_threshold() {
        // 3 stale turns; min_batch_size=4 → no fold, no aux call.
        let messages = build_history(11);
        let stale = collect_stale_indices(&messages, 8);
        assert_eq!(stale.len(), 3, "test setup: expected 3 stale tool results");

        let driver: Arc<dyn LlmDriver> = Arc::new(OkDriver(
            "should never be called — fold should skip below batch threshold".to_string(),
        ));
        let (out, result) =
            fold_stale_tool_results(messages.clone(), 8, 4, "test-model", None, driver).await;

        assert_eq!(
            result.groups_folded, 0,
            "fold must skip when stale_count < min_batch_size"
        );
        assert_eq!(result.messages_replaced, 0);
        // History returns unchanged.
        assert_eq!(out.len(), messages.len());
    }

    #[test]
    fn collect_stale_indices_boundary() {
        // Build: user, asst, tool, asst, tool, asst, tool  (3 asst turns)
        // fold_after=2 → turn at index 0 is stale; tool-result at index 2 is stale.
        let msgs = vec![
            user_msg("q"),
            assistant_msg("a0"),
            tool_result_msg("t", "r0"),
            assistant_msg("a1"),
            tool_result_msg("t", "r1"),
            assistant_msg("a2"),
            tool_result_msg("t", "r2"),
        ];
        let stale = collect_stale_indices(&msgs, 2);
        // Tool-result at index 2 should be stale (before the last 2 assistant turns).
        assert!(stale.contains(&2), "index 2 should be stale, got {stale:?}");
        // Tool-result at index 4 should NOT be stale (within the last 2 turns).
        assert!(
            !stale.contains(&4),
            "index 4 should not be stale, got {stale:?}"
        );
    }

    #[test]
    fn group_consecutive_basic() {
        let g = group_consecutive(vec![0, 1, 2, 5, 6, 9]);
        assert_eq!(g, vec![vec![0, 1, 2], vec![5, 6], vec![9]]);
    }

    #[tokio::test]
    async fn already_folded_stub_not_re_folded() {
        // Build history where one "stale" message is already a fold stub
        // (Blocks with a ToolResult.content prefixed by FOLD_PREFIX). It
        // must never be re-selected for folding regardless of turn count.
        let mut msgs = vec![user_msg("initial question")];
        msgs.push(assistant_msg("prior assistant turn"));
        msgs.push(Message {
            role: Role::User,
            content: MessageContent::Blocks(vec![ContentBlock::ToolResult {
                tool_use_id: "tid_prior".to_string(),
                tool_name: "shell".to_string(),
                content: format!("{FOLD_PREFIX} prior summary text"),
                is_error: false,
                status: librefang_types::tool::ToolExecutionStatus::Completed,
                approval_request_id: None,
            }]),
            pinned: false,
            timestamp: None,
        });
        // Add enough assistant turns to push the stub into the stale window.
        for i in 0..10 {
            msgs.push(assistant_msg(&format!("response {i}")));
            msgs.push(tool_result_msg("shell", &format!("output {i}")));
        }
        let driver: Arc<dyn LlmDriver> = Arc::new(OkDriver("new summary".to_string()));
        let (out, result) = fold_stale_tool_results(msgs, 8, 1, "test-model", None, driver).await;

        // The existing fold stub must still be present in the output unchanged.
        let prior_stub_present = out.iter().any(|m| match &m.content {
            MessageContent::Blocks(blocks) => blocks.iter().any(|b| match b {
                ContentBlock::ToolResult { content, .. } => {
                    content.starts_with(FOLD_PREFIX) && content.contains("prior summary text")
                }
                _ => false,
            }),
            _ => false,
        });
        assert!(
            prior_stub_present,
            "pre-existing fold stub must survive unchanged: {result:?}"
        );
    }

    #[test]
    fn pinned_messages_not_folded() {
        let mut msgs = vec![user_msg("q"), assistant_msg("a0")];
        // Pinned tool result — must not be folded.
        let mut pinned = tool_result_msg("t", "important pinned result");
        pinned.pinned = true;
        msgs.push(pinned);
        for _ in 0..8 {
            msgs.push(assistant_msg("ax"));
            msgs.push(tool_result_msg("t", "recent"));
        }
        let stale = collect_stale_indices(&msgs, 8);
        // The pinned message at index 2 must not appear.
        assert!(
            !stale.contains(&2),
            "pinned message should never be stale: {stale:?}"
        );
    }
}
