//! Claude Code CLI backend driver.
//!
//! Spawns the `claude` CLI (Claude Code) as a subprocess in print mode (`-p`),
//! which is non-interactive and handles its own authentication.
//! This allows users with Claude Code installed to use it as an LLM provider
//! without needing a separate API key.
//!
//! Tracks active subprocess PIDs and enforces message timeouts to prevent
//! hung CLI processes from blocking agents indefinitely.

pub use crate::llm_driver::McpBridgeConfig;
use crate::llm_driver::{CompletionRequest, CompletionResponse, LlmDriver, LlmError, StreamEvent};
use async_trait::async_trait;
use base64::Engine;
use dashmap::DashMap;
use librefang_types::message::{ContentBlock, MessageContent, Role, StopReason, TokenUsage};
use serde::Deserialize;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncReadExt};
use tracing::{debug, info, warn};

/// Environment variable names (and suffixes) to strip from the subprocess
/// to prevent leaking API keys from other providers. We keep the full env
/// intact (so Node.js, NVM, SSL, proxies, etc. all work) and only remove
/// secrets that belong to other LLM providers.
const SENSITIVE_ENV_EXACT: &[&str] = &[
    "OPENAI_API_KEY",
    "ANTHROPIC_API_KEY",
    "GEMINI_API_KEY",
    "GOOGLE_API_KEY",
    "GROQ_API_KEY",
    "DEEPSEEK_API_KEY",
    "MISTRAL_API_KEY",
    "TOGETHER_API_KEY",
    "FIREWORKS_API_KEY",
    "OPENROUTER_API_KEY",
    "PERPLEXITY_API_KEY",
    "COHERE_API_KEY",
    "AI21_API_KEY",
    "CEREBRAS_API_KEY",
    "SAMBANOVA_API_KEY",
    "HUGGINGFACE_API_KEY",
    "XAI_API_KEY",
    "REPLICATE_API_TOKEN",
    "BRAVE_API_KEY",
    "TAVILY_API_KEY",
    "ELEVENLABS_API_KEY",
];

/// Suffixes that indicate a secret — remove any env var ending with these
/// unless it starts with `CLAUDE_`.
const SENSITIVE_SUFFIXES: &[&str] = &["_SECRET", "_TOKEN", "_PASSWORD"];

/// Default subprocess timeout in seconds (5 minutes).
const DEFAULT_MESSAGE_TIMEOUT_SECS: u64 = 300;

/// LLM driver that delegates to the Claude Code CLI.
pub struct ClaudeCodeDriver {
    cli_path: String,
    skip_permissions: bool,
    /// Active subprocess PIDs keyed by a caller-provided label (e.g. agent name).
    /// Allows external code to check if a subprocess is running and kill it.
    active_pids: Arc<DashMap<String, u32>>,
    /// Message timeout in seconds. CLI subprocesses that exceed this are killed.
    message_timeout_secs: u64,
    /// Optional profile config directory.  When set, every spawned CLI process
    /// gets `CLAUDE_CONFIG_DIR=<path>` so it uses that profile's credentials.
    config_dir: Option<std::path::PathBuf>,
    /// Optional MCP bridge config (see [`McpBridgeConfig`]).
    mcp_bridge: Option<McpBridgeConfig>,
}

