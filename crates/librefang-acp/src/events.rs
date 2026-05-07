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
//!   `name → FIFO of in-progress tool_call_ids` and pops the front on
//!   each result. This matches hermes's approach (see `acp_adapter/events.py`).
//!
//!   **Known limitation:** when multiple same-named calls are in
//!   flight and complete out of start-order, the FIFO pop attributes
//!   the first finished result to the first started call regardless
//!   of which one actually finished. The proper fix lives in
//!   `librefang-runtime`'s `StreamEvent::ToolExecutionResult` — it
//!   needs to carry the originating tool-use id (cross-crate change,
//!   tracked as a follow-up). Until that lands, the pump prepends a
//!   disambiguation note to the result content whenever ≥2 calls are
//!   in flight for the same name (#3313 review, PR-3). The editor
//!   user sees the wire-level attribution may be a guess and can
//!   verify against tool input args before relying on it. The
//!   misattribution still doesn't affect what runs or what the agent
//!   sees — it only colours the modal-↔-card mapping in the editor.

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
                let queue = self.in_flight_by_name.get_mut(&name);
                // Snapshot the queue depth *before* popping so we can
                // tell whether more than one call is in flight for this
                // tool name. When >1 are pending the FIFO pop is a
                // best-effort guess (the runtime can't yet tell us
                // which call this result came from — see crate-level
                // doc for the limitation), so we annotate the wire
                // payload to surface that ambiguity instead of
                // confidently attributing the result to the wrong
                // tool-call card. (#3313 review, PR-3)
                let pending_before = queue.as_ref().map(|q| q.len()).unwrap_or(0);
                let tool_call_id = queue
                    .and_then(|q| q.pop_front())
                    .unwrap_or_else(|| ToolCallId::new(format!("orphan-{name}")));
                let status = if is_error {
                    ToolCallStatus::Failed
                } else {
                    ToolCallStatus::Completed
                };
                let payload = if pending_before > 1 {
                    format!(
                        "[note: {pending_before} concurrent calls to `{name}` are in flight; the runtime does \
                         not yet correlate results back to a specific tool_use_id, so this result may be \
                         attributed to a sibling call. Verify against the tool's input arguments before relying \
                         on the attribution.]\n\n{result_preview}"
                    )
                } else {
                    result_preview
                };
                vec![SessionUpdate::ToolCallUpdate(ToolCallUpdate::new(
                    tool_call_id,
                    ToolCallUpdateFields::new().status(status).content(vec![
                        ToolCallContent::from(ContentBlock::Text(TextContent::new(payload))),
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
pub(crate) fn infer_tool_kind(name: &str) -> ToolKind {
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

    /// Without `ToolExecutionResult.id` from the runtime we cannot
    /// correlate the result back to a specific tool_use_id when
    /// multiple parallel same-named calls are in flight (see
    /// crate-level docs). PR-3 (#3313 review) takes the
    /// best-available middle ground: the result still ends up on the
    /// front-of-queue card (so the FIFO test above still passes),
    /// but the wire payload carries a disambiguation note when two
    /// or more calls are pending so the editor user knows the
    /// attribution is a guess.
    #[test]
    fn parallel_same_named_first_result_carries_ambiguity_note() {
        let mut t = EventTranslator::new();
        let _ = t.translate(StreamEvent::ToolUseStart {
            id: "a".into(),
            name: "fetch".into(),
        });
        let _ = t.translate(StreamEvent::ToolUseStart {
            id: "b".into(),
            name: "fetch".into(),
        });
        let r1 = t.translate(StreamEvent::ToolExecutionResult {
            name: "fetch".into(),
            result_preview: "the body".into(),
            is_error: false,
        });
        // 2 pending before pop — the result carries the
        // disambiguation note prepended.
        match &r1[0] {
            SessionUpdate::ToolCallUpdate(u) => {
                let content = u.fields.content.as_ref().expect("content set");
                let text = match &content[0] {
                    ToolCallContent::Content(c) => match &c.content {
                        ContentBlock::Text(t) => t.text.clone(),
                        _ => panic!("expected text content"),
                    },
                    _ => panic!("expected ToolCallContent::Content"),
                };
                assert!(
                    text.contains("concurrent calls to `fetch`"),
                    "expected ambiguity note, got: {text}"
                );
                assert!(
                    text.contains("the body"),
                    "original preview must still be present"
                );
            }
            _ => panic!(),
        }
        // After the first pop only one is pending — second result
        // is unambiguous, no note.
        let r2 = t.translate(StreamEvent::ToolExecutionResult {
            name: "fetch".into(),
            result_preview: "second body".into(),
            is_error: false,
        });
        match &r2[0] {
            SessionUpdate::ToolCallUpdate(u) => {
                let content = u.fields.content.as_ref().expect("content set");
                let text = match &content[0] {
                    ToolCallContent::Content(c) => match &c.content {
                        ContentBlock::Text(t) => t.text.clone(),
                        _ => panic!("expected text content"),
                    },
                    _ => panic!("expected ToolCallContent::Content"),
                };
                assert!(
                    !text.contains("concurrent calls"),
                    "single-pending result must not carry ambiguity note"
                );
                assert_eq!(text, "second body");
            }
            _ => panic!(),
        }
    }
}
