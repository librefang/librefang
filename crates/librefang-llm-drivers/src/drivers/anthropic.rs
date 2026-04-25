//! Anthropic Claude API driver.
//!
//! Full implementation of the Anthropic Messages API with tool use support,
//! system prompt extraction, and retry on 429/529 errors.

use crate::backoff::standard_retry_delay;
use crate::llm_driver::{
    CompletionRequest, CompletionResponse, LlmDriver, LlmError, LlmFamily, StreamEvent,
};
use crate::rate_limit_tracker::RateLimitSnapshot;
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
    /// Per-provider HTTP request timeout in seconds.
    /// Overrides the HTTP client's default read timeout when set.
    request_timeout_secs: Option<u64>,
}

impl AnthropicDriver {
    /// Create a new Anthropic driver.
    pub fn new(api_key: String, base_url: String) -> Self {
        Self::with_proxy(api_key, base_url, None)
    }

    /// Create a new Anthropic driver with an optional per-provider proxy.
    pub fn with_proxy(api_key: String, base_url: String, proxy_url: Option<&str>) -> Self {
        Self::with_proxy_and_timeout(api_key, base_url, proxy_url, None)
    }

    /// Create a new Anthropic driver with optional proxy and request timeout.
    pub fn with_proxy_and_timeout(
        api_key: String,
        base_url: String,
        proxy_url: Option<&str>,
        request_timeout_secs: Option<u64>,
    ) -> Self {
        let client = match proxy_url {
            Some(url) => librefang_http::proxied_client_with_override(url),
            None => librefang_http::proxied_client(),
        };
        Self {
            api_key: Zeroizing::new(api_key),
            base_url,
            client,
            request_timeout_secs,
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

    // Resolve cache TTL once. `None` here means caching is disabled for
    // this request and no markers should be written anywhere.
    let cache_ttl = if request.prompt_caching {
        Some(CacheTtl::from_request_field(request.cache_ttl))
    } else {
        None
    };

    // Build the system field: structured blocks with cache_control when
    // prompt caching is enabled, plain string otherwise.
    let system = system_text.map(|text| build_system_value(&text, cache_ttl));

    // Build API messages, filtering out system messages.
    let mut api_messages: Vec<ApiMessage> = request
        .messages
        .iter()
        .filter(|m| m.role != Role::System)
        .map(convert_message)
        .collect();

    // Build tools. When caching is on, stamp the last tool so Anthropic
    // caches the (system + tools) prefix as a single unit — without this
    // the multi-KB tool schemas would be rewritten on every call.
    let tool_count = request.tools.len();
    let has_tools = tool_count > 0;
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
                cache_control: match cache_ttl {
                    Some(ttl) if is_last => Some(ttl.to_marker()),
                    _ => None,
                },
            }
        })
        .collect();

    // Apply system_and_3 rolling-window markers on the message list.
    // Anthropic allows up to 4 cache_control breakpoints per request,
    // counted across system + tools + messages. We've already used 1 for
    // system and (when present) 1 for the last tool, leaving 2-3 slots
    // for the trailing messages. This is what makes mid-conversation
    // tool_use/tool_result rounds cache-eligible — the previous turn's
    // tail enters the prefix instead of being re-billed every call.
    if let Some(ttl) = cache_ttl {
        apply_cache_markers_system_and_3(&mut api_messages, has_tools, ttl);
    }

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

    // Anthropic rejects max_tokens=0 with HTTP 400; fall back to a safe
    // minimum so callers that forget to set max_tokens still work.
    let effective_max_tokens = if effective_max_tokens == 0 {
        warn!(
            model = %request.model,
            "max_tokens resolved to 0, falling back to safe minimum of 8192"
        );
        8192
    } else {
        effective_max_tokens
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

            let mut req_builder = self
                .client
                .post(&url)
                .header("x-api-key", self.api_key.as_str())
                .header("anthropic-version", "2023-06-01")
                .header("content-type", "application/json");
            if request_uses_1h_cache(&request) {
                req_builder = req_builder.header("anthropic-beta", ANTHROPIC_1H_CACHE_BETA);
            }
            let mut req_builder = req_builder.json(&api_request);
            if let Some(secs) = self.request_timeout_secs {
                req_builder = req_builder.timeout(std::time::Duration::from_secs(secs));
            }
            let resp = req_builder
                .send()
                .await
                .map_err(|e| LlmError::Http(e.to_string()))?;

            let status = resp.status().as_u16();

            if status == 429 || status == 529 {
                if attempt < max_retries {
                    let retry_after = resp
                        .headers()
                        .get("retry-after")
                        .and_then(|v| v.to_str().ok())
                        .and_then(|s| s.parse::<u64>().ok())
                        .map(std::time::Duration::from_secs)
                        .unwrap_or(std::time::Duration::ZERO);
                    let delay = standard_retry_delay(attempt + 1, retry_after);
                    warn!(
                        status,
                        delay_ms = delay.as_millis(),
                        "Rate limited, retrying"
                    );
                    tokio::time::sleep(delay).await;
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

            // Extract and log rate limit headers before consuming the response body.
            if let Some(snap) = RateLimitSnapshot::from_headers(resp.headers()) {
                if snap.has_warning() {
                    warn!(
                        target: "librefang::rate_limit",
                        "Anthropic rate limit warning:\n{}",
                        snap.display()
                    );
                } else {
                    debug!(
                        target: "librefang::rate_limit",
                        "Anthropic rate limits OK:\n{}",
                        snap.display()
                    );
                }
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

            let mut req_builder = self
                .client
                .post(&url)
                .header("x-api-key", self.api_key.as_str())
                .header("anthropic-version", "2023-06-01")
                .header("content-type", "application/json");
            if request_uses_1h_cache(&request) {
                req_builder = req_builder.header("anthropic-beta", ANTHROPIC_1H_CACHE_BETA);
            }
            let mut req_builder = req_builder.json(&api_request);
            if let Some(secs) = self.request_timeout_secs {
                req_builder = req_builder.timeout(std::time::Duration::from_secs(secs));
            }
            let resp = req_builder
                .send()
                .await
                .map_err(|e| LlmError::Http(e.to_string()))?;

            let status = resp.status().as_u16();

            if status == 429 || status == 529 {
                if attempt < max_retries {
                    let retry_after = resp
                        .headers()
                        .get("retry-after")
                        .and_then(|v| v.to_str().ok())
                        .and_then(|s| s.parse::<u64>().ok())
                        .map(std::time::Duration::from_secs)
                        .unwrap_or(std::time::Duration::ZERO);
                    let delay = standard_retry_delay(attempt + 1, retry_after);
                    warn!(
                        status,
                        delay_ms = delay.as_millis(),
                        "Rate limited (stream), retrying"
                    );
                    tokio::time::sleep(delay).await;
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

            // Extract and log rate limit headers before consuming the stream.
            if let Some(snap) = RateLimitSnapshot::from_headers(resp.headers()) {
                if snap.has_warning() {
                    warn!(
                        target: "librefang::rate_limit",
                        "Anthropic rate limit warning (stream):\n{}",
                        snap.display()
                    );
                } else {
                    debug!(
                        target: "librefang::rate_limit",
                        "Anthropic rate limits OK (stream):\n{}",
                        snap.display()
                    );
                }
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
                                let input: serde_json::Value = match super::openai::parse_tool_args(
                                    input_json,
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
                            match super::openai::parse_tool_args(&input_json) {
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

    fn family(&self) -> LlmFamily {
        LlmFamily::Anthropic
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

/// Cache TTL hint for Anthropic prompt caching. `Short` is the default
/// 5-minute ephemeral cache; `Long` is the 1-hour cache (gated by the
/// `extended-cache-ttl-2025-04-11` beta header — driver attaches it).
#[derive(Clone, Copy, PartialEq, Eq)]
enum CacheTtl {
    Short,
    Long,
}

impl CacheTtl {
    /// Resolve the TTL from the user-facing `cache_ttl` field. Anything
    /// other than `Some("1h")` collapses to the default 5-minute window.
    fn from_request_field(field: Option<&'static str>) -> Self {
        match field {
            Some("1h") => Self::Long,
            _ => Self::Short,
        }
    }

    /// JSON marker to write into a `cache_control` slot.
    fn to_marker(self) -> serde_json::Value {
        match self {
            Self::Short => serde_json::json!({"type": "ephemeral"}),
            Self::Long => serde_json::json!({"type": "ephemeral", "ttl": "1h"}),
        }
    }
}

/// Beta header required for 1-hour prompt cache TTL. See
/// <https://docs.anthropic.com/en/docs/build-with-claude/prompt-caching#1-hour-cache-duration-beta>.
const ANTHROPIC_1H_CACHE_BETA: &str = "extended-cache-ttl-2025-04-11";

/// Whether this request needs the 1h cache beta header.
fn request_uses_1h_cache(req: &CompletionRequest) -> bool {
    req.prompt_caching && matches!(req.cache_ttl, Some("1h"))
}

/// Apply `system_and_3` rolling-window cache markers on the trailing
/// messages.
///
/// Anthropic allows at most 4 `cache_control` breakpoints per request,
/// counted across system + tools + messages. The system block always
/// consumes 1; the tools-last marker consumes another when tools are
/// non-empty. This function fills the remaining slots from the tail of
/// the message list — newest first, so the cached prefix always covers
/// the maximum amount of recent history.
fn apply_cache_markers_system_and_3(
    api_messages: &mut [ApiMessage],
    has_tools: bool,
    ttl: CacheTtl,
) {
    let used_outside = 1usize + if has_tools { 1 } else { 0 }; // system [+ tools]
    let remaining = 4usize.saturating_sub(used_outside); // 2 or 3
    let n = api_messages.len();
    let take = remaining.min(n);
    if take == 0 {
        return;
    }
    let marker = ttl.to_marker();
    for msg in api_messages.iter_mut().skip(n - take) {
        stamp_block_with_marker(msg, &marker);
    }
}

/// Stamp `marker` on the last content block of this message. If the
/// message uses the plain-string `ApiContent::Text` form, upgrade it to
/// a single-element `Blocks` payload first — Anthropic only accepts
/// `cache_control` on structured content blocks, not on shorthand
/// strings. This upgrade is a lossless wire-format change.
fn stamp_block_with_marker(msg: &mut ApiMessage, marker: &serde_json::Value) {
    if let ApiContent::Text(text) = &msg.content {
        let text = text.clone();
        msg.content = ApiContent::Blocks(vec![ApiContentBlock::Text {
            text,
            cache_control: Some(marker.clone()),
        }]);
        return;
    }
    if let ApiContent::Blocks(blocks) = &mut msg.content {
        // Empty `Blocks` → nothing to mark; downstream already handles
        // zero-block messages and the marker would have nowhere to live.
        // Thinking blocks were already filtered out by `convert_message`,
        // so any block reachable here can safely carry `cache_control`.
        if let Some(last) = blocks.last_mut() {
            match last {
                ApiContentBlock::Text { cache_control, .. }
                | ApiContentBlock::Image { cache_control, .. }
                | ApiContentBlock::ToolUse { cache_control, .. }
                | ApiContentBlock::ToolResult { cache_control, .. } => {
                    *cache_control = Some(marker.clone());
                }
            }
        }
    }
}

/// Render the system field. When caching is disabled (`ttl: None`) the
/// shorthand string form is used; otherwise the value is upgraded to a
/// single-element block array carrying the cache marker.
fn build_system_value(text: &str, ttl: Option<CacheTtl>) -> serde_json::Value {
    match ttl {
        Some(t) => {
            let marker = t.to_marker();
            serde_json::json!([
                {
                    "type": "text",
                    "text": text,
                    "cache_control": marker,
                }
            ])
        }
        None => serde_json::Value::String(text.to_string()),
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
    fn test_anthropic_driver_family_is_anthropic() {
        let driver = AnthropicDriver::new(
            "test-key".to_string(),
            "https://api.anthropic.com".to_string(),
        );
        assert_eq!(driver.family(), LlmFamily::Anthropic);
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
        let val = build_system_value("You are helpful.", None);
        assert_eq!(val.as_str().unwrap(), "You are helpful.");
    }

    #[test]
    fn test_build_system_value_cached() {
        let val = build_system_value("You are helpful.", Some(CacheTtl::Short));
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
            timestamp: None,
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
            cache_ttl: None,
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
            cache_ttl: None,
            response_format: None,
            timeout_secs: None,
            extra_body: None,
            agent_id: None,
        };
        let api_request = build_anthropic_request(&request);
        assert!(api_request.tools[0].cache_control.is_none());
    }

    /// Helper: extract the cache_control marker from a message's last block,
    /// or `None` if the message is in plain-string form (no marker possible).
    fn last_block_cache_control(msg: &ApiMessage) -> Option<&serde_json::Value> {
        let blocks = match &msg.content {
            ApiContent::Blocks(b) => b,
            ApiContent::Text(_) => return None,
        };
        match blocks.last()? {
            ApiContentBlock::Text { cache_control, .. }
            | ApiContentBlock::Image { cache_control, .. }
            | ApiContentBlock::ToolUse { cache_control, .. }
            | ApiContentBlock::ToolResult { cache_control, .. } => cache_control.as_ref(),
        }
    }

    /// In a 5-turn conversation with no tools, system_and_3 fills 3 message
    /// breakpoints (the 1 system slot + 3 message slots = 4 total, the
    /// Anthropic per-request cap). Messages [2..=4] (trailing 3) carry the
    /// marker; [0..=1] do not.
    #[test]
    fn multi_turn_rolling_window_stamps_last_three() {
        let request = CompletionRequest {
            model: "claude-sonnet-4-5".to_string(),
            messages: vec![
                Message::user("u1"),
                Message::assistant("a1"),
                Message::user("u2"),
                Message::assistant("a2"),
                Message::user("u3 (last)"),
            ],
            tools: vec![],
            max_tokens: 100,
            temperature: 0.0,
            system: Some("sys".to_string()),
            thinking: None,
            prompt_caching: true,
            cache_ttl: None,
            response_format: None,
            timeout_secs: None,
            extra_body: None,
            agent_id: None,
        };
        let api_request = build_anthropic_request(&request);
        // Trailing 3 must carry the marker.
        for i in 2..5 {
            let cc = last_block_cache_control(&api_request.messages[i])
                .unwrap_or_else(|| panic!("message[{i}] missing cache_control"));
            assert_eq!(cc["type"], "ephemeral");
            assert!(cc.get("ttl").is_none(), "default ttl should be 5m (no key)");
        }
        // Earlier messages must NOT carry a marker — burning a breakpoint
        // there would split the cache and waste the 4-slot budget.
        for i in 0..2 {
            assert!(
                last_block_cache_control(&api_request.messages[i]).is_none(),
                "message[{i}] should not be marked",
            );
        }
    }

    /// When tools are present, the tools-last marker consumes 1 of the 4
    /// breakpoints; only 2 message slots remain (1 system + 1 tools-last
    /// + 2 messages = 4). Messages [4..=5] are stamped; [0..=3] are not.
    #[test]
    fn rolling_window_reserves_slot_for_tools() {
        let tool = ToolDefinition {
            name: "alpha".to_string(),
            description: "x".to_string(),
            input_schema: serde_json::json!({"type":"object"}),
        };
        let request = CompletionRequest {
            model: "claude-sonnet-4-5".to_string(),
            messages: vec![
                Message::user("u1"),
                Message::assistant("a1"),
                Message::user("u2"),
                Message::assistant("a2"),
                Message::user("u3"),
                Message::assistant("a3 (last)"),
            ],
            tools: vec![tool.clone(), tool],
            max_tokens: 100,
            temperature: 0.0,
            system: Some("sys".to_string()),
            thinking: None,
            prompt_caching: true,
            cache_ttl: None,
            response_format: None,
            timeout_secs: None,
            extra_body: None,
            agent_id: None,
        };
        let api_request = build_anthropic_request(&request);
        // Last 2 messages carry the marker.
        for i in 4..6 {
            let cc = last_block_cache_control(&api_request.messages[i])
                .unwrap_or_else(|| panic!("message[{i}] missing cache_control"));
            assert_eq!(cc["type"], "ephemeral");
        }
        // Earlier 4 messages must not.
        for i in 0..4 {
            assert!(
                last_block_cache_control(&api_request.messages[i]).is_none(),
                "message[{i}] should not be marked",
            );
        }
        // Tools-last keeps its marker.
        assert!(api_request.tools[0].cache_control.is_none());
        assert!(api_request.tools[1].cache_control.is_some());
    }

    /// A `ToolResult` content block at the tail of the rolling window must
    /// receive its `cache_control` field — not just the `Text` arm. This
    /// guards the `match` arm coverage in `stamp_block_with_marker`.
    #[test]
    fn tool_result_block_in_window_is_stamped() {
        let tool_result_msg = Message::user_with_blocks(vec![ContentBlock::ToolResult {
            tool_use_id: "tu_1".to_string(),
            tool_name: "alpha".to_string(),
            content: "ok".to_string(),
            is_error: false,
            status: Default::default(),
            approval_request_id: None,
        }]);
        let request = CompletionRequest {
            model: "claude-sonnet-4-5".to_string(),
            messages: vec![tool_result_msg],
            tools: vec![],
            max_tokens: 100,
            temperature: 0.0,
            system: Some("sys".to_string()),
            thinking: None,
            prompt_caching: true,
            cache_ttl: None,
            response_format: None,
            timeout_secs: None,
            extra_body: None,
            agent_id: None,
        };
        let api_request = build_anthropic_request(&request);
        let last = api_request.messages.last().expect("has last message");
        let blocks = match &last.content {
            ApiContent::Blocks(b) => b,
            ApiContent::Text(_) => panic!("expected Blocks for tool_result"),
        };
        let cc = match blocks.last().expect("has last block") {
            ApiContentBlock::ToolResult { cache_control, .. } => cache_control
                .as_ref()
                .expect("tool_result must carry cache_control"),
            _ => panic!("expected ToolResult"),
        };
        assert_eq!(cc["type"], "ephemeral");
    }

    /// Even with zero non-system messages, the system block carries a
    /// cache_control marker — `build_system_value` must always upgrade
    /// the system field to the structured form when caching is on, so
    /// the system breakpoint is preserved for tools-only or probe calls.
    #[test]
    fn system_block_always_stamped_when_caching_on() {
        let request = CompletionRequest {
            model: "claude-sonnet-4-5".to_string(),
            messages: vec![Message::user("hi")], // dummy: api requires >=1 user msg
            tools: vec![],
            max_tokens: 100,
            temperature: 0.0,
            system: Some("sys-prompt".to_string()),
            thinking: None,
            prompt_caching: true,
            cache_ttl: None,
            response_format: None,
            timeout_secs: None,
            extra_body: None,
            agent_id: None,
        };
        let api_request = build_anthropic_request(&request);
        let system = api_request.system.expect("system field set");
        // Caching on → system rendered as a single-element block array.
        let arr = system.as_array().expect("system must be array form");
        let cc = arr[0]
            .get("cache_control")
            .expect("system block must carry cache_control");
        assert_eq!(cc["type"], "ephemeral");
    }

    /// `cache_ttl: Some("1h")` propagates the `"ttl":"1h"` field into every
    /// marker (system + tools + messages) and triggers the 1h beta header
    /// gate (`request_uses_1h_cache`). The header itself is attached at
    /// the HTTP layer; here we verify the request-shaping logic.
    #[test]
    fn ttl_1h_propagates_into_all_markers() {
        let tool = ToolDefinition {
            name: "alpha".to_string(),
            description: "x".to_string(),
            input_schema: serde_json::json!({"type":"object"}),
        };
        let request = CompletionRequest {
            model: "claude-sonnet-4-5".to_string(),
            messages: vec![
                Message::user("u1"),
                Message::assistant("a1"),
                Message::user("u2 (last)"),
            ],
            tools: vec![tool],
            max_tokens: 100,
            temperature: 0.0,
            system: Some("sys".to_string()),
            thinking: None,
            prompt_caching: true,
            cache_ttl: Some("1h"),
            response_format: None,
            timeout_secs: None,
            extra_body: None,
            agent_id: None,
        };
        // HTTP-layer header gate.
        assert!(request_uses_1h_cache(&request));
        let api_request = build_anthropic_request(&request);
        // System carries 1h ttl.
        let sys_arr = api_request
            .system
            .as_ref()
            .and_then(|v| v.as_array())
            .expect("system in array form");
        assert_eq!(sys_arr[0]["cache_control"]["ttl"], "1h");
        // Tools-last carries 1h ttl.
        let tool_cc = api_request.tools[0]
            .cache_control
            .as_ref()
            .expect("tools-last marked");
        assert_eq!(tool_cc["ttl"], "1h");
        // Last message (only 1 slot left after system + tools) carries 1h ttl.
        let last_cc = last_block_cache_control(api_request.messages.last().unwrap())
            .expect("last message marked");
        assert_eq!(last_cc["ttl"], "1h");
    }

    /// 5m default: caching on but `cache_ttl = None` → markers carry no
    /// `ttl` key (the wire format Anthropic interprets as 5-minute
    /// ephemeral cache), and the 1h beta header gate stays closed.
    #[test]
    fn ttl_default_omits_ttl_field_and_skips_beta_header() {
        let request = CompletionRequest {
            model: "claude-sonnet-4-5".to_string(),
            messages: vec![Message::user("hi")],
            tools: vec![],
            max_tokens: 100,
            temperature: 0.0,
            system: Some("sys".to_string()),
            thinking: None,
            prompt_caching: true,
            cache_ttl: None,
            response_format: None,
            timeout_secs: None,
            extra_body: None,
            agent_id: None,
        };
        assert!(!request_uses_1h_cache(&request));
        let api_request = build_anthropic_request(&request);
        let sys_arr = api_request
            .system
            .as_ref()
            .and_then(|v| v.as_array())
            .expect("system in array form");
        let cc = &sys_arr[0]["cache_control"];
        assert_eq!(cc["type"], "ephemeral");
        assert!(cc.get("ttl").is_none(), "5m default must not write ttl key");
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
            cache_ttl: None,
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
            cache_ttl: None,
            response_format: None,
            timeout_secs: None,
            extra_body: None,
            agent_id: None,
        };
        let api_request = build_anthropic_request(&request);
        assert!(api_request.tools.is_empty());
    }
}
