//! Tool-result history fold — mechanism 3 of #3347.
//!
//! After `history_fold_after_turns` assistant turns have elapsed, tool-result
//! messages from those older turns are "stale" and contribute noise to the
//! context window without materially helping the agent.  This module folds
//! them into a single compact summary message so the LLM sees the history
//! without the raw payload bulk.
//!
//! # Algorithm
//!
//! 1. Walk `messages` and count assistant turns.  Any tool-result user message
//!    that was answered before the most recent `history_fold_after_turns`
//!    assistant turns is marked stale.
//! 2. Group consecutive stale tool-result messages and ask the aux-LLM (or the
//!    primary driver when no aux chain is configured) to produce a 1–2 sentence
//!    summary per group.
//! 3. Replace each group with a single synthetic `[user]` message:
//!    `[history-fold: <count> tool result(s) from turns <range> ago: <summary>]`
//! 4. Pinned messages are never folded (they are protected work product).
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

/// Result of a single fold pass.
#[derive(Debug, Default)]
pub struct FoldResult {
    /// Number of groups that were folded.
    pub groups_folded: usize,
    /// Total tool-result messages that were replaced.
    pub messages_replaced: usize,
    /// Whether the LLM summarisation was available (false = fallback used).
    pub used_fallback: bool,
}

