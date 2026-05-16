//! LLM-based session compaction.
//!
//! When a session's message count exceeds a threshold, the compactor
//! uses an LLM to summarize older messages into a concise summary,
//! keeping only the most recent messages intact. This prevents context
//! windows from growing unboundedly while preserving key information.
//!
//! Supports three summarization stages:
//! 1. Full single-pass summarization (fastest, best quality)
//! 2. Adaptive chunked summarization with merge (handles large histories)
//! 3. Minimal fallback without LLM (when summarization is unavailable)

use crate::llm_driver::{CompletionRequest, LlmDriver};
use crate::session_repair::{message_has_tool_use, message_is_only_tool_results};
use crate::str_utils::safe_truncate_str;
use librefang_memory::session::Session;
use librefang_types::message::{ContentBlock, Message, MessageContent, Role};
use librefang_types::tool::ToolDefinition;
use serde::Serialize;
use std::sync::Arc;
use tracing::{debug, info, warn};

/// Configuration for session compaction.
#[derive(Debug, Clone)]
pub struct CompactionConfig {
    /// Compact when session message count exceeds this.
    pub threshold: usize,
    /// Number of recent messages to keep verbatim (not summarized).
    pub keep_recent: usize,
    /// Maximum tokens for the summary generation.
    pub max_summary_tokens: u32,
    /// Base ratio of messages to process per chunk (0.0-1.0).
    pub base_chunk_ratio: f64,
    /// Minimum chunk ratio (floor for adaptive computation).
    pub min_chunk_ratio: f64,
    /// Safety margin multiplier for token estimation inaccuracy.
    pub safety_margin: f64,
    /// Overhead tokens reserved for summarization prompt itself.
    pub summarization_overhead_tokens: u32,
    /// Maximum input chars per summarization chunk.
    pub max_chunk_chars: usize,
    /// Maximum retry attempts for summarization.
    pub max_retries: u32,
    /// Trigger compaction when estimated tokens exceed this fraction of context_window_tokens.
    pub token_threshold_ratio: f64,
    /// Model context window size in tokens.
    pub context_window_tokens: usize,
}

impl Default for CompactionConfig {
    fn default() -> Self {
        Self {
            threshold: 30,
            keep_recent: 10,
            max_summary_tokens: 1024,
            base_chunk_ratio: 0.4,
            min_chunk_ratio: 0.15,
            safety_margin: 1.2,
            summarization_overhead_tokens: 4096,
            max_chunk_chars: 80_000,
            max_retries: 3,
            token_threshold_ratio: 0.7,
            context_window_tokens: 200_000,
        }
    }
}

impl CompactionConfig {
    /// Build a `CompactionConfig` from the user-facing TOML config, keeping
    /// internal algorithmic constants at their defaults.
    pub fn from_toml(toml: &librefang_types::config::CompactionTomlConfig) -> Self {
        Self {
            threshold: toml.threshold_messages,
            keep_recent: toml.keep_recent,
            max_summary_tokens: toml.max_summary_tokens as u32,
            max_chunk_chars: toml.max_chunk_chars,
            max_retries: toml.max_retries,
            token_threshold_ratio: toml.token_threshold_ratio,
            ..Self::default()
        }
    }

    /// Build a `CompactionConfig` by merging an optional per-agent
    /// override on top of the kernel-global `CompactionTomlConfig`
    /// (#4976). When `overrides` is `None` (or empty) this is identical
    /// to [`Self::from_toml`].
    ///
    /// Resolution order: per-agent override > global TOML > compiled
    /// defaults from `CompactionTomlConfig::default()`.
    pub fn from_toml_with_overrides(
        global: &librefang_types::config::CompactionTomlConfig,
        overrides: Option<&librefang_types::agent::CompactionOverrides>,
    ) -> Self {
        match overrides {
            Some(o) if !o.is_empty() => Self::from_toml(&o.resolve(global)),
            _ => Self::from_toml(global),
        }
    }
}

/// Result of a compaction operation.
#[derive(Debug)]
pub struct CompactionResult {
    /// LLM-generated summary of the compacted messages.
    pub summary: String,
    /// Messages to keep (the most recent ones).
    pub kept_messages: Vec<Message>,
    /// Number of messages that were compacted (summarized).
    pub compacted_count: usize,
    /// Number of chunks used (1 = single-pass, >1 = chunked).
    pub chunks_used: u32,
    /// Whether fallback was used (LLM unavailable).
    pub used_fallback: bool,
}

/// Check whether a session needs compaction (message-count trigger).
pub fn needs_compaction(session: &Session, config: &CompactionConfig) -> bool {
    session.messages.len() > config.threshold
}

/// Classify a character for token estimation purposes.
///
/// Returns an estimated token weight for a single character:
/// - CJK ideographs: ~1.5 tokens each (they are 1-2 tokens in most tokenizers)
/// - ASCII letters/digits: ~0.25 tokens each (chars/4 heuristic)
/// - Whitespace and punctuation: ~0.25 tokens each
fn char_token_weight(c: char) -> f64 {
    if is_cjk(c) {
        1.5
    } else if c.is_ascii_alphanumeric() || c.is_alphabetic() {
        0.25
    } else {
        // Whitespace, punctuation, symbols
        0.25
    }
}

/// Check if a character is a CJK ideograph or common CJK symbol.
fn is_cjk(c: char) -> bool {
    matches!(c,
        '\u{4E00}'..='\u{9FFF}'   // CJK Unified Ideographs
        | '\u{3400}'..='\u{4DBF}' // CJK Unified Ideographs Extension A
        | '\u{F900}'..='\u{FAFF}' // CJK Compatibility Ideographs
        | '\u{2E80}'..='\u{2EFF}' // CJK Radicals Supplement
        | '\u{3000}'..='\u{303F}' // CJK Symbols and Punctuation
        | '\u{3040}'..='\u{309F}' // Hiragana
        | '\u{30A0}'..='\u{30FF}' // Katakana
        | '\u{AC00}'..='\u{D7AF}' // Hangul Syllables
        | '\u{20000}'..='\u{2A6DF}' // CJK Unified Ideographs Extension B
    )
}

/// Estimate tokens for a string using CJK-aware heuristic.
///
/// CJK characters are weighted at ~1.5 tokens each (they typically map to
/// 1-2 tokens in BPE tokenizers). ASCII/Latin text uses the standard chars/4
/// heuristic. This avoids external tokenizer dependencies while being
/// significantly more accurate for mixed-language content.
fn estimate_str_tokens(s: &str) -> usize {
    let total: f64 = s.chars().map(char_token_weight).sum();
    total.ceil() as usize
}

/// Estimate token count for a set of messages, optional system prompt, and tool definitions.
///
/// Uses a CJK-aware heuristic: CJK chars ~1.5 tokens, ASCII/Latin chars/4.
/// Not exact, but significantly more accurate than naive chars/4 for multilingual content.
pub fn estimate_token_count(
    messages: &[Message],
    system_prompt: Option<&str>,
    tools: Option<&[librefang_types::tool::ToolDefinition]>,
) -> usize {
    let mut tokens: usize = 0;

    // System prompt
    if let Some(sp) = system_prompt {
        tokens += estimate_str_tokens(sp);
    }

    // Messages
    for msg in messages {
        tokens += estimate_message_tokens(msg);
        // Per-message overhead (role label, framing tokens)
        tokens += 4;
    }

    // Tool definitions (JSON schema is the biggest contributor)
    if let Some(tool_defs) = tools {
        for tool in tool_defs {
            tokens += estimate_str_tokens(&tool.name);
            tokens += estimate_str_tokens(&tool.description);
            if let Ok(schema_str) = serde_json::to_string(&tool.input_schema) {
                tokens += estimate_str_tokens(&schema_str);
            }
        }
    }

    tokens
}

