//! Message-shape helpers used across the agent loop:
//!
//! - bounded text accumulator for the empty-response fallback buffer
//! - leak / no-reply / soft-error / parameter-error classifiers
//! - memory-persistence sanitizer that strips channel envelopes
//! - safe message-history trimming on conversation-turn boundaries
//! - image-data stripping on already-processed turns
//! - token-usage accumulator
//! - tool-result content sanitization (injection markers, dynamic truncation)
//! - bounded sender-label sanitizer for `[name]:` prefixes

use crate::context_budget::{truncate_tool_result_dynamic, ContextBudget};
use crate::silent_response::{ENVELOPE_LINE_PREFIXES, ENVELOPE_STANDALONE_MARKERS};
use crate::workspace_sandbox::{ERR_PATH_TRAVERSAL, ERR_SANDBOX_ESCAPE};
use librefang_types::message::{Message, Role, TokenUsage};
use tracing::{info, warn};

/// Hard cap on the in-memory `accumulated_text` buffer used as a fallback for
/// the empty-response guard.
///
/// Each agent loop turn may push intermediate text emitted alongside
/// `tool_use` blocks into this buffer. Across many iterations (autonomous
/// agents, retry loops, long-running tasks) the buffer can grow unbounded,
/// pinning megabytes of heap per active session. 64 KiB is far above any
/// reasonable user-facing message (~10× a Slack message limit) while still
/// being orders of magnitude below problematic memory pressure.
///
/// Once the cap is reached the buffer is sealed: subsequent appends short-
/// circuit and a single `warn!` is emitted on the transition so the log
/// isn't spammed. The existing buffered prefix is preserved so the empty-
/// response fallback still has something useful to surface.
pub(super) const ACCUMULATED_TEXT_MAX_BYTES: usize = 64 * 1024;

/// Append `intermediate_text` to `accumulated_text`, bounded by
/// `ACCUMULATED_TEXT_MAX_BYTES`. See the constant's doc-comment for rationale.
pub(super) fn push_accumulated_text(accumulated_text: &mut String, intermediate_text: &str) {
    // Buffer already sealed on a prior call.
    if accumulated_text.len() >= ACCUMULATED_TEXT_MAX_BYTES {
        return;
    }
    let separator = if accumulated_text.is_empty() {
        ""
    } else {
        "\n\n"
    };
    let projected = accumulated_text.len() + separator.len() + intermediate_text.len();
    if projected > ACCUMULATED_TEXT_MAX_BYTES {
        warn!(
            current_bytes = accumulated_text.len(),
            incoming_bytes = intermediate_text.len(),
            cap_bytes = ACCUMULATED_TEXT_MAX_BYTES,
            "accumulated_text fallback buffer cap reached; further intermediate \
             text for this loop will be dropped (existing buffer is preserved \
             for the empty-response fallback)"
        );
        // Seal the buffer with ASCII padding so future calls trip the early
        // return above. ASCII is always UTF-8 boundary-safe.
        let sentinel = " [accumulated_text capped]";
        if accumulated_text.len() + sentinel.len() <= ACCUMULATED_TEXT_MAX_BYTES {
            accumulated_text.push_str(sentinel);
        }
        let pad = ACCUMULATED_TEXT_MAX_BYTES.saturating_sub(accumulated_text.len());
        if pad > 0 {
            accumulated_text.reserve(pad);
            for _ in 0..pad {
                accumulated_text.push(' ');
            }
        }
        return;
    }
    accumulated_text.push_str(separator);
    accumulated_text.push_str(intermediate_text);
}

/// Thin delegate to the canonical `silent_response::is_silent_response`. Kept
/// the single source of truth for `NO_REPLY` / `[no reply needed]` /
/// `no reply needed` detection (case-insensitive, punctuation- and
/// emoji-tolerant). See `crates/librefang-runtime/src/silent_response.rs`.
pub(super) fn is_no_reply(text: &str) -> bool {
    crate::silent_response::is_silent_response(text)
}

