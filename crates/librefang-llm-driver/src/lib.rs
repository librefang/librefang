//! LLM driver trait and types.
//!
//! Abstracts over multiple LLM providers (Anthropic, OpenAI, Ollama, etc.).

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use librefang_types::config::{AzureOpenAiConfig, ResponseFormat, VertexAiConfig};
use librefang_types::message::{ContentBlock, Message, StopReason, TokenUsage};
use librefang_types::tool::{ToolCall, ToolDefinition};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Error type for LLM driver operations.
#[derive(Error, Debug)]
#[non_exhaustive]
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
        /// Typed provider error code parsed from the structured response body
        /// (e.g. `error.code = "rate_limit_exceeded"`). When present,
        /// [`LlmError::failover_reason`] classifies via this typed value
        /// instead of substring-matching the human-readable `message`. Drivers
        /// that have not been migrated to populate this field (or transport
        /// paths that never see a structured body) leave this `None` and fall
        /// back to status-code-only classification. See #3745.
        code: Option<crate::llm_errors::ProviderErrorCode>,
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
    ///
    /// `partial_text` is wrapped in `Option<Arc<str>>` so cloning the error
    /// (e.g. when stringifying through `LibreFangError::LlmDriver(e.to_string())`,
    /// matching for failover decisions, etc.) is an O(1) refcount bump rather
    /// than copying potentially-megabyte payloads. Most consumers only ever
    /// read `partial_text_len` (which is what `Display` references) and never
    /// touch the body; CLI driver callers that DO want to forward the partial
    /// to the user can still pattern-match the variant and clone cheaply. See
    /// #3552.
    #[error("Timed out after {inactivity_secs}s of inactivity (last: {last_activity}, {partial_text_len} chars partial output)")]
    TimedOut {
        inactivity_secs: u64,
        partial_text: Option<Arc<str>>,
        partial_text_len: usize,
        /// Last known activity before the process stalled.
        last_activity: String,
    },
}

impl LlmError {
    /// Classify this error into a [`crate::llm_errors::FailoverReason`] that
    /// drives provider-switching decisions in `FallbackChain`.
    ///
    /// Classification is purely structural (variant + embedded status/message)
    /// and therefore allocation-free and infallible.
    pub fn failover_reason(&self) -> crate::llm_errors::FailoverReason {
        use crate::llm_errors::{FailoverReason, ProviderErrorCode};
        match self {
            // Rate-limited: retry the same provider after a backoff.
            LlmError::RateLimited { retry_after_ms, .. } => {
                FailoverReason::RateLimit(if *retry_after_ms > 0 {
                    Some(*retry_after_ms)
                } else {
                    None
                })
            }

            // HTTP-level API error.
            //
            // When the driver populated `code`, classify by the typed enum —
            // exhaustive, locale-independent, and immune to provider rewording
            // (#3745). When `code` is `None`, fall back to status-code-only
            // classification (no substring matching of the human-readable
            // message). Drivers that need fine-grained behaviour from
            // ambiguous statuses (403, 404, 400) must populate `code`.
            LlmError::Api {
                status,
                code: Some(code),
                ..
            } => match code {
                ProviderErrorCode::RateLimit => FailoverReason::RateLimit(None),
                ProviderErrorCode::CreditExhausted => FailoverReason::CreditExhausted,
                ProviderErrorCode::ContextLengthExceeded => FailoverReason::ContextTooLong,
                ProviderErrorCode::ModelNotFound | ProviderErrorCode::ServerUnavailable => {
                    FailoverReason::ModelUnavailable
                }
                ProviderErrorCode::AuthError => FailoverReason::AuthError,
                ProviderErrorCode::ServerError | ProviderErrorCode::BadRequest => {
                    // Honour known unambiguous status hints even when the
                    // typed code is generic.
                    match status {
                        413 => FailoverReason::ContextTooLong,
                        _ => FailoverReason::HttpError,
                    }
                }
            },
            LlmError::Api {
                status, code: None, ..
            } => match status {
                429 => FailoverReason::RateLimit(None),
                401 => FailoverReason::AuthError,
                402 => FailoverReason::CreditExhausted,
                413 => FailoverReason::ContextTooLong,
                503 => FailoverReason::ModelUnavailable,
                // 400/403/404/500 without a typed `code` are ambiguous —
                // skip to the next provider rather than guessing from the
                // message text.
                _ => FailoverReason::HttpError,
            },

            // Inactivity / subprocess timeout maps to Timeout.
            LlmError::TimedOut { .. } => FailoverReason::Timeout,

            // Overloaded — transient capacity error, retry same provider with back-off.
            LlmError::Overloaded { retry_after_ms } => {
                FailoverReason::RateLimit(if *retry_after_ms > 0 {
                    Some(*retry_after_ms)
                } else {
                    None
                })
            }

            // ModelNotFound → ModelUnavailable (skip to next provider).
            LlmError::ModelNotFound(_) => FailoverReason::ModelUnavailable,

            // Auth failures and missing keys indicate a misconfigured provider
            // slot.  Classify as AuthError so FallbackChain can skip to the
            // next slot, which may have a valid key.
            LlmError::AuthenticationFailed(_) | LlmError::MissingApiKey(_) => {
                FailoverReason::AuthError
            }

            // Parse errors are opaque and not recoverable by switching providers.
            LlmError::Parse(_) => FailoverReason::Unknown,

            // HTTP transport errors (connection refused, TLS failure, etc.).
            // Distinct from Timeout (inactivity/subprocess) — these are network
            // layer failures before the API even responded.
            LlmError::Http(_) => FailoverReason::HttpError,
        }
    }
}