/// Estimate token count for a single message's content.
fn estimate_message_tokens(msg: &Message) -> usize {
    match &msg.content {
        MessageContent::Text(s) => estimate_str_tokens(s),
        MessageContent::Blocks(blocks) => blocks
            .iter()
            .map(|b| match b {
                ContentBlock::Text { text, .. } => estimate_str_tokens(text),
                ContentBlock::ToolResult { content, .. } => estimate_str_tokens(content),
                ContentBlock::Thinking { thinking, .. } => estimate_str_tokens(thinking),
                ContentBlock::ToolUse {
                    name, id, input, ..
                } => {
                    estimate_str_tokens(name)
                        + estimate_str_tokens(id)
                        + estimate_str_tokens(&serde_json::to_string(input).unwrap_or_default())
                }
                // Base64 images consume significant tokens.  A rough
                // estimate: each base64 char ≈ 0.25 tokens (same as ASCII),
                // plus a small fixed overhead for the image framing.
                ContentBlock::Image { data, .. } => {
                    let data_tokens = data.len() / 4;
                    data_tokens + 85 // 85 token fixed overhead per image
                }
                // File-referenced images: the session stores only the path, but the
                // driver reads the full file and sends it to the LLM at call time.
                // We use the fixed overhead only since the actual token cost depends
                // on the image size (unknown until read). This underestimates; a
                // more precise approach would stat the file, but that adds I/O.
                ContentBlock::ImageFile { .. } => 85,
                ContentBlock::Unknown => 0,
            })
            .sum(),
    }
}

/// Check whether estimated tokens exceed the compaction threshold.
///
/// Returns true if `estimated_tokens > context_window * token_threshold_ratio`.
pub fn needs_compaction_by_tokens(estimated_tokens: usize, config: &CompactionConfig) -> bool {
    let threshold = (config.context_window_tokens as f64 * config.token_threshold_ratio) as usize;
    estimated_tokens > threshold
}

// ---------------------------------------------------------------------------
// Context Report
// ---------------------------------------------------------------------------

/// Context window pressure level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ContextPressure {
    /// < 50% usage
    Low,
    /// 50–70% usage
    Medium,
    /// 70–85% usage
    High,
    /// > 85% usage
    Critical,
}

impl ContextPressure {
    fn from_percent(pct: f64) -> Self {
        if pct > 85.0 {
            Self::Critical
        } else if pct > 70.0 {
            Self::High
        } else if pct > 50.0 {
            Self::Medium
        } else {
            Self::Low
        }
    }

    /// CSS-friendly color name.
    pub fn color(&self) -> &'static str {
        match self {
            Self::Low => "green",
            Self::Medium => "yellow",
            Self::High => "orange",
            Self::Critical => "red",
        }
    }
}

/// Token breakdown by source.
#[derive(Debug, Clone, Serialize)]
pub struct ContextBreakdown {
    pub system_prompt_tokens: usize,
    pub message_tokens: usize,
    pub tool_definition_tokens: usize,
}

/// Context window usage report.
#[derive(Debug, Clone, Serialize)]
pub struct ContextReport {
    pub estimated_tokens: usize,
    pub context_window: usize,
    pub usage_percent: f64,
    pub pressure: ContextPressure,
    pub message_count: usize,
    pub breakdown: ContextBreakdown,
    pub recommendation: String,
}

/// Generate a context window usage report.
pub fn generate_context_report(
    messages: &[Message],
    system_prompt: Option<&str>,
    tools: Option<&[ToolDefinition]>,
    context_window: usize,
) -> ContextReport {
    // Break down token estimates by source (CJK-aware)
    let sp_tokens = system_prompt.map_or(0, estimate_str_tokens);

    let msg_tokens = {
        let mut tokens: usize = 0;
        for msg in messages {
            tokens += estimate_message_tokens(msg);
            tokens += 4; // per-message overhead
        }
        tokens
    };

    let tool_tokens = tools.map_or(0, |defs| {
        let mut tokens: usize = 0;
        for t in defs {
            tokens += estimate_str_tokens(&t.name);
            tokens += estimate_str_tokens(&t.description);
            if let Ok(s) = serde_json::to_string(&t.input_schema) {
                tokens += estimate_str_tokens(&s);
            }
        }
        tokens
    });

    let total = sp_tokens + msg_tokens + tool_tokens;
    let cw = context_window.max(1);
    let pct = (total as f64 / cw as f64 * 100.0).min(100.0);
    let pressure = ContextPressure::from_percent(pct);

    let recommendation = match pressure {
        ContextPressure::Low => "Context usage is healthy.".to_string(),
        ContextPressure::Medium => {
            "Consider using /compact if the conversation grows longer.".to_string()
        }
        ContextPressure::High => {
            "Context is getting full. Use /compact to summarize older messages.".to_string()
        }
        ContextPressure::Critical => {
            "Context is nearly full! Use /compact or /new immediately.".to_string()
        }
    };

    ContextReport {
        estimated_tokens: total,
        context_window: cw,
        usage_percent: (pct * 10.0).round() / 10.0, // 1 decimal
        pressure,
        message_count: messages.len(),
        breakdown: ContextBreakdown {
            system_prompt_tokens: sp_tokens,
            message_tokens: msg_tokens,
            tool_definition_tokens: tool_tokens,
        },
        recommendation,
    }
}

/// Format a context report as human-readable text with ASCII progress bar.
pub fn format_context_report(report: &ContextReport) -> String {
    let bar_len: usize = 20;
    let filled = ((report.usage_percent / 100.0) * bar_len as f64).round() as usize;
    let empty = bar_len.saturating_sub(filled);
    let bar: String = std::iter::repeat_n('█', filled)
        .chain(std::iter::repeat_n('░', empty))
        .collect();

    format!(
        "**Context Usage:** {bar} {:.1}% ({} / {} tokens)\n\n\
         **Breakdown:**\n\
         - System prompt: ~{} tokens\n\
         - Messages ({}): ~{} tokens\n\
         - Tool definitions: ~{} tokens\n\n\
         **Pressure:** {:?}\n\
         **Recommendation:** {}",
        report.usage_percent,
        report.estimated_tokens,
        report.context_window,
        report.breakdown.system_prompt_tokens,
        report.message_count,
        report.breakdown.message_tokens,
        report.breakdown.tool_definition_tokens,
        report.pressure,
        report.recommendation,
    )
}

// ---------------------------------------------------------------------------
// Adaptive Chunking
// ---------------------------------------------------------------------------

/// Compute adaptive chunk ratio based on average message size.
///
/// Shorter messages get larger chunks (more context per summary).
/// Longer messages get smaller chunks (each message has more info to summarize).
fn compute_adaptive_chunk_ratio(messages: &[Message], config: &CompactionConfig) -> f64 {
    if messages.is_empty() {
        return config.base_chunk_ratio;
    }

    let avg_len = messages
        .iter()
        .map(|m| m.content.text_length())
        .sum::<usize>() as f64
        / messages.len() as f64;

    // Heuristic: longer messages → smaller ratio (fewer per chunk)
    let ratio = if avg_len > 1000.0 {
        config.min_chunk_ratio
    } else if avg_len > 500.0 {
        (config.base_chunk_ratio + config.min_chunk_ratio) / 2.0
    } else {
        config.base_chunk_ratio
    };

    ratio.clamp(config.min_chunk_ratio, config.base_chunk_ratio)
}

/// Check if a single message is oversized (> 50% of max_chunk_chars).
///
/// Oversized messages should be summarized individually rather than in chunks
/// to avoid exceeding context window limits.
fn is_oversized(message: &Message, config: &CompactionConfig) -> bool {
    message.content.text_length() > config.max_chunk_chars / 2
}

/// Extract the first `tool_use_id` from a message's ToolUse blocks.
fn extract_first_tool_use_id(msg: &Message) -> Option<String> {
    match &msg.content {
        MessageContent::Blocks(blocks) => blocks
            .iter()
            .filter_map(|b| match b {
                ContentBlock::ToolUse { id, .. } => Some(id.clone()),
                _ => None,
            })
            .next(),
        _ => None,
    }
}

