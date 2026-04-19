//! Anthropic Claude API driver.
//!
//! Full implementation of the Anthropic Messages API with tool use support,
//! system prompt extraction, and retry on 429/529 errors.

use crate::llm_driver::{CompletionRequest, CompletionResponse, LlmDriver, LlmError, StreamEvent};
use async_trait::async_trait;
use futures::StreamExt;
use librefang_types::config::ResponseFormat;
use librefang_types::message::{
    ContentBlock, Message, MessageContent, Role, StopReason, TokenUsage,
};
use librefang_types::tool::ToolCall;
use serde::{Deserialize, Serialize};
use tracing::{debug, warn};
use zeroize::Zeroizing;

/// Anthropic Claude API driver.
pub struct AnthropicDriver {
    api_key: Zeroizing<String>,
    base_url: String,
    client: reqwest::Client,
}

impl AnthropicDriver {
    /// Create a new Anthropic driver.
    pub fn new(api_key: String, base_url: String) -> Self {
        Self::with_proxy(api_key, base_url, None)
    }

    /// Create a new Anthropic driver with an optional per-provider proxy.
    pub fn with_proxy(api_key: String, base_url: String, proxy_url: Option<&str>) -> Self {
        let client = match proxy_url {
            Some(url) => librefang_http::proxied_client_with_override(url),
            None => librefang_http::proxied_client(),
        };
        Self {
            api_key: Zeroizing::new(api_key),
            base_url,
            client,
        }
    }
}

/// Anthropic Messages API request body.
#[derive(Debug, Serialize)]
struct ApiRequest {
    model: String,
    max_tokens: u32,
    /// System prompt — either a plain string or structured blocks with
    /// `cache_control` for prompt caching.
    #[serde(skip_serializing_if = "Option::is_none")]
    system: Option<serde_json::Value>,
    messages: Vec<ApiMessage>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tools: Vec<ApiTool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    stream: bool,
    /// Extended thinking configuration.
    /// Anthropic API expects: `{"type": "enabled", "budget_tokens": N}`
    #[serde(skip_serializing_if = "Option::is_none")]
    thinking: Option<serde_json::Value>,
}

#[derive(Debug, Serialize)]
struct ApiMessage {
    role: String,
    content: ApiContent,
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
enum ApiContent {
    Text(String),
    Blocks(Vec<ApiContentBlock>),
}

#[derive(Debug, Serialize)]
#[serde(tag = "type")]
enum ApiContentBlock {
    #[serde(rename = "text")]
    Text {
        text: String,
        /// `cache_control: {"type":"ephemeral"}` marker, stamped on the
        /// last block of the last message when prompt caching is enabled.
        /// Anthropic caches the prefix up to and including the marked
        /// block — so the next turn's matching prefix hits the cache.
        #[serde(skip_serializing_if = "Option::is_none")]
        cache_control: Option<serde_json::Value>,
    },
    #[serde(rename = "image")]
    Image {
        source: ApiImageSource,
        #[serde(skip_serializing_if = "Option::is_none")]
        cache_control: Option<serde_json::Value>,
    },
    #[serde(rename = "tool_use")]
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
        #[serde(skip_serializing_if = "Option::is_none")]
        cache_control: Option<serde_json::Value>,
    },
    #[serde(rename = "tool_result")]
    ToolResult {
        tool_use_id: String,
        content: String,
        #[serde(skip_serializing_if = "std::ops::Not::not")]
        is_error: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        cache_control: Option<serde_json::Value>,
    },
}

#[derive(Debug, Serialize)]
struct ApiImageSource {
    #[serde(rename = "type")]
    source_type: String,
    media_type: String,
    data: String,
}

#[derive(Debug, Serialize)]
struct ApiTool {
    name: String,
    description: String,
    input_schema: serde_json::Value,
    /// Optional `cache_control: {"type":"ephemeral"}` marker. When present
    /// on a tool block, Anthropic caches the system prompt AND the tool
    /// schema prefix up through that block. We stamp this on the *last*
    /// tool only when prompt caching is on, so the common (system + all
    /// tools) prefix is cached as one unit — the next call with the same
    /// tools list hits cache for the whole block.
    #[serde(skip_serializing_if = "Option::is_none")]
    cache_control: Option<serde_json::Value>,
}

/// Anthropic Messages API response body.
#[derive(Debug, Deserialize)]
struct ApiResponse {
    content: Vec<ResponseContentBlock>,
    stop_reason: String,
    usage: ApiUsage,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
enum ResponseContentBlock {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "tool_use")]
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
    },
    #[serde(rename = "thinking")]
    Thinking { thinking: String },
}

#[derive(Debug, Deserialize)]
struct ApiUsage {
    input_tokens: u64,
    output_tokens: u64,
    /// Tokens written to the prompt cache on this request.
    #[serde(default)]
    cache_creation_input_tokens: u64,
    /// Tokens read from the prompt cache on this request.
    #[serde(default)]
    cache_read_input_tokens: u64,
}

/// Anthropic API error response.
#[derive(Debug, Deserialize)]
struct ApiErrorResponse {
    error: ApiErrorDetail,
}

