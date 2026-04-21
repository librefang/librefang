//! Automatic context compression for long-running conversations.
//!
//! When estimated token usage exceeds a configurable threshold (default 80%
//! of the context window), this module uses the LLM to summarise the middle
//! portion of the conversation history, replacing old turns with a compact
//! handoff summary while preserving the system prompt and the most recent
//! messages verbatim.
//!
//! # Algorithm
//!
//! 1. Estimate total token usage with the CJK-aware heuristic from `compactor`.
//! 2. If usage ≥ `threshold_ratio * context_window`, trigger compression.
//! 3. Protect the first `protect_head` messages (system prompt + opening turns).
//! 4. Protect the last `keep_recent` messages (tail — most current context).
//! 5. Summarise the "middle" slice via the LLM using `compactor::compact_session`.
//! 6. Replace the middle with a single synthetic `[user]` summary message.
//! 7. Repeat up to `max_iterations` times if still over the threshold.
//!
//! # Design Notes
//!
//! - Zero new external crate dependencies — reuses `compactor`, `llm_driver`, and
//!   `librefang_types` primitives already present in this crate.
//! - The compressor is intentionally stateless per-call: the agent loop passes
//!   in a fresh `Vec<Message>` each iteration and gets back a compressed copy.
//!   State (e.g. "previous summary") is tracked via the injected summary message
//!   that persists in the message list across turns.
//! - System-prompt messages (`Role::System`) in the head are preserved unchanged.

use crate::compactor::{self, CompactionConfig};
use crate::llm_driver::LlmDriver;
use librefang_memory::session::Session;
use librefang_types::agent::{AgentId, SessionId};
use librefang_types::message::{ContentBlock, Message, MessageContent, Role};
use librefang_types::tool::ToolDefinition;
use std::sync::Arc;
use tracing::{debug, info, warn};

/// Prefix injected into compression summary messages so downstream code and
/// the LLM itself can recognise that earlier turns were compacted.
const SUMMARY_PREFIX: &str = "[CONTEXT COMPRESSION SUMMARY] Earlier conversation turns have \
    been summarised to preserve context space. The state described below reflects work \
    already completed — do NOT repeat it. Continue from where the conversation left off, \
    responding only to the most recent user message that appears AFTER this summary.";

/// Configuration for the context compressor.
#[derive(Debug, Clone)]
pub struct CompressionConfig {
    /// Trigger compression when estimated tokens exceed this fraction of the
    /// context window (0.0–1.0). Default: 0.80.
    pub threshold_ratio: f64,
    /// Number of messages at the beginning of the history to leave untouched
    /// (typically includes the system prompt and the first user/assistant exchange).
    /// Default: 3.
    pub protect_head: usize,
    /// Number of most-recent messages to preserve verbatim (the "tail").
    /// Default: 10.
    pub keep_recent: usize,
    /// Maximum compression iterations per agent-loop turn.
    /// Each iteration may trigger another LLM summarisation call if the context
    /// is still over budget after the first pass. Default: 3.
    pub max_iterations: u32,
    /// Maximum tokens the LLM may use for generating the summary.
    /// Default: 1024.
    pub max_summary_tokens: u32,
}

impl Default for CompressionConfig {
    fn default() -> Self {
        Self {
            threshold_ratio: 0.80,
            protect_head: 3,
            keep_recent: 10,
            max_iterations: 3,
            max_summary_tokens: 1024,
        }
    }
}

/// Metadata recorded for each compression pass.
#[derive(Debug, Clone)]
pub struct CompressionEvent {
    /// Estimated token count before compression.
    pub before_tokens: usize,
    /// Estimated token count after compression.
    pub after_tokens: usize,
    /// Number of messages before compression.
    pub before_count: usize,
    /// Number of messages after compression.
    pub after_count: usize,
    /// Compression iteration index (0-based).
    pub iteration: u32,
    /// Whether the LLM summarisation was available (false = fallback text used).
    pub used_fallback: bool,
}

