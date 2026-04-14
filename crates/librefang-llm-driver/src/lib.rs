//! LLM driver trait and types.
//!
//! Abstracts over multiple LLM providers (Anthropic, OpenAI, Ollama, etc.).

use std::collections::HashMap;

use async_trait::async_trait;
use librefang_types::config::{AzureOpenAiConfig, ResponseFormat, VertexAiConfig};
use librefang_types::message::{ContentBlock, Message, StopReason, TokenUsage};
use librefang_types::tool::{ToolCall, ToolDefinition};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Error type for LLM driver operations.
#[derive(Error, Debug)]
pub enum LlmError {
    /// HTTP request failed.
    #[error("HTTP error: {0}")]
    Http(String),
    /// API returned an error.
    #[error("API error ({status}): {message}")]
    Api {
        /// HTTP status code.
        status: u16,
        /// Error message from the API.
        message: String,
    },
    /// Rate limited — should retry after delay.
    #[error("Rate limited, retry after {retry_after_ms}ms{}", message.as_deref().map(|m| format!(": {m}")).unwrap_or_default())]
    RateLimited {
        /// How long to wait before retrying.
        retry_after_ms: u64,
        /// Optional original message from the provider (e.g. "You've hit your limit · resets 10am (UTC)").
        message: Option<String>,
    },
    /// Response parsing failed.
    #[error("Parse error: {0}")]
    Parse(String),
    /// No API key configured.
    #[error("Missing API key: {0}")]
    MissingApiKey(String),
    /// Model overloaded.
    #[error("Model overloaded, retry after {retry_after_ms}ms")]
    Overloaded {
        /// How long to wait before retrying.
        retry_after_ms: u64,
    },
    /// Authentication failed (invalid/missing API key).
    #[error("Authentication failed: {0}")]
    AuthenticationFailed(String),
    /// Model not found.
    #[error("Model not found: {0}")]
    ModelNotFound(String),
    /// Subprocess timed out due to inactivity, but partial output was captured.
    #[error("Timed out after {inactivity_secs}s of inactivity (last: {last_activity}, {partial_text_len} chars partial output)")]
    TimedOut {
        inactivity_secs: u64,
        partial_text: String,
        partial_text_len: usize,
        /// Last known activity before the process stalled.
        last_activity: String,
    },
}

/// A request to an LLM for completion.
#[derive(Debug, Clone)]
pub struct CompletionRequest {
    /// Model identifier.
    pub model: String,
    /// Conversation messages.
    pub messages: Vec<Message>,
    /// Available tools the model can use.
    pub tools: Vec<ToolDefinition>,
    /// Maximum tokens to generate.
    pub max_tokens: u32,
    /// Sampling temperature.
    pub temperature: f32,
    /// System prompt (extracted from messages for APIs that need it separately).
    pub system: Option<String>,
    /// Extended thinking configuration (if supported by the model).
    pub thinking: Option<librefang_types::config::ThinkingConfig>,
    /// Enable prompt caching for providers that support it.
    ///
    /// - **Anthropic**: adds `cache_control: {"type": "ephemeral"}` to system
    ///   message blocks and the last user turn.
    /// - **OpenAI**: automatic prefix caching (no request changes needed, but
    ///   cached token counts are parsed from the response).
    pub prompt_caching: bool,
    /// Desired response format (structured output).
    ///
    /// When set, instructs the LLM to return output in the specified format.
    /// `None` preserves the default free-form text behaviour.
    pub response_format: Option<ResponseFormat>,
    /// Per-request timeout override (seconds).  When set, the CLI driver uses
    /// this instead of the global `message_timeout_secs`.  Allows the agent
    /// loop to grant longer timeouts for requests that involve browser tools.
    pub timeout_secs: Option<u64>,
    /// Provider-specific extension parameters merged directly into the
    /// top-level API request body.
    ///
    /// When keys conflict with standard parameters (temperature, max_tokens, etc.),
    /// values from `extra_body` take precedence (last-wins in JSON serialization).
    pub extra_body: Option<HashMap<String, serde_json::Value>>,
}

