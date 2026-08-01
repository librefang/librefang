//! Flattened, text-only "advisory view" of a conversation.
//!
//! Advisor models in a Mixture-of-Agents turn are deliberately *blind*: they
//! see a lossy, text-only rendering of the conversation and cannot call tools.
//! This module builds that rendering from the live `CompletionRequest` message
//! history.
//!
//! Rules (spec §4):
//! 1. Drop all system messages.
//! 2. Extract text only; images and other binary blocks are skipped.
//! 3. User turns with no text but non-text blocks render a placeholder; empty
//!    user turns are dropped.
//! 4. Assistant turns keep their text and render each tool call as
//!    `[called tool: name(args)]`.
//! 5. Tool results fold into the preceding assistant turn as
//!    `[tool result: …]` with a head+tail preview.
//! 6. The view always ends on a user turn (a constant synthetic marker is
//!    appended when needed — constant so the advisory prompt stays
//!    cache-stable).
//! 7. If nothing rendered, fall back to the latest real user message.
//!
//! Output contains only `Role::User` / `Role::Assistant` messages with
//! `MessageContent::Text` and zero tool blocks.

use librefang_types::message::{ContentBlock, Message, MessageContent, Role};

/// Placeholder for a user turn that carried only non-text content.
const NON_TEXT_PLACEHOLDER: &str = "[user sent non-text content]";

/// Character cap for a folded tool-result preview (head + tail).
const TOOL_RESULT_CAP: usize = 4000;

/// Character cap for a rendered tool-call argument preview.
const TOOL_ARGS_CAP: usize = 200;

/// Constant marker appended so the advisory view always ends on a user turn.
/// Kept verbatim-stable so provider prompt caches stay warm across turns.
pub const END_ON_USER_MARKER: &str = "[The conversation above is the current state of the task. Give your most intelligent judgement: what is going on, what should happen next, what risks or mistakes you see, and how the acting agent should proceed.]";

/// Build the flattened advisory view from a conversation history.
pub fn build_advisory_view(messages: &[Message]) -> Vec<Message> {
    let mut out: Vec<Message> = Vec::new();

    for msg in messages {
        match msg.role {
            Role::System => continue,
            Role::User => {
                if let Some(text) = render_user(msg) {
                    out.push(Message::user(text));
                }
            }
            Role::Assistant => {
                let rendered = render_assistant(msg);
                if !rendered.is_empty() {
                    out.push(Message::assistant(rendered));
                }
                // Fold tool results into the preceding assistant turn.
                for result in tool_results(msg) {
                    let preview = head_tail_preview(&result, TOOL_RESULT_CAP);
                    append_to_last_assistant(&mut out, &format!("[tool result: {preview}]"));
                }
            }
        }
    }

    // Rule 7: nothing rendered → fall back to the latest real user message.
    if out.is_empty() {
        if let Some(last_user) = messages.iter().rev().find(|m| m.role == Role::User) {
            let text = last_user.content.text_content();
            let text = if text.trim().is_empty() {
                NON_TEXT_PLACEHOLDER.to_string()
            } else {
                text
            };
            out.push(Message::user(text));
        }
    }

    // Rule 6: end-on-user invariant.
    let ends_with_user = out.last().is_some_and(|m| m.role == Role::User);
    if !ends_with_user {
        out.push(Message::user(END_ON_USER_MARKER));
    }

    out
}

/// Render a user turn to text, or `None` if it should be dropped.
fn render_user(msg: &Message) -> Option<String> {
    let text = msg.content.text_content();
    if !text.trim().is_empty() {
        return Some(text);
    }
    if has_non_text_block(&msg.content) {
        return Some(NON_TEXT_PLACEHOLDER.to_string());
    }
    None
}

/// Render an assistant turn's text and tool calls (not tool results).
fn render_assistant(msg: &Message) -> String {
    let mut parts: Vec<String> = Vec::new();
    let text = msg.content.text_content();
    if !text.trim().is_empty() {
        parts.push(text);
    }
    for block in blocks(&msg.content) {
        if let ContentBlock::ToolUse { name, input, .. } = block {
            parts.push(render_tool_use(name, input));
        }
    }
    parts.join("\n")
}

/// Render a single tool call as `[called tool: name(args)]`.
fn render_tool_use(name: &str, input: &serde_json::Value) -> String {
    let args = tool_args_preview(input);
    if args.is_empty() {
        format!("[called tool: {name}]")
    } else {
        format!("[called tool: {name}({args})]")
    }
}

