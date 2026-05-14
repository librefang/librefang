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
///
/// Note: ANTHROPIC_API_KEY is intentionally absent — the Claude Code CLI
/// is Anthropic's own tool and requires its own key to authenticate API
/// calls (OAuth alone is insufficient in -p/--print mode).
const SENSITIVE_ENV_EXACT: &[&str] = &[
    "OPENAI_API_KEY",
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
/// unless it starts with `CLAUDE_` or `ANTHROPIC_` (our own provider's
/// credentials, including gateway / proxy variants like
/// `ANTHROPIC_AUTH_TOKEN`).
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
    /// When `true` (the default), set `LIBREFANG_AGENT_ID`, `LIBREFANG_SESSION_ID`,
    /// and `LIBREFANG_STEP_ID` env vars on the spawned subprocess so operators can
    /// correlate process-tree entries with LibreFang agent sessions.
    emit_caller_trace_headers: bool,
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
            emit_caller_trace_headers: true,
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

    /// Control whether caller-trace env vars are injected into the spawned
    /// subprocess. When `true` (the default), `LIBREFANG_AGENT_ID`,
    /// `LIBREFANG_SESSION_ID`, and `LIBREFANG_STEP_ID` are set from the
    /// `CompletionRequest` fields so operators can correlate OS process-tree
    /// entries with LibreFang agent sessions.
    pub fn with_emit_caller_trace_headers(mut self, emit: bool) -> Self {
        self.emit_caller_trace_headers = emit;
        self
    }

    /// Inject caller-trace env vars into a subprocess command when the flag is on.
    ///
    /// Sets `LIBREFANG_AGENT_ID`, `LIBREFANG_SESSION_ID`, and
    /// `LIBREFANG_STEP_ID` from the `CompletionRequest`. Empty / `None` values
    /// are skipped so the subprocess environment stays clean.
    fn apply_caller_trace_envs(cmd: &mut tokio::process::Command, request: &CompletionRequest) {
        if let Some(ref id) = request.agent_id {
            if !id.is_empty() {
                cmd.env("LIBREFANG_AGENT_ID", id);
            }
        }
        if let Some(ref sid) = request.session_id {
            if !sid.is_empty() {
                cmd.env("LIBREFANG_SESSION_ID", sid);
            }
        }
        if let Some(ref step) = request.step_id {
            if !step.is_empty() {
                cmd.env("LIBREFANG_STEP_ID", step);
            }
        }
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
        let mut extra_image_dirs: std::collections::BTreeSet<PathBuf> =
            std::collections::BTreeSet::new();
        let mut image_count = 0u32;

        if let Some(ref sys) = request.system {
            parts.push(format!("[System]\n{sys}"));
        }

        for msg in request.messages.iter() {
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
                            ContentBlock::Text { text, .. } if !text.is_empty() => {
                                msg_parts.push(text.clone());
                            }
                            ContentBlock::Text { .. } => {}
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
                                    if let Some(parent) = file_path.parent() {
                                        // Claude CLI refuses to read files outside
                                        // the working directory or an explicit
                                        // `--add-dir`. The bridge writes channel
                                        // images to `/tmp/librefang_uploads/`, which
                                        // is neither — register the parent so the
                                        // `@<path>` reference below actually resolves.
                                        extra_image_dirs.insert(parent.to_path_buf());
                                    }
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
            extra_image_dirs,
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
    fn write_mcp_config(
        bridge: &McpBridgeConfig,
        agent_id: Option<&str>,
    ) -> std::io::Result<PathBuf> {
        let path =
            std::env::temp_dir().join(format!("librefang-mcp-{}.json", uuid::Uuid::new_v4()));
        let base = bridge.base_url.trim_end_matches('/');
        let url = format!("{base}/mcp");

        // Collect per-connection headers. Claude CLI reuses the same config
        // for every tool call in a CLI invocation, and one invocation serves
        // exactly one agent, so agent identity can live on the connection
        // instead of on each request.
        let mut headers = serde_json::Map::new();
        if let Some(key) = bridge.api_key.as_deref() {
            if !key.trim().is_empty() {
                headers.insert("X-API-Key".to_string(), serde_json::json!(key));
            }
        }
        if let Some(id) = agent_id {
            if !id.is_empty() {
                // Used by `/mcp` to rehydrate `ToolExecContext` with the
                // owning agent's workspace, tool allowlist, and skill
                // allowlist. Without it, file/media/cron/schedule tools
                // fail with "workspace sandbox not configured" or
                // "Agent ID required" even though the agent is fully
                // registered (issue #2699).
                headers.insert("X-LibreFang-Agent-Id".to_string(), serde_json::json!(id));
            }
        }

        let mut server = serde_json::json!({
            "type": "http",
            "url": url,
        });
        if !headers.is_empty() {
            server["headers"] = serde_json::Value::Object(headers);
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
        // or ANTHROPIC_*. The ANTHROPIC_ exception covers gateway / proxy
        // credentials such as ANTHROPIC_AUTH_TOKEN, typically paired with
        // ANTHROPIC_BASE_URL for Bedrock-style routing.
        //
        // The prefix match is case-sensitive by design — Unix env var names
        // are case-sensitive, and the exact-list above also matches verbatim.
        // A user typing `anthropic_foo_token` would still hit the suffix
        // strip below, which is the intended fail-safe.
        for (key, _) in std::env::vars() {
            if key.starts_with("CLAUDE_") || key.starts_with("ANTHROPIC_") {
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
        output_format: &str,
        verbose: bool,
        model_flag: Option<&str>,
    ) -> Vec<String> {
        // Prompt is fed via stdin, not argv. Passing the full rendered
        // prompt as a CLI argument crashed `execve` with `E2BIG` once the
        // skill catalog + history + tool registry pushed it past Linux
        // ARG_MAX (~128 KB on most kernels).
        let mut args = vec![
            "-p".to_string(),
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

    /// On stdin write failure (typically EPIPE because the CLI exited
    /// during its own init) drain the CLI's stderr for up to 2 s and
    /// fold the captured snippet into the surfaced error message.
    /// Without this the caller sees only `Broken pipe` and has no clue
    /// why the CLI quit — invalid auth profile, missing/unreadable cwd,
    /// bad `--mcp-config`, etc. Kills the child as a side effect.
    async fn diagnose_stdin_write_failure(
        child: &mut tokio::process::Child,
        write_err: &std::io::Error,
    ) -> String {
        use tokio::io::AsyncReadExt;
        const STDERR_CAP: usize = 4096;
        const STDERR_WAIT: std::time::Duration = std::time::Duration::from_millis(2000);

        let stderr_pipe = child.stderr.take();
        // Best-effort kill: the child is almost certainly already dead
        // (that's why the pipe broke), but never leak a stuck process.
        let _ = child.kill().await;

        let stderr_snippet = if let Some(mut err) = stderr_pipe {
            let mut buf: Vec<u8> = Vec::with_capacity(STDERR_CAP);
            let _ = tokio::time::timeout(STDERR_WAIT, async {
                let mut chunk = [0u8; 1024];
                while buf.len() < STDERR_CAP {
                    match err.read(&mut chunk).await {
                        Ok(0) | Err(_) => break,
                        Ok(n) => {
                            let take = (STDERR_CAP - buf.len()).min(n);
                            buf.extend_from_slice(&chunk[..take]);
                        }
                    }
                }
            })
            .await;
            String::from_utf8_lossy(&buf).trim().to_string()
        } else {
            String::new()
        };

        if stderr_snippet.is_empty() {
            format!(
                "Failed to write prompt to Claude Code CLI stdin: {write_err}. \
                 The CLI exited before reading stdin (no stderr captured). \
                 Common causes: invalid auth profile (run `claude /status` to verify), \
                 missing or unreadable workspace cwd, invalid `--mcp-config` path."
            )
        } else {
            format!(
                "Failed to write prompt to Claude Code CLI stdin: {write_err}. \
                 The CLI exited before reading stdin. Captured stderr: {stderr_snippet}"
            )
        }
    }
}

/// Prompt text plus optional temp directory containing decoded images.
struct PreparedPrompt {
    text: String,
    /// Temporary directory this driver created to hold base64-decoded images
    /// from `ContentBlock::Image` blocks. Owned: must be passed to `--add-dir`
    /// AND removed by `cleanup()` after the CLI exits.
    image_dir: Option<PathBuf>,
    /// Parent directories of `ContentBlock::ImageFile` paths — files already
    /// on disk, owned by the bridge (or the caller). These must also be
    /// passed to `--add-dir` so the CLI can read the referenced files, but
    /// the driver MUST NOT delete them (not ours to clean up).
    extra_image_dirs: std::collections::BTreeSet<PathBuf>,
    /// Temporary file holding the MCP bridge config (when tools are enabled).
    /// Passed to the CLI via `--mcp-config` and removed after the CLI exits.
    mcp_config_path: Option<PathBuf>,
}

impl PreparedPrompt {
    /// Iterate every directory that must be made readable to the CLI via
    /// `--add-dir`: the driver-owned image temp dir (if any) plus every
    /// externally-owned ImageFile parent directory.
    fn add_dirs(&self) -> impl Iterator<Item = &PathBuf> {
        self.image_dir.iter().chain(self.extra_image_dirs.iter())
    }

    /// Clean up temporary image files and MCP config, if any. Only removes
    /// driver-owned artifacts; `extra_image_dirs` are intentionally left
    /// alone because they belong to the bridge or the caller.
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
            code: None,
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
    #[tracing::instrument(
        name = "llm.complete",
        skip_all,
        fields(provider = "claude_code", model = %request.model)
    )]
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
                match Self::write_mcp_config(bridge, request.agent_id.as_deref()) {
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
        let mut args = self.build_command_args("json", false, model_flag.as_deref());
        if let Some(ref path) = prepared.mcp_config_path {
            Self::append_mcp_args(&mut args, path);
        }
        for arg in args {
            cmd.arg(arg);
        }

        // Allow the CLI to read every directory containing referenced images:
        // the driver-owned base64 temp dir plus bridge-owned ImageFile parents.
        for dir in prepared.add_dirs() {
            cmd.arg("--add-dir").arg(dir);
        }

        Self::apply_env_filter(&mut cmd);
        if let Some(ref dir) = self.config_dir {
            cmd.env("CLAUDE_CONFIG_DIR", dir);
        }
        if self.emit_caller_trace_headers {
            Self::apply_caller_trace_envs(&mut cmd, &request);
        }

        // Prompt is piped to the CLI's stdin. Passing it as argv crashed
        // execve with E2BIG once the skill catalog + history + tool registry
        // pushed the rendered text past Linux ARG_MAX (~128 KB).
        cmd.stdin(std::process::Stdio::piped());
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());

        debug!(
            cli = %self.cli_path,
            skip_permissions = self.skip_permissions,
            prompt_bytes = prepared.text.len(),
            "Spawning Claude Code CLI"
        );

        // Spawn child process instead of cmd.output() so we can track PID and timeout
        let mut child = cmd.spawn().map_err(|e| {
            prepared.cleanup();
            LlmError::Http(format!(
                "Claude Code CLI not found or failed to start ({}). \
                 Install: npm install -g @anthropic-ai/claude-code && claude auth",
                e
            ))
        })?;

        // Write the prompt to stdin and close it so the CLI sees EOF and
        // begins processing. tokio::io::AsyncWriteExt::write_all chunks
        // automatically — no per-write size limit applies here, only the
        // pipe-buffer ceiling, which the kernel handles transparently.
        if let Some(mut stdin) = child.stdin.take() {
            use tokio::io::AsyncWriteExt;
            if let Err(e) = stdin.write_all(prepared.text.as_bytes()).await {
                let diag = Self::diagnose_stdin_write_failure(&mut child, &e).await;
                prepared.cleanup();
                return Err(LlmError::Http(diag));
            }
            // Drop closes stdin; CLI proceeds with the full prompt.
            drop(stdin);
        }

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
                code: None,
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
                    code: None,
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

    #[tracing::instrument(
        name = "llm.stream",
        skip_all,
        fields(provider = "claude_code", model = %request.model)
    )]
    async fn stream(
        &self,
        request: CompletionRequest,
        tx: tokio::sync::mpsc::Sender<StreamEvent>,
    ) -> Result<CompletionResponse, LlmError> {
        let mut prepared = Self::build_prompt(&request);
        let model_flag = Self::model_flag(&request.model);

        if !request.tools.is_empty() {
            if let Some(ref bridge) = self.mcp_bridge {
                match Self::write_mcp_config(bridge, request.agent_id.as_deref()) {
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
        let mut args = self.build_command_args("stream-json", true, model_flag.as_deref());
        if let Some(ref path) = prepared.mcp_config_path {
            Self::append_mcp_args(&mut args, path);
        }
        for arg in args {
            cmd.arg(arg);
        }

        // Allow the CLI to read every directory containing referenced images:
        // the driver-owned base64 temp dir plus bridge-owned ImageFile parents.
        for dir in prepared.add_dirs() {
            cmd.arg("--add-dir").arg(dir);
        }

        Self::apply_env_filter(&mut cmd);
        if let Some(ref dir) = self.config_dir {
            cmd.env("CLAUDE_CONFIG_DIR", dir);
        }
        if self.emit_caller_trace_headers {
            Self::apply_caller_trace_envs(&mut cmd, &request);
        }

        // Same stdin-piping rationale as the non-streaming path: prompt
        // exceeds ARG_MAX once the skill catalog and history grow.
        cmd.stdin(std::process::Stdio::piped());
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());

        debug!(
            cli = %self.cli_path,
            prompt_bytes = prepared.text.len(),
            "Spawning Claude Code CLI (streaming)"
        );

        let mut child = cmd.spawn().map_err(|e| {
            prepared.cleanup();
            LlmError::Http(format!(
                "Claude Code CLI not found or failed to start ({}). \
                 Install: npm install -g @anthropic-ai/claude-code && claude auth",
                e
            ))
        })?;

        if let Some(mut stdin) = child.stdin.take() {
            use tokio::io::AsyncWriteExt;
            if let Err(e) = stdin.write_all(prepared.text.as_bytes()).await {
                let diag = Self::diagnose_stdin_write_failure(&mut child, &e).await;
                prepared.cleanup();
                return Err(LlmError::Http(diag));
            }
            drop(stdin);
        }

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
                    //
                    // NB: this driver runs at a layer BELOW librefang-runtime
                    // (drivers is a leaf dep of runtime — depending the other
                    // way would create a cycle). The sentinel check here is a
                    // deliberate, narrow duplicate of the canonical
                    // `librefang_runtime::silent_response::is_silent_response`
                    // detector — it operates on streaming line fragments
                    // (which may be partial sentences), so a permissive
                    // trim/ends_with check is the correct semantics for this
                    // call site. The canonical module remains the
                    // single-source-of-truth for whole-response classification
                    // upstream (agent_loop, session_repair, gateway).
                    let should_suppress = |t: &str| -> bool {
                        let l = t.to_lowercase();
                        l.contains("hit your limit")
                            || l.contains("out of extra usage")
                            || l.contains("you've been rate limited")
                            || l.contains("too many requests")
                            || (l.contains("resets") && l.contains("utc"))
                            || t.trim() == "NO_REPLY"
                            || t.trim().ends_with("NO_REPLY")
                            // Suppress CLI progress placeholders that leak
                            // into channel text when the model emits a
                            // status preamble alone (no real reply). Both
                            // bracket and paren shapes — observed live as
                            // `[Reading the conversation context]` and
                            // `(thinking)` whole-message replies. Narrow
                            // on purpose so legitimate user content that
                            // happens to start with a paren or bracket
                            // (e.g. `[1] First...` lists) is not suppressed.
                            || {
                                let trimmed = t.trim();
                                let bracket_wrapped = (trimmed.starts_with('[')
                                    && trimmed.ends_with(']'))
                                    || (trimmed.starts_with('(')
                                        && trimmed.ends_with(')'));
                                bracket_wrapped
                                    && (l.contains("reading")
                                        || l.contains("thinking")
                                        || l.contains("loading")
                                        || l.contains("processing")
                                        || l.contains("analyzing")
                                        || l.contains("conversation context")
                                        || l.contains("still working"))
                            }
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
                                        if !should_suppress(content)
                                            && tx
                                                .send(StreamEvent::TextDelta {
                                                    text: content.clone(),
                                                })
                                                .await
                                                .is_err()
                                        {
                                            // Receiver dropped — stop streaming events.
                                            // The CLI subprocess will be killed below
                                            // when the loop ends (#3769).
                                            tracing::debug!(
                                                "streaming receiver dropped; cancelling Claude Code CLI stream"
                                            );
                                            let _ = child.kill().await;
                                            break None;
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
                                            if !event.is_error
                                                && !should_suppress(result)
                                                && tx
                                                    .send(StreamEvent::TextDelta {
                                                        text: result.clone(),
                                                    })
                                                    .await
                                                    .is_err()
                                            {
                                                break None;
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
                                        if !should_suppress(content)
                                            && tx
                                                .send(StreamEvent::TextDelta {
                                                    text: content.clone(),
                                                })
                                                .await
                                                .is_err()
                                        {
                                            break None;
                                        }
                                    }
                                }
                            }
                        }
                        Err(e) => {
                            warn!(line = %line, error = %e, "Non-JSON line from Claude CLI");
                            full_text.push_str(&line);
                            if !should_suppress(&line)
                                && tx
                                    .send(StreamEvent::TextDelta { text: line })
                                    .await
                                    .is_err()
                            {
                                break None;
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
                        let partial_body: std::sync::Arc<str> =
                            std::sync::Arc::from(std::mem::take(&mut full_text));
                        break Some(LlmError::TimedOut {
                            inactivity_secs: kill_secs,
                            partial_text_len: partial_len,
                            // #3552: Arc-shared so error clone / stringify is O(1).
                            partial_text: if partial_body.is_empty() {
                                None
                            } else {
                                Some(partial_body)
                            },
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
                code: None,
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

    fn family(&self) -> crate::llm_driver::LlmFamily {
        crate::llm_driver::LlmFamily::Anthropic
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

/// Check if Claude Code credentials exist on disk.
///
/// Only looks for actual credential files:
/// - `~/.claude/.credentials.json` — older CLI versions (file-based auth)
/// - `~/.claude/credentials.json`  — newer CLI versions (file-based auth)
///
/// `settings.json` is intentionally NOT checked. It is created on first launch
/// as a preference file (theme, default model, etc.) whether or not the user
/// ever authenticates, so treating it as a credential falsely marks Claude
/// Code as "configured" for anyone who merely installed the CLI.
///
/// Keychain-based auth (newer versions) is already covered by the primary
/// `detect()` path, which finds the `claude` binary on PATH or at common
/// install locations (`/opt/homebrew/bin`, `/usr/local/bin`, `/usr/bin`).
/// The credentials-file fallback is only relevant for non-standard installs.
fn claude_credentials_exist() -> bool {
    home_dir()
        .map(|h| claude_credentials_in_dir(&h.join(".claude")))
        .unwrap_or(false)
}

fn claude_credentials_in_dir(dir: &std::path::Path) -> bool {
    dir.join(".credentials.json").exists() || dir.join("credentials.json").exists()
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

    /// Pin: when stdin write fails (child exits during init), the error
    /// surface must include any stderr the CLI emitted before death.
    /// Without this the operator gets a bare "Broken pipe" with no clue
    /// whether to re-auth, fix the workspace, or rebuild the binary.
    #[tokio::test]
    async fn diagnose_stdin_write_failure_includes_child_stderr() {
        // Spawn /bin/sh that prints a recognisable error to stderr and
        // immediately exits without ever reading stdin → next stdin
        // write will EPIPE just like a real claude-code init failure.
        let mut cmd = tokio::process::Command::new("sh");
        cmd.arg("-c")
            .arg("echo 'mock cli: auth profile invalid' >&2; exit 7");
        cmd.stdin(std::process::Stdio::piped());
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());
        let mut child = cmd.spawn().expect("spawn mock cli");

        // Give the child a moment to exit so its stdin pipe is closed.
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;

        let write_err = std::io::Error::new(std::io::ErrorKind::BrokenPipe, "Broken pipe");
        let diag = ClaudeCodeDriver::diagnose_stdin_write_failure(&mut child, &write_err).await;

        assert!(
            diag.contains("Failed to write prompt to Claude Code CLI stdin"),
            "diag must keep the original failure header, got: {diag}"
        );
        assert!(
            diag.contains("mock cli: auth profile invalid"),
            "diag must include captured stderr from the dying child, got: {diag}"
        );
    }

    /// When the child produces no stderr at all, the diagnostic must
    /// still be actionable — fall back to a hint pointing at the common
    /// auth/cwd/MCP causes rather than just leaking the io::Error.
    #[tokio::test]
    async fn diagnose_stdin_write_failure_falls_back_to_hint_when_silent() {
        let mut cmd = tokio::process::Command::new("sh");
        // No stderr output, just immediate exit.
        cmd.arg("-c").arg("exit 0");
        cmd.stdin(std::process::Stdio::piped());
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());
        let mut child = cmd.spawn().expect("spawn silent child");
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        let write_err = std::io::Error::new(std::io::ErrorKind::BrokenPipe, "Broken pipe");
        let diag = ClaudeCodeDriver::diagnose_stdin_write_failure(&mut child, &write_err).await;

        assert!(
            diag.contains("Common causes"),
            "silent-child diag must surface the hint block, got: {diag}"
        );
        assert!(
            diag.contains("claude /status"),
            "hint must mention the auth check command, got: {diag}"
        );
    }

    #[test]
    fn test_build_prompt_simple() {
        use librefang_types::message::{Message, MessageContent};

        let request = CompletionRequest {
            model: "claude-code/sonnet".to_string(),
            messages: std::sync::Arc::new(vec![Message {
                role: Role::User,
                content: MessageContent::text("Hello"),
                pinned: false,
                timestamp: None,
            }]),
            tools: std::sync::Arc::new(vec![]),
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
            reasoning_echo_policy: librefang_types::model_catalog::ReasoningEchoPolicy::default(),
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
            messages: std::sync::Arc::new(vec![Message {
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
                timestamp: None,
            }]),
            tools: std::sync::Arc::new(vec![]),
            max_tokens: 1024,
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
            reasoning_echo_policy: librefang_types::model_catalog::ReasoningEchoPolicy::default(),
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
    fn test_build_prompt_with_image_file_registers_parent_dir() {
        use librefang_types::message::{Message, MessageContent};

        // Regression: `ContentBlock::ImageFile` points at a path on disk
        // (written by the channel bridge, e.g. `/tmp/librefang_uploads/<uuid>.jpg`).
        // The CLI refuses to read outside the working directory or an
        // explicit `--add-dir`, so the driver must register the file's
        // parent directory on `extra_image_dirs` and emit it via
        // `add_dirs()` at spawn time.
        let bridge_dir =
            std::env::temp_dir().join(format!("librefang-imagefile-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&bridge_dir).unwrap();
        let image_path = bridge_dir.join("photo.jpg");
        std::fs::write(&image_path, b"fake jpg bytes").unwrap();

        let request = CompletionRequest {
            model: "claude-code/sonnet".to_string(),
            messages: std::sync::Arc::new(vec![Message {
                role: Role::User,
                content: MessageContent::Blocks(vec![
                    ContentBlock::Text {
                        text: "What is in this image?".to_string(),
                        provider_metadata: None,
                    },
                    ContentBlock::ImageFile {
                        media_type: "image/jpeg".to_string(),
                        path: image_path.to_string_lossy().to_string(),
                    },
                ]),
                pinned: false,
                timestamp: None,
            }]),
            tools: std::sync::Arc::new(vec![]),
            max_tokens: 1024,
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
            reasoning_echo_policy: librefang_types::model_catalog::ReasoningEchoPolicy::default(),
        };

        let prompt = ClaudeCodeDriver::build_prompt(&request);

        // The `@<abs-path>` reference is injected verbatim.
        assert!(
            prompt.text.contains(&format!("@{}", image_path.display())),
            "prompt text must contain @-reference to ImageFile path: {}",
            prompt.text
        );

        // No owned temp dir was created (no base64 blocks).
        assert!(prompt.image_dir.is_none());

        // The bridge-owned parent dir must be in `extra_image_dirs`.
        assert_eq!(prompt.extra_image_dirs.len(), 1);
        assert!(prompt.extra_image_dirs.contains(&bridge_dir));

        // `add_dirs()` must surface the parent so spawn-site callers pass
        // it as `--add-dir`.
        let dirs: Vec<&PathBuf> = prompt.add_dirs().collect();
        assert_eq!(dirs.len(), 1);
        assert_eq!(dirs[0], &bridge_dir);

        // Cleanup must NOT touch externally-owned dirs.
        prompt.cleanup();
        assert!(
            bridge_dir.exists(),
            "cleanup must leave bridge-owned dir in place"
        );

        // Manual cleanup of this test's fixture.
        let _ = std::fs::remove_dir_all(&bridge_dir);
    }

    #[test]
    fn test_add_dirs_combines_owned_and_external() {
        // With both a base64 image (owned temp dir) and an ImageFile
        // (external parent), `add_dirs()` must yield both, and `cleanup()`
        // must only remove the owned one.
        use librefang_types::message::{Message, MessageContent};

        let png_b64 = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==";

        let bridge_dir =
            std::env::temp_dir().join(format!("librefang-mixed-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&bridge_dir).unwrap();
        let image_path = bridge_dir.join("photo.jpg");
        std::fs::write(&image_path, b"fake jpg bytes").unwrap();

        let request = CompletionRequest {
            model: "claude-code/sonnet".to_string(),
            messages: std::sync::Arc::new(vec![Message {
                role: Role::User,
                content: MessageContent::Blocks(vec![
                    ContentBlock::Image {
                        media_type: "image/png".to_string(),
                        data: png_b64.to_string(),
                    },
                    ContentBlock::ImageFile {
                        media_type: "image/jpeg".to_string(),
                        path: image_path.to_string_lossy().to_string(),
                    },
                ]),
                pinned: false,
                timestamp: None,
            }]),
            tools: std::sync::Arc::new(vec![]),
            max_tokens: 1024,
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
            reasoning_echo_policy: librefang_types::model_catalog::ReasoningEchoPolicy::default(),
        };

        let prompt = ClaudeCodeDriver::build_prompt(&request);
        let owned = prompt.image_dir.clone().expect("base64 creates owned dir");

        let dirs: Vec<PathBuf> = prompt.add_dirs().cloned().collect();
        assert_eq!(dirs.len(), 2);
        assert!(dirs.contains(&owned));
        assert!(dirs.contains(&bridge_dir));

        prompt.cleanup();
        assert!(!owned.exists(), "owned dir removed");
        assert!(bridge_dir.exists(), "external dir preserved");

        let _ = std::fs::remove_dir_all(&bridge_dir);
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
        let args = driver.build_command_args("json", false, Some("sonnet"));

        assert_eq!(
            args,
            vec![
                "-p",
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
        let args = driver.build_command_args("stream-json", true, Some("sonnet"));

        assert_eq!(
            args,
            vec![
                "-p",
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
        let args = driver.build_command_args("json", false, Some("sonnet"));

        assert!(!args
            .iter()
            .any(|arg| arg == "--dangerously-skip-permissions"));
    }

    #[test]
    fn test_args_no_longer_carry_prompt_in_argv() {
        // Regression: passing the prompt as a CLI argument crashed
        // execve with E2BIG once the rendered text exceeded ARG_MAX
        // (~128 KB on most kernels). Prompt is now piped to stdin.
        let driver = ClaudeCodeDriver::new(None, true);
        let args = driver.build_command_args("json", false, None);
        // Argument vector must be small and bounded — no prompt body in it.
        assert!(args.iter().all(|a| a.len() < 256));
        assert!(args.contains(&"-p".to_string()));
    }

    #[test]
    fn test_sensitive_env_list_coverage() {
        // Ensure all major provider keys are in the strip list
        assert!(SENSITIVE_ENV_EXACT.contains(&"OPENAI_API_KEY"));
        assert!(SENSITIVE_ENV_EXACT.contains(&"GEMINI_API_KEY"));
        assert!(SENSITIVE_ENV_EXACT.contains(&"GROQ_API_KEY"));
        assert!(SENSITIVE_ENV_EXACT.contains(&"DEEPSEEK_API_KEY"));
    }

    #[test]
    fn test_apply_env_filter_keeps_anthropic_auth_token() {
        // Regression for #5006: the suffix-sweep used to strip
        // ANTHROPIC_AUTH_TOKEN because it ends in _TOKEN and lacks the
        // CLAUDE_ prefix. The ANTHROPIC_* exception keeps gateway / proxy
        // credentials (ANTHROPIC_AUTH_TOKEN + ANTHROPIC_BASE_URL pattern)
        // intact while still stripping other providers' secrets.
        //
        // SAFETY: unique env var names; the test removes each one before
        // returning.
        unsafe {
            std::env::set_var("ANTHROPIC_AUTH_TOKEN", "keep-me-5006");
            std::env::set_var("OPENAI_API_KEY", "strip-openai-5006");
            std::env::set_var("GROQ_API_KEY", "strip-groq-5006");
            std::env::set_var("GEMINI_API_KEY", "strip-gemini-5006");
            std::env::set_var("LIBREFANG_TEST_5006_OTHER_TOKEN", "strip-suffix-5006");
        }

        let mut cmd = tokio::process::Command::new("echo");
        ClaudeCodeDriver::apply_env_filter(&mut cmd);

        // `env_remove` records `(key, None)` in the Command's env table.
        // Inspect it to learn which keys the filter targeted for removal.
        let removed: std::collections::HashSet<String> = cmd
            .as_std()
            .get_envs()
            .filter_map(|(k, v)| {
                if v.is_none() {
                    Some(k.to_string_lossy().to_string())
                } else {
                    None
                }
            })
            .collect();

        assert!(
            !removed.contains("ANTHROPIC_AUTH_TOKEN"),
            "ANTHROPIC_AUTH_TOKEN must be preserved for gateway / proxy users (#5006)"
        );
        assert!(
            removed.contains("OPENAI_API_KEY"),
            "OPENAI_API_KEY must still be stripped"
        );
        assert!(
            removed.contains("GROQ_API_KEY"),
            "GROQ_API_KEY must still be stripped"
        );
        assert!(
            removed.contains("GEMINI_API_KEY"),
            "GEMINI_API_KEY must still be stripped"
        );
        assert!(
            removed.contains("LIBREFANG_TEST_5006_OTHER_TOKEN"),
            "Generic *_TOKEN env vars (no CLAUDE_/ANTHROPIC_ prefix) must still be stripped"
        );

        // SAFETY: matches the set_var calls above.
        unsafe {
            std::env::remove_var("ANTHROPIC_AUTH_TOKEN");
            std::env::remove_var("OPENAI_API_KEY");
            std::env::remove_var("GROQ_API_KEY");
            std::env::remove_var("GEMINI_API_KEY");
            std::env::remove_var("LIBREFANG_TEST_5006_OTHER_TOKEN");
        }
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

    fn make_claude_tmp_dir(label: &str) -> std::path::PathBuf {
        let p = std::env::temp_dir().join(format!(
            "librefang-test-claude-{label}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0),
        ));
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn settings_json_alone_is_not_a_credential() {
        // `settings.json` is created on first launch as a preference file
        // (theme, default model) even when the user never signs in. It must
        // not be treated as proof of authentication.
        let dir = make_claude_tmp_dir("settings-only");
        std::fs::write(dir.join("settings.json"), "{}").unwrap();
        assert!(
            !claude_credentials_in_dir(&dir),
            "settings.json must not be treated as a Claude Code credential"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn credentials_json_variants_are_recognised() {
        for name in [".credentials.json", "credentials.json"] {
            let dir = make_claude_tmp_dir(&format!("creds-{name}"));
            std::fs::write(dir.join(name), "{}").unwrap();
            assert!(
                claude_credentials_in_dir(&dir),
                "{name} should be recognised"
            );
            std::fs::remove_dir_all(&dir).unwrap();
        }
    }

    #[test]
    fn claude_empty_dir_has_no_credentials() {
        let dir = make_claude_tmp_dir("empty");
        assert!(!claude_credentials_in_dir(&dir));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn test_caller_trace_envs_set_when_flag_on() {
        // apply_caller_trace_envs must set all three vars when all IDs are present.
        let mut cmd = tokio::process::Command::new("echo");
        let request = CompletionRequest {
            model: "claude-code/sonnet".to_string(),
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
            agent_id: Some("agent-abc".to_string()),
            session_id: Some("sess-xyz".to_string()),
            step_id: Some("step-001".to_string()),
            reasoning_echo_policy: librefang_types::model_catalog::ReasoningEchoPolicy::default(),
        };
        ClaudeCodeDriver::apply_caller_trace_envs(&mut cmd, &request);
        let envs: std::collections::HashMap<_, _> = cmd
            .as_std()
            .get_envs()
            .filter_map(|(k, v)| {
                v.map(|v| {
                    (
                        k.to_string_lossy().to_string(),
                        v.to_string_lossy().to_string(),
                    )
                })
            })
            .collect();
        assert_eq!(
            envs.get("LIBREFANG_AGENT_ID").map(|s| s.as_str()),
            Some("agent-abc")
        );
        assert_eq!(
            envs.get("LIBREFANG_SESSION_ID").map(|s| s.as_str()),
            Some("sess-xyz")
        );
        assert_eq!(
            envs.get("LIBREFANG_STEP_ID").map(|s| s.as_str()),
            Some("step-001")
        );
    }

    #[test]
    fn test_caller_trace_envs_absent_when_flag_off() {
        // When emit_caller_trace_headers is false the driver must not inject any
        // LIBREFANG_* env vars onto the subprocess command.
        let driver = ClaudeCodeDriver::new(None, false).with_emit_caller_trace_headers(false);
        assert!(!driver.emit_caller_trace_headers);
    }

    #[test]
    fn test_caller_trace_envs_skips_empty_values() {
        // None / empty IDs must not produce empty env var entries.
        let mut cmd = tokio::process::Command::new("echo");
        let request = CompletionRequest {
            model: "claude-code/sonnet".to_string(),
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
            session_id: Some(String::new()),
            step_id: None,
            reasoning_echo_policy: librefang_types::model_catalog::ReasoningEchoPolicy::default(),
        };
        ClaudeCodeDriver::apply_caller_trace_envs(&mut cmd, &request);
        let envs: std::collections::HashMap<_, _> = cmd
            .as_std()
            .get_envs()
            .filter_map(|(k, v)| {
                v.map(|v| {
                    (
                        k.to_string_lossy().to_string(),
                        v.to_string_lossy().to_string(),
                    )
                })
            })
            .collect();
        assert!(!envs.contains_key("LIBREFANG_AGENT_ID"));
        assert!(!envs.contains_key("LIBREFANG_SESSION_ID"));
        assert!(!envs.contains_key("LIBREFANG_STEP_ID"));
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

    #[test]
    fn test_mcp_config_carries_agent_id_header() {
        // Regression: without the X-LibreFang-Agent-Id header, the /mcp
        // endpoint has no way to rehydrate the caller's workspace / tool
        // allowlist / skill allowlist / exec_policy, so every file_*,
        // media_*, cron_create, schedule_create tool invoked from the
        // spawned Claude CLI fails with "workspace sandbox not configured"
        // or "Agent ID required" even though the agent is fully
        // registered. See issue #2699.
        let bridge = McpBridgeConfig {
            base_url: "http://127.0.0.1:4545".to_string(),
            api_key: Some("secret-key".to_string()),
        };
        let path = ClaudeCodeDriver::write_mcp_config(&bridge, Some("agent-1234")).unwrap();
        let written = std::fs::read_to_string(&path).unwrap();
        let _ = std::fs::remove_file(&path);
        let cfg: serde_json::Value = serde_json::from_str(&written).unwrap();
        let headers = &cfg["mcpServers"]["librefang"]["headers"];
        assert_eq!(headers["X-API-Key"], "secret-key");
        assert_eq!(headers["X-LibreFang-Agent-Id"], "agent-1234");
    }

    #[test]
    fn test_mcp_config_omits_agent_id_header_when_absent() {
        // No agent_id → no X-LibreFang-Agent-Id header. The `/mcp`
        // endpoint then falls back to its legacy unauthenticated
        // behaviour (all context fields None), preserving backward
        // compatibility for non-agent MCP clients that connect to /mcp
        // directly.
        let bridge = McpBridgeConfig {
            base_url: "http://127.0.0.1:4545".to_string(),
            api_key: Some("secret-key".to_string()),
        };
        let path = ClaudeCodeDriver::write_mcp_config(&bridge, None).unwrap();
        let written = std::fs::read_to_string(&path).unwrap();
        let _ = std::fs::remove_file(&path);
        let cfg: serde_json::Value = serde_json::from_str(&written).unwrap();
        let headers = &cfg["mcpServers"]["librefang"]["headers"];
        assert_eq!(headers["X-API-Key"], "secret-key");
        assert!(headers.get("X-LibreFang-Agent-Id").is_none());
    }

    #[test]
    fn test_mcp_config_no_headers_when_nothing_to_send() {
        // Neither api_key nor agent_id set — the `headers` object must
        // be omitted entirely (not an empty {}). Claude CLI tolerates
        // either but the clean shape matches what the driver wrote
        // before this change.
        let bridge = McpBridgeConfig {
            base_url: "http://127.0.0.1:4545".to_string(),
            api_key: None,
        };
        let path = ClaudeCodeDriver::write_mcp_config(&bridge, None).unwrap();
        let written = std::fs::read_to_string(&path).unwrap();
        let _ = std::fs::remove_file(&path);
        let cfg: serde_json::Value = serde_json::from_str(&written).unwrap();
        assert!(cfg["mcpServers"]["librefang"].get("headers").is_none());
    }
}
