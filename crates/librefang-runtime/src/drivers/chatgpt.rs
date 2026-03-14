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
use tracing::debug;
use zeroize::Zeroizing;

use crate::chatgpt_oauth::CHATGPT_BASE_URL;
use crate::llm_driver::{CompletionRequest, CompletionResponse, LlmError, StreamEvent};
use librefang_types::message::{ContentBlock, MessageContent, Role, StopReason, TokenUsage};
use serde::{Deserialize, Serialize};

/// How long a ChatGPT session token is valid (conservative estimate).
/// ChatGPT session tokens typically last ~2 weeks, but we refresh at 7 days.
const SESSION_TOKEN_TTL_SECS: u64 = 7 * 24 * 3600; // 7 days

/// Refresh buffer — refresh this many seconds before estimated expiry.
const REFRESH_BUFFER_SECS: u64 = 3600; // 1 hour

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
#[derive(Debug, Deserialize)]
struct ResponsesOutputItem {
    #[serde(rename = "type")]
    item_type: String,
    #[serde(default)]
    content: Option<Vec<ResponsesContentPart>>,
    // tool_calls would go here if we support them later
}

/// Content part within an output item.
#[derive(Debug, Deserialize)]
struct ResponsesContentPart {
    #[serde(rename = "type")]
    part_type: String,
    #[serde(default)]
    text: Option<String>,
}

/// Token usage from the Responses API.
#[derive(Debug, Deserialize)]
struct ResponsesUsage {
    #[serde(default)]
    input_tokens: u64,
    #[serde(default)]
    output_tokens: u64,
}

/// Top-level response from the Responses API.
#[derive(Debug, Deserialize)]
struct ResponsesApiResponse {
    #[serde(default)]
    output: Vec<ResponsesOutputItem>,
    #[serde(default)]
    usage: Option<ResponsesUsage>,
    #[serde(default)]
    error: Option<ResponsesError>,
    // status can be "completed", "failed", "incomplete"
    #[serde(default)]
    status: Option<String>,
}

/// Error object from the Responses API.
#[derive(Debug, Deserialize)]
struct ResponsesError {
    #[serde(default)]
    message: Option<String>,
    #[serde(default)]
    code: Option<String>,
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
}

impl ChatGptDriver {
    pub fn new(session_token: String, base_url: String) -> Self {
        Self {
            session_token: Zeroizing::new(session_token),
            base_url: if base_url.is_empty() {
                CHATGPT_BASE_URL.to_string()
            } else {
                base_url
            },
            token_cache: ChatGptTokenCache::new(),
            client: reqwest::Client::builder()
                .user_agent(crate::USER_AGENT)
                .build()
                .unwrap_or_default(),
        }
    }

