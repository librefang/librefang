//! Translation from LibreFang `StreamEvent` to ACP `SessionUpdate`.
//!
//! The agent loop in `librefang-runtime` emits a flat stream of
//! [`librefang_llm_driver::StreamEvent`] values during one prompt turn.
//! ACP expects those events delivered as `session/update` notifications
//! whose payload is a [`agent_client_protocol::schema::v1::SessionUpdate`].
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
//!   disambiguation note to every result in a same-name group that
//!   reached ≥2 concurrent calls (#3313 review, PR-3). The editor
//!   user sees the wire-level attribution may be a guess and can
//!   verify against tool input args before relying on it. The
//!   misattribution still doesn't affect what runs or what the agent
//!   sees — it only colours the modal-↔-card mapping in the editor.

use std::collections::{HashMap, VecDeque};

use agent_client_protocol::schema::v1::{
    ContentBlock, ContentChunk, SessionUpdate, TextContent, ToolCall, ToolCallContent, ToolCallId,
    ToolCallStatus, ToolCallUpdate, ToolCallUpdateFields, ToolKind,
};
use librefang_llm_driver::StreamEvent;

/// Bound state retained for starts whose result has not arrived yet.
///
/// A normal turn stays far below this. The cap protects the turn translator
/// from a malformed stream that emits starts without results.
const MAX_TRACKED_TOOL_CALLS: usize = 1024;

#[derive(Debug, Default)]
struct InFlightCalls {
    ids: VecDeque<ToolCallId>,
    max_concurrent: usize,
}

/// Stateful translator. One per session/prompt turn.
#[derive(Debug, Default)]
pub(crate) struct EventTranslator {
    /// FIFO of in-flight tool call ids keyed by tool name. We use a queue
    /// per name so parallel calls of the same tool don't get conflated.
    in_flight_by_name: HashMap<String, InFlightCalls>,
    tracked_tool_calls: usize,
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

            // Owner notices are private side-channel messages for the agent owner's DM.
            // ACP has no owner-authenticated update channel.
            // Forwarding one as ordinary agent speech would disclose it to the editor session participant.
            StreamEvent::OwnerNotice { .. } => Vec::new(),

