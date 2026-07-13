//! OpenAI-compatible `/v1/chat/completions` API endpoint.
//!
//! Allows any OpenAI-compatible client library to talk to LibreFang agents.
//! The `model` field resolves to an agent (by name, UUID, or `librefang:<name>`),
//! and the messages are forwarded to the agent's LLM loop.
//!
//! Supports both streaming (SSE) and non-streaming responses.

use crate::routes::AppState;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::sse::{Event as SseEvent, KeepAlive, Sse};
use axum::response::IntoResponse;
use axum::Json;
use librefang_kernel::kernel_handle::prelude::*;
use librefang_kernel::llm_driver::StreamEvent;
use librefang_types::agent::AgentId;
use librefang_types::message::{ContentBlock, Message, MessageContent, Role, StopReason};
use serde::{Deserialize, Serialize};
use std::convert::Infallible;
use std::sync::Arc;
use tracing::warn;

// ── Request types ──────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub(crate) struct ChatCompletionRequest {
    pub model: String,
    pub messages: Vec<OaiMessage>,
    #[serde(default)]
    pub stream: bool,
    // OpenAI wire fields accepted but not yet plumbed through to the driver.
    #[allow(dead_code)]
    pub max_tokens: Option<u32>,
    #[allow(dead_code)]
    pub temperature: Option<f32>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct OaiMessage {
    pub role: String,
    #[serde(default)]
    pub content: OaiContent,
}

#[derive(Debug, Deserialize, Default)]
#[serde(untagged)]
pub(crate) enum OaiContent {
    Text(String),
    Parts(Vec<OaiContentPart>),
    #[default]
    Null,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
pub(crate) enum OaiContentPart {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "image_url")]
    ImageUrl { image_url: OaiImageUrlRef },
}

#[derive(Debug, Deserialize)]
pub(crate) struct OaiImageUrlRef {
    pub url: String,
}

// ── Response types ──────────────────────────────────────────────────────────

#[derive(Serialize)]
struct ChatCompletionResponse {
    id: String,
    object: &'static str,
    created: u64,
    model: String,
    choices: Vec<Choice>,
    usage: UsageInfo,
}

#[derive(Serialize)]
struct Choice {
    index: u32,
    message: ChoiceMessage,
    finish_reason: &'static str,
}

#[derive(Serialize)]
struct ChoiceMessage {
    role: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_calls: Option<Vec<OaiToolCall>>,
}

#[derive(Serialize)]
struct UsageInfo {
    prompt_tokens: u64,
    completion_tokens: u64,
    total_tokens: u64,
}

#[derive(Serialize)]
struct ChatCompletionChunk {
    id: String,
    object: &'static str,
    created: u64,
    model: String,
    choices: Vec<ChunkChoice>,
}

#[derive(Serialize)]
struct ChunkChoice {
    index: u32,
    delta: ChunkDelta,
    finish_reason: Option<&'static str>,
}

#[derive(Serialize)]
struct ChunkDelta {
    #[serde(skip_serializing_if = "Option::is_none")]
    role: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_calls: Option<Vec<OaiToolCall>>,
}

#[derive(Serialize, Clone)]
struct OaiToolCall {
    index: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "type")]
    call_type: Option<&'static str>,
    function: OaiToolCallFunction,
}

#[derive(Serialize, Clone)]
struct OaiToolCallFunction {
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    arguments: Option<String>,
}

#[derive(Serialize)]
struct ModelObject {
    id: String,
    object: &'static str,
    created: u64,
    owned_by: String,
}

#[derive(Serialize)]
struct ModelListResponse {
    object: &'static str,
    data: Vec<ModelObject>,
}

// ── Agent resolution ────────────────────────────────────────────────────────

fn resolve_agent(state: &AppState, model: &str) -> Option<(AgentId, String)> {
    // 1. "librefang:<name>" → find agent by name
    if let Some(name) = model.strip_prefix("librefang:") {
        if let Some(entry) = state.kernel.agent_registry().find_by_name(name) {
            return Some((entry.id, entry.name.clone()));
        }
    }

    // 2. Valid UUID → find agent by ID
    if let Ok(id) = model.parse::<AgentId>() {
        if let Some(entry) = state.kernel.agent_registry().get(id) {
            return Some((entry.id, entry.name.clone()));
        }
    }

    // 3. Plain string → try as agent name
    if let Some(entry) = state.kernel.agent_registry().find_by_name(model) {
        return Some((entry.id, entry.name.clone()));
    }

    // No match — return None so the caller returns a proper 404
    None
}