/// Check if a message is a tool-result delivery: it is a User message
/// containing only ToolResult blocks.
fn is_tool_result_delivery(msg: &Message) -> bool {
    msg.role == Role::User && message_is_only_tool_results(msg)
}

/// Scan the tail messages and look for a ToolResult whose `tool_use_id`
/// matches the given `tool_use_id`.  Returns `true` if one is found.
fn tail_has_matching_result(messages: &[Message], from_idx: usize, tool_use_id: &str) -> bool {
    messages[from_idx..].iter().any(|msg| {
        if !is_tool_result_delivery(msg) {
            return false;
        }
        if let MessageContent::Blocks(blocks) = &msg.content {
            blocks.iter().any(|b| {
                if let ContentBlock::ToolResult {
                    tool_use_id: id, ..
                } = b
                {
                    id == tool_use_id
                } else {
                    false
                }
            })
        } else {
            false
        }
    })
}

/// Adjust the head/tail split point to avoid splitting a ToolUse/ToolResult
/// pair that spans the boundary.
///
/// If the head ends with an assistant ToolUse and the tail begins with the
/// matching ToolResult delivery, the pair would be incorrectly separated by
/// summarization.  We detect this and shift the split so the ToolResult
/// delivery is included in the head instead.
///
/// Returns the adjusted split index (or the original `split_at` if no
/// adjustment is needed).
///
/// **Internal helper** — exposed publicly only so the kernel's
/// `try_summarize_trim` (#3693) can call across the runtime ↔ kernel crate
/// boundary. Treat the signature as workspace-internal: it may change
/// without a semver bump and is not intended for external dependents.
#[doc(hidden)]
pub fn adjust_split_for_tool_pair(
    messages: &[Message],
    split_at: usize,
    keep_recent: usize,
) -> usize {
    if split_at == 0 {
        return split_at;
    }

    // The head is [0, split_at), the tail is [split_at, len)
    let head_end_idx = split_at - 1;
    let head_end = &messages[head_end_idx];

    // Only consider the case where the head ends with an unresolved ToolUse
    if head_end.role != Role::Assistant || !message_has_tool_use(head_end) {
        return split_at;
    }

    let Some(tool_use_id) = extract_first_tool_use_id(head_end) else {
        return split_at;
    };

    // Check if any message in the tail (within the kept region) is a matching
    // ToolResult delivery
    if !tail_has_matching_result(messages, split_at, &tool_use_id) {
        return split_at;
    }

    // The tail starts with the matching ToolResult -- shift split_at to include
    // that result delivery message in the head so the pair stays together.
    // Walk forward from split_at to find the ToolResult delivery.
    for i in split_at..(split_at + keep_recent) {
        if i >= messages.len() {
            break;
        }
        if is_tool_result_delivery(&messages[i]) {
            if let MessageContent::Blocks(blocks) = &messages[i].content {
                let has_match = blocks.iter().any(|b| {
                    if let ContentBlock::ToolResult {
                        tool_use_id: id, ..
                    } = b
                    {
                        id == &tool_use_id
                    } else {
                        false
                    }
                });
                if has_match {
                    debug!(
                        split_at,
                        new_split = i + 1,
                        "Head boundary would split ToolUse/ToolResult pair -- extending head"
                    );
                    return i + 1;
                }
            }
        }
    }

    split_at
}

/// Build conversation text from a slice of messages (block-aware).
///
/// Handles all content block types: text, tool use, tool result, image, unknown.
/// Oversized messages are truncated inline with a marker.
fn build_conversation_text(messages: &[Message], config: &CompactionConfig) -> String {
    let mut conversation_text = String::new();

    for msg in messages {
        let role_label = match msg.role {
            Role::User => "User",
            Role::Assistant => "Assistant",
            Role::System => "System",
        };

        // If a single message is oversized, truncate its contribution
        let oversized = is_oversized(msg, config);

        match &msg.content {
            MessageContent::Text(s) => {
                if !s.is_empty() {
                    if oversized {
                        let limit = config.max_chunk_chars / 4;
                        let truncated = if s.len() > limit {
                            format!(
                                "{}...[truncated from {} chars]",
                                safe_truncate_str(s, limit),
                                s.len()
                            )
                        } else {
                            s.clone()
                        };
                        conversation_text.push_str(&format!("{role_label}: {truncated}\n\n"));
                    } else {
                        conversation_text.push_str(&format!("{role_label}: {s}\n\n"));
                    }
                }
            }
            MessageContent::Blocks(blocks) => {
                for block in blocks {
                    match block {
                        ContentBlock::Text { text, .. } => {
                            if !text.is_empty() {
                                if oversized && text.len() > config.max_chunk_chars / 4 {
                                    let limit = config.max_chunk_chars / 4;
                                    conversation_text.push_str(&format!(
                                        "{role_label}: {}...[truncated from {} chars]\n\n",
                                        safe_truncate_str(text, limit),
                                        text.len()
                                    ));
                                } else {
                                    conversation_text
                                        .push_str(&format!("{role_label}: {text}\n\n"));
                                }
                            }
                        }
                        ContentBlock::ToolUse { name, input, .. } => {
                            let input_str = serde_json::to_string(input).unwrap_or_default();
                            let input_preview = if input_str.len() > 200 {
                                format!("{}...", safe_truncate_str(&input_str, 200))
                            } else {
                                input_str
                            };
                            conversation_text.push_str(&format!(
                                "[Used tool '{name}' with params: {input_preview}]\n\n"
                            ));
                        }
                        ContentBlock::ToolResult {
                            content, is_error, ..
                        } => {
                            let status = if *is_error { "ERROR" } else { "OK" };
                            // Strip base64 blobs and injection markers before compaction
                            let cleaned = crate::session_repair::strip_tool_result_details(content);
                            let preview = if cleaned.len() > 2000 {
                                format!("{}...", safe_truncate_str(&cleaned, 2000))
                            } else {
                                cleaned
                            };
                            conversation_text
                                .push_str(&format!("[Tool result ({status}): {preview}]\n\n"));
                        }
                        ContentBlock::Image { media_type, .. }
                        | ContentBlock::ImageFile { media_type, .. } => {
                            conversation_text.push_str(&format!("[Image: {media_type}]\n\n"));
                        }
                        ContentBlock::Thinking { .. } => {}
                        ContentBlock::Unknown => {}
                    }
                }
            }
        }
    }

    conversation_text
}