    /// Get a valid session token, caching it with an estimated TTL.
    ///
    /// If the cached token is expired and a `CHATGPT_REFRESH_TOKEN` environment
    /// variable is set, attempts to refresh the access token automatically via
    /// the OAuth refresh endpoint before falling back to an error.
    fn ensure_token(&self) -> Result<CachedSessionToken, LlmError> {
        // Check cache first
        if let Some(cached) = self.token_cache.get() {
            return Ok(cached);
        }

        // Try refreshing via OAuth if a refresh token is available.
        if let Ok(refresh_tok) = std::env::var("CHATGPT_REFRESH_TOKEN") {
            if !refresh_tok.is_empty() {
                debug!("Access token expired; attempting OAuth refresh");
                let refresh_result = match tokio::runtime::Handle::try_current() {
                    Ok(handle) => std::thread::scope(|s| {
                        s.spawn(|| {
                            handle
                                .block_on(crate::chatgpt_oauth::refresh_access_token(&refresh_tok))
                        })
                        .join()
                        .unwrap_or_else(|_| Err("Refresh thread panicked".to_string()))
                    }),
                    Err(_) => {
                        let rt = tokio::runtime::Runtime::new().map_err(|e| {
                            LlmError::Http(format!(
                                "Failed to create runtime for token refresh: {e}"
                            ))
                        })?;
                        rt.block_on(crate::chatgpt_oauth::refresh_access_token(&refresh_tok))
                    }
                };

                match refresh_result {
                    Ok(auth) => {
                        let ttl = auth.expires_in.unwrap_or(SESSION_TOKEN_TTL_SECS);
                        let token = CachedSessionToken {
                            token: auth.access_token,
                            expires_at: Instant::now() + Duration::from_secs(ttl),
                        };
                        self.token_cache.set(token.clone());
                        return Ok(token);
                    }
                    Err(e) => {
                        debug!("OAuth refresh failed: {e}");
                        // Fall through to use the session token directly, or error.
                    }
                }
            }
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

    /// Convert a CompletionRequest (messages-based) to Responses API format.
    fn build_responses_request(request: &CompletionRequest) -> ResponsesApiRequest {
        let mut instructions: Option<String> = request.system.clone();
        let mut input_items = Vec::new();

        for msg in &request.messages {
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

    /// Parse SSE stream from Responses API and extract final response.
    async fn parse_sse_stream(resp: reqwest::Response) -> Result<ResponsesApiResponse, LlmError> {
        let body = resp
            .text()
            .await
            .map_err(|e| LlmError::Http(format!("Failed to read SSE response: {e}")))?;

        // Find the last `response.completed` event which has the full response
        // Handle both "data:" and "data: " formats for SSE compatibility
        let mut final_response: Option<ResponsesApiResponse> = None;
        for line in body.lines() {
            let data = match line.strip_prefix("data:") {
                Some(d) => d.trim_start(),
                None => continue,
            };
            // Parse each SSE data line looking for response.completed
            if let Ok(event) = serde_json::from_str::<serde_json::Value>(data) {
                if event.get("type").and_then(|t| t.as_str()) == Some("response.completed") {
                    if let Some(resp_obj) = event.get("response") {
                        if let Ok(parsed) =
                            serde_json::from_value::<ResponsesApiResponse>(resp_obj.clone())
                        {
                            final_response = Some(parsed);
                        }
                    }
                }
            }
        }

        final_response.ok_or_else(|| {
            LlmError::Parse("No response.completed event found in SSE stream".to_string())
        })
    }

    /// Parse SSE stream and forward deltas to the streaming channel.
    async fn parse_sse_stream_with_events(
        resp: reqwest::Response,
        tx: &tokio::sync::mpsc::Sender<StreamEvent>,
    ) -> Result<ResponsesApiResponse, LlmError> {
        let body = resp
            .text()
            .await
            .map_err(|e| LlmError::Http(format!("Failed to read SSE response: {e}")))?;

        let mut final_response: Option<ResponsesApiResponse> = None;
        for line in body.lines() {
            // Handle both "data:" and "data: " formats for SSE compatibility
            let data = match line.strip_prefix("data:") {
                Some(d) => d.trim_start(),
                None => continue,
            };
            if let Ok(event) = serde_json::from_str::<serde_json::Value>(data) {
                let event_type = event.get("type").and_then(|t| t.as_str()).unwrap_or("");
                match event_type {
                    "response.output_text.delta" => {
                        if let Some(delta) = event.get("delta").and_then(|d| d.as_str()) {
                            let _ = tx
                                .send(StreamEvent::TextDelta {
                                    text: delta.to_string(),
                                })
                                .await;
                        }
                    }
                    "response.completed" => {
                        if let Some(resp_obj) = event.get("response") {
                            if let Ok(parsed) =
                                serde_json::from_value::<ResponsesApiResponse>(resp_obj.clone())
                            {
                                final_response = Some(parsed);
                            }
                        }
                    }
                    _ => {}
                }
            }
        }

        final_response.ok_or_else(|| {
            LlmError::Parse("No response.completed event found in SSE stream".to_string())
        })
    }

    /// Parse a Responses API response into a CompletionResponse.
    fn parse_responses_response(
        resp: ResponsesApiResponse,
    ) -> Result<CompletionResponse, LlmError> {
        // Check for error
        if let Some(err) = resp.error {
            let msg = err
                .message
                .unwrap_or_else(|| err.code.unwrap_or_else(|| "unknown error".to_string()));
            return Err(LlmError::Api {
                status: 400,
                message: msg,
            });
        }

        // Check status
        if resp.status.as_deref() == Some("failed") {
            return Err(LlmError::Api {
                status: 500,
                message: "Responses API returned failed status".to_string(),
            });
        }

        // Extract text from output items
        let mut content_blocks = Vec::new();
        for item in &resp.output {
            if item.item_type == "message" {
                if let Some(ref parts) = item.content {
                    for part in parts {
                        if part.part_type == "output_text" || part.part_type == "text" {
                            if let Some(ref text) = part.text {
                                content_blocks.push(ContentBlock::Text {
                                    text: text.clone(),
                                    provider_metadata: None,
                                });
                            }
                        }
                    }
                }
            }
        }

        let usage = resp.usage.map_or(
            TokenUsage {
                input_tokens: 0,
                output_tokens: 0,
            },
            |u| TokenUsage {
                input_tokens: u.input_tokens,
                output_tokens: u.output_tokens,
            },
        );

        let stop_reason = match resp.status.as_deref() {
            Some("incomplete") => StopReason::MaxTokens,
            _ => StopReason::EndTurn,
        };

        Ok(CompletionResponse {
            content: content_blocks,
            stop_reason,
            tool_calls: Vec::new(),
            usage,
        })
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

#[async_trait::async_trait]
impl crate::llm_driver::LlmDriver for ChatGptDriver {
    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        let token = self.ensure_token()?;
        let api_request = Self::build_responses_request(&request);

        let base = self.base_url.trim_end_matches('/');
        let url = format!("{base}/codex/responses");

        debug!("ChatGPT Responses API POST {url}");

        let http_resp = self
            .client
            .post(&url)
            .bearer_auth(token.token.as_str())
            .json(&api_request)
            .send()
            .await
            .map_err(|e| LlmError::Http(e.to_string()))?;

        let status = http_resp.status();
        if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
            let body = http_resp.text().await.unwrap_or_default();
            return Err(LlmError::AuthenticationFailed(format!(
                "ChatGPT API auth failed ({status}): {body}. Run `librefang auth chatgpt` to re-authenticate."
            )));
        }

        if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            return Err(LlmError::RateLimited {
                retry_after_ms: 5000,
            });
        }

        if !status.is_success() {
            let body = http_resp.text().await.unwrap_or_default();
            return Err(LlmError::Api {
                status: status.as_u16(),
                message: body,
            });
        }

        // ChatGPT Codex endpoint always returns SSE stream
        let resp_body = Self::parse_sse_stream(http_resp).await?;
        Self::parse_responses_response(resp_body)
    }

    async fn stream(
        &self,
        request: CompletionRequest,
        tx: tokio::sync::mpsc::Sender<StreamEvent>,
    ) -> Result<CompletionResponse, LlmError> {
        let token = self.ensure_token()?;
        let api_request = Self::build_responses_request(&request);

        let base = self.base_url.trim_end_matches('/');
        let url = format!("{base}/codex/responses");

        debug!("ChatGPT Responses API SSE stream POST {url}");

        let http_resp = self
            .client
            .post(&url)
            .bearer_auth(token.token.as_str())
            .json(&api_request)
            .send()
            .await
            .map_err(|e| LlmError::Http(e.to_string()))?;

        let status = http_resp.status();
        if !status.is_success() {
            let body = http_resp.text().await.unwrap_or_default();
            return Err(LlmError::Api {
                status: status.as_u16(),
                message: body,
            });
        }

        let resp_body = Self::parse_sse_stream_with_events(http_resp, &tx).await?;
        let response = Self::parse_responses_response(resp_body)?;

        let _ = tx
            .send(StreamEvent::ContentComplete {
                stop_reason: response.stop_reason,
                usage: response.usage,
            })
            .await;

        Ok(response)
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
    fn test_build_responses_request_basic() {
        let req = CompletionRequest {
            model: "gpt-4o".to_string(),
            messages: vec![Message {
                role: Role::User,
                content: MessageContent::Text("Hello".to_string()),
            }],
            tools: Vec::new(),
            max_tokens: 1024,
            temperature: 0.7,
            system: Some("You are helpful.".to_string()),
            thinking: None,
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
            messages: vec![
                Message {
                    role: Role::System,
                    content: MessageContent::Text("System prompt.".to_string()),
                },
                Message {
                    role: Role::User,
                    content: MessageContent::Text("Hi".to_string()),
                },
            ],
            tools: Vec::new(),
            max_tokens: 0,
            temperature: 1.0,
            system: None,
            thinking: None,
        };
        let api_req = ChatGptDriver::build_responses_request(&req);
        assert_eq!(api_req.instructions.as_deref(), Some("System prompt."));
        assert_eq!(api_req.input.len(), 1);
        assert!(api_req.max_output_tokens.is_none());
        assert!(api_req.temperature.is_none());
    }

    #[test]
    fn test_parse_responses_response_ok() {
        let json = serde_json::json!({
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
        let resp: ResponsesApiResponse = serde_json::from_value(json).unwrap();
        let parsed = ChatGptDriver::parse_responses_response(resp).unwrap();
        assert_eq!(parsed.text(), "Hello world!");
        assert_eq!(parsed.usage.input_tokens, 10);
        assert_eq!(parsed.usage.output_tokens, 5);
        assert_eq!(parsed.stop_reason, StopReason::EndTurn);
    }

    #[test]
    fn test_parse_responses_response_error() {
        let json = serde_json::json!({
            "output": [],
            "error": {
                "message": "Something went wrong",
                "code": "server_error"
            }
        });
        let resp: ResponsesApiResponse = serde_json::from_value(json).unwrap();
        let result = ChatGptDriver::parse_responses_response(resp);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_responses_response_incomplete() {
        let json = serde_json::json!({
            "output": [{
                "type": "message",
                "content": [{
                    "type": "output_text",
                    "text": "partial"
                }]
            }],
            "status": "incomplete"
        });
        let resp: ResponsesApiResponse = serde_json::from_value(json).unwrap();
        let parsed = ChatGptDriver::parse_responses_response(resp).unwrap();
        assert_eq!(parsed.stop_reason, StopReason::MaxTokens);
    }
}
