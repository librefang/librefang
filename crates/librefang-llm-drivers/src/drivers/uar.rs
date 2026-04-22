//! UAR (Universal Agent Runtime) LLM driver.
//!
//! Wraps UAR's [`LiterLlmDriver`] to give LibreFang agents access to 142+
//! LLM providers via liter-llm's unified `provider/model` addressing (e.g.
//! `"openai/gpt-4o"`, `"anthropic/claude-opus-4"`,
//! `"groq/llama-3.3-70b-versatile"`).
//!
//! # Usage in an agent manifest
//!
//! ```toml
//! [model]
//! provider = "uar"
//! model    = "openai/gpt-4o"   # any liter-llm provider/model string
//!
//! [env]
//! UAR_LLM__API_KEY = "sk-..."  # or a provider-specific key
//! ```
//!
//! # Key resolution order
//!
//! 1. `model.api_key` in the agent manifest
//! 2. `UAR_LLM__API_KEY` environment variable
//! 3. `LLM_API_KEY` environment variable (legacy)

use std::collections::HashSet;
use std::sync::Arc;

use async_trait::async_trait;
use futures::StreamExt;
use librefang_types::{
    message::{ContentBlock, MessageContent, Role, StopReason, TokenUsage},
    tool::ToolDefinition,
};
use universal_agent_runtime::{
    config::{build_client_config, LlmConfig},
    llm::{LiterLlmDriver, LlmDriver as UarLlmDriver, LlmRequest},
    normalized::NormalizedEvent,
};

use crate::llm_driver::{
    CompletionRequest, CompletionResponse, DriverConfig, LlmDriver, LlmError, StreamEvent,
};

// ---------------------------------------------------------------------------
// Driver struct
// ---------------------------------------------------------------------------

/// LLM driver backed by UAR's [`LiterLlmDriver`].
///
/// Created via `provider = "uar"` in an agent manifest; the model string
/// must use liter-llm's `provider/model` convention.
pub struct UarDriver {
    /// API key forwarded to liter-llm.
    api_key: Option<String>,
    /// Optional base-URL override (passed straight through to liter-llm).
    base_url: Option<String>,
}

impl std::fmt::Debug for UarDriver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UarDriver")
            .field("api_key", &self.api_key.as_ref().map(|_| "<redacted>"))
            .field("base_url", &self.base_url)
            .finish()
    }
}

impl UarDriver {
    /// Construct a `UarDriver` and return it as `Arc<dyn LlmDriver>`.
    ///
    /// # Errors
    ///
    /// Currently infallible — returns `Err` only to satisfy the
    /// `create_driver` call-site convention.
    pub fn create(config: &DriverConfig) -> Result<Arc<dyn LlmDriver>, LlmError> {
        let api_key = config
            .api_key
            .clone()
            .or_else(|| std::env::var("UAR_LLM__API_KEY").ok())
            .or_else(|| std::env::var("LLM_API_KEY").ok());

        Ok(Arc::new(Self {
            api_key,
            base_url: config.base_url.clone(),
        }))
    }

    /// Build a [`LiterLlmDriver`] for the given model string.
    ///
    /// A fresh driver is created per call so that the model — which comes
    /// from `CompletionRequest.model` at call-time — is correctly embedded
    /// inside the liter-llm `ChatCompletionRequest`.
    fn make_driver(&self, model: &str) -> Result<LiterLlmDriver, LlmError> {
        let llm_cfg = LlmConfig {
            model: model.to_string(),
            api_key: self.api_key.clone(),
            base_url: self.base_url.clone(),
            ..LlmConfig::default()
        };
        let client_config = build_client_config(&llm_cfg);
        LiterLlmDriver::new(client_config, model.to_string(), None)
            .map_err(|e| LlmError::Http(format!("UarDriver: failed to build LiterLlmDriver: {e}")))
    }
}