#[derive(Debug, Deserialize)]
struct ApiErrorDetail {
    message: String,
}

/// Accumulator for content blocks during streaming.
enum ContentBlockAccum {
    Text(String),
    Thinking(String),
    ToolUse {
        id: String,
        name: String,
        input_json: String,
    },
}

/// Build an `ApiRequest` from a `CompletionRequest`.
///
/// Shared between `complete()` and `stream()`.  The caller sets
/// the `stream` field on the returned struct before sending.
fn build_anthropic_request(request: &CompletionRequest) -> ApiRequest {
    // Extract system prompt from messages or use the provided one
    let mut system_text = request.system.clone().or_else(|| {
        request.messages.iter().find_map(|m| {
            if m.role == Role::System {
                match &m.content {
                    MessageContent::Text(t) => Some(t.clone()),
                    _ => None,
                }
            } else {
                None
            }
        })
    });

    // Anthropic has no native response_format field — inject instructions
    // into the system prompt when structured output is requested.
    if let Some(rf) = &request.response_format {
        append_response_format_instructions(&mut system_text, rf);
    }

    // Build the system field: structured blocks with cache_control when
    // prompt caching is enabled, plain string otherwise.
    let system = system_text.map(|text| build_system_value(&text, request.prompt_caching));

    // Build API messages, filtering out system messages
    let mut api_messages: Vec<ApiMessage> = request
        .messages
        .iter()
        .filter(|m| m.role != Role::System)
        .map(convert_message)
        .collect();

    // When prompt caching is on, stamp `cache_control: ephemeral` on the
    // last content block of the last message. Anthropic caches the prefix
    // up through that block — so the next turn (which shares everything
    // except a new trailing user message) hits cache for the whole
    // conversation history so far, not just system + tools. This is what
    // turns forkedAgent into a meaningful cost saver on multi-turn
    // conversations.
    //
    // Text-content messages (the common plain-string form) are upgraded
    // to a single-block `Blocks` payload so the marker has somewhere to
    // live — Anthropic doesn't allow cache_control on the shorthand
    // plain-string content form.
    if request.prompt_caching {
        if let Some(last_msg) = api_messages.last_mut() {
            stamp_last_block_cache_control(last_msg);
        }
    }

    // Build tools. When prompt caching is enabled, stamp `cache_control`
    // on the last tool so Anthropic caches the system + tools prefix as a
    // single unit. Without this, only the system block cached — tools
    // (often several KB of schema) were rewritten on every call.
    let tool_count = request.tools.len();
    let api_tools: Vec<ApiTool> = request
        .tools
        .iter()
        .enumerate()
        .map(|(idx, t)| {
            let is_last = idx + 1 == tool_count;
            ApiTool {
                name: t.name.clone(),
                description: t.description.clone(),
                input_schema: t.input_schema.clone(),
                cache_control: if request.prompt_caching && is_last {
                    Some(serde_json::json!({"type": "ephemeral"}))
                } else {
                    None
                },
            }
        })
        .collect();

    // Anthropic requires budget_tokens >= 1024 for extended thinking.
    // Skip thinking if budget is too low.
    let thinking_value = request
        .thinking
        .as_ref()
        .filter(|tc| tc.budget_tokens >= 1024)
        .map(|tc| {
            serde_json::json!({
                "type": "enabled",
                "budget_tokens": tc.budget_tokens
            })
        });

    // When thinking is enabled, max_tokens must be > budget_tokens.
    let effective_max_tokens = if let Some(ref tv) = thinking_value {
        let budget = tv["budget_tokens"].as_u64().unwrap_or(0) as u32;
        request.max_tokens.max(budget + 1024)
    } else {
        request.max_tokens
    };

    ApiRequest {
        model: request.model.clone(),
        max_tokens: effective_max_tokens,
        system,
        messages: api_messages,
        tools: api_tools,
        temperature: if thinking_value.is_some() {
            None
        } else {
            Some(request.temperature)
        },
        stream: false,
        thinking: thinking_value,
    }
}

#[async_trait]
impl LlmDriver for AnthropicDriver {
    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        let api_request = build_anthropic_request(&request);

        // Retry loop for rate limits and overloads
        let max_retries = 3;
        for attempt in 0..=max_retries {
            let url = format!("{}/v1/messages", self.base_url);
            debug!(url = %url, attempt, "Sending Anthropic API request");

            let resp = self
                .client
                .post(&url)
                .header("x-api-key", self.api_key.as_str())
                .header("anthropic-version", "2023-06-01")
                .header("content-type", "application/json")
                .json(&api_request)
                .send()
                .await
                .map_err(|e| LlmError::Http(e.to_string()))?;

            let status = resp.status().as_u16();

            if status == 429 || status == 529 {
                if attempt < max_retries {
                    let retry_ms = (attempt + 1) as u64 * 2000;
                    warn!(status, retry_ms, "Rate limited, retrying");
                    tokio::time::sleep(std::time::Duration::from_millis(retry_ms)).await;
                    continue;
                }
                return Err(if status == 429 {
                    LlmError::RateLimited {
                        retry_after_ms: 5000,
                        message: None,
                    }
                } else {
                    LlmError::Overloaded {
                        retry_after_ms: 5000,
                    }
                });
            }

            if !resp.status().is_success() {
                let body = resp.text().await.unwrap_or_default();
                let message = serde_json::from_str::<ApiErrorResponse>(&body)
                    .map(|e| e.error.message)
                    .unwrap_or(body);
                return Err(LlmError::Api { status, message });
            }

            let body = resp
                .text()
                .await
                .map_err(|e| LlmError::Http(e.to_string()))?;
            let api_response: ApiResponse =
                serde_json::from_str(&body).map_err(|e| LlmError::Parse(e.to_string()))?;

            return Ok(convert_response(api_response));
        }