/// Context compressor — wraps `compactor::compact_session` with automatic
/// threshold detection and iterative refinement.
#[derive(Debug, Clone)]
pub struct ContextCompressor {
    config: CompressionConfig,
}

impl ContextCompressor {
    /// Create a new compressor with the given configuration.
    pub fn new(config: CompressionConfig) -> Self {
        Self { config }
    }

    /// Create a compressor with default settings.
    pub fn with_defaults() -> Self {
        Self::new(CompressionConfig::default())
    }

    /// Check whether the message history currently exceeds the compression
    /// threshold.
    ///
    /// Uses the same CJK-aware token estimator as `compactor` so the trigger
    /// condition is consistent with the rest of the context budget logic.
    pub fn should_compress(
        &self,
        messages: &[Message],
        system_prompt: &str,
        tools: &[ToolDefinition],
        context_window: usize,
    ) -> bool {
        let estimated = compactor::estimate_token_count(messages, Some(system_prompt), Some(tools));
        let threshold = (context_window as f64 * self.config.threshold_ratio) as usize;
        let over = estimated >= threshold;
        if over {
            debug!(
                estimated_tokens = estimated,
                threshold, context_window, "Context compression threshold exceeded"
            );
        }
        over
    }

    /// Compress `messages` if needed, returning the (possibly compressed)
    /// message list along with any compression events that occurred.
    ///
    /// The caller should replace its working message list with the returned
    /// one. Session state is NOT persisted here — that remains the agent
    /// loop's responsibility.
    ///
    /// # Parameters
    ///
    /// - `messages` — current LLM working copy of the conversation
    /// - `system_prompt` — system prompt (used for token estimation only; not
    ///   modified)
    /// - `tools` — available tool definitions (used for token estimation)
    /// - `context_window` — model context window in tokens
    /// - `model` — LLM model string forwarded to the summariser
    /// - `driver` — LLM driver used to generate the summary
    pub async fn compress_if_needed(
        &self,
        messages: Vec<Message>,
        system_prompt: &str,
        tools: &[ToolDefinition],
        context_window: usize,
        model: &str,
        driver: Arc<dyn LlmDriver>,
    ) -> (Vec<Message>, Vec<CompressionEvent>) {
        let mut current = messages;
        let mut events = Vec::new();

        for iteration in 0..self.config.max_iterations {
            if !self.should_compress(&current, system_prompt, tools, context_window) {
                break;
            }

            let before_count = current.len();
            let before_tokens =
                compactor::estimate_token_count(&current, Some(system_prompt), Some(tools));

            // Need at least head + 1 (middle) + tail messages to compress anything.
            let min_for_compress = self.config.protect_head + self.config.keep_recent + 1;
            if before_count <= min_for_compress {
                debug!(
                    before_count,
                    min_for_compress, "Too few messages to compress — skipping"
                );
                break;
            }

            match self.compress_once(&current, model, driver.clone()).await {
                Ok((compressed, used_fallback)) => {
                    let after_count = compressed.len();
                    let after_tokens = compactor::estimate_token_count(
                        &compressed,
                        Some(system_prompt),
                        Some(tools),
                    );

                    if after_tokens >= before_tokens {
                        // The summary is larger than (or equal to) the original
                        // middle — no net reduction achieved.  Stop iterating to
                        // avoid growing the context on subsequent passes.
                        warn!(
                            iteration,
                            before_tokens,
                            after_tokens,
                            "Context compression summary expanded context — stopping iteration"
                        );
                        break;
                    }

                    info!(
                        iteration,
                        before_count,
                        after_count,
                        before_tokens,
                        after_tokens,
                        used_fallback,
                        "Context compression complete"
                    );

                    events.push(CompressionEvent {
                        before_tokens,
                        after_tokens,
                        before_count,
                        after_count,
                        iteration,
                        used_fallback,
                    });

                    current = compressed;
                }
                Err(e) => {
                    warn!(iteration, error = %e, "Context compression failed — keeping original messages");
                    break;
                }
            }
        }

        (current, events)
    }