/// A request to an LLM for completion.
#[derive(Debug, Clone)]
pub struct CompletionRequest {
    /// Model identifier.
    pub model: String,
    /// Conversation messages.
    ///
    /// Wrapped in `Arc` so cloning the request (e.g. retry on rate-limit
    /// inside `call_with_retry`) only bumps a refcount instead of deep-copying
    /// 200-600 KB of message history every turn (#3766). All driver code
    /// reads through `&request.messages` / `request.messages.iter()`, both
    /// of which auto-deref through `Arc<Vec<_>>`.
    pub messages: std::sync::Arc<Vec<Message>>,
    /// Available tools the model can use.
    ///
    /// Wrapped in `Arc` so cloning the request (retry, fallback, etc.) only
    /// bumps a refcount instead of deep-copying the full tool definition list
    /// — and so the agent loop can share a single resolved tool snapshot
    /// across iterations without re-cloning every `ToolDefinition` per turn
    /// (#3586). All driver code reads through `&request.tools` /
    /// `request.tools.iter()`, both of which auto-deref through
    /// `Arc<Vec<_>>`.
    pub tools: std::sync::Arc<Vec<ToolDefinition>>,
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
    /// - **Anthropic**: adds `cache_control: {"type": "ephemeral"}` markers
    ///   on the system block, the last tool, and the trailing 2-3 messages
    ///   (system_and_3 rolling window — uses all 4 cache breakpoints).
    /// - **OpenAI**: automatic prefix caching (no request changes needed, but
    ///   cached token counts are parsed from the response).
    pub prompt_caching: bool,
    /// Cache TTL hint when [`Self::prompt_caching`] is enabled.
    ///
    /// - `None` (default) → 5-minute ephemeral cache (1.25x write multiplier).
    /// - `Some("1h")` → 1-hour cache; only honored by the Anthropic driver,
    ///   which auto-injects the `anthropic-beta: extended-cache-ttl-2025-04-11`
    ///   header. Other values are treated as 5m.
    ///
    /// Ignored by drivers that don't implement `cache_control` markers.
    pub cache_ttl: Option<&'static str>,
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
    /// Caller agent identity.
    ///
    /// When a CLI driver re-exposes LibreFang tools to the model through an
    /// MCP bridge (e.g. `claude-code`'s `--mcp-config`), the bridge has no
    /// implicit way to know which agent spawned the CLI. This field carries
    /// the owning agent's ID so the driver can forward it (as an HTTP
    /// header on the bridge connection) and the bridge can resolve the
    /// agent's workspace, tool allowlist, and skill allowlist from the
    /// registry. `None` for out-of-band callers (compaction, routing
    /// probes, tests) that have no agent identity to propagate.
    pub agent_id: Option<String>,
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

/// Phase name emitted via `StreamEvent::PhaseChange` to signal that the final
/// LLM text for the turn has been streamed and the agent loop is about to
/// enter post-processing (session save, proactive memory). Consumers use
/// this to unblock user input before the full response payload is ready.
pub const PHASE_RESPONSE_COMPLETE: &str = "response_complete";

/// Events emitted during streaming LLM completion.
#[derive(Debug, Clone)]
#[non_exhaustive]
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
    /// §A — Owner-side private notice produced by the `notify_owner` tool
    /// during a streaming turn. Emitted by the agent loop (not LLM drivers).
    /// Channel-bridge consumers route this to the owner's DM (e.g. WhatsApp
    /// gateway → OWNER_JID) instead of the source chat.
    OwnerNotice { text: String },
}

