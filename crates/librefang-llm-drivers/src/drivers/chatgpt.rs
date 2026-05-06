//! ChatGPT driver using the Responses API.
//!
//! Uses OAuth tokens (obtained via `librefang auth chatgpt`) to call the
//! ChatGPT backend Responses API. This is different from the standard
//! OpenAI `/v1/chat/completions` endpoint — OAuth tokens with
//! `api.connectors` scopes only work with the Responses API.
//!
//! Token lifecycle:
//! - Access token provided via env var `CHATGPT_SESSION_TOKEN` or browser auth flow
//! - Refresh token in `CHATGPT_REFRESH_TOKEN` used for automatic renewal
//! - Token is cached and reused until it expires

use std::sync::Mutex;
use std::time::{Duration, Instant};
use tracing::{debug, warn};
use zeroize::Zeroizing;

use crate::llm_driver::{CompletionRequest, CompletionResponse, LlmError, StreamEvent};
use futures::StreamExt;
use librefang_runtime_oauth::chatgpt_oauth::CHATGPT_BASE_URL;
use librefang_types::config::ResponseFormat;
use librefang_types::message::{ContentBlock, MessageContent, Role, StopReason, TokenUsage};
use librefang_types::tool::ToolCall;
#[cfg(test)]
use serde::Deserialize;
use serde::Serialize;

/// How long a ChatGPT session token is valid (conservative estimate).
/// ChatGPT session tokens typically last ~2 weeks, but we refresh at 7 days.
const SESSION_TOKEN_TTL_SECS: u64 = 7 * 24 * 3600; // 7 days

/// Refresh buffer — refresh this many seconds before estimated expiry.
const REFRESH_BUFFER_SECS: u64 = 3600; // 1 hour

/// Hard timeout for OAuth refresh requests after explicit auth failures.
const TOKEN_REFRESH_TIMEOUT_SECS: u64 = 15;

// ── Responses API request/response types ──────────────────────────────

/// A single input item for the Responses API.
#[derive(Debug, Clone, Serialize)]
struct ResponsesInputItem {
    role: String,
    content: String,
}

/// Request body for `POST /codex/responses`.
#[derive(Debug, Serialize)]
struct ResponsesApiRequest {
    model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    instructions: Option<String>,
    input: Vec<ResponsesInputItem>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_output_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    store: bool,
    /// ChatGPT Codex endpoint requires stream=true.
    stream: bool,
}

/// A single output item in the Responses API response.
#[cfg(test)]
#[allow(dead_code)]
#[derive(Debug, Deserialize)]
struct ResponsesOutputItem {
    #[serde(rename = "type")]
    item_type: String,
    #[serde(default)]
    content: Option<Vec<ResponsesContentPart>>,
    // Fields for function_call output items
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    call_id: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    arguments: Option<String>,
    // Reasoning summary items
    #[serde(default)]
    summary: Option<Vec<ResponsesReasoningSummary>>,
}

/// Content part within an output item.
#[cfg(test)]
#[allow(dead_code)]
#[derive(Debug, Deserialize)]
struct ResponsesContentPart {
    #[serde(rename = "type")]
    part_type: String,
    #[serde(default)]
    text: Option<String>,
}

/// Reasoning summary text entry.
#[cfg(test)]
#[allow(dead_code)]
#[derive(Debug, Deserialize)]
struct ResponsesReasoningSummary {
    #[serde(rename = "type")]
    #[serde(default)]
    summary_type: Option<String>,
    #[serde(default)]
    text: Option<String>,
}

// ── Token cache ───────────────────────────────────────────────────────

/// Cached ChatGPT session token with estimated expiry.
#[derive(Clone)]
pub struct CachedSessionToken {
    /// The bearer token (zeroized on drop).
    pub token: Zeroizing<String>,
    /// Estimated expiry time.
    pub expires_at: Instant,
}

impl CachedSessionToken {
    /// Check if the token is still considered valid (with refresh buffer).
    pub fn is_valid(&self) -> bool {
        self.expires_at > Instant::now() + Duration::from_secs(REFRESH_BUFFER_SECS)
    }
}

/// Thread-safe token cache for a ChatGPT session.
pub struct ChatGptTokenCache {
    cached: Mutex<Option<CachedSessionToken>>,
}

impl ChatGptTokenCache {
    pub fn new() -> Self {
        Self {
            cached: Mutex::new(None),
        }
    }

    /// Get a valid cached token, or None if expired/missing.
    pub fn get(&self) -> Option<CachedSessionToken> {
        let lock = self.cached.lock().unwrap_or_else(|e| e.into_inner());
        lock.as_ref().filter(|t| t.is_valid()).cloned()
    }

    /// Store a new token in the cache.
    pub fn set(&self, token: CachedSessionToken) {
        let mut lock = self.cached.lock().unwrap_or_else(|e| e.into_inner());
        *lock = Some(token);
    }

    /// Drop a token that was just rejected by the API. Without this, the
    /// post-lock re-check in `refresh_token()` would observe the still-TTL-
    /// valid rejected token and return it as "fresh", causing the retry to
    /// 401 again (#3625). Concurrent callers all racing on the same rejected
    /// token converge on a single refresh: first invalidates → all see None
    /// after the lock → leader refreshes → followers pick up the new token.
    pub fn invalidate_if_matches(&self, rejected: &str) {
        let mut lock = self.cached.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(t) = lock.as_ref() {
            if t.token.as_str() == rejected {
                *lock = None;
            }
        }
    }
}

impl Default for ChatGptTokenCache {
    fn default() -> Self {
        Self::new()
    }
}

// ── Driver ────────────────────────────────────────────────────────────

