// Wired into `server.rs::handle_prompt` in the next milestone of #3313;
// the workspace `warnings = "deny"` lint would otherwise reject the
// scaffolding-only state of this PR.
#![allow(dead_code)]

//! Translation from LibreFang `StreamEvent` to ACP `SessionUpdate`.
//!
//! The agent loop in `librefang-runtime` emits a flat stream of
//! [`librefang_llm_driver::StreamEvent`] values during one prompt turn.
//! ACP expects those events delivered as `session/update` notifications
//! whose payload is a [`agent_client_protocol::schema::SessionUpdate`].
//!
//! We do the translation here. State is needed across events because:
//!
//! * `ToolUseStart` opens a tool call with no input yet; subsequent
//!   `ToolInputDelta` chunks accumulate the JSON; `ToolUseEnd` finalises
//!   it. ACP wants the input attached to the *first* `ToolCall` update,
//!   so we either delay emission until `ToolUseEnd` or emit a `ToolCall`
//!   skeleton on `ToolUseStart` and follow up with `ToolCallUpdate`s.
//!   We pick the latter — clients can render a "running" tool indicator
//!   immediately, which matches Zed's UX.
//!
//! * `ToolExecutionResult` carries no `id`, only `name`. The pump tracks
//!   `name → most-recent in-progress tool_call_id` and applies updates by
//!   that mapping. This matches hermes's approach (see `acp_adapter/events.py`).

use std::collections::{HashMap, VecDeque};

use agent_client_protocol::schema::{
    ContentBlock, ContentChunk, SessionUpdate, TextContent, ToolCall, ToolCallContent, ToolCallId,
    ToolCallStatus, ToolCallUpdate, ToolCallUpdateFields, ToolKind,
};
use librefang_llm_driver::StreamEvent;

/// Stateful translator. One per session/prompt turn.
#[derive(Debug, Default)]
pub(crate) struct EventTranslator {
    /// FIFO of in-flight tool call ids keyed by tool name. We use a queue
    /// per name so parallel calls of the same tool don't get conflated.
    in_flight_by_name: HashMap<String, VecDeque<ToolCallId>>,
}

impl EventTranslator {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Translate a single `StreamEvent` into zero or more `SessionUpdate`s.
    ///
    /// Some events (notably `ContentComplete`) carry no on-wire update —
    /// they're consumed by the pump to decide on `PromptResponse.stop_reason`.
    /// In those cases this returns an empty `Vec`.
    pub(crate) fn translate(&mut self, ev: StreamEvent) -> Vec<SessionUpdate> {
        match ev {
            StreamEvent::TextDelta { text } => {
                vec![SessionUpdate::AgentMessageChunk(ContentChunk::new(
                    ContentBlock::Text(TextContent::new(text)),
                ))]
            }

            StreamEvent::ThinkingDelta { text } => {
                vec![SessionUpdate::AgentThoughtChunk(ContentChunk::new(
                    ContentBlock::Text(TextContent::new(text)),
                ))]
            }

            StreamEvent::OwnerNotice { text } => {
                // Surface owner-private notices as a regular agent
                // message chunk for now. Phase 2 may give them their own
                // visual treatment when a dedicated SessionUpdate variant
                // exists.
                vec![SessionUpdate::AgentMessageChunk(ContentChunk::new(
                    ContentBlock::Text(TextContent::new(text)),
                ))]
            }

            StreamEvent::ToolUseStart { id, name } => {
                let tool_call_id = ToolCallId::new(id);
                self.in_flight_by_name
                    .entry(name.clone())
                    .or_default()
                    .push_back(tool_call_id.clone());
                vec![SessionUpdate::ToolCall(
                    ToolCall::new(tool_call_id, name.clone())
                        .kind(infer_tool_kind(&name))
                        .status(ToolCallStatus::Pending),
                )]
            }

            // We do not push a `session/update` for every JSON character —
            // it would generate hundreds of tiny notifications. ACP clients
            // don't need raw input streamed; they get the final value via
            // the `ToolCallUpdate` that follows `ToolUseEnd`.
            StreamEvent::ToolInputDelta { text: _ } => Vec::new(),

            StreamEvent::ToolUseEnd { id, name: _, input } => {
                let tool_call_id = ToolCallId::new(id);
                vec![SessionUpdate::ToolCallUpdate(ToolCallUpdate::new(
                    tool_call_id,
                    ToolCallUpdateFields::new()
                        .status(ToolCallStatus::InProgress)
                        .raw_input(input),
                ))]
            }

            StreamEvent::ToolExecutionResult {
                name,
                result_preview,
                is_error,
            } => {
                // Pop the oldest in-flight call for this name. If we don't
                // have one (mismatched event ordering), fall back to a
                // synthetic id so the update is still well-formed.
                let tool_call_id = self
                    .in_flight_by_name
                    .get_mut(&name)
                    .and_then(|q| q.pop_front())
                    .unwrap_or_else(|| ToolCallId::new(format!("orphan-{name}")));
                let status = if is_error {
                    ToolCallStatus::Failed
                } else {
                    ToolCallStatus::Completed
                };
                vec![SessionUpdate::ToolCallUpdate(ToolCallUpdate::new(
                    tool_call_id,
                    ToolCallUpdateFields::new().status(status).content(vec![
                        ToolCallContent::from(ContentBlock::Text(TextContent::new(result_preview))),
                    ]),
                ))]
            }

            // `ContentComplete` and `PhaseChange` are signalling events
            // for the pump, not for the wire. The pump reads `ContentComplete`
            // to know which `StopReason` to put on the `PromptResponse`.
            StreamEvent::ContentComplete { .. } | StreamEvent::PhaseChange { .. } => Vec::new(),

            // `StreamEvent` is `#[non_exhaustive]` upstream — newly added
            // variants land here as no-op until we map them in a follow-up.
            _ => Vec::new(),
        }
    }
}