/// Classify a response as a progress-text leak: a short ellipsis-terminated
/// acknowledgment the model sometimes emits *before* a tool call that the
/// turn ended without ever producing.
///
/// Observed in production when Claude Code / Qwen Code hit an internal limit
/// after emitting a verbal preamble (e.g. `"Waiting for the script to
/// complete..."`, `"Let me check that..."`) without the corresponding
/// `tool_use` block. Without this guard the runtime delivers the preamble
/// to the channel as the agent's final reply, which reads as nonsense to
/// the user (cron-triggered ynab report surfaced on Telegram as the
/// literal string `"Waiting for the script to complete..."`).
///
/// Heuristic is intentionally narrow to avoid swallowing legitimate replies:
/// - Trimmed length ≤ 120 chars (progress preambles are short)
/// - Ends with `...` or `…`. Two-dot `..` is intentionally excluded —
///   models almost never emit it deliberately, and skipping it avoids
///   clipping truncated abbreviations like `"See p.."`.
pub(super) fn is_progress_text_leak(text: &str) -> bool {
    let t = text.trim();
    if t.is_empty() || t.chars().count() > 120 {
        return false;
    }
    t.ends_with("...") || t.ends_with("…")
}

/// Strip channel-envelope prefixes from a user-message or agent-response
/// string before persisting it as long-term episodic memory.
///
/// Channel adapters wrap inbound text with session-context envelopes that
/// disambiguate the current turn but become toxic prompt scaffolding when
/// persisted verbatim: on recall the model sees `User asked: [Group
/// message from X]\n…\nI responded: …` (a mirror of training-data turn
/// frames) and dumps the literal bullet back into the next chat reply.
///
/// Drops any line that, after `trim_start`, begins with one of
/// `ENVELOPE_LINE_PREFIXES` or whose trimmed body equals one of
/// `ENVELOPE_STANDALONE_MARKERS`. Inline bracketed content is preserved.
/// Returns `None` when nothing meaningful remains — callers MUST skip
/// persistence to avoid a `[Past exchange]\nThem: \nYou: …` row that itself
/// trips the cascade-leak guard.
///
/// Scope: WhatsApp-gateway envelope shapes only. Matrix/Telegram/Signal
/// adapters will need their prefixes added to the const arrays here when
/// they start persisting through this path.
pub(super) fn sanitize_for_memory(text: &str) -> Option<String> {
    // Fast path: a text with no `[` cannot match any envelope prefix.
    // Trim once and return without splitting into lines.
    if !text.contains('[') {
        let trimmed = text.trim();
        return if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        };
    }
    let mut out = String::with_capacity(text.len());
    for line in text.split_inclusive('\n') {
        let trimmed_start = line.trim_start();
        let line_body = trimmed_start.trim_end();
        let is_envelope = ENVELOPE_LINE_PREFIXES
            .iter()
            .any(|p| trimmed_start.starts_with(p))
            || ENVELOPE_STANDALONE_MARKERS.contains(&line_body);
        if !is_envelope {
            out.push_str(line);
        }
    }
    // In-place trim to avoid a second allocation on the slow path.
    out.truncate(out.trim_end().len());
    let leading_ws = out.len() - out.trim_start().len();
    out.drain(..leading_ws);
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

/// Thin delegate to the canonical cascade-leak detector in `silent_response`.
/// See `crates/librefang-runtime/src/silent_response.rs` for full docs,
/// marker lists, and trip-condition rationale.
pub(super) fn is_cascade_leak(text: &str) -> bool {
    crate::silent_response::is_cascade_leak(text)
}

/// Returns true if this tool-error content is a "soft" error — one the LLM is
/// expected to recover from cheaply on the next iteration (approval denials,
/// sandbox rejections, modify-and-retry hints, argument-truncation nudges).
/// Hard errors (unrecognized tool, network failure, etc.) are caller's problem.
///
/// Prefer `ToolExecutionStatus::is_soft_error()` where the status is available.
/// This content-based fallback covers legacy paths and sandbox string errors that
/// don't yet carry a typed status.
pub(super) fn is_soft_error_content(content: &str) -> bool {
    content.contains(ERR_PATH_TRAVERSAL)
        || content.contains(ERR_SANDBOX_ESCAPE)
        || content.contains("arguments were truncated")
        || is_parameter_error_content(content)
}