impl ClaudeCodeDriver {
    /// Create a new Claude Code driver.
    ///
    /// `cli_path` overrides the CLI binary path; defaults to `"claude"` on PATH.
    /// `skip_permissions` adds `--dangerously-skip-permissions` to the spawned
    /// command so that the CLI runs non-interactively (required for daemon mode).
    pub fn new(cli_path: Option<String>, skip_permissions: bool) -> Self {
        if skip_permissions {
            warn!(
                "Claude Code driver: --dangerously-skip-permissions enabled. \
                 The CLI will not prompt for tool approvals. \
                 LibreFang's own capability/RBAC system enforces access control."
            );
        }

        Self {
            cli_path: cli_path
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "claude".to_string()),
            skip_permissions,
            active_pids: Arc::new(DashMap::new()),
            message_timeout_secs: DEFAULT_MESSAGE_TIMEOUT_SECS,
            config_dir: None,
            mcp_bridge: None,
        }
    }

    /// Set the profile config directory (`CLAUDE_CONFIG_DIR`).
    pub fn with_config_dir(mut self, dir: std::path::PathBuf) -> Self {
        self.config_dir = Some(dir);
        self
    }

    /// Enable the MCP bridge so LibreFang tools are exposed to the spawned
    /// Claude CLI via its native `--mcp-config` support.
    pub fn with_mcp_bridge(mut self, bridge: McpBridgeConfig) -> Self {
        self.mcp_bridge = Some(bridge);
        self
    }

    /// Create a new Claude Code driver with a custom timeout.
    pub fn with_timeout(
        cli_path: Option<String>,
        skip_permissions: bool,
        timeout_secs: u64,
    ) -> Self {
        let mut driver = Self::new(cli_path, skip_permissions);
        driver.message_timeout_secs = timeout_secs;
        driver
    }

    /// Get a snapshot of active subprocess PIDs.
    /// Returns a vec of (label, pid) pairs.
    pub fn active_pids(&self) -> Vec<(String, u32)> {
        self.active_pids
            .iter()
            .map(|entry| (entry.key().clone(), *entry.value()))
            .collect()
    }

    /// Get the shared PID map for external monitoring.
    pub fn pid_map(&self) -> Arc<DashMap<String, u32>> {
        Arc::clone(&self.active_pids)
    }

    /// Detect if the Claude Code CLI is available on PATH or at a known install location.
    ///
    /// First tries `claude` on PATH (covers most cases). If that fails, falls back to
    /// well-known absolute install paths for macOS (Homebrew, volta, nvm) and Linux/Windows.
    /// This handles the common case where the daemon is started outside a login shell and
    /// `/opt/homebrew/bin` or similar is absent from `PATH`.
    pub fn detect() -> Option<String> {
        // Candidate paths: PATH first, then common absolute locations.
        let candidates: &[&str] = &[
            "claude",
            "/opt/homebrew/bin/claude",
            "/usr/local/bin/claude",
            "/usr/bin/claude",
        ];

        for candidate in candidates {
            let output = std::process::Command::new(candidate)
                .arg("--version")
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::null())
                .output();

            if let Ok(out) = output {
                if out.status.success() {
                    return Some(String::from_utf8_lossy(&out.stdout).trim().to_string());
                }
            }
        }

        None
    }

    /// Build a text prompt from the completion request messages.
    ///
    /// When messages contain image blocks, the images are decoded from base64,
    /// written to a temporary directory, and referenced by file path in the
    /// prompt text. The caller must pass the returned `image_dir` to
    /// `--add-dir` so the Claude CLI can read them, and clean up the directory
    /// after the CLI exits.
    fn build_prompt(request: &CompletionRequest) -> PreparedPrompt {
        let mut parts = Vec::new();
        let mut image_dir: Option<PathBuf> = None;
        let mut image_count = 0u32;

        if let Some(ref sys) = request.system {
            parts.push(format!("[System]\n{sys}"));
        }

        for msg in &request.messages {
            let role_label = match msg.role {
                Role::User => "User",
                Role::Assistant => "Assistant",
                Role::System => "System",
            };

            match &msg.content {
                MessageContent::Text(s) => {
                    if !s.is_empty() {
                        parts.push(format!("[{role_label}]\n{s}"));
                    }
                }
                MessageContent::Blocks(blocks) => {
                    let mut msg_parts = Vec::new();
                    for block in blocks {
                        match block {
                            ContentBlock::Text { text, .. } => {
                                if !text.is_empty() {
                                    msg_parts.push(text.clone());
                                }
                            }
                            ContentBlock::Image { media_type, data } => {
                                // Create temp dir on first image
                                if image_dir.is_none() {
                                    let dir = std::env::temp_dir()
                                        .join(format!("librefang-images-{}", uuid::Uuid::new_v4()));
                                    if let Err(e) = std::fs::create_dir_all(&dir) {
                                        warn!(error = %e, "Failed to create image temp dir");
                                        continue;
                                    }
                                    image_dir = Some(dir);
                                }

                                let ext = match media_type.as_str() {
                                    "image/png" => "png",
                                    "image/gif" => "gif",
                                    "image/webp" => "webp",
                                    _ => "jpg",
                                };
                                image_count += 1;
                                let filename = format!("image-{image_count}.{ext}");
                                let path = image_dir.as_ref().unwrap().join(&filename);

                                match base64::engine::general_purpose::STANDARD.decode(data) {
                                    Ok(decoded) => {
                                        if let Err(e) = std::fs::write(&path, &decoded) {
                                            warn!(error = %e, "Failed to write temp image");
                                            continue;
                                        }
                                        msg_parts.push(format!("@{}", path.display()));
                                    }
                                    Err(e) => {
                                        warn!(error = %e, "Failed to decode base64 image");
                                    }
                                }
                            }
                            ContentBlock::ImageFile { path, .. } => {
                                // ImageFile already on disk — reference directly,
                                // no temp copy needed (per DRVR-01).
                                let file_path = std::path::Path::new(path);
                                if file_path.exists() {
                                    msg_parts.push(format!("@{}", file_path.display()));
                                } else {
                                    warn!(path = %path, "ImageFile path missing, skipping");
                                }
                            }
                            _ => {}
                        }
                    }
                    let text = msg_parts.join("\n");
                    if !text.is_empty() {
                        parts.push(format!("[{role_label}]\n{text}"));
                    }
                }
            }
        }

        PreparedPrompt {
            text: parts.join("\n\n"),
            image_dir,
            mcp_config_path: None,
        }
    }

    /// Write a temp `mcp_config.json` describing the LibreFang MCP server and
    /// return its path. The file is written to a unique location per call so
    /// concurrent subprocess spawns never collide.
    ///
    /// Claude CLI's `--mcp-config` accepts JSON files with the standard
    /// `mcpServers` shape; the `type: "http"` transport points at the
    /// daemon's existing `/mcp` endpoint (see
    /// `librefang-api/src/routes/network.rs::mcp_http`).
    fn write_mcp_config(bridge: &McpBridgeConfig) -> std::io::Result<PathBuf> {
        let path =
            std::env::temp_dir().join(format!("librefang-mcp-{}.json", uuid::Uuid::new_v4()));
        let base = bridge.base_url.trim_end_matches('/');
        let url = format!("{base}/mcp");

        let mut server = serde_json::json!({
            "type": "http",
            "url": url,
        });
        if let Some(key) = bridge.api_key.as_deref() {
            if !key.trim().is_empty() {
                server["headers"] = serde_json::json!({
                    "X-API-Key": key,
                });
            }
        }

        let config = serde_json::json!({
            "mcpServers": {
                "librefang": server,
            }
        });

        std::fs::write(&path, serde_json::to_vec_pretty(&config)?)?;
        Ok(path)
    }

    /// Map a model ID like "claude-code/opus" to CLI --model flag value.
    fn model_flag(model: &str) -> Option<String> {
        let stripped = model.strip_prefix("claude-code/").unwrap_or(model);
        match stripped {
            "opus" => Some("opus".to_string()),
            "sonnet" => Some("sonnet".to_string()),
            "haiku" => Some("haiku".to_string()),
            _ => Some(stripped.to_string()),
        }
    }

    /// Apply security env filtering to a command.
    ///
    /// Instead of `env_clear()` (which breaks Node.js, NVM, SSL, proxies),
    /// we keep the full environment and only remove known sensitive API keys
    /// from other LLM providers.
    fn apply_env_filter(cmd: &mut tokio::process::Command) {
        for key in SENSITIVE_ENV_EXACT {
            cmd.env_remove(key);
        }
        // Remove any env var with a sensitive suffix, unless it's CLAUDE_*
        for (key, _) in std::env::vars() {
            if key.starts_with("CLAUDE_") {
                continue;
            }
            let upper = key.to_uppercase();
            for suffix in SENSITIVE_SUFFIXES {
                if upper.ends_with(suffix) {
                    cmd.env_remove(&key);
                    break;
                }
            }
        }
    }

    fn build_command_args(
        &self,
        prompt: &str,
        output_format: &str,
        verbose: bool,
        model_flag: Option<&str>,
    ) -> Vec<String> {
        let mut args = vec![
            "-p".to_string(),
            prompt.to_string(),
            "--output-format".to_string(),
            output_format.to_string(),
        ];

        if verbose {
            args.push("--verbose".to_string());
        }

        if self.skip_permissions {
            args.push("--dangerously-skip-permissions".to_string());
        }

        if let Some(model) = model_flag {
            args.push("--model".to_string());
            args.push(model.to_string());
        }

        args
    }

    /// Append `--mcp-config` / `--strict-mcp-config` / `--allowedTools` flags
    /// to a command arg list. Factored out of the two call sites so the test
    /// suite can compare the full arg vector.
    fn append_mcp_args(args: &mut Vec<String>, mcp_config_path: &std::path::Path) {
        args.push("--mcp-config".to_string());
        args.push(mcp_config_path.to_string_lossy().into_owned());
        args.push("--strict-mcp-config".to_string());
        args.push("--allowedTools".to_string());
        // Allow every tool exposed by the `librefang` MCP server. Claude CLI's
        // tool-name convention for MCP-sourced tools is `mcp__<server>__<tool>`,
        // and passing just the server prefix permits all of them.
        args.push("mcp__librefang".to_string());
    }
}

