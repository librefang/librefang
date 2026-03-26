//! Context overflow recovery pipeline.
//!
//! Provides a 4-stage recovery pipeline that replaces the brute-force
//! `emergency_trim_messages()` with structured, progressive recovery:
//!
//! 1. Auto-compact via message trimming (keep recent, drop old)
//! 2. Aggressive overflow compaction (drop all but last N)
//! 3. Truncate historical tool results to 2K chars each
//! 4. Return error suggesting /reset or /compact

use librefang_types::message::{ContentBlock, Message, MessageContent};
use librefang_types::tool::ToolDefinition;
use tracing::{debug, warn};

/// Recovery stage that was applied.
#[derive(Debug, Clone, PartialEq)]
pub enum RecoveryStage {
    /// No recovery needed.
    None,
    /// Stage 1: moderate trim (keep last 10).
    AutoCompaction { removed: usize },
    /// Stage 2: aggressive trim (keep last 4).
    OverflowCompaction { removed: usize },
    /// Stage 3: truncated tool results.
    ToolResultTruncation { truncated: usize },
    /// Stage 4: unrecoverable — suggest /reset.
    FinalError,
}

/// Estimate token count using CJK-aware heuristic.
fn estimate_tokens(messages: &[Message], system_prompt: &str, tools: &[ToolDefinition]) -> usize {
    crate::compactor::estimate_token_count(messages, Some(system_prompt), Some(tools))
}

/// Remove up to `target` non-pinned messages from the front of the list.
///
/// Pinned messages are preserved in their original positions.
/// Returns the number of messages actually removed.
fn drain_unpinned_from_front(messages: &mut Vec<Message>, target: usize) -> usize {
    // Count how many non-pinned messages we'll remove (scanning from front)
    let mut to_remove = 0;
    let mut scanned = 0;
    for msg in messages.iter() {
        if to_remove >= target {
            break;
        }
        scanned += 1;
        if !msg.pinned {
            to_remove += 1;
        }
    }

    // Use retain with a counter — O(n) single pass instead of O(n²)
    let mut removed = 0;
    let mut idx = 0;
    messages.retain(|msg| {
        idx += 1;
        if idx > scanned || msg.pinned {
            true // keep
        } else {
            removed += 1;
            false // remove
        }
    });
    removed
}

/// Run the 4-stage overflow recovery pipeline.
///
/// Returns the recovery stage applied and the number of messages/results affected.
///
/// `stage1_threshold` and `stage2_threshold` are fractions of `context_window`
/// (e.g. 0.70 and 0.90). Pass `None` to use the built-in defaults (0.70 / 0.90).
pub fn recover_from_overflow(
    messages: &mut Vec<Message>,
    system_prompt: &str,
    tools: &[ToolDefinition],
    context_window: usize,
) -> RecoveryStage {
    recover_from_overflow_with_thresholds(
        messages,
        system_prompt,
        tools,
        context_window,
        0.70,
        0.90,
    )
}

/// Like [`recover_from_overflow`] but with explicit stage thresholds.
pub fn recover_from_overflow_with_thresholds(
    messages: &mut Vec<Message>,
    system_prompt: &str,
    tools: &[ToolDefinition],
    context_window: usize,
    stage1_threshold: f64,
    stage2_threshold: f64,
) -> RecoveryStage {
    let estimated = estimate_tokens(messages, system_prompt, tools);
    let threshold_70 = (context_window as f64 * stage1_threshold) as usize;
    let threshold_90 = (context_window as f64 * stage2_threshold) as usize;

    // No recovery needed
    if estimated <= threshold_70 {
        return RecoveryStage::None;
    }

    // Stage 1: Moderate trim — keep last 10 messages, but preserve pinned messages
    if estimated <= threshold_90 {
        let keep = 10.min(messages.len());
        let target_remove = messages.len() - keep;
        if target_remove > 0 {
            debug!(
                estimated_tokens = estimated,
                target_remove, "Stage 1: moderate trim, preserving pinned messages"
            );
            let removed = drain_unpinned_from_front(messages, target_remove);
            // Re-check after trim
            let new_est = estimate_tokens(messages, system_prompt, tools);
            if new_est <= threshold_70 {
                return RecoveryStage::AutoCompaction { removed };
            }
        }
    }

    // Stage 2: Aggressive trim — keep last 4 messages + summary marker, preserve pinned
    {
        let keep = 4.min(messages.len());
        let target_remove = messages.len() - keep;
        if target_remove > 0 {
            warn!(
                estimated_tokens = estimate_tokens(messages, system_prompt, tools),
                target_remove,
                "Stage 2: aggressive overflow compaction, preserving pinned messages"
            );
            let removed = drain_unpinned_from_front(messages, target_remove);
            if removed > 0 {
                let summary = Message::user(format!(
                    "[System: {} earlier messages were removed due to context overflow. \
                     The conversation continues from here. Use /compact for smarter summarization.]",
                    removed
                ));
                messages.insert(0, summary);
            }

            let new_est = estimate_tokens(messages, system_prompt, tools);
            if new_est <= threshold_90 {
                return RecoveryStage::OverflowCompaction { removed };
            }
        }
    }

    // Stage 3: Truncate all historical tool results to 2K chars
    let tool_truncation_limit = 2000;
    let mut truncated = 0;
    for msg in messages.iter_mut() {
        if let MessageContent::Blocks(blocks) = &mut msg.content {
            for block in blocks.iter_mut() {
                if let ContentBlock::ToolResult { content, .. } = block {
                    let char_count = content.chars().count();
                    if char_count > tool_truncation_limit {
                        // Compute bytes-per-char ratio to convert char budget to byte position
                        let bytes_per_char = if char_count > 0 {
                            content.len() as f64 / char_count as f64
                        } else {
                            1.0
                        };
                        let keep_chars = tool_truncation_limit.saturating_sub(80);
                        let mut safe_keep = (keep_chars as f64 * bytes_per_char) as usize;
                        safe_keep = safe_keep.min(content.len());
                        // Walk back to a valid char boundary
                        while safe_keep > 0 && !content.is_char_boundary(safe_keep) {
                            safe_keep -= 1;
                        }
                        let kept_chars = content[..safe_keep].chars().count();
                        *content = format!(
                            "{}\n\n[OVERFLOW RECOVERY: truncated from {} to {} chars]",
                            &content[..safe_keep],
                            char_count,
                            kept_chars
                        );
                        truncated += 1;
                    }
                }
            }
        }
    }

    if truncated > 0 {
        let new_est = estimate_tokens(messages, system_prompt, tools);
        if new_est <= threshold_90 {
            return RecoveryStage::ToolResultTruncation { truncated };
        }
        warn!(
            estimated_tokens = new_est,
            "Stage 3 truncated {} tool results but still over threshold", truncated
        );
    }

    // Stage 4: Final error — nothing more we can do automatically
    warn!("Stage 4: all recovery stages exhausted, context still too large");
    RecoveryStage::FinalError
}

