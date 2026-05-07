//! `session/prompt` request handler.
//!
//! Drives a single prompt turn end-to-end:
//!
//! 1. Look up the ACP session and resolve to a LibreFang `SessionId`.
//! 2. Concatenate the prompt's text content blocks (Phase 1 only
//!    supports text — image/audio/embedded resource content needs the
//!    `prompt_capabilities` flags we don't advertise yet).
//! 3. Call [`AcpKernel::send_prompt`] to start a streaming agent turn.
//! 4. Race the event channel against the per-session cancel token,
//!    translating each [`StreamEvent`] into one or more
//!    `session/update` notifications.
//! 5. When the channel closes (or cancel fires), resolve the
//!    [`librefang_types::message::StopReason`] last seen on
//!    [`StreamEvent::ContentComplete`] and return a `PromptResponse`
//!    to the editor.

use std::sync::Arc;

use agent_client_protocol::schema::{
    ContentBlock, ContentChunk, PromptRequest, PromptResponse, SessionNotification, SessionUpdate,
    StopReason, TextContent,
};
use agent_client_protocol::Client;
use agent_client_protocol::ConnectionTo;
use agent_client_protocol::Responder;
use librefang_llm_driver::StreamEvent;
use librefang_types::agent::AgentId;
use librefang_types::message::StopReason as LfStopReason;

use crate::events::EventTranslator;
use crate::session::SessionStore;
use crate::AcpKernel;

/// Handle a single ACP `session/prompt` request. Pumps streaming events
/// to the client over the lifetime of the call, then returns a
/// `PromptResponse`.
pub(crate) async fn handle<K: AcpKernel>(
    kernel: Arc<K>,
    sessions: Arc<SessionStore>,
    agent_id: AgentId,
    req: PromptRequest,
    responder: Responder<PromptResponse>,
    cx: ConnectionTo<Client>,
) -> Result<(), agent_client_protocol::Error> {
    let Some(state) = sessions.get(&req.session_id) else {
        return responder.respond_with_error(agent_client_protocol::Error::invalid_params().data(
            serde_json::json!({
                "reason": "unknown session id",
                "session_id": req.session_id.0.as_ref(),
            }),
        ));
    };

    let (message, dropped) = concat_text_blocks(&req.prompt);
    // Surface dropped multimodal blocks to the user so they aren't
    // left guessing where their attachment went. The build's
    // `PromptCapabilities` already declares image / audio /
    // embedded_context as unsupported, but defensive editors may
    // still send them — be loud, not silent.
    if dropped > 0 {
        let warning = format!(
            "[note: dropped {dropped} non-text content block{} — \
             image / audio / embedded resource not yet supported by \
             this LibreFang build]\n\n",
            if dropped == 1 { "" } else { "s" }
        );
        cx.send_notification(SessionNotification::new(
            req.session_id.clone(),
            SessionUpdate::AgentMessageChunk(ContentChunk::new(ContentBlock::Text(
                TextContent::new(warning),
            ))),
        ))?;
    }
    if message.is_empty() {
        // Nothing to send — return immediately with an end-turn so the
        // editor doesn't spin on an empty user message.
        return responder.respond(PromptResponse::new(StopReason::EndTurn));
    }

    let mut events = match kernel
        .send_prompt(agent_id, message, state.librefang_session_id)
        .await
    {
        Ok(rx) => rx,
        Err(e) => return responder.respond_with_error(e.into_acp_error()),
    };

    let mut translator = EventTranslator::new();
    let session_id = req.session_id.clone();
    let cancel = state.cancel.clone();
    let mut last_stop_reason: Option<LfStopReason> = None;

    'pump: loop {
        tokio::select! {
            biased;

            _ = cancel.cancelled() => {
                last_stop_reason = Some(LfStopReason::EndTurn);
                // Drop the receiver so the kernel-side sender notices
                // and tears down the agent loop. The actual stop_reason
                // we *return* is `Cancelled` — see the mapping below.
                drop(events);
                break 'pump;
            }
            ev = events.recv() => match ev {
                Some(StreamEvent::ContentComplete { stop_reason, .. }) => {
                    last_stop_reason = Some(stop_reason);
                }
                Some(other) => {
                    for update in translator.translate(other) {
                        cx.send_notification(SessionNotification::new(
                            session_id.clone(),
                            update,
                        ))?;
                    }
                }
                None => break 'pump,
            }
        }
    }

    let stop = if cancel.is_cancelled() {
        StopReason::Cancelled
    } else {
        map_stop_reason(last_stop_reason.unwrap_or(LfStopReason::EndTurn))
    };

    responder.respond(PromptResponse::new(stop))
}