        Err(LlmError::Api {
            status: 0,
            message: "Max retries exceeded".to_string(),
        })
    }

    async fn stream(
        &self,
        request: CompletionRequest,
        tx: tokio::sync::mpsc::Sender<StreamEvent>,
    ) -> Result<CompletionResponse, LlmError> {
        let mut api_request = build_anthropic_request(&request);
        api_request.stream = true;

        // Retry loop for the initial HTTP request
        let max_retries = 3;
        for attempt in 0..=max_retries {
            let url = format!("{}/v1/messages", self.base_url);
            debug!(url = %url, attempt, "Sending Anthropic streaming request");

            let resp = self
                .client
                .post(&url)
                .header("x-api-key", self.api_key.as_str())
                .header("anthropic-version", "2023-06-01")
                .header("content-type", "application/json")
                .json(&api_request)
                .send()
                .await
                .map_err(|e| LlmError::Http(e.to_string()))?;

            let status = resp.status().as_u16();

            if status == 429 || status == 529 {
                if attempt < max_retries {
                    let retry_ms = (attempt + 1) as u64 * 2000;
                    warn!(status, retry_ms, "Rate limited (stream), retrying");
                    tokio::time::sleep(std::time::Duration::from_millis(retry_ms)).await;
                    continue;
                }
                return Err(if status == 429 {
                    LlmError::RateLimited {
                        retry_after_ms: 5000,
                        message: None,
                    }
                } else {
                    LlmError::Overloaded {
                        retry_after_ms: 5000,
                    }
                });
            }

            if !resp.status().is_success() {
                let body = resp.text().await.unwrap_or_default();
                let message = serde_json::from_str::<ApiErrorResponse>(&body)
                    .map(|e| e.error.message)
                    .unwrap_or(body);
                return Err(LlmError::Api { status, message });
            }

            // Parse the SSE stream
            let mut buffer = String::new();
            let mut blocks: Vec<ContentBlockAccum> = Vec::new();
            let mut stop_reason = StopReason::EndTurn;
            let mut usage = TokenUsage::default();

            let mut byte_stream = resp.bytes_stream();
            while let Some(chunk_result) = byte_stream.next().await {
                let chunk = chunk_result.map_err(|e| LlmError::Http(e.to_string()))?;
                buffer.push_str(&String::from_utf8_lossy(&chunk));

                while let Some(pos) = buffer.find("\n\n") {
                    let event_text = buffer[..pos].to_string();
                    buffer = buffer[pos + 2..].to_string();

                    let mut event_type = String::new();
                    let mut data = String::new();
                    for line in event_text.lines() {
                        if let Some(et) = line.strip_prefix("event:") {
                            event_type = et.trim_start().to_string();
                        } else if let Some(d) = line.strip_prefix("data:") {
                            data = d.trim_start().to_string();
                        }
                    }

                    if data.is_empty() {
                        continue;
                    }

                    let json: serde_json::Value = match serde_json::from_str(&data) {
                        Ok(v) => v,
                        Err(_) => continue,
                    };

                    match event_type.as_str() {
                        "message_start" => {
                            let u = &json["message"]["usage"];
                            if let Some(it) = u["input_tokens"].as_u64() {
                                usage.input_tokens = it;
                            }
                            if let Some(ct) = u["cache_creation_input_tokens"].as_u64() {
                                usage.cache_creation_input_tokens = ct;
                            }
                            if let Some(cr) = u["cache_read_input_tokens"].as_u64() {
                                usage.cache_read_input_tokens = cr;
                            }
                        }
                        "content_block_start" => {
                            let block = &json["content_block"];
                            match block["type"].as_str().unwrap_or("") {
                                "text" => {
                                    blocks.push(ContentBlockAccum::Text(String::new()));
                                }
                                "tool_use" => {
                                    let id = block["id"].as_str().unwrap_or("").to_string();
                                    let name = block["name"].as_str().unwrap_or("").to_string();
                                    let _ = tx
                                        .send(StreamEvent::ToolUseStart {
                                            id: id.clone(),
                                            name: name.clone(),
                                        })
                                        .await;
                                    blocks.push(ContentBlockAccum::ToolUse {
                                        id,
                                        name,
                                        input_json: String::new(),
                                    });
                                }
                                "thinking" => {
                                    blocks.push(ContentBlockAccum::Thinking(String::new()));
                                }
                                _ => {}
                            }
                        }
                        "content_block_delta" => {
                            let block_idx = json["index"].as_u64().unwrap_or(0) as usize;
                            let delta = &json["delta"];
                            match delta["type"].as_str().unwrap_or("") {
                                "text_delta" => {
                                    if let Some(text) = delta["text"].as_str() {
                                        if let Some(ContentBlockAccum::Text(ref mut t)) =
                                            blocks.get_mut(block_idx)
                                        {
                                            t.push_str(text);
                                        }
                                        let _ = tx
                                            .send(StreamEvent::TextDelta {
                                                text: text.to_string(),
                                            })
                                            .await;
                                    }
                                }
                                "input_json_delta" => {
                                    if let Some(partial) = delta["partial_json"].as_str() {
                                        if let Some(ContentBlockAccum::ToolUse {
                                            ref mut input_json,
                                            ..
                                        }) = blocks.get_mut(block_idx)
                                        {
                                            input_json.push_str(partial);
                                        }
                                        let _ = tx
                                            .send(StreamEvent::ToolInputDelta {
                                                text: partial.to_string(),
                                            })
                                            .await;
                                    }
                                }
                                "thinking_delta" => {
                                    if let Some(thinking) = delta["thinking"].as_str() {
                                        if let Some(ContentBlockAccum::Thinking(ref mut t)) =
                                            blocks.get_mut(block_idx)
                                        {
                                            t.push_str(thinking);
                                        }
                                        let _ = tx
                                            .send(StreamEvent::ThinkingDelta {
                                                text: thinking.to_string(),
                                            })
                                            .await;
                                    }
                                }
                                _ => {}
                            }
                        }
                        "content_block_stop" => {
                            let block_idx = json["index"].as_u64().unwrap_or(0) as usize;
                            if let Some(ContentBlockAccum::ToolUse {
                                id,
                                name,
                                input_json,
                            }) = blocks.get(block_idx)
                            {
                                let input: serde_json::Value = match serde_json::from_str::<
                                    serde_json::Value,
                                >(
                                    input_json
                                ) {
                                    Ok(v) => ensure_object(v),
                                    Err(e) => {
                                        tracing::warn!(
                                            tool = %name,
                                            raw_args_len = input_json.len(),
                                            error = %e,
                                            "Malformed tool call arguments from Anthropic stream"
                                        );
                                        super::openai::malformed_tool_input(&e, input_json.len())
                                    }
                                };
                                let _ = tx
                                    .send(StreamEvent::ToolUseEnd {
                                        id: id.clone(),
                                        name: name.clone(),
                                        input,
                                    })
                                    .await;
                            }
                        }
                        "message_delta" => {
                            if let Some(sr) = json["delta"]["stop_reason"].as_str() {
                                stop_reason = match sr {
                                    "end_turn" => StopReason::EndTurn,
                                    "tool_use" => StopReason::ToolUse,
                                    "max_tokens" => StopReason::MaxTokens,
                                    "stop_sequence" => StopReason::StopSequence,
                                    _ => StopReason::EndTurn,
                                };
                            }
                            if let Some(ot) = json["usage"]["output_tokens"].as_u64() {
                                usage.output_tokens = ot;
                            }
                        }
                        _ => {} // message_stop, ping, etc.
                    }
                }
            }

            // Build CompletionResponse from accumulated blocks
            let mut content = Vec::new();
            let mut tool_calls = Vec::new();
            for block in blocks {
                match block {
                    ContentBlockAccum::Text(text) => {
                        content.push(ContentBlock::Text {
                            text,
                            provider_metadata: None,
                        });
                    }
                    ContentBlockAccum::Thinking(thinking) => {
                        content.push(ContentBlock::Thinking {
                            thinking,
                            provider_metadata: None,
                        });
                    }
                    ContentBlockAccum::ToolUse {
                        id,
                        name,
                        input_json,
                    } => {
                        let input: serde_json::Value =
                            match serde_json::from_str::<serde_json::Value>(&input_json) {
                                Ok(v) => ensure_object(v),
                                Err(e) => {
                                    tracing::warn!(
                                        tool = %name,
                                        raw_args_len = input_json.len(),
                                        error = %e,
                                        "Malformed tool call arguments from Anthropic"
                                    );
                                    super::openai::malformed_tool_input(&e, input_json.len())
                                }
                            };
                        content.push(ContentBlock::ToolUse {
                            id: id.clone(),
                            name: name.clone(),
                            input: input.clone(),
                            provider_metadata: None,
                        });
                        tool_calls.push(ToolCall { id, name, input });
                    }
                }
            }

            let _ = tx
                .send(StreamEvent::ContentComplete { stop_reason, usage })
                .await;

            return Ok(CompletionResponse {
                content,
                stop_reason,
                tool_calls,
                usage,
            });
        }

        Err(LlmError::Api {
            status: 0,
            message: "Max retries exceeded".to_string(),
        })
    }
}

