//! Native Ollama API driver.
//!
//! Targets the **native** Ollama protocol (`POST /api/chat`, NDJSON
//! streaming, first-class `think` / `thinking` / `tool_calls` fields).
//! This is distinct from the OpenAI-compatibility shim Ollama also ships
//! at `/v1/chat/completions` — the native protocol is the upstream's
//! first-class surface, exposes more accurate token counts
//! (`prompt_eval_count` / `eval_count`), preserves model-side reasoning
//! without relying on `<think>` tag parsing, and is the only protocol
//! that "Ollama-protocol" servers like AMD's Lemonade implement
//! (#4810). Real Ollama supports both, so swapping to native is a
//! lossless change for direct-Ollama users while it unblocks the long
//! tail of compat-only-via-native servers.
//!
//! ## Wire format
//!
//! ### Request
//! ```jsonc
//! {
//!   "model": "llama3.1:8b",
//!   "messages": [
//!     {"role": "system",    "content": "..."},
//!     {"role": "user",      "content": "...", "images": ["<base64>"]},
//!     {"role": "assistant", "content": "...", "thinking": "...",
//!      "tool_calls": [{"function": {"name": "X", "arguments": {...}}}]},
//!     {"role": "tool",      "content": "...", "tool_name": "X"}
//!   ],
//!   "stream": true,
//!   "tools": [{"type": "function", "function": {...}}],
//!   "think": true,
//!   "format": "json" | <schema>,
//!   "options": {"temperature": 0.7, "num_predict": 1024}
//! }
//! ```
//!
//! ### Response (non-streaming)
//! ```jsonc
//! {
//!   "model": "...", "created_at": "...",
//!   "message": {
//!     "role": "assistant", "content": "...", "thinking": "...",
//!     "tool_calls": [{"function": {"name": "X", "arguments": {...}}}]
//!   },
//!   "done": true, "done_reason": "stop"|"length"|...,
//!   "prompt_eval_count": 26, "eval_count": 298
//! }
//! ```
//!
//! ### Response (streaming, NDJSON — one JSON object per line)
//! Each chunk has the same envelope; non-final chunks carry incremental
//! `content` / `thinking`, the final chunk carries `done: true` plus
//! aggregated counts and (when applicable) the full `tool_calls` array.
//!
//! ### Differences from OpenAI-compat that drove design decisions
//!
//! - **No `id` on tool calls.** We synthesize stable per-call IDs
//!   (`ollama-call-<uuid>`) at parse time so the agent loop's
//!   tool-result round-trip keeps working. On the way back, tool
//!   results carry `tool_name` (which Ollama indexes on) rather than
//!   `tool_call_id`, mirroring the upstream protocol.
//! - **`arguments` is an object, not a string.** No `serde_json::from_str`
//!   round-trip needed; we still defensively normalise via the shared
//!   `ensure_object` helper for buggy models.
//! - **Token usage in `eval_count` / `prompt_eval_count`.** When absent
//!   (stream cancelled, server crash mid-turn, …) we leave them at 0
//!   rather than fabricating, matching the OpenAI driver's policy.

use crate::llm_driver::{
    CompletionRequest, CompletionResponse, LlmDriver, LlmError, LlmFamily, StreamEvent,
};
use async_trait::async_trait;
use futures::StreamExt;
use librefang_types::message::{ContentBlock, MessageContent, Role, StopReason, TokenUsage};
use librefang_types::tool::ToolCall;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tracing::{debug, info, warn};
use uuid::Uuid;
use zeroize::Zeroizing;

use super::openai::{ensure_object, parse_tool_args};
use super::trace_headers::build_trace_header_map;

/// Native Ollama API driver. See module-level docs for protocol details.
pub struct OllamaDriver {
    api_key: Zeroizing<String>,
    base_url: String,
    client: reqwest::Client,
    request_timeout_secs: Option<u64>,
    emit_caller_trace_headers: bool,
}

impl OllamaDriver {
    /// Create a new Ollama driver. `api_key` may be empty for the default
    /// localhost setup; tunnelled / hosted Ollama servers use Bearer auth.
    pub fn new(api_key: String, base_url: String) -> Self {
        Self::with_proxy(api_key, base_url, None)
    }

    /// Create a new Ollama driver with an optional per-provider proxy.
    pub fn with_proxy(api_key: String, base_url: String, proxy_url: Option<&str>) -> Self {
        Self::with_proxy_and_timeout(api_key, base_url, proxy_url, None)
    }

    /// Create a new Ollama driver with optional proxy and request timeout.
    pub fn with_proxy_and_timeout(
        api_key: String,
        base_url: String,
        proxy_url: Option<&str>,
        request_timeout_secs: Option<u64>,
    ) -> Self {
        let client = match proxy_url {
            Some(url) => librefang_http::proxied_client_with_override(url).unwrap_or_else(|e| {
                tracing::warn!(url, error = %e, "Invalid per-provider proxy URL, using global proxy");
                librefang_http::proxied_client()
            }),
            None => librefang_http::proxied_client(),
        };
        let base_url = sanitize_base_url(&base_url);
        Self {
            api_key: Zeroizing::new(api_key),
            base_url,
            client,
            request_timeout_secs,
            emit_caller_trace_headers: true,
        }
    }

    /// Override trace-header emission. Mirrors `OpenAIDriver` so kernel
    /// plumbing can pass the toml flag through uniformly.
    pub fn with_emit_caller_trace_headers(mut self, emit: bool) -> Self {
        self.emit_caller_trace_headers = emit;
        self
    }