/// Compact, truncated JSON preview of a tool call's arguments.
fn tool_args_preview(input: &serde_json::Value) -> String {
    match input {
        serde_json::Value::Null => String::new(),
        serde_json::Value::Object(map) if map.is_empty() => String::new(),
        _ => {
            let s = serde_json::to_string(input).unwrap_or_default();
            truncate_chars(&s, TOOL_ARGS_CAP)
        }
    }
}

/// Collect the text of every tool result in a message.
fn tool_results(msg: &Message) -> Vec<String> {
    let mut out = Vec::new();
    for block in blocks(&msg.content) {
        if let ContentBlock::ToolResult {
            content, is_error, ..
        } = block
        {
            let text = content.clone();
            if *is_error {
                out.push(format!("[error] {text}"));
            } else {
                out.push(text);
            }
        }
    }
    out
}

/// Append a line to the last assistant message, or create one if absent.
fn append_to_last_assistant(out: &mut Vec<Message>, line: &str) {
    if let Some(last) = out.last_mut() {
        if last.role == Role::Assistant {
            if let MessageContent::Text(text) = &mut last.content {
                if !text.is_empty() {
                    text.push('\n');
                }
                text.push_str(line);
                return;
            }
        }
    }
    out.push(Message::assistant(line.to_string()));
}

/// Truncate to `max` chars, appending an ellipsis when cut.
fn truncate_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max).collect();
    out.push('…');
    out
}

/// Head + tail preview of a long string, noting the omitted middle.
///
/// Keeps roughly the first and last third of `max` chars so both the start
/// (headers, command echo) and the end (final output, error tail) survive.
fn head_tail_preview(s: &str, max: usize) -> String {
    let total = s.chars().count();
    if total <= max {
        return s.to_string();
    }
    let head = max * 2 / 3;
    let tail = max - head;
    let omitted = total - head - tail;
    let head_part: String = s.chars().take(head).collect();
    let tail_part: String = s.chars().skip(total - tail).collect();
    format!("{head_part}\n… [{omitted} chars omitted] …\n{tail_part}")
}

/// The structured blocks of a message (empty slice for plain text).
fn blocks(content: &MessageContent) -> &[ContentBlock] {
    match content {
        MessageContent::Blocks(b) => b.as_slice(),
        MessageContent::Text(_) => &[],
    }
}