/// High-level grouping of LLM providers that share wire format and
/// policy-relevant behaviour (prompt-cache semantics, tool-schema style,
/// thinking-block handling, …).
///
/// This is intentionally coarser than `provider`/`api_format` — it exists so
/// that future cross-cutting policy code can be hung off a single dimension
/// without re-implementing the same logic per concrete driver. No policy
/// logic is attached to the variants in this PR; consumers should treat the
/// enum as opaque metadata until follow-up work introduces family-aware
/// hooks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum LlmFamily {
    /// Anthropic Claude family (direct API, Anthropic-compatible providers,
    /// Claude Code CLI).
    Anthropic,
    /// OpenAI Chat Completions wire format (OpenAI, Azure OpenAI, Groq,
    /// OpenRouter, DeepInfra, Together, Cerebras, …).
    OpenAi,
    /// Google Gemini family (Gemini API, Vertex AI Gemini, Gemini CLI).
    Google,
    /// Locally-hosted runtimes accessed via their own native protocol
    /// (Ollama, LM Studio, vLLM, sglang, llama.cpp). Drivers that proxy
    /// local servers via the OpenAI-compatible shim still report `OpenAi`.
    Local,
    /// Anything that does not fit the above (Cohere v2, Aider, custom
    /// CLIs, etc.). Default for drivers that have not opted into a family.
    Other,
}

impl std::fmt::Display for LlmFamily {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LlmFamily::Anthropic => write!(f, "anthropic"),
            LlmFamily::OpenAi => write!(f, "open_ai"),
            LlmFamily::Google => write!(f, "google"),
            LlmFamily::Local => write!(f, "local"),
            LlmFamily::Other => write!(f, "other"),
        }
    }
}

/// Trait for LLM drivers.
#[async_trait]
pub trait LlmDriver: Send + Sync {
    /// Send a completion request and get a response.
    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, LlmError>;

    /// Stream a completion request, sending incremental events to the channel.
    /// Returns the full response when complete. Default wraps `complete()`.
    ///
    /// #3543: propagate `tx.send` errors. When the receiver is dropped (client
    /// disconnect, abort, etc.) we treat it as cancellation and return an
    /// error so the caller stops driving more work.
    async fn stream(
        &self,
        request: CompletionRequest,
        tx: tokio::sync::mpsc::Sender<StreamEvent>,
    ) -> Result<CompletionResponse, LlmError> {
        let response = self.complete(request).await?;
        let text = response.text();
        if !text.is_empty() {
            tx.send(StreamEvent::TextDelta { text })
                .await
                .map_err(|_| LlmError::Http("stream receiver dropped".to_string()))?;
        }
        tx.send(StreamEvent::ContentComplete {
            stop_reason: response.stop_reason,
            usage: response.usage,
        })
        .await
        .map_err(|_| LlmError::Http("stream receiver dropped".to_string()))?;
        Ok(response)
    }

    /// Whether this driver has a working provider configuration.
    /// Returns false only for StubDriver; all real drivers return true.
    fn is_configured(&self) -> bool {
        true
    }