            StreamEvent::ToolUseStart { id, name } => {
                let tool_call_id = ToolCallId::new(id);
                let kind = infer_tool_kind(&name);
                if self.tracked_tool_calls < MAX_TRACKED_TOOL_CALLS {
                    let calls = self.in_flight_by_name.entry(name.clone()).or_default();
                    calls.ids.push_back(tool_call_id.clone());
                    calls.max_concurrent = calls.max_concurrent.max(calls.ids.len());
                    self.tracked_tool_calls += 1;
                } else {
                    tracing::warn!(
                        tool_name = %name,
                        max_tracked = MAX_TRACKED_TOOL_CALLS,
                        "ACP tool-call tracker is full; start will not be correlated to its result"
                    );
                }
                vec![SessionUpdate::ToolCall(
                    ToolCall::new(tool_call_id, name)
                        .kind(kind)
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
                // Pop the oldest in-flight call for this name. Preserve the
                // group's high-water mark so every result in a concurrently
                // ambiguous group carries the same warning, including the
                // final result after its siblings have already completed.
                let matched = self.in_flight_by_name.get_mut(&name).and_then(|calls| {
                    calls
                        .ids
                        .pop_front()
                        .map(|id| (id, calls.max_concurrent, calls.ids.is_empty()))
                });
                let (tool_call_id, max_concurrent, drained, is_orphan) = match matched {
                    Some((id, max_concurrent, drained)) => {
                        debug_assert!(self.tracked_tool_calls > 0);
                        self.tracked_tool_calls -= 1;
                        (id, max_concurrent, drained, false)
                    }
                    None => {
                        // A translator is recreated for every prompt, while ACP
                        // tool-call ids live in the longer session timeline. A
                        // per-translator counter would therefore repeat across
                        // turns and let a client attach a new orphan result to
                        // an old card.
                        let id =
                            ToolCallId::new(format!("librefang-orphan-{}", uuid::Uuid::new_v4()));
                        tracing::warn!(
                            tool_name = %name,
                            tool_call_id = %id,
                            "ACP received a tool result without a matching start"
                        );
                        (id, 1, false, true)
                    }
                };
                // Reap the outer entry once its queue drains. Without
                // this the `(name, empty queue)` pair lingers for the
                // life of the translator (this turn / ACP session) and
                // grows with the count of distinct tool names invoked —
                // a per-session leak (#5144).
                if drained {
                    self.in_flight_by_name.remove(&name);
                }
                let status = if is_error {
                    ToolCallStatus::Failed
                } else {
                    ToolCallStatus::Completed
                };
                let payload = if max_concurrent > 1 {
                    format!(
                        "[note: this tool had {max_concurrent} concurrent calls to `{name}`; the runtime does \
                         not yet correlate results back to a specific tool_use_id, so this result may be \
                         attributed to a sibling call. Verify against the tool's input arguments before relying \
                         on the attribution.]\n\n{result_preview}"
                    )
                } else {
                    result_preview
                };
                let mut updates = Vec::with_capacity(if is_orphan { 2 } else { 1 });
                if is_orphan {
                    // ACP clients expect a ToolCall before its ToolCallUpdate.
                    // Create the missing card rather than emitting an update
                    // for an identifier the client has never observed.
                    updates.push(SessionUpdate::ToolCall(
                        ToolCall::new(tool_call_id.clone(), name.clone())
                            .kind(infer_tool_kind(&name))
                            .status(ToolCallStatus::InProgress),
                    ));
                }
                updates.push(SessionUpdate::ToolCallUpdate(ToolCallUpdate::new(
                    tool_call_id,
                    ToolCallUpdateFields::new().status(status).content(vec![
                        ToolCallContent::from(ContentBlock::Text(TextContent::new(payload))),
                    ]),
                )));
                updates
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

/// Conservative mapping from a LibreFang tool name to an ACP `ToolKind`.
///
/// We err on the side of `Other` so unknown tools still render with a neutral
/// icon. Matching uses complete separator-delimited action words. Generic
/// editing is restricted to known file/document targets so names such as
/// `edit_history` are not assigned a misleading file-edit affordance.
pub(crate) fn infer_tool_kind(name: &str) -> ToolKind {
    let lower = name.to_ascii_lowercase();
    let words: Vec<&str> = lower
        .split(|character: char| !character.is_ascii_alphanumeric())
        .filter(|word| !word.is_empty())
        .collect();
    let has = |word: &str| words.contains(&word);
    let has_edit_target = ["file", "files", "document", "documents", "text", "content"]
        .iter()
        .any(|target| has(target));
    let is_run_action = has("run")
        && ["command", "script", "process"]
            .iter()
            .any(|target| has(target));
    let is_http_fetch = has("http") && ["get", "request", "fetch"].iter().any(|action| has(action));

    if has("fetch") || is_http_fetch {
        ToolKind::Fetch
    } else if has("read") || has("get") || has("list") || has("cat") || has("ls") {
        ToolKind::Read
    } else if has("write") || has("patch") || (has("edit") && has_edit_target) {
        ToolKind::Edit
    } else if has("delete") || has("remove") || has("rm") {
        ToolKind::Delete
    } else if has("move") || has("rename") {
        ToolKind::Move
    } else if has("search") || has("grep") || has("find") || has("glob") {
        ToolKind::Search
    } else if has("bash") || has("exec") || has("execute") || has("shell") || is_run_action {
        ToolKind::Execute
    } else if has("think") || has("plan") {
        ToolKind::Think
    } else {
        ToolKind::Other
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use librefang_types::message::{StopReason as LfStopReason, TokenUsage};

    fn tool_call_id(update: &ToolCallUpdate) -> String {
        update.tool_call_id.to_string()
    }

    fn tool_call_status(update: &ToolCallUpdate) -> Option<ToolCallStatus> {
        update.fields.status
    }

    fn tool_call_text(update: &ToolCallUpdate) -> &str {
        let content = update.fields.content.as_ref().expect("content set");
        match &content[0] {
            ToolCallContent::Content(content) => match &content.content {
                ContentBlock::Text(text) => &text.text,
                _ => panic!("expected text content"),
            },
            _ => panic!("expected ToolCallContent::Content"),
        }
    }

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
    fn owner_notice_is_not_forwarded_to_the_acp_session() {
        let mut t = EventTranslator::new();
        let out = t.translate(StreamEvent::OwnerNotice {
            text: "owner-private policy notice".into(),
        });
        assert!(out.is_empty());
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
                assert_eq!(tool_call_status(u), Some(ToolCallStatus::Completed));
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
            SessionUpdate::ToolCallUpdate(u) => assert_eq!(tool_call_id(u), "a"),
            _ => panic!(),
        }
        let r2 = t.translate(StreamEvent::ToolExecutionResult {
            name: "fetch".into(),
            result_preview: "second".into(),
            is_error: true,
        });
        match &r2[0] {
            SessionUpdate::ToolCallUpdate(u) => {
                assert_eq!(tool_call_id(u), "b");
                assert_eq!(tool_call_status(u), Some(ToolCallStatus::Failed));
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
    /// but every result from a group that reached two or more pending
    /// calls carries a disambiguation note so the editor user knows
    /// the attribution is a guess.
    #[test]
    fn every_parallel_same_named_result_carries_ambiguity_note() {
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
                let text = tool_call_text(u);
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
        // After the first pop only one is pending, but this result is still
        // part of the same ambiguous concurrent group and must be annotated.
        let r2 = t.translate(StreamEvent::ToolExecutionResult {
            name: "fetch".into(),
            result_preview: "second body".into(),
            is_error: false,
        });
        match &r2[0] {
            SessionUpdate::ToolCallUpdate(u) => {
                let text = tool_call_text(u);
                assert!(
                    text.contains("concurrent calls to `fetch`"),
                    "last sibling must retain the ambiguity note"
                );
                assert!(text.contains("second body"));
            }
            _ => panic!(),
        }
    }

    #[test]
    fn orphan_result_emits_a_synthetic_call_before_its_update() {
        let mut translator = EventTranslator::new();
        let updates = translator.translate(StreamEvent::ToolExecutionResult {
            name: "web_fetch".into(),
            result_preview: "orphan body".into(),
            is_error: false,
        });

        assert_eq!(updates.len(), 2);
        let synthetic_id = match &updates[0] {
            SessionUpdate::ToolCall(call) => call.tool_call_id.to_string(),
            _ => panic!("expected synthetic ToolCall first"),
        };
        match &updates[1] {
            SessionUpdate::ToolCallUpdate(update) => {
                assert_eq!(tool_call_id(update), synthetic_id);
                assert_eq!(tool_call_status(update), Some(ToolCallStatus::Completed));
                assert_eq!(tool_call_text(update), "orphan body");
            }
            _ => panic!("expected ToolCallUpdate second"),
        }

        let next = translator.translate(StreamEvent::ToolExecutionResult {
            name: "web_fetch".into(),
            result_preview: "another orphan".into(),
            is_error: true,
        });
        let next_id = match &next[0] {
            SessionUpdate::ToolCall(call) => call.tool_call_id.to_string(),
            _ => panic!("expected synthetic ToolCall first"),
        };
        assert_ne!(synthetic_id, next_id, "synthetic ids must not collide");

        // EventTranslator is recreated for every prompt. IDs must remain
        // unique across those instances because the ACP client keeps the
        // session timeline from earlier prompts.
        let mut next_prompt_translator = EventTranslator::new();
        let next_prompt = next_prompt_translator.translate(StreamEvent::ToolExecutionResult {
            name: "web_fetch".into(),
            result_preview: "later prompt orphan".into(),
            is_error: false,
        });
        let next_prompt_id = match &next_prompt[0] {
            SessionUpdate::ToolCall(call) => call.tool_call_id.to_string(),
            _ => panic!("expected synthetic ToolCall first"),
        };
        assert_ne!(
            synthetic_id, next_prompt_id,
            "synthetic ids must not repeat across prompt translators"
        );
    }

    #[test]
    fn tool_kind_matching_uses_complete_action_words() {
        for (name, expected) in [
            ("mcp_filesystem_read_file", ToolKind::Read),
            ("get_weather", ToolKind::Read),
            ("file_edit", ToolKind::Edit),
            ("apply_patch", ToolKind::Edit),
            ("file_delete", ToolKind::Delete),
            ("skill_evolve_remove_file", ToolKind::Delete),
            ("rename_file", ToolKind::Move),
            ("memory_search", ToolKind::Search),
            ("shell_exec", ToolKind::Execute),
            ("execute_command", ToolKind::Execute),
            ("plan", ToolKind::Think),
            ("web_fetch", ToolKind::Fetch),
            ("http_get", ToolKind::Fetch),
        ] {
            assert_eq!(
                infer_tool_kind(name),
                expected,
                "unexpected kind for {name}"
            );
        }

        for name in [
            "edit_history",
            "planet_tool",
            "findings_aggregator",
            "deletegate",
            "refind",
            "shellfish",
            "run_id",
            "http_server",
        ] {
            assert_eq!(
                infer_tool_kind(name),
                ToolKind::Other,
                "unknown tool {name} must keep the neutral kind"
            );
        }
    }

    #[test]
    fn unmatched_tool_starts_are_bounded() {
        let mut translator = EventTranslator::new();
        for index in 0..=MAX_TRACKED_TOOL_CALLS {
            let _ = translator.translate(StreamEvent::ToolUseStart {
                id: format!("id-{index}"),
                name: format!("tool-{index}"),
            });
        }

        assert_eq!(translator.tracked_tool_calls, MAX_TRACKED_TOOL_CALLS);
        assert_eq!(translator.in_flight_by_name.len(), MAX_TRACKED_TOOL_CALLS);

        let overflow_result = translator.translate(StreamEvent::ToolExecutionResult {
            name: format!("tool-{MAX_TRACKED_TOOL_CALLS}"),
            result_preview: "done".into(),
            is_error: false,
        });
        assert!(matches!(overflow_result[0], SessionUpdate::ToolCall(_)));
        assert!(matches!(
            overflow_result[1],
            SessionUpdate::ToolCallUpdate(_)
        ));
    }

    /// Regression (#5144): once a tool name's in-flight queue drains,
    /// the outer `in_flight_by_name` entry must be removed, not left as
    /// a `(name, empty-VecDeque)` pair. Before the fix the map grew with
    /// the count of distinct tool names invoked in the session/turn even
    /// though every call had completed.
    #[test]
    fn drained_tool_queue_is_reaped_from_map() {
        let mut t = EventTranslator::new();

        // Two distinct tool names, each a single start→result round-trip.
        for name in ["read_file", "write_file"] {
            let _ = t.translate(StreamEvent::ToolUseStart {
                id: format!("{name}-1"),
                name: name.to_string(),
            });
            // While in flight the entry exists.
            assert!(
                t.in_flight_by_name.contains_key(name),
                "in-flight entry must exist for {name}"
            );
            let _ = t.translate(StreamEvent::ToolExecutionResult {
                name: name.to_string(),
                result_preview: "done".into(),
                is_error: false,
            });
            // After the result drains the queue the entry must be gone.
            assert!(
                !t.in_flight_by_name.contains_key(name),
                "drained entry for {name} must be reaped"
            );
        }

        assert!(
            t.in_flight_by_name.is_empty(),
            "map must be empty after all tool calls completed, got {} entries",
            t.in_flight_by_name.len()
        );
    }

    /// A still-pending sibling call must NOT cause premature reaping:
    /// the entry survives until its queue is fully drained.
    #[test]
    fn map_entry_survives_while_siblings_pending() {
        let mut t = EventTranslator::new();
        let _ = t.translate(StreamEvent::ToolUseStart {
            id: "a".into(),
            name: "fetch".into(),
        });
        let _ = t.translate(StreamEvent::ToolUseStart {
            id: "b".into(),
            name: "fetch".into(),
        });
        // First result pops "a" but "b" is still pending → keep entry.
        let _ = t.translate(StreamEvent::ToolExecutionResult {
            name: "fetch".into(),
            result_preview: "first".into(),
            is_error: false,
        });
        assert!(
            t.in_flight_by_name.contains_key("fetch"),
            "entry must survive while a sibling call is still in flight"
        );
        // Second result drains the queue → entry reaped.
        let _ = t.translate(StreamEvent::ToolExecutionResult {
            name: "fetch".into(),
            result_preview: "second".into(),
            is_error: false,
        });
        assert!(
            !t.in_flight_by_name.contains_key("fetch"),
            "entry must be reaped once all siblings have completed"
        );
    }
}