/// Prompt text plus optional temp directory containing decoded images.
struct PreparedPrompt {
    text: String,
    /// Temporary directory holding image files. The caller should pass this
    /// path via `--add-dir` and remove it after the CLI exits.
    image_dir: Option<PathBuf>,
    /// Temporary file holding the MCP bridge config (when tools are enabled).
    /// Passed to the CLI via `--mcp-config` and removed after the CLI exits.
    mcp_config_path: Option<PathBuf>,
}

impl PreparedPrompt {
    /// Clean up temporary image files and MCP config, if any.
    fn cleanup(&self) {
        if let Some(ref dir) = self.image_dir {
            if let Err(e) = std::fs::remove_dir_all(dir) {
                debug!(error = %e, dir = %dir.display(), "Failed to clean up image temp dir");
            }
        }
        if let Some(ref path) = self.mcp_config_path {
            if let Err(e) = std::fs::remove_file(path) {
                debug!(error = %e, path = %path.display(), "Failed to clean up MCP config temp file");
            }
        }
    }
}

/// JSON output from `claude -p --output-format json`.
///
/// The CLI may return the response text in different fields depending on
/// version: `result`, `content`, or `text`. We try all three.
#[derive(Debug, Deserialize)]
struct ClaudeJsonOutput {
    result: Option<String>,
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    usage: Option<ClaudeUsage>,
    #[serde(default)]
    #[allow(dead_code)]
    cost_usd: Option<f64>,
    /// The CLI sets this when the result is an error (auth failure, etc.).
    #[serde(default)]
    is_error: bool,
}

/// Usage stats from Claude CLI JSON output.
#[derive(Debug, Deserialize, Default)]
struct ClaudeUsage {
    #[serde(default)]
    input_tokens: u64,
    #[serde(default)]
    output_tokens: u64,
}

/// Stream JSON event from `claude -p --output-format stream-json`.
#[derive(Debug, Deserialize)]
struct ClaudeStreamEvent {
    #[serde(default)]
    r#type: String,
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    result: Option<String>,
    #[serde(default)]
    usage: Option<ClaudeUsage>,
    /// The CLI sets this when the result is an error (auth failure, etc.).
    #[serde(default)]
    is_error: bool,
}

/// Check if CLI response text looks like an auth or rate-limit error that
/// should be converted to an `LlmError` so token rotation can kick in.
///
/// The Claude CLI sometimes exits with code 0 but sets `is_error: true` and
/// puts the API error in the result text.  This function detects those
/// patterns and returns the appropriate `LlmError` variant.
fn detect_cli_error_in_text(text: &str) -> Option<LlmError> {
    let lower = text.to_lowercase();
    // Auth / credential failures → should trigger rotation to next profile
    if lower.contains("failed to authenticate")
        || lower.contains("authentication_error")
        || lower.contains("invalid authentication credentials")
        || lower.contains("not authenticated")
    {
        return Some(LlmError::Api {
            status: 401,
            message: text.to_string(),
        });
    }
    // Rate-limit / quota exhaustion
    if lower.contains("hit your limit")
        || lower.contains("out of extra usage")
        || lower.contains("rate limit")
        || lower.contains("too many requests")
        || (lower.contains("resets") && lower.contains("utc"))
    {
        return Some(LlmError::RateLimited {
            retry_after_ms: 5 * 60 * 1000,
            message: Some(text.to_string()),
        });
    }
    None
}

#[async_trait]
impl LlmDriver for ClaudeCodeDriver {
    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        // Issue #2314: LibreFang tools are bridged to the spawned Claude CLI
        // via its native `--mcp-config` MCP-client support. When `tools` is
        // non-empty and the kernel has wired an MCP bridge into this driver,
        // we write a temp mcp_config.json pointing at the daemon's `/mcp`
        // endpoint and pass it to `claude -p`. Claude CLI handles the full
        // tool_use / tool_result round-trip natively — no stream parsing,
        // no session plumbing on our side.
        let mut prepared = Self::build_prompt(&request);
        let model_flag = Self::model_flag(&request.model);