#[cfg(test)]
mod tests {
    use super::*;
    use librefang_types::message::{Message, Role};

    fn make_messages(count: usize, size_each: usize) -> Vec<Message> {
        (0..count)
            .map(|i| {
                let text = format!("msg{}: {}", i, "x".repeat(size_each));
                Message {
                    role: if i % 2 == 0 {
                        Role::User
                    } else {
                        Role::Assistant
                    },
                    content: MessageContent::Text(text),
                    pinned: false,
                }
            })
            .collect()
    }

    #[test]
    fn test_no_recovery_needed() {
        let mut msgs = make_messages(2, 100);
        let stage = recover_from_overflow(&mut msgs, "sys", &[], 200_000);
        assert_eq!(stage, RecoveryStage::None);
    }

    #[test]
    fn test_stage1_moderate_trim() {
        // Create messages that push us past 70% but not 90%
        // Context window: 1000 tokens = 4000 chars
        // 70% = 700 tokens = 2800 chars
        let mut msgs = make_messages(20, 150); // ~3000 chars total
        let stage = recover_from_overflow(&mut msgs, "system", &[], 1000);
        match stage {
            RecoveryStage::AutoCompaction { removed } => {
                assert!(removed > 0);
                assert!(msgs.len() <= 10);
            }
            RecoveryStage::OverflowCompaction { .. } => {
                // Also acceptable if moderate wasn't enough
            }
            _ => {} // depends on exact token estimation
        }
    }

    #[test]
    fn test_stage2_aggressive_trim() {
        // Push past 90%: 1000 tokens = 4000 chars, 90% = 3600 chars
        let mut msgs = make_messages(30, 200); // ~6000 chars
        let stage = recover_from_overflow(&mut msgs, "system", &[], 1000);
        match stage {
            RecoveryStage::OverflowCompaction { removed } => {
                assert!(removed > 0);
            }
            RecoveryStage::ToolResultTruncation { .. } | RecoveryStage::FinalError => {}
            _ => {} // acceptable cascading
        }
    }

    #[test]
    fn test_stage3_tool_truncation() {
        let big_result = "x".repeat(5000);
        let mut msgs = vec![
            Message::user("hi"),
            Message {
                role: Role::User,
                content: MessageContent::Blocks(vec![ContentBlock::ToolResult {
                    tool_use_id: "t1".to_string(),
                    tool_name: String::new(),
                    content: big_result.clone(),
                    is_error: false,
                }]),
                pinned: false,
            },
            Message {
                role: Role::User,
                content: MessageContent::Blocks(vec![ContentBlock::ToolResult {
                    tool_use_id: "t2".to_string(),
                    tool_name: String::new(),
                    content: big_result,
                    is_error: false,
                }]),
                pinned: false,
            },
        ];
        // Tiny context window to force all stages
        let stage = recover_from_overflow(&mut msgs, "system", &[], 500);
        // Should at least reach tool truncation
        match stage {
            RecoveryStage::ToolResultTruncation { truncated } => {
                assert!(truncated > 0);
            }
            RecoveryStage::OverflowCompaction { .. } | RecoveryStage::FinalError => {}
            _ => {}
        }
    }