// ── Message conversion ──────────────────────────────────────────────────────

fn convert_messages(oai_messages: &[OaiMessage]) -> Vec<Message> {
    oai_messages
        .iter()
        .filter_map(|m| {
            let role = match m.role.as_str() {
                "user" => Role::User,
                "assistant" => Role::Assistant,
                "system" => Role::System,
                _ => Role::User,
            };

            let content = match &m.content {
                OaiContent::Text(text) => MessageContent::Text(text.clone()),
                OaiContent::Parts(parts) => {
                    let blocks: Vec<ContentBlock> = parts
                        .iter()
                        .filter_map(|part| match part {
                            OaiContentPart::Text { text } => Some(ContentBlock::Text {
                                text: text.clone(),
                                provider_metadata: None,
                            }),
                            OaiContentPart::ImageUrl { image_url } => {
                                // Parse data URI: data:{media_type};base64,{data}
                                if let Some(rest) = image_url.url.strip_prefix("data:") {
                                    let parts: Vec<&str> = rest.splitn(2, ',').collect();
                                    if parts.len() == 2 {
                                        let media_type = parts[0]
                                            .strip_suffix(";base64")
                                            .unwrap_or(parts[0])
                                            .to_string();
                                        let data = parts[1].to_string();
                                        Some(ContentBlock::Image { media_type, data })
                                    } else {
                                        None
                                    }
                                } else {
                                    // URL-based images not supported (would require fetching)
                                    None
                                }
                            }
                        })
                        .collect();
                    if blocks.is_empty() {
                        return None;
                    }
                    MessageContent::Blocks(blocks)
                }
                OaiContent::Null => return None,
            };

            Some(Message {
                role,
                content,
                pinned: false,
                timestamp: None,
            })
        })
        .collect()
}

/// Extract the forwarding inputs from the latest user turn.
///
/// Returns the flattened text, whether the turn carries any image blocks, and
/// the structured content blocks (present only when the content is block-form,
/// so a plain-text turn yields `None` and takes the text-only kernel path).
/// Only the latest user turn is considered — LibreFang agents keep their own
/// per-session history, so prior turns are not replayed through this endpoint.
fn extract_latest_user_input(messages: &[Message]) -> (String, bool, Option<Vec<ContentBlock>>) {
    match messages
        .iter()
        .rev()
        .find(|m| m.role == Role::User)
        .map(|m| &m.content)
    {
        Some(content) => {
            let blocks = match content {
                MessageContent::Blocks(blocks) => Some(blocks.clone()),
                MessageContent::Text(_) => None,
            };
            (content.text_content(), content.has_images(), blocks)
        }
        None => (String::new(), false, None),
    }
}

// ── Handlers ────────────────────────────────────────────────────────────────