// ---------------------------------------------------------------------------
// LlmDriver implementation
// ---------------------------------------------------------------------------

#[async_trait]
impl LlmDriver for UarDriver {
    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        let driver = self.make_driver(&request.model)?;
        let uar_req = build_uar_request(&request);

        let mut stream = driver
            .stream(uar_req)
            .await
            .map_err(|e| LlmError::Http(format!("UarDriver stream error: {e}")))?;

        let mut text = String::new();
        let mut tool_calls: Vec<librefang_types::tool::ToolCall> = Vec::new();
        let mut usage = TokenUsage::default();
        let mut has_tool_calls = false;

        while let Some(event) = stream.next().await {
            match event.map_err(|e| LlmError::Http(e.to_string()))? {
                NormalizedEvent::MessageDelta { text: t } => {
                    text.push_str(&t);
                }
                NormalizedEvent::ToolCallComplete {
                    id,
                    name,
                    arguments_json,
                    ..
                } => {
                    has_tool_calls = true;
                    let input = serde_json::from_str::<serde_json::Value>(&arguments_json)
                        .unwrap_or_else(|_| serde_json::Value::Object(Default::default()));
                    tool_calls.push(librefang_types::tool::ToolCall { id, name, input });
                }
                NormalizedEvent::Usage {
                    prompt_tokens,
                    completion_tokens,
                    ..
                } => {
                    usage.input_tokens = u64::from(prompt_tokens);
                    usage.output_tokens = u64::from(completion_tokens);
                }
                NormalizedEvent::Error { message, .. } => {
                    return Err(LlmError::Http(message));
                }
                _ => {}
            }
        }

        let stop_reason = if has_tool_calls {
            StopReason::ToolUse
        } else {
            StopReason::EndTurn
        };

        let mut content: Vec<ContentBlock> = Vec::new();
        if !text.is_empty() {
            content.push(ContentBlock::Text {
                text,
                provider_metadata: None,
            });
        }
        for tc in &tool_calls {
            content.push(ContentBlock::ToolUse {
                id: tc.id.clone(),
                name: tc.name.clone(),
                input: tc.input.clone(),
                provider_metadata: None,
            });
        }