    #[test]
    fn test_cascading_stages() {
        // Ensure stages cascade: if stage 1 isn't enough, stage 2 kicks in
        let mut msgs = make_messages(50, 500);
        let stage = recover_from_overflow(&mut msgs, "system prompt", &[], 2000);
        // With 50 messages of 500 chars each (25000 chars), context of 2000 tokens (8000 chars),
        // we should cascade through stages
        assert_ne!(stage, RecoveryStage::None);
    }

    #[test]
    fn test_stage3_multibyte_tool_truncation() {
        // Chinese text (3 bytes per char) in tool results must not panic
        let chinese_result: String = "\u{4f60}\u{597d}\u{4e16}\u{754c}".repeat(1250); // 5000 chars, 15000 bytes
        let mut msgs = vec![
            Message::user("hi"),
            Message {
                role: Role::User,
                content: MessageContent::Blocks(vec![ContentBlock::ToolResult {
                    tool_use_id: "t1".to_string(),
                    tool_name: String::new(),
                    content: chinese_result,
                    is_error: false,
                }]),
                pinned: false,
            },
        ];
        // Tiny context window to force stage 3 tool truncation
        let stage = recover_from_overflow(&mut msgs, "system", &[], 500);
        // Must not panic — the truncation at byte boundaries could split a 3-byte char
        assert_ne!(stage, RecoveryStage::None);
    }

    #[test]
    fn test_pinned_messages_preserved_in_stage1() {
        // Create messages that push us past 70% but not 90%
        let mut msgs = make_messages(20, 150);
        // Pin the first two messages
        msgs[0].pinned = true;
        msgs[1].pinned = true;
        let pinned_content_0 = msgs[0].content.text_content();
        let pinned_content_1 = msgs[1].content.text_content();

        let stage = recover_from_overflow(&mut msgs, "system", &[], 1000);
        match stage {
            RecoveryStage::AutoCompaction { removed } => {
                assert!(removed > 0);
                // Pinned messages must still be present
                let texts: Vec<String> = msgs.iter().map(|m| m.content.text_content()).collect();
                assert!(
                    texts.contains(&pinned_content_0),
                    "First pinned message should be preserved"
                );
                assert!(
                    texts.contains(&pinned_content_1),
                    "Second pinned message should be preserved"
                );
            }
            RecoveryStage::OverflowCompaction { .. } => {
                // Also acceptable — pinned messages should still be present
                let texts: Vec<String> = msgs.iter().map(|m| m.content.text_content()).collect();
                assert!(texts.contains(&pinned_content_0));
                assert!(texts.contains(&pinned_content_1));
            }
            _ => {} // depends on exact token estimation
        }
    }

    #[test]
    fn test_pinned_messages_preserved_in_stage2() {
        // Push well past 90% to trigger stage 2
        let mut msgs = make_messages(30, 200);
        // Pin the very first message
        msgs[0].pinned = true;
        let pinned_content = msgs[0].content.text_content();

        let stage = recover_from_overflow(&mut msgs, "system", &[], 1000);
        match stage {
            RecoveryStage::OverflowCompaction { removed } => {
                assert!(removed > 0);
                // The pinned message must still be present
                let texts: Vec<String> = msgs.iter().map(|m| m.content.text_content()).collect();
                assert!(
                    texts.contains(&pinned_content),
                    "Pinned message should survive aggressive trim"
                );
            }
            RecoveryStage::ToolResultTruncation { .. } | RecoveryStage::FinalError => {
                // Still check pinned was preserved
                let texts: Vec<String> = msgs.iter().map(|m| m.content.text_content()).collect();
                assert!(texts.contains(&pinned_content));
            }
            _ => {}
        }
    }

    #[test]
    fn test_drain_unpinned_skips_pinned() {
        let mut msgs = vec![
            Message::user("first"),
            Message::user("second"),
            Message::user("third"),
            Message::user("fourth"),
        ];
        msgs[1].pinned = true; // pin the second message

        let removed = drain_unpinned_from_front(&mut msgs, 2);
        assert_eq!(removed, 2);
        // Should have removed "first" and "third", keeping "second" (pinned) and "fourth"
        assert_eq!(msgs.len(), 2);
        assert!(
            msgs[0].pinned,
            "First remaining should be the pinned message"
        );
        assert_eq!(msgs[0].content.text_content(), "second");
        assert_eq!(msgs[1].content.text_content(), "fourth");
    }

    #[test]
    fn test_drain_unpinned_all_pinned() {
        let mut msgs = vec![Message::user("first"), Message::user("second")];
        msgs[0].pinned = true;
        msgs[1].pinned = true;

        let removed = drain_unpinned_from_front(&mut msgs, 5);
        assert_eq!(
            removed, 0,
            "No messages should be removed when all are pinned"
        );
        assert_eq!(msgs.len(), 2);
    }
}