/// Best-effort mapping from a LibreFang tool name to an ACP `ToolKind`.
///
/// We err on the side of `Other` so unknown tools still render with a neutral
/// icon. The categories we recognise are the ones that have established
/// names across LibreFang's stdlib (`read_*`, `write_*`, `bash`, etc.).
fn infer_tool_kind(name: &str) -> ToolKind {
    let lower = name.to_ascii_lowercase();
    if lower.starts_with("read") || lower.contains("get_") || lower.contains("list_") {
        ToolKind::Read
    } else if lower.starts_with("write") || lower.contains("edit") || lower.contains("patch") {
        ToolKind::Edit
    } else if lower.starts_with("delete") || lower.starts_with("rm_") {
        ToolKind::Delete
    } else if lower.starts_with("move") || lower.starts_with("rename") {
        ToolKind::Move
    } else if lower.contains("search") || lower.contains("grep") || lower.contains("find") {
        ToolKind::Search
    } else if lower == "bash"
        || lower.starts_with("exec")
        || lower.starts_with("run_")
        || lower.contains("shell")
    {
        ToolKind::Execute
    } else if lower.contains("think") || lower.contains("plan") {
        ToolKind::Think
    } else if lower.contains("fetch") || lower.starts_with("http_") {
        ToolKind::Fetch
    } else {
        ToolKind::Other
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use librefang_types::message::{StopReason as LfStopReason, TokenUsage};

    #[test]
    fn text_delta_becomes_agent_message_chunk() {
        let mut t = EventTranslator::new();
        let out = t.translate(StreamEvent::TextDelta {
            text: "hello".into(),
        });
        assert_eq!(out.len(), 1);
        match &out[0] {
            SessionUpdate::AgentMessageChunk(chunk) => match &chunk.content {
                ContentBlock::Text(tc) => assert_eq!(tc.text, "hello"),
                _ => panic!("expected text content"),
            },
            _ => panic!("expected AgentMessageChunk"),
        }
    }

    #[test]
    fn thinking_delta_becomes_thought_chunk() {
        let mut t = EventTranslator::new();
        let out = t.translate(StreamEvent::ThinkingDelta {
            text: "reasoning".into(),
        });
        assert!(matches!(out[0], SessionUpdate::AgentThoughtChunk(_)));
    }

    #[test]
    fn tool_lifecycle_emits_call_then_update() {
        let mut t = EventTranslator::new();
        let start = t.translate(StreamEvent::ToolUseStart {
            id: "tc-1".into(),
            name: "bash".into(),
        });
        assert!(matches!(start[0], SessionUpdate::ToolCall(_)));

        // Input deltas suppressed.
        assert!(t
            .translate(StreamEvent::ToolInputDelta { text: "{".into() })
            .is_empty());

        let end = t.translate(StreamEvent::ToolUseEnd {
            id: "tc-1".into(),
            name: "bash".into(),
            input: serde_json::json!({"command":"ls"}),
        });
        assert!(matches!(end[0], SessionUpdate::ToolCallUpdate(_)));

        let result = t.translate(StreamEvent::ToolExecutionResult {
            name: "bash".into(),
            result_preview: "ok".into(),
            is_error: false,
        });
        match &result[0] {
            SessionUpdate::ToolCallUpdate(u) => {
                assert_eq!(u.fields.status, Some(ToolCallStatus::Completed));
            }
            _ => panic!("expected ToolCallUpdate"),
        }
    }

    #[test]
    fn content_complete_yields_no_wire_update() {
        let mut t = EventTranslator::new();
        let out = t.translate(StreamEvent::ContentComplete {
            stop_reason: LfStopReason::EndTurn,
            usage: TokenUsage::default(),
        });
        assert!(out.is_empty());
    }

    #[test]
    fn parallel_same_named_tool_calls_use_fifo() {
        let mut t = EventTranslator::new();
        let _ = t.translate(StreamEvent::ToolUseStart {
            id: "a".into(),
            name: "fetch".into(),
        });
        let _ = t.translate(StreamEvent::ToolUseStart {
            id: "b".into(),
            name: "fetch".into(),
        });
        // First result corresponds to first start (id "a").
        let r1 = t.translate(StreamEvent::ToolExecutionResult {
            name: "fetch".into(),
            result_preview: "first".into(),
            is_error: false,
        });
        match &r1[0] {
            SessionUpdate::ToolCallUpdate(u) => assert_eq!(u.tool_call_id.0.as_ref(), "a"),
            _ => panic!(),
        }
        let r2 = t.translate(StreamEvent::ToolExecutionResult {
            name: "fetch".into(),
            result_preview: "second".into(),
            is_error: true,
        });
        match &r2[0] {
            SessionUpdate::ToolCallUpdate(u) => {
                assert_eq!(u.tool_call_id.0.as_ref(), "b");
                assert_eq!(u.fields.status, Some(ToolCallStatus::Failed));
            }
            _ => panic!(),
        }
    }
}