/// POST /v1/chat/completions
#[utoipa::path(post, path = "/v1/chat/completions", tag = "openai", request_body = crate::types::JsonObject, responses((status = 200, description = "OpenAI-compatible chat completion", body = crate::types::JsonObject)))]
#[allow(private_interfaces)]
pub async fn chat_completions(
    State(state): State<Arc<AppState>>,
    Json(req): Json<ChatCompletionRequest>,
) -> impl IntoResponse {
    let (agent_id, agent_name) = match resolve_agent(&state, &req.model) {
        Some(pair) => pair,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({
                    "error": {
                        "message": format!("No agent found for model '{}'", req.model),
                        "type": "invalid_request_error",
                        "code": "model_not_found"
                    }
                })),
            )
                .into_response();
        }
    };

    // Extract the latest user turn as the input.
    //
    // Only the most recent user message is forwarded: LibreFang agents keep
    // their own per-session conversation history, so prior user/assistant turns
    // and system messages in the request are intentionally not replayed here —
    // the kernel already holds the session context. Any image blocks on that
    // latest turn ARE threaded through to the kernel (vision) on the
    // non-streaming path via `content_blocks` below.
    let messages = convert_messages(&req.messages);
    let (last_user_msg, has_images, content_blocks) = extract_latest_user_input(&messages);

    // A user turn that is text-empty AND carries no image is a genuinely empty
    // request; an image-only turn is valid input for a vision model.
    if last_user_msg.is_empty() && !has_images {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": {
                    "message": "No user message found in request",
                    "type": "invalid_request_error",
                    "code": "missing_message"
                }
            })),
        )
            .into_response();
    }

    let request_id = format!("chatcmpl-{}", uuid::Uuid::new_v4());
    let created = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    if req.stream {
        // Streaming vision is not yet supported: the streaming kernel entry
        // point (`send_message_streaming_with_routing`) has no content-blocks
        // variant, so an image on the latest turn would be silently dropped.
        // Reject loudly rather than corrupt the request — vision works on the
        // non-streaming endpoint. Threading images through the streaming path
        // needs a new kernel method (cross-crate) and is tracked separately.
        if has_images {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({
                    "error": {
                        "message": "Image input is not supported with stream=true; send the request without streaming for vision",
                        "type": "invalid_request_error",
                        "code": "streaming_vision_unsupported"
                    }
                })),
            )
                .into_response();
        }

        // Streaming response
        return match stream_response(
            state,
            agent_id,
            agent_name,
            &last_user_msg,
            request_id,
            created,
        )
        .await
        {
            Ok(sse) => sse.into_response(),
            Err(e) => streaming_setup_error_response(&e),
        };
    }

    // Non-streaming response. Thread any parsed content blocks (text + images)
    // through so vision requests reach the kernel; a text-only turn yields
    // `content_blocks == None`, matching the previous text-only behaviour.
    let kernel_handle: Arc<dyn KernelHandle> = state.kernel.clone();
    match state
        .kernel
        .send_message_with_handle_and_blocks(
            agent_id,
            &last_user_msg,
            Some(kernel_handle),
            content_blocks,
        )
        .await
    {
        Ok(result) => {
            let response = ChatCompletionResponse {
                id: request_id,
                object: "chat.completion",
                created,
                model: agent_name,
                choices: vec![Choice {
                    index: 0,
                    message: ChoiceMessage {
                        role: "assistant",
                        content: Some(crate::ws::strip_think_tags(&result.response)),
                        tool_calls: None,
                    },
                    finish_reason: "stop",
                }],
                usage: UsageInfo {
                    prompt_tokens: result.total_usage.input_tokens,
                    completion_tokens: result.total_usage.output_tokens,
                    total_tokens: result.total_usage.input_tokens
                        + result.total_usage.output_tokens,
                },
            };
            Json(serde_json::to_value(&response).unwrap_or_default()).into_response()
        }
        Err(e) => {
            warn!("OpenAI compat: agent error: {e}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "error": {
                        "message": "Agent processing failed",
                        "type": "server_error"
                    }
                })),
            )
                .into_response()
        }
    }
}

/// Build the client-facing 500 response for a streaming-setup failure.
///
/// The detailed error is logged server-side; the client only sees a generic
/// message so internal kernel/provider detail is not leaked over the wire.
/// This mirrors the non-streaming branch's error handling.
fn streaming_setup_error_response(detail: &str) -> axum::response::Response {
    warn!("OpenAI compat: streaming setup error: {detail}");
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(serde_json::json!({
            "error": {
                "message": "Agent processing failed",
                "type": "server_error"
            }
        })),
    )
        .into_response()
}

/// Build a chunk with a delta and optional finish_reason, serialized to JSON.
fn make_chunk(
    id: &str,
    created: u64,
    model: &str,
    delta: ChunkDelta,
    finish_reason: Option<&'static str>,
) -> String {
    let chunk = ChatCompletionChunk {
        id: id.to_string(),
        object: "chat.completion.chunk",
        created,
        model: model.to_string(),
        choices: vec![ChunkChoice {
            index: 0,
            delta,
            finish_reason,
        }],
    };
    serde_json::to_string(&chunk).unwrap_or_default()
}

/// Mutable state threaded through the streaming forwarder as it maps
/// [`StreamEvent`]s onto OpenAI chunk frames.
struct ForwarderState {
    /// Monotonic tool_call index across the entire flattened completion (all
    /// agent-loop iterations). OpenAI clients reconstruct tool calls solely by
    /// this index, so it must never reset between iterations — resetting it
    /// makes a later iteration's first tool call collide with an earlier one,
    /// merging two distinct calls into one garbled entry.
    tool_index: u32,
    /// Set once a terminal `stop_reason` (EndTurn / MaxTokens / StopSequence)
    /// is observed, so the finalizer can tell a graceful end-of-turn apart from
    /// an aborted stream. `ToolUse` (an inter-iteration boundary) and
    /// `ContentFiltered` (a provider refusal) are deliberately NOT terminal.
    saw_terminal: bool,
}