/// Ensure a `serde_json::Value` is an object.  The Anthropic API requires the
/// `input` field of `tool_use` blocks to be a JSON object (`{}`), never `null`.
///
/// Handles several malformed-input scenarios that occur when models hallucinate
/// or return non-standard tool calls:
///
/// - `null` → `{}`
/// - A JSON string that parses as an object → use the parsed object
/// - Any other type (string, number, array, bool) → `{"raw_input": <value>}`
///   so the original value is preserved for debugging rather than silently lost.
fn ensure_object(v: serde_json::Value) -> serde_json::Value {
    match v {
        serde_json::Value::Object(_) => v,
        serde_json::Value::Null => serde_json::json!({}),
        serde_json::Value::String(ref s) => {
            // The model may return a JSON-encoded string instead of a proper
            // object — attempt to parse it.
            if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(s) {
                if parsed.is_object() {
                    warn!("Tool input was a JSON string instead of an object, parsed successfully");
                    return parsed;
                }
            }
            warn!(value = %s, "Tool input was a non-parseable string, wrapping in raw_input");
            serde_json::json!({"raw_input": v})
        }
        other => {
            warn!(value = ?other, "Tool input was not an object, wrapping in raw_input");
            serde_json::json!({"raw_input": other})
        }
    }
}

/// Build the `system` field value for the Anthropic API request.
///
/// When prompt caching is enabled, returns a JSON array of content blocks
/// with `cache_control: {"type": "ephemeral"}` on the last block so that
/// Anthropic caches the system prompt prefix.  When disabled, returns a
/// plain JSON string.
/// Append structured-output instructions to the system prompt for Anthropic.
///
/// Anthropic does not have a native `response_format` field, so we inject
/// formatting instructions into the system prompt instead.
fn append_response_format_instructions(system: &mut Option<String>, rf: &ResponseFormat) {
    match rf {
        ResponseFormat::Text => {} // nothing to do
        ResponseFormat::Json => {
            let suffix = "\n\nIMPORTANT: You MUST respond with valid JSON only. \
                           Do not include any text outside the JSON object.";
            if let Some(s) = system.as_mut() {
                s.push_str(suffix);
            } else {
                *system = Some(suffix.trim_start().to_string());
            }
        }
        ResponseFormat::JsonSchema {
            name,
            schema,
            strict: _,
        } => {
            let suffix = format!(
                "\n\nIMPORTANT: You MUST respond with valid JSON that conforms to the \
                 following schema (name: \"{name}\"):\n```json\n{schema}\n```\n\
                 Do not include any text outside the JSON object."
            );
            if let Some(s) = system.as_mut() {
                s.push_str(&suffix);
            } else {
                *system = Some(suffix.trim_start().to_string());
            }
        }
    }
}

