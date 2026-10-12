//! `session/prompt` request handler.
//!
//! Drives a single prompt turn end-to-end:
//!
//! 1. Look up the ACP session and resolve to a LibreFang `SessionId`.
//! 2. Concatenate the prompt's text content blocks (Phase 1 only
//!    supports text — image/audio/embedded resource content needs the
//!    `prompt_capabilities` flags we don't advertise yet).
//! 3. Call [`AcpKernel::send_prompt`] to start a streaming agent turn.
//! 4. Race the event channel against this turn's cancel token,
//!    translating each [`StreamEvent`] into one or more
//!    `session/update` notifications.
//! 5. Wait for the event channel to close or for cancellation.
//! 6. Return the [`librefang_types::message::StopReason`] from the last [`StreamEvent::ContentComplete`].
//! 7. Report a channel close without [`StreamEvent::ContentComplete`] as an internal error.

use std::sync::Arc;

use agent_client_protocol::schema::v1::{
    ContentBlock, ContentChunk, PromptRequest, PromptResponse, SessionNotification, SessionUpdate,
    StopReason, TextContent,
};
use agent_client_protocol::Client;
use agent_client_protocol::ConnectionTo;
use agent_client_protocol::Responder;
use librefang_llm_driver::StreamEvent;
use librefang_types::agent::AgentId;
use librefang_types::message::StopReason as LfStopReason;

use tokio_util::sync::CancellationToken;

use crate::events::EventTranslator;
use crate::session::{SessionState, SessionStore};
use crate::AcpKernel;

/// Handle a single ACP `session/prompt` request: start the turn, then pump its streaming events to the client from a task spawned on the connection, which sends the `PromptResponse` when the turn ends.
///
/// Request callbacks run inside the connection's dispatch loop, which processes no other message until the callback returns.
/// Pumping the turn inline therefore held back every message the editor sent during it: `session/cancel` was only seen after the turn had finished, and the responses to the agent's own `fs/*`, `terminal/*` and permission requests could not be routed until their reverse-RPC timed out.
/// The session lookup and the per-turn cancel token stay on the dispatch loop, so a `session/cancel` the editor sends right after the prompt is always ordered after `begin_turn` and cancels this turn.
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
    // A fresh per-turn token: a `session/cancel` aimed at an earlier turn (or sent while idle) must not cancel this one.
    let cancel = state.begin_turn();

    cx.spawn(run_turn(
        kernel,
        state,
        cancel,
        agent_id,
        req,
        responder,
        cx.clone(),
    ))
}