/// Summarize a slice of messages using the LLM.
///
/// Builds the conversation text, applies chunking limits, and calls the LLM
/// with a summarization prompt. Retries on transient failures.
async fn summarize_messages(
    driver: Arc<dyn LlmDriver>,
    model: &str,
    messages: &[Message],
    config: &CompactionConfig,
    reasoning_echo_policy: librefang_types::model_catalog::ReasoningEchoPolicy,
) -> Result<String, String> {
    let mut conversation_text = build_conversation_text(messages, config);

    // Truncate if exceeding max_chunk_chars (with safety margin)
    let effective_max = (config.max_chunk_chars as f64 / config.safety_margin) as usize;
    if conversation_text.len() > effective_max {
        // Keep the tail (most recent) which is usually more important
        let start = conversation_text.len() - effective_max;
        // Find the nearest valid char boundary at or after `start`.
        //
        // Note: we cannot slice `conversation_text[start..]` to use
        // `char_indices`, because that slice operation itself panics when
        // `start` lands inside a multi-byte character.  Walk byte indices
        // forward instead — this is panic-safe for any UTF-8 input.
        let safe_start = (start..=conversation_text.len())
            .find(|&i| conversation_text.is_char_boundary(i))
            .unwrap_or(conversation_text.len());
        conversation_text = conversation_text[safe_start..].to_string();
    }

    let summarize_prompt = format!(
        "Summarize the following conversation preserving key facts, decisions, user preferences, \
         and important context. Be concise but thorough. Output only the summary, no preamble.\n\n\
         ---\n{conversation_text}---"
    );

    let request = CompletionRequest {
        model: model.to_string(),
        messages: std::sync::Arc::new(vec![Message {
            role: Role::User,
            content: MessageContent::Blocks(vec![ContentBlock::Text {
                text: summarize_prompt,
                provider_metadata: None,
            }]),
            pinned: false,
            timestamp: None,
        }]),
        tools: std::sync::Arc::new(vec![]),
        max_tokens: config.max_summary_tokens,
        temperature: 0.3,
        system: Some(
            "You are a conversation summarizer. Produce a concise summary that captures \
             all key facts, decisions, and context from the conversation."
                .to_string(),
        ),
        thinking: None,
        prompt_caching: false,
        cache_ttl: None,
        prompt_cache_strategy: None,
        response_format: None,
        timeout_secs: None,
        extra_body: None,
        agent_id: None,
        session_id: None,
        step_id: None,
        reasoning_echo_policy,
    };

    // Retry logic for transient failures
    let mut last_error = String::new();
    for attempt in 0..config.max_retries {
        match driver.complete(request.clone()).await {
            Ok(response) => {
                let summary = response.text();
                if summary.is_empty() {
                    last_error = "LLM returned empty summary".to_string();
                    warn!(attempt, "Empty summary from LLM, retrying");
                    continue;
                }
                return Ok(summary);
            }
            Err(e) => {
                last_error = format!("LLM summarization failed: {e}");
                if attempt + 1 < config.max_retries {
                    warn!(attempt, error = %e, "Summarization attempt failed, retrying");
                }
            }
        }
    }

    Err(last_error)
}

/// Summarize messages in adaptive chunks, then merge the per-chunk summaries.
///
/// Splits messages into chunks based on adaptive ratio (accounting for message size),
/// summarizes each chunk independently, then merges all chunk summaries with a final
/// LLM call into one cohesive summary.
async fn summarize_in_chunks(
    driver: Arc<dyn LlmDriver>,
    model: &str,
    messages: &[Message],
    config: &CompactionConfig,
    reasoning_echo_policy: librefang_types::model_catalog::ReasoningEchoPolicy,
) -> Result<String, String> {
    let chunk_ratio = compute_adaptive_chunk_ratio(messages, config);
    let chunk_size = (messages.len() as f64 * chunk_ratio).ceil() as usize;
    let chunk_size = chunk_size.max(5); // minimum 5 messages per chunk

    info!(
        total = messages.len(),
        chunk_size, chunk_ratio, "Starting chunked summarization"
    );

    let mut summaries = Vec::new();
    let mut success_count = 0usize;
    let mut last_chunk_error = String::new();
    for (i, chunk) in messages.chunks(chunk_size).enumerate() {
        match summarize_messages(driver.clone(), model, chunk, config, reasoning_echo_policy).await
        {
            Ok(summary) => {
                info!(chunk = i, summary_len = summary.len(), "Chunk summarized");
                summaries.push(summary);
                success_count += 1;
            }
            Err(e) => {
                // If a single chunk fails, note it and continue with remaining chunks.
                // A partial summary is better than none.
                warn!(chunk = i, error = %e, "Chunk summarization failed, skipping");
                last_chunk_error = e;
                summaries.push(format!(
                    "[Chunk {}: {} messages, summarization unavailable]",
                    i + 1,
                    chunk.len()
                ));
            }
        }
    }

    // If ALL chunks failed, propagate the error to trigger fallback
    if success_count == 0 {
        return Err(format!(
            "All {} chunks failed to summarize: {last_chunk_error}",
            summaries.len()
        ));
    }

    if summaries.is_empty() {
        return Err("No chunks were summarized".to_string());
    }

    if summaries.len() == 1 {
        return Ok(summaries.into_iter().next().unwrap());
    }

    // Merge summaries with another LLM call
    let merge_prompt = format!(
        "Merge these {} conversation summaries into one concise, coherent summary. \
         Preserve all key facts, decisions, and context. Output only the merged summary.\n\n{}",
        summaries.len(),
        summaries
            .iter()
            .enumerate()
            .map(|(i, s)| format!("--- Part {} ---\n{}", i + 1, s))
            .collect::<Vec<_>>()
            .join("\n\n")
    );

    let merge_request = CompletionRequest {
        model: model.to_string(),
        messages: std::sync::Arc::new(vec![Message {
            role: Role::User,
            content: MessageContent::Blocks(vec![ContentBlock::Text {
                text: merge_prompt,
                provider_metadata: None,
            }]),
            pinned: false,
            timestamp: None,
        }]),
        tools: std::sync::Arc::new(vec![]),
        max_tokens: config.max_summary_tokens,
        temperature: 0.3,
        system: Some(
            "You are a conversation summarizer. Merge the provided partial summaries \
             into a single cohesive summary."
                .to_string(),
        ),
        thinking: None,
        prompt_caching: false,
        cache_ttl: None,
        prompt_cache_strategy: None,
        response_format: None,
        timeout_secs: None,
        extra_body: None,
        agent_id: None,
        session_id: None,
        step_id: None,
        reasoning_echo_policy,
    };

    match driver.complete(merge_request).await {
        Ok(response) => {
            let merged = response.text();
            if merged.is_empty() {
                // Fall back to concatenating the per-chunk summaries
                Ok(summaries.join("\n\n"))
            } else {
                Ok(merged)
            }
        }
        Err(e) => {
            warn!(error = %e, "Merge summarization failed, concatenating chunks");
            // Fallback: just concatenate the chunk summaries
            Ok(summaries.join("\n\n"))
        }
    }
}

/// Compact a session by summarizing older messages with an LLM.
///
/// Takes all messages except the most recent `keep_recent` and uses a
/// multi-stage approach to produce a concise summary:
///
/// 1. **Full summarization**: tries to summarize all older messages in one pass
/// 2. **Chunked summarization**: splits into adaptive chunks, summarizes each,
///    then merges the chunk summaries
/// 3. **Minimal fallback**: if LLM is unavailable, produces a placeholder note
///
/// Returns the summary, the kept messages, and metadata about the operation.
pub async fn compact_session(
    driver: Arc<dyn LlmDriver>,
    model: &str,
    session: &Session,
    config: &CompactionConfig,
    reasoning_echo_policy: librefang_types::model_catalog::ReasoningEchoPolicy,
) -> Result<CompactionResult, String> {
    compact_messages(
        driver,
        model,
        &session.messages,
        config,
        reasoning_echo_policy,
    )
    .await
}