        Ok(CompletionResponse {
            content,
            stop_reason,
            tool_calls,
            usage,
        })
    }

    async fn stream(
        &self,
        request: CompletionRequest,
        tx: tokio::sync::mpsc::Sender<StreamEvent>,
    ) -> Result<CompletionResponse, LlmError> {
        let driver = self.make_driver(&request.model)?;
        let uar_req = build_uar_request(&request);

        let mut stream = driver
            .stream(uar_req)
            .await
            .map_err(|e| LlmError::Http(format!("UarDriver stream error: {e}")))?;

        let mut text = String::new();
        let mut tool_calls: Vec<librefang_types::tool::ToolCall> = Vec::new();
        let mut usage = TokenUsage::default();
        let mut has_tool_calls = false;
        // Track which call_index values have already triggered a ToolUseStart.
        let mut started_indices: HashSet<usize> = HashSet::new();

        while let Some(event) = stream.next().await {
            let event = event.map_err(|e| LlmError::Http(e.to_string()))?;
            match &event {
                NormalizedEvent::MessageDelta { text: t } => {
                    text.push_str(t);
                    let _ = tx.send(StreamEvent::TextDelta { text: t.clone() }).await;
                }
                NormalizedEvent::ThinkingDelta { text: t } => {
                    let _ = tx
                        .send(StreamEvent::ThinkingDelta { text: t.clone() })
                        .await;
                }
                NormalizedEvent::ToolCallDelta {
                    call_index,
                    id,
                    name,
                    arguments_delta,
                } => {
                    // Emit ToolUseStart once per call_index, when both id and name are known.
                    if !started_indices.contains(call_index) {
                        if let (Some(id_val), Some(name_val)) = (id, name) {
                            started_indices.insert(*call_index);
                            let _ = tx
                                .send(StreamEvent::ToolUseStart {
                                    id: id_val.clone(),
                                    name: name_val.clone(),
                                })
                                .await;
                        }
                    }
                    if let Some(delta) = arguments_delta {
                        let _ = tx
                            .send(StreamEvent::ToolInputDelta {
                                text: delta.clone(),
                            })
                            .await;
                    }
                }
                NormalizedEvent::ToolCallComplete {
                    id,
                    name,
                    arguments_json,
                    ..
                } => {
                    has_tool_calls = true;
                    let input = serde_json::from_str::<serde_json::Value>(arguments_json)
                        .unwrap_or_else(|_| serde_json::Value::Object(Default::default()));
                    let _ = tx
                        .send(StreamEvent::ToolUseEnd {
                            id: id.clone(),
                            name: name.clone(),
                            input: input.clone(),
                        })
                        .await;
                    tool_calls.push(librefang_types::tool::ToolCall {
                        id: id.clone(),
                        name: name.clone(),
                        input,
                    });
                }
                NormalizedEvent::Usage {
                    prompt_tokens,
                    completion_tokens,
                    ..
                } => {
                    usage.input_tokens = u64::from(*prompt_tokens);
                    usage.output_tokens = u64::from(*completion_tokens);
                }
                NormalizedEvent::Error { message, .. } => {
                    return Err(LlmError::Http(message.clone()));
                }
                NormalizedEvent::Done => {
                    let stop_reason = if has_tool_calls {
                        StopReason::ToolUse
                    } else {
                        StopReason::EndTurn
                    };
                    let _ = tx
                        .send(StreamEvent::ContentComplete { stop_reason, usage })
                        .await;
                }
                _ => {}
            }
        }

        let stop_reason = if has_tool_calls {
            StopReason::ToolUse
        } else {
            StopReason::EndTurn
        };

        let mut content: Vec<ContentBlock> = Vec::new();
        if !text.is_empty() {
            content.push(ContentBlock::Text {
                text,
                provider_metadata: None,
            });
        }
        for tc in &tool_calls {
            content.push(ContentBlock::ToolUse {
                id: tc.id.clone(),
                name: tc.name.clone(),
                input: tc.input.clone(),
                provider_metadata: None,
            });
        }

        Ok(CompletionResponse {
            content,
            stop_reason,
            tool_calls,
            usage,
        })
    }
}

// ---------------------------------------------------------------------------
// Request translation helpers
// ---------------------------------------------------------------------------