    /// Build the wire request from a `CompletionRequest`. Shared between
    /// `complete()` and `stream()`; the caller flips `stream` on the
    /// returned struct as appropriate.
    fn build_request(&self, request: &CompletionRequest) -> Result<OllamaRequest, LlmError> {
        let mut messages: Vec<OllamaMessage> = Vec::new();

        if let Some(ref system) = request.system {
            messages.push(OllamaMessage {
                role: "system".to_string(),
                content: Some(system.clone()),
                ..Default::default()
            });
        }

        for msg in request.messages.iter() {
            match (&msg.role, &msg.content) {
                (Role::System, MessageContent::Text(text)) if request.system.is_none() => {
                    messages.push(OllamaMessage {
                        role: "system".to_string(),
                        content: Some(text.clone()),
                        ..Default::default()
                    });
                }
                (Role::System, MessageContent::Text(_)) => {
                    // System already extracted into request.system — drop the
                    // duplicate so the model doesn't see two competing system
                    // prompts.
                }
                (Role::User, MessageContent::Text(text)) => {
                    messages.push(OllamaMessage {
                        role: "user".to_string(),
                        content: Some(text.clone()),
                        ..Default::default()
                    });
                }
                (Role::Assistant, MessageContent::Text(text)) => {
                    messages.push(OllamaMessage {
                        role: "assistant".to_string(),
                        content: Some(text.clone()),
                        ..Default::default()
                    });
                }
                (Role::User, MessageContent::Blocks(blocks)) => {
                    let mut text_buf = String::new();
                    let mut images: Vec<String> = Vec::new();
                    let mut emitted_tool_result = false;

                    for block in blocks {
                        match block {
                            ContentBlock::ToolResult {
                                tool_name, content, ..
                            } => {
                                emitted_tool_result = true;
                                messages.push(OllamaMessage {
                                    role: "tool".to_string(),
                                    content: Some(if content.is_empty() {
                                        "(empty)".to_string()
                                    } else {
                                        content.clone()
                                    }),
                                    tool_name: if tool_name.is_empty() {
                                        None
                                    } else {
                                        Some(tool_name.clone())
                                    },
                                    ..Default::default()
                                });
                            }
                            ContentBlock::Text { text, .. } => {
                                if !text_buf.is_empty() {
                                    text_buf.push_str("\n\n");
                                }
                                text_buf.push_str(text);
                            }
                            ContentBlock::Image { data, .. } => {
                                images.push(data.clone());
                            }
                            ContentBlock::ImageFile { path, .. } => match std::fs::read(path) {
                                Ok(bytes) => {
                                    use base64::Engine;
                                    images.push(
                                        base64::engine::general_purpose::STANDARD.encode(&bytes),
                                    );
                                }
                                Err(e) => {
                                    warn!(
                                        path = %path,
                                        error = %e,
                                        "ImageFile missing, skipping"
                                    );
                                }
                            },
                            ContentBlock::Thinking { .. } | ContentBlock::ToolUse { .. } => {
                                // Thinking and ToolUse on a User role are nonsensical —
                                // session_repair shouldn't produce them, and the
                                // Anthropic / OpenAI drivers also drop them silently.
                            }
                            ContentBlock::Unknown => {}
                        }
                    }

                    if !emitted_tool_result && (!text_buf.is_empty() || !images.is_empty()) {
                        messages.push(OllamaMessage {
                            role: "user".to_string(),
                            content: if text_buf.is_empty() {
                                None
                            } else {
                                Some(text_buf)
                            },
                            images: if images.is_empty() {
                                None
                            } else {
                                Some(images)
                            },
                            ..Default::default()
                        });
                    }
                }
                (Role::System, MessageContent::Blocks(blocks)) => {
                    // Rare but legal: a system message arrived as blocks
                    // (e.g. from a session-repair pass). Ollama only
                    // accepts string content on system, so flatten the
                    // text blocks; warn so operators notice unexpected
                    // shapes rather than silently dropping them.
                    let mut text_buf = String::new();
                    let mut had_non_text = false;
                    for block in blocks {
                        match block {
                            ContentBlock::Text { text, .. } => {
                                if !text_buf.is_empty() {
                                    text_buf.push_str("\n\n");
                                }
                                text_buf.push_str(text);
                            }
                            _ => had_non_text = true,
                        }
                    }
                    if had_non_text {
                        warn!(
                            "System message contained non-text blocks; \
                             Ollama requires string content on system, \
                             non-text blocks dropped",
                        );
                    }
                    if !text_buf.is_empty() && request.system.is_none() {
                        messages.push(OllamaMessage {
                            role: "system".to_string(),
                            content: Some(text_buf),
                            ..Default::default()
                        });
                    }
                }
                (Role::Assistant, MessageContent::Blocks(blocks)) => {
                    let mut text_parts: Vec<String> = Vec::new();
                    let mut thinking_parts: Vec<String> = Vec::new();
                    let mut tool_calls: Vec<OllamaToolCall> = Vec::new();

                    for block in blocks {
                        match block {
                            ContentBlock::Text { text, .. } => text_parts.push(text.clone()),
                            ContentBlock::Thinking { thinking, .. } => {
                                thinking_parts.push(thinking.clone());
                            }
                            ContentBlock::ToolUse { name, input, .. } => {
                                tool_calls.push(OllamaToolCall {
                                    function: OllamaFunctionCall {
                                        name: name.clone(),
                                        arguments: input.clone(),
                                    },
                                });
                            }
                            _ => {}
                        }
                    }

                    let content = if text_parts.is_empty() {
                        None
                    } else {
                        Some(text_parts.join(""))
                    };
                    let thinking = if thinking_parts.is_empty() {
                        None
                    } else {
                        Some(thinking_parts.join("\n\n"))
                    };
                    messages.push(OllamaMessage {
                        role: "assistant".to_string(),
                        content,
                        thinking,
                        tool_calls: if tool_calls.is_empty() {
                            None
                        } else {
                            Some(tool_calls)
                        },
                        ..Default::default()
                    });
                }
            }
        }

        if messages.is_empty() {
            return Err(LlmError::Api {
                status: 0,
                message: "Cannot send request with no messages — \
                          this usually means aggressive history trimming emptied \
                          the conversation"
                    .to_string(),
                code: None,
            });
        }

        let tools: Vec<OllamaTool> = request
            .tools
            .iter()
            .map(|t| OllamaTool {
                tool_type: "function".to_string(),
                function: OllamaToolDef {
                    name: t.name.clone(),
                    description: t.description.clone(),
                    parameters: librefang_types::tool::normalize_schema_for_provider(
                        &t.input_schema,
                        "openai",
                    ),
                },
            })
            .collect();

        let format = request
            .response_format
            .as_ref()
            .and_then(ollama_response_format);

        let options = if request.temperature == 0.0 && request.max_tokens == 0 {
            None
        } else {
            Some(OllamaOptions {
                temperature: Some(request.temperature),
                num_predict: if request.max_tokens > 0 {
                    Some(request.max_tokens)
                } else {
                    None
                },
            })
        };

        Ok(OllamaRequest {
            model: request.model.clone(),
            messages,
            stream: None,
            tools,
            // Native first-class field — drives whether reasoning models
            // run their chain-of-thought phase. We mirror the legacy
            // OpenAI-shim path's contract: `think` is ALWAYS sent, and
            // the toggle is driven by `request.thinking.is_some()`.
            //
            // Why not "let the model use its default" (i.e. omit the
            // field): qwen3 / deepseek-r1 / gpt-oss default `think: true`
            // upstream. Omitting the field would silently flip
            // chain-of-thought on for users who never enabled the
            // dashboard's deep-thinking toggle, surfacing reasoning text
            // and adding latency. Keeping the explicit `false` preserves
            // the pre-#4810 user-visible behaviour exactly.
            think: Some(request.thinking.is_some()),
            format,
            options,
            extra_body: request.extra_body.clone(),
        })
    }
}