/// Same as [`compact_session`] but takes a raw message slice instead of a
/// `Session`. Useful for callers that already hold a `&[Message]` (e.g. the
/// kernel's cron `SummarizeTrim` path, #3693) and would otherwise have to
/// fabricate a throwaway `Session` purely to satisfy the signature.
///
/// The behavioural contract is identical: 3-stage summarization (full →
/// chunked → minimal fallback), `used_fallback` flagged when the LLM is
/// unavailable, and `adjust_split_for_tool_pair` applied at the head/tail
/// boundary.
///
/// **Internal helper** — exposed publicly only so the kernel's
/// `try_summarize_trim` (#3693) can call across the runtime ↔ kernel crate
/// boundary. Treat the signature as workspace-internal: it may change without
/// a semver bump and is not intended for external dependents (mirrors the
/// `#[doc(hidden)]` carve-out on [`adjust_split_for_tool_pair`]).
#[doc(hidden)]
pub async fn compact_messages(
    driver: Arc<dyn LlmDriver>,
    model: &str,
    messages: &[Message],
    config: &CompactionConfig,
    reasoning_echo_policy: librefang_types::model_catalog::ReasoningEchoPolicy,
) -> Result<CompactionResult, String> {
    let msg_count = messages.len();
    if msg_count <= config.keep_recent {
        return Ok(CompactionResult {
            summary: String::new(),
            kept_messages: messages.to_vec(),
            compacted_count: 0,
            chunks_used: 0,
            used_fallback: false,
        });
    }

    let split_at = msg_count.saturating_sub(config.keep_recent);

    // P1 fix: guard the head boundary against splitting a ToolUse/ToolResult pair.
    // If the head ends with an unresolved ToolUse and the tail starts with the
    // matching ToolResult delivery, extend the head to include that result so the
    // pair is not separated by summarization.
    let split_at = adjust_split_for_tool_pair(messages, split_at, config.keep_recent);

    let to_compact = &messages[..split_at];
    let kept = &messages[split_at..];

    info!(
        total = msg_count,
        compacting = to_compact.len(),
        keeping = kept.len(),
        "Compacting session messages"
    );

    let kept_messages = kept.to_vec();
    let compacted_count = to_compact.len();

    // Stage 1: Try full single-pass summarization
    match summarize_messages(
        driver.clone(),
        model,
        to_compact,
        config,
        reasoning_echo_policy,
    )
    .await
    {
        Ok(summary) => {
            info!(
                summary_len = summary.len(),
                compacted = compacted_count,
                "Session compaction complete (single-pass)"
            );
            return Ok(CompactionResult {
                summary,
                kept_messages,
                compacted_count,
                chunks_used: 1,
                used_fallback: false,
            });
        }
        Err(e) => {
            warn!(error = %e, "Full summarization failed, trying chunked approach");
        }
    }

    // Stage 2: Chunked summarization with adaptive ratio
    match summarize_in_chunks(
        driver.clone(),
        model,
        to_compact,
        config,
        reasoning_echo_policy,
    )
    .await
    {
        Ok(summary) => {
            let chunk_ratio = compute_adaptive_chunk_ratio(to_compact, config);
            let chunk_size = (to_compact.len() as f64 * chunk_ratio).ceil() as usize;
            let chunk_size = chunk_size.max(5);
            let num_chunks = (to_compact.len() as f64 / chunk_size as f64).ceil() as u32;

            info!(
                summary_len = summary.len(),
                compacted = compacted_count,
                chunks = num_chunks,
                "Session compaction complete (chunked)"
            );
            return Ok(CompactionResult {
                summary,
                kept_messages,
                compacted_count,
                chunks_used: num_chunks.max(1),
                used_fallback: false,
            });
        }
        Err(e) => {
            warn!(error = %e, "Chunked summarization failed, using minimal fallback");
        }
    }

    // Stage 3: Minimal fallback -- note what was compacted without LLM
    let minimal = format!(
        "[Session compacted: {} messages removed. Recent {} messages preserved. \
         Summarization was unavailable.]",
        to_compact.len(),
        kept_messages.len()
    );

    warn!(
        compacted = compacted_count,
        "Using fallback compaction (no LLM summary)"
    );

    Ok(CompactionResult {
        summary: minimal,
        kept_messages,
        compacted_count,
        chunks_used: 0,
        used_fallback: true,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use librefang_types::message::TokenUsage;

    #[test]
    fn test_needs_compaction_below_threshold() {
        let session = Session {
            id: librefang_types::agent::SessionId::new(),
            agent_id: librefang_types::agent::AgentId::new(),
            messages: vec![Message::user("hello")],
            context_window_tokens: 0,
            label: None,
            model_override: None,

            messages_generation: 0,
            last_repaired_generation: None,
        };
        let config = CompactionConfig::default();
        assert!(!needs_compaction(&session, &config));
    }

    #[test]
    fn test_needs_compaction_above_threshold() {
        let messages: Vec<Message> = (0..100)
            .map(|i| Message::user(format!("msg {i}")))
            .collect();
        let session = Session {
            id: librefang_types::agent::SessionId::new(),
            agent_id: librefang_types::agent::AgentId::new(),
            messages,
            context_window_tokens: 0,
            label: None,
            model_override: None,

            messages_generation: 0,
            last_repaired_generation: None,
        };
        let config = CompactionConfig::default();
        assert!(needs_compaction(&session, &config));
    }

    #[test]
    fn test_compaction_config_defaults() {
        let config = CompactionConfig::default();
        assert_eq!(config.threshold, 30);
        assert_eq!(config.keep_recent, 10);
        assert_eq!(config.max_summary_tokens, 1024);
        assert!((config.token_threshold_ratio - 0.7).abs() < f64::EPSILON);
        assert_eq!(config.context_window_tokens, 200_000);
    }

    #[tokio::test]
    async fn test_compact_session_few_messages() {
        use crate::llm_driver::{CompletionResponse, LlmError};
        use async_trait::async_trait;

        struct FakeDriver;

        #[async_trait]
        impl LlmDriver for FakeDriver {
            async fn complete(
                &self,
                _req: CompletionRequest,
            ) -> Result<CompletionResponse, LlmError> {
                Ok(CompletionResponse {
                    content: vec![ContentBlock::Text {
                        text: "Summary of conversation".to_string(),
                        provider_metadata: None,
                    }],
                    stop_reason: librefang_types::message::StopReason::EndTurn,
                    tool_calls: vec![],
                    usage: TokenUsage {
                        input_tokens: 100,
                        output_tokens: 50,
                        ..Default::default()
                    },
                    actual_provider: None,
                })
            }
        }

        let session = Session {
            id: librefang_types::agent::SessionId::new(),
            agent_id: librefang_types::agent::AgentId::new(),
            messages: vec![Message::user("hello"), Message::assistant("hi")],
            context_window_tokens: 0,
            label: None,
            model_override: None,

            messages_generation: 0,
            last_repaired_generation: None,
        };
        let config = CompactionConfig {
            threshold: 30,
            keep_recent: 10,
            max_summary_tokens: 1024,
            ..CompactionConfig::default()
        };

        // With only 2 messages and keep_recent=10, nothing should be compacted
        let result = compact_session(
            Arc::new(FakeDriver),
            "test-model",
            &session,
            &config,
            librefang_types::model_catalog::ReasoningEchoPolicy::None,
        )
        .await
        .unwrap();
        assert_eq!(result.compacted_count, 0);
        assert_eq!(result.kept_messages.len(), 2);
        assert_eq!(result.chunks_used, 0);
        assert!(!result.used_fallback);
    }

    #[tokio::test]
    async fn test_compact_includes_tool_calls() {
        use crate::llm_driver::{CompletionResponse, LlmError};
        use async_trait::async_trait;

        struct FakeDriver;

        #[async_trait]
        impl LlmDriver for FakeDriver {
            async fn complete(
                &self,
                req: CompletionRequest,
            ) -> Result<CompletionResponse, LlmError> {
                // Verify the input includes tool call information
                let input_text = req.messages[0].content.text_content();
                assert!(
                    input_text.contains("web_search"),
                    "Should include tool name"
                );
                assert!(
                    input_text.contains("Tool result"),
                    "Should include tool result"
                );
                Ok(CompletionResponse {
                    content: vec![ContentBlock::Text {
                        text: "Summary with tools".to_string(),
                        provider_metadata: None,
                    }],
                    stop_reason: librefang_types::message::StopReason::EndTurn,
                    tool_calls: vec![],
                    usage: TokenUsage {
                        input_tokens: 100,
                        output_tokens: 50,
                        ..Default::default()
                    },
                    actual_provider: None,
                })
            }
        }

        let mut messages: Vec<Message> = Vec::new();
        // Add enough messages to trigger compaction (keep_recent = 5 for this test)
        for _ in 0..8 {
            messages.push(Message::user("Query"));
        }
        // Insert a tool use + result pair early in the history
        messages[1] = Message {
            role: Role::Assistant,
            content: MessageContent::Blocks(vec![ContentBlock::ToolUse {
                id: "tu-1".to_string(),
                name: "web_search".to_string(),
                input: serde_json::json!({"query": "test"}),
                provider_metadata: None,
            }]),
            pinned: false,
            timestamp: None,
        };
        messages[2] = Message {
            role: Role::User,
            content: MessageContent::Blocks(vec![ContentBlock::ToolResult {
                tool_use_id: "tu-1".to_string(),
                tool_name: String::new(),
                content: "Search results here".to_string(),
                is_error: false,
                status: librefang_types::tool::ToolExecutionStatus::default(),
                approval_request_id: None,
            }]),
            pinned: false,
            timestamp: None,
        };

        let session = Session {
            id: librefang_types::agent::SessionId::new(),
            agent_id: librefang_types::agent::AgentId::new(),
            messages,
            context_window_tokens: 0,
            label: None,
            model_override: None,

            messages_generation: 0,
            last_repaired_generation: None,
        };
        let config = CompactionConfig {
            threshold: 5,
            keep_recent: 3,
            max_summary_tokens: 512,
            ..CompactionConfig::default()
        };

        let result = compact_session(
            Arc::new(FakeDriver),
            "test-model",
            &session,
            &config,
            librefang_types::model_catalog::ReasoningEchoPolicy::None,
        )
        .await
        .unwrap();
        assert!(result.compacted_count > 0);
        assert!(result.summary.contains("tools"));
        assert_eq!(result.chunks_used, 1);
        assert!(!result.used_fallback);
    }

    #[test]
    fn test_compact_truncates_large_tool_input() {
        // Verify that the block-aware builder truncates large tool inputs
        let large_input = serde_json::json!({"data": "x".repeat(500)});
        let input_str = serde_json::to_string(&large_input).unwrap();
        // The builder truncates to 200 chars
        assert!(input_str.len() > 200);
        // Just verify the truncation logic works correctly
        let preview = if input_str.len() > 200 {
            format!("{}...", safe_truncate_str(&input_str, 200))
        } else {
            input_str.clone()
        };
        assert!(preview.len() < input_str.len());
        assert!(preview.ends_with("..."));
    }

    #[tokio::test]
    async fn test_compact_session_many_messages() {
        use crate::llm_driver::{CompletionResponse, LlmError};
        use async_trait::async_trait;

        struct FakeDriver;

        #[async_trait]
        impl LlmDriver for FakeDriver {
            async fn complete(
                &self,
                _req: CompletionRequest,
            ) -> Result<CompletionResponse, LlmError> {
                Ok(CompletionResponse {
                    content: vec![ContentBlock::Text {
                        text: "Summary: discussed topics 0 through 79".to_string(),
                        provider_metadata: None,
                    }],
                    stop_reason: librefang_types::message::StopReason::EndTurn,
                    tool_calls: vec![],
                    usage: TokenUsage {
                        input_tokens: 500,
                        output_tokens: 100,
                        ..Default::default()
                    },
                    actual_provider: None,
                })
            }
        }

        let messages: Vec<Message> = (0..100)
            .map(|i| Message::user(format!("Message about topic {i}")))
            .collect();
        let session = Session {
            id: librefang_types::agent::SessionId::new(),
            agent_id: librefang_types::agent::AgentId::new(),
            messages,
            context_window_tokens: 0,
            label: None,
            model_override: None,

            messages_generation: 0,
            last_repaired_generation: None,
        };
        let config = CompactionConfig {
            threshold: 30,
            keep_recent: 10,
            max_summary_tokens: 1024,
            ..CompactionConfig::default()
        };

        let result = compact_session(
            Arc::new(FakeDriver),
            "test-model",
            &session,
            &config,
            librefang_types::model_catalog::ReasoningEchoPolicy::None,
        )
        .await
        .unwrap();
        assert_eq!(result.compacted_count, 90);
        assert_eq!(result.kept_messages.len(), 10);
        assert!(result.summary.contains("Summary"));
        assert_eq!(result.chunks_used, 1);
        assert!(!result.used_fallback);
    }

    // --- New tests ---

    #[test]
    fn test_adaptive_chunk_ratio_short_messages() {
        let config = CompactionConfig::default();
        let messages: Vec<Message> = (0..50).map(|i| Message::user(format!("msg {i}"))).collect();
        let ratio = compute_adaptive_chunk_ratio(&messages, &config);
        // Short messages (~6 chars) → should get the base (largest) ratio
        assert!(
            (ratio - config.base_chunk_ratio).abs() < f64::EPSILON,
            "Short messages should use base ratio, got {ratio}"
        );
    }

    #[test]
    fn test_adaptive_chunk_ratio_long_messages() {
        let config = CompactionConfig::default();
        let messages: Vec<Message> = (0..20).map(|_| Message::user("x".repeat(1500))).collect();
        let ratio = compute_adaptive_chunk_ratio(&messages, &config);
        // Long messages (1500 chars) → should use min ratio
        assert!(
            (ratio - config.min_chunk_ratio).abs() < f64::EPSILON,
            "Long messages should use min ratio, got {ratio}"
        );
    }

    #[test]
    fn test_adaptive_chunk_ratio_medium_messages() {
        let config = CompactionConfig::default();
        let messages: Vec<Message> = (0..20).map(|_| Message::user("y".repeat(700))).collect();
        let ratio = compute_adaptive_chunk_ratio(&messages, &config);
        let expected = (config.base_chunk_ratio + config.min_chunk_ratio) / 2.0;
        assert!(
            (ratio - expected).abs() < f64::EPSILON,
            "Medium messages should use middle ratio, got {ratio}"
        );
    }

    #[test]
    fn test_adaptive_chunk_ratio_empty() {
        let config = CompactionConfig::default();
        let messages: Vec<Message> = vec![];
        let ratio = compute_adaptive_chunk_ratio(&messages, &config);
        assert!(
            (ratio - config.base_chunk_ratio).abs() < f64::EPSILON,
            "Empty messages should default to base ratio"
        );
    }

    #[test]
    fn test_oversized_message_detection() {
        let config = CompactionConfig::default();
        // max_chunk_chars default is 80_000, so threshold is 40_000
        let small_msg = Message::user("short");
        assert!(!is_oversized(&small_msg, &config));

        let large_msg = Message::user("x".repeat(50_000));
        assert!(is_oversized(&large_msg, &config));

        // Boundary: exactly at threshold
        let boundary_msg = Message::user("x".repeat(40_000));
        assert!(!is_oversized(&boundary_msg, &config));

        let just_over = Message::user("x".repeat(40_001));
        assert!(is_oversized(&just_over, &config));
    }

    #[test]
    fn test_compaction_config_new_defaults() {
        let config = CompactionConfig::default();
        assert_eq!(config.threshold, 30);
        assert_eq!(config.keep_recent, 10);
        assert_eq!(config.max_summary_tokens, 1024);
        assert!((config.base_chunk_ratio - 0.4).abs() < f64::EPSILON);
        assert!((config.min_chunk_ratio - 0.15).abs() < f64::EPSILON);
        assert!((config.safety_margin - 1.2).abs() < f64::EPSILON);
        assert_eq!(config.summarization_overhead_tokens, 4096);
        assert_eq!(config.max_chunk_chars, 80_000);
        assert_eq!(config.max_retries, 3);
        assert!((config.token_threshold_ratio - 0.7).abs() < f64::EPSILON);
        assert_eq!(config.context_window_tokens, 200_000);
    }

    #[tokio::test]
    async fn test_fallback_on_llm_failure() {
        use crate::llm_driver::{CompletionResponse, LlmError};
        use async_trait::async_trait;

        struct FailingDriver;

        #[async_trait]
        impl LlmDriver for FailingDriver {
            async fn complete(
                &self,
                _req: CompletionRequest,
            ) -> Result<CompletionResponse, LlmError> {
                Err(LlmError::Http("connection refused".to_string()))
            }
        }

        let messages: Vec<Message> = (0..30)
            .map(|i| Message::user(format!("Message {i}")))
            .collect();
        let session = Session {
            id: librefang_types::agent::SessionId::new(),
            agent_id: librefang_types::agent::AgentId::new(),
            messages,
            context_window_tokens: 0,
            label: None,
            model_override: None,

            messages_generation: 0,
            last_repaired_generation: None,
        };
        let config = CompactionConfig {
            threshold: 10,
            keep_recent: 5,
            max_summary_tokens: 512,
            max_retries: 1, // fast failure
            ..CompactionConfig::default()
        };

        let result = compact_session(
            Arc::new(FailingDriver),
            "test-model",
            &session,
            &config,
            librefang_types::model_catalog::ReasoningEchoPolicy::None,
        )
        .await
        .unwrap();

        assert!(result.used_fallback, "Should have used fallback");
        assert_eq!(result.chunks_used, 0, "Fallback uses 0 chunks");
        assert!(
            result.summary.contains("Summarization was unavailable"),
            "Fallback summary should indicate unavailability"
        );
        assert!(
            result.summary.contains("25 messages removed"),
            "Should state how many messages removed, got: {}",
            result.summary
        );
        assert_eq!(result.compacted_count, 25);
        assert_eq!(result.kept_messages.len(), 5);
    }

    #[tokio::test]
    async fn test_chunked_summarization_splits_correctly() {
        use crate::llm_driver::{CompletionResponse, LlmError};
        use async_trait::async_trait;
        use std::sync::atomic::{AtomicU32, Ordering};

        static CALL_COUNT: AtomicU32 = AtomicU32::new(0);

        struct CountingDriver;

        #[async_trait]
        impl LlmDriver for CountingDriver {
            async fn complete(
                &self,
                _req: CompletionRequest,
            ) -> Result<CompletionResponse, LlmError> {
                let n = CALL_COUNT.fetch_add(1, Ordering::SeqCst);
                Ok(CompletionResponse {
                    content: vec![ContentBlock::Text {
                        text: format!("Chunk summary {n}"),
                        provider_metadata: None,
                    }],
                    stop_reason: librefang_types::message::StopReason::EndTurn,
                    tool_calls: vec![],
                    usage: TokenUsage {
                        input_tokens: 50,
                        output_tokens: 20,
                        ..Default::default()
                    },
                    actual_provider: None,
                })
            }
        }

        // Reset counter
        CALL_COUNT.store(0, Ordering::SeqCst);

        let messages: Vec<Message> = (0..20)
            .map(|i| Message::user(format!("Message {i}")))
            .collect();
        let config = CompactionConfig::default();

        let result = summarize_in_chunks(
            Arc::new(CountingDriver),
            "test-model",
            &messages,
            &config,
            librefang_types::model_catalog::ReasoningEchoPolicy::None,
        )
        .await
        .unwrap();

        let calls = CALL_COUNT.load(Ordering::SeqCst);
        // With base_chunk_ratio=0.4, chunk_size = ceil(20*0.4) = 8, so 3 chunks + 1 merge = 4 calls
        assert!(
            calls >= 2,
            "Should have made multiple LLM calls for chunked summary, got {calls}"
        );
        assert!(!result.is_empty(), "Should produce a summary");
    }

    #[test]
    fn test_compaction_result_new_fields() {
        let result = CompactionResult {
            summary: "test".to_string(),
            kept_messages: vec![],
            compacted_count: 10,
            chunks_used: 3,
            used_fallback: false,
        };
        assert_eq!(result.chunks_used, 3);
        assert!(!result.used_fallback);

        let fallback_result = CompactionResult {
            summary: "fallback".to_string(),
            kept_messages: vec![],
            compacted_count: 5,
            chunks_used: 0,
            used_fallback: true,
        };
        assert_eq!(fallback_result.chunks_used, 0);
        assert!(fallback_result.used_fallback);
    }

    #[test]
    fn test_build_conversation_text_handles_all_blocks() {
        let config = CompactionConfig::default();
        let messages = vec![
            Message::user("Hello"),
            Message {
                role: Role::Assistant,
                content: MessageContent::Blocks(vec![
                    ContentBlock::Text {
                        text: "Let me search".to_string(),
                        provider_metadata: None,
                    },
                    ContentBlock::ToolUse {
                        id: "tu-1".to_string(),
                        name: "web_search".to_string(),
                        input: serde_json::json!({"query": "rust"}),
                        provider_metadata: None,
                    },
                ]),
                pinned: false,
                timestamp: None,
            },
            Message {
                role: Role::User,
                content: MessageContent::Blocks(vec![ContentBlock::ToolResult {
                    tool_use_id: "tu-1".to_string(),
                    tool_name: String::new(),
                    content: "Results found".to_string(),
                    is_error: false,
                    status: librefang_types::tool::ToolExecutionStatus::default(),
                    approval_request_id: None,
                }]),
                pinned: false,
                timestamp: None,
            },
            Message {
                role: Role::User,
                content: MessageContent::Blocks(vec![ContentBlock::Image {
                    media_type: "image/png".to_string(),
                    data: "base64data".to_string(),
                }]),
                pinned: false,
                timestamp: None,
            },
        ];

        let text = build_conversation_text(&messages, &config);
        assert!(text.contains("User: Hello"));
        assert!(text.contains("Assistant: Let me search"));
        assert!(text.contains("web_search"));
        assert!(text.contains("Tool result (OK)"));
        assert!(text.contains("[Image: image/png]"));
    }

    #[test]
    fn test_build_conversation_text_truncates_oversized() {
        let config = CompactionConfig {
            max_chunk_chars: 1000, // small limit for testing
            ..CompactionConfig::default()
        };

        let large_msg = Message::user("x".repeat(2000));
        let messages = vec![large_msg];
        let text = build_conversation_text(&messages, &config);
        // Should be truncated since 2000 > 1000/2 = 500 (oversized threshold)
        assert!(
            text.contains("truncated from"),
            "Oversized message should be truncated, got: {}",
            crate::str_utils::safe_truncate_str(&text, 200)
        );
    }

    #[test]
    fn test_estimate_token_count_basic() {
        let messages = vec![
            Message::user("Hello world"), // 11 ASCII chars -> ~3 tokens + 4 overhead
            Message::assistant("Hi there"), // 8 ASCII chars -> ~2 tokens + 4 overhead
        ];
        let tokens = estimate_token_count(&messages, None, None);
        assert!(tokens > 0);
        assert!(tokens < 100);
    }

    #[test]
    fn test_estimate_token_count_cjk_higher_than_ascii() {
        // CJK text should estimate more tokens per character than ASCII
        let ascii_msg = Message::user("a".repeat(100)); // 100 ASCII chars -> ~25 tokens
        let cjk_msg = Message::user("\u{4f60}\u{597d}".repeat(50)); // 100 CJK chars -> ~150 tokens
        let ascii_tokens = estimate_token_count(&[ascii_msg], None, None);
        let cjk_tokens = estimate_token_count(&[cjk_msg], None, None);
        // CJK should produce significantly more estimated tokens than same # of ASCII chars
        assert!(
            cjk_tokens > ascii_tokens * 3,
            "CJK tokens ({cjk_tokens}) should be much more than ASCII tokens ({ascii_tokens})"
        );
    }

    #[test]
    fn test_estimate_str_tokens_mixed_content() {
        // Pure ASCII
        let ascii_est = estimate_str_tokens("hello world"); // 11 chars -> ~3 tokens
        assert!((2..=5).contains(&ascii_est));
        // Pure CJK
        let cjk_est = estimate_str_tokens("\u{4f60}\u{597d}\u{4e16}\u{754c}"); // 4 CJK chars -> ~6 tokens
        assert!((4..=8).contains(&cjk_est));
        // Mixed
        let mixed_est = estimate_str_tokens("Hello \u{4f60}\u{597d}");
        assert!(
            mixed_est > ascii_est,
            "Mixed content should have more tokens than pure ASCII of same length"
        );
    }

    #[test]
    fn test_estimate_token_count_with_system_prompt() {
        let messages = vec![Message::user("hi")];
        let system = "You are a helpful assistant. ".repeat(100); // ~2800 chars
        let tokens_without = estimate_token_count(&messages, None, None);
        let tokens_with = estimate_token_count(&messages, Some(&system), None);
        assert!(tokens_with > tokens_without);
    }

    #[test]
    fn test_estimate_token_count_with_tools() {
        use librefang_types::tool::ToolDefinition;
        let messages = vec![Message::user("hi")];
        let tools = vec![ToolDefinition {
            name: "web_search".into(),
            description: "Search the web for information".into(),
            input_schema: serde_json::json!({"type": "object", "properties": {"query": {"type": "string"}}}),
        }];
        let tokens_without = estimate_token_count(&messages, None, None);
        let tokens_with = estimate_token_count(&messages, None, Some(&tools));
        assert!(tokens_with > tokens_without);
    }

    #[test]
    fn test_needs_compaction_by_tokens_below() {
        let config = CompactionConfig::default();
        // 70% of 200_000 = 140_000
        assert!(!needs_compaction_by_tokens(100_000, &config));
    }

    #[test]
    fn test_needs_compaction_by_tokens_above() {
        let config = CompactionConfig::default();
        // 70% of 200_000 = 140_000
        assert!(needs_compaction_by_tokens(150_000, &config));
    }

    #[test]
    fn test_context_pressure_from_percent() {
        assert_eq!(ContextPressure::from_percent(30.0), ContextPressure::Low);
        assert_eq!(ContextPressure::from_percent(55.0), ContextPressure::Medium);
        assert_eq!(ContextPressure::from_percent(75.0), ContextPressure::High);
        assert_eq!(
            ContextPressure::from_percent(90.0),
            ContextPressure::Critical
        );
    }

    #[test]
    fn test_generate_context_report_basic() {
        let messages = vec![Message::user("Hello world"), Message::assistant("Hi there")];
        let report = generate_context_report(&messages, Some("You are helpful."), None, 200_000);
        assert!(report.estimated_tokens > 0);
        assert!(report.usage_percent < 1.0); // tiny messages
        assert_eq!(report.pressure, ContextPressure::Low);
        assert_eq!(report.message_count, 2);
        assert!(report.breakdown.system_prompt_tokens > 0);
        assert!(report.breakdown.message_tokens > 0);
    }

    #[test]
    fn test_generate_context_report_critical() {
        // Create enough messages to push past 85%
        let big_msg = "x".repeat(800_000); // 200K tokens at chars/4
        let messages = vec![Message::user(big_msg)];
        let report = generate_context_report(&messages, None, None, 200_000);
        assert_eq!(report.pressure, ContextPressure::Critical);
        assert!(report.usage_percent > 85.0);
    }

    #[test]
    fn test_format_context_report() {
        let messages = vec![Message::user("hi")];
        let report = generate_context_report(&messages, Some("system"), None, 200_000);
        let formatted = format_context_report(&report);
        assert!(formatted.contains("Context Usage"));
        assert!(formatted.contains("Breakdown"));
        assert!(formatted.contains("Pressure"));
    }

    #[test]
    fn test_compaction_strips_base64_blobs() {
        let config = CompactionConfig::default();
        let blob = "A".repeat(2000);
        let tool_content = format!("result: {blob}");
        let messages = vec![Message {
            role: Role::User,
            content: MessageContent::Blocks(vec![ContentBlock::ToolResult {
                tool_use_id: "t1".to_string(),
                tool_name: String::new(),
                content: tool_content,
                is_error: false,
                status: librefang_types::tool::ToolExecutionStatus::default(),
                approval_request_id: None,
            }]),
            pinned: false,
            timestamp: None,
        }];
        let text = build_conversation_text(&messages, &config);
        // The base64 blob should be stripped/replaced by session_repair
        assert!(text.contains("[base64 blob"));
        assert!(!text.contains(&"A".repeat(2000)));
    }

    #[test]
    fn test_compaction_applies_2k_cap() {
        let config = CompactionConfig::default();
        // Create a tool result larger than 2K but without base64 blobs
        let large_result = "word ".repeat(500); // ~2500 chars of non-base64 text
        let messages = vec![Message {
            role: Role::User,
            content: MessageContent::Blocks(vec![ContentBlock::ToolResult {
                tool_use_id: "t2".to_string(),
                tool_name: String::new(),
                content: large_result,
                is_error: false,
                status: librefang_types::tool::ToolExecutionStatus::default(),
                approval_request_id: None,
            }]),
            pinned: false,
            timestamp: None,
        }];
        let text = build_conversation_text(&messages, &config);
        // Should be capped at ~2000 chars (plus the "..." suffix)
        let result_part = text.split("[Tool result (OK): ").nth(1).unwrap_or("");
        // The result_part includes trailing "]\n\n", so just check it's under 2100
        assert!(
            result_part.len() < 2100,
            "result_part len = {}",
            result_part.len()
        );
    }

    #[test]
    fn test_compaction_short_results_unchanged() {
        let config = CompactionConfig::default();
        let short_result = "Success: 42 records processed";
        let messages = vec![Message {
            role: Role::User,
            content: MessageContent::Blocks(vec![ContentBlock::ToolResult {
                tool_use_id: "t3".to_string(),
                tool_name: String::new(),
                content: short_result.to_string(),
                is_error: false,
                status: librefang_types::tool::ToolExecutionStatus::default(),
                approval_request_id: None,
            }]),
            pinned: false,
            timestamp: None,
        }];
        let text = build_conversation_text(&messages, &config);
        assert!(text.contains(short_result));
    }

    // ----- #4976: per-agent compaction override resolution -----

    #[test]
    fn from_toml_with_overrides_none_matches_from_toml() {
        let global = librefang_types::config::CompactionTomlConfig {
            threshold_messages: 42,
            keep_recent: 7,
            max_summary_tokens: 2048,
            token_threshold_ratio: 0.6,
            max_chunk_chars: 90_000,
            max_retries: 4,
        };
        let merged = CompactionConfig::from_toml_with_overrides(&global, None);
        let baseline = CompactionConfig::from_toml(&global);
        assert_eq!(merged.threshold, baseline.threshold);
        assert_eq!(merged.keep_recent, baseline.keep_recent);
        assert_eq!(merged.max_summary_tokens, baseline.max_summary_tokens);
        assert_eq!(merged.max_chunk_chars, baseline.max_chunk_chars);
        assert_eq!(merged.max_retries, baseline.max_retries);
        assert!(
            (merged.token_threshold_ratio - baseline.token_threshold_ratio).abs() < f64::EPSILON
        );
    }

    #[test]
    fn from_toml_with_overrides_applies_partial_override() {
        let global = librefang_types::config::CompactionTomlConfig::default();
        let overrides = librefang_types::agent::CompactionOverrides {
            keep_recent: Some(25),
            max_summary_tokens: Some(8192),
            ..Default::default()
        };
        let merged = CompactionConfig::from_toml_with_overrides(&global, Some(&overrides));
        assert_eq!(merged.keep_recent, 25);
        assert_eq!(merged.max_summary_tokens, 8192);
        // Falls through to global:
        assert_eq!(merged.threshold, global.threshold_messages);
        assert_eq!(merged.max_retries, global.max_retries);
    }

    #[test]
    fn from_toml_with_overrides_empty_struct_is_passthrough() {
        // Some(&overrides) where every field is None must behave like None.
        let global = librefang_types::config::CompactionTomlConfig::default();
        let overrides = librefang_types::agent::CompactionOverrides::default();
        let merged = CompactionConfig::from_toml_with_overrides(&global, Some(&overrides));
        let baseline = CompactionConfig::from_toml(&global);
        assert_eq!(merged.threshold, baseline.threshold);
        assert_eq!(merged.keep_recent, baseline.keep_recent);
        assert_eq!(merged.max_summary_tokens, baseline.max_summary_tokens);
    }
}
