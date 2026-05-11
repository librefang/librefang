//! Integration tests for cron session compaction (SummarizeTrim mode, #3693).
//!
//! These tests exercise the kernel-level `try_summarize_trim` logic through
//! the `librefang_runtime::compactor` surface that the kernel delegates to.
//! They cover:
//!
//! - Successful LLM summarization produces `[summary_msg] + tail` output
//!   with `used_fallback = false` (H2 gap 1).
//! - LLM driver failure causes `compact_session` to return `used_fallback =
//!   true`, which the kernel treats as a fallback trigger (H2 gap 2 / M4).
//! - `adjust_split_for_tool_pair` never splits an `Assistant{ToolUse}` /
//!   `User{ToolResult}` pair across the summary / tail boundary (H1).

use async_trait::async_trait;
use librefang_runtime::compactor::{adjust_split_for_tool_pair, compact_session, CompactionConfig};
use librefang_runtime::llm_driver::{CompletionRequest, CompletionResponse, LlmDriver, LlmError};
use librefang_types::agent::{AgentId, SessionId};
use librefang_types::message::{
    ContentBlock, Message, MessageContent, Role, StopReason, TokenUsage,
};
use std::sync::Arc;

// ---------------------------------------------------------------------------
// Test helpers
// ---------------------------------------------------------------------------

fn make_session(
    agent_id: AgentId,
    session_id: SessionId,
    messages: Vec<Message>,
) -> librefang_memory::session::Session {
    librefang_memory::session::Session {
        id: session_id,
        agent_id,
        messages,
        context_window_tokens: 0,
        label: None,
        messages_generation: 0,
        last_repaired_generation: None,
    }
}

/// A driver that always returns a canned summary response.
struct FakeDriver {
    summary: String,
}

#[async_trait]
impl LlmDriver for FakeDriver {
    async fn complete(&self, _req: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        Ok(CompletionResponse {
            content: vec![ContentBlock::Text {
                text: self.summary.clone(),
                provider_metadata: None,
            }],
            stop_reason: StopReason::EndTurn,
            tool_calls: vec![],
            usage: TokenUsage {
                input_tokens: 50,
                output_tokens: 10,
                ..Default::default()
            },
        })
    }
}

/// A driver that always returns an LLM error.
struct FailingDriver;

#[async_trait]
impl LlmDriver for FailingDriver {
    async fn complete(&self, _req: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        Err(LlmError::Http("connection refused".to_string()))
    }
}

// ---------------------------------------------------------------------------
// H2 gap 1 — successful LLM summarization produces summary + tail
// ---------------------------------------------------------------------------

/// `compact_session` with a working driver produces a non-empty, non-fallback
/// summary. This mirrors what `try_summarize_trim` expects before calling
/// `session.set_messages([summary_msg] + kept_tail)`.
#[tokio::test(flavor = "multi_thread")]
async fn summarize_trim_successful_llm_produces_summary_not_fallback() {
    let agent_id = AgentId::new();
    let session_id = SessionId::for_channel(agent_id, "cron");

    let messages: Vec<Message> = (0..12)
        .map(|i| Message::user(format!("cron turn {i}")))
        .collect();

    let keep_recent: usize = 4;
    let tail_start = messages.len().saturating_sub(keep_recent);
    // adjust_split_for_tool_pair on plain user messages should not change the split
    let adjusted = adjust_split_for_tool_pair(&messages, tail_start, keep_recent);
    assert_eq!(
        adjusted, tail_start,
        "plain user messages: no tool pair to protect, split should be unchanged"
    );

    let to_summarize = &messages[..adjusted];
    let kept_tail = messages[adjusted..].to_vec();

    let tmp_session = make_session(agent_id, session_id, to_summarize.to_vec());
    let compact_cfg = CompactionConfig {
        threshold: 0,
        keep_recent: 0,
        ..CompactionConfig::default()
    };
    let driver = Arc::new(FakeDriver {
        summary: "Canned summary of older cron messages.".to_string(),
    });

    let result = compact_session(
        driver,
        "test-model",
        &tmp_session,
        &compact_cfg,
        librefang_types::model_catalog::ReasoningEchoPolicy::None,
    )
    .await
    .expect("compact_session must succeed with FakeDriver");

    assert!(!result.summary.is_empty(), "summary must be non-empty");
    assert!(
        !result.used_fallback,
        "used_fallback must be false with a working driver"
    );

    // Reconstruct output as the kernel does.
    let summary_msg = Message {
        role: Role::Assistant,
        content: MessageContent::Text(format!(
            "[Cron session summary — {} messages compacted]\n\n{}",
            result.compacted_count, result.summary,
        )),
        pinned: false,
        timestamp: None,
    };
    let mut new_messages = vec![summary_msg];
    new_messages.extend(kept_tail);

    assert_eq!(
        new_messages.len(),
        1 + keep_recent,
        "output must be 1 summary + kept_tail"
    );
    assert!(
        new_messages[0]
            .content
            .text_content()
            .contains("[Cron session summary"),
        "first message must be the summary sentinel"
    );
    assert!(
        new_messages[0]
            .content
            .text_content()
            .contains("Canned summary"),
        "summary content must appear in the first message"
    );
    assert_eq!(new_messages[1].content.text_content(), "cron turn 8");
    assert_eq!(new_messages[4].content.text_content(), "cron turn 11");
}

// ---------------------------------------------------------------------------
// H2 gap 2 / M4 — LLM failure triggers used_fallback = true
// ---------------------------------------------------------------------------