    /// Perform a single compression pass over `messages`.
    ///
    /// Splits the message list into:
    /// - **head** — first `protect_head` messages (preserved as-is)
    /// - **middle** — messages to summarise
    /// - **tail** — last `keep_recent` messages (preserved as-is)
    ///
    /// Calls `compactor::compact_session` on the middle slice, then
    /// reassembles: `[head] + [summary_message] + [tail]`.
    async fn compress_once(
        &self,
        messages: &[Message],
        model: &str,
        driver: Arc<dyn LlmDriver>,
    ) -> Result<(Vec<Message>, bool), String> {
        let n = messages.len();
        // Extend the head boundary to avoid splitting a ToolUse/ToolResult pair:
        // if the last message in protect_head is an assistant message that contains
        // ToolUse blocks, the corresponding user ToolResult delivery falls into
        // `middle` and would get summarised away.  Pull it into `head` so the pair
        // travels together.
        let mut head_end = self.config.protect_head.min(n);
        if head_end < n {
            let last_head = &messages[head_end - 1];
            if last_head.role == Role::Assistant && msg_has_tool_use(last_head) {
                // Include the next message (the ToolResult delivery) in the head.
                head_end = (head_end + 1).min(n);
            }
        }

        let nominal_tail_start = if n > self.config.keep_recent {
            n - self.config.keep_recent
        } else {
            n
        };

        // Walk the tail boundary forward until it lands on a clean turn boundary
        // so we never split a ToolUse/ToolResult pair.  A clean boundary is a
        // message that is NOT a ToolResult delivery (those must stay immediately
        // after their matching assistant ToolUse).
        let tail_start = {
            let mut ts = nominal_tail_start;
            while ts < n && msg_is_tool_result_delivery(&messages[ts]) {
                ts += 1;
            }
            ts
        };

        // Ensure there is actually a middle section to compress.
        if head_end >= tail_start {
            return Err("No middle section to compress".to_string());
        }

        let head = &messages[..head_end];
        let middle = &messages[head_end..tail_start];
        let tail = &messages[tail_start..];

        debug!(
            head = head.len(),
            middle = middle.len(),
            tail = tail.len(),
            "Compressing middle section"
        );

        // Build a temporary session containing only the middle messages.
        let temp_session = Session {
            id: SessionId::new(),
            agent_id: AgentId::new(),
            messages: middle.to_vec(),
            context_window_tokens: 0,
            label: None,
        };

        let compaction_config = CompactionConfig {
            threshold: 0,   // always compact (we've already decided to compress)
            keep_recent: 0, // include all middle messages in the summary
            max_summary_tokens: self.config.max_summary_tokens,
            ..CompactionConfig::default()
        };

        let result =
            compactor::compact_session(driver, model, &temp_session, &compaction_config).await?;

        // Choose the summary message role to maintain User/Assistant alternation.
        //
        // The only case where an Assistant summary produces clean alternation at
        // BOTH boundaries is when head ends with User AND tail starts with User.
        // In all other cases a User summary is chosen:
        //  - head ends Assistant → User summary avoids back-to-back Assistant.
        //  - head ends User, tail starts Assistant → User summary creates a
        //    consecutive-User pair that session_repair will merge; inserting
        //    an Assistant summary before an Assistant tail would be equally bad.
        //
        // The tail boundary was already adjusted above to skip ToolResult-only
        // deliveries, so an Assistant summary placed before a User tail is safe
        // (no risk of orphaning a ToolUse/ToolResult pair).
        let last_head_role = head.last().map(|m| m.role).unwrap_or(Role::User);
        let first_tail_role = tail.first().map(|m| m.role).unwrap_or(Role::User);

        let summary_role = if last_head_role == Role::User && first_tail_role == Role::User {
            Role::Assistant
        } else {
            Role::User
        };

        let summary_content = format!("{}\n\n{}", SUMMARY_PREFIX, result.summary);
        // Propagate pinning: if any compressed message was pinned, the replacement
        // summary must also be pinned so it survives future compression rounds.
        let any_middle_pinned = middle.iter().any(|m| m.pinned);
        let summary_msg = Message {
            role: summary_role,
            content: MessageContent::Text(summary_content),
            pinned: any_middle_pinned,
        };

        let mut compressed = Vec::with_capacity(head.len() + 1 + tail.len());
        compressed.extend_from_slice(head);
        compressed.push(summary_msg);
        compressed.extend_from_slice(tail);

        Ok((compressed, result.used_fallback))
    }
}

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