/// Strip a trailing `/v1` from a user-provided base URL when (and only
/// when) it is the entire path component, logging an INFO at
/// construction time so the migration is visible to operators.
///
/// Existing user configs predate the move to the native API and still
/// pin `base_url = "http://127.0.0.1:11434/v1"`. Quietly migrating
/// instead of failing-closed avoids paper-cut breakage on upgrade.
///
/// The strip is gated on "URL has no path beyond `/v1`" so reverse-proxy
/// mounts like `http://api.corp.com/openai/v1` are left alone — those
/// users meant a non-standard mount point, and stripping would compose
/// `…/openai/api/chat` (still wrong) or worse, mask a misconfiguration
/// they need to see. Inputs we explicitly migrate:
///
/// - `http://x:11434/v1`        → `http://x:11434`
/// - `http://x:11434/v1/`       → `http://x:11434`
/// - `http://x:11434/v1///`     → `http://x:11434`
/// - `http://[::1]:11434/v1`    → `http://[::1]:11434`
///
/// Inputs we DO NOT touch (path is more than `/v1`):
///
/// - `http://api.corp.com/openai/v1`
/// - `http://api.corp.com/api/v1`
fn sanitize_base_url(input: &str) -> String {
    let trimmed = input.trim_end_matches('/');
    if let Some(stripped) = trimmed.strip_suffix("/v1") {
        // Decide whether `/v1` was the entire path or a trailing segment
        // of a custom mount. Look at what's left after `scheme://`: if
        // there are no further `/`s, the authority is the whole tail and
        // `/v1` was the only path component → safe to strip.
        let after_scheme = match stripped.find("://") {
            Some(i) => &stripped[i + 3..],
            None => stripped,
        };
        if after_scheme.contains('/') {
            // Reverse-proxy mount with a custom path; leave alone.
            return trimmed.to_string();
        }
        info!(
            original = %input,
            sanitized = %stripped,
            rebuilt_chat_url = %format!("{stripped}/api/chat"),
            "Stripped legacy /v1 suffix from Ollama base_url; native API lives at /api/chat"
        );
        return stripped.to_string();
    }
    trimmed.to_string()
}

/// Build the `format` field from a `ResponseFormat`.
///
/// Ollama accepts the literal string `"json"` for free-form JSON output,
/// or a JSON-Schema object directly (no `{type, json_schema:{…}}`
/// envelope like OpenAI). Free-text responses leave the field unset.
fn ollama_response_format(
    rf: &librefang_types::config::ResponseFormat,
) -> Option<serde_json::Value> {
    use librefang_types::config::ResponseFormat;
    match rf {
        ResponseFormat::Text => None,
        ResponseFormat::Json => Some(serde_json::Value::String("json".to_string())),
        ResponseFormat::JsonSchema { schema, .. } => Some(schema.clone()),
    }
}

#[derive(Debug, Serialize)]
struct OllamaRequest {
    model: String,
    messages: Vec<OllamaMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stream: Option<bool>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tools: Vec<OllamaTool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    think: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    format: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    options: Option<OllamaOptions>,
    /// Pass-through extras merged into the top-level body in `complete`/`stream`
    /// so callers can override standard fields (`keep_alive`, custom sampler
    /// options, …) without driver changes.
    ///
    /// `HashMap` iteration order is non-deterministic, but the merge below
    /// goes through `serde_json::Value::Object`, which (without the
    /// `preserve_order` feature — the workspace does NOT enable it) is
    /// `BTreeMap`-backed and re-sorts keys alphabetically on serialise.
    /// The final wire body is therefore deterministic regardless of
    /// HashMap iteration, so prompt-cache stability (#3298) is preserved.
    /// If we ever turn on `preserve_order`, swap this to `BTreeMap`.
    #[serde(skip_serializing)]
    extra_body: Option<HashMap<String, serde_json::Value>>,
}

#[derive(Debug, Serialize)]
struct OllamaOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    num_predict: Option<u32>,
}