/// Stamp `cache_control: ephemeral` on the last content block of this
/// message. If the message is in the plain-string form (`ApiContent::Text`),
/// upgrade it to a single-element `Blocks` payload first — Anthropic's
/// API only accepts `cache_control` on structured content blocks, not on
/// shorthand strings.
fn stamp_last_block_cache_control(msg: &mut ApiMessage) {
    let marker = serde_json::json!({"type": "ephemeral"});
    // Upgrade plain string content to block form so the marker has a
    // block to live on. This is a lossless transformation — Anthropic
    // treats `{type: text, text: "…"}` identically to the raw string.
    if let ApiContent::Text(text) = &msg.content {
        let text = text.clone();
        msg.content = ApiContent::Blocks(vec![ApiContentBlock::Text {
            text,
            cache_control: Some(marker),
        }]);
        return;
    }
    if let ApiContent::Blocks(blocks) = &mut msg.content {
        // Walk backward to the last serializable block (some Thinking-like
        // entries were already filtered out during convert_message, so the
        // raw tail is fine to mark). Empty `Blocks` → nothing to do;
        // downstream code already handles zero-block messages, marker
        // would have nowhere to live.
        if let Some(last) = blocks.last_mut() {
            match last {
                ApiContentBlock::Text { cache_control, .. }
                | ApiContentBlock::Image { cache_control, .. }
                | ApiContentBlock::ToolUse { cache_control, .. }
                | ApiContentBlock::ToolResult { cache_control, .. } => {
                    *cache_control = Some(marker);
                }
            }
        }
    }
}

fn build_system_value(text: &str, prompt_caching: bool) -> serde_json::Value {
    if prompt_caching {
        serde_json::json!([
            {
                "type": "text",
                "text": text,
                "cache_control": {"type": "ephemeral"}
            }
        ])
    } else {
        serde_json::Value::String(text.to_string())
    }
}

