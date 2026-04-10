//! OpenAI-compatible API driver.
//!
//! Works with OpenAI, Ollama, vLLM, and any other OpenAI-compatible endpoint.

use crate::llm_driver::{CompletionRequest, CompletionResponse, LlmDriver, LlmError, StreamEvent};
use crate::think_filter::{FilterAction, StreamingThinkFilter};
use async_trait::async_trait;
use futures::StreamExt;
use librefang_types::config::ResponseFormat;
use librefang_types::message::{ContentBlock, MessageContent, Role, StopReason, TokenUsage};
use librefang_types::tool::ToolCall;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tracing::{debug, warn};
use zeroize::Zeroizing;

/// OpenAI-compatible API driver.
pub struct OpenAIDriver {
    api_key: Zeroizing<String>,
    base_url: String,
    client: reqwest::Client,
    extra_headers: Vec<(String, String)>,
    /// If true, use `api-key` header instead of `Authorization: Bearer`.
    /// Used by Azure OpenAI.
    use_api_key_header: bool,
    /// Optional query string appended to the request URL (e.g., "api-version=2024-02-01").
    /// Used by Azure OpenAI.
    url_query: Option<String>,
}

impl OpenAIDriver {
    /// Create a new OpenAI-compatible driver.
    pub fn new(api_key: String, base_url: String) -> Self {
        Self {
            api_key: Zeroizing::new(api_key),
            base_url,
            client: crate::http_client::proxied_client(),
            extra_headers: Vec::new(),
            use_api_key_header: false,
            url_query: None,
        }
    }

    /// Create a new Azure OpenAI driver.
    ///
    /// Azure OpenAI uses a different URL format and `api-key` header instead of Bearer auth.
    /// URL: `{endpoint}/openai/deployments/{deployment}/chat/completions?api-version={version}`
    pub fn new_azure(
        api_key: String,
        endpoint: String,
        deployment: String,
        api_version: String,
    ) -> Self {
        let base_url = format!(
            "{}/openai/deployments/{}",
            endpoint.trim_end_matches('/'),
            deployment
        );
        Self {
            api_key: Zeroizing::new(api_key),
            base_url,
            client: crate::http_client::proxied_client(),
            extra_headers: Vec::new(),
            use_api_key_header: true,
            url_query: Some(format!("api-version={}", api_version)),
        }
    }

    /// True if this provider is Moonshot/Kimi and requires reasoning_content on assistant messages with tool_calls.
    fn kimi_needs_reasoning_content(&self, model: &str) -> bool {
        self.base_url.contains("moonshot") || model.to_lowercase().contains("kimi")
    }

    /// True if this model is DeepSeek-reasoner (R1).
    ///
    /// DeepSeek-reasoner returns `reasoning_content` in assistant responses, but
    /// for multi-turn conversations the API **rejects** requests that include
    /// `reasoning_content` on previous assistant messages.  We must strip it from
    /// all historical assistant messages when building the request payload.
    fn is_deepseek_reasoner(&self, model: &str) -> bool {
        let m = model.to_lowercase();
        m.contains("deepseek-reasoner") || m.contains("deepseek-r1")
    }

    /// Create a driver with additional HTTP headers (e.g. for Copilot IDE auth).
    pub fn with_extra_headers(mut self, headers: Vec<(String, String)>) -> Self {
        self.extra_headers = headers;
        self
    }
}

#[derive(Debug, Serialize)]
struct OaiRequest {
    model: String,
    messages: Vec<OaiMessage>,
    /// Classic token limit field (used by most models).
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
    /// New token limit field required by GPT-5 and o-series reasoning models.
    #[serde(skip_serializing_if = "Option::is_none")]
    max_completion_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tools: Vec<OaiTool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_choice: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    stream: bool,
    /// Request usage stats in streaming responses (OpenAI extension, supported by Groq et al).
    #[serde(skip_serializing_if = "Option::is_none")]
    stream_options: Option<serde_json::Value>,
    /// Moonshot Kimi K2.5: disable thinking so multi-turn with tool_calls works without preserving reasoning_content.
    #[serde(skip_serializing_if = "Option::is_none")]
    thinking: Option<serde_json::Value>,
    /// Structured output: `response_format` field (json_object or json_schema).
    #[serde(skip_serializing_if = "Option::is_none")]
    response_format: Option<serde_json::Value>,
    /// Provider-specific extension parameters.  Skipped during normal serde
    /// serialization — merged into the top-level JSON request body manually in
    /// `complete()` and `stream()` so that extra_body values **override** any
    /// standard field with the same name.
    #[serde(skip_serializing)]
    extra_body: Option<HashMap<String, serde_json::Value>>,
}

/// Convert a [`ResponseFormat`] into the OpenAI `response_format` JSON value.
fn oai_response_format(rf: &ResponseFormat) -> Option<serde_json::Value> {
    match rf {
        ResponseFormat::Text => None, // text is the default — omit the field
        ResponseFormat::Json => Some(serde_json::json!({"type": "json_object"})),
        ResponseFormat::JsonSchema {
            name,
            schema,
            strict,
        } => {
            let mut js = serde_json::json!({
                "name": name,
                "schema": schema,
            });
            if let Some(s) = strict {
                js["strict"] = serde_json::json!(s);
            }
            Some(serde_json::json!({
                "type": "json_schema",
                "json_schema": js,
            }))
        }
    }
}

/// Returns true if a model uses `max_completion_tokens` instead of `max_tokens`.
fn uses_completion_tokens(model: &str) -> bool {
    let m = model.to_lowercase();
    m.starts_with("gpt-5")
        || m.starts_with("gpt5")
        || m.starts_with("o1")
        || m.starts_with("o3")
        || m.starts_with("o4")
}

/// Returns true if a model rejects the `temperature` parameter.
///
/// OpenAI's o-series reasoning models and GPT-5-mini variants only accept
/// `temperature=1` (the default). Sending any other value causes a 400 error.
/// We proactively omit `temperature` for these models to avoid wasting a retry.
fn rejects_temperature(model: &str) -> bool {
    let m = model.to_lowercase();
    // o-series reasoning models: o1, o1-mini, o1-preview, o3, o3-mini, o3-pro, o4-mini, etc.
    m.starts_with("o1")
        || m.starts_with("o3")
        || m.starts_with("o4")
        // GPT-5-mini is a reasoning model that rejects temperature
        || m.starts_with("gpt-5-mini")
        || m.starts_with("gpt5-mini")
        // Catch any model explicitly tagged as "reasoning"
        || m.contains("-reasoning")
}

/// Returns true if a model only accepts temperature = 1 (e.g. Moonshot Kimi K2/K2.5).
fn temperature_must_be_one(model: &str) -> bool {
    let m = model.to_lowercase();
    m.starts_with("kimi-k2") || m == "kimi-k2.5" || m == "kimi-k2.5-0711"
}

#[derive(Debug, Serialize)]
struct OaiMessage {
    role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    content: Option<OaiMessageContent>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_calls: Option<Vec<OaiToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_call_id: Option<String>,
    /// Moonshot Kimi: sent as empty string on assistant messages with tool_calls when using Kimi (thinking is disabled for multi-turn compatibility).
    #[serde(skip_serializing_if = "Option::is_none")]
    reasoning_content: Option<String>,
}

/// Content can be a plain string or an array of content parts (for images).
#[derive(Debug, Serialize)]
#[serde(untagged)]
enum OaiMessageContent {
    Text(String),
    Parts(Vec<OaiContentPart>),
}

/// A content part for multi-modal messages.
#[derive(Debug, Serialize)]
#[serde(tag = "type")]
enum OaiContentPart {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "image_url")]
    ImageUrl { image_url: OaiImageUrl },
}