#[derive(Debug, Default, Serialize)]
struct OllamaMessage {
    role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    content: Option<String>,
    /// Base64-encoded image data, one entry per image attached to this turn.
    #[serde(skip_serializing_if = "Option::is_none")]
    images: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_calls: Option<Vec<OllamaToolCall>>,
    /// Set on `role: "tool"` messages so Ollama can correlate the result
    /// with the call. Native API uses `tool_name` rather than the
    /// OpenAI-style `tool_call_id`.
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_name: Option<String>,
    /// Round-tripped reasoning trace on assistant turns when the model
    /// emitted one previously. Lets Ollama feed the prior thinking back
    /// into context on multi-turn reasoning workflows.
    #[serde(skip_serializing_if = "Option::is_none")]
    thinking: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct OllamaToolCall {
    function: OllamaFunctionCall,
}

#[derive(Debug, Serialize, Deserialize)]
struct OllamaFunctionCall {
    name: String,
    /// Native API delivers arguments as a JSON object directly (not a
    /// stringified JSON like OpenAI compat). We still tolerate string
    /// inputs from buggy models via `ensure_object` / `parse_tool_args`.
    arguments: serde_json::Value,
}

#[derive(Debug, Serialize)]
struct OllamaTool {
    #[serde(rename = "type")]
    tool_type: String,
    function: OllamaToolDef,
}

#[derive(Debug, Serialize)]
struct OllamaToolDef {
    name: String,
    description: String,
    parameters: serde_json::Value,
}

#[derive(Debug, Deserialize)]
struct OllamaResponse {
    #[serde(default)]
    message: OllamaResponseMessage,
    #[serde(default)]
    done_reason: Option<String>,
    #[serde(default)]
    prompt_eval_count: Option<u64>,
    #[serde(default)]
    eval_count: Option<u64>,
}

#[derive(Debug, Default, Deserialize)]
struct OllamaResponseMessage {
    #[serde(default)]
    content: String,
    #[serde(default)]
    thinking: Option<String>,
    #[serde(default)]
    tool_calls: Option<Vec<OllamaToolCall>>,
}

/// Map a native Ollama `done_reason` plus the presence of tool calls onto
/// our cross-provider [`StopReason`].
///
/// Ollama's documented values are `stop`, `length`, `load`, `unload`; some
/// builds emit `tool_calls` when the turn ended on a tool request.
fn map_done_reason(reason: Option<&str>, has_tool_calls: bool) -> StopReason {
    match reason {
        Some("stop") => {
            if has_tool_calls {
                StopReason::ToolUse
            } else {
                StopReason::EndTurn
            }
        }
        Some("length") => StopReason::MaxTokens,
        Some("tool_calls") => StopReason::ToolUse,
        _ => {
            if has_tool_calls {
                StopReason::ToolUse
            } else {
                StopReason::EndTurn
            }
        }
    }
}

/// Coerce an Ollama tool-call's `arguments` field into a JSON object.
///
/// Ollama's documented contract is "object", but in practice some local
/// quantised models occasionally emit the arguments as a stringified
/// JSON value. We accept both shapes by routing through the OpenAI
/// driver's [`parse_tool_args`] (handles trailing-text recovery) and
/// then [`ensure_object`] (wraps non-object results in `raw_input` so
/// nothing is silently dropped).
fn coerce_tool_args(raw: &serde_json::Value) -> serde_json::Value {
    match raw {
        serde_json::Value::Object(_) => raw.clone(),
        serde_json::Value::String(s) => match parse_tool_args(s) {
            Ok(v) => ensure_object(v),
            Err(e) => {
                warn!(error = %e, "Ollama tool arguments string failed to parse, wrapping raw input");
                ensure_object(serde_json::Value::String(s.clone()))
            }
        },
        other => ensure_object(other.clone()),
    }
}

#[async_trait]
impl LlmDriver for OllamaDriver {
    #[tracing::instrument(
        name = "llm.complete",
        skip_all,
        fields(provider = "ollama", model = %request.model)
    )]
    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        let mut wire = self.build_request(&request)?;
        wire.stream = Some(false);

        let url = format!("{}/api/chat", self.base_url);
        debug!(url = %url, "Sending Ollama API request");

        let mut body = serde_json::to_value(&wire).map_err(|e| LlmError::Http(e.to_string()))?;
        if let (Some(extra), Some(obj)) = (&wire.extra_body, body.as_object_mut()) {
            for (k, v) in extra {
                obj.insert(k.clone(), v.clone());
            }
        }

        let mut req = self
            .client
            .post(&url)
            .header("content-type", "application/json")
            .json(&body);

        if !self.api_key.as_str().is_empty() {
            req = req.header("authorization", format!("Bearer {}", self.api_key.as_str()));
        }
        req = req.headers(build_trace_header_map(
            &[],
            &request,
            self.emit_caller_trace_headers,
        ));
        let timeout_secs = request
            .timeout_secs
            .or(self.request_timeout_secs)
            .unwrap_or(300);
        req = req.timeout(std::time::Duration::from_secs(timeout_secs));

        let resp = req
            .send()
            .await
            .map_err(|e| LlmError::Http(e.to_string()))?;
        let status = resp.status().as_u16();

        if !resp.status().is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(map_http_error(status, &body));
        }

        let body = resp
            .text()
            .await
            .map_err(|e| LlmError::Http(e.to_string()))?;
        let parsed: OllamaResponse =
            serde_json::from_str(&body).map_err(|e| LlmError::Parse(e.to_string()))?;

        let mut content_blocks: Vec<ContentBlock> = Vec::new();
        if let Some(thinking) = parsed.message.thinking.filter(|s| !s.is_empty()) {
            content_blocks.push(ContentBlock::Thinking {
                thinking,
                provider_metadata: None,
            });
        }
        if !parsed.message.content.is_empty() {
            content_blocks.push(ContentBlock::Text {
                text: parsed.message.content,
                provider_metadata: None,
            });
        }

        let mut tool_calls: Vec<ToolCall> = Vec::new();
        if let Some(calls) = parsed.message.tool_calls {
            for call in calls {
                let id = format!("ollama-call-{}", Uuid::new_v4());
                let input = coerce_tool_args(&call.function.arguments);
                content_blocks.push(ContentBlock::ToolUse {
                    id: id.clone(),
                    name: call.function.name.clone(),
                    input: input.clone(),
                    provider_metadata: None,
                });
                tool_calls.push(ToolCall {
                    id,
                    name: call.function.name,
                    input,
                });
            }
        }

        let stop_reason = map_done_reason(parsed.done_reason.as_deref(), !tool_calls.is_empty());
        let usage = TokenUsage {
            input_tokens: parsed.prompt_eval_count.unwrap_or(0),
            output_tokens: parsed.eval_count.unwrap_or(0),
            cache_creation_input_tokens: 0,
            cache_read_input_tokens: 0,
        };

        debug!(
            prompt_tokens = usage.input_tokens,
            completion_tokens = usage.output_tokens,
            tool_count = tool_calls.len(),
            done_reason = ?parsed.done_reason,
            "Ollama usage"
        );

        Ok(CompletionResponse {
            content: content_blocks,
            stop_reason,
            tool_calls,
            usage,
        })
    }

    #[tracing::instrument(
        name = "llm.stream",
        skip_all,
        fields(provider = "ollama", model = %request.model)
    )]
    async fn stream(
        &self,
        request: CompletionRequest,
        tx: tokio::sync::mpsc::Sender<StreamEvent>,
    ) -> Result<CompletionResponse, LlmError> {
        let mut wire = self.build_request(&request)?;
        wire.stream = Some(true);

        let url = format!("{}/api/chat", self.base_url);
        debug!(url = %url, "Sending Ollama streaming request");

        let mut body = serde_json::to_value(&wire).map_err(|e| LlmError::Http(e.to_string()))?;
        if let (Some(extra), Some(obj)) = (&wire.extra_body, body.as_object_mut()) {
            for (k, v) in extra {
                obj.insert(k.clone(), v.clone());
            }
        }

        let mut req = self
            .client
            .post(&url)
            .header("content-type", "application/json")
            .json(&body);
        if !self.api_key.as_str().is_empty() {
            req = req.header("authorization", format!("Bearer {}", self.api_key.as_str()));
        }
        req = req.headers(build_trace_header_map(
            &[],
            &request,
            self.emit_caller_trace_headers,
        ));
        let timeout_secs = request
            .timeout_secs
            .or(self.request_timeout_secs)
            .unwrap_or(300);
        req = req.timeout(std::time::Duration::from_secs(timeout_secs));

        let resp = req
            .send()
            .await
            .map_err(|e| LlmError::Http(e.to_string()))?;
        let status = resp.status().as_u16();
        if !resp.status().is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(map_http_error(status, &body));
        }

        // ── NDJSON aggregator ────────────────────────────────────
        let mut buffer = String::new();
        let mut text_content = String::new();
        let mut thinking_content = String::new();
        // The native API delivers tool_calls in the chunk where they're
        // finalised (never as a per-arg-character stream like OpenAI), so
        // we keep the latest snapshot and emit start/end events on stream
        // close. Indexed by position in the array Ollama returned.
        let mut tool_snapshot: Vec<OllamaToolCall> = Vec::new();
        let mut done_reason: Option<String> = None;
        let mut input_tokens: u64 = 0;
        let mut output_tokens: u64 = 0;
        let mut chunk_count: u32 = 0;
        let mut receiver_dropped = false;
        let mut utf8 = crate::utf8_stream::Utf8StreamDecoder::new();

        let mut byte_stream = resp.bytes_stream();
        while let Some(chunk_result) = byte_stream.next().await {
            if receiver_dropped {
                debug!("streaming receiver dropped; cancelling Ollama stream");
                break;
            }
            let chunk = chunk_result.map_err(|e| LlmError::Http(e.to_string()))?;
            chunk_count += 1;
            buffer.push_str(&utf8.decode(&chunk));

            // NDJSON: one complete JSON object per line.
            while let Some(pos) = buffer.find('\n') {
                let line = buffer[..pos].trim().to_string();
                buffer = buffer[pos + 1..].to_string();
                if line.is_empty() {
                    continue;
                }
                let json: serde_json::Value = match serde_json::from_str(&line) {
                    Ok(v) => v,
                    Err(e) => {
                        debug!(error = %e, line_len = line.len(), "Skipping unparseable Ollama line");
                        continue;
                    }
                };

                if let Some(msg) = json.get("message") {
                    if let Some(content) = msg.get("content").and_then(|v| v.as_str()) {
                        if !content.is_empty() {
                            text_content.push_str(content);
                            if tx
                                .send(StreamEvent::TextDelta {
                                    text: content.to_string(),
                                })
                                .await
                                .is_err()
                            {
                                receiver_dropped = true;
                            }
                        }
                    }
                    if let Some(thinking) = msg.get("thinking").and_then(|v| v.as_str()) {
                        if !thinking.is_empty() {
                            thinking_content.push_str(thinking);
                            if tx
                                .send(StreamEvent::ThinkingDelta {
                                    text: thinking.to_string(),
                                })
                                .await
                                .is_err()
                            {
                                receiver_dropped = true;
                            }
                        }
                    }
                    if let Some(calls) = msg.get("tool_calls").and_then(|v| v.as_array()) {
                        // Replace, don't append: each chunk that carries
                        // tool_calls represents the model's current view of
                        // the full call list. Streaming-tool releases
                        // (Ollama 0.5+) emit the same shape progressively.
                        let parsed: Result<Vec<OllamaToolCall>, _> =
                            serde_json::from_value(serde_json::Value::Array(calls.clone()));
                        match parsed {
                            Ok(snap) => tool_snapshot = snap,
                            // Surface protocol drift so operators can see
                            // it in logs instead of silently observing
                            // tool calls disappear.
                            Err(e) => debug!(
                                error = %e,
                                "Skipping unparseable tool_calls chunk; keeping prior snapshot",
                            ),
                        }
                    }
                }

                if json.get("done").and_then(|v| v.as_bool()) == Some(true) {
                    if let Some(reason) = json.get("done_reason").and_then(|v| v.as_str()) {
                        done_reason = Some(reason.to_string());
                    }
                    if let Some(n) = json.get("prompt_eval_count").and_then(|v| v.as_u64()) {
                        input_tokens = n;
                    }
                    if let Some(n) = json.get("eval_count").and_then(|v| v.as_u64()) {
                        output_tokens = n;
                    }
                }
            }
        }

        // Drain any half-codepoint left in the decoder. We don't append it
        // to `buffer` because there are no further line-aligned reads to
        // do — calling `finish()` exists only so the decoder explicitly
        // releases that state on a stream-truncated path (#3448 parity).
        let _ = utf8.finish();

        // ── Emit aggregated tool calls ───────────────────────────
        let mut content_blocks: Vec<ContentBlock> = Vec::new();
        if !thinking_content.is_empty() {
            content_blocks.push(ContentBlock::Thinking {
                thinking: thinking_content,
                provider_metadata: None,
            });
        }
        if !text_content.is_empty() {
            content_blocks.push(ContentBlock::Text {
                text: text_content,
                provider_metadata: None,
            });
        }

        let mut tool_calls: Vec<ToolCall> = Vec::new();
        for call in tool_snapshot {
            let id = format!("ollama-call-{}", Uuid::new_v4());
            let input = coerce_tool_args(&call.function.arguments);
            // Best-effort start/end pair so consumers expecting the
            // canonical event sequence still see one. Native streaming
            // tool calls don't carry incremental arg deltas (the snapshot
            // is always-final), so we skip ToolInputDelta entirely.
            let _ = tx
                .send(StreamEvent::ToolUseStart {
                    id: id.clone(),
                    name: call.function.name.clone(),
                })
                .await;
            content_blocks.push(ContentBlock::ToolUse {
                id: id.clone(),
                name: call.function.name.clone(),
                input: input.clone(),
                provider_metadata: None,
            });
            tool_calls.push(ToolCall {
                id: id.clone(),
                name: call.function.name.clone(),
                input: input.clone(),
            });
            let _ = tx
                .send(StreamEvent::ToolUseEnd {
                    id,
                    name: call.function.name,
                    input,
                })
                .await;
        }

        let stop_reason = map_done_reason(done_reason.as_deref(), !tool_calls.is_empty());
        let usage = TokenUsage {
            input_tokens,
            output_tokens,
            cache_creation_input_tokens: 0,
            cache_read_input_tokens: 0,
        };

        debug!(
            chunks = chunk_count,
            text_len = content_blocks
                .iter()
                .filter_map(|b| match b {
                    ContentBlock::Text { text, .. } => Some(text.len()),
                    _ => None,
                })
                .sum::<usize>(),
            tool_count = tool_calls.len(),
            input_tokens,
            output_tokens,
            done_reason = ?done_reason,
            "Ollama stream completed"
        );

        let _ = tx
            .send(StreamEvent::ContentComplete { stop_reason, usage })
            .await;

        Ok(CompletionResponse {
            content: content_blocks,
            stop_reason,
            tool_calls,
            usage,
        })
    }

    fn family(&self) -> LlmFamily {
        LlmFamily::Local
    }
}