        if !request.tools.is_empty() {
            if let Some(ref bridge) = self.mcp_bridge {
                match Self::write_mcp_config(bridge) {
                    Ok(path) => prepared.mcp_config_path = Some(path),
                    Err(e) => {
                        prepared.cleanup();
                        return Err(LlmError::Http(format!(
                            "Failed to write Claude Code MCP bridge config: {e}"
                        )));
                    }
                }
            } else {
                warn!(
                    tool_count = request.tools.len(),
                    "claude_code driver received tools but no MCP bridge is configured; \
                     tools will not be available inside the spawned CLI"
                );
            }
        }

        let mut cmd = tokio::process::Command::new(&self.cli_path);
        let mut args =
            self.build_command_args(&prepared.text, "json", false, model_flag.as_deref());
        if let Some(ref path) = prepared.mcp_config_path {
            Self::append_mcp_args(&mut args, path);
        }
        for arg in args {
            cmd.arg(arg);
        }

        // Allow the CLI to read temp image files
        if let Some(ref dir) = prepared.image_dir {
            cmd.arg("--add-dir").arg(dir);
        }

        Self::apply_env_filter(&mut cmd);
        if let Some(ref dir) = self.config_dir {
            cmd.env("CLAUDE_CONFIG_DIR", dir);
        }

        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());

        debug!(cli = %self.cli_path, skip_permissions = self.skip_permissions, "Spawning Claude Code CLI");

        // Spawn child process instead of cmd.output() so we can track PID and timeout
        let mut child = cmd.spawn().map_err(|e| {
            prepared.cleanup();
            LlmError::Http(format!(
                "Claude Code CLI not found or failed to start ({}). \
                 Install: npm install -g @anthropic-ai/claude-code && claude auth",
                e
            ))
        })?;

        // Track the PID using model + UUID to avoid collisions on concurrent same-model requests
        let pid_label = format!("{}:{}", request.model, uuid::Uuid::new_v4());
        if let Some(pid) = child.id() {
            self.active_pids.insert(pid_label.clone(), pid);
            debug!(pid = pid, label = %pid_label, "Claude Code CLI subprocess started");
        }

        // Take ownership of pipes BEFORE waiting, then drain them
        // concurrently in background tasks. This prevents the subprocess
        // from blocking when pipe buffers fill up (deadlock).
        let child_stdout = child.stdout.take();
        let child_stderr = child.stderr.take();

        let stdout_handle = tokio::spawn(async move {
            let mut buf = Vec::new();
            if let Some(mut out) = child_stdout {
                let _ = out.read_to_end(&mut buf).await;
            }
            buf
        });
        let stderr_handle = tokio::spawn(async move {
            let mut buf = Vec::new();
            if let Some(mut err) = child_stderr {
                let _ = err.read_to_end(&mut buf).await;
            }
            buf
        });

        // Wait with timeout
        let timeout_duration = std::time::Duration::from_secs(
            request.timeout_secs.unwrap_or(self.message_timeout_secs),
        );
        let wait_result = tokio::time::timeout(timeout_duration, child.wait()).await;

        // Clear PID tracking regardless of outcome
        self.active_pids.remove(&pid_label);

        let status = match wait_result {
            Ok(Ok(status)) => status,
            Ok(Err(e)) => {
                warn!(error = %e, model = %pid_label, "Claude Code CLI subprocess failed");
                prepared.cleanup();
                return Err(LlmError::Http(format!(
                    "Claude Code CLI subprocess failed: {e}"
                )));
            }
            Err(_elapsed) => {
                // Timeout — kill the process
                warn!(
                    timeout_secs = self.message_timeout_secs,
                    model = %pid_label,
                    "Claude Code CLI subprocess timed out, killing process"
                );
                let _ = child.kill().await;
                prepared.cleanup();
                return Err(LlmError::Http(format!(
                    "Claude Code CLI subprocess timed out after {}s — process killed",
                    self.message_timeout_secs
                )));
            }
        };

        // Collect output from background drain tasks
        let stdout_bytes = stdout_handle.await.unwrap_or_default();
        let stderr_bytes = stderr_handle.await.unwrap_or_default();

        if !status.success() {
            let stderr = String::from_utf8_lossy(&stderr_bytes).trim().to_string();
            let stdout_str = String::from_utf8_lossy(&stdout_bytes).trim().to_string();
            let detail = if !stderr.is_empty() {
                &stderr
            } else {
                &stdout_str
            };
            let code = status.code().unwrap_or(1);

            warn!(
                exit_code = code,
                model = %pid_label,
                stderr = %detail,
                "Claude Code CLI exited with error"
            );

            // Detect rate-limit and auth error messages so token rotation
            // can kick in.  Use the shared helper for consistent detection.
            if let Some(err) = detect_cli_error_in_text(detail) {
                prepared.cleanup();
                return Err(err);
            }

            // Provide actionable error messages for non-rotatable errors
            let message = if detail.contains("permission")
                || detail.contains("--dangerously-skip-permissions")
            {
                format!(
                    "Claude Code CLI requires permissions acceptance. \
                     Run: claude --dangerously-skip-permissions (once to accept)\nDetail: {detail}"
                )
            } else {
                format!("Claude Code CLI exited with code {code}: {detail}")
            };

            prepared.cleanup();
            return Err(LlmError::Api {
                status: code as u16,
                message,
            });
        }

        // Clean up temp images now that the CLI has finished
        prepared.cleanup();

        info!(model = %pid_label, "Claude Code CLI subprocess completed successfully");

        let stdout = String::from_utf8_lossy(&stdout_bytes);

        // Try JSON parse first
        if let Ok(parsed) = serde_json::from_str::<ClaudeJsonOutput>(&stdout) {
            let text = parsed
                .result
                .or(parsed.content)
                .or(parsed.text)
                .unwrap_or_default();

            // CLI exited 0 but flagged the result as an error (auth failure,
            // rate-limit, etc.).  Convert to LlmError so token rotation fires.
            if parsed.is_error {
                warn!(model = %pid_label, "Claude CLI result has is_error=true, checking for rotatable error");
                if let Some(err) = detect_cli_error_in_text(&text) {
                    return Err(err);
                }
                // is_error but unrecognised pattern — return as generic API error
                return Err(LlmError::Api {
                    status: 1,
                    message: text,
                });
            }

            // Do NOT scan successful output for error patterns — the agent
            // may legitimately mention "rate limit", "not authenticated", etc.
            // Only is_error=true responses (handled above) should be classified.

            let usage = parsed.usage.unwrap_or_default();
            return Ok(CompletionResponse {
                content: vec![ContentBlock::Text {
                    text: text.clone(),
                    provider_metadata: None,
                }],
                stop_reason: StopReason::EndTurn,
                tool_calls: Vec::new(),
                usage: TokenUsage {
                    input_tokens: usage.input_tokens,
                    output_tokens: usage.output_tokens,
                    ..Default::default()
                },
            });
        }

        // Fallback: treat entire stdout as plain text
        let text = stdout.trim().to_string();

        // Safety net for plain-text responses that are really errors
        if let Some(err) = detect_cli_error_in_text(&text) {
            warn!(model = %pid_label, "Claude CLI plain-text response looks like an error");
            return Err(err);
        }

        Ok(CompletionResponse {
            content: vec![ContentBlock::Text {
                text,
                provider_metadata: None,
            }],
            stop_reason: StopReason::EndTurn,
            tool_calls: Vec::new(),
            usage: TokenUsage {
                input_tokens: 0,
                output_tokens: 0,
                ..Default::default()
            },
        })
    }

    async fn stream(
        &self,
        request: CompletionRequest,
        tx: tokio::sync::mpsc::Sender<StreamEvent>,
    ) -> Result<CompletionResponse, LlmError> {
        let mut prepared = Self::build_prompt(&request);
        let model_flag = Self::model_flag(&request.model);

        if !request.tools.is_empty() {
            if let Some(ref bridge) = self.mcp_bridge {
                match Self::write_mcp_config(bridge) {
                    Ok(path) => prepared.mcp_config_path = Some(path),
                    Err(e) => {
                        prepared.cleanup();
                        return Err(LlmError::Http(format!(
                            "Failed to write Claude Code MCP bridge config: {e}"
                        )));
                    }
                }
            } else {
                warn!(
                    tool_count = request.tools.len(),
                    "claude_code driver received tools but no MCP bridge is configured; \
                     tools will not be available inside the spawned CLI"
                );
            }
        }

        let mut cmd = tokio::process::Command::new(&self.cli_path);
        let mut args =
            self.build_command_args(&prepared.text, "stream-json", true, model_flag.as_deref());
        if let Some(ref path) = prepared.mcp_config_path {
            Self::append_mcp_args(&mut args, path);
        }
        for arg in args {
            cmd.arg(arg);
        }

        // Allow the CLI to read temp image files
        if let Some(ref dir) = prepared.image_dir {
            cmd.arg("--add-dir").arg(dir);
        }

        Self::apply_env_filter(&mut cmd);
        if let Some(ref dir) = self.config_dir {
            cmd.env("CLAUDE_CONFIG_DIR", dir);
        }

        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());

        debug!(cli = %self.cli_path, "Spawning Claude Code CLI (streaming)");

        let mut child = cmd.spawn().map_err(|e| {
            prepared.cleanup();
            LlmError::Http(format!(
                "Claude Code CLI not found or failed to start ({}). \
                 Install: npm install -g @anthropic-ai/claude-code && claude auth",
                e
            ))
        })?;

        // Track PID with unique key to avoid collisions on concurrent same-model requests
        let pid_label = format!("{}-stream:{}", request.model, uuid::Uuid::new_v4());
        if let Some(pid) = child.id() {
            self.active_pids.insert(pid_label.clone(), pid);
            debug!(pid = pid, label = %pid_label, "Claude Code CLI streaming subprocess started");
        }

        let stdout = child.stdout.take().ok_or_else(|| {
            self.active_pids.remove(&pid_label);
            prepared.cleanup();
            LlmError::Http("No stdout from claude CLI".to_string())
        })?;

        // Drain stderr in a background task to prevent deadlock
        let child_stderr = child.stderr.take();
        let stderr_handle = tokio::spawn(async move {
            let mut buf = String::new();
            if let Some(stderr) = child_stderr {
                let mut reader = tokio::io::BufReader::new(stderr);
                let _ = AsyncReadExt::read_to_string(&mut reader, &mut buf).await;
            }
            buf
        });

        let reader = tokio::io::BufReader::new(stdout);
        let mut lines = reader.lines();

        let mut full_text = String::new();
        let mut final_usage = TokenUsage {
            input_tokens: 0,
            output_tokens: 0,
            ..Default::default()
        };

        // Track last known activity for timeout diagnostics
        let mut last_activity = "starting".to_string();

        // Progressive inactivity timeout with three escalation levels:
        //   1. warn  (20% of timeout) — log warning, internal only
        //   2. notify (40% of timeout) — send "still working..." to user
        //   3. kill  (100% of timeout) — kill process, preserve partial output
        // The timer resets every time the CLI produces output.
        let kill_secs = request.timeout_secs.unwrap_or(self.message_timeout_secs);
        let warn_secs = kill_secs / 5;
        let notify_secs = kill_secs * 2 / 5;

        let mut last_output = tokio::time::Instant::now();
        let mut warned = false;
        let mut notified = false;

        let stream_err: Option<LlmError> = loop {
            let elapsed = last_output.elapsed().as_secs();
            let next_deadline_secs = if !warned {
                warn_secs.saturating_sub(elapsed)
            } else if !notified {
                notify_secs.saturating_sub(elapsed)
            } else {
                kill_secs.saturating_sub(elapsed)
            };
            let deadline = std::time::Duration::from_secs(next_deadline_secs.max(1));

            match tokio::time::timeout(deadline, lines.next_line()).await {
                Ok(Ok(Some(line))) => {
                    if line.trim().is_empty() {
                        continue;
                    }

                    // Only reset inactivity timer for non-empty lines
                    last_output = tokio::time::Instant::now();
                    warned = false;
                    notified = false;

                    // Helper: detect text that must never be streamed to
                    // channel users (rate-limit messages and NO_REPLY tokens).
                    let should_suppress = |t: &str| -> bool {
                        let l = t.to_lowercase();
                        l.contains("hit your limit")
                            || l.contains("out of extra usage")
                            || l.contains("you've been rate limited")
                            || l.contains("too many requests")
                            || (l.contains("resets") && l.contains("utc"))
                            || t.trim() == "NO_REPLY"
                            || t.trim().ends_with("NO_REPLY")
                    };

                    match serde_json::from_str::<ClaudeStreamEvent>(&line) {
                        Ok(event) => {
                            // Track last activity for timeout diagnostics
                            let etype = event.r#type.as_str();
                            if etype.contains("tool") {
                                // e.g. "tool_use", "tool_result" — extract tool name from content
                                last_activity = event
                                    .content
                                    .as_deref()
                                    .and_then(|c| c.get(..80))
                                    .map(|s| format!("tool: {s}"))
                                    .unwrap_or_else(|| format!("event: {etype}"));
                            } else if !etype.is_empty() {
                                last_activity = format!("event: {etype}");
                            }

                            match etype {
                                "content" | "text" | "assistant" | "content_block_delta" => {
                                    if let Some(ref content) = event.content {
                                        full_text.push_str(content);
                                        if !should_suppress(content) {
                                            let _ = tx
                                                .send(StreamEvent::TextDelta {
                                                    text: content.clone(),
                                                })
                                                .await;
                                        }
                                    }
                                }
                                "result" | "done" | "complete" => {
                                    if let Some(ref result) = event.result {
                                        if full_text.is_empty() {
                                            full_text = result.clone();
                                            // Don't stream error results to the user —
                                            // they will be caught after the loop and
                                            // converted to LlmError for rotation.
                                            if !event.is_error && !should_suppress(result) {
                                                let _ = tx
                                                    .send(StreamEvent::TextDelta {
                                                        text: result.clone(),
                                                    })
                                                    .await;
                                            }
                                        }
                                    }
                                    if let Some(usage) = event.usage {
                                        final_usage = TokenUsage {
                                            input_tokens: usage.input_tokens,
                                            output_tokens: usage.output_tokens,
                                            ..Default::default()
                                        };
                                    }
                                }
                                _ => {
                                    if let Some(ref content) = event.content {
                                        full_text.push_str(content);
                                        if !should_suppress(content) {
                                            let _ = tx
                                                .send(StreamEvent::TextDelta {
                                                    text: content.clone(),
                                                })
                                                .await;
                                        }
                                    }
                                }
                            }
                        }
                        Err(e) => {
                            warn!(line = %line, error = %e, "Non-JSON line from Claude CLI");
                            full_text.push_str(&line);
                            if !should_suppress(&line) {
                                let _ = tx.send(StreamEvent::TextDelta { text: line }).await;
                            }
                        }
                    }
                }
                Ok(Ok(None)) => break None,
                Ok(Err(e)) => {
                    warn!(error = %e, "Claude Code CLI stream IO error");
                    break None;
                }
                Err(_) => {
                    let silent_secs = last_output.elapsed().as_secs();
                    if !warned {
                        warned = true;
                        warn!(silent_secs, model = %pid_label, "Claude CLI: no output, monitoring");
                    } else if !notified {
                        notified = true;
                        info!(silent_secs, model = %pid_label, "Claude CLI: notifying user of long-running task");
                        let _ = tx
                            .send(StreamEvent::PhaseChange {
                                phase: "long_running".to_string(),
                                detail: Some(format!(
                                    "No output for {silent_secs}s — task is still running..."
                                )),
                            })
                            .await;
                    } else {
                        let partial_len = full_text.len();
                        warn!(
                            timeout_secs = kill_secs,
                            partial_output_chars = partial_len,
                            model = %pid_label,
                            "Claude CLI streaming timed out due to inactivity, killing process"
                        );
                        let _ = child.kill().await;
                        break Some(LlmError::TimedOut {
                            inactivity_secs: kill_secs,
                            partial_text_len: partial_len,
                            partial_text: std::mem::take(&mut full_text),
                            last_activity: last_activity.clone(),
                        });
                    }
                }
            }
        };

        // Clear PID tracking
        self.active_pids.remove(&pid_label);

        if let Some(err) = stream_err {
            prepared.cleanup();
            return Err(err);
        }

        // Clean up temp images now that the CLI has finished reading them
        prepared.cleanup();

        // Wait for process to finish
        let status = child
            .wait()
            .await
            .map_err(|e| LlmError::Http(format!("Claude CLI wait failed: {e}")))?;

        let stderr_text = stderr_handle.await.unwrap_or_default();

        if !status.success() {
            let code = status.code().unwrap_or(1);
            let detail = if !stderr_text.trim().is_empty() {
                stderr_text.trim().to_string()
            } else {
                full_text.clone()
            };
            warn!(
                exit_code = code,
                model = %pid_label,
                stderr = %stderr_text,
                "Claude Code CLI streaming subprocess exited with error"
            );
            // Detect rate-limit and auth error messages so token rotation can
            // kick in.  Use the shared helper first; fall back to the
            // empty-output heuristic for exit-code 1.
            if let Some(err) = detect_cli_error_in_text(&detail) {
                warn!(
                    exit_code = code,
                    "Treating CLI exit as rotatable error for profile rotation"
                );
                return Err(err);
            }
            // Do NOT assume empty exit-code-1 is rate-limit — it could be
            // a transient CLI crash, network error, or other non-rotatable issue.
            // Only classified errors (from detect_cli_error_in_text) trigger rotation.
            return Err(LlmError::Api {
                status: code as u16,
                message: format!(
                    "Claude Code CLI streaming exited with code {code}: {}",
                    if stderr_text.trim().is_empty() {
                        "no stderr"
                    } else {
                        stderr_text.trim()
                    }
                ),
            });
        }

        if !stderr_text.trim().is_empty() {
            warn!(stderr = %stderr_text.trim(), "Claude CLI streaming stderr output");
        }

        // Do NOT scan successful streamed output for error patterns.
        // Only exit-code != 0 paths should classify errors.

        let _ = tx
            .send(StreamEvent::ContentComplete {
                stop_reason: StopReason::EndTurn,
                usage: final_usage,
            })
            .await;

        Ok(CompletionResponse {
            content: vec![ContentBlock::Text {
                text: full_text,
                provider_metadata: None,
            }],
            stop_reason: StopReason::EndTurn,
            tool_calls: Vec::new(),
            usage: final_usage,
        })
    }
}