/// Convert an LibreFang Message to an Anthropic API message.
fn convert_message(msg: &Message) -> ApiMessage {
    let role = match msg.role {
        Role::User => "user",
        Role::Assistant => "assistant",
        Role::System => "user", // Should be filtered out, but handle gracefully
    };

    let content = match &msg.content {
        MessageContent::Text(text) => ApiContent::Text(text.clone()),
        MessageContent::Blocks(blocks) => {
            let api_blocks: Vec<ApiContentBlock> = blocks
                .iter()
                .filter_map(|block| match block {
                    ContentBlock::Text { text, .. } => Some(ApiContentBlock::Text {
                        text: text.clone(),
                        cache_control: None,
                    }),
                    ContentBlock::Image { media_type, data } => Some(ApiContentBlock::Image {
                        source: ApiImageSource {
                            source_type: "base64".to_string(),
                            media_type: media_type.clone(),
                            data: data.clone(),
                        },
                        cache_control: None,
                    }),
                    ContentBlock::ToolUse {
                        id, name, input, ..
                    } => Some(ApiContentBlock::ToolUse {
                        id: id.clone(),
                        name: name.clone(),
                        input: ensure_object(input.clone()),
                        cache_control: None,
                    }),
                    ContentBlock::ToolResult {
                        tool_use_id,
                        content,
                        is_error,
                        ..
                    } => Some(ApiContentBlock::ToolResult {
                        tool_use_id: tool_use_id.clone(),
                        content: content.clone(),
                        is_error: *is_error,
                        cache_control: None,
                    }),
                    ContentBlock::Thinking { .. } => None,
                    ContentBlock::ImageFile { media_type, path } => match std::fs::read(path) {
                        Ok(bytes) => {
                            use base64::Engine;
                            let data = base64::engine::general_purpose::STANDARD.encode(&bytes);
                            Some(ApiContentBlock::Image {
                                source: ApiImageSource {
                                    source_type: "base64".to_string(),
                                    media_type: media_type.clone(),
                                    data,
                                },
                                cache_control: None,
                            })
                        }
                        Err(e) => {
                            warn!(path = %path, error = %e, "ImageFile missing, skipping");
                            None
                        }
                    },
                    ContentBlock::Unknown => None,
                })
                .collect();
            ApiContent::Blocks(api_blocks)
        }
    };

    ApiMessage {
        role: role.to_string(),
        content,
    }
}