/// Map a non-success HTTP response from Ollama (or an Ollama-protocol
/// server like Lemonade) to the cross-driver `LlmError` taxonomy.
///
/// Ollama returns errors as `{"error": "<message>"}` (string, not
/// envelope), so we extract that payload when present and fall back to
/// the raw body otherwise.
fn map_http_error(status: u16, body: &str) -> LlmError {
    let extracted = serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|v| v.get("error").and_then(|e| e.as_str()).map(str::to_string))
        .unwrap_or_else(|| body.to_string());
    let message = if extracted.is_empty() {
        format!("HTTP {status}")
    } else {
        extracted
    };

    match status {
        404 => LlmError::ModelNotFound(message),
        401 | 403 => LlmError::AuthenticationFailed(message),
        429 => LlmError::RateLimited {
            retry_after_ms: 0,
            message: Some(message),
        },
        _ => LlmError::Api {
            status,
            message,
            code: None,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use librefang_types::config::ThinkingConfig;
    use librefang_types::message::{Message, Role};
    use librefang_types::tool::ToolDefinition;

    fn req(model: &str) -> CompletionRequest {
        CompletionRequest {
            model: model.to_string(),
            messages: std::sync::Arc::new(vec![Message::user("hello")]),
            tools: std::sync::Arc::new(Vec::new()),
            max_tokens: 256,
            temperature: 0.7,
            system: None,
            thinking: None,
            prompt_caching: false,
            cache_ttl: None,
            response_format: None,
            timeout_secs: None,
            extra_body: None,
            agent_id: None,
            session_id: None,
            step_id: None,
        }
    }

    #[test]
    fn sanitize_strips_trailing_v1_and_slashes() {
        assert_eq!(sanitize_base_url("http://x:11434/v1/"), "http://x:11434");
        assert_eq!(sanitize_base_url("http://x:11434/v1"), "http://x:11434");
        assert_eq!(sanitize_base_url("http://x:11434"), "http://x:11434");
        assert_eq!(sanitize_base_url("http://x:11434/"), "http://x:11434");
        assert_eq!(sanitize_base_url("http://x:11434/v1///"), "http://x:11434");
        // IPv6 literal — bracketed authority must still be detected as
        // "no path beyond /v1" so the strip applies.
        assert_eq!(
            sanitize_base_url("http://[::1]:11434/v1"),
            "http://[::1]:11434"
        );
    }

    /// Reverse-proxy mounts where `/v1` is a *suffix* of a custom path
    /// (not the entire path) are left alone — the user clearly meant a
    /// non-standard mount point, and stripping would either compose
    /// `…/openai/api/chat` (still wrong) or mask a misconfiguration.
    #[test]
    fn sanitize_preserves_reverse_proxy_paths() {
        assert_eq!(
            sanitize_base_url("http://api.corp.com/openai/v1"),
            "http://api.corp.com/openai/v1",
        );
        assert_eq!(
            sanitize_base_url("http://api.corp.com/api/v1"),
            "http://api.corp.com/api/v1",
        );
        assert_eq!(
            sanitize_base_url("https://gateway.example.com/v1-proxy/v1/"),
            "https://gateway.example.com/v1-proxy/v1",
        );
    }

    #[test]
    fn driver_strips_legacy_v1_in_base_url() {
        // #4810 migration: existing user configs may still pin /v1.
        // The driver auto-strips so the native /api/chat URL composes
        // correctly without breaking the deployment on upgrade.
        let driver = OllamaDriver::new(String::new(), "http://127.0.0.1:11434/v1".to_string());
        assert_eq!(driver.base_url, "http://127.0.0.1:11434");
    }

    #[test]
    fn build_request_user_message_text_only() {
        let driver = OllamaDriver::new(String::new(), "http://127.0.0.1:11434".to_string());
        let wire = driver.build_request(&req("llama3.1:8b")).expect("build");
        assert_eq!(wire.model, "llama3.1:8b");
        assert_eq!(wire.messages.len(), 1);
        assert_eq!(wire.messages[0].role, "user");
        assert_eq!(wire.messages[0].content.as_deref(), Some("hello"));
        assert!(wire.messages[0].images.is_none());
        assert!(wire.tools.is_empty());
    }

    #[test]
    fn build_request_promotes_system_prompt_to_first_message() {
        let driver = OllamaDriver::new(String::new(), "http://x".to_string());
        let mut r = req("m");
        r.system = Some("be helpful".to_string());
        let wire = driver.build_request(&r).expect("build");
        assert_eq!(wire.messages.len(), 2);
        assert_eq!(wire.messages[0].role, "system");
        assert_eq!(wire.messages[0].content.as_deref(), Some("be helpful"));
    }

    #[test]
    fn build_request_think_field_true_when_thinking_enabled() {
        let driver = OllamaDriver::new(String::new(), "http://x".to_string());
        let mut r = req("qwen3:8b");
        r.thinking = Some(ThinkingConfig::default());
        let wire = driver.build_request(&r).expect("build");
        assert_eq!(wire.think, Some(true));
    }

    /// The `think` field is ALWAYS sent (not omitted) when thinking is
    /// disabled, mirroring the legacy OpenAI-shim contract. Reasoning
    /// models (qwen3, deepseek-r1, gpt-oss) default `think: true`
    /// upstream, so omitting the field would silently flip
    /// chain-of-thought on for users who never enabled the dashboard's
    /// deep-thinking toggle.
    #[test]
    fn build_request_think_field_explicit_false_when_thinking_unset() {
        let driver = OllamaDriver::new(String::new(), "http://x".to_string());
        let wire = driver.build_request(&req("m")).expect("build");
        assert_eq!(wire.think, Some(false));
    }

    #[test]
    fn build_request_serializes_tool_definitions_as_function_envelope() {
        let driver = OllamaDriver::new(String::new(), "http://x".to_string());
        let mut r = req("m");
        r.tools = std::sync::Arc::new(vec![ToolDefinition {
            name: "get_weather".to_string(),
            description: "weather".to_string(),
            input_schema: serde_json::json!({"type":"object","properties":{}}),
        }]);
        let wire = driver.build_request(&r).expect("build");
        let serialized = serde_json::to_value(&wire).unwrap();
        assert_eq!(serialized["tools"][0]["type"], "function");
        assert_eq!(serialized["tools"][0]["function"]["name"], "get_weather");
    }

    #[test]
    fn build_request_user_blocks_with_image_attaches_base64_to_message() {
        let driver = OllamaDriver::new(String::new(), "http://x".to_string());
        let mut r = req("m");
        r.messages = std::sync::Arc::new(vec![Message {
            role: Role::User,
            content: MessageContent::Blocks(vec![
                ContentBlock::Text {
                    text: "what's this?".to_string(),
                    provider_metadata: None,
                },
                ContentBlock::Image {
                    media_type: "image/png".to_string(),
                    data: "AAAA".to_string(),
                },
            ]),
            pinned: false,
            timestamp: None,
        }]);
        let wire = driver.build_request(&r).expect("build");
        let user = wire
            .messages
            .iter()
            .find(|m| m.role == "user")
            .expect("user message");
        assert_eq!(user.content.as_deref(), Some("what's this?"));
        let images = user.images.as_ref().expect("images present");
        assert_eq!(images, &vec!["AAAA".to_string()]);
    }

    #[test]
    fn build_request_assistant_blocks_round_trip_thinking_and_tool_calls() {
        let driver = OllamaDriver::new(String::new(), "http://x".to_string());
        let mut r = req("m");
        r.messages = std::sync::Arc::new(vec![
            Message::user("call a tool"),
            Message {
                role: Role::Assistant,
                content: MessageContent::Blocks(vec![
                    ContentBlock::Thinking {
                        thinking: "deciding which tool".to_string(),
                        provider_metadata: None,
                    },
                    ContentBlock::ToolUse {
                        id: "ollama-call-1".to_string(),
                        name: "get_weather".to_string(),
                        input: serde_json::json!({"city":"Paris"}),
                        provider_metadata: None,
                    },
                ]),
                pinned: false,
                timestamp: None,
            },
        ]);
        let wire = driver.build_request(&r).expect("build");
        let asst = wire
            .messages
            .iter()
            .find(|m| m.role == "assistant")
            .expect("assistant message");
        assert_eq!(asst.thinking.as_deref(), Some("deciding which tool"));
        let tool_calls = asst.tool_calls.as_ref().expect("tool_calls present");
        assert_eq!(tool_calls.len(), 1);
        assert_eq!(tool_calls[0].function.name, "get_weather");
        assert_eq!(tool_calls[0].function.arguments["city"], "Paris");
    }

    #[test]
    fn build_request_tool_result_emits_role_tool_with_tool_name() {
        let driver = OllamaDriver::new(String::new(), "http://x".to_string());
        let mut r = req("m");
        r.messages = std::sync::Arc::new(vec![Message {
            role: Role::User,
            content: MessageContent::Blocks(vec![ContentBlock::ToolResult {
                tool_use_id: "ollama-call-1".to_string(),
                tool_name: "get_weather".to_string(),
                content: "sunny, 72F".to_string(),
                is_error: false,
                status: Default::default(),
                approval_request_id: None,
            }]),
            pinned: false,
            timestamp: None,
        }]);
        let wire = driver.build_request(&r).expect("build");
        let tool = wire
            .messages
            .iter()
            .find(|m| m.role == "tool")
            .expect("tool message");
        assert_eq!(tool.tool_name.as_deref(), Some("get_weather"));
        assert_eq!(tool.content.as_deref(), Some("sunny, 72F"));
    }

    /// A system message that arrives as `MessageContent::Blocks` is
    /// flattened to text before going on the wire — Ollama's `system`
    /// role only accepts string content, and we want to surface (rather
    /// than silently drop) the rare session-repair shape.
    #[test]
    fn build_request_system_blocks_flatten_to_text_system() {
        let driver = OllamaDriver::new(String::new(), "http://x".to_string());
        let mut r = req("m");
        r.system = None;
        r.messages = std::sync::Arc::new(vec![
            Message {
                role: Role::System,
                content: MessageContent::Blocks(vec![
                    ContentBlock::Text {
                        text: "be terse".to_string(),
                        provider_metadata: None,
                    },
                    ContentBlock::Text {
                        text: "and accurate".to_string(),
                        provider_metadata: None,
                    },
                ]),
                pinned: false,
                timestamp: None,
            },
            Message::user("hello"),
        ]);
        let wire = driver.build_request(&r).expect("build");
        let sys = wire
            .messages
            .iter()
            .find(|m| m.role == "system")
            .expect("system message");
        assert_eq!(sys.content.as_deref(), Some("be terse\n\nand accurate"));
    }

    #[test]
    fn build_request_empty_messages_errors() {
        let driver = OllamaDriver::new(String::new(), "http://x".to_string());
        let mut r = req("m");
        r.messages = std::sync::Arc::new(vec![]);
        let err = driver.build_request(&r).err().expect("error");
        match err {
            LlmError::Api { message, .. } => assert!(message.contains("no messages")),
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn map_done_reason_handles_known_values() {
        assert_eq!(map_done_reason(Some("stop"), false), StopReason::EndTurn);
        assert_eq!(map_done_reason(Some("stop"), true), StopReason::ToolUse);
        assert_eq!(
            map_done_reason(Some("length"), false),
            StopReason::MaxTokens
        );
        assert_eq!(
            map_done_reason(Some("tool_calls"), false),
            StopReason::ToolUse
        );
        assert_eq!(map_done_reason(None, false), StopReason::EndTurn);
        assert_eq!(map_done_reason(None, true), StopReason::ToolUse);
        assert_eq!(map_done_reason(Some("weird"), false), StopReason::EndTurn);
    }

    #[test]
    fn coerce_tool_args_passes_through_objects() {
        let v = serde_json::json!({"a": 1});
        assert_eq!(coerce_tool_args(&v), v);
    }

    #[test]
    fn coerce_tool_args_parses_stringified_object() {
        let v = serde_json::Value::String("{\"a\":1}".to_string());
        assert_eq!(coerce_tool_args(&v), serde_json::json!({"a": 1}));
    }

    #[test]
    fn coerce_tool_args_wraps_non_object_string() {
        let v = serde_json::Value::String("not json".to_string());
        let out = coerce_tool_args(&v);
        assert!(out.is_object());
        assert!(out.get("raw_input").is_some());
    }

    #[test]
    fn ollama_response_format_maps_variants() {
        use librefang_types::config::ResponseFormat;
        assert_eq!(ollama_response_format(&ResponseFormat::Text), None);
        assert_eq!(
            ollama_response_format(&ResponseFormat::Json),
            Some(serde_json::Value::String("json".to_string()))
        );
        let schema = serde_json::json!({"type":"object","required":["x"]});
        assert_eq!(
            ollama_response_format(&ResponseFormat::JsonSchema {
                name: "ans".to_string(),
                schema: schema.clone(),
                strict: Some(true),
            }),
            Some(schema)
        );
    }

    #[test]
    fn map_http_error_404_is_model_not_found() {
        let err = map_http_error(
            404,
            r#"{"error":"model 'gemma' not found, try pulling it first"}"#,
        );
        assert!(matches!(err, LlmError::ModelNotFound(ref m) if m.contains("gemma")));
    }

    #[test]
    fn map_http_error_401_is_auth_failure() {
        let err = map_http_error(401, r#"{"error":"unauthorized"}"#);
        assert!(matches!(err, LlmError::AuthenticationFailed(_)));
    }

    #[test]
    fn map_http_error_500_is_api_error_with_message() {
        let err = map_http_error(500, r#"{"error":"runtime panic"}"#);
        match err {
            LlmError::Api {
                status, message, ..
            } => {
                assert_eq!(status, 500);
                assert!(message.contains("runtime panic"));
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn map_http_error_falls_back_to_raw_body_when_unstructured() {
        let err = map_http_error(502, "Bad Gateway");
        match err {
            LlmError::Api {
                status, message, ..
            } => {
                assert_eq!(status, 502);
                assert_eq!(message, "Bad Gateway");
            }
            other => panic!("unexpected: {other:?}"),
        }
    }
}