/// When the driver fails, `compact_session` returns `Ok(result)` with
/// `used_fallback = true` rather than an `Err`. The kernel checks
/// `!result.used_fallback` (M4 fix) and falls back to plain prune.
#[tokio::test(flavor = "multi_thread")]
async fn summarize_trim_llm_failure_sets_used_fallback_true() {
    let agent_id = AgentId::new();
    let session_id = SessionId::for_channel(agent_id, "cron");

    let messages: Vec<Message> = (0..10)
        .map(|i| Message::user(format!("turn {i}")))
        .collect();

    let keep_recent: usize = 3;
    let tail_start = messages.len().saturating_sub(keep_recent);
    let to_summarize = &messages[..tail_start];

    let tmp_session = make_session(agent_id, session_id, to_summarize.to_vec());
    let compact_cfg = CompactionConfig {
        threshold: 0,
        keep_recent: 0,
        max_retries: 1, // fail fast
        ..CompactionConfig::default()
    };

    let result = compact_session(
        Arc::new(FailingDriver),
        "test-model",
        &tmp_session,
        &compact_cfg,
        librefang_types::model_catalog::ReasoningEchoPolicy::None,
    )
    .await
    .expect("compact_session returns Ok even on LLM failure (stage-3 fallback)");

    // The kernel must detect this and fall back to plain prune.
    assert!(
        result.used_fallback,
        "used_fallback must be true when the LLM driver fails"
    );
    // The fallback summary is non-empty (a placeholder string), so checking
    // only `is_empty()` would incorrectly accept it as a real summary (M4 bug).
    // Verify the kernel guard `!result.used_fallback` correctly rejects it.
    let would_accept_without_m4_fix = !result.summary.is_empty();
    let correctly_rejected = !result.summary.is_empty() && result.used_fallback;
    assert!(
        would_accept_without_m4_fix,
        "fallback summary string is non-empty — old guard would accept it"
    );
    assert!(
        correctly_rejected,
        "M4 guard (!is_empty() && !used_fallback) correctly rejects this result"
    );
}

// ---------------------------------------------------------------------------
// H1 — adjust_split_for_tool_pair never cuts a ToolUse/ToolResult pair
// ---------------------------------------------------------------------------

/// Build a message sequence containing an Assistant{ToolUse} + User{ToolResult}
/// pair and verify that `adjust_split_for_tool_pair` shifts the split so the
/// pair is never separated across the summary / tail boundary.
#[test]
fn adjust_split_does_not_cut_tool_use_tool_result_pair() {
    use librefang_types::message::ContentBlock;

    let tool_use_id = "tool-abc-123".to_string();

    // 6 plain messages, then a ToolUse/ToolResult pair at indices 6 and 7,
    // then 2 more plain messages.
    let mut messages: Vec<Message> = (0..6)
        .map(|i| Message::user(format!("pre-turn {i}")))
        .collect();

    // Assistant message containing a ToolUse block.
    messages.push(Message {
        role: Role::Assistant,
        content: MessageContent::Blocks(vec![ContentBlock::ToolUse {
            id: tool_use_id.clone(),
            name: "shell_exec".to_string(),
            input: serde_json::json!({"cmd": "echo hi"}),
            provider_metadata: None,
        }]),
        pinned: false,
        timestamp: None,
    });

    // User message delivering the matching ToolResult.
    messages.push(Message {
        role: Role::User,
        content: MessageContent::Blocks(vec![ContentBlock::ToolResult {
            tool_use_id: tool_use_id.clone(),
            tool_name: String::new(),
            content: "hi".to_string(),
            is_error: false,
            status: librefang_types::tool::ToolExecutionStatus::default(),
            approval_request_id: None,
        }]),
        pinned: false,
        timestamp: None,
    });

    // Two follow-up messages.
    messages.push(Message::user("after-turn 0".to_string()));
    messages.push(Message::user("after-turn 1".to_string()));

    // Total: 10 messages. keep_recent = 3. Raw split = 10 - 3 = 7.
    // Index 6 = ToolUse (Assistant), index 7 = ToolResult (User).
    // Raw split = 7 puts ToolUse in the head and ToolResult in the tail → pair cut.
    // adjust_split_for_tool_pair must push split to 8 (include ToolResult in head).
    let keep_recent = 3usize;
    let raw_split = messages.len().saturating_sub(keep_recent); // = 7
    assert_eq!(
        raw_split, 7,
        "raw split should land between ToolUse and ToolResult"
    );

    let adjusted = adjust_split_for_tool_pair(&messages, raw_split, keep_recent);
    assert!(
        adjusted >= 8,
        "split must be moved past the ToolResult (index 7) to avoid cutting the pair; \
         got adjusted={adjusted}"
    );

    // Verify the head now includes both ToolUse and ToolResult.
    let head = &messages[..adjusted];
    let has_tool_use = head.iter().any(|m| matches!(&m.content, MessageContent::Blocks(b) if b.iter().any(|bl| matches!(bl, ContentBlock::ToolUse { id, .. } if id == &tool_use_id))));
    let has_tool_result = head.iter().any(|m| matches!(&m.content, MessageContent::Blocks(b) if b.iter().any(|bl| matches!(bl, ContentBlock::ToolResult { tool_use_id: id, .. } if id == &tool_use_id))));
    assert!(has_tool_use, "ToolUse must remain in the head");
    assert!(
        has_tool_result,
        "ToolResult must also be in the head (pair kept together)"
    );
}