/// Convert a LibreFang [`CompletionRequest`] into a UAR [`LlmRequest`].
///
/// Messages are serialized as OpenAI-format JSON values, which liter-llm
/// then deserializes into its own typed `Message` enum inside the driver.
fn build_uar_request(request: &CompletionRequest) -> LlmRequest {
    let mut messages: Vec<serde_json::Value> = Vec::new();

    // Inject system prompt first when supplied via the dedicated field.
    if let Some(ref sys) = request.system {
        messages.push(serde_json::json!({ "role": "system", "content": sys }));
    }

    for msg in &request.messages {
        match (&msg.role, &msg.content) {
            // ── System ──────────────────────────────────────────────────
            // Only include if no dedicated system field was provided.
            (Role::System, MessageContent::Text(text)) if request.system.is_none() => {
                messages.push(serde_json::json!({ "role": "system", "content": text }));
            }

            // ── User — plain text ────────────────────────────────────────
            (Role::User, MessageContent::Text(text)) => {
                messages.push(serde_json::json!({ "role": "user", "content": text }));
            }

            // ── Assistant — plain text ───────────────────────────────────
            (Role::Assistant, MessageContent::Text(text)) => {
                messages.push(serde_json::json!({ "role": "assistant", "content": text }));
            }

            // ── User — structured blocks ─────────────────────────────────
            // Tool results become separate `tool`-role messages; other content
            // (text, images) becomes a single `user`-role message with parts.
            (Role::User, MessageContent::Blocks(blocks)) => {
                let mut parts: Vec<serde_json::Value> = Vec::new();
                let mut has_tool_results = false;

                for block in blocks {
                    match block {
                        ContentBlock::ToolResult {
                            tool_use_id,
                            content,
                            ..
                        } => {
                            has_tool_results = true;
                            messages.push(serde_json::json!({
                                "role": "tool",
                                "content": if content.is_empty() { "(empty)" } else { content.as_str() },
                                "tool_call_id": tool_use_id,
                            }));
                        }
                        ContentBlock::Text { text, .. } => {
                            parts.push(serde_json::json!({ "type": "text", "text": text }));
                        }
                        ContentBlock::Image { media_type, data } => {
                            parts.push(serde_json::json!({
                                "type": "image_url",
                                "image_url": {
                                    "url": format!("data:{media_type};base64,{data}")
                                }
                            }));
                        }
                        // `ImageFile` references a local on-disk image — not
                        // sent over the wire (we'd need to read+base64-encode
                        // here; defer to callers that already inline images).
                        ContentBlock::ImageFile { .. }
                        | ContentBlock::Thinking { .. }
                        | ContentBlock::ToolUse { .. }
                        | ContentBlock::Unknown => {}
                    }
                }

                // Only emit a user message when there is non-tool-result content
                // and it wasn't a pure tool-result block list.
                if !parts.is_empty() && !has_tool_results {
                    messages.push(serde_json::json!({ "role": "user", "content": parts }));
                }
            }

            // ── Assistant — structured blocks ────────────────────────────
            // Extract text, tool-use calls, and thinking blocks.
            (Role::Assistant, MessageContent::Blocks(blocks)) => {
                let mut text_parts: Vec<String> = Vec::new();
                let mut tool_calls_json: Vec<serde_json::Value> = Vec::new();

                for block in blocks {
                    match block {
                        ContentBlock::Text { text, .. } => {
                            text_parts.push(text.clone());
                        }
                        ContentBlock::ToolUse {
                            id, name, input, ..
                        } => {
                            tool_calls_json.push(serde_json::json!({
                                "id": id,
                                "type": "function",
                                "function": {
                                    "name": name,
                                    "arguments": serde_json::to_string(input)
                                        .unwrap_or_else(|_| "{}".to_string()),
                                }
                            }));
                        }
                        // Thinking blocks are not forwarded — providers that
                        // support thinking don't need the prior round's trace.
                        ContentBlock::Thinking { .. }
                        | ContentBlock::ToolResult { .. }
                        | ContentBlock::Image { .. }
                        | ContentBlock::ImageFile { .. }
                        | ContentBlock::Unknown => {}
                    }
                }

                let mut msg_val = serde_json::json!({ "role": "assistant" });
                msg_val["content"] = if text_parts.is_empty() {
                    serde_json::Value::Null
                } else {
                    serde_json::Value::String(text_parts.join(""))
                };
                if !tool_calls_json.is_empty() {
                    msg_val["tool_calls"] = serde_json::Value::Array(tool_calls_json);
                }
                messages.push(msg_val);
            }

            _ => {}
        }
    }

    let tools = tools_to_openai_json(&request.tools);
    LlmRequest {
        messages,
        tools,
        cache_strategy: None,
        thinking_config: None,
        anthropic_system: None,
    }
}

/// Convert LibreFang [`ToolDefinition`]s to OpenAI function-calling JSON schema.
fn tools_to_openai_json(tools: &[ToolDefinition]) -> Vec<serde_json::Value> {
    tools
        .iter()
        .map(|t| {
            serde_json::json!({
                "type": "function",
                "function": {
                    "name": t.name,
                    "description": t.description,
                    "parameters": t.input_schema,
                }
            })
        })
        .collect()
}