/// Run one prompt turn to completion off the dispatch loop and answer the `session/prompt` request.
async fn run_turn<K: AcpKernel>(
    kernel: Arc<K>,
    state: Arc<SessionState>,
    cancel: CancellationToken,
    agent_id: AgentId,
    req: PromptRequest,
    responder: Responder<PromptResponse>,
    cx: ConnectionTo<Client>,
) -> Result<(), agent_client_protocol::Error> {
    let (message, converted) = concat_text_blocks(&req.prompt);
    // Surface converted multimodal blocks so the user knows the
    // attachment landed but didn't reach the LLM verbatim — the
    // agent only saw a bracketed placeholder. True multimodal
    // support is tracked as a separate epic in `librefang-llm-drivers`.
    if converted > 0 {
        let warning = format!(
            "[note: {converted} non-text content block{} converted to \
             text placeholder{} — true multimodal pipeline (image / \
             audio bytes reaching the LLM) is tracked as a separate \
             follow-up]\n\n",
            if converted == 1 { "" } else { "s" },
            if converted == 1 { "" } else { "s" }
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
    if cancel.is_cancelled() {
        // Cancelled before the kernel turn started: don't persist the message or bill the LLM.
        return responder.respond(PromptResponse::new(StopReason::Cancelled));
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
    let mut last_stop_reason: Option<LfStopReason> = None;

    'pump: loop {
        tokio::select! {
            biased;

            _ = cancel.cancelled() => {
                // The streaming loop treats a dropped receiver as a non-fatal disconnect and keeps running, so stop the kernel-side run explicitly before dropping it.
                // The actual stop_reason we *return* is `Cancelled` — see the mapping below.
                kernel
                    .cancel_prompt(agent_id, state.librefang_session_id)
                    .await;
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
    } else if let Some(reason) = last_stop_reason {
        map_stop_reason(reason)
    } else {
        return responder.respond_with_error(
            crate::AcpError::internal("prompt event channel closed before ContentComplete")
                .into_acp_error(),
        );
    };

    responder.respond(PromptResponse::new(stop))
}

/// Fold a prompt's content blocks into a single text body. Non-text
/// blocks (image / audio / resource link / embedded resource) are
/// converted to bracketed placeholders inline with the text so the
/// agent at least sees *that* an attachment was sent, even though
/// the build doesn't yet plumb the binary payload through to the
/// LLM driver.
///
/// Returns `(text, converted)` — `converted` is the count of
/// non-text blocks that were folded in as placeholders. The caller
/// surfaces a non-zero count via a `session/update` notice so the
/// user knows the attachment landed but didn't reach the LLM
/// verbatim.
///
/// True multimodal — image / audio bytes actually reaching the
/// LLM — is tracked as an independent epic in `librefang-llm-drivers`
/// because it requires a `ContentBlock::Image` variant on
/// `librefang_types::message` plus per-driver wire-format support
/// (Anthropic inline base64, OpenAI image_url, Gemini base64, …).
fn concat_text_blocks(blocks: &[ContentBlock]) -> (String, usize) {
    let mut out = String::new();
    let mut converted = 0usize;
    for block in blocks {
        let placeholder = match block {
            ContentBlock::Text(tc) => {
                if !out.is_empty() && !out.ends_with(char::is_whitespace) {
                    out.push(' ');
                }
                out.push_str(&tc.text);
                continue;
            }
            // Don't include the base64-encoded size in the placeholder
            // — leaking the byte length to the LLM is a small but
            // unnecessary signal (a prompt-injection probe could use
            // it to sniff what was attached). Mime-type alone is
            // enough for the agent to know to ask the user to paste
            // the relevant text.
            ContentBlock::Image(img) => {
                format!("[image attachment: {}]", img.mime_type)
            }
            ContentBlock::Audio(aud) => {
                format!("[audio attachment: {}]", aud.mime_type)
            }
            ContentBlock::ResourceLink(rl) => {
                format!("[resource link: {}]", rl.uri)
            }
            ContentBlock::Resource(_) => "[embedded resource]".to_string(),
            // Future ACP content variants — render as a generic
            // placeholder so the agent still sees that *something*
            // was sent.
            _ => "[unsupported content block]".to_string(),
        };
        if !out.is_empty() && !out.ends_with(char::is_whitespace) {
            out.push('\n');
        }
        out.push_str(&placeholder);
        converted += 1;
    }
    (out, converted)
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
    use agent_client_protocol::schema::v1::TextContent;

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
    fn concat_text_only_text_block_no_conversions() {
        let blocks = vec![ContentBlock::Text(TextContent::new("only text"))];
        let (text, converted) = concat_text_blocks(&blocks);
        assert_eq!(text, "only text");
        assert_eq!(converted, 0);
    }

    #[test]
    fn concat_text_image_block_converts_to_placeholder() {
        use agent_client_protocol::schema::v1::ImageContent;
        let blocks = vec![
            ContentBlock::Text(TextContent::new("look at this:")),
            ContentBlock::Image(ImageContent::new("AAAA", "image/png")),
        ];
        let (text, converted) = concat_text_blocks(&blocks);
        assert_eq!(converted, 1);
        assert!(text.starts_with("look at this:"));
        assert!(
            text.contains("[image attachment: image/png]"),
            "image placeholder missing in {text:?}"
        );
        // Don't leak base64 size — the placeholder must not contain
        // the raw byte count.
        assert!(
            !text.contains("bytes"),
            "placeholder should not include byte count: {text:?}"
        );
    }

    #[test]
    fn concat_text_audio_block_converts_to_placeholder() {
        use agent_client_protocol::schema::v1::AudioContent;
        let blocks = vec![ContentBlock::Audio(AudioContent::new(
            "BBBBCCCC",
            "audio/wav",
        ))];
        let (text, converted) = concat_text_blocks(&blocks);
        assert_eq!(converted, 1);
        assert!(
            text.contains("[audio attachment: audio/wav]"),
            "audio placeholder missing in {text:?}"
        );
        assert!(
            !text.contains("bytes"),
            "placeholder should not include byte count: {text:?}"
        );
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