/// Detect tool errors that are caused by the LLM sending wrong/missing parameters.
/// These are soft errors because the LLM can self-correct by retrying with different
/// input — they should NOT count toward the consecutive-failure abort threshold.
pub(super) fn is_parameter_error_content(content: &str) -> bool {
    let lower = content.to_ascii_lowercase();
    lower.contains("missing '") || // "Missing 'path' parameter"
    lower.contains("missing parameter") ||
    lower.contains("required parameter") ||
    lower.contains("invalid parameter") ||
    lower.contains("parameter is required") ||
    lower.contains("argument is required")
}

/// Safely trim message history to `DEFAULT_MAX_HISTORY_MESSAGES`, cutting at
/// conversation-turn boundaries so ToolUse/ToolResult pairs are never split.
///
/// Both the LLM working copy (`messages`) and the canonical session store
/// (`session_messages`) are trimmed so that the truncated history is
/// persisted on the next `save_session_async` call — preventing unbounded
/// growth of the on-disk session blob.
///
/// After trim + repair, if fewer than 2 messages survive the function
/// synthesises a minimal `[user_message]` so the LLM always gets at least
/// the current question.
pub(super) fn safe_trim_messages(
    messages: &mut Vec<Message>,
    session_messages: &mut Vec<Message>,
    agent_name: &str,
    user_message: &str,
    max_history: usize,
) -> (bool, bool) {
    let mut working_mutated = false;
    let mut session_mutated = false;

    // Trim the persistent session messages first so the truncated version is
    // saved back to the database, preventing reload-OOM on next boot.
    if session_messages.len() > max_history {
        let desired = session_messages.len() - max_history;
        let trim_point = crate::session_repair::find_safe_trim_point(session_messages, desired)
            .filter(|&p| p > 0)
            .unwrap_or(desired);

        let rescued: Vec<Message> = session_messages[..trim_point]
            .iter()
            .filter(|m| m.pinned)
            .cloned()
            .collect();

        info!(
            agent = %agent_name,
            total_messages = session_messages.len(),
            trimming = trim_point,
            rescued_pinned = rescued.len(),
            "Trimming persistent session messages"
        );

        session_messages.drain(..trim_point);
        session_mutated = true;

        for (i, msg) in rescued.into_iter().enumerate() {
            session_messages.insert(i, msg);
        }
    }

    if messages.len() <= max_history {
        return (working_mutated, session_mutated);
    }

    working_mutated = true;

    let desired_trim = messages.len() - max_history;

    // Find a trim point that does not split ToolUse/ToolResult pairs.
    // Filter out 0 — drain(..0) is a no-op and would leave messages untrimmed.
    let trim_point = crate::session_repair::find_safe_trim_point(messages, desired_trim)
        .filter(|&p| p > 0)
        .unwrap_or(desired_trim);

    // Rescue pinned messages (delegation results) from the drain range so they
    // survive history trim. Without this, agent_send results from earlier in the
    // conversation are silently dropped, causing the LLM to redo delegated work.
    let rescued: Vec<Message> = messages[..trim_point]
        .iter()
        .filter(|m| m.pinned)
        .cloned()
        .collect();

    warn!(
        agent = %agent_name,
        total_messages = messages.len(),
        trimming = trim_point,
        rescued_pinned = rescued.len(),
        desired = desired_trim,
        "Trimming old messages at safe turn boundary"
    );

    messages.drain(..trim_point);

    // Re-insert rescued pinned messages at the beginning of the remaining history.
    for (i, msg) in rescued.into_iter().enumerate() {
        messages.insert(i, msg);
    }

    // Re-validate after trim.
    *messages = crate::session_repair::validate_and_repair(messages);
    // Ensure history starts with a user turn: trimming may have left an
    // assistant turn at position 0, which strict providers (e.g. Gemini)
    // reject with INVALID_ARGUMENT on function-call turns.
    *messages = crate::session_repair::ensure_starts_with_user(std::mem::take(messages));

    // Post-trim safety: ensure at least a user message survives so the LLM
    // request body is never empty.
    if messages.len() < 2 || !messages.iter().any(|m| m.role == Role::User) {
        warn!(
            agent = %agent_name,
            remaining = messages.len(),
            "Trim + repair left too few messages, synthesizing minimal conversation"
        );
        // Keep any surviving system message, then append the current user turn.
        let system_msgs: Vec<Message> = messages
            .drain(..)
            .filter(|m| m.role == Role::System)
            .collect();
        *messages = system_msgs;
        messages.push(Message::user(user_message));
    }

    (working_mutated, session_mutated)
}