/// Map one [`StreamEvent`] onto an OpenAI chunk frame, updating `state`.
///
/// Returns `None` for events that produce no client-visible chunk
/// (ContentComplete boundaries, tool-end / thinking / phase events).
fn stream_event_to_chunk(
    state: &mut ForwarderState,
    event: StreamEvent,
    req_id: &str,
    created: u64,
    model: &str,
) -> Option<String> {
    match event {
        StreamEvent::TextDelta { text } => Some(make_chunk(
            req_id,
            created,
            model,
            ChunkDelta {
                role: None,
                content: Some(text),
                tool_calls: None,
            },
            None,
        )),
        StreamEvent::ToolUseStart { id, name } => {
            let idx = state.tool_index;
            state.tool_index += 1;
            Some(make_chunk(
                req_id,
                created,
                model,
                ChunkDelta {
                    role: None,
                    content: None,
                    tool_calls: Some(vec![OaiToolCall {
                        index: idx,
                        id: Some(id),
                        call_type: Some("function"),
                        function: OaiToolCallFunction {
                            name: Some(name),
                            arguments: Some(String::new()),
                        },
                    }]),
                },
                None,
            ))
        }
        StreamEvent::ToolInputDelta { text } => {
            // tool_index already incremented past the current tool, so current = index - 1.
            let idx = state.tool_index.saturating_sub(1);
            Some(make_chunk(
                req_id,
                created,
                model,
                ChunkDelta {
                    role: None,
                    content: None,
                    tool_calls: Some(vec![OaiToolCall {
                        index: idx,
                        id: None,
                        call_type: None,
                        function: OaiToolCallFunction {
                            name: None,
                            arguments: Some(text),
                        },
                    }]),
                },
                None,
            ))
        }
        StreamEvent::ContentComplete { stop_reason, .. } => {
            // Terminal → mark a clean end-of-turn; wait for channel close to finish.
            // ToolUse → inter-iteration boundary, more iterations follow (do NOT
            // reset tool_index — see the field docs). ContentFiltered → refusal.
            if matches!(
                stop_reason,
                StopReason::EndTurn | StopReason::MaxTokens | StopReason::StopSequence
            ) {
                state.saw_terminal = true;
            }
            None
        }
        // ToolUseEnd, ToolExecutionResult, ThinkingDelta, PhaseChange, … — skip.
        _ => None,
    }
}

/// Build the terminal SSE frame once the event channel has closed.
///
/// On a clean completion this is the OpenAI `finish_reason: "stop"` chunk; on a
/// mid-stream failure (the agent loop returned `Err`, its task panicked, or the
/// stream closed without any terminal marker) it is an OpenAI-style in-band
/// error frame instead. Headers and HTTP 200 were already flushed when the
/// first chunk went out, so an in-band error frame is the only way to signal
/// failure to the client — this matches OpenAI's own streaming-error convention
/// and prevents a truncated response from being framed as a full success.
fn finish_or_error_frame(ended_cleanly: bool, req_id: &str, created: u64, model: &str) -> String {
    if ended_cleanly {
        make_chunk(
            req_id,
            created,
            model,
            ChunkDelta {
                role: None,
                content: None,
                tool_calls: None,
            },
            Some("stop"),
        )
    } else {
        serde_json::json!({
            "error": {
                "message": "upstream error",
                "type": "server_error"
            }
        })
        .to_string()
    }
}

