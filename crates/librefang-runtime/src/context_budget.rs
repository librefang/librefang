//! Dynamic context budget for tool result truncation.
//!
//! Replaces the hardcoded MAX_TOOL_RESULT_CHARS with a two-layer system:
//! - Layer 1: Per-result cap based on context window size (30% of window)
//! - Layer 2: Context guard that scans all tool results before LLM calls
//!   and compacts oldest results when total exceeds 75% headroom.

use librefang_types::message::{ContentBlock, Message, MessageContent};
use librefang_types::tool::ToolDefinition;
use tracing::debug;

/// Budget parameters derived from the model's context window.
#[derive(Debug, Clone)]
pub struct ContextBudget {
    /// Total context window size in tokens.
    pub context_window_tokens: usize,
    /// Estimated characters per token for tool results (denser content).
    pub tool_chars_per_token: f64,
    /// Estimated characters per token for general content.
    pub general_chars_per_token: f64,
}

impl ContextBudget {
    /// Create a new budget from a context window size.
    pub fn new(context_window_tokens: usize) -> Self {
        Self {
            context_window_tokens,
            tool_chars_per_token: 2.0,
            general_chars_per_token: 4.0,
        }
    }

    /// Per-result character cap: 30% of context window converted to chars.
    pub fn per_result_cap(&self) -> usize {
        let tokens_for_tool = (self.context_window_tokens as f64 * 0.30) as usize;
        (tokens_for_tool as f64 * self.tool_chars_per_token) as usize
    }

    /// Single result absolute max: 50% of context window.
    pub fn single_result_max(&self) -> usize {
        let tokens = (self.context_window_tokens as f64 * 0.50) as usize;
        (tokens as f64 * self.tool_chars_per_token) as usize
    }

    /// Total tool result headroom: 75% of context window in chars.
    pub fn total_tool_headroom_chars(&self) -> usize {
        let tokens = (self.context_window_tokens as f64 * 0.75) as usize;
        (tokens as f64 * self.tool_chars_per_token) as usize
    }
}

impl Default for ContextBudget {
    fn default() -> Self {
        Self::new(200_000)
    }
}

/// Truncation marker inserted between head and tail portions.
const TRUNCATION_MARKER: &str = "\n\n[...truncated middle...]\n\n";

fn byte_index_at_char(content: &str, char_index: usize) -> usize {
    content
        .char_indices()
        .nth(char_index)
        .map_or(content.len(), |(index, _)| index)
}

fn hard_char_prefix(content: &str, max_chars: usize) -> String {
    content[..byte_index_at_char(content, max_chars)].to_string()
}

fn enforce_char_cap(candidate: String, content: &str, max_chars: usize) -> String {
    if candidate.chars().count() <= max_chars {
        candidate
    } else {
        hard_char_prefix(content, max_chars)
    }
}

/// Layer 1: Truncate a single tool result dynamically based on context budget.
///
/// Uses a head+tail strategy: keeps the first 60% and last 40% of the budget,
/// with a truncation marker in between. This preserves both the beginning
/// (context, parameters) and the end (errors, final output) of tool results.
pub fn truncate_tool_result_dynamic(content: &str, budget: &ContextBudget) -> String {
    let cap = budget.per_result_cap(); // character budget
    let char_count = content.chars().count();
    if char_count <= cap {
        return content.to_string();
    }

    let marker_len = TRUNCATION_MARKER.chars().count();
    let summary = format!(
        "\n\n[TRUNCATED: result was {char_count} chars (budget: 30% of {}K context window)]",
        budget.context_window_tokens / 1000
    );
    let summary_len = summary.chars().count();
    if cap <= marker_len + summary_len {
        return hard_char_prefix(content, cap);
    }
    let usable = cap - marker_len - summary_len;
    let head_chars = (usable as f64 * 0.6) as usize;
    let tail_chars = usable.saturating_sub(head_chars);

    let head_end = find_safe_break_before(content, byte_index_at_char(content, head_chars));
    let tail_start = find_safe_break_after(
        content,
        byte_index_at_char(content, char_count.saturating_sub(tail_chars)),
    );

    // Only use head+tail if there's actually a gap to skip
    if tail_start <= head_end {
        // Not enough content to skip; just keep the head
        let break_point = find_safe_break_before(
            content,
            byte_index_at_char(content, cap.saturating_sub(summary_len)),
        );
        let truncated = format!("{}{summary}", &content[..break_point]);
        return enforce_char_cap(truncated, content, cap);
    }

    enforce_char_cap(
        format!(
            "{}{}{}{summary}",
            &content[..head_end],
            TRUNCATION_MARKER,
            &content[tail_start..]
        ),
        content,
        cap,
    )
}