/// Strip base64 data from image blocks in session messages that the LLM has
/// already processed, replacing them with lightweight text placeholders.
///
/// Each image block (~56K tokens of base64) is replaced with a small text
/// note so the conversation context is preserved without token bloat.
pub(super) fn strip_processed_image_data(messages: &mut [Message]) -> bool {
    let mut mutated = false;

    for msg in messages.iter_mut() {
        mutated |= msg.content.strip_images();
    }

    mutated
}

pub(super) fn accumulate_token_usage(total_usage: &mut TokenUsage, usage: &TokenUsage) {
    total_usage.input_tokens += usage.input_tokens;
    total_usage.output_tokens += usage.output_tokens;
    total_usage.cache_creation_input_tokens += usage.cache_creation_input_tokens;
    total_usage.cache_read_input_tokens += usage.cache_read_input_tokens;
}

/// Strip base64 data from image blocks in messages older than the last user
/// turn that contains images.
///
/// Called *before* the LLM call to proactively clean stale images from
/// previous turns (e.g. images that survived a crash or session reload).
/// The last user message is preserved so the LLM can see any freshly
/// attached image on the current turn.
pub(super) fn strip_prior_image_data(messages: &mut [Message]) -> bool {
    // Find the index of the last user message
    let last_user_idx = messages
        .iter()
        .rposition(|m| m.role == Role::User && m.content.has_images());

    let mut mutated = false;

    for (i, msg) in messages.iter_mut().enumerate() {
        // Skip the last user message that contains images — it hasn't been
        // processed by the LLM yet.
        if Some(i) == last_user_idx {
            continue;
        }
        mutated |= msg.content.strip_images();
    }

    mutated
}

/// Sanitize tool result content: strip injection markers, then truncate.
///
/// When a `context_engine` is provided, truncation is delegated to the engine
/// so plugins can customize the strategy. Otherwise falls back to the built-in
/// head+tail truncation.
pub(super) fn sanitize_tool_result_content(
    content: &str,
    context_budget: &ContextBudget,
    context_engine: Option<&dyn crate::context_engine::ContextEngine>,
    context_window_tokens: usize,
) -> String {
    let stripped = crate::session_repair::strip_tool_result_details(content);
    if let Some(engine) = context_engine {
        engine.truncate_tool_result(&stripped, context_window_tokens)
    } else {
        truncate_tool_result_dynamic(&stripped, context_budget)
    }
}

/// Sanitize a group-chat sender label so it can be safely embedded in a `[name]:` prefix.
///
/// Removes characters that could be used to spoof other senders or break out of the prefix
/// format (brackets, colons, newlines, control chars), collapses whitespace, and truncates
/// to a bounded length.
pub(super) fn sanitize_sender_label(name: &str) -> String {
    const MAX_LEN: usize = 64;
    let mut out = String::with_capacity(name.len().min(MAX_LEN));
    let mut last_space = false;
    for ch in name.chars() {
        let sanitized = match ch {
            '[' | ']' | ':' | '\n' | '\r' | '\t' => ' ',
            c if c.is_control() => ' ',
            c => c,
        };
        if sanitized == ' ' {
            if last_space || out.is_empty() {
                continue;
            }
            last_space = true;
        } else {
            last_space = false;
        }
        out.push(sanitized);
        if out.chars().count() >= MAX_LEN {
            break;
        }
    }
    let trimmed = out.trim().to_string();
    if trimmed.is_empty() {
        "user".to_string()
    } else {
        trimmed
    }
}