/// Fold stale tool-result messages in `messages`.
///
/// `fold_after_turns` — fold tool results older than this many assistant turns.
/// `model` — model slug forwarded to the summariser.
/// `aux_client` — optional aux-LLM client; when `None`, fallback text is used.
/// `driver` — primary driver (used when aux chain resolves to primary).
///
/// Returns the (possibly modified) message list and a [`FoldResult`] summary.
pub async fn fold_stale_tool_results(
    messages: Vec<Message>,
    fold_after_turns: u32,
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

    debug!(
        stale_count = stale_indices.len(),
        fold_after_turns, "history_fold: folding stale tool-result messages"
    );

    // Resolve the summarisation driver (aux preferred, primary as fallback).
    let summary_driver = aux_client
        .map(|c| c.driver_for(AuxTask::Fold))
        .unwrap_or_else(|| Arc::clone(&driver));

    // Group consecutive stale indices so we produce one stub per contiguous run.
    let groups = group_consecutive(stale_indices);

    let mut result = FoldResult::default();
    let mut used_fallback = false;

    // Build the new message list, replacing each stale group.
    let mut out: Vec<Message> = Vec::with_capacity(messages.len());
    let mut skip_set: std::collections::HashSet<usize> = std::collections::HashSet::new();
    for g in &groups {
        for &i in g {
            skip_set.insert(i);
        }
    }

    // Map from first-index-in-group → summary stub (computed below).
    let mut group_stubs: std::collections::HashMap<usize, Message> =
        std::collections::HashMap::new();

    for (g_idx, group) in groups.iter().enumerate() {
        let count = group.len();
        let group_msgs: Vec<&Message> = group.iter().map(|&i| &messages[i]).collect();
        let summary = summarise_group(group_msgs.as_slice(), model, &*summary_driver, g_idx).await;
        match summary {
            Ok(text) => {
                info!(
                    count,
                    "history_fold: summarised group of {count} tool-result(s)"
                );
                let stub = Message::user(format!("{FOLD_PREFIX} {count} tool result(s): {text}"));
                group_stubs.insert(group[0], stub);
            }
            Err(e) => {
                warn!(count, error = %e, "history_fold: summarisation failed, using fallback stub");
                used_fallback = true;
                let stub = Message::user(format!(
                    "{FOLD_PREFIX} {count} tool result(s) [summarisation unavailable]"
                ));
                group_stubs.insert(group[0], stub);
            }
        }
        result.groups_folded += 1;
        result.messages_replaced += count;
    }

    // Reconstruct the message list.
    for (i, msg) in messages.into_iter().enumerate() {
        if skip_set.contains(&i) {
            // First index in a group → emit the stub; rest → skip.
            if let Some(stub) = group_stubs.remove(&i) {
                out.push(stub);
            }
            // else: non-first member of the group, already represented by the stub.
        } else {
            out.push(msg);
        }
    }

    result.used_fallback = used_fallback;
    (out, result)
}

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
    messages
        .iter()
        .enumerate()
        .filter(|(i, msg)| {
            *i < boundary_idx
                && !msg.pinned
                && msg.role == Role::User
                && is_tool_result_message(msg)
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
                        let preview: String = content.chars().take(500).collect();
                        let has_more = content.len() > 500;
                        text.push_str(&format!("- {tool_name}: {preview}"));
                        if has_more {
                            text.push_str(" ...[truncated]");
                        }
                        text.push('\n');
                    }
                }
            }
            MessageContent::Text(t) => {
                let preview: String = t.chars().take(500).collect();
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
                model: None,
            })
        }

        async fn stream(
            &self,
            _req: CompletionRequest,
            _tx: tokio::sync::mpsc::Sender<crate::llm_driver::StreamEvent>,
        ) -> Result<(), LlmError> {
            Ok(())
        }
    }

    /// Driver that always returns an error.
    struct FailDriver;

    #[async_trait::async_trait]
    impl LlmDriver for FailDriver {
        async fn complete(&self, _req: CompletionRequest) -> Result<CompletionResponse, LlmError> {
            Err(LlmError::Other("simulated failure".to_string()))
        }

        async fn stream(
            &self,
            _req: CompletionRequest,
            _tx: tokio::sync::mpsc::Sender<crate::llm_driver::StreamEvent>,
        ) -> Result<(), LlmError> {
            Ok(())
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

    #[tokio::test]
    async fn fold_after_8_folds_old_turns() {
        // 10 turns total; fold_after=8 → turns 0 and 1 are stale (oldest 2).
        let messages = build_history(10);
        let driver: Arc<dyn LlmDriver> = Arc::new(OkDriver("nice summary".to_string()));

        let (out, result) = fold_stale_tool_results(messages, 8, "test-model", None, driver).await;

        assert!(
            result.groups_folded >= 1,
            "expected at least one group folded"
        );
        assert!(
            result.messages_replaced >= 1,
            "expected at least one message replaced"
        );
        // Folded stubs should start with FOLD_PREFIX.
        let fold_msgs: Vec<_> = out
            .iter()
            .filter(|m| matches!(&m.content, MessageContent::Text(t) if t.starts_with(FOLD_PREFIX)))
            .collect();
        assert!(
            !fold_msgs.is_empty(),
            "expected fold stub message in output"
        );
    }

    #[tokio::test]
    async fn no_fold_when_not_enough_turns() {
        // Only 5 turns; fold_after=8 → nothing should be folded.
        let messages = build_history(5);
        let original_len = messages.len();
        let driver: Arc<dyn LlmDriver> = Arc::new(OkDriver("summary".to_string()));

        let (out, result) = fold_stale_tool_results(messages, 8, "test-model", None, driver).await;

        assert_eq!(result.groups_folded, 0);
        assert_eq!(result.messages_replaced, 0);
        assert_eq!(out.len(), original_len, "history unchanged");
    }

    #[tokio::test]
    async fn fallback_stub_when_llm_unavailable() {
        // 10 turns, fold_after=8, but the LLM driver always fails.
        let messages = build_history(10);
        let driver: Arc<dyn LlmDriver> = Arc::new(FailDriver);

        let (out, result) = fold_stale_tool_results(messages, 8, "test-model", None, driver).await;

        // Should still fold (with fallback stubs).
        assert!(result.used_fallback, "expected fallback to be used");
        assert!(result.groups_folded >= 1);
        let fold_msgs: Vec<_> = out
            .iter()
            .filter(|m| {
                matches!(&m.content, MessageContent::Text(t)
                    if t.starts_with(FOLD_PREFIX) && t.contains("summarisation unavailable"))
            })
            .collect();
        assert!(!fold_msgs.is_empty(), "expected fallback stub in output");
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