/// LLM driver that calls the ChatGPT Responses API.
///
/// Instead of delegating to OpenAIDriver (which uses `/v1/chat/completions`),
/// this driver directly calls the Responses API which is compatible with
/// OAuth tokens having `api.connectors` scopes.
pub struct ChatGptDriver {
    /// The session token (provided at construction or via env).
    session_token: Zeroizing<String>,
    /// Base URL (defaults to `https://chatgpt.com/backend-api`).
    base_url: String,
    /// Token cache.
    token_cache: ChatGptTokenCache,
    /// HTTP client.
    client: reqwest::Client,
    /// Single-flight mutex for token refresh (issue #3625).
    ///
    /// OAuth 2.1 rotates the refresh token on every use: only the first
    /// concurrent refresh wins; the rest receive `invalid_grant`. By
    /// serialising refresh attempts here, concurrent 401 responders wait
    /// for the in-flight refresh and then re-check the cache — avoiding the
    /// thundering-herd that burns the refresh token.
    refresh_lock: tokio::sync::Mutex<()>,
    /// Whether to emit the three `x-librefang-{agent,session,step}-id` trace
    /// headers on outbound requests. Mirrors
    /// `KernelConfig.telemetry.emit_caller_trace_headers`; when `false`, no
    /// trace headers are emitted regardless of whether `CompletionRequest`'s
    /// caller-id fields are populated.
    emit_caller_trace_headers: bool,
}

impl ChatGptDriver {
    pub fn new(session_token: String, base_url: String) -> Self {
        Self::with_proxy(session_token, base_url, None)
    }

    pub fn with_proxy(session_token: String, base_url: String, proxy_url: Option<&str>) -> Self {
        let client = match proxy_url {
            Some(url) => librefang_http::proxied_client_with_override(url).unwrap_or_else(|e| {
                tracing::warn!(url, error = %e, "Invalid per-provider proxy URL, using global proxy");
                librefang_http::proxied_client()
            }),
            None => librefang_http::proxied_client(),
        };
        Self {
            session_token: Zeroizing::new(session_token),
            base_url: if base_url.is_empty() {
                CHATGPT_BASE_URL.to_string()
            } else {
                base_url
            },
            token_cache: ChatGptTokenCache::new(),
            client,
            refresh_lock: tokio::sync::Mutex::new(()),
            emit_caller_trace_headers: true,
        }
    }

    /// Override the trace-header emission flag (mirrors
    /// `KernelConfig.telemetry.emit_caller_trace_headers`). Default is `true`,
    /// meaning the three `x-librefang-{agent,session,step}-id` headers are
    /// emitted on every request that has those fields populated. Pass `false`
    /// to suppress them entirely. Non-trace `extra_headers` are unaffected.
    pub fn with_emit_caller_trace_headers(mut self, emit: bool) -> Self {
        self.emit_caller_trace_headers = emit;
        self
    }

    /// Get a valid session token, caching it with an estimated TTL.
    ///
    /// The session token produced by `librefang auth chatgpt` is treated as
    /// the primary bearer token. OAuth refresh is attempted later only if the
    /// API explicitly rejects that token with 401/403.
    fn ensure_token(&self) -> Result<CachedSessionToken, LlmError> {
        // Check cache first
        if let Some(cached) = self.token_cache.get() {
            return Ok(cached);
        }

        // Use the session token directly (it's a bearer token)
        if self.session_token.is_empty() {
            return Err(LlmError::MissingApiKey(
                "ChatGPT session token not set or expired. Run `librefang auth chatgpt` to re-authenticate"
                    .to_string(),
            ));
        }

        debug!("Caching ChatGPT session token");
        let token = CachedSessionToken {
            token: self.session_token.clone(),
            expires_at: Instant::now() + Duration::from_secs(SESSION_TOKEN_TTL_SECS),
        };

        self.token_cache.set(token.clone());
        Ok(token)
    }

    /// Refresh the access token after the API explicitly rejects the current one.
    ///
    /// Uses a `tokio::sync::Mutex` to ensure only one refresh runs at a time
    /// (issue #3625 — single-flight). After acquiring the lock we re-check the
    /// cache: a sibling waiter may have already completed the refresh while we
    /// were queued, so we can skip the network call and return the fresh token.
    async fn refresh_token(&self) -> Result<CachedSessionToken, LlmError> {
        // Acquire the single-flight lock first.
        let _lock = self.refresh_lock.lock().await;

        // Re-check cache after acquiring the lock — a previous waiter may
        // have already refreshed successfully while we were waiting.
        if let Some(cached) = self.token_cache.get() {
            debug!("ChatGPT token already refreshed by concurrent caller; reusing cached token");
            return Ok(cached);
        }

        let refresh_tok = std::env::var("CHATGPT_REFRESH_TOKEN").map_err(|_| {
            LlmError::AuthenticationFailed(
                "ChatGPT session token was rejected and no refresh token is available. Run `librefang auth chatgpt` to re-authenticate."
                    .to_string(),
            )
        })?;

        if refresh_tok.is_empty() {
            return Err(LlmError::AuthenticationFailed(
                "ChatGPT session token was rejected and the refresh token is empty. Run `librefang auth chatgpt` to re-authenticate."
                    .to_string(),
            ));
        }

        debug!("ChatGPT session token rejected; attempting OAuth refresh (single-flight)");
        let auth = tokio::time::timeout(
            Duration::from_secs(TOKEN_REFRESH_TIMEOUT_SECS),
            librefang_runtime_oauth::chatgpt_oauth::refresh_access_token(&refresh_tok),
        )
        .await
        .map_err(|_| {
            LlmError::Http(format!(
                "Timed out while refreshing ChatGPT access token after {}s",
                TOKEN_REFRESH_TIMEOUT_SECS
            ))
        })?
        .map_err(LlmError::Http)?;

        let ttl = auth.expires_in.unwrap_or(SESSION_TOKEN_TTL_SECS);
        let token = CachedSessionToken {
            token: auth.access_token,
            expires_at: Instant::now() + Duration::from_secs(ttl),
        };
        self.token_cache.set(token.clone());
        Ok(token)
    }