    /// The high-level family this driver belongs to.
    ///
    /// Defaults to [`LlmFamily::Other`] so that out-of-tree drivers continue
    /// to compile without modification. Concrete in-tree drivers override
    /// this to enable future family-level shared policy (prompt-cache
    /// replay, tool-schema normalisation, …) without per-driver duplication.
    fn family(&self) -> LlmFamily {
        LlmFamily::Other
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
    /// Optional MCP bridge configuration (Claude Code provider only).
    ///
    /// When set, the driver writes a temp `mcp_config.json` and passes
    /// `--mcp-config` to the spawned Claude CLI so the subprocess discovers
    /// LibreFang tools via the daemon's `/mcp` endpoint. See issue #2314.
    ///
    /// Not serialized: set only by the kernel when constructing drivers.
    #[serde(skip)]
    pub mcp_bridge: Option<McpBridgeConfig>,
    /// Per-provider proxy URL override.
    /// When set, the driver uses this proxy instead of the global proxy config.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proxy_url: Option<String>,
    /// Per-provider HTTP request timeout in seconds.
    ///
    /// When set, this overrides the HTTP client's default read timeout for LLM
    /// API requests. Useful for providers known to be slower (e.g. local models,
    /// long-context workloads). CLI-based providers use `message_timeout_secs`
    /// instead; this field only applies to HTTP API drivers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_timeout_secs: Option<u64>,
}

/// Configuration for bridging LibreFang tools into a CLI-based driver via MCP.
///
/// Kept in the base crate so `DriverConfig` can carry it without a circular
/// dependency on `librefang-llm-drivers`. The driver crate re-exports this
/// type under its own `claude_code` module for convenience.
#[derive(Debug, Clone, Default)]
pub struct McpBridgeConfig {
    /// Daemon base URL (e.g. `http://127.0.0.1:4545`). The MCP endpoint lives
    /// at `{base_url}/mcp`.
    pub base_url: String,
    /// Optional API key for the `X-API-Key` header. Empty disables the header
    /// (matches daemon "no auth configured" mode).
    pub api_key: Option<String>,
}

impl Default for DriverConfig {
    fn default() -> Self {
        Self {
            provider: String::new(),
            api_key: None,
            base_url: None,
            vertex_ai: VertexAiConfig::default(),
            azure_openai: AzureOpenAiConfig::default(),
            skip_permissions: default_skip_permissions(),
            message_timeout_secs: default_message_timeout_secs(),
            mcp_bridge: None,
            proxy_url: None,
            request_timeout_secs: None,
        }
    }
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
            .field("mcp_bridge", &self.mcp_bridge.as_ref().map(|b| &b.base_url))
            .field("proxy_url", &self.proxy_url.as_ref().map(|_| "<redacted>"))
            .field("request_timeout_secs", &self.request_timeout_secs)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // #3552: `LlmError::TimedOut.partial_text` is `Option<Arc<str>>` so that
    // cloning the variant (or the whole error) is an O(1) refcount bump.
    // Display still interpolates `partial_text_len` only — the body is opaque
    // to most consumers — and pattern-matching the variant must keep working
    // for the CLI-driver callers that DO want to forward the partial.
    #[test]
    fn test_timed_out_partial_text_is_arc_shared_and_display_unchanged() {
        let body: Arc<str> = Arc::from("hello world partial output");
        let err = LlmError::TimedOut {
            inactivity_secs: 30,
            partial_text: Some(Arc::clone(&body)),
            partial_text_len: body.len(),
            last_activity: "tool_use".to_string(),
        };

        // Display references only `inactivity_secs`, `last_activity`, and
        // `partial_text_len` — the body is intentionally not interpolated.
        assert_eq!(
            err.to_string(),
            format!(
                "Timed out after 30s of inactivity (last: tool_use, {} chars partial output)",
                body.len()
            )
        );

        // Pattern-match still exposes the partial for CLI callers that want it.
        match &err {
            LlmError::TimedOut { partial_text, .. } => {
                assert_eq!(partial_text.as_deref(), Some(body.as_ref()));
            }
            other => panic!("expected TimedOut, got {other:?}"),
        }

        // The `None` shape is also valid for callers that don't have a partial.
        let empty = LlmError::TimedOut {
            inactivity_secs: 5,
            partial_text: None,
            partial_text_len: 0,
            last_activity: "init".to_string(),
        };
        assert_eq!(
            empty.to_string(),
            "Timed out after 5s of inactivity (last: init, 0 chars partial output)"
        );

        // Failover classification is unaffected by the field-shape change.
        assert_eq!(err.failover_reason(), FailoverReason::Timeout);
        assert_eq!(empty.failover_reason(), FailoverReason::Timeout);
    }

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