/// Whether a message carries any non-text block (image, tool use, …).
fn has_non_text_block(content: &MessageContent) -> bool {
    blocks(content)
        .iter()
        .any(|b| !matches!(b, ContentBlock::Text { .. }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use librefang_types::tool::ToolExecutionStatus;

    #[test]
    fn drops_system_messages() {
        let msgs = vec![Message::system("you are helpful"), Message::user("hello")];
        let view = build_advisory_view(&msgs);
        assert!(view.iter().all(|m| m.role != Role::System));
        assert_eq!(view[0].content.text_content(), "hello");
    }

    #[test]
    fn renders_tool_calls_without_parens_when_no_args() {
        let msgs = vec![Message {
            role: Role::Assistant,
            content: MessageContent::Blocks(vec![
                ContentBlock::Text {
                    text: "Let me check.".into(),
                    provider_metadata: None,
                },
                ContentBlock::ToolUse {
                    id: "t1".into(),
                    name: "list_files".into(),
                    input: serde_json::json!({}),
                    provider_metadata: None,
                },
            ]),
            pinned: false,
            timestamp: None,
        }];
        let view = build_advisory_view(&msgs);
        let text = view[0].content.text_content();
        assert!(text.contains("Let me check."));
        assert!(text.contains("[called tool: list_files]"));
        // Ends on the synthetic user marker.
        assert_eq!(
            view.last().unwrap().content.text_content(),
            END_ON_USER_MARKER
        );
    }

    #[test]
    fn renders_tool_calls_with_args() {
        let msgs = vec![Message {
            role: Role::Assistant,
            content: MessageContent::Blocks(vec![ContentBlock::ToolUse {
                id: "t1".into(),
                name: "read".into(),
                input: serde_json::json!({"path": "src/main.rs"}),
                provider_metadata: None,
            }]),
            pinned: false,
            timestamp: None,
        }];
        let view = build_advisory_view(&msgs);
        let text = view[0].content.text_content();
        assert!(text.contains(r#"[called tool: read({"path":"src/main.rs"})]"#));
    }

    #[test]
    fn folds_tool_result_into_preceding_assistant() {
        let msgs = vec![
            Message {
                role: Role::Assistant,
                content: MessageContent::Blocks(vec![ContentBlock::ToolUse {
                    id: "t1".into(),
                    name: "read".into(),
                    input: serde_json::json!({"path": "a.rs"}),
                    provider_metadata: None,
                }]),
                pinned: false,
                timestamp: None,
            },
            Message {
                role: Role::Assistant,
                content: MessageContent::Blocks(vec![ContentBlock::ToolResult {
                    tool_use_id: "t1".into(),
                    tool_name: "read".into(),
                    content: "file contents here".into(),
                    is_error: false,
                    status: ToolExecutionStatus::Completed,
                    approval_request_id: None,
                }]),
                pinned: false,
                timestamp: None,
            },
        ];
        let view = build_advisory_view(&msgs);
        // Both the tool call and its result land in a single assistant turn.
        assert_eq!(view.len(), 2); // assistant + synthetic user
        let text = view[0].content.text_content();
        assert!(text.contains("[called tool: read"));
        assert!(text.contains("[tool result: file contents here]"));
    }

    #[test]
    fn creates_assistant_for_orphan_tool_result() {
        let msgs = vec![Message {
            role: Role::Assistant,
            content: MessageContent::Blocks(vec![ContentBlock::ToolResult {
                tool_use_id: "t1".into(),
                tool_name: "read".into(),
                content: "orphan result".into(),
                is_error: false,
                status: ToolExecutionStatus::Completed,
                approval_request_id: None,
            }]),
            pinned: false,
            timestamp: None,
        }];
        let view = build_advisory_view(&msgs);
        assert_eq!(view[0].role, Role::Assistant);
        assert!(view[0]
            .content
            .text_content()
            .contains("[tool result: orphan result]"));
    }

    #[test]
    fn appends_end_on_user_marker() {
        let msgs = vec![Message::user("do the thing"), Message::assistant("done")];
        let view = build_advisory_view(&msgs);
        assert_eq!(view.last().unwrap().role, Role::User);
        assert_eq!(
            view.last().unwrap().content.text_content(),
            END_ON_USER_MARKER
        );
    }

    #[test]
    fn no_marker_when_already_ending_on_user() {
        let msgs = vec![Message::assistant("thinking"), Message::user("now do this")];
        let view = build_advisory_view(&msgs);
        assert_eq!(view.last().unwrap().content.text_content(), "now do this");
    }

    #[test]
    fn empty_input_falls_back_to_nothing_but_marker() {
        let view = build_advisory_view(&[]);
        assert_eq!(view.len(), 1);
        assert_eq!(view[0].content.text_content(), END_ON_USER_MARKER);
    }

    #[test]
    fn falls_back_to_latest_user_when_only_system() {
        let msgs = vec![
            Message::system("sys"),
            Message::user("real question"),
            Message::system("sys2"),
        ];
        // System messages drop; "real question" renders, so no fallback needed.
        let view = build_advisory_view(&msgs);
        assert_eq!(view[0].content.text_content(), "real question");
    }

    #[test]
    fn image_only_user_turn_gets_placeholder() {
        let msgs = vec![Message::user_with_blocks(vec![ContentBlock::Image {
            media_type: "image/png".into(),
            data: "AAAA".into(),
        }])];
        let view = build_advisory_view(&msgs);
        assert_eq!(view[0].content.text_content(), NON_TEXT_PLACEHOLDER);
    }

    #[test]
    fn genuinely_empty_user_turn_dropped() {
        let msgs = vec![Message::user(""), Message::user("actual content")];
        let view = build_advisory_view(&msgs);
        // Empty user dropped; "actual content" is the only real turn and ends
        // on user, so no synthetic marker.
        assert_eq!(view.len(), 1);
        assert_eq!(view[0].content.text_content(), "actual content");
    }

    #[test]
    fn head_tail_preview_truncates_large_results() {
        let big = "x".repeat(10_000);
        let preview = head_tail_preview(&big, 4000);
        assert!(preview.contains("chars omitted"));
        assert!(preview.chars().count() < 10_000);
    }
}