/// Layer 2: Context guard — scan all tool_result blocks in the message history.
///
/// If total tool result content exceeds 75% of the context headroom,
/// compact oldest results first. Returns the number of results compacted.
pub fn apply_context_guard(
    messages: &mut [Message],
    budget: &ContextBudget,
    _tools: &[ToolDefinition],
) -> usize {
    let headroom = budget.total_tool_headroom_chars();
    let single_max = budget.single_result_max();

    // Collect all tool result sizes and locations
    struct ToolResultLoc {
        msg_idx: usize,
        block_idx: usize,
        char_len: usize,
        is_delegation: bool,
        compacted: bool,
    }

    let mut locations: Vec<ToolResultLoc> = Vec::new();
    let mut total_chars: usize = 0;

    for (msg_idx, msg) in messages.iter().enumerate() {
        if let MessageContent::Blocks(blocks) = &msg.content {
            for (block_idx, block) in blocks.iter().enumerate() {
                if let ContentBlock::ToolResult {
                    content, tool_name, ..
                } = block
                {
                    let len = content.chars().count();
                    total_chars += len;
                    locations.push(ToolResultLoc {
                        msg_idx,
                        block_idx,
                        char_len: len,
                        is_delegation: tool_name == "agent_send",
                        compacted: false,
                    });
                }
            }
        }
    }

    if total_chars <= headroom {
        return 0;
    }

    debug!(
        total_chars,
        headroom,
        results = locations.len(),
        "Context guard: tool results exceed headroom, compacting oldest"
    );

    // First pass: cap any single result that exceeds 50% of context
    for loc in &mut locations {
        if loc.char_len > single_max {
            // Bounds check: indices may be stale if messages were modified concurrently
            if loc.msg_idx >= messages.len() {
                continue;
            }
            if let MessageContent::Blocks(blocks) = &mut messages[loc.msg_idx].content {
                if loc.block_idx >= blocks.len() {
                    continue;
                }
                if let ContentBlock::ToolResult { content, .. } = &mut blocks[loc.block_idx] {
                    let old_char_len = content.chars().count();
                    *content = truncate_to(content, single_max);
                    let new_char_len = content.chars().count();
                    total_chars = total_chars.saturating_sub(old_char_len) + new_char_len;
                    loc.char_len = new_char_len; // update so second pass uses correct value
                    loc.compacted = true;
                }
            }
        }
    }

    // Second pass: compact oldest results until under headroom
    // (locations are already in chronological order)
    const COMPACT_DEFAULT: usize = 2_000;
    const COMPACT_DELEGATION: usize = 8_000; // agent_send results need more context (#4135)
    for loc in &mut locations {
        if total_chars <= headroom {
            break;
        }
        let compact_target = if loc.is_delegation {
            COMPACT_DELEGATION
        } else {
            COMPACT_DEFAULT
        };
        if loc.char_len <= compact_target {
            continue;
        }
        if loc.msg_idx >= messages.len() {
            continue;
        }
        if let MessageContent::Blocks(blocks) = &mut messages[loc.msg_idx].content {
            if loc.block_idx >= blocks.len() {
                continue;
            }
            if let ContentBlock::ToolResult { content, .. } = &mut blocks[loc.block_idx] {
                if content.chars().count() > compact_target {
                    let old_char_len = content.chars().count();
                    *content = truncate_to(content, compact_target);
                    let new_char_len = content.chars().count();
                    total_chars = total_chars.saturating_sub(old_char_len) + new_char_len;
                    loc.char_len = new_char_len;
                    loc.compacted = true;
                }
            }
        }
    }

    // Retention floors are preferences, not permission to exceed the model's
    // total tool-result budget. If they were insufficient, remove exactly the
    // remaining excess from the oldest results.
    for loc in &mut locations {
        if total_chars <= headroom {
            break;
        }
        if loc.msg_idx >= messages.len() {
            continue;
        }
        if let MessageContent::Blocks(blocks) = &mut messages[loc.msg_idx].content {
            if loc.block_idx >= blocks.len() {
                continue;
            }
            if let ContentBlock::ToolResult { content, .. } = &mut blocks[loc.block_idx] {
                let old_char_len = content.chars().count();
                let excess = total_chars.saturating_sub(headroom);
                let target = old_char_len.saturating_sub(excess);
                if target < old_char_len {
                    *content = truncate_to(content, target);
                    let new_char_len = content.chars().count();
                    total_chars = total_chars.saturating_sub(old_char_len) + new_char_len;
                    loc.char_len = new_char_len;
                    loc.compacted = true;
                }
            }
        }
    }

    debug_assert!(total_chars <= headroom);
    locations
        .iter()
        .filter(|location| location.compacted)
        .count()
}