    #[test]
    fn test_llm_family_serializes_to_snake_case() {
        assert_eq!(
            serde_json::to_string(&LlmFamily::Anthropic).unwrap(),
            "\"anthropic\""
        );
        assert_eq!(
            serde_json::to_string(&LlmFamily::OpenAi).unwrap(),
            "\"open_ai\""
        );
        assert_eq!(
            serde_json::to_string(&LlmFamily::Google).unwrap(),
            "\"google\""
        );
        assert_eq!(
            serde_json::to_string(&LlmFamily::Local).unwrap(),
            "\"local\""
        );
        assert_eq!(
            serde_json::to_string(&LlmFamily::Other).unwrap(),
            "\"other\""
        );
    }

    #[test]
    fn test_llm_family_deserializes_from_snake_case() {
        assert_eq!(
            serde_json::from_str::<LlmFamily>("\"anthropic\"").unwrap(),
            LlmFamily::Anthropic
        );
        assert_eq!(
            serde_json::from_str::<LlmFamily>("\"open_ai\"").unwrap(),
            LlmFamily::OpenAi
        );
        assert_eq!(
            serde_json::from_str::<LlmFamily>("\"google\"").unwrap(),
            LlmFamily::Google
        );
        assert_eq!(
            serde_json::from_str::<LlmFamily>("\"local\"").unwrap(),
            LlmFamily::Local
        );
        assert_eq!(
            serde_json::from_str::<LlmFamily>("\"other\"").unwrap(),
            LlmFamily::Other
        );
    }

    #[test]
    fn test_llm_family_display_matches_serde() {
        assert_eq!(LlmFamily::Anthropic.to_string(), "anthropic");
        assert_eq!(LlmFamily::OpenAi.to_string(), "open_ai");
        assert_eq!(LlmFamily::Google.to_string(), "google");
        assert_eq!(LlmFamily::Local.to_string(), "local");
        assert_eq!(LlmFamily::Other.to_string(), "other");
    }

    #[test]
    fn test_llm_driver_family_default_is_other() {
        struct BareDriver;

        #[async_trait]
        impl LlmDriver for BareDriver {
            async fn complete(
                &self,
                _request: CompletionRequest,
            ) -> Result<CompletionResponse, LlmError> {
                unreachable!("test does not call complete")
            }
        }

        assert_eq!(BareDriver.family(), LlmFamily::Other);
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
            messages: std::sync::Arc::new(vec![]),
            tools: std::sync::Arc::new(vec![]),
            max_tokens: 100,
            temperature: 0.0,
            system: None,
            thinking: None,
            prompt_caching: false,
            cache_ttl: None,
            response_format: None,
            timeout_secs: None,
            extra_body: None,
            agent_id: None,
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

    // #3543: dropping the receiver must surface as an error rather than being
    // silently swallowed, otherwise callers keep driving cancelled work.
    #[tokio::test]
    async fn test_default_stream_errors_when_receiver_dropped() {
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
                        text: "hi".to_string(),
                        provider_metadata: None,
                    }],
                    stop_reason: StopReason::EndTurn,
                    tool_calls: vec![],
                    usage: TokenUsage::default(),
                })
            }
        }

        let driver = FakeDriver;
        let (tx, rx) = mpsc::channel(1);
        drop(rx);
        let request = CompletionRequest {
            model: "test".to_string(),
            messages: std::sync::Arc::new(vec![]),
            tools: std::sync::Arc::new(vec![]),
            max_tokens: 1,
            temperature: 0.0,
            system: None,
            thinking: None,
            prompt_caching: false,
            cache_ttl: None,
            response_format: None,
            timeout_secs: None,
            extra_body: None,
            agent_id: None,
        };
        let err = driver.stream(request, tx).await.unwrap_err();
        assert!(
            matches!(err, LlmError::Http(ref m) if m.contains("receiver dropped")),
            "expected receiver-dropped error, got: {err:?}"
        );
    }
}

pub mod llm_errors;
pub use llm_errors::FailoverReason;