    async fn post_responses_request(
        &self,
        url: &str,
        api_request: &ResponsesApiRequest,
        bearer_token: &str,
        trace_headers: reqwest::header::HeaderMap,
    ) -> Result<reqwest::Response, LlmError> {
        self.client
            .post(url)
            .bearer_auth(bearer_token)
            .headers(trace_headers)
            .json(api_request)
            .send()
            .await
            .map_err(|e| LlmError::Http(e.to_string()))
    }

    async fn send_with_auth_retry(
        &self,
        url: &str,
        api_request: &ResponsesApiRequest,
        trace_headers: reqwest::header::HeaderMap,
    ) -> Result<reqwest::Response, LlmError> {
        let token = self.ensure_token()?;
        // Clone the header map for the potential retry path below.
        let headers_clone = trace_headers.clone();
        let http_resp = self
            .post_responses_request(url, api_request, token.token.as_str(), trace_headers)
            .await?;
        match http_resp.status() {
            reqwest::StatusCode::UNAUTHORIZED => {}
            reqwest::StatusCode::FORBIDDEN => {
                let body = http_resp.text().await.unwrap_or_default();
                if !should_refresh_after_forbidden(&body) {
                    return Err(LlmError::Api {
                        status: reqwest::StatusCode::FORBIDDEN.as_u16(),
                        message: body,
                        code: None,
                    });
                }
            }
            _ => return Ok(http_resp),
        }

        // Drop the just-rejected token so the post-lock cache re-check in
        // refresh_token() can't return it as "still valid" (#3625).
        self.token_cache.invalidate_if_matches(token.token.as_str());

        let refreshed = self.refresh_token().await?;
        let http_resp = self
            .post_responses_request(url, api_request, refreshed.token.as_str(), headers_clone)
            .await?;

        // Preserve post-refresh 403s so higher-level classification can
        // distinguish quota/model-access/region errors from real auth failures.
        if should_treat_post_refresh_status_as_auth_failure(http_resp.status()) {
            let status = http_resp.status();
            let body = http_resp.text().await.unwrap_or_default();
            return Err(LlmError::AuthenticationFailed(format!(
                "ChatGPT API auth failed after refresh ({status}): {body}. Run `librefang auth chatgpt` to re-authenticate."
            )));
        }

        Ok(http_resp)
    }

    /// Convert a CompletionRequest (messages-based) to Responses API format.
    fn build_responses_request(request: &CompletionRequest) -> ResponsesApiRequest {
        let mut instructions: Option<String> = request.system.clone();
        let mut input_items = Vec::new();

        for msg in request.messages.iter() {
            let role_str = match msg.role {
                Role::System => {
                    // Merge system messages into instructions
                    let text = extract_text_content(&msg.content);
                    if !text.is_empty() {
                        if let Some(ref mut instr) = instructions {
                            instr.push('\n');
                            instr.push_str(&text);
                        } else {
                            instructions = Some(text);
                        }
                    }
                    continue;
                }
                Role::User => "user",
                Role::Assistant => "assistant",
            };

            let text = extract_text_content(&msg.content);
            if !text.is_empty() {
                input_items.push(ResponsesInputItem {
                    role: role_str.to_string(),
                    content: text,
                });
            }
        }

        if let Some(rf) = &request.response_format {
            append_response_format_instructions(&mut instructions, rf);
        }

        ResponsesApiRequest {
            model: request.model.clone(),
            instructions,
            input: input_items,
            // ChatGPT Codex endpoint does not support max_output_tokens or temperature.
            max_output_tokens: None,
            temperature: None,
            store: false,
            stream: true,
        }
    }