/// Build an SSE stream response for streaming completions.
async fn stream_response(
    state: Arc<AppState>,
    agent_id: AgentId,
    agent_name: String,
    message: &str,
    request_id: String,
    created: u64,
) -> Result<axum::response::Response, String> {
    let kernel_handle: Arc<dyn KernelHandle> = state.kernel.clone();

    // Keep the JoinHandle: it is the only carrier of a mid-stream failure. The
    // agent loop can error after deltas have already streamed (timeout, content
    // filter, repeated tool failures); it then returns `Err` on this handle and
    // drops the StreamEvent sender, which merely closes `rx`. Without inspecting
    // the handle we could not tell an aborted stream from a clean end-of-turn.
    let (mut rx, handle) = state
        .kernel
        .clone()
        .send_message_streaming_with_routing(agent_id, message, Some(kernel_handle))
        .await
        .map_err(|e| format!("Streaming setup failed: {e}"))?;

    let (tx, stream_rx) = tokio::sync::mpsc::channel::<Result<SseEvent, Infallible>>(64);

    // Send initial role delta
    let first_chunk = ChatCompletionChunk {
        id: request_id.clone(),
        object: "chat.completion.chunk",
        created,
        model: agent_name.clone(),
        choices: vec![ChunkChoice {
            index: 0,
            delta: ChunkDelta {
                role: Some("assistant"),
                content: None,
                tool_calls: None,
            },
            finish_reason: None,
        }],
    };
    let _ = tx
        .send(Ok(SseEvent::default().data(
            serde_json::to_string(&first_chunk).unwrap_or_default(),
        )))
        .await;

    // Spawn forwarder task — streams ALL agent-loop iterations, flattened into
    // one OpenAI completion, until the event channel closes.
    let req_id = request_id.clone();
    tokio::spawn(async move {
        let mut fwd = ForwarderState {
            tool_index: 0,
            saw_terminal: false,
        };

        while let Some(event) = rx.recv().await {
            let Some(json) = stream_event_to_chunk(&mut fwd, event, &req_id, created, &agent_name)
            else {
                continue;
            };
            if tx.send(Ok(SseEvent::default().data(json))).await.is_err() {
                break;
            }
        }

        // Channel closed — the agent loop task has finished. Await the handle
        // (it has already resolved, so this cannot deadlock) to learn whether
        // the loop erred mid-stream. Emit a clean finish only on a successful
        // loop that reached a terminal marker; otherwise send an in-band error
        // frame so the client is not handed truncated output framed as success.
        let loop_result = handle.await;
        let ended_cleanly = fwd.saw_terminal && matches!(loop_result, Ok(Ok(_)));
        if !ended_cleanly {
            match &loop_result {
                Ok(Err(e)) => warn!("OpenAI compat: streaming agent error: {e}"),
                Err(e) => warn!("OpenAI compat: streaming agent task failed to join: {e}"),
                Ok(Ok(_)) => {
                    warn!("OpenAI compat: stream closed without a terminal completion marker")
                }
            }
        }
        let final_json = finish_or_error_frame(ended_cleanly, &req_id, created, &agent_name);
        let _ = tx.send(Ok(SseEvent::default().data(final_json))).await;
        let _ = tx.send(Ok(SseEvent::default().data("[DONE]"))).await;
    });

    let stream = tokio_stream::wrappers::ReceiverStream::new(stream_rx);
    Ok(Sse::new(stream)
        .keep_alive(
            KeepAlive::new()
                .interval(std::time::Duration::from_secs(15))
                .text("keep-alive"),
        )
        .into_response())
}