/// Returns `true` when `msg` is a ToolResult delivery — a User-role message
/// that contains AT LEAST ONE `ToolResult` block.
///
/// Using `any` (rather than `all`) is intentional: `finalize_tool_use_results`
/// can append guidance `Text` blocks to the same user message for denied calls,
/// producing a mixed `ToolResult + Text` message.  Such mixed messages are still
/// boundary-sensitive and must remain immediately after their matching assistant
/// `ToolUse` message; the tail must not start there.
fn msg_is_tool_result_delivery(msg: &Message) -> bool {
    if msg.role != Role::User {
        return false;
    }
    match &msg.content {
        MessageContent::Blocks(blocks) => blocks
            .iter()
            .any(|b| matches!(b, ContentBlock::ToolResult { .. })),
        _ => false,
    }
}

/// Returns `true` when `msg` is an Assistant-role message that contains at
/// least one `ToolUse` block.  Used to detect the head boundary case where
/// extending `protect_head` is required to keep the ToolUse/ToolResult pair
/// together.
fn msg_has_tool_use(msg: &Message) -> bool {
    match &msg.content {
        MessageContent::Blocks(blocks) => blocks
            .iter()
            .any(|b| matches!(b, ContentBlock::ToolUse { .. })),
        _ => false,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm_driver::{CompletionResponse, LlmDriver, LlmError};
    use async_trait::async_trait;
    use librefang_types::message::{ContentBlock, StopReason, TokenUsage};

    struct EchoDriver;

    #[async_trait]
    impl LlmDriver for EchoDriver {
        async fn complete(
            &self,
            _req: crate::llm_driver::CompletionRequest,
        ) -> Result<CompletionResponse, LlmError> {
            Ok(CompletionResponse {
                content: vec![ContentBlock::Text {
                    text: "Summary of earlier conversation turns.".to_string(),
                    provider_metadata: None,
                }],
                stop_reason: StopReason::EndTurn,
                tool_calls: vec![],
                usage: TokenUsage {
                    input_tokens: 100,
                    output_tokens: 50,
                    ..Default::default()
                },
            })
        }
    }

    fn make_messages(n: usize) -> Vec<Message> {
        (0..n)
            .map(|i| {
                if i % 2 == 0 {
                    Message::user(format!("User message {i}: {}", "x".repeat(200)))
                } else {
                    Message::assistant(format!("Assistant reply {i}: {}", "y".repeat(200)))
                }
            })
            .collect()
    }

    #[test]
    fn test_should_compress_below_threshold() {
        let compressor = ContextCompressor::with_defaults();
        let messages = make_messages(5);
        // Large context window — should not trigger
        assert!(!compressor.should_compress(&messages, "system", &[], 200_000));
    }

    #[test]
    fn test_should_compress_above_threshold() {
        let compressor = ContextCompressor::new(CompressionConfig {
            threshold_ratio: 0.001, // very low threshold to force trigger
            ..CompressionConfig::default()
        });
        let messages = make_messages(10);
        assert!(compressor.should_compress(&messages, "system", &[], 200_000));
    }

    #[tokio::test]
    async fn test_compress_if_needed_no_compression() {
        let compressor = ContextCompressor::with_defaults();
        let messages = make_messages(5);
        let original_len = messages.len();
        let (result, events) = compressor
            .compress_if_needed(
                messages,
                "system",
                &[],
                200_000,
                "test",
                Arc::new(EchoDriver),
            )
            .await;
        // Should not compress with large context window
        assert_eq!(result.len(), original_len);
        assert!(events.is_empty());
    }

    #[tokio::test]
    async fn test_compress_if_needed_triggers_compression() {
        let compressor = ContextCompressor::new(CompressionConfig {
            threshold_ratio: 0.001, // force trigger
            protect_head: 2,
            keep_recent: 2,
            max_iterations: 1,
            max_summary_tokens: 256,
        });
        let messages = make_messages(20);
        let original_len = messages.len();
        let (result, events) = compressor
            .compress_if_needed(
                messages,
                "system",
                &[],
                200_000,
                "test",
                Arc::new(EchoDriver),
            )
            .await;
        // Should have compressed: head(2) + summary(1) + tail(2) = 5
        assert!(
            result.len() < original_len,
            "Should have fewer messages after compression"
        );
        assert_eq!(result.len(), 5, "head(2) + summary(1) + tail(2)");
        assert!(!events.is_empty(), "Should record compression events");
        assert_eq!(events[0].iteration, 0);
        assert_eq!(events[0].before_count, original_len);
    }

    #[tokio::test]
    async fn test_compress_preserves_head_and_tail() {
        let compressor = ContextCompressor::new(CompressionConfig {
            threshold_ratio: 0.001,
            protect_head: 2,
            keep_recent: 3,
            max_iterations: 1,
            max_summary_tokens: 256,
        });
        let messages = make_messages(15);
        let head_content: Vec<String> = messages[..2]
            .iter()
            .map(|m| m.content.text_content())
            .collect();
        let tail_content: Vec<String> = messages[12..]
            .iter()
            .map(|m| m.content.text_content())
            .collect();

        let (result, _events) = compressor
            .compress_if_needed(
                messages,
                "system",
                &[],
                200_000,
                "test",
                Arc::new(EchoDriver),
            )
            .await;

        // Head messages preserved at start
        let result_head: Vec<String> = result[..2]
            .iter()
            .map(|m| m.content.text_content())
            .collect();
        assert_eq!(
            result_head, head_content,
            "Head messages should be preserved"
        );

        // Tail messages preserved at end
        let result_tail: Vec<String> = result[result.len() - 3..]
            .iter()
            .map(|m| m.content.text_content())
            .collect();
        assert_eq!(
            result_tail, tail_content,
            "Tail messages should be preserved"
        );
    }

    #[tokio::test]
    async fn test_compress_once_inserts_summary_marker() {
        let compressor = ContextCompressor::new(CompressionConfig {
            threshold_ratio: 0.001,
            protect_head: 1,
            keep_recent: 1,
            max_iterations: 1,
            max_summary_tokens: 256,
        });
        let messages = make_messages(10);
        let (compressed, _fallback) = compressor
            .compress_once(&messages, "test", Arc::new(EchoDriver))
            .await
            .expect("compress_once should succeed");

        // The summary message should contain the SUMMARY_PREFIX marker
        let has_marker = compressed.iter().any(|m| {
            m.content
                .text_content()
                .contains("CONTEXT COMPRESSION SUMMARY")
        });
        assert!(
            has_marker,
            "Compressed messages should contain the summary marker"
        );
    }

    #[tokio::test]
    async fn test_too_few_messages_skips_compression() {
        let compressor = ContextCompressor::new(CompressionConfig {
            threshold_ratio: 0.001,
            protect_head: 3,
            keep_recent: 10,
            max_iterations: 3,
            max_summary_tokens: 256,
        });
        // Only 5 messages — less than protect_head(3) + keep_recent(10) + 1 = 14
        let messages = make_messages(5);
        let original_len = messages.len();
        let (result, events) = compressor
            .compress_if_needed(
                messages,
                "system",
                &[],
                200_000,
                "test",
                Arc::new(EchoDriver),
            )
            .await;
        assert_eq!(
            result.len(),
            original_len,
            "Should not compress when too few messages"
        );
        assert!(events.is_empty(), "Should have no compression events");
    }
}