/// Find a char-boundary-safe break point at or before `pos`, preferring newlines.
fn find_safe_break_before(content: &str, pos: usize) -> usize {
    let mut safe = pos.min(content.len());
    while safe > 0 && !content.is_char_boundary(safe) {
        safe -= 1;
    }
    // Try to break at a newline within the last 200 chars
    let search_start = safe.saturating_sub(200);
    let mut ss = search_start;
    while ss > 0 && !content.is_char_boundary(ss) {
        ss -= 1;
    }
    content[ss..safe]
        .rfind('\n')
        .map(|p| ss + p)
        .unwrap_or(safe)
}

/// Find a char-boundary-safe break point at or after `pos`, preferring newlines.
fn find_safe_break_after(content: &str, pos: usize) -> usize {
    let mut safe = pos.min(content.len());
    while safe < content.len() && !content.is_char_boundary(safe) {
        safe += 1;
    }
    // Try to advance to next newline within 200 chars
    let search_end = (safe + 200).min(content.len());
    let mut se = search_end;
    while se < content.len() && !content.is_char_boundary(se) {
        se += 1;
    }
    content[safe..se]
        .find('\n')
        .map(|p| safe + p + 1) // start after the newline
        .unwrap_or(safe)
}

/// Truncate content to `max_chars` using head+tail strategy with a marker.
///
/// Keeps the first 60% and last 40% of the budget to preserve both
/// the beginning (context) and end (errors, final output) of content.
fn truncate_to(content: &str, max_chars: usize) -> String {
    let char_count = content.chars().count();
    if char_count <= max_chars {
        return content.to_string();
    }

    let marker = TRUNCATION_MARKER;
    let marker_chars = marker.chars().count();
    let summary = format!("\n\n[COMPACTED: {char_count} chars by context guard]");
    let summary_chars = summary.chars().count();
    if max_chars <= marker_chars + summary_chars {
        return hard_char_prefix(content, max_chars);
    }
    let usable = max_chars - marker_chars - summary_chars;
    let head_chars = (usable as f64 * 0.6) as usize;
    let tail_chars = usable.saturating_sub(head_chars);

    let head_end = find_safe_break_before(content, byte_index_at_char(content, head_chars));
    let tail_start = find_safe_break_after(
        content,
        byte_index_at_char(content, char_count.saturating_sub(tail_chars)),
    );

    // Only use head+tail if there's a meaningful gap to skip
    if tail_start <= head_end {
        let break_point = find_safe_break_before(
            content,
            byte_index_at_char(content, max_chars.saturating_sub(summary_chars)),
        );
        return enforce_char_cap(
            format!("{}{summary}", &content[..break_point]),
            content,
            max_chars,
        );
    }

    enforce_char_cap(
        format!(
            "{}{}{}{summary}",
            &content[..head_end],
            marker,
            &content[tail_start..]
        ),
        content,
        max_chars,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_budget_defaults() {
        let budget = ContextBudget::default();
        assert_eq!(budget.context_window_tokens, 200_000);
        // 30% of 200K * 2.0 chars/token = 120K chars
        assert_eq!(budget.per_result_cap(), 120_000);
    }

    #[test]
    fn test_small_model_budget() {
        let budget = ContextBudget::new(8_000);
        // 30% of 8K * 2.0 = 4800 chars
        assert_eq!(budget.per_result_cap(), 4_800);
    }

    #[test]
    fn test_truncate_within_limit() {
        let budget = ContextBudget::default();
        let short = "Hello world";
        assert_eq!(truncate_tool_result_dynamic(short, &budget), short);
    }

    #[test]
    fn tiny_caps_never_expand_truncated_content() {
        let budget = ContextBudget::new(10);
        let content = "abcdefghij";
        let result = truncate_tool_result_dynamic(content, &budget);
        assert_eq!(result.chars().count(), budget.per_result_cap());
        assert_eq!(result, "abcdef");

        let compacted = truncate_to(content, 3);
        assert_eq!(compacted, "abc");
    }

    #[test]
    fn test_tiny_truncate_respects_character_cap() {
        let budget = ContextBudget::new(100); // very small: cap = 60 chars
        let content =
            "line1\nline2\nline3\nline4\nline5\nline6\nline7\nline8\nline9\nline10\nline11\nline12";
        let result = truncate_tool_result_dynamic(content, &budget);
        assert_eq!(result.chars().count(), budget.per_result_cap());
        assert!(content.starts_with(&result));
    }

    #[test]
    fn test_context_guard_no_compaction_needed() {
        let budget = ContextBudget::default();
        let mut messages = vec![Message::user("hello")];
        let compacted = apply_context_guard(&mut messages, &budget, &[]);
        assert_eq!(compacted, 0);
    }

    #[test]
    fn test_context_guard_compacts_oldest() {
        // Use tiny budget to trigger compaction
        let budget = ContextBudget::new(100); // headroom = 75% of 100 * 2.0 = 150 chars
        let big_result = "x".repeat(500);
        let mut messages = vec![
            Message {
                role: librefang_types::message::Role::User,
                content: MessageContent::Blocks(vec![ContentBlock::ToolResult {
                    tool_use_id: "t1".to_string(),
                    tool_name: String::new(),
                    content: big_result.clone(),
                    is_error: false,
                    status: librefang_types::tool::ToolExecutionStatus::default(),
                    approval_request_id: None,
                }]),
                pinned: false,
                timestamp: None,
            },
            Message {
                role: librefang_types::message::Role::User,
                content: MessageContent::Blocks(vec![ContentBlock::ToolResult {
                    tool_use_id: "t2".to_string(),
                    tool_name: String::new(),
                    content: big_result,
                    is_error: false,
                    status: librefang_types::tool::ToolExecutionStatus::default(),
                    approval_request_id: None,
                }]),
                pinned: false,
                timestamp: None,
            },
        ];

        let compacted = apply_context_guard(&mut messages, &budget, &[]);
        assert!(compacted > 0);

        // Verify results were actually truncated
        if let MessageContent::Blocks(blocks) = &messages[0].content {
            if let ContentBlock::ToolResult { content, .. } = &blocks[0] {
                assert!(content.len() < 500);
            }
        }
    }

    #[test]
    fn context_guard_compacts_below_floors_to_meet_headroom() {
        let budget = ContextBudget::new(100);
        let mut messages: Vec<Message> = (0..3)
            .map(|index| Message {
                role: librefang_types::message::Role::User,
                content: MessageContent::Blocks(vec![ContentBlock::ToolResult {
                    tool_use_id: format!("t{index}"),
                    tool_name: "shell_exec".to_string(),
                    content: "x".repeat(100),
                    is_error: false,
                    status: librefang_types::tool::ToolExecutionStatus::default(),
                    approval_request_id: None,
                }]),
                pinned: false,
                timestamp: None,
            })
            .collect();

        let compacted = apply_context_guard(&mut messages, &budget, &[]);
        let remaining: usize = messages
            .iter()
            .filter_map(|message| match &message.content {
                MessageContent::Blocks(blocks) => match &blocks[0] {
                    ContentBlock::ToolResult { content, .. } => Some(content.chars().count()),
                    _ => None,
                },
                _ => None,
            })
            .sum();

        assert_eq!(compacted, 2);
        assert!(remaining <= budget.total_tool_headroom_chars());
    }

    #[test]
    fn test_truncate_tool_result_multibyte_chinese() {
        // Tiny budget: cap = 30% of 100 * 2.0 = 60 characters
        let budget = ContextBudget::new(100);
        // Each Chinese char is 3 bytes in UTF-8; 100 chars = 300 bytes
        let content: String = "\u{4f60}\u{597d}\u{4e16}\u{754c}".repeat(25);
        assert_eq!(content.len(), 300);
        // Must not panic on multi-byte content
        let result = truncate_tool_result_dynamic(&content, &budget);
        assert_eq!(result.chars().count(), budget.per_result_cap());
        // The visible portion must be valid UTF-8 (implicit: no panic)
        assert!(result.is_char_boundary(0));
    }

    #[test]
    fn test_truncate_to_multibyte_emoji() {
        // Each emoji is 4 bytes; 200 emojis = 800 bytes
        let content: String = "\u{1f600}".repeat(200);
        let result = truncate_to(&content, 100);
        assert_eq!(result.chars().count(), 100);
        // Must not panic and must produce valid UTF-8
        assert!(result.is_char_boundary(0));
    }

    #[test]
    fn test_context_guard_multibyte_tool_results() {
        let budget = ContextBudget::new(100);
        // Chinese text: 500 chars * 3 bytes = 1500 bytes
        let big_chinese: String = "\u{4e2d}\u{6587}\u{6d4b}\u{8bd5}\u{6570}\u{636e}".repeat(83);
        let mut messages = vec![Message {
            role: librefang_types::message::Role::User,
            content: MessageContent::Blocks(vec![ContentBlock::ToolResult {
                tool_use_id: "t1".to_string(),
                tool_name: String::new(),
                content: big_chinese,
                is_error: false,
                status: librefang_types::tool::ToolExecutionStatus::default(),
                approval_request_id: None,
            }]),
            pinned: false,
            timestamp: None,
        }];
        // Must not panic on multi-byte content
        let compacted = apply_context_guard(&mut messages, &budget, &[]);
        assert!(compacted > 0);
    }

    #[test]
    fn test_truncate_to_head_tail_strategy() {
        // Content with distinct head and tail sections
        let head = "HEAD-SECTION ".repeat(100); // ~1300 chars
        let middle = "MIDDLE ".repeat(500); // ~3500 chars
        let tail = "TAIL-SECTION ".repeat(100); // ~1300 chars
        let content = format!("{head}{middle}{tail}");

        // Truncate to a budget that forces truncation but allows head+tail
        let result = truncate_to(&content, 2000);
        assert!(
            result.contains("[...truncated middle...]"),
            "Should contain head+tail marker"
        );
        assert!(
            result.contains("HEAD-SECTION"),
            "Should preserve head content"
        );
        assert!(
            result.contains("TAIL-SECTION"),
            "Should preserve tail content"
        );
        assert!(result.contains("[COMPACTED:"));
    }

    #[test]
    fn test_truncate_tool_result_dynamic_head_tail() {
        let budget = ContextBudget::new(500); // cap = 30% of 500 * 2.0 = 300 chars
                                              // Create content with important error at the end
        let beginning = "Starting process...\n".repeat(20); // ~400 chars
        let ending = "\nERROR: Critical failure at line 42\nStack trace: important details\n";
        let content = format!("{beginning}{ending}");

        let result = truncate_tool_result_dynamic(&content, &budget);
        assert!(result.contains("[TRUNCATED:"));
        // With head+tail, the ending should be preserved
        if result.contains("[...truncated middle...]") {
            assert!(
                result.contains("ERROR: Critical failure"),
                "Tail should preserve error output, got: {}",
                &result[result.len().saturating_sub(200)..]
            );
        }
    }

    #[test]
    fn test_truncate_to_small_content_no_headtail() {
        // When content is only slightly over budget, may not use head+tail
        let content = "x".repeat(150);
        let result = truncate_to(&content, 100);
        assert_eq!(result.chars().count(), 100);
        assert!(result.contains("[COMPACTED:"));
    }

    #[test]
    fn test_context_guard_delegation_higher_floor() {
        // Budget sized so first pass (single_max) doesn't trigger but second pass does:
        // single_max = 50% * 6000 * 2.0 = 6000 chars (> 5K, no first-pass truncation)
        // headroom  = 75% * 6000 * 2.0 = 9000 chars (< 10K total, triggers second pass)
        let budget = ContextBudget::new(6000);
        let big_result = "x".repeat(5000); // 5K chars — above 2K default, below 8K delegation floor

        let mut messages = vec![
            // Normal tool result — should be compacted to 2K
            Message {
                role: librefang_types::message::Role::User,
                content: MessageContent::Blocks(vec![ContentBlock::ToolResult {
                    tool_use_id: "t1".to_string(),
                    tool_name: "shell_exec".to_string(),
                    content: big_result.clone(),
                    is_error: false,
                    status: librefang_types::tool::ToolExecutionStatus::default(),
                    approval_request_id: None,
                }]),
                pinned: false,
                timestamp: None,
            },
            // agent_send result — should NOT be compacted (5K < 8K floor)
            Message {
                role: librefang_types::message::Role::User,
                content: MessageContent::Blocks(vec![ContentBlock::ToolResult {
                    tool_use_id: "t2".to_string(),
                    tool_name: "agent_send".to_string(),
                    content: big_result,
                    is_error: false,
                    status: librefang_types::tool::ToolExecutionStatus::default(),
                    approval_request_id: None,
                }]),
                pinned: false,
                timestamp: None,
            },
        ];

        apply_context_guard(&mut messages, &budget, &[]);

        // Normal result should be compacted (well below 5K)
        if let MessageContent::Blocks(blocks) = &messages[0].content {
            if let ContentBlock::ToolResult { content, .. } = &blocks[0] {
                assert!(
                    content.chars().count() < 3000,
                    "Normal tool result should be compacted to ~2K, got {}",
                    content.chars().count()
                );
            }
        }

        // agent_send result should be preserved (5K is under 8K floor)
        if let MessageContent::Blocks(blocks) = &messages[1].content {
            if let ContentBlock::ToolResult { content, .. } = &blocks[0] {
                assert!(
                    content.chars().count() >= 4000,
                    "agent_send result should be preserved at 5K (under 8K floor), got {}",
                    content.chars().count()
                );
            }
        }
    }
}