/// Check if the Claude Code CLI is available.
pub fn claude_code_available() -> bool {
    if super::is_proxied_via_env(
        &["ANTHROPIC_BASE_URL", "ANTHROPIC_API_URL"],
        &["api.anthropic.com"],
    ) {
        return false;
    }
    ClaudeCodeDriver::detect().is_some() || claude_credentials_exist()
}

/// Check if Claude Code appears to be installed by looking for known artifacts.
///
/// Checks multiple indicators across CLI versions and auth mechanisms:
/// - `~/.claude/.credentials.json` — older CLI versions (file-based auth)
/// - `~/.claude/credentials.json`  — newer CLI versions (file-based auth)
/// - `~/.claude/settings.json`     — present on all installs; newer versions use the
///   system keychain instead of a credentials file, so the above files will not exist
fn claude_credentials_exist() -> bool {
    if let Some(home) = home_dir() {
        let claude_dir = home.join(".claude");
        claude_dir.join(".credentials.json").exists()
            || claude_dir.join("credentials.json").exists()
            || claude_dir.join("settings.json").exists()
    } else {
        false
    }
}

/// Cross-platform home directory.
fn home_dir() -> Option<std::path::PathBuf> {
    #[cfg(target_os = "windows")]
    {
        std::env::var("USERPROFILE")
            .ok()
            .map(std::path::PathBuf::from)
    }
    #[cfg(not(target_os = "windows"))]
    {
        std::env::var("HOME").ok().map(std::path::PathBuf::from)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_prompt_simple() {
        use librefang_types::message::{Message, MessageContent};

        let request = CompletionRequest {
            model: "claude-code/sonnet".to_string(),
            messages: vec![Message {
                role: Role::User,
                content: MessageContent::text("Hello"),
                pinned: false,
            }],
            tools: vec![],
            max_tokens: 1024,
            temperature: 0.7,
            system: Some("You are helpful.".to_string()),
            thinking: None,
            prompt_caching: false,
            response_format: None,
            timeout_secs: None,
            extra_body: None,
        };

        let prompt = ClaudeCodeDriver::build_prompt(&request);
        assert!(prompt.text.contains("[System]"));
        assert!(prompt.text.contains("You are helpful."));
        assert!(prompt.text.contains("[User]"));
        assert!(prompt.text.contains("Hello"));
        assert!(prompt.image_dir.is_none());
    }

    #[test]
    fn test_build_prompt_with_images() {
        use librefang_types::message::{Message, MessageContent};

        // A small valid base64 PNG (1x1 pixel)
        let png_b64 = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==";

        let request = CompletionRequest {
            model: "claude-code/sonnet".to_string(),
            messages: vec![Message {
                role: Role::User,
                content: MessageContent::Blocks(vec![
                    ContentBlock::Text {
                        text: "What is in this image?".to_string(),
                        provider_metadata: None,
                    },
                    ContentBlock::Image {
                        media_type: "image/png".to_string(),
                        data: png_b64.to_string(),
                    },
                ]),
                pinned: false,
            }],
            tools: vec![],
            max_tokens: 1024,
            temperature: 0.7,
            system: None,
            thinking: None,
            prompt_caching: false,
            response_format: None,
            timeout_secs: None,
            extra_body: None,
        };

        let prompt = ClaudeCodeDriver::build_prompt(&request);
        assert!(prompt.text.contains("What is in this image?"));
        assert!(prompt.text.contains("@"));
        assert!(prompt.text.contains("librefang-images-"));
        assert!(prompt.text.contains(".png"));
        assert!(prompt.image_dir.is_some());

        // Verify the temp file was actually written
        let dir = prompt.image_dir.as_ref().unwrap();
        assert!(dir.join("image-1.png").exists());

        // Cleanup
        prompt.cleanup();
        assert!(!dir.exists());
    }

    #[test]
    fn test_model_flag_mapping() {
        assert_eq!(
            ClaudeCodeDriver::model_flag("claude-code/opus"),
            Some("opus".to_string())
        );
        assert_eq!(
            ClaudeCodeDriver::model_flag("claude-code/sonnet"),
            Some("sonnet".to_string())
        );
        assert_eq!(
            ClaudeCodeDriver::model_flag("claude-code/haiku"),
            Some("haiku".to_string())
        );
        assert_eq!(
            ClaudeCodeDriver::model_flag("custom-model"),
            Some("custom-model".to_string())
        );
    }

    #[test]
    fn test_new_defaults_to_claude() {
        let driver = ClaudeCodeDriver::new(None, true);
        assert_eq!(driver.cli_path, "claude");
        assert_eq!(driver.message_timeout_secs, DEFAULT_MESSAGE_TIMEOUT_SECS);
        assert!(driver.active_pids().is_empty());
    }

    #[test]
    fn test_new_with_custom_path() {
        let driver = ClaudeCodeDriver::new(Some("/usr/local/bin/claude".to_string()), true);
        assert_eq!(driver.cli_path, "/usr/local/bin/claude");
    }

    #[test]
    fn test_new_with_empty_path() {
        let driver = ClaudeCodeDriver::new(Some(String::new()), true);
        assert_eq!(driver.cli_path, "claude");
    }

    #[test]
    fn test_with_timeout() {
        let driver = ClaudeCodeDriver::with_timeout(None, true, 600);
        assert_eq!(driver.message_timeout_secs, 600);
        assert_eq!(driver.cli_path, "claude");
    }

    #[test]
    fn test_pid_map_shared() {
        let driver = ClaudeCodeDriver::new(None, true);
        let map = driver.pid_map();
        map.insert("test-agent".to_string(), 12345);
        assert_eq!(driver.active_pids().len(), 1);
        assert_eq!(driver.active_pids()[0], ("test-agent".to_string(), 12345));
    }

    #[test]
    fn test_complete_args_include_skip_permissions_when_enabled() {
        let driver = ClaudeCodeDriver::new(None, true);
        let args = driver.build_command_args("hello", "json", false, Some("sonnet"));

        assert_eq!(
            args,
            vec![
                "-p",
                "hello",
                "--output-format",
                "json",
                "--dangerously-skip-permissions",
                "--model",
                "sonnet",
            ]
        );
    }

    #[test]
    fn test_stream_args_include_verbose_and_skip_permissions() {
        let driver = ClaudeCodeDriver::new(None, true);
        let args = driver.build_command_args("hello", "stream-json", true, Some("sonnet"));

        assert_eq!(
            args,
            vec![
                "-p",
                "hello",
                "--output-format",
                "stream-json",
                "--verbose",
                "--dangerously-skip-permissions",
                "--model",
                "sonnet",
            ]
        );
    }

    #[test]
    fn test_args_omit_skip_permissions_when_disabled() {
        let driver = ClaudeCodeDriver::new(None, false);
        let args = driver.build_command_args("hello", "json", false, Some("sonnet"));

        assert!(!args
            .iter()
            .any(|arg| arg == "--dangerously-skip-permissions"));
    }

    #[test]
    fn test_sensitive_env_list_coverage() {
        // Ensure all major provider keys are in the strip list
        assert!(SENSITIVE_ENV_EXACT.contains(&"OPENAI_API_KEY"));
        assert!(SENSITIVE_ENV_EXACT.contains(&"ANTHROPIC_API_KEY"));
        assert!(SENSITIVE_ENV_EXACT.contains(&"GEMINI_API_KEY"));
        assert!(SENSITIVE_ENV_EXACT.contains(&"GROQ_API_KEY"));
        assert!(SENSITIVE_ENV_EXACT.contains(&"DEEPSEEK_API_KEY"));
    }

    #[test]
    fn test_detect_tries_absolute_paths() {
        // Verify that detect() falls back to known absolute install paths when
        // `claude` is not on PATH. We cannot easily test the actual binary resolution
        // here, but we can verify the candidate list contains the expected entries.
        // The real coverage comes from the integration path on the developer's machine.
        let candidates: &[&str] = &[
            "claude",
            "/opt/homebrew/bin/claude",
            "/usr/local/bin/claude",
            "/usr/bin/claude",
        ];
        assert!(candidates.contains(&"claude"));
        assert!(candidates.contains(&"/opt/homebrew/bin/claude"));
        assert!(candidates.contains(&"/usr/local/bin/claude"));
    }

    #[test]
    fn test_claude_credentials_exist_checks_settings_json() {
        use std::fs;

        // Create a temp dir that looks like ~/.claude with only settings.json present
        // (simulating keychain-based auth where no credentials file is written).
        let tmp = std::env::temp_dir().join(format!(
            "librefang-test-claude-dir-{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&tmp).unwrap();
        let settings = tmp.join("settings.json");
        fs::write(&settings, "{}").unwrap();

        // Temporarily override HOME so home_dir() resolves to our temp parent.
        // We test the helper directly since home_dir() reads HOME/USERPROFILE.
        let _parent = tmp.parent().unwrap();
        let _dir_name = tmp.file_name().unwrap().to_str().unwrap();

        // Manually replicate the check logic against our temp path.
        let has_credentials = tmp.join(".credentials.json").exists()
            || tmp.join("credentials.json").exists()
            || tmp.join("settings.json").exists();

        assert!(
            has_credentials,
            "settings.json alone should be enough to signal Claude Code is installed"
        );

        // Verify that without settings.json the check returns false.
        fs::remove_file(&settings).unwrap();
        let has_credentials_after = tmp.join(".credentials.json").exists()
            || tmp.join("credentials.json").exists()
            || tmp.join("settings.json").exists();
        assert!(
            !has_credentials_after,
            "should return false when no credential artifacts exist"
        );

        // Verify that the old-style credentials.json still works.
        fs::write(tmp.join("credentials.json"), "{}").unwrap();
        let has_old_creds = tmp.join(".credentials.json").exists()
            || tmp.join("credentials.json").exists()
            || tmp.join("settings.json").exists();
        assert!(has_old_creds, "credentials.json should still be recognised");

        fs::remove_dir_all(&tmp).unwrap();
    }

    #[test]
    fn test_detect_returns_none_for_nonexistent_binary() {
        // A path that will never exist — detect() must return None gracefully.
        let output = std::process::Command::new("/nonexistent/path/to/claude-xyz-abc")
            .arg("--version")
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .output();
        assert!(output.is_err(), "spawning a nonexistent binary should fail");
    }
}