/// GET /v1/models — List available agents as OpenAI model objects.
#[utoipa::path(get, path = "/v1/models", tag = "openai", operation_id = "list_openai_models", responses((status = 200, description = "OpenAI-compatible model list", body = crate::types::JsonObject)))]
pub async fn list_models(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    // Read-only iteration: prefer cheap Arc clones over full manifest deep-copy (#3569).
    let agents = state.kernel.agent_registry().list_arcs();
    let created = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let models: Vec<ModelObject> = agents
        .iter()
        .map(|e| ModelObject {
            id: format!("librefang:{}", e.name),
            object: "model",
            created,
            owned_by: "librefang".to_string(),
        })
        .collect();

    Json(
        serde_json::to_value(&ModelListResponse {
            object: "list",
            data: models,
        })
        .unwrap_or_default(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn streaming_setup_error_scrubs_internal_detail() {
        let raw = "Streaming setup failed: provider connection refused at 10.0.0.5:8443";
        let resp = streaming_setup_error_response(raw);
        assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);

        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let body = String::from_utf8(bytes.to_vec()).unwrap();

        // The raw internal error must not leak to the client.
        assert!(!body.contains("provider connection refused"));
        assert!(!body.contains("10.0.0.5"));
        // The generic client-facing message is returned instead.
        assert!(body.contains("Agent processing failed"));
        assert!(body.contains("server_error"));
    }

    #[test]
    fn test_oai_content_deserialize_string() {
        let json = r#"{"role":"user","content":"hello"}"#;
        let msg: OaiMessage = serde_json::from_str(json).unwrap();
        assert!(matches!(msg.content, OaiContent::Text(ref t) if t == "hello"));
    }

    #[test]
    fn test_oai_content_deserialize_parts() {
        let json = r#"{"role":"user","content":[{"type":"text","text":"what is this?"},{"type":"image_url","image_url":{"url":"data:image/png;base64,abc123"}}]}"#;
        let msg: OaiMessage = serde_json::from_str(json).unwrap();
        assert!(matches!(msg.content, OaiContent::Parts(ref p) if p.len() == 2));
    }

    #[test]
    fn test_convert_messages_text() {
        let oai = vec![
            OaiMessage {
                role: "system".to_string(),
                content: OaiContent::Text("You are helpful.".to_string()),
            },
            OaiMessage {
                role: "user".to_string(),
                content: OaiContent::Text("Hello!".to_string()),
            },
        ];
        let msgs = convert_messages(&oai);
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].role, Role::System);
        assert_eq!(msgs[1].role, Role::User);
    }

    #[test]
    fn test_convert_messages_with_image() {
        let oai = vec![OaiMessage {
            role: "user".to_string(),
            content: OaiContent::Parts(vec![
                OaiContentPart::Text {
                    text: "What is this?".to_string(),
                },
                OaiContentPart::ImageUrl {
                    image_url: OaiImageUrlRef {
                        url: "data:image/png;base64,iVBORw0KGgo=".to_string(),
                    },
                },
            ]),
        }];
        let msgs = convert_messages(&oai);
        assert_eq!(msgs.len(), 1);
        match &msgs[0].content {
            MessageContent::Blocks(blocks) => {
                assert_eq!(blocks.len(), 2);
                assert!(matches!(&blocks[0], ContentBlock::Text { .. }));
                assert!(matches!(&blocks[1], ContentBlock::Image { .. }));
            }
            _ => panic!("Expected Blocks"),
        }
    }

    #[test]
    fn test_response_serialization() {
        let resp = ChatCompletionResponse {
            id: "chatcmpl-test".to_string(),
            object: "chat.completion",
            created: 1234567890,
            model: "test-agent".to_string(),
            choices: vec![Choice {
                index: 0,
                message: ChoiceMessage {
                    role: "assistant",
                    content: Some("Hello!".to_string()),
                    tool_calls: None,
                },
                finish_reason: "stop",
            }],
            usage: UsageInfo {
                prompt_tokens: 10,
                completion_tokens: 5,
                total_tokens: 15,
            },
        };
        let json = serde_json::to_value(&resp).unwrap();
        assert_eq!(json["object"], "chat.completion");
        assert_eq!(json["choices"][0]["message"]["content"], "Hello!");
        assert_eq!(json["usage"]["total_tokens"], 15);
        // tool_calls should be omitted when None
        assert!(json["choices"][0]["message"].get("tool_calls").is_none());
    }

    #[test]
    fn test_chunk_serialization() {
        let chunk = ChatCompletionChunk {
            id: "chatcmpl-test".to_string(),
            object: "chat.completion.chunk",
            created: 1234567890,
            model: "test-agent".to_string(),
            choices: vec![ChunkChoice {
                index: 0,
                delta: ChunkDelta {
                    role: None,
                    content: Some("Hello".to_string()),
                    tool_calls: None,
                },
                finish_reason: None,
            }],
        };
        let json = serde_json::to_value(&chunk).unwrap();
        assert_eq!(json["object"], "chat.completion.chunk");
        assert_eq!(json["choices"][0]["delta"]["content"], "Hello");
        assert!(json["choices"][0]["delta"]["role"].is_null());
        // tool_calls should be omitted when None
        assert!(json["choices"][0]["delta"].get("tool_calls").is_none());
    }

    #[test]
    fn test_tool_call_serialization() {
        let tc = OaiToolCall {
            index: 0,
            id: Some("call_abc123".to_string()),
            call_type: Some("function"),
            function: OaiToolCallFunction {
                name: Some("get_weather".to_string()),
                arguments: Some(r#"{"location":"NYC"}"#.to_string()),
            },
        };
        let json = serde_json::to_value(&tc).unwrap();
        assert_eq!(json["index"], 0);
        assert_eq!(json["id"], "call_abc123");
        assert_eq!(json["type"], "function");
        assert_eq!(json["function"]["name"], "get_weather");
        assert_eq!(json["function"]["arguments"], r#"{"location":"NYC"}"#);
    }

    #[test]
    fn test_chunk_delta_with_tool_calls() {
        let chunk = ChatCompletionChunk {
            id: "chatcmpl-test".to_string(),
            object: "chat.completion.chunk",
            created: 1234567890,
            model: "test-agent".to_string(),
            choices: vec![ChunkChoice {
                index: 0,
                delta: ChunkDelta {
                    role: None,
                    content: None,
                    tool_calls: Some(vec![OaiToolCall {
                        index: 0,
                        id: Some("call_1".to_string()),
                        call_type: Some("function"),
                        function: OaiToolCallFunction {
                            name: Some("search".to_string()),
                            arguments: Some(String::new()),
                        },
                    }]),
                },
                finish_reason: None,
            }],
        };
        let json = serde_json::to_value(&chunk).unwrap();
        let tc = &json["choices"][0]["delta"]["tool_calls"][0];
        assert_eq!(tc["index"], 0);
        assert_eq!(tc["id"], "call_1");
        assert_eq!(tc["type"], "function");
        assert_eq!(tc["function"]["name"], "search");
        // content should be omitted
        assert!(json["choices"][0]["delta"].get("content").is_none());
    }

    #[test]
    fn test_tool_input_delta_chunk() {
        // Incremental arguments chunk — no id, no type, no name
        let tc = OaiToolCall {
            index: 2,
            id: None,
            call_type: None,
            function: OaiToolCallFunction {
                name: None,
                arguments: Some(r#"{"q":"rust"}"#.to_string()),
            },
        };
        let json = serde_json::to_value(&tc).unwrap();
        assert_eq!(json["index"], 2);
        // id and type should be omitted
        assert!(json.get("id").is_none());
        assert!(json.get("type").is_none());
        assert!(json["function"].get("name").is_none());
        assert_eq!(json["function"]["arguments"], r#"{"q":"rust"}"#);
    }

    #[test]
    fn test_backward_compat_no_tool_calls() {
        // When tool_calls is None, it should not appear in JSON at all (backward compat)
        let msg = ChoiceMessage {
            role: "assistant",
            content: Some("Hello".to_string()),
            tool_calls: None,
        };
        let json_str = serde_json::to_string(&msg).unwrap();
        assert!(!json_str.contains("tool_calls"));

        let delta = ChunkDelta {
            role: Some("assistant"),
            content: Some("Hi".to_string()),
            tool_calls: None,
        };
        let json_str = serde_json::to_string(&delta).unwrap();
        assert!(!json_str.contains("tool_calls"));
    }

    // Regression (audit finding 12): the streamed tool_call `index` must be
    // monotonic across the whole flattened completion. Resetting it at each
    // agent-loop iteration boundary (`ContentComplete{ToolUse}`) made a later
    // iteration's first tool call reuse index 0, merging two distinct calls in
    // the client's accumulator.
    #[test]
    fn tool_call_index_is_monotonic_across_iterations() {
        use librefang_types::message::TokenUsage;

        let mut fwd = ForwarderState {
            tool_index: 0,
            saw_terminal: false,
        };
        let c1 = stream_event_to_chunk(
            &mut fwd,
            StreamEvent::ToolUseStart {
                id: "call_A".to_string(),
                name: "search".to_string(),
            },
            "req",
            0,
            "agent",
        )
        .expect("ToolUseStart yields a chunk");
        // Iteration boundary — must NOT reset the index (was the bug).
        assert!(stream_event_to_chunk(
            &mut fwd,
            StreamEvent::ContentComplete {
                stop_reason: StopReason::ToolUse,
                usage: TokenUsage::default(),
            },
            "req",
            0,
            "agent",
        )
        .is_none());
        let c2 = stream_event_to_chunk(
            &mut fwd,
            StreamEvent::ToolUseStart {
                id: "call_B".to_string(),
                name: "fetch".to_string(),
            },
            "req",
            0,
            "agent",
        )
        .expect("ToolUseStart yields a chunk");

        let j1: serde_json::Value = serde_json::from_str(&c1).unwrap();
        let j2: serde_json::Value = serde_json::from_str(&c2).unwrap();
        assert_eq!(j1["choices"][0]["delta"]["tool_calls"][0]["index"], 0);
        assert_eq!(j2["choices"][0]["delta"]["tool_calls"][0]["index"], 1);
        // ToolUse is an inter-iteration boundary, not a terminal completion.
        assert!(!fwd.saw_terminal);
    }

    // Regression (audit finding 6): only genuine end-of-turn stop reasons mark
    // a terminal completion. ToolUse (inter-iteration) and ContentFiltered
    // (refusal) must NOT, so the finalizer emits an error frame for them.
    #[test]
    fn terminal_marker_only_set_on_end_of_turn_reasons() {
        use librefang_types::message::TokenUsage;

        for (reason, expect_terminal) in [
            (StopReason::EndTurn, true),
            (StopReason::MaxTokens, true),
            (StopReason::StopSequence, true),
            (StopReason::ToolUse, false),
            (StopReason::ContentFiltered, false),
        ] {
            let mut fwd = ForwarderState {
                tool_index: 0,
                saw_terminal: false,
            };
            let out = stream_event_to_chunk(
                &mut fwd,
                StreamEvent::ContentComplete {
                    stop_reason: reason,
                    usage: TokenUsage::default(),
                },
                "req",
                0,
                "agent",
            );
            assert!(out.is_none(), "ContentComplete emits no client chunk");
            assert_eq!(fwd.saw_terminal, expect_terminal, "reason {reason:?}");
        }
    }

    // Regression (audit finding 6): an aborted stream must end with an in-band
    // error frame, never a bogus `finish_reason:"stop"` that frames truncated
    // output as a full success.
    #[test]
    fn finalizer_emits_error_frame_on_aborted_stream() {
        let clean = finish_or_error_frame(true, "req", 0, "agent");
        let cj: serde_json::Value = serde_json::from_str(&clean).unwrap();
        assert_eq!(cj["choices"][0]["finish_reason"], "stop");
        assert!(cj.get("error").is_none());

        let aborted = finish_or_error_frame(false, "req", 0, "agent");
        let aj: serde_json::Value = serde_json::from_str(&aborted).unwrap();
        assert_eq!(aj["error"]["type"], "server_error");
        assert!(aj.get("choices").is_none());
        assert!(!aborted.contains("\"finish_reason\":\"stop\""));
    }

    // Regression (audit finding 13): an image-only user turn must be accepted
    // (not rejected as empty) and its image blocks captured for threading to
    // the kernel; a text+image turn keeps its blocks; a plain-text turn yields
    // no blocks and takes the text-only path.
    #[test]
    fn extract_latest_user_input_handles_images_and_history() {
        // Image-only turn: no text, but has_images true and blocks present.
        let image_only = convert_messages(&[OaiMessage {
            role: "user".to_string(),
            content: OaiContent::Parts(vec![OaiContentPart::ImageUrl {
                image_url: OaiImageUrlRef {
                    url: "data:image/png;base64,iVBORw0KGgo=".to_string(),
                },
            }]),
        }]);
        let (text, has_images, blocks) = extract_latest_user_input(&image_only);
        assert!(text.is_empty());
        assert!(has_images);
        assert!(matches!(blocks, Some(ref b) if b.len() == 1));
        // The pre-fix empty-text guard would have 400'd this valid vision request.
        assert!(!text.is_empty() || has_images);

        // Text + image: text extracted, blocks retained.
        let text_and_image = convert_messages(&[OaiMessage {
            role: "user".to_string(),
            content: OaiContent::Parts(vec![
                OaiContentPart::Text {
                    text: "what is this?".to_string(),
                },
                OaiContentPart::ImageUrl {
                    image_url: OaiImageUrlRef {
                        url: "data:image/png;base64,iVBORw0KGgo=".to_string(),
                    },
                },
            ]),
        }]);
        let (text, has_images, blocks) = extract_latest_user_input(&text_and_image);
        assert_eq!(text, "what is this?");
        assert!(has_images);
        assert!(matches!(blocks, Some(ref b) if b.len() == 2));

        // Plain text: no blocks, text-only kernel path.
        let plain = convert_messages(&[
            OaiMessage {
                role: "system".to_string(),
                content: OaiContent::Text("ignored history".to_string()),
            },
            OaiMessage {
                role: "user".to_string(),
                content: OaiContent::Text("hello".to_string()),
            },
        ]);
        let (text, has_images, blocks) = extract_latest_user_input(&plain);
        assert_eq!(text, "hello");
        assert!(!has_images);
        assert!(blocks.is_none());
    }
}