/// Concatenate the text body of a prompt and count any non-text
/// content blocks (image / audio / embedded resource) the build
/// can't yet pipe to the agent loop. The caller surfaces a
/// `dropped > 0` count to the user via a `session/update` message
/// chunk so they're not left wondering why their attachment
/// vanished.
fn concat_text_blocks(blocks: &[ContentBlock]) -> (String, usize) {
    let mut out = String::new();
    let mut dropped = 0usize;
    for block in blocks {
        match block {
            ContentBlock::Text(tc) => {
                if !out.is_empty() && !out.ends_with(char::is_whitespace) {
                    out.push(' ');
                }
                out.push_str(&tc.text);
            }
            // Anything non-text — image / audio / embedded resource /
            // future block types — counts as dropped. The runtime
            // doesn't yet plumb these through to the LLM driver, and
            // we already declared `image: false` / `audio: false` /
            // `embedded_context: false` in `PromptCapabilities` so
            // conformant editors won't send them.
            _ => dropped += 1,
        }
    }
    (out, dropped)
}

fn map_stop_reason(reason: LfStopReason) -> StopReason {
    match reason {
        LfStopReason::EndTurn => StopReason::EndTurn,
        LfStopReason::MaxTokens => StopReason::MaxTokens,
        // ToolUse / StopSequence are not user-visible end states in ACP's
        // model — the agent is mid-turn or just hit a stop string. We
        // surface these as `EndTurn` so the editor lets the user reply.
        LfStopReason::ToolUse | LfStopReason::StopSequence => StopReason::EndTurn,
        // Provider safety filter — surface explicitly so editor UIs can
        // distinguish a refused response from a successful completion.
        LfStopReason::ContentFiltered => StopReason::Refusal,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_client_protocol::schema::TextContent;

    #[test]
    fn concat_text_inserts_separator_between_blocks() {
        let blocks = vec![
            ContentBlock::Text(TextContent::new("hello")),
            ContentBlock::Text(TextContent::new("world")),
        ];
        let (text, dropped) = concat_text_blocks(&blocks);
        assert_eq!(text, "hello world");
        assert_eq!(dropped, 0);
    }

    #[test]
    fn concat_text_preserves_existing_whitespace() {
        let blocks = vec![
            ContentBlock::Text(TextContent::new("hello ")),
            ContentBlock::Text(TextContent::new("world")),
        ];
        let (text, dropped) = concat_text_blocks(&blocks);
        assert_eq!(text, "hello world");
        assert_eq!(dropped, 0);
    }

    #[test]
    fn concat_text_empty_input_returns_empty() {
        let (text, dropped) = concat_text_blocks(&[]);
        assert_eq!(text, "");
        assert_eq!(dropped, 0);
    }

    #[test]
    fn concat_text_only_text_block_no_drops() {
        let blocks = vec![ContentBlock::Text(TextContent::new("only text"))];
        let (text, dropped) = concat_text_blocks(&blocks);
        assert_eq!(text, "only text");
        assert_eq!(dropped, 0);
    }

    #[test]
    fn map_stop_reason_passes_through_known_variants() {
        assert!(matches!(
            map_stop_reason(LfStopReason::EndTurn),
            StopReason::EndTurn
        ));
        assert!(matches!(
            map_stop_reason(LfStopReason::MaxTokens),
            StopReason::MaxTokens
        ));
    }
}
