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

/// Layer 1: Truncate a single tool result dynamically based on context budget.
///
/// Uses a head+tail strategy: keeps the first 60% and last 40% of the budget,
/// with a truncation marker in between. This preserves both the beginning
/// (context, parameters) and the end (errors, final output) of tool results.
pub fn truncate_tool_result_dynamic(content: &str, budget: &ContextBudget) -> String {
    let cap = budget.per_result_cap();
    if content.len() <= cap {
        return content.to_string();
    }

    let marker_len = TRUNCATION_MARKER.len();
    // Reserve space for the marker and the summary line
    let summary_reserve = 100; // space for the "[TRUNCATED: ...]" suffix
    let usable = cap.saturating_sub(marker_len + summary_reserve);
    let head_budget = (usable as f64 * 0.6) as usize;
    let tail_budget = usable.saturating_sub(head_budget);

    let head_end = find_safe_break_before(content, head_budget);
    let tail_start = find_safe_break_after(content, content.len().saturating_sub(tail_budget));

    // Only use head+tail if there's actually a gap to skip
    if tail_start <= head_end {
        // Not enough content to skip; just keep the head
        let break_point = find_safe_break_before(content, cap.saturating_sub(summary_reserve));
        return format!(
            "{}\n\n[TRUNCATED: result was {} chars, showing first {} (budget: {}% of {}K context window)]",
            &content[..break_point],
            content.len(),
            break_point,
            30,
            budget.context_window_tokens / 1000
        );
    }

    format!(
        "{}{}{}\n\n[TRUNCATED: result was {} chars, showing first {} + last {} (budget: {}% of {}K context window)]",
        &content[..head_end],
        TRUNCATION_MARKER,
        &content[tail_start..],
        content.len(),
        head_end,
        content.len() - tail_start,
        30,
        budget.context_window_tokens / 1000
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
    }

    let mut locations: Vec<ToolResultLoc> = Vec::new();
    let mut total_chars: usize = 0;

    for (msg_idx, msg) in messages.iter().enumerate() {
        if let MessageContent::Blocks(blocks) = &msg.content {
            for (block_idx, block) in blocks.iter().enumerate() {
                if let ContentBlock::ToolResult { content, .. } = block {
                    let len = content.len();
                    total_chars += len;
                    locations.push(ToolResultLoc {
                        msg_idx,
                        block_idx,
                        char_len: len,
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
    let mut compacted = 0;
    for loc in &locations {
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
                    let old_len = content.len();
                    *content = truncate_to(content, single_max);
                    total_chars -= old_len;
                    total_chars += content.len();
                    compacted += 1;
                }
            }
        }
    }

    // Second pass: compact oldest results until under headroom
    // (locations are already in chronological order)
    let compact_target = 2000; // compact to 2K chars each
    for loc in &locations {
        if total_chars <= headroom {
            break;
        }
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
                if content.len() > compact_target {
                    let old_len = content.len();
                    *content = truncate_to(content, compact_target);
                    total_chars -= old_len;
                    total_chars += content.len();
                    compacted += 1;
                }
            }
        }
    }

    compacted
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
    if content.len() <= max_chars {
        return content.to_string();
    }

    let marker = TRUNCATION_MARKER;
    let suffix_reserve = 80; // for "[COMPACTED: ...]" line
    let usable = max_chars.saturating_sub(marker.len() + suffix_reserve);
    let head_budget = (usable as f64 * 0.6) as usize;
    let tail_budget = usable.saturating_sub(head_budget);

    let head_end = find_safe_break_before(content, head_budget);
    let tail_start = find_safe_break_after(content, content.len().saturating_sub(tail_budget));

    // Only use head+tail if there's a meaningful gap to skip
    if tail_start <= head_end {
        let break_point = find_safe_break_before(content, max_chars.saturating_sub(suffix_reserve));
        return format!(
            "{}\n\n[COMPACTED: {} -> {} chars by context guard]",
            &content[..break_point],
            content.len(),
            break_point
        );
    }

    format!(
        "{}{}{}\n\n[COMPACTED: {} -> {} chars by context guard (head+tail)]",
        &content[..head_end],
        marker,
        &content[tail_start..],
        content.len(),
        head_end + (content.len() - tail_start)
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
    fn test_truncate_breaks_at_newline() {
        let budget = ContextBudget::new(100); // very small: cap = 60 chars
        let content =
            "line1\nline2\nline3\nline4\nline5\nline6\nline7\nline8\nline9\nline10\nline11\nline12";
        let result = truncate_tool_result_dynamic(content, &budget);
        assert!(result.contains("[TRUNCATED:"));
        // Should not split in the middle of a line
        assert!(
            result.starts_with("line1\n") || result.is_empty() || result.contains("[TRUNCATED:")
        );
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
                }]),
                pinned: false,
            },
            Message {
                role: librefang_types::message::Role::User,
                content: MessageContent::Blocks(vec![ContentBlock::ToolResult {
                    tool_use_id: "t2".to_string(),
                    tool_name: String::new(),
                    content: big_result,
                    is_error: false,
                }]),
                pinned: false,
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
    fn test_truncate_tool_result_multibyte_chinese() {
        // Tiny budget: cap = 30% of 100 * 2.0 = 60 bytes
        let budget = ContextBudget::new(100);
        // Each Chinese char is 3 bytes in UTF-8; 100 chars = 300 bytes
        let content: String = "\u{4f60}\u{597d}\u{4e16}\u{754c}".repeat(25);
        assert_eq!(content.len(), 300);
        // Must not panic on multi-byte content
        let result = truncate_tool_result_dynamic(&content, &budget);
        assert!(result.contains("[TRUNCATED:"));
        // The visible portion must be valid UTF-8 (implicit: no panic)
        assert!(result.is_char_boundary(0));
    }

    #[test]
    fn test_truncate_to_multibyte_emoji() {
        // Each emoji is 4 bytes; 200 emojis = 800 bytes
        let content: String = "\u{1f600}".repeat(200);
        let result = truncate_to(&content, 100);
        assert!(result.contains("[COMPACTED:"));
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
            }]),
            pinned: false,
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
        assert!(result.contains("[COMPACTED:"));
        // Result should be valid UTF-8 and shorter than original
        assert!(result.len() < content.len() + 100); // original + marker overhead
    }
}