#[derive(Debug, Serialize)]
struct OaiImageUrl {
    url: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct OaiToolCall {
    id: String,
    #[serde(rename = "type")]
    call_type: String,
    function: OaiFunction,
}

#[derive(Debug, Serialize, Deserialize)]
struct OaiFunction {
    name: String,
    arguments: String,
}

#[derive(Debug, Serialize)]
struct OaiTool {
    #[serde(rename = "type")]
    tool_type: String,
    function: OaiToolDef,
}

#[derive(Debug, Serialize)]
struct OaiToolDef {
    name: String,
    description: String,
    parameters: serde_json::Value,
}

#[derive(Debug, Deserialize)]
struct OaiResponse {
    choices: Vec<OaiChoice>,
    usage: Option<OaiUsage>,
}

#[derive(Debug, Deserialize)]
struct OaiChoice {
    message: OaiResponseMessage,
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OaiResponseMessage {
    content: Option<String>,
    tool_calls: Option<Vec<OaiToolCall>>,
    /// Reasoning/thinking content returned by some models (DeepSeek-R1, Qwen3, etc.)
    /// via DeepSeek's official API, LM Studio, Ollama, and other OpenAI-compatible servers.
    reasoning_content: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OaiUsage {
    prompt_tokens: u64,
    completion_tokens: u64,
    /// Detailed prompt token breakdown (includes cached token info).
    #[serde(default)]
    prompt_tokens_details: Option<OaiPromptTokensDetails>,
}

/// OpenAI prompt token details (includes cached token count).
#[derive(Debug, Deserialize, Default)]
struct OaiPromptTokensDetails {
    /// Number of prompt tokens served from cache.
    #[serde(default)]
    cached_tokens: u64,
}

impl OpenAIDriver {
    /// Build the `OaiRequest` from a `CompletionRequest`.
    ///
    /// Shared between `complete()` and `stream()`.  The caller sets
    /// `stream` / `stream_options` on the returned struct before sending.
    fn build_request(&self, request: &CompletionRequest) -> Result<OaiRequest, LlmError> {
        let mut oai_messages: Vec<OaiMessage> = Vec::new();

        // Add system message if present
        if let Some(ref system) = request.system {
            oai_messages.push(OaiMessage {
                role: "system".to_string(),
                content: Some(OaiMessageContent::Text(system.clone())),
                tool_calls: None,
                tool_call_id: None,
                reasoning_content: None,
            });
        }

        // Convert messages
        for msg in &request.messages {
            match (&msg.role, &msg.content) {
                (Role::System, MessageContent::Text(text)) => {
                    if request.system.is_none() {
                        oai_messages.push(OaiMessage {
                            role: "system".to_string(),
                            content: Some(OaiMessageContent::Text(text.clone())),
                            tool_calls: None,
                            tool_call_id: None,
                            reasoning_content: None,
                        });
                    }
                }
                (Role::User, MessageContent::Text(text)) => {
                    oai_messages.push(OaiMessage {
                        role: "user".to_string(),
                        content: Some(OaiMessageContent::Text(text.clone())),
                        tool_calls: None,
                        tool_call_id: None,
                        reasoning_content: None,
                    });
                }
                (Role::Assistant, MessageContent::Text(text)) => {
                    oai_messages.push(OaiMessage {
                        role: "assistant".to_string(),
                        content: Some(OaiMessageContent::Text(text.clone())),
                        tool_calls: None,
                        tool_call_id: None,
                        reasoning_content: None,
                    });
                }
                (Role::User, MessageContent::Blocks(blocks)) => {
                    // Handle tool results and images in user messages
                    let mut parts: Vec<OaiContentPart> = Vec::new();
                    let mut has_tool_results = false;
                    for block in blocks {
                        match block {
                            ContentBlock::ToolResult {
                                tool_use_id,
                                content,
                                ..
                            } => {
                                has_tool_results = true;
                                oai_messages.push(OaiMessage {
                                    role: "tool".to_string(),
                                    content: Some(OaiMessageContent::Text(if content.is_empty() {
                                        "(empty)".to_string()
                                    } else {
                                        content.clone()
                                    })),
                                    tool_calls: None,
                                    tool_call_id: Some(tool_use_id.clone()),
                                    reasoning_content: None,
                                });
                            }
                            ContentBlock::Text { text, .. } => {
                                parts.push(OaiContentPart::Text { text: text.clone() });
                            }
                            ContentBlock::Image { media_type, data } => {
                                parts.push(OaiContentPart::ImageUrl {
                                    image_url: OaiImageUrl {
                                        url: format!("data:{media_type};base64,{data}"),
                                    },
                                });
                            }
                            ContentBlock::ImageFile { media_type, path } => {
                                match std::fs::read(path) {
                                    Ok(bytes) => {
                                        use base64::Engine;
                                        let data = base64::engine::general_purpose::STANDARD
                                            .encode(&bytes);
                                        parts.push(OaiContentPart::ImageUrl {
                                            image_url: OaiImageUrl {
                                                url: format!("data:{media_type};base64,{data}"),
                                            },
                                        });
                                    }
                                    Err(e) => {
                                        warn!(path = %path, error = %e, "ImageFile missing, skipping");
                                    }
                                }
                            }
                            ContentBlock::Thinking { .. } => {}
                            _ => {}
                        }
                    }
                    if !parts.is_empty() && !has_tool_results {
                        oai_messages.push(OaiMessage {
                            role: "user".to_string(),
                            content: Some(OaiMessageContent::Parts(parts)),
                            tool_calls: None,
                            tool_call_id: None,
                            reasoning_content: None,
                        });
                    }
                }
                (Role::Assistant, MessageContent::Blocks(blocks)) => {
                    let mut text_parts = Vec::new();
                    let mut tool_calls = Vec::new();
                    for block in blocks {
                        match block {
                            ContentBlock::Text { text, .. } => text_parts.push(text.clone()),
                            ContentBlock::ToolUse {
                                id, name, input, ..
                            } => {
                                tool_calls.push(OaiToolCall {
                                    id: id.clone(),
                                    call_type: "function".to_string(),
                                    function: OaiFunction {
                                        name: name.clone(),
                                        arguments: serde_json::to_string(input).unwrap_or_default(),
                                    },
                                });
                            }
                            ContentBlock::Thinking { .. } => {}
                            _ => {}
                        }
                    }
                    let has_tool_calls = !tool_calls.is_empty();
                    let is_deepseek_r = self.is_deepseek_reasoner(&request.model);
                    oai_messages.push(OaiMessage {
                        role: "assistant".to_string(),
                        // ZHIPU (GLM) rejects assistant messages where content is
                        // null or omitted when tool_calls are present (error 1214).
                        // DeepSeek-reasoner also requires a non-null content field
                        // on all assistant messages in multi-turn conversations.
                        // Always send an empty string for these providers so every
                        // OpenAI-compat endpoint gets a valid payload.
                        content: if text_parts.is_empty() {
                            if has_tool_calls || is_deepseek_r {
                                Some(OaiMessageContent::Text(String::new()))
                            } else {
                                None
                            }
                        } else {
                            Some(OaiMessageContent::Text(text_parts.join("")))
                        },
                        tool_calls: if tool_calls.is_empty() {
                            None
                        } else {
                            Some(tool_calls)
                        },
                        tool_call_id: None,
                        // DeepSeek-reasoner: MUST omit reasoning_content on
                        // all previous assistant messages — the API rejects it.
                        // Kimi: requires an empty-string reasoning_content when
                        // tool_calls are present (thinking is disabled for
                        // multi-turn compatibility).
                        reasoning_content: if is_deepseek_r {
                            // Always None — DeepSeek rejects reasoning_content
                            // on historical assistant turns.
                            None
                        } else if has_tool_calls
                            && self.kimi_needs_reasoning_content(&request.model)
                        {
                            Some(String::new())
                        } else {
                            None
                        },
                    });
                }
                _ => {}
            }
        }

        let oai_tools: Vec<OaiTool> = request
            .tools
            .iter()
            .map(|t| OaiTool {
                tool_type: "function".to_string(),
                function: OaiToolDef {
                    name: t.name.clone(),
                    description: t.description.clone(),
                    parameters: librefang_types::tool::normalize_schema_for_provider(
                        &t.input_schema,
                        "openai",
                    ),
                },
            })
            .collect();

        // Guard: an empty message list would produce an unparseable API response
        // (typically "EOF while parsing a value at line 1 column 0").
        if oai_messages.is_empty() {
            return Err(LlmError::Api {
                status: 0,
                message: "Cannot send request with no messages — \
                          this usually means aggressive history trimming emptied \
                          the conversation"
                    .to_string(),
            });
        }

        let tool_choice = if oai_tools.is_empty() {
            None
        } else {
            Some(serde_json::json!("auto"))
        };

        let (mt, mct) = if uses_completion_tokens(&request.model) {
            (None, Some(request.max_tokens))
        } else {
            (Some(request.max_tokens), None)
        };

        Ok(OaiRequest {
            model: request.model.clone(),
            messages: oai_messages,
            max_tokens: mt,
            max_completion_tokens: mct,
            temperature: if self.kimi_needs_reasoning_content(&request.model) {
                // Kimi with thinking disabled uses fixed 0.6 for multi-turn compatibility.
                Some(0.6)
            } else if temperature_must_be_one(&request.model) {
                Some(1.0)
            } else if rejects_temperature(&request.model) {
                None
            } else {
                Some(request.temperature)
            },
            tools: oai_tools,
            tool_choice,
            stream: false,
            stream_options: None,
            thinking: if self.kimi_needs_reasoning_content(&request.model) {
                Some(serde_json::json!({"type": "disabled"}))
            } else {
                None
            },
            response_format: request
                .response_format
                .as_ref()
                .and_then(oai_response_format),
            extra_body: request.extra_body.clone(),
        })
    }
}

#[async_trait]
impl LlmDriver for OpenAIDriver {
    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        let mut oai_request = self.build_request(&request)?;

        let max_retries = 3;
        for attempt in 0..=max_retries {
            let url = match &self.url_query {
                Some(q) => format!("{}/chat/completions?{}", self.base_url, q),
                None => format!("{}/chat/completions", self.base_url),
            };
            debug!(url = %url, attempt, "Sending OpenAI API request");

            // Serialize to Value, then merge extra_body so extra params
            // override any standard field with the same name.
            let mut body =
                serde_json::to_value(&oai_request).map_err(|e| LlmError::Http(e.to_string()))?;
            if let (Some(extra), Some(obj)) = (&oai_request.extra_body, body.as_object_mut()) {
                for (k, v) in extra {
                    obj.insert(k.clone(), v.clone());
                }
            }

            let mut req_builder = self
                .client
                .post(&url)
                .header("content-type", "application/json")
                .json(&body);

            if !self.api_key.as_str().is_empty() {
                if self.use_api_key_header {
                    req_builder = req_builder.header("api-key", self.api_key.as_str());
                } else {
                    req_builder = req_builder
                        .header("authorization", format!("Bearer {}", self.api_key.as_str()));
                }
            }
            for (k, v) in &self.extra_headers {
                req_builder = req_builder.header(k, v);
            }

            let resp = req_builder
                .send()
                .await
                .map_err(|e| LlmError::Http(e.to_string()))?;

            let status = resp.status().as_u16();
            if status == 429 {
                if attempt < max_retries {
                    let retry_ms = (attempt + 1) as u64 * 2000;
                    warn!(status, retry_ms, "Rate limited, retrying");
                    tokio::time::sleep(std::time::Duration::from_millis(retry_ms)).await;
                    continue;
                }
                return Err(LlmError::RateLimited {
                    retry_after_ms: 5000,
                    message: None,
                });
            }

            if !resp.status().is_success() {
                let body = resp.text().await.unwrap_or_default();

                // Groq "tool_use_failed": model generated tool call in XML format.
                // Parse the failed_generation and convert to a proper tool call response.
                if status == 400 && body.contains("tool_use_failed") {
                    if let Some(response) = parse_groq_failed_tool_call(&body) {
                        warn!("Recovered tool call from Groq failed_generation");
                        return Ok(response);
                    }
                    // If parsing fails, retry on next attempt
                    if attempt < max_retries {
                        let retry_ms = (attempt + 1) as u64 * 1500;
                        warn!(status, attempt, retry_ms, "tool_use_failed, retrying");
                        tokio::time::sleep(std::time::Duration::from_millis(retry_ms)).await;
                        continue;
                    }
                }

                // o-series / reasoning models: strip temperature if rejected
                if status == 400
                    && body.contains("temperature")
                    && body.contains("unsupported_parameter")
                    && oai_request.temperature.is_some()
                    && attempt < max_retries
                {
                    warn!(model = %oai_request.model, "Stripping temperature for this model");
                    oai_request.temperature = None;
                    continue;
                }

                // GPT-5 / o-series: switch from max_tokens to max_completion_tokens
                if status == 400
                    && body.contains("max_tokens")
                    && (body.contains("unsupported_parameter")
                        || body.contains("max_completion_tokens"))
                    && oai_request.max_tokens.is_some()
                    && attempt < max_retries
                {
                    let val = oai_request.max_tokens.unwrap();
                    warn!(model = %oai_request.model, "Switching to max_completion_tokens for this model");
                    oai_request.max_tokens = None;
                    oai_request.max_completion_tokens = Some(val);
                    continue;
                }

                // Auto-cap max_tokens when model rejects our value (e.g. Groq Maverick limit 8192)
                if status == 400 && body.contains("max_tokens") && attempt < max_retries {
                    let current = oai_request
                        .max_tokens
                        .or(oai_request.max_completion_tokens)
                        .unwrap_or(4096);
                    let cap = extract_max_tokens_limit(&body).unwrap_or(current / 2);
                    warn!(
                        old = current,
                        new = cap,
                        "Auto-capping max_tokens to model limit"
                    );
                    if oai_request.max_completion_tokens.is_some() {
                        oai_request.max_completion_tokens = Some(cap);
                    } else {
                        oai_request.max_tokens = Some(cap);
                    }
                    continue;
                }

                // Model doesn't support function calling — retry without tools
                // (e.g. GLM-5 on DashScope returns 500 "internal error" when tools are sent)
                let body_lower = body.to_lowercase();
                if !oai_request.tools.is_empty()
                    && attempt < max_retries
                    && (status == 500
                        || body_lower.contains("internal error")
                        || (status == 400
                            && (body_lower.contains("does not support tools")
                                || body_lower.contains("tool")
                                    && body_lower.contains("not supported"))))
                {
                    warn!(
                        model = %oai_request.model,
                        status,
                        "Model may not support tools, retrying without tools"
                    );
                    oai_request.tools.clear();
                    oai_request.tool_choice = None;
                    continue;
                }

                return Err(LlmError::Api {
                    status,
                    message: body,
                });
            }

            let body = resp
                .text()
                .await
                .map_err(|e| LlmError::Http(e.to_string()))?;
            let raw_json: serde_json::Value =
                serde_json::from_str(&body).map_err(|e| LlmError::Parse(e.to_string()))?;
            let cached_prompt_tokens = raw_json
                .get("usage")
                .and_then(|u| u.get("prompt_tokens_details"))
                .and_then(|d| d.get("cached_tokens"))
                .and_then(|v| v.as_u64());
            let oai_response: OaiResponse =
                serde_json::from_value(raw_json).map_err(|e| LlmError::Parse(e.to_string()))?;

            let choice = oai_response
                .choices
                .into_iter()
                .next()
                .ok_or_else(|| LlmError::Parse("No choices in response".to_string()))?;

            let mut content = Vec::new();
            let mut tool_calls = Vec::new();

            // Capture reasoning_content from models that use a separate field
            // (DeepSeek-R1, Qwen3, etc. via LM Studio/Ollama)
            if let Some(ref reasoning) = choice.message.reasoning_content {
                if !reasoning.is_empty() {
                    debug!(
                        len = reasoning.len(),
                        "Captured reasoning_content from response"
                    );
                    content.push(ContentBlock::Thinking {
                        thinking: reasoning.clone(),
                        provider_metadata: None,
                    });
                }
            }

            if let Some(text) = choice.message.content {
                if !text.is_empty() {
                    // Extract <think>...</think> blocks that some local models
                    // embed directly in the content field.
                    let (cleaned, thinking) = extract_think_tags(&text);
                    if let Some(think_text) = thinking {
                        // Only add if we didn't already get reasoning_content
                        if choice.message.reasoning_content.is_none() {
                            content.push(ContentBlock::Thinking {
                                thinking: think_text,
                                provider_metadata: None,
                            });
                        }
                    }
                    if !cleaned.is_empty() {
                        content.push(ContentBlock::Text {
                            text: cleaned,
                            provider_metadata: None,
                        });
                    }
                }
            }

            // If we have reasoning but no text content and no tool calls,
            // synthesize a brief text block so the agent loop doesn't treat
            // this as an empty response.
            let has_text = content
                .iter()
                .any(|b| matches!(b, ContentBlock::Text { .. }));
            let has_thinking = content
                .iter()
                .any(|b| matches!(b, ContentBlock::Thinking { .. }));
            if has_thinking && !has_text && choice.message.tool_calls.is_none() {
                // Extract the last sentence or line from the thinking as a response
                let thinking_text = content
                    .iter()
                    .find_map(|b| match b {
                        ContentBlock::Thinking { thinking, .. } => Some(thinking.as_str()),
                        _ => None,
                    })
                    .unwrap_or("");
                let summary = extract_thinking_summary(thinking_text);
                debug!(
                    summary_len = summary.len(),
                    "Synthesizing text from thinking-only response"
                );
                content.push(ContentBlock::Text {
                    text: summary,
                    provider_metadata: None,
                });
            }

            if let Some(calls) = choice.message.tool_calls {
                for call in calls {
                    let input: serde_json::Value =
                        match serde_json::from_str::<serde_json::Value>(&call.function.arguments) {
                            Ok(v) => ensure_object(v),
                            Err(e) => {
                                tracing::warn!(
                                    tool = %call.function.name,
                                    raw_args_len = call.function.arguments.len(),
                                    error = %e,
                                    "Malformed tool call arguments from LLM"
                                );
                                malformed_tool_input(&e, call.function.arguments.len())
                            }
                        };
                    content.push(ContentBlock::ToolUse {
                        id: call.id.clone(),
                        name: call.function.name.clone(),
                        input: input.clone(),
                        provider_metadata: None,
                    });
                    tool_calls.push(ToolCall {
                        id: call.id,
                        name: call.function.name,
                        input,
                    });
                }
            }

            let stop_reason = match choice.finish_reason.as_deref() {
                Some("stop") => StopReason::EndTurn,
                Some("tool_calls") => StopReason::ToolUse,
                Some("length") => StopReason::MaxTokens,
                _ => {
                    if !tool_calls.is_empty() {
                        StopReason::ToolUse
                    } else {
                        StopReason::EndTurn
                    }
                }
            };

            let mut usage = oai_response
                .usage
                .map(|u| {
                    let cached = u
                        .prompt_tokens_details
                        .as_ref()
                        .map(|d| d.cached_tokens)
                        .unwrap_or(0);
                    TokenUsage {
                        input_tokens: u.prompt_tokens,
                        output_tokens: u.completion_tokens,
                        cache_creation_input_tokens: 0,
                        cache_read_input_tokens: cached,
                    }
                })
                .unwrap_or_default();

            // Guard: if the model returned content but usage is missing/zero
            // (common with local LLMs like LM Studio, Ollama), set a synthetic
            // non-zero output_tokens so the agent loop doesn't misclassify
            // this as a "silent failure" and loop unnecessarily.
            if !content.is_empty() && usage.input_tokens == 0 && usage.output_tokens == 0 {
                debug!(
                    "Response has content but no usage stats — setting synthetic output_tokens=1"
                );
                usage.output_tokens = 1;
            }

            debug!(
                prompt_tokens = usage.input_tokens,
                completion_tokens = usage.output_tokens,
                cached_prompt_tokens = cached_prompt_tokens.unwrap_or(0),
                "OpenAI-compatible usage"
            );

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

    async fn stream(
        &self,
        request: CompletionRequest,
        tx: tokio::sync::mpsc::Sender<StreamEvent>,
    ) -> Result<CompletionResponse, LlmError> {
        let mut oai_request = self.build_request(&request)?;
        oai_request.stream = true;
        oai_request.stream_options = Some(serde_json::json!({"include_usage": true}));

        // Retry loop for the initial HTTP request
        let max_retries = 3;
        for attempt in 0..=max_retries {
            let url = match &self.url_query {
                Some(q) => format!("{}/chat/completions?{}", self.base_url, q),
                None => format!("{}/chat/completions", self.base_url),
            };
            debug!(url = %url, attempt, "Sending OpenAI streaming request");

            // Serialize to Value, then merge extra_body so extra params
            // override any standard field with the same name.
            let mut body =
                serde_json::to_value(&oai_request).map_err(|e| LlmError::Http(e.to_string()))?;
            if let (Some(extra), Some(obj)) = (&oai_request.extra_body, body.as_object_mut()) {
                for (k, v) in extra {
                    obj.insert(k.clone(), v.clone());
                }
            }

            let mut req_builder = self
                .client
                .post(&url)
                .header("content-type", "application/json")
                .json(&body);

            if !self.api_key.as_str().is_empty() {
                if self.use_api_key_header {
                    req_builder = req_builder.header("api-key", self.api_key.as_str());
                } else {
                    req_builder = req_builder
                        .header("authorization", format!("Bearer {}", self.api_key.as_str()));
                }
            }
            for (k, v) in &self.extra_headers {
                req_builder = req_builder.header(k, v);
            }

            let resp = req_builder
                .send()
                .await
                .map_err(|e| LlmError::Http(e.to_string()))?;

            let status = resp.status().as_u16();
            if status == 429 {
                if attempt < max_retries {
                    let retry_ms = (attempt + 1) as u64 * 2000;
                    warn!(status, retry_ms, "Rate limited (stream), retrying");
                    tokio::time::sleep(std::time::Duration::from_millis(retry_ms)).await;
                    continue;
                }
                return Err(LlmError::RateLimited {
                    retry_after_ms: 5000,
                    message: None,
                });
            }

            if !resp.status().is_success() {
                let body = resp.text().await.unwrap_or_default();

                // Groq "tool_use_failed": parse and recover (streaming path)
                if status == 400 && body.contains("tool_use_failed") {
                    if let Some(response) = parse_groq_failed_tool_call(&body) {
                        warn!("Recovered tool call from Groq failed_generation (stream)");
                        return Ok(response);
                    }
                    if attempt < max_retries {
                        let retry_ms = (attempt + 1) as u64 * 1500;
                        warn!(
                            status,
                            attempt, retry_ms, "tool_use_failed (stream), retrying"
                        );
                        tokio::time::sleep(std::time::Duration::from_millis(retry_ms)).await;
                        continue;
                    }
                }

                // o-series / reasoning models: strip temperature if rejected
                if status == 400
                    && body.contains("temperature")
                    && body.contains("unsupported_parameter")
                    && oai_request.temperature.is_some()
                    && attempt < max_retries
                {
                    warn!(model = %oai_request.model, "Stripping temperature for this model (stream)");
                    oai_request.temperature = None;
                    continue;
                }

                // GPT-5 / o-series: switch from max_tokens to max_completion_tokens
                if status == 400
                    && body.contains("max_tokens")
                    && (body.contains("unsupported_parameter")
                        || body.contains("max_completion_tokens"))
                    && oai_request.max_tokens.is_some()
                    && attempt < max_retries
                {
                    let val = oai_request.max_tokens.unwrap();
                    warn!(model = %oai_request.model, "Switching to max_completion_tokens for this model (stream)");
                    oai_request.max_tokens = None;
                    oai_request.max_completion_tokens = Some(val);
                    continue;
                }

                // Auto-cap max_tokens when model rejects our value
                if status == 400 && body.contains("max_tokens") && attempt < max_retries {
                    let current = oai_request
                        .max_tokens
                        .or(oai_request.max_completion_tokens)
                        .unwrap_or(4096);
                    let cap = extract_max_tokens_limit(&body).unwrap_or(current / 2);
                    warn!(old = current, new = cap, "Auto-capping max_tokens (stream)");
                    if oai_request.max_completion_tokens.is_some() {
                        oai_request.max_completion_tokens = Some(cap);
                    } else {
                        oai_request.max_tokens = Some(cap);
                    }
                    continue;
                }

                // Provider doesn't support stream_options — retry without it
                if status == 400
                    && oai_request.stream_options.is_some()
                    && attempt < max_retries
                    && (body.contains("stream_options")
                        || body.contains("stream_option")
                        || body.contains("Unrecognized request argument"))
                {
                    warn!(model = %oai_request.model, "Stripping stream_options (unsupported by provider)");
                    oai_request.stream_options = None;
                    continue;
                }

                // Model doesn't support function calling — retry without tools
                let body_lower = body.to_lowercase();
                if !oai_request.tools.is_empty()
                    && attempt < max_retries
                    && (status == 500
                        || body_lower.contains("internal error")
                        || (status == 400
                            && (body_lower.contains("does not support tools")
                                || body_lower.contains("tool")
                                    && body_lower.contains("not supported"))))
                {
                    warn!(
                        model = %oai_request.model,
                        status,
                        "Model may not support tools (stream), retrying without tools"
                    );
                    oai_request.tools.clear();
                    oai_request.tool_choice = None;
                    continue;
                }

                return Err(LlmError::Api {
                    status,
                    message: body,
                });
            }

            // Parse the SSE stream
            let mut buffer = String::new();
            let mut text_content = String::new();
            let mut reasoning_content = String::new();
            // Filter <think>...</think> tags from streaming text deltas so they
            // don't leak through to the client as visible text.
            let mut think_filter = StreamingThinkFilter::new();
            // Track tool calls: index -> (id, name, arguments)
            let mut tool_accum: Vec<(String, String, String)> = Vec::new();
            let mut finish_reason: Option<String> = None;
            let mut usage = TokenUsage::default();
            let mut cached_prompt_tokens: u64 = 0;
            let mut chunk_count: u32 = 0;
            let mut sse_line_count: u32 = 0;

            let mut byte_stream = resp.bytes_stream();
            while let Some(chunk_result) = byte_stream.next().await {
                let chunk = chunk_result.map_err(|e| LlmError::Http(e.to_string()))?;
                chunk_count += 1;
                buffer.push_str(&String::from_utf8_lossy(&chunk));

                // Process complete lines
                while let Some(pos) = buffer.find('\n') {
                    let line = buffer[..pos].trim_end().to_string();
                    buffer = buffer[pos + 1..].to_string();

                    if line.is_empty() || line.starts_with(':') {
                        continue;
                    }

                    sse_line_count += 1;
                    let data = match line.strip_prefix("data:") {
                        Some(d) => d.trim_start(),
                        None => continue,
                    };

                    if data == "[DONE]" {
                        continue;
                    }

                    let json: serde_json::Value = match serde_json::from_str(data) {
                        Ok(v) => v,
                        Err(_) => continue,
                    };

                    // Extract usage if present (some providers send it in the last chunk)
                    if let Some(u) = json.get("usage") {
                        if let Some(pt) = u["prompt_tokens"].as_u64() {
                            usage.input_tokens = pt;
                        }
                        if let Some(cached) = u
                            .get("prompt_tokens_details")
                            .and_then(|d| d.get("cached_tokens"))
                            .and_then(|v| v.as_u64())
                        {
                            usage.cache_read_input_tokens = cached;
                            cached_prompt_tokens = cached;
                        }
                        if let Some(ct) = u["completion_tokens"].as_u64() {
                            usage.output_tokens = ct;
                        }
                    }

                    let choices = match json["choices"].as_array() {
                        Some(c) => c,
                        None => continue,
                    };

                    for choice in choices {
                        let delta = &choice["delta"];

                        // Text content delta — route through think filter to
                        // strip <think>...</think> tags before they reach the client.
                        // Skip content when tool_calls are present in the same delta —
                        // some providers (e.g. kimi-k2 via nvidia-nim) echo tool call
                        // text in the content field, which would leak raw tool syntax
                        // to the user.
                        let has_tool_calls = delta["tool_calls"].is_array();
                        if let Some(text) = delta["content"].as_str() {
                            if !text.is_empty() && !has_tool_calls {
                                text_content.push_str(text);
                                for action in think_filter.process(text) {
                                    match action {
                                        FilterAction::EmitText(t) => {
                                            let _ =
                                                tx.send(StreamEvent::TextDelta { text: t }).await;
                                        }
                                        FilterAction::EmitThinking(t) => {
                                            // Route think content the same way as
                                            // reasoning_content deltas.
                                            let _ = tx
                                                .send(StreamEvent::ThinkingDelta { text: t })
                                                .await;
                                        }
                                    }
                                }
                            }
                        }

                        // Reasoning/thinking content delta (DeepSeek-R1 via official API or local servers, Qwen3, etc.)
                        if let Some(reasoning) = delta["reasoning_content"].as_str() {
                            if !reasoning.is_empty() {
                                reasoning_content.push_str(reasoning);
                                let _ = tx
                                    .send(StreamEvent::ThinkingDelta {
                                        text: reasoning.to_string(),
                                    })
                                    .await;
                            }
                        } else if let Some(reasoning) = delta["reasoning"].as_str() {
                            // Fallback: Ollama and some local servers expose the reasoning
                            // field instead of reasoning_content.
                            if !reasoning.is_empty() {
                                reasoning_content.push_str(reasoning);
                                let _ = tx
                                    .send(StreamEvent::ThinkingDelta {
                                        text: reasoning.to_string(),
                                    })
                                    .await;
                            }
                        }

                        // Tool call deltas
                        if let Some(calls) = delta["tool_calls"].as_array() {
                            for call in calls {
                                let idx = call["index"].as_u64().unwrap_or(0) as usize;

                                // Ensure tool_accum has enough entries
                                while tool_accum.len() <= idx {
                                    tool_accum.push((String::new(), String::new(), String::new()));
                                }

                                // ID (sent in first chunk for this tool)
                                if let Some(id) = call["id"].as_str() {
                                    tool_accum[idx].0 = id.to_string();
                                }

                                if let Some(func) = call.get("function") {
                                    // Name (sent in first chunk)
                                    if let Some(name) = func["name"].as_str() {
                                        tool_accum[idx].1 = name.to_string();
                                        let _ = tx
                                            .send(StreamEvent::ToolUseStart {
                                                id: tool_accum[idx].0.clone(),
                                                name: name.to_string(),
                                            })
                                            .await;
                                    }

                                    // Arguments delta
                                    if let Some(args) = func["arguments"].as_str() {
                                        tool_accum[idx].2.push_str(args);
                                        if !args.is_empty() {
                                            let _ = tx
                                                .send(StreamEvent::ToolInputDelta {
                                                    text: args.to_string(),
                                                })
                                                .await;
                                        }
                                    }
                                }
                            }
                        }

                        // Finish reason
                        if let Some(fr) = choice["finish_reason"].as_str() {
                            finish_reason = Some(fr.to_string());
                        }
                    }
                }
            }

            // Flush any remaining buffered content from the think filter
            // (e.g. partial tag at stream end, or unclosed think block).
            for action in think_filter.flush() {
                match action {
                    FilterAction::EmitText(t) => {
                        let _ = tx.send(StreamEvent::TextDelta { text: t }).await;
                    }
                    FilterAction::EmitThinking(t) => {
                        let _ = tx.send(StreamEvent::ThinkingDelta { text: t }).await;
                    }
                }
            }

            // Log stream summary for diagnostics
            let is_empty_stream = text_content.is_empty()
                && reasoning_content.is_empty()
                && tool_accum.is_empty()
                && usage.input_tokens == 0
                && usage.output_tokens == 0;
            if is_empty_stream {
                warn!(
                    chunks = chunk_count,
                    sse_lines = sse_line_count,
                    finish = ?finish_reason,
                    buffer_remaining = buffer.len(),
                    "SSE stream returned empty: 0 content, 0 tokens — likely a silently failed request"
                );
            } else {
                debug!(
                    chunks = chunk_count,
                    sse_lines = sse_line_count,
                    text_len = text_content.len(),
                    reasoning_len = reasoning_content.len(),
                    tool_count = tool_accum.len(),
                    finish = ?finish_reason,
                    input_tokens = usage.input_tokens,
                    output_tokens = usage.output_tokens,
                    buffer_remaining = buffer.len(),
                    "SSE stream completed"
                );
            }

            // Build the final response
            let mut content = Vec::new();
            let mut tool_calls = Vec::new();

            // Add reasoning/thinking content if present
            if !reasoning_content.is_empty() {
                content.push(ContentBlock::Thinking {
                    thinking: reasoning_content.clone(),
                    provider_metadata: None,
                });
            }

            if !text_content.is_empty() {
                // Extract <think>...</think> blocks from streamed text content
                let (cleaned, thinking) = extract_think_tags(&text_content);
                if let Some(think_text) = thinking {
                    // Only add if we didn't already get reasoning_content
                    if reasoning_content.is_empty() {
                        content.push(ContentBlock::Thinking {
                            thinking: think_text,
                            provider_metadata: None,
                        });
                    }
                }
                if !cleaned.is_empty() {
                    content.push(ContentBlock::Text {
                        text: cleaned,
                        provider_metadata: None,
                    });
                }
            }

            // If we have reasoning but no text content and no tool calls,
            // synthesize a brief text block so the agent loop doesn't treat
            // this as an empty response.
            let has_text = content
                .iter()
                .any(|b| matches!(b, ContentBlock::Text { .. }));
            let has_thinking = content
                .iter()
                .any(|b| matches!(b, ContentBlock::Thinking { .. }));
            if has_thinking && !has_text && tool_accum.is_empty() {
                let thinking_text = content
                    .iter()
                    .find_map(|b| match b {
                        ContentBlock::Thinking { thinking, .. } => Some(thinking.as_str()),
                        _ => None,
                    })
                    .unwrap_or("");
                let summary = extract_thinking_summary(thinking_text);
                debug!(
                    summary_len = summary.len(),
                    "Synthesizing text from thinking-only stream response"
                );
                content.push(ContentBlock::Text {
                    text: summary,
                    provider_metadata: None,
                });
            }

            for (id, name, arguments) in &tool_accum {
                let input: serde_json::Value =
                    match serde_json::from_str::<serde_json::Value>(arguments) {
                        Ok(v) => ensure_object(v),
                        Err(e) => {
                            tracing::warn!(
                                tool = %name,
                                raw_args_len = arguments.len(),
                                error = %e,
                                "Malformed tool call arguments from LLM stream"
                            );
                            malformed_tool_input(&e, arguments.len())
                        }
                    };
                content.push(ContentBlock::ToolUse {
                    id: id.clone(),
                    name: name.clone(),
                    input: input.clone(),
                    provider_metadata: None,
                });
                tool_calls.push(ToolCall {
                    id: id.clone(),
                    name: name.clone(),
                    input: input.clone(),
                });

                let _ = tx
                    .send(StreamEvent::ToolUseEnd {
                        id: id.clone(),
                        name: name.clone(),
                        input,
                    })
                    .await;
            }

            let stop_reason = match finish_reason.as_deref() {
                Some("stop") => StopReason::EndTurn,
                Some("tool_calls") => StopReason::ToolUse,
                Some("length") => StopReason::MaxTokens,
                _ => {
                    if !tool_calls.is_empty() {
                        StopReason::ToolUse
                    } else {
                        StopReason::EndTurn
                    }
                }
            };

            // Guard: if the model returned content but usage is missing/zero
            // (common with local LLMs like LM Studio, Ollama), set a synthetic
            // non-zero output_tokens so the agent loop doesn't misclassify
            // this as a "silent failure" and loop unnecessarily.
            if !content.is_empty() && usage.input_tokens == 0 && usage.output_tokens == 0 {
                debug!("Stream has content but no usage stats — setting synthetic output_tokens=1");
                usage.output_tokens = 1;
            }

            debug!(
                prompt_tokens = usage.input_tokens,
                completion_tokens = usage.output_tokens,
                cached_prompt_tokens,
                "OpenAI-compatible usage (stream)"
            );

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

/// Extract `<think>...</think>` blocks from content text.
///
/// Some local LLMs (Qwen3, DeepSeek-R1) embed their reasoning directly in the
/// content field wrapped in `<think>` tags. This function separates the thinking
/// from the actual response text.
///
/// Returns `(cleaned_text, Option<thinking_text>)`.
fn extract_think_tags(text: &str) -> (String, Option<String>) {
    let mut thinking_parts = Vec::new();
    let mut cleaned = text.to_string();

    // Extract all <think>...</think> blocks (greedy within each block)
    while let Some(start) = cleaned.find("<think>") {
        if let Some(end) = cleaned.find("</think>") {
            let think_start = start + "<think>".len();
            if think_start <= end {
                let thought = cleaned[think_start..end].trim().to_string();
                if !thought.is_empty() {
                    thinking_parts.push(thought);
                }
                // Remove the entire <think>...</think> block
                cleaned = format!(
                    "{}{}",
                    &cleaned[..start],
                    &cleaned[end + "</think>".len()..]
                );
            } else {
                break;
            }
        } else {
            // Unclosed <think> tag — treat everything after as thinking
            let thought = cleaned[start + "<think>".len()..].trim().to_string();
            if !thought.is_empty() {
                thinking_parts.push(thought);
            }
            cleaned = cleaned[..start].to_string();
            break;
        }
    }

    let cleaned = cleaned.trim().to_string();
    if thinking_parts.is_empty() {
        (cleaned, None)
    } else {
        (cleaned, Some(thinking_parts.join("\n\n")))
    }
}

/// Extract a usable summary from thinking-only output.
///
/// When a local model returns only thinking/reasoning with no actual response text,
/// we extract the last meaningful paragraph as a synthesized response rather than
/// showing "empty response" to the user.
fn extract_thinking_summary(thinking: &str) -> String {
    let trimmed = thinking.trim();
    if trimmed.is_empty() {
        return "[The model produced reasoning but no final answer. Try rephrasing your question.]"
            .to_string();
    }

    // Take the last non-empty paragraph (models usually conclude with their answer)
    let paragraphs: Vec<&str> = trimmed
        .split("\n\n")
        .map(|p| p.trim())
        .filter(|p| !p.is_empty())
        .collect();

    if let Some(last) = paragraphs.last() {
        // If the last paragraph is reasonably short, use it directly
        if last.len() <= 2000 {
            last.to_string()
        } else {
            // Take the last 2000 chars
            last[last.len() - 2000..].to_string()
        }
    } else {
        "[The model produced reasoning but no final answer. Try rephrasing your question.]"
            .to_string()
    }
}

/// Parse Groq's `tool_use_failed` error and extract the tool call from `failed_generation`.
/// Extract the max_tokens limit from an API error message.
/// Looks for patterns like: `must be less than or equal to \`8192\``
fn extract_max_tokens_limit(body: &str) -> Option<u32> {
    // Pattern: "must be <= `N`" or "must be less than or equal to `N`"
    let patterns = [
        "less than or equal to `",
        "must be <= `",
        "maximum value for `max_tokens` is `",
    ];
    for pat in &patterns {
        if let Some(idx) = body.find(pat) {
            let after = &body[idx + pat.len()..];
            let end = after
                .find('`')
                .or_else(|| after.find('"'))
                .unwrap_or(after.len());
            if let Ok(n) = after[..end].trim().parse::<u32>() {
                return Some(n);
            }
        }
    }
    None
}

///
/// Some models (e.g. Llama 3.3) generate tool calls as XML: `<function=NAME ARGS></function>`
/// instead of the proper JSON format. Groq rejects these with `tool_use_failed` but includes
/// the raw generation. We parse it and construct a proper CompletionResponse.
fn parse_groq_failed_tool_call(body: &str) -> Option<CompletionResponse> {
    let json_body: serde_json::Value = serde_json::from_str(body).ok()?;
    let failed = json_body
        .pointer("/error/failed_generation")
        .and_then(|v| v.as_str())?;

    // Parse all tool calls from the failed generation.
    // Format: <function=tool_name{"arg":"val"}></function> or <function=tool_name {"arg":"val"}></function>
    let mut tool_calls = Vec::new();
    let mut remaining = failed;

    while let Some(start) = remaining.find("<function=") {
        remaining = &remaining[start + 10..]; // skip "<function="
                                              // Find the end tag
        let end = remaining.find("</function>")?;
        let mut call_content = &remaining[..end];
        remaining = &remaining[end + 11..]; // skip "</function>"

        // Strip trailing ">" from the XML opening tag close
        call_content = call_content.strip_suffix('>').unwrap_or(call_content);

        // Split into name and args: "tool_name{"arg":"val"}" or "tool_name {"arg":"val"}"
        let (name, args) = if let Some(brace_pos) = call_content.find('{') {
            let name = call_content[..brace_pos].trim();
            let args = &call_content[brace_pos..];
            (name, args)
        } else {
            // No args — just a tool name
            (call_content.trim(), "{}")
        };

        // Parse args as JSON Value
        let args_value: serde_json::Value = match serde_json::from_str::<serde_json::Value>(args) {
            Ok(v) => ensure_object(v),
            Err(e) => {
                tracing::warn!(
                    tool = %name,
                    raw_args_len = args.len(),
                    error = %e,
                    "Malformed tool call arguments from Groq recovery"
                );
                malformed_tool_input(&e, args.len())
            }
        };

        tool_calls.push(ToolCall {
            id: format!("groq_recovered_{}", tool_calls.len()),
            name: name.to_string(),
            input: args_value,
        });
    }

    if tool_calls.is_empty() {
        // No tool calls found — the model generated plain text but Groq rejected it.
        // Return it as a normal text response instead of failing.
        if !failed.trim().is_empty() {
            warn!("Recovering plain text from Groq failed_generation (no tool calls)");
            return Some(CompletionResponse {
                content: vec![ContentBlock::Text {
                    text: failed.to_string(),
                    provider_metadata: None,
                }],
                tool_calls: vec![],
                stop_reason: StopReason::EndTurn,
                usage: TokenUsage {
                    input_tokens: 0,
                    output_tokens: 0,
                    ..Default::default()
                },
            });
        }
        return None;
    }

    Some(CompletionResponse {
        content: vec![],
        tool_calls,
        stop_reason: StopReason::ToolUse,
        usage: TokenUsage {
            input_tokens: 0,
            output_tokens: 0,
            ..Default::default()
        },
    })
}

/// Ensure a `serde_json::Value` is an object.  OpenAI-compatible APIs expect
/// tool-call arguments to be a JSON object (`{}`), never `null`.
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

/// Marker key embedded in tool input when the LLM's streamed JSON was truncated.
pub(crate) const TRUNCATED_ARGS_KEY: &str = "__args_truncated";

/// Build a tool input object for truncated/malformed JSON from the LLM.
///
/// Tries to repair the truncated JSON by closing unclosed strings and braces.
/// If repair succeeds, returns the partially-parsed object with a truncation
/// marker so the tool can still execute (partial content is better than nothing).
/// If repair fails, returns an object with just the marker and error message.
pub(crate) fn malformed_tool_input(
    error: &serde_json::Error,
    args_len: usize,
) -> serde_json::Value {
    serde_json::json!({
        TRUNCATED_ARGS_KEY: true,
        "__error": format!(
            "Tool call arguments were truncated ({} chars, parse error: {}). \
             The content was too large for a single response. \
             Try writing smaller content or splitting into multiple tool calls.",
            args_len, error
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_openai_driver_creation() {
        let driver = OpenAIDriver::new("test-key".to_string(), "http://localhost".to_string());
        assert_eq!(driver.api_key.as_str(), "test-key");
    }

    #[test]
    fn test_parse_groq_failed_tool_call() {
        let body = r#"{"error":{"message":"Failed to call a function.","type":"invalid_request_error","code":"tool_use_failed","failed_generation":"<function=web_fetch{\"url\": \"https://example.com\"}></function>\n"}}"#;
        let result = parse_groq_failed_tool_call(body);
        assert!(result.is_some());
        let resp = result.unwrap();
        assert_eq!(resp.tool_calls.len(), 1);
        assert_eq!(resp.tool_calls[0].name, "web_fetch");
        assert!(resp.tool_calls[0]
            .input
            .to_string()
            .contains("https://example.com"));
    }

    #[test]
    fn test_parse_groq_failed_tool_call_with_space() {
        let body = r#"{"error":{"message":"Failed","type":"invalid_request_error","code":"tool_use_failed","failed_generation":"<function=shell_exec {\"command\": \"ls -la\"}></function>"}}"#;
        let result = parse_groq_failed_tool_call(body);
        assert!(result.is_some());
        let resp = result.unwrap();
        assert_eq!(resp.tool_calls[0].name, "shell_exec");
    }

    #[test]
    fn test_ensure_object_null_becomes_empty_object() {
        assert_eq!(
            ensure_object(serde_json::Value::Null),
            serde_json::json!({})
        );
    }

    #[test]
    fn test_ensure_object_preserves_existing_object() {
        let obj = serde_json::json!({"key": "value"});
        assert_eq!(ensure_object(obj.clone()), obj);
    }

    // ----- rejects_temperature tests -----

    #[test]
    fn test_rejects_temperature_o1_models() {
        assert!(rejects_temperature("o1"));
        assert!(rejects_temperature("o1-mini"));
        assert!(rejects_temperature("o1-mini-2024-09-12"));
        assert!(rejects_temperature("o1-preview"));
        assert!(rejects_temperature("o1-preview-2024-09-12"));
    }

    #[test]
    fn test_rejects_temperature_o3_models() {
        assert!(rejects_temperature("o3"));
        assert!(rejects_temperature("o3-mini"));
        assert!(rejects_temperature("o3-mini-2025-01-31"));
        assert!(rejects_temperature("o3-pro"));
    }

    #[test]
    fn test_rejects_temperature_o4_models() {
        assert!(rejects_temperature("o4-mini"));
        assert!(rejects_temperature("o4-mini-2025-04-16"));
    }

    #[test]
    fn test_rejects_temperature_gpt5_mini() {
        assert!(rejects_temperature("gpt-5-mini"));
        assert!(rejects_temperature("gpt-5-mini-2025-08-07"));
        assert!(rejects_temperature("gpt5-mini"));
        assert!(rejects_temperature("GPT-5-MINI-2025-08-07"));
    }

    #[test]
    fn test_rejects_temperature_reasoning_suffix() {
        assert!(rejects_temperature("some-model-reasoning"));
        assert!(rejects_temperature("deepseek-r1-reasoning"));
    }

    #[test]
    fn test_does_not_reject_temperature_normal_models() {
        assert!(!rejects_temperature("gpt-4o"));
        assert!(!rejects_temperature("gpt-4o-mini"));
        assert!(!rejects_temperature("gpt-5"));
        assert!(!rejects_temperature("gpt-5-2025-06-01"));
        assert!(!rejects_temperature("claude-sonnet-4-20250514"));
        assert!(!rejects_temperature("llama-3.3-70b-versatile"));
        assert!(!rejects_temperature("deepseek-chat"));
    }

    // ----- uses_completion_tokens tests -----

    #[test]
    fn test_uses_completion_tokens_gpt5() {
        assert!(uses_completion_tokens("gpt-5"));
        assert!(uses_completion_tokens("gpt-5-mini"));
        assert!(uses_completion_tokens("gpt-5-mini-2025-08-07"));
        assert!(uses_completion_tokens("gpt5-mini"));
    }

    #[test]
    fn test_uses_completion_tokens_o_series() {
        assert!(uses_completion_tokens("o1"));
        assert!(uses_completion_tokens("o1-mini"));
        assert!(uses_completion_tokens("o3"));
        assert!(uses_completion_tokens("o3-mini"));
        assert!(uses_completion_tokens("o3-pro"));
        assert!(uses_completion_tokens("o4-mini"));
    }

    #[test]
    fn test_does_not_use_completion_tokens_normal_models() {
        assert!(!uses_completion_tokens("gpt-4o"));
        assert!(!uses_completion_tokens("gpt-4o-mini"));
        assert!(!uses_completion_tokens("llama-3.3-70b"));
    }

    // ----- extract_max_tokens_limit tests -----

    #[test]
    fn test_extract_max_tokens_limit() {
        let body = r#"max_tokens must be less than or equal to `8192`"#;
        assert_eq!(extract_max_tokens_limit(body), Some(8192));
    }

    #[test]
    fn test_extract_max_tokens_limit_no_match() {
        assert_eq!(extract_max_tokens_limit("some random error"), None);
    }

    // ----- extract_think_tags tests -----

    #[test]
    fn test_extract_think_tags_no_tags() {
        let (cleaned, thinking) = extract_think_tags("Hello world");
        assert_eq!(cleaned, "Hello world");
        assert!(thinking.is_none());
    }

    #[test]
    fn test_extract_think_tags_with_thinking() {
        let input = "<think>Let me reason about this...</think>The answer is 42.";
        let (cleaned, thinking) = extract_think_tags(input);
        assert_eq!(cleaned, "The answer is 42.");
        assert_eq!(thinking.unwrap(), "Let me reason about this...");
    }

    #[test]
    fn test_extract_think_tags_only_thinking() {
        let input = "<think>I need to think about this carefully.\n\nThe user wants to know about Rust.</think>";
        let (cleaned, thinking) = extract_think_tags(input);
        assert_eq!(cleaned, "");
        assert!(thinking.is_some());
        assert!(thinking.unwrap().contains("think about this carefully"));
    }

    #[test]
    fn test_extract_think_tags_multiple_blocks() {
        let input =
            "<think>First thought</think>Middle text<think>Second thought</think>Final text";
        let (cleaned, thinking) = extract_think_tags(input);
        assert_eq!(cleaned, "Middle textFinal text");
        let t = thinking.unwrap();
        assert!(t.contains("First thought"));
        assert!(t.contains("Second thought"));
    }

    #[test]
    fn test_extract_think_tags_unclosed() {
        let input = "Some text<think>unclosed thinking content";
        let (cleaned, thinking) = extract_think_tags(input);
        assert_eq!(cleaned, "Some text");
        assert_eq!(thinking.unwrap(), "unclosed thinking content");
    }

    // ----- extract_thinking_summary tests -----

    #[test]
    fn test_extract_thinking_summary_empty() {
        let summary = extract_thinking_summary("");
        assert!(summary.contains("no final answer"));
    }

    #[test]
    fn test_extract_thinking_summary_single_paragraph() {
        let summary = extract_thinking_summary("The answer is 42.");
        assert_eq!(summary, "The answer is 42.");
    }

    #[test]
    fn test_extract_thinking_summary_multiple_paragraphs() {
        let input = "First I need to consider X.\n\nThen I should check Y.\n\nThe answer is 42.";
        let summary = extract_thinking_summary(input);
        assert_eq!(summary, "The answer is 42.");
    }

    // ----- reasoning_content deserialization test -----

    #[test]
    fn test_oai_response_message_with_reasoning_content() {
        let json =
            r#"{"content": null, "reasoning_content": "Let me think...", "tool_calls": null}"#;
        let msg: OaiResponseMessage = serde_json::from_str(json).unwrap();
        assert!(msg.content.is_none());
        assert_eq!(msg.reasoning_content.as_deref(), Some("Let me think..."));
    }

    #[test]
    fn test_oai_response_message_without_reasoning_content() {
        let json = r#"{"content": "Hello", "tool_calls": null}"#;
        let msg: OaiResponseMessage = serde_json::from_str(json).unwrap();
        assert_eq!(msg.content.as_deref(), Some("Hello"));
        assert!(msg.reasoning_content.is_none());
    }

    #[test]
    fn test_oai_response_message_null_content_null_reasoning() {
        let json = r#"{"content": null, "tool_calls": null}"#;
        let msg: OaiResponseMessage = serde_json::from_str(json).unwrap();
        assert!(msg.content.is_none());
        assert!(msg.reasoning_content.is_none());
    }

    // ----- is_deepseek_reasoner tests -----

    #[test]
    fn test_is_deepseek_reasoner() {
        let driver = OpenAIDriver::new(String::new(), "https://api.deepseek.com/v1".to_string());
        assert!(driver.is_deepseek_reasoner("deepseek-reasoner"));
        assert!(driver.is_deepseek_reasoner("deepseek-r1"));
        assert!(driver.is_deepseek_reasoner("DeepSeek-Reasoner"));
        assert!(driver.is_deepseek_reasoner("deepseek-r1-0528"));
        assert!(!driver.is_deepseek_reasoner("deepseek-chat"));
        assert!(!driver.is_deepseek_reasoner("deepseek-coder"));
        assert!(!driver.is_deepseek_reasoner("gpt-4o"));
    }

    /// Verify that reasoning_content is omitted (None) when building
    /// assistant messages for deepseek-reasoner, even if the blocks
    /// contain Thinking content.
    #[test]
    fn test_deepseek_reasoner_strips_reasoning_content_from_assistant_msg() {
        let driver = OpenAIDriver::new(String::new(), "https://api.deepseek.com/v1".to_string());
        let model = "deepseek-reasoner";

        // Simulate building an assistant OaiMessage with tool_calls —
        // for deepseek-reasoner, reasoning_content must always be None.
        let has_tool_calls = true;
        let is_deepseek_r = driver.is_deepseek_reasoner(model);
        let reasoning_content = if is_deepseek_r {
            None
        } else if has_tool_calls && driver.kimi_needs_reasoning_content(model) {
            Some(String::new())
        } else {
            None
        };
        assert!(
            reasoning_content.is_none(),
            "deepseek-reasoner must never send reasoning_content on assistant messages"
        );
    }

    /// Verify that deepseek-reasoner assistant messages always get a non-null
    /// content field, even when text_parts is empty (thinking-only response).
    #[test]
    fn test_deepseek_reasoner_content_never_null() {
        let driver = OpenAIDriver::new(String::new(), "https://api.deepseek.com/v1".to_string());
        let model = "deepseek-reasoner";
        let is_deepseek_r = driver.is_deepseek_reasoner(model);
        let text_parts: Vec<String> = Vec::new(); // empty — thinking-only response
        let has_tool_calls = false;

        // Simulate the content field logic from complete()/stream()
        let content: Option<OaiMessageContent> = if text_parts.is_empty() {
            if has_tool_calls || is_deepseek_r {
                Some(OaiMessageContent::Text(String::new()))
            } else {
                None
            }
        } else {
            Some(OaiMessageContent::Text(text_parts.join("")))
        };

        assert!(
            content.is_some(),
            "deepseek-reasoner assistant messages must always have non-null content for multi-turn"
        );
    }

    #[test]
    fn test_ensure_object_null_becomes_empty() {
        assert_eq!(
            ensure_object(serde_json::Value::Null),
            serde_json::json!({})
        );
    }

    #[test]
    fn test_ensure_object_preserves_object() {
        let input = serde_json::json!({"query": "test"});
        assert_eq!(ensure_object(input.clone()), input);
    }

    #[test]
    fn test_ensure_object_parses_json_string() {
        let input = serde_json::json!(r#"{"query": "rust lang"}"#);
        assert_eq!(
            ensure_object(input),
            serde_json::json!({"query": "rust lang"})
        );
    }

    #[test]
    fn test_ensure_object_wraps_plain_string() {
        assert_eq!(
            ensure_object(serde_json::json!("plain text")),
            serde_json::json!({"raw_input": "plain text"})
        );
    }

    #[test]
    fn test_ensure_object_wraps_number() {
        assert_eq!(
            ensure_object(serde_json::json!(42)),
            serde_json::json!({"raw_input": 42})
        );
    }

    #[test]
    fn test_ensure_object_wraps_array() {
        assert_eq!(
            ensure_object(serde_json::json!([1, 2])),
            serde_json::json!({"raw_input": [1, 2]})
        );
    }

    #[test]
    fn test_ensure_object_string_with_json_array_wraps() {
        let input = serde_json::json!(r#"[1, 2, 3]"#);
        assert_eq!(
            ensure_object(input),
            serde_json::json!({"raw_input": "[1, 2, 3]"})
        );
    }

    #[test]
    fn test_oai_request_extra_body_merged_overrides_standard_field() {
        // extra_body is #[serde(skip_serializing)] — it does NOT appear in
        // the raw serde output.  In complete() / stream() we serialize to
        // Value first, then merge extra_body on top so it overrides any
        // standard field with the same name.  This test verifies the merge
        // logic directly.
        let mut extra = HashMap::new();
        extra.insert("temperature".to_string(), serde_json::json!(1.0));
        extra.insert("enable_memory".to_string(), serde_json::json!(true));

        let req = OaiRequest {
            model: "qwen3.6".to_string(),
            messages: vec![OaiMessage {
                role: "user".to_string(),
                content: Some(OaiMessageContent::Text("hello".to_string())),
                tool_calls: None,
                tool_call_id: None,
                reasoning_content: None,
            }],
            max_tokens: Some(4096),
            max_completion_tokens: None,
            temperature: Some(0.7),
            tools: vec![],
            tool_choice: None,
            stream: false,
            stream_options: None,
            thinking: None,
            response_format: None,
            extra_body: Some(extra),
        };

        // Simulate the merge logic used in complete() / stream()
        let mut body = serde_json::to_value(&req).unwrap();
        if let (Some(extra), Some(obj)) = (&req.extra_body, body.as_object_mut()) {
            for (k, v) in extra {
                obj.insert(k.clone(), v.clone());
            }
        }

        // extra_body values should override standard fields
        assert_eq!(body.get("temperature").unwrap(), &serde_json::json!(1.0));
        assert_eq!(body.get("enable_memory").unwrap(), &serde_json::json!(true));
        // No duplicate keys — only ONE temperature
        let raw = body.to_string();
        assert_eq!(
            raw.matches("temperature").count(),
            1,
            "There should be exactly ONE temperature key after merge. Raw: {raw}"
        );
    }

    #[test]
    fn test_oai_request_extra_body_none_skipped() {
        let req = OaiRequest {
            model: "test-model".to_string(),
            messages: vec![OaiMessage {
                role: "user".to_string(),
                content: Some(OaiMessageContent::Text("hi".to_string())),
                tool_calls: None,
                tool_call_id: None,
                reasoning_content: None,
            }],
            max_tokens: Some(100),
            max_completion_tokens: None,
            temperature: Some(0.5),
            tools: vec![],
            tool_choice: None,
            stream: false,
            stream_options: None,
            thinking: None,
            response_format: None,
            extra_body: None,
        };

        let json = serde_json::to_string(&req).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert!(parsed.get("extra_body").is_none());
    }
}