/// A response from an LLM completion.
#[derive(Debug, Clone)]
pub struct CompletionResponse {
    /// The content blocks in the response.
    pub content: Vec<ContentBlock>,
    /// Why the model stopped generating.
    pub stop_reason: StopReason,
    /// Tool calls extracted from the response.
    pub tool_calls: Vec<ToolCall>,
    /// Token usage statistics.
    pub usage: TokenUsage,
}

impl CompletionResponse {
    /// Extract text content from the response.
    pub fn text(&self) -> String {
        self.content
            .iter()
            .filter_map(|block| match block {
                ContentBlock::Text { text, .. } => Some(text.as_str()),
                ContentBlock::Thinking { .. } => None,
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("")
    }
}

/// Events emitted during streaming LLM completion.
#[derive(Debug, Clone)]
pub enum StreamEvent {
    /// Incremental text content.
    TextDelta { text: String },
    /// A tool use block has started.
    ToolUseStart { id: String, name: String },
    /// Incremental JSON input for an in-progress tool use.
    ToolInputDelta { text: String },
    /// A tool use block is complete with parsed input.
    ToolUseEnd {
        id: String,
        name: String,
        input: serde_json::Value,
    },
    /// Incremental thinking/reasoning text.
    ThinkingDelta { text: String },
    /// The entire response is complete.
    ContentComplete {
        stop_reason: StopReason,
        usage: TokenUsage,
    },
    /// Agent lifecycle phase change (for UX indicators).
    PhaseChange {
        phase: String,
        detail: Option<String>,
    },
    /// Tool execution completed with result (emitted by agent loop, not LLM driver).
    ToolExecutionResult {
        name: String,
        result_preview: String,
        is_error: bool,
    },
}

/// Trait for LLM drivers.
#[async_trait]
pub trait LlmDriver: Send + Sync {
    /// Send a completion request and get a response.
    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, LlmError>;

    /// Stream a completion request, sending incremental events to the channel.
    /// Returns the full response when complete. Default wraps `complete()`.
    async fn stream(
        &self,
        request: CompletionRequest,
        tx: tokio::sync::mpsc::Sender<StreamEvent>,
    ) -> Result<CompletionResponse, LlmError> {
        let response = self.complete(request).await?;
        let text = response.text();
        if !text.is_empty() {
            let _ = tx.send(StreamEvent::TextDelta { text }).await;
        }
        let _ = tx
            .send(StreamEvent::ContentComplete {
                stop_reason: response.stop_reason,
                usage: response.usage,
            })
            .await;
        Ok(response)
    }

    /// Whether this driver has a working provider configuration.
    /// Returns false only for StubDriver; all real drivers return true.
    fn is_configured(&self) -> bool {
        true
    }
}

/// Configuration for creating an LLM driver.
#[derive(Clone, Serialize, Deserialize)]
pub struct DriverConfig {
    /// Provider name.
    pub provider: String,
    /// API key.
    pub api_key: Option<String>,
    /// Base URL override.
    pub base_url: Option<String>,
    /// Provider-specific Vertex AI settings from `KernelConfig.vertex_ai`.
    #[serde(default)]
    pub vertex_ai: VertexAiConfig,
    /// Provider-specific Azure OpenAI settings from `KernelConfig.azure_openai`.
    #[serde(default)]
    pub azure_openai: AzureOpenAiConfig,
    /// Skip interactive permission prompts (Claude Code provider only).
    ///
    /// When `true`, adds `--dangerously-skip-permissions` to the spawned
    /// `claude` CLI.  Defaults to `true` because LibreFang runs as a daemon
    /// with no interactive terminal, so permission prompts would block
    /// indefinitely.  LibreFang's own capability / RBAC layer already
    /// restricts what agents can do, making this safe.
    #[serde(default = "default_skip_permissions")]
    pub skip_permissions: bool,
    /// Message timeout in seconds for CLI-based providers (e.g. Claude Code).
    /// Inactivity-based: the process is killed after this many seconds of
    /// silence on stdout, not wall-clock time.
    #[serde(default = "default_message_timeout_secs")]
    pub message_timeout_secs: u64,
}

fn default_skip_permissions() -> bool {
    true
}

fn default_message_timeout_secs() -> u64 {
    300
}

/// SECURITY: Custom Debug impl redacts the API key.
impl std::fmt::Debug for DriverConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DriverConfig")
            .field("provider", &self.provider)
            .field("api_key", &self.api_key.as_ref().map(|_| "<redacted>"))
            .field("base_url", &self.base_url)
            .field("vertex_ai.project_id", &self.vertex_ai.project_id)
            .field("vertex_ai.region", &self.vertex_ai.region)
            .field(
                "vertex_ai.credentials_path",
                &self
                    .vertex_ai
                    .credentials_path
                    .as_ref()
                    .map(|_| "<redacted>"),
            )
            .field("azure_openai.endpoint", &self.azure_openai.endpoint)
            .field("azure_openai.deployment", &self.azure_openai.deployment)
            .field("azure_openai.api_version", &self.azure_openai.api_version)
            .field("skip_permissions", &self.skip_permissions)
            .field("message_timeout_secs", &self.message_timeout_secs)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_completion_response_text() {
        let response = CompletionResponse {
            content: vec![
                ContentBlock::Text {
                    text: "Hello ".to_string(),
                    provider_metadata: None,
                },
                ContentBlock::Text {
                    text: "world!".to_string(),
                    provider_metadata: None,
                },
            ],
            stop_reason: StopReason::EndTurn,
            tool_calls: vec![],
            usage: TokenUsage::default(),
        };
        assert_eq!(response.text(), "Hello world!");
    }

    #[test]
    fn test_stream_event_clone() {
        let event = StreamEvent::TextDelta {
            text: "hello".to_string(),
        };
        let cloned = event.clone();
        assert!(matches!(cloned, StreamEvent::TextDelta { text } if text == "hello"));
    }

    #[test]
    fn test_stream_event_variants() {
        let events: Vec<StreamEvent> = vec![
            StreamEvent::TextDelta {
                text: "hi".to_string(),
            },
            StreamEvent::ToolUseStart {
                id: "t1".to_string(),
                name: "web_search".to_string(),
            },
            StreamEvent::ToolInputDelta {
                text: "{\"q".to_string(),
            },
            StreamEvent::ToolUseEnd {
                id: "t1".to_string(),
                name: "web_search".to_string(),
                input: serde_json::json!({"query": "rust"}),
            },
            StreamEvent::ContentComplete {
                stop_reason: StopReason::EndTurn,
                usage: TokenUsage {
                    input_tokens: 10,
                    output_tokens: 5,
                    ..Default::default()
                },
            },
        ];
        assert_eq!(events.len(), 5);
    }

    #[tokio::test]
    async fn test_default_stream_sends_events() {
        use tokio::sync::mpsc;

        struct FakeDriver;

        #[async_trait]
        impl LlmDriver for FakeDriver {
            async fn complete(
                &self,
                _request: CompletionRequest,
            ) -> Result<CompletionResponse, LlmError> {
                Ok(CompletionResponse {
                    content: vec![ContentBlock::Text {
                        text: "Hello!".to_string(),
                        provider_metadata: None,
                    }],
                    stop_reason: StopReason::EndTurn,
                    tool_calls: vec![],
                    usage: TokenUsage {
                        input_tokens: 5,
                        output_tokens: 3,
                        ..Default::default()
                    },
                })
            }
        }

        let driver = FakeDriver;
        let (tx, mut rx) = mpsc::channel(16);
        let request = CompletionRequest {
            model: "test".to_string(),
            messages: vec![],
            tools: vec![],
            max_tokens: 100,
            temperature: 0.0,
            system: None,
            thinking: None,
            prompt_caching: false,
            response_format: None,
            timeout_secs: None,
            extra_body: None,
        };

        let response = driver.stream(request, tx).await.unwrap();
        assert_eq!(response.text(), "Hello!");

        // Should receive TextDelta then ContentComplete
        let ev1 = rx.recv().await.unwrap();
        assert!(matches!(ev1, StreamEvent::TextDelta { text } if text == "Hello!"));

        let ev2 = rx.recv().await.unwrap();
        assert!(matches!(
            ev2,
            StreamEvent::ContentComplete {
                stop_reason: StopReason::EndTurn,
                ..
            }
        ));
    }
}

pub mod llm_errors;