    /// Parse SSE byte-stream from the Responses API using true incremental
    /// streaming.  When `tx` is `Some`, delta events are forwarded to the
    /// channel as they arrive; when `None` (the `complete()` path) only the
    /// final `CompletionResponse` is built.
    async fn stream_sse(
        resp: reqwest::Response,
        tx: Option<&tokio::sync::mpsc::Sender<StreamEvent>>,
    ) -> Result<CompletionResponse, LlmError> {
        let mut byte_stream = resp.bytes_stream();
        let mut line_buf = String::new();

        // Accumulated state
        let mut full_text = String::new();
        let mut thinking_text = String::new();
        let mut usage = TokenUsage::default();
        let mut stop_reason = StopReason::EndTurn;
        // tool_accum: (call_id, name, arguments, arguments_done_emitted) indexed by output_index
        let mut tool_accum: Vec<(String, String, String, bool)> = Vec::new();
        let mut completed_response: Option<serde_json::Value> = None;
        // Buffers partial UTF-8 codepoints across chunk boundaries (#3448).
        let mut utf8 = crate::utf8_stream::Utf8StreamDecoder::new();
        // Set when a `tx.send(...)` fails — abort upstream stream on next
        // iteration instead of fetching the rest for nobody (#3769).
        let mut receiver_dropped = false;

        while let Some(chunk) = byte_stream.next().await {
            if receiver_dropped {
                tracing::debug!("streaming receiver dropped; cancelling ChatGPT LLM stream");
                break;
            }
            let bytes = chunk.map_err(|e| LlmError::Http(format!("SSE stream error: {e}")))?;
            line_buf.push_str(&utf8.decode(&bytes));

            while let Some(newline_pos) = line_buf.find('\n') {
                let line = line_buf[..newline_pos].trim_end_matches('\r').to_string();
                line_buf = line_buf[newline_pos + 1..].to_string();

                let data = match line.strip_prefix("data:") {
                    Some(d) => d.trim_start(),
                    None => continue,
                };

                if data == "[DONE]" {
                    continue;
                }

                let event: serde_json::Value = match serde_json::from_str(data) {
                    Ok(v) => v,
                    Err(e) => {
                        warn!("ChatGPT SSE: failed to parse JSON event: {e}");
                        continue;
                    }
                };

                let event_type = event
                    .get("type")
                    .and_then(|t| t.as_str())
                    .unwrap_or_default();

                match event_type {
                    // ── Text deltas ──────────────────────────────────
                    "response.output_text.delta" => {
                        if let Some(delta) = event.get("delta").and_then(|d| d.as_str()) {
                            full_text.push_str(delta);
                            if let Some(tx) = tx {
                                if tx
                                    .send(StreamEvent::TextDelta {
                                        text: delta.to_string(),
                                    })
                                    .await
                                    .is_err()
                                {
                                    receiver_dropped = true;
                                }
                            }
                        }
                    }

                    // ── Reasoning / thinking deltas ──────────────────
                    "response.reasoning.delta" => {
                        if let Some(delta) = event.get("delta").and_then(|d| d.as_str()) {
                            thinking_text.push_str(delta);
                            if let Some(tx) = tx {
                                if tx
                                    .send(StreamEvent::ThinkingDelta {
                                        text: delta.to_string(),
                                    })
                                    .await
                                    .is_err()
                                {
                                    receiver_dropped = true;
                                }
                            }
                        }
                    }

                    // Reasoning summary text deltas (treated as thinking)
                    "response.reasoning_summary_text.delta" => {
                        if let Some(delta) = event.get("delta").and_then(|d| d.as_str()) {
                            thinking_text.push_str(delta);
                            if let Some(tx) = tx {
                                if tx
                                    .send(StreamEvent::ThinkingDelta {
                                        text: delta.to_string(),
                                    })
                                    .await
                                    .is_err()
                                {
                                    receiver_dropped = true;
                                }
                            }
                        }
                    }

                    // ── Tool / function call events ──────────────────
                    "response.output_item.added" => {
                        // A new output item appeared.  For function_call items
                        // we capture metadata (call_id, name).
                        if let Some(item) = event.get("item") {
                            let item_type = item.get("type").and_then(|t| t.as_str()).unwrap_or("");
                            if item_type == "function_call" {
                                let call_id = item
                                    .get("call_id")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or_default()
                                    .to_string();
                                let name = item
                                    .get("name")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or_default()
                                    .to_string();
                                let output_index = event
                                    .get("output_index")
                                    .and_then(|v| v.as_u64())
                                    .unwrap_or(tool_accum.len() as u64)
                                    as usize;

                                // Ensure tool_accum is large enough
                                while tool_accum.len() <= output_index {
                                    tool_accum.push((
                                        String::new(),
                                        String::new(),
                                        String::new(),
                                        false,
                                    ));
                                }
                                tool_accum[output_index] =
                                    (call_id.clone(), name.clone(), String::new(), false);

                                if let Some(tx) = tx {
                                    if tx
                                        .send(StreamEvent::ToolUseStart { id: call_id, name })
                                        .await
                                        .is_err()
                                    {
                                        receiver_dropped = true;
                                    }
                                }
                            }
                        }
                    }

                    "response.function_call_arguments.delta" => {
                        if let Some(delta) = event.get("delta").and_then(|d| d.as_str()) {
                            let output_index = event
                                .get("output_index")
                                .and_then(|v| v.as_u64())
                                .unwrap_or(0)
                                as usize;
                            if output_index < tool_accum.len() {
                                tool_accum[output_index].2.push_str(delta);
                            }
                            if let Some(tx) = tx {
                                if tx
                                    .send(StreamEvent::ToolInputDelta {
                                        text: delta.to_string(),
                                    })
                                    .await
                                    .is_err()
                                {
                                    receiver_dropped = true;
                                }
                            }
                        }
                    }

                    "response.function_call_arguments.done" => {
                        let output_index = event
                            .get("output_index")
                            .and_then(|v| v.as_u64())
                            .unwrap_or(0) as usize;
                        if output_index < tool_accum.len() {
                            tool_accum[output_index].3 = true;
                            let (ref id, ref name, ref args, _) = tool_accum[output_index];
                            let input: serde_json::Value = match serde_json::from_str(args) {
                                Ok(v) => v,
                                Err(e) => {
                                    warn!("ChatGPT SSE: failed to parse tool call arguments: {e}");
                                    serde_json::Value::Object(serde_json::Map::new())
                                }
                            };
                            if let Some(tx) = tx {
                                if tx
                                    .send(StreamEvent::ToolUseEnd {
                                        id: id.clone(),
                                        name: name.clone(),
                                        input: input.clone(),
                                    })
                                    .await
                                    .is_err()
                                {
                                    receiver_dropped = true;
                                }
                            }
                        }
                    }

                    "response.output_item.done" => {
                        // Function call items finalize here if arguments.done wasn't received
                        if let Some(item) = event.get("item") {
                            let item_type = item.get("type").and_then(|t| t.as_str()).unwrap_or("");
                            if item_type == "function_call" {
                                let output_index = event
                                    .get("output_index")
                                    .and_then(|v| v.as_u64())
                                    .unwrap_or(0)
                                    as usize;
                                // Only emit ToolUseEnd if arguments.done didn't already emit it
                                if output_index < tool_accum.len() && !tool_accum[output_index].3 {
                                    let (ref id, ref name, ref args, _) = tool_accum[output_index];
                                    if !id.is_empty() {
                                        let input: serde_json::Value = match serde_json::from_str(
                                            args,
                                        ) {
                                            Ok(v) => v,
                                            Err(e) => {
                                                warn!("ChatGPT SSE: failed to parse tool call arguments in output_item.done: {e}");
                                                serde_json::Value::Object(serde_json::Map::new())
                                            }
                                        };
                                        if let Some(tx) = tx {
                                            if tx
                                                .send(StreamEvent::ToolUseEnd {
                                                    id: id.clone(),
                                                    name: name.clone(),
                                                    input,
                                                })
                                                .await
                                                .is_err()
                                            {
                                                receiver_dropped = true;
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }

                    // ── Lifecycle events ─────────────────────────────
                    "response.completed" => {
                        if let Some(resp_obj) = event.get("response") {
                            // Extract usage from completed response
                            if let Some(u) = resp_obj.get("usage") {
                                usage = TokenUsage {
                                    input_tokens: u
                                        .get("input_tokens")
                                        .and_then(|v| v.as_u64())
                                        .unwrap_or(0),
                                    output_tokens: u
                                        .get("output_tokens")
                                        .and_then(|v| v.as_u64())
                                        .unwrap_or(0),
                                    ..Default::default()
                                };
                            }
                            // Extract stop reason from status. Responses API
                            // surfaces refusals via incomplete_details.reason
                            // — treat as ContentFiltered (#3450).
                            let status = resp_obj
                                .get("status")
                                .and_then(|s| s.as_str())
                                .unwrap_or("completed");
                            let incomplete_reason = resp_obj
                                .get("incomplete_details")
                                .and_then(|d| d.get("reason"))
                                .and_then(|r| r.as_str());
                            stop_reason = match (status, incomplete_reason) {
                                (_, Some("content_filter")) => StopReason::ContentFiltered,
                                ("incomplete", _) => StopReason::MaxTokens,
                                _ => StopReason::EndTurn,
                            };
                            completed_response = Some(resp_obj.clone());
                        }
                    }

                    "error" => {
                        let msg = event
                            .get("error")
                            .and_then(|e| e.get("message"))
                            .and_then(|m| m.as_str())
                            .unwrap_or("unknown error");
                        return Err(LlmError::Api {
                            status: 500,
                            message: msg.to_string(),
                            code: None,
                        });
                    }

                    _ => {
                        // Ignored: response.created, response.in_progress,
                        // response.output_text.done, etc.
                    }
                }
            }
        }

        // Drain any partial codepoint left in the decoder. No-op in
        // a well-formed stream; on a truncated connection the residue
        // surfaces as U+FFFD instead of vanishing (#3448).
        line_buf.push_str(&utf8.finish());

        // Build content blocks
        let mut content_blocks = Vec::new();

        // Add thinking block if we captured reasoning
        if !thinking_text.is_empty() {
            content_blocks.push(ContentBlock::Thinking {
                thinking: thinking_text,
                provider_metadata: None,
            });
        }

        // Add text block if we captured any text
        if !full_text.is_empty() {
            content_blocks.push(ContentBlock::Text {
                text: full_text,
                provider_metadata: None,
            });
        }

        // Build tool_calls from accumulated data
        let mut tool_calls: Vec<ToolCall> = Vec::new();
        for (call_id, name, args, _) in &tool_accum {
            if !call_id.is_empty() {
                let input: serde_json::Value = match serde_json::from_str(args) {
                    Ok(v) => v,
                    Err(e) => {
                        warn!("ChatGPT: failed to parse accumulated tool call arguments: {e}");
                        serde_json::Value::Object(serde_json::Map::new())
                    }
                };
                tool_calls.push(ToolCall {
                    id: call_id.clone(),
                    name: name.clone(),
                    input,
                });
            }
        }

        // If tool_calls are present, set stop_reason to ToolUse
        if !tool_calls.is_empty() {
            stop_reason = StopReason::ToolUse;
        }

        // Backfill: if we got a response.completed payload but missed
        // streaming deltas, fill in from the completed output array.
        if content_blocks.is_empty() && tool_calls.is_empty() {
            if let Some(ref resp_val) = completed_response {
                Self::backfill_from_completed_output(
                    resp_val,
                    &mut content_blocks,
                    &mut tool_calls,
                    &mut stop_reason,
                );
            }
        }

        // If still nothing, that's an error
        if content_blocks.is_empty() && tool_calls.is_empty() && completed_response.is_none() {
            return Err(LlmError::Parse(
                "No response.completed event found in SSE stream".to_string(),
            ));
        }

        Ok(CompletionResponse {
            content: content_blocks,
            stop_reason,
            tool_calls,
            usage,
        })
    }

    /// Backfill content and tool calls from a `response.completed` output
    /// array.  This is a fallback when streaming deltas were missed.
    fn backfill_from_completed_output(
        resp_val: &serde_json::Value,
        content_blocks: &mut Vec<ContentBlock>,
        tool_calls: &mut Vec<ToolCall>,
        stop_reason: &mut StopReason,
    ) {
        let output = match resp_val.get("output").and_then(|o| o.as_array()) {
            Some(arr) => arr,
            None => return,
        };

        for item in output {
            let item_type = item
                .get("type")
                .and_then(|t| t.as_str())
                .unwrap_or_default();

            match item_type {
                "message" => {
                    if let Some(parts) = item.get("content").and_then(|c| c.as_array()) {
                        for part in parts {
                            let part_type = part
                                .get("type")
                                .and_then(|t| t.as_str())
                                .unwrap_or_default();
                            if part_type == "output_text" || part_type == "text" {
                                if let Some(text) = part.get("text").and_then(|t| t.as_str()) {
                                    if !text.is_empty() {
                                        content_blocks.push(ContentBlock::Text {
                                            text: text.to_string(),
                                            provider_metadata: None,
                                        });
                                    }
                                }
                            }
                        }
                    }
                    // Check for reasoning summaries
                    if let Some(summaries) = item.get("summary").and_then(|s| s.as_array()) {
                        for summary in summaries {
                            if let Some(text) = summary.get("text").and_then(|t| t.as_str()) {
                                if !text.is_empty() {
                                    content_blocks.push(ContentBlock::Thinking {
                                        thinking: text.to_string(),
                                        provider_metadata: None,
                                    });
                                }
                            }
                        }
                    }
                }
                "function_call" => {
                    let call_id = item
                        .get("call_id")
                        .and_then(|v| v.as_str())
                        .unwrap_or_default()
                        .to_string();
                    let name = item
                        .get("name")
                        .and_then(|v| v.as_str())
                        .unwrap_or_default()
                        .to_string();
                    let args = item
                        .get("arguments")
                        .and_then(|v| v.as_str())
                        .unwrap_or("{}");
                    let input: serde_json::Value = match serde_json::from_str(args) {
                        Ok(v) => v,
                        Err(e) => {
                            warn!("ChatGPT: failed to parse tool arguments in completed response: {e}");
                            serde_json::Value::Object(serde_json::Map::new())
                        }
                    };
                    if !call_id.is_empty() {
                        tool_calls.push(ToolCall {
                            id: call_id,
                            name,
                            input,
                        });
                    }
                }
                _ => {}
            }
        }

        if !tool_calls.is_empty() {
            *stop_reason = StopReason::ToolUse;
        }
    }
}

/// Extract plain text from a MessageContent.
fn extract_text_content(content: &MessageContent) -> String {
    match content {
        MessageContent::Text(t) => t.clone(),
        MessageContent::Blocks(blocks) => blocks
            .iter()
            .filter_map(|b| match b {
                ContentBlock::Text { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join(""),
    }
}

fn append_response_format_instructions(instructions: &mut Option<String>, rf: &ResponseFormat) {
    match rf {
        ResponseFormat::Text => {}
        ResponseFormat::Json => {
            let suffix = "\n\nIMPORTANT: You MUST respond with valid JSON only. \
                           Do not include any text outside the JSON object.";
            if let Some(existing) = instructions.as_mut() {
                existing.push_str(suffix);
            } else {
                *instructions = Some(suffix.trim_start().to_string());
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
            if let Some(existing) = instructions.as_mut() {
                existing.push_str(&suffix);
            } else {
                *instructions = Some(suffix.trim_start().to_string());
            }
        }
    }
}

fn should_treat_post_refresh_status_as_auth_failure(status: reqwest::StatusCode) -> bool {
    status == reqwest::StatusCode::UNAUTHORIZED
}

fn should_refresh_after_forbidden(body: &str) -> bool {
    crate::llm_errors::classify_error(body, Some(reqwest::StatusCode::FORBIDDEN.as_u16())).category
        == crate::llm_errors::LlmErrorCategory::Auth
}

#[async_trait::async_trait]
impl crate::llm_driver::LlmDriver for ChatGptDriver {
    #[tracing::instrument(
        name = "llm.complete",
        skip_all,
        fields(provider = "chatgpt", model = %request.model)
    )]
    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        let api_request = Self::build_responses_request(&request);

        let base = self.base_url.trim_end_matches('/');
        let url = format!("{base}/codex/responses");

        debug!("ChatGPT Responses API POST {url}");
        let http_resp = self
            .send_with_auth_retry(
                &url,
                &api_request,
                super::trace_headers::build_trace_header_map(
                    &[],
                    &request,
                    self.emit_caller_trace_headers,
                ),
            )
            .await?;
        let status = http_resp.status();

        if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            let retry_after_ms = http_resp
                .headers()
                .get("retry-after")
                .and_then(|v| v.to_str().ok())
                .and_then(|s| s.parse::<u64>().ok())
                .map(|secs| secs * 1000)
                .unwrap_or(5000);
            return Err(LlmError::RateLimited {
                retry_after_ms,
                message: None,
            });
        }

        if !status.is_success() {
            let body = http_resp.text().await.unwrap_or_default();
            return Err(LlmError::Api {
                status: status.as_u16(),
                message: body,
                code: None,
            });
        }

        // ChatGPT Codex endpoint always returns SSE stream — consume
        // without forwarding events.
        Self::stream_sse(http_resp, None).await
    }

    #[tracing::instrument(
        name = "llm.stream",
        skip_all,
        fields(provider = "chatgpt", model = %request.model)
    )]
    async fn stream(
        &self,
        request: CompletionRequest,
        tx: tokio::sync::mpsc::Sender<StreamEvent>,
    ) -> Result<CompletionResponse, LlmError> {
        let api_request = Self::build_responses_request(&request);

        let base = self.base_url.trim_end_matches('/');
        let url = format!("{base}/codex/responses");

        debug!("ChatGPT Responses API SSE stream POST {url}");
        let http_resp = self
            .send_with_auth_retry(
                &url,
                &api_request,
                super::trace_headers::build_trace_header_map(
                    &[],
                    &request,
                    self.emit_caller_trace_headers,
                ),
            )
            .await?;
        let status = http_resp.status();

        if !status.is_success() {
            let body = http_resp.text().await.unwrap_or_default();
            return Err(LlmError::Api {
                status: status.as_u16(),
                message: body,
                code: None,
            });
        }

        let response = Self::stream_sse(http_resp, Some(&tx)).await?;

        let _ = tx
            .send(StreamEvent::ContentComplete {
                stop_reason: response.stop_reason,
                usage: response.usage,
            })
            .await;

        Ok(response)
    }

    fn family(&self) -> crate::llm_driver::LlmFamily {
        crate::llm_driver::LlmFamily::OpenAi
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use librefang_types::message::{Message, MessageContent, Role};

    #[test]
    fn test_token_cache_empty() {
        let cache = ChatGptTokenCache::new();
        assert!(cache.get().is_none());
    }

    #[test]
    fn test_token_cache_set_get() {
        let cache = ChatGptTokenCache::new();
        let token = CachedSessionToken {
            token: Zeroizing::new("test-session-token".to_string()),
            expires_at: Instant::now() + Duration::from_secs(86400),
        };
        cache.set(token);
        let cached = cache.get();
        assert!(cached.is_some());
        assert_eq!(*cached.unwrap().token, "test-session-token");
    }

    #[test]
    fn test_invalidate_if_matches_drops_only_matching_token() {
        let cache = ChatGptTokenCache::new();
        cache.set(CachedSessionToken {
            token: Zeroizing::new("rejected".to_string()),
            expires_at: Instant::now() + Duration::from_secs(86400),
        });

        // Different token in cache → no-op (e.g. another waiter already refreshed).
        cache.invalidate_if_matches("some-other-token");
        assert!(cache.get().is_some());

        // Matching token → cache cleared so the next refresh actually runs.
        cache.invalidate_if_matches("rejected");
        assert!(cache.get().is_none());
    }

    #[test]
    fn test_token_validity_check() {
        let valid = CachedSessionToken {
            token: Zeroizing::new("t".to_string()),
            expires_at: Instant::now() + Duration::from_secs(86400),
        };
        assert!(valid.is_valid());

        let almost_expired = CachedSessionToken {
            token: Zeroizing::new("t".to_string()),
            expires_at: Instant::now() + Duration::from_secs(60),
        };
        assert!(!almost_expired.is_valid());
    }

    #[test]
    fn test_chatgpt_driver_new_default_url() {
        let driver = ChatGptDriver::new("tok".to_string(), String::new());
        assert_eq!(driver.base_url, CHATGPT_BASE_URL);
    }

    #[test]
    fn test_chatgpt_driver_new_custom_url() {
        let driver = ChatGptDriver::new("tok".to_string(), "https://custom.api.com/v1".to_string());
        assert_eq!(driver.base_url, "https://custom.api.com/v1");
    }

    #[test]
    fn test_ensure_token_empty_errors() {
        let driver = ChatGptDriver::new(String::new(), String::new());
        let result = driver.ensure_token();
        assert!(result.is_err());
    }

    #[test]
    fn test_ensure_token_caches() {
        let driver = ChatGptDriver::new("my-token".to_string(), String::new());
        let tok1 = driver.ensure_token().unwrap();
        let tok2 = driver.ensure_token().unwrap();
        assert_eq!(*tok1.token, *tok2.token);
    }

    #[test]
    fn test_ensure_token_uses_session_token() {
        let driver = ChatGptDriver::new("my-token".to_string(), String::new());
        let token = driver.ensure_token().unwrap();
        assert_eq!(*token.token, "my-token".to_string());
    }

    #[test]
    fn test_post_refresh_only_401_is_auth_failure() {
        assert!(should_treat_post_refresh_status_as_auth_failure(
            reqwest::StatusCode::UNAUTHORIZED
        ));
        assert!(!should_treat_post_refresh_status_as_auth_failure(
            reqwest::StatusCode::FORBIDDEN
        ));
    }

    #[test]
    fn test_should_refresh_after_forbidden_for_auth_like_body() {
        assert!(should_refresh_after_forbidden(
            "Invalid API key or unauthorized access"
        ));
    }

    #[test]
    fn test_should_not_refresh_after_forbidden_for_model_or_quota_body() {
        assert!(!should_refresh_after_forbidden(
            "Model access is not enabled for your account"
        ));
        assert!(!should_refresh_after_forbidden(
            "Quota exceeded for this model"
        ));
    }

    #[test]
    fn test_build_responses_request_basic() {
        let req = CompletionRequest {
            model: "gpt-4o".to_string(),
            messages: std::sync::Arc::new(vec![Message {
                role: Role::User,
                content: MessageContent::Text("Hello".to_string()),
                pinned: false,
                timestamp: None,
            }]),
            tools: std::sync::Arc::new(Vec::new()),
            max_tokens: 1024,
            temperature: 0.7,
            system: Some("You are helpful.".to_string()),
            thinking: None,
            prompt_caching: false,
            cache_ttl: None,
            response_format: None,
            timeout_secs: None,
            extra_body: None,
            agent_id: None,
            session_id: None,
            step_id: None,
        };
        let api_req = ChatGptDriver::build_responses_request(&req);
        assert_eq!(api_req.model, "gpt-4o");
        assert_eq!(api_req.instructions.as_deref(), Some("You are helpful."));
        assert_eq!(api_req.input.len(), 1);
        assert_eq!(api_req.input[0].role, "user");
        assert_eq!(api_req.input[0].content, "Hello");
    }

    #[test]
    fn test_build_responses_request_system_merged() {
        let req = CompletionRequest {
            model: "gpt-4o".to_string(),
            messages: std::sync::Arc::new(vec![
                Message {
                    role: Role::System,
                    content: MessageContent::Text("System prompt.".to_string()),
                    pinned: false,
                    timestamp: None,
                },
                Message {
                    role: Role::User,
                    content: MessageContent::Text("Hi".to_string()),
                    pinned: false,
                    timestamp: None,
                },
            ]),
            tools: std::sync::Arc::new(Vec::new()),
            max_tokens: 0,
            temperature: 1.0,
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
        };
        let api_req = ChatGptDriver::build_responses_request(&req);
        assert_eq!(api_req.instructions.as_deref(), Some("System prompt."));
        assert_eq!(api_req.input.len(), 1);
        assert!(api_req.max_output_tokens.is_none());
        assert!(api_req.temperature.is_none());
    }

    #[test]
    fn test_build_responses_request_appends_json_response_format() {
        let req = CompletionRequest {
            model: "gpt-4o".to_string(),
            messages: std::sync::Arc::new(vec![Message {
                role: Role::User,
                content: MessageContent::Text("Hi".to_string()),
                pinned: false,
                timestamp: None,
            }]),
            tools: std::sync::Arc::new(Vec::new()),
            max_tokens: 0,
            temperature: 1.0,
            system: Some("System prompt.".to_string()),
            thinking: None,
            prompt_caching: false,
            cache_ttl: None,
            response_format: Some(ResponseFormat::Json),
            timeout_secs: None,
            extra_body: None,
            agent_id: None,
            session_id: None,
            step_id: None,
        };
        let api_req = ChatGptDriver::build_responses_request(&req);
        let instructions = api_req.instructions.expect("instructions");
        assert!(instructions.contains("System prompt."));
        assert!(instructions.contains("valid JSON only"));
    }

    #[test]
    fn test_build_responses_request_appends_json_schema_response_format() {
        let req = CompletionRequest {
            model: "gpt-4o".to_string(),
            messages: std::sync::Arc::new(vec![Message {
                role: Role::User,
                content: MessageContent::Text("Hi".to_string()),
                pinned: false,
                timestamp: None,
            }]),
            tools: std::sync::Arc::new(Vec::new()),
            max_tokens: 0,
            temperature: 1.0,
            system: None,
            thinking: None,
            prompt_caching: false,
            cache_ttl: None,
            response_format: Some(ResponseFormat::JsonSchema {
                name: "answer".to_string(),
                schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "ok": {"type": "boolean"}
                    },
                    "required": ["ok"]
                }),
                strict: Some(true),
            }),
            timeout_secs: None,
            extra_body: None,
            agent_id: None,
            session_id: None,
            step_id: None,
        };
        let api_req = ChatGptDriver::build_responses_request(&req);
        let instructions = api_req.instructions.expect("instructions");
        assert!(instructions.contains("name: \"answer\""));
        assert!(instructions.contains("\"required\":[\"ok\"]"));
    }

    #[test]
    fn test_backfill_text_from_completed_output() {
        let resp_val = serde_json::json!({
            "output": [{
                "type": "message",
                "content": [{
                    "type": "output_text",
                    "text": "Hello world!"
                }]
            }],
            "usage": {
                "input_tokens": 10,
                "output_tokens": 5
            },
            "status": "completed"
        });

        let mut content_blocks = Vec::new();
        let mut tool_calls = Vec::new();
        let mut stop_reason = StopReason::EndTurn;

        ChatGptDriver::backfill_from_completed_output(
            &resp_val,
            &mut content_blocks,
            &mut tool_calls,
            &mut stop_reason,
        );

        assert_eq!(content_blocks.len(), 1);
        assert!(
            matches!(&content_blocks[0], ContentBlock::Text { text, .. } if text == "Hello world!"),
            "Expected Text block with 'Hello world!', got {:?}",
            content_blocks[0]
        );
        assert!(tool_calls.is_empty());
        assert_eq!(stop_reason, StopReason::EndTurn);
    }

    #[test]
    fn test_backfill_tool_calls_from_completed_output() {
        let resp_val = serde_json::json!({
            "output": [{
                "type": "function_call",
                "call_id": "call_123",
                "name": "web_search",
                "arguments": "{\"query\": \"rust\"}"
            }],
            "status": "completed"
        });

        let mut content_blocks = Vec::new();
        let mut tool_calls = Vec::new();
        let mut stop_reason = StopReason::EndTurn;

        ChatGptDriver::backfill_from_completed_output(
            &resp_val,
            &mut content_blocks,
            &mut tool_calls,
            &mut stop_reason,
        );

        assert!(content_blocks.is_empty());
        assert_eq!(tool_calls.len(), 1);
        assert_eq!(tool_calls[0].id, "call_123");
        assert_eq!(tool_calls[0].name, "web_search");
        assert_eq!(tool_calls[0].input, serde_json::json!({"query": "rust"}));
        assert_eq!(stop_reason, StopReason::ToolUse);
    }

    #[test]
    fn test_backfill_reasoning_summary() {
        let resp_val = serde_json::json!({
            "output": [{
                "type": "message",
                "content": [{
                    "type": "output_text",
                    "text": "Answer"
                }],
                "summary": [{
                    "type": "summary_text",
                    "text": "I thought about this carefully."
                }]
            }],
            "status": "completed"
        });

        let mut content_blocks = Vec::new();
        let mut tool_calls = Vec::new();
        let mut stop_reason = StopReason::EndTurn;

        ChatGptDriver::backfill_from_completed_output(
            &resp_val,
            &mut content_blocks,
            &mut tool_calls,
            &mut stop_reason,
        );

        assert_eq!(content_blocks.len(), 2);
        assert!(
            matches!(&content_blocks[0], ContentBlock::Text { text, .. } if text == "Answer"),
            "Expected Text block with 'Answer', got {:?}",
            content_blocks[0]
        );
        assert!(
            matches!(&content_blocks[1], ContentBlock::Thinking { thinking, .. } if thinking == "I thought about this carefully."),
            "Expected Thinking block, got {:?}",
            content_blocks[1]
        );
    }

    #[test]
    fn test_responses_output_item_deserialize_function_call() {
        let json = serde_json::json!({
            "type": "function_call",
            "id": "fc_1",
            "call_id": "call_abc",
            "name": "get_weather",
            "arguments": "{\"city\": \"London\"}"
        });
        let item: ResponsesOutputItem = serde_json::from_value(json).unwrap();
        assert_eq!(item.item_type, "function_call");
        assert_eq!(item.call_id.as_deref(), Some("call_abc"));
        assert_eq!(item.name.as_deref(), Some("get_weather"));
        assert_eq!(item.arguments.as_deref(), Some("{\"city\": \"London\"}"));
    }

    #[test]
    fn test_responses_output_item_deserialize_message() {
        let json = serde_json::json!({
            "type": "message",
            "content": [{
                "type": "output_text",
                "text": "Hello!"
            }]
        });
        let item: ResponsesOutputItem = serde_json::from_value(json).unwrap();
        assert_eq!(item.item_type, "message");
        assert!(item.content.is_some());
        assert_eq!(item.content.unwrap()[0].text.as_deref(), Some("Hello!"));
    }
}