/// Convert an Anthropic API response to our CompletionResponse.
fn convert_response(api: ApiResponse) -> CompletionResponse {
    let mut content = Vec::new();
    let mut tool_calls = Vec::new();

    for block in api.content {
        match block {
            ResponseContentBlock::Text { text } => {
                content.push(ContentBlock::Text {
                    text,
                    provider_metadata: None,
                });
            }
            ResponseContentBlock::ToolUse { id, name, input } => {
                let input = ensure_object(input);
                content.push(ContentBlock::ToolUse {
                    id: id.clone(),
                    name: name.clone(),
                    input: input.clone(),
                    provider_metadata: None,
                });
                tool_calls.push(ToolCall { id, name, input });
            }
            ResponseContentBlock::Thinking { thinking } => {
                content.push(ContentBlock::Thinking {
                    thinking,
                    provider_metadata: None,
                });
            }
        }
    }

    let stop_reason = match api.stop_reason.as_str() {
        "end_turn" => StopReason::EndTurn,
        "tool_use" => StopReason::ToolUse,
        "max_tokens" => StopReason::MaxTokens,
        "stop_sequence" => StopReason::StopSequence,
        _ => StopReason::EndTurn,
    };

    CompletionResponse {
        content,
        stop_reason,
        tool_calls,
        usage: TokenUsage {
            input_tokens: api.usage.input_tokens,
            output_tokens: api.usage.output_tokens,
            cache_creation_input_tokens: api.usage.cache_creation_input_tokens,
            cache_read_input_tokens: api.usage.cache_read_input_tokens,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use librefang_types::tool::ToolDefinition;

    #[test]
    fn test_convert_message_text() {
        let msg = Message::user("Hello");
        let api_msg = convert_message(&msg);
        assert_eq!(api_msg.role, "user");
    }

    #[test]
    fn test_convert_response() {
        let api_response = ApiResponse {
            content: vec![
                ResponseContentBlock::Text {
                    text: "I'll help you.".to_string(),
                },
                ResponseContentBlock::ToolUse {
                    id: "tool_1".to_string(),
                    name: "web_search".to_string(),
                    input: serde_json::json!({"query": "rust lang"}),
                },
            ],
            stop_reason: "tool_use".to_string(),
            usage: ApiUsage {
                input_tokens: 100,
                output_tokens: 50,
                cache_creation_input_tokens: 0,
                cache_read_input_tokens: 0,
            },
        };

        let response = convert_response(api_response);
        assert_eq!(response.stop_reason, StopReason::ToolUse);
        assert_eq!(response.tool_calls.len(), 1);
        assert_eq!(response.tool_calls[0].name, "web_search");
        assert_eq!(response.usage.total(), 150);
    }

    #[test]
    fn test_build_system_value_plain() {
        let val = build_system_value("You are helpful.", false);
        assert_eq!(val.as_str().unwrap(), "You are helpful.");
    }

    #[test]
    fn test_build_system_value_cached() {
        let val = build_system_value("You are helpful.", true);
        let arr = val.as_array().unwrap();
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["type"], "text");
        assert_eq!(arr[0]["text"], "You are helpful.");
        assert_eq!(arr[0]["cache_control"]["type"], "ephemeral");
    }

    #[test]
    fn test_ensure_object_null_becomes_empty_object() {
        let result = ensure_object(serde_json::Value::Null);
        assert_eq!(result, serde_json::json!({}));
    }

    #[test]
    fn test_ensure_object_preserves_existing_object() {
        let input = serde_json::json!({"query": "rust lang"});
        let result = ensure_object(input.clone());
        assert_eq!(result, input);
    }

    #[test]
    fn test_ensure_object_non_object_wraps_in_raw_input() {
        assert_eq!(
            ensure_object(serde_json::json!("plain string")),
            serde_json::json!({"raw_input": "plain string"})
        );
        assert_eq!(
            ensure_object(serde_json::json!(42)),
            serde_json::json!({"raw_input": 42})
        );
        assert_eq!(
            ensure_object(serde_json::json!([1, 2, 3])),
            serde_json::json!({"raw_input": [1, 2, 3]})
        );
    }

    #[test]
    fn test_ensure_object_string_containing_json_object_is_parsed() {
        let input = serde_json::json!(r#"{"query": "rust lang"}"#);
        let result = ensure_object(input);
        assert_eq!(result, serde_json::json!({"query": "rust lang"}));
    }

    #[test]
    fn test_ensure_object_string_containing_json_array_wraps() {
        // A string that parses as JSON but not as an object should be wrapped
        let input = serde_json::json!(r#"[1, 2, 3]"#);
        let result = ensure_object(input);
        assert_eq!(result, serde_json::json!({"raw_input": "[1, 2, 3]"}));
    }

    #[test]
    fn test_ensure_object_bool_wraps_in_raw_input() {
        assert_eq!(
            ensure_object(serde_json::json!(true)),
            serde_json::json!({"raw_input": true})
        );
    }

    #[test]
    fn test_parameterless_tool_use_serializes_empty_object() {
        let block = ApiContentBlock::ToolUse {
            id: "tool_1".to_string(),
            name: "get_time".to_string(),
            input: ensure_object(serde_json::Value::Null),
            cache_control: None,
        };
        let json = serde_json::to_value(&block).unwrap();
        assert_eq!(json["input"], serde_json::json!({}));
    }

    #[test]
    fn test_convert_message_null_tool_use_input_becomes_empty_object() {
        let msg = Message {
            role: Role::Assistant,
            content: MessageContent::Blocks(vec![ContentBlock::ToolUse {
                id: "tool_1".to_string(),
                name: "get_time".to_string(),
                input: serde_json::Value::Null,
                provider_metadata: None,
            }]),
            pinned: false,
        };
        let api_msg = convert_message(&msg);
        match api_msg.content {
            ApiContent::Blocks(blocks) => {
                assert_eq!(blocks.len(), 1);
                let json = serde_json::to_value(&blocks[0]).unwrap();
                assert_eq!(json["input"], serde_json::json!({}));
            }
            _ => panic!("Expected Blocks content"),
        }
    }

    #[test]
    fn test_convert_response_null_tool_input_becomes_empty_object() {
        let api_response = ApiResponse {
            content: vec![ResponseContentBlock::ToolUse {
                id: "tool_1".to_string(),
                name: "get_time".to_string(),
                input: serde_json::Value::Null,
            }],
            stop_reason: "tool_use".to_string(),
            usage: ApiUsage {
                input_tokens: 10,
                output_tokens: 5,
                cache_creation_input_tokens: 0,
                cache_read_input_tokens: 0,
            },
        };

        let response = convert_response(api_response);
        assert_eq!(response.tool_calls.len(), 1);
        assert_eq!(response.tool_calls[0].input, serde_json::json!({}));
        match &response.content[0] {
            ContentBlock::ToolUse { input, .. } => {
                assert_eq!(*input, serde_json::json!({}));
            }
            _ => panic!("Expected ToolUse content block"),
        }
    }

    /// With prompt_caching enabled, the LAST tool in the request must carry
    /// `cache_control: ephemeral`; preceding tools must not. This means the
    /// (system + tools) prefix is cached as one unit — the common expensive
    /// part that derivative calls can reuse.
    #[test]
    fn test_tools_cache_control_on_last_only() {
        let tool_a = ToolDefinition {
            name: "alpha".to_string(),
            description: "first".to_string(),
            input_schema: serde_json::json!({"type":"object"}),
        };
        let tool_b = ToolDefinition {
            name: "beta".to_string(),
            description: "second".to_string(),
            input_schema: serde_json::json!({"type":"object"}),
        };
        let request = CompletionRequest {
            model: "claude-sonnet-4-5".to_string(),
            messages: vec![Message::user("hi")],
            tools: vec![tool_a, tool_b],
            max_tokens: 100,
            temperature: 0.0,
            system: Some("sys".to_string()),
            thinking: None,
            prompt_caching: true,
            response_format: None,
            timeout_secs: None,
            extra_body: None,
            agent_id: None,
        };
        let api_request = build_anthropic_request(&request);
        assert_eq!(api_request.tools.len(), 2);
        assert!(
            api_request.tools[0].cache_control.is_none(),
            "first tool must NOT have cache_control",
        );
        let last_cc = api_request.tools[1]
            .cache_control
            .as_ref()
            .expect("last tool must have cache_control");
        assert_eq!(last_cc["type"], "ephemeral");
    }

    /// With prompt_caching disabled, no tool gets cache_control. Ensures
    /// we don't accidentally leak cache markers to providers that can't
    /// handle them or incur unintended cost-accounting.
    #[test]
    fn test_tools_cache_control_absent_when_caching_off() {
        let tool = ToolDefinition {
            name: "only".to_string(),
            description: "solo".to_string(),
            input_schema: serde_json::json!({"type":"object"}),
        };
        let request = CompletionRequest {
            model: "claude-sonnet-4-5".to_string(),
            messages: vec![Message::user("hi")],
            tools: vec![tool],
            max_tokens: 100,
            temperature: 0.0,
            system: Some("sys".to_string()),
            thinking: None,
            prompt_caching: false,
            response_format: None,
            timeout_secs: None,
            extra_body: None,
            agent_id: None,
        };
        let api_request = build_anthropic_request(&request);
        assert!(api_request.tools[0].cache_control.is_none());
    }

    /// Last message gets cache_control: ephemeral on its last content block
    /// when prompt caching is on. This is what turns forkedAgent into a real
    /// cost saver for multi-turn conversations — the full message prefix
    /// caches, not just system + tools.
    #[test]
    fn test_messages_cache_control_on_last_block() {
        let request = CompletionRequest {
            model: "claude-sonnet-4-5".to_string(),
            messages: vec![
                Message::user("first user msg"),
                Message::assistant("first assistant reply"),
                Message::user("second user msg (last)"),
            ],
            tools: vec![],
            max_tokens: 100,
            temperature: 0.0,
            system: Some("sys".to_string()),
            thinking: None,
            prompt_caching: true,
            response_format: None,
            timeout_secs: None,
            extra_body: None,
            agent_id: None,
        };
        let api_request = build_anthropic_request(&request);
        let last = api_request.messages.last().expect("has last message");
        // Plain-string Text content was upgraded to Blocks form so the
        // marker has somewhere to live. Anthropic rejects cache_control
        // on shorthand strings, so this upgrade is load-bearing.
        let blocks = match &last.content {
            ApiContent::Blocks(b) => b,
            ApiContent::Text(_) => {
                panic!("cache_control on last msg requires Blocks form, got Text")
            }
        };
        let last_block = blocks.last().expect("has last block");
        let cc = match last_block {
            ApiContentBlock::Text { cache_control, .. } => cache_control.as_ref(),
            _ => panic!("expected Text block"),
        };
        assert_eq!(cc.expect("cache_control set")["type"], "ephemeral");

        // Only the LAST message's last block is marked — earlier messages
        // would split the cache into fragments and waste the 4-breakpoint
        // budget Anthropic allows per request.
        let first = &api_request.messages[0];
        if let ApiContent::Blocks(blocks) = &first.content {
            for block in blocks {
                if let ApiContentBlock::Text { cache_control, .. } = block {
                    assert!(cache_control.is_none(), "first message must NOT be marked");
                }
            }
        }
    }

    /// With caching disabled, no message block gets cache_control — ensures
    /// we don't leak markers to providers that can't handle them (or incur
    /// cache-cost billing on providers that do).
    #[test]
    fn test_messages_cache_control_absent_when_caching_off() {
        let request = CompletionRequest {
            model: "claude-sonnet-4-5".to_string(),
            messages: vec![Message::user("hi")],
            tools: vec![],
            max_tokens: 100,
            temperature: 0.0,
            system: Some("sys".to_string()),
            thinking: None,
            prompt_caching: false,
            response_format: None,
            timeout_secs: None,
            extra_body: None,
            agent_id: None,
        };
        let api_request = build_anthropic_request(&request);
        let last = api_request.messages.last().expect("has last message");
        // With caching off, plain Text stays plain Text — we don't
        // eagerly upgrade to Blocks form because that would change
        // the wire format for no benefit.
        match &last.content {
            ApiContent::Text(_) => { /* expected */ }
            ApiContent::Blocks(_) => panic!("expected Text form when caching off"),
        }
    }

    /// With caching on but zero tools, the request still builds cleanly
    /// — the `is_last` check must not underflow or special-case an empty
    /// list. Skipping this test once hid a bug where `tool_count - 1`
    /// produced an out-of-range index on empty input.
    #[test]
    fn test_tools_cache_control_empty_tools_list() {
        let request = CompletionRequest {
            model: "claude-sonnet-4-5".to_string(),
            messages: vec![Message::user("hi")],
            tools: vec![],
            max_tokens: 100,
            temperature: 0.0,
            system: Some("sys".to_string()),
            thinking: None,
            prompt_caching: true,
            response_format: None,
            timeout_secs: None,
            extra_body: None,
            agent_id: None,
        };
        let api_request = build_anthropic_request(&request);
        assert!(api_request.tools.is_empty());
    }
}
