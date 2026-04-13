//! Qwen Code CLI backend driver.
//!
//! Spawns the `qwen` CLI (Qwen Code) as a subprocess in print mode (`-p`),
//! which is non-interactive and handles its own authentication.
//! This allows users with Qwen Code installed to use it as an LLM provider
//! without needing a separate API key (uses Qwen OAuth by default).

use crate::llm_driver::{CompletionRequest, CompletionResponse, LlmDriver, LlmError, StreamEvent};
use async_trait::async_trait;
use base64::Engine;
use librefang_types::message::{ContentBlock, MessageContent, Role, StopReason, TokenUsage};
use serde::Deserialize;
use std::path::{Path, PathBuf};
use tokio::io::AsyncBufReadExt;
use tracing::{debug, warn};

/// Environment variable names to strip from the subprocess to prevent
/// leaking API keys from other providers.
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
/// unless it starts with `QWEN_`.
const SENSITIVE_SUFFIXES: &[&str] = &["_SECRET", "_TOKEN", "_PASSWORD"];

/// LLM driver that delegates to the Qwen Code CLI.
pub struct QwenCodeDriver {
    cli_path: String,
    skip_permissions: bool,
}

impl QwenCodeDriver {
    /// Create a new Qwen Code driver.
    ///
    /// `cli_path` overrides the CLI binary path; defaults to `"qwen"` on PATH.
    /// `skip_permissions` adds `--yolo` to the spawned command so that the CLI
    /// runs non-interactively (required for daemon mode).
    pub fn new(cli_path: Option<String>, skip_permissions: bool) -> Self {
        if skip_permissions {
            warn!(
                "Qwen Code driver: --yolo enabled. \
                 The CLI will not prompt for tool approvals. \
                 LibreFang's own capability/RBAC system enforces access control."
            );
        }

        Self {
            cli_path: cli_path
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "qwen".to_string()),
            skip_permissions,
        }
    }

    /// Detect if the Qwen Code CLI is available.
    ///
    /// Tries the bare `qwen` command first (standard PATH lookup), then falls
    /// back to common install locations that may not be on PATH when LibreFang
    /// runs as a daemon/service.
    pub fn detect() -> Option<String> {
        // 1. Try bare command on PATH.
        if let Some(version) = Self::try_cli("qwen") {
            return Some(version);
        }

        // 2. Try `which qwen` to resolve through shell aliases / env managers.
        if let Some(path) = Self::which("qwen") {
            if let Some(version) = Self::try_cli(&path) {
                return Some(version);
            }
        }

        // 3. Try common install locations (npm global, cargo, etc.).
        let candidates = Self::common_cli_paths("qwen");
        for candidate in &candidates {
            if let Some(version) = Self::try_cli(candidate) {
                return Some(version);
            }
        }

        None
    }

    /// Try to run a CLI binary and return its version string.
    fn try_cli(path: &str) -> Option<String> {
        let output = std::process::Command::new(path)
            .arg("--version")
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .output()
            .ok()?;

        if output.status.success() {
            Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
        } else {
            None
        }
    }

    /// Use `which` (Unix) or `where` (Windows) to resolve a binary path.
    fn which(name: &str) -> Option<String> {
        #[cfg(target_os = "windows")]
        let cmd = "where";
        #[cfg(not(target_os = "windows"))]
        let cmd = "which";

        let output = std::process::Command::new(cmd)
            .arg(name)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .output()
            .ok()?;

        if output.status.success() {
            let path = String::from_utf8_lossy(&output.stdout)
                .lines()
                .next()?
                .trim()
                .to_string();
            if !path.is_empty() {
                return Some(path);
            }
        }
        None
    }

    /// Return common install locations for a CLI binary.
    fn common_cli_paths(name: &str) -> Vec<String> {
        let mut paths = Vec::new();
        if let Some(home) = home_dir() {
            // npm global installs (nvm, fnm, volta, etc.)
            paths.push(
                home.join(".local")
                    .join("bin")
                    .join(name)
                    .to_string_lossy()
                    .to_string(),
            );
            paths.push(
                home.join(".nvm")
                    .join("versions")
                    .join("node")
                    .to_string_lossy()
                    .to_string(),
            );
            // Cargo-installed binaries
            paths.push(
                home.join(".cargo")
                    .join("bin")
                    .join(name)
                    .to_string_lossy()
                    .to_string(),
            );
        }

        // System-wide locations
        #[cfg(not(target_os = "windows"))]
        {
            paths.push(format!("/usr/local/bin/{name}"));
            paths.push(format!("/usr/bin/{name}"));
            paths.push(format!("/opt/homebrew/bin/{name}"));
        }

        #[cfg(target_os = "windows")]
        {
            if let Ok(appdata) = std::env::var("APPDATA") {
                paths.push(format!("{appdata}\\npm\\{name}.cmd"));
            }
        }

        paths
    }

    /// Build the CLI arguments for a given request.
    pub fn build_args(&self, prompt: &str, model: &str, streaming: bool) -> Vec<String> {
        let mut args = vec!["-p".to_string(), prompt.to_string()];

        args.push("--output-format".to_string());
        if streaming {
            args.push("stream-json".to_string());
            args.push("--include-partial-messages".to_string());
        } else {
            args.push("json".to_string());
        }

        if self.skip_permissions {
            args.push("--yolo".to_string());
        }

        let model_flag = Self::model_flag(model);
        if let Some(ref m) = model_flag {
            args.push("--model".to_string());
            args.push(m.clone());
        }

        args
    }

    /// Build a text prompt from the completion request messages.
    ///
    /// When messages contain image blocks, the images are decoded from base64
    /// (or referenced directly for `ImageFile` blocks), written to a temporary
    /// directory, and referenced by file path in the prompt text using the
    /// `@path` syntax recognized by Qwen Code CLI. The caller passes every
    /// directory in `read_dirs()` to `--add-dir` so the CLI sandbox can read
    /// those files. The owned temp dir is removed when the returned
    /// `PreparedPrompt` is dropped — no explicit cleanup call is required,
    /// which means the temp dir is also released if the future is cancelled
    /// mid-`await`.
    ///
    /// Note: the Qwen Code CLI forwards the files to the underlying model in
    /// the same way Claude Code does, but Qwen's coding-focused models
    /// (`qwen3-coder`, `qwen-coder-plus`) are text-only — only the Qwen-VL
    /// family (`qwen-vl-max`, `qwen2.5-vl-*`) will actually interpret the
    /// image payload. For text-only models the file path still reaches the
    /// CLI subprocess, so no information is silently dropped at the driver
    /// boundary.
    fn build_prompt(request: &CompletionRequest) -> PreparedPrompt {
        let mut parts = Vec::new();
        let mut image_dir: Option<PathBuf> = None;
        let mut extra_read_dirs: Vec<PathBuf> = Vec::new();
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
                                // Decode first — if the base64 is bad, we
                                // don't want to have created a temp dir or
                                // burned an image index slot.
                                let decoded = match base64::engine::general_purpose::STANDARD
                                    .decode(data)
                                {
                                    Ok(d) => d,
                                    Err(e) => {
                                        warn!(error = %e, "Failed to decode base64 image");
                                        continue;
                                    }
                                };

                                let dir = match image_dir.as_ref() {
                                    Some(d) => d.clone(),
                                    None => {
                                        let d = std::env::temp_dir().join(format!(
                                            "librefang-qwen-images-{}",
                                            uuid::Uuid::new_v4()
                                        ));
                                        if let Err(e) = std::fs::create_dir_all(&d) {
                                            warn!(
                                                error = %e,
                                                "Failed to create Qwen Code image temp dir"
                                            );
                                            continue;
                                        }
                                        image_dir = Some(d.clone());
                                        d
                                    }
                                };

                                let ext = match media_type.as_str() {
                                    "image/png" => "png",
                                    "image/gif" => "gif",
                                    "image/webp" => "webp",
                                    _ => "jpg",
                                };
                                // Use next index for the candidate filename,
                                // but only commit the increment once the file
                                // is successfully written. This keeps
                                // image-N.ext contiguous even if a previous
                                // decode/write failed earlier in the loop.
                                let next_index = image_count + 1;
                                let filename = format!("image-{next_index}.{ext}");
                                let path = dir.join(&filename);

                                if let Err(e) = std::fs::write(&path, &decoded) {
                                    warn!(
                                        error = %e,
                                        "Failed to write Qwen Code temp image"
                                    );
                                    continue;
                                }
                                image_count = next_index;
                                msg_parts.push(format!("@{}", display_cli_path(&path)));
                            }
                            ContentBlock::ImageFile { path, .. } => {
                                // ImageFile is already on disk — reference it
                                // directly without a temp copy, and whitelist
                                // its parent directory via --add-dir so the
                                // CLI sandbox can read it.
                                //
                                // Canonicalize so `@path` is absolute
                                // regardless of the CLI subprocess cwd. On
                                // Windows, strip the `\\?\` verbatim prefix
                                // that `canonicalize` adds, since the Qwen
                                // CLI's `@path` lexer does not understand it.
                                let canonical = match Path::new(path).canonicalize() {
                                    Ok(p) => p,
                                    Err(e) => {
                                        warn!(
                                            error = %e,
                                            path = %path,
                                            "ImageFile path missing or not canonicalizable, \
                                             skipping"
                                        );
                                        continue;
                                    }
                                };
                                if let Some(parent) = canonical.parent() {
                                    let parent = parent.to_path_buf();
                                    if !extra_read_dirs.contains(&parent) {
                                        extra_read_dirs.push(parent);
                                    }
                                }
                                msg_parts.push(format!("@{}", display_cli_path(&canonical)));
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
            extra_read_dirs,
        }
    }

    /// Map a model ID like "qwen-code/qwen3-coder" to CLI --model flag value.
    fn model_flag(model: &str) -> Option<String> {
        let stripped = model.strip_prefix("qwen-code/").unwrap_or(model);
        match stripped {
            "qwen3-coder" | "coder" => Some("qwen3-coder".to_string()),
            "qwen-coder-plus" | "coder-plus" => Some("qwen-coder-plus".to_string()),
            "qwq-32b" | "qwq" => Some("qwq-32b".to_string()),
            _ => Some(stripped.to_string()),
        }
    }

    /// Apply security env filtering to a command.
    fn apply_env_filter(cmd: &mut tokio::process::Command) {
        for key in SENSITIVE_ENV_EXACT {
            cmd.env_remove(key);
        }
        for (key, _) in std::env::vars() {
            if key.starts_with("QWEN_") {
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
}

/// Format an absolute path for handing to the Qwen CLI, either as an
/// `@path` token in the prompt or as a `--add-dir` argument.
///
/// On Windows, `Path::canonicalize` returns a verbatim path. The Qwen
/// CLI's path handling does not understand that prefix:
///   - `\\?\C:\Users\foo\pic.png` must become `C:\Users\foo\pic.png`
///   - `\\?\UNC\server\share\pic.png` must become `\\server\share\pic.png`
/// On non-Windows the canonical form is already plain, so this is a no-op.
fn display_cli_path(path: &Path) -> String {
    #[cfg(windows)]
    {
        let s = path.display().to_string();
        if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
            return format!(r"\\{rest}");
        }
        if let Some(rest) = s.strip_prefix(r"\\?\") {
            return rest.to_string();
        }
        s
    }
    #[cfg(not(windows))]
    {
        path.display().to_string()
    }
}

/// Prompt text plus optional temp directory containing decoded images.
///
/// Mirrors `claude_code::PreparedPrompt`: the driver decodes inline image
/// blocks onto disk, emits `@path` tokens in the prompt text, and hands the
/// directories back to the caller so they can be passed via `--add-dir`.
/// The owned temp dir is removed by the `Drop` impl, which means the temp
/// dir is released even if the driver future is cancelled mid-`await`.
struct PreparedPrompt {
    text: String,
    /// Temporary directory holding decoded image files, if any messages
    /// contained inline `ContentBlock::Image` blocks. `None` means the
    /// request had no inline images. Owned by this struct and removed on
    /// drop.
    image_dir: Option<PathBuf>,
    /// Additional directories that must be readable by the CLI because they
    /// contain `ContentBlock::ImageFile` paths referenced in the prompt.
    /// Not owned — never removed on drop.
    extra_read_dirs: Vec<PathBuf>,
}

impl PreparedPrompt {
    /// All directories that must be passed to the CLI via `--add-dir`.
    fn read_dirs(&self) -> impl Iterator<Item = &PathBuf> {
        self.image_dir.iter().chain(self.extra_read_dirs.iter())
    }
}

impl Drop for PreparedPrompt {
    /// Remove the temporary image directory, if any. Errors are logged at
    /// debug level and otherwise ignored — cleanup failure must never mask
    /// a real driver error.
    fn drop(&mut self) {
        if let Some(dir) = self.image_dir.take() {
            if let Err(e) = std::fs::remove_dir_all(&dir) {
                debug!(
                    error = %e,
                    dir = %dir.display(),
                    "Failed to clean up Qwen Code image temp dir"
                );
            }
        }
    }
}

/// JSON output from `qwen -p --output-format json`.
#[derive(Debug, Deserialize)]
struct QwenJsonOutput {
    result: Option<String>,
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    usage: Option<QwenUsage>,
    #[serde(default)]
    #[allow(dead_code)]
    cost_usd: Option<f64>,
}

/// Usage stats from Qwen CLI JSON output.
#[derive(Debug, Deserialize, Default)]
struct QwenUsage {
    #[serde(default)]
    input_tokens: u64,
    #[serde(default)]
    output_tokens: u64,
}

/// Stream JSON event from `qwen -p --output-format stream-json`.
#[derive(Debug, Deserialize)]
struct QwenStreamEvent {
    #[serde(default)]
    r#type: String,
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    result: Option<String>,
    #[serde(default)]
    usage: Option<QwenUsage>,
}

impl QwenCodeDriver {
    /// Non-streaming CLI invocation. Split out of `complete` so the caller
    /// can run cleanup on the `PreparedPrompt` regardless of success path.
    async fn complete_inner(
        &self,
        prepared: &PreparedPrompt,
        request: &CompletionRequest,
    ) -> Result<CompletionResponse, LlmError> {
        let args = self.build_args(&prepared.text, &request.model, false);

        let mut cmd = tokio::process::Command::new(&self.cli_path);
        for arg in &args {
            cmd.arg(arg);
        }

        // Allow the CLI to read every directory that backs an `@path` token
        // in the prompt: our owned temp dir for inline images, plus the
        // parents of any ImageFile blocks. Run paths through
        // display_cli_path so Windows verbatim prefixes are stripped — the
        // Qwen CLI rejects `\\?\` on both `@path` and `--add-dir`.
        for dir in prepared.read_dirs() {
            cmd.arg("--add-dir").arg(display_cli_path(dir));
        }

        Self::apply_env_filter(&mut cmd);

        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());

        debug!(
            cli = %self.cli_path,
            skip_permissions = self.skip_permissions,
            has_images = prepared.read_dirs().next().is_some(),
            "Spawning Qwen Code CLI"
        );

        let output = cmd.output().await.map_err(|e| {
            LlmError::Http(format!(
                "Qwen Code CLI not found or failed to start ({}). \
                 Install: npm install -g @qwen-code/qwen-code && qwen auth. \
                 If the CLI is installed in a non-standard location, set \
                 provider_urls.qwen-code in your LibreFang config.toml \
                 (e.g. provider_urls.qwen-code = \"/path/to/qwen\")",
                e
            ))
        })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
            let detail = if !stderr.is_empty() { &stderr } else { &stdout };
            let code = output.status.code().unwrap_or(1);

            let message = if detail.contains("not authenticated")
                || detail.contains("auth")
                || detail.contains("login")
                || detail.contains("credentials")
            {
                format!("Qwen Code CLI is not authenticated. Run: qwen auth\nDetail: {detail}")
            } else {
                format!("Qwen Code CLI exited with code {code}: {detail}")
            };

            return Err(LlmError::Api {
                status: code as u16,
                message,
            });
        }

        let stdout = String::from_utf8_lossy(&output.stdout);

        if let Ok(parsed) = serde_json::from_str::<QwenJsonOutput>(&stdout) {
            let text = parsed
                .result
                .or(parsed.content)
                .or(parsed.text)
                .unwrap_or_default();
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

        let text = stdout.trim().to_string();
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

    /// Streaming CLI invocation. Split out of `stream` so the caller can
    /// run cleanup on the `PreparedPrompt` regardless of success path.
    async fn stream_inner(
        &self,
        prepared: &PreparedPrompt,
        request: &CompletionRequest,
        tx: &tokio::sync::mpsc::Sender<StreamEvent>,
    ) -> Result<CompletionResponse, LlmError> {
        let args = self.build_args(&prepared.text, &request.model, true);

        let mut cmd = tokio::process::Command::new(&self.cli_path);
        for arg in &args {
            cmd.arg(arg);
        }

        // Allow the CLI to read every directory that backs an `@path` token
        // in the prompt: our owned temp dir for inline images, plus the
        // parents of any ImageFile blocks. Run paths through
        // display_cli_path so Windows verbatim prefixes are stripped — the
        // Qwen CLI rejects `\\?\` on both `@path` and `--add-dir`.
        for dir in prepared.read_dirs() {
            cmd.arg("--add-dir").arg(display_cli_path(dir));
        }

        Self::apply_env_filter(&mut cmd);

        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());

        debug!(
            cli = %self.cli_path,
            skip_permissions = self.skip_permissions,
            has_images = prepared.read_dirs().next().is_some(),
            "Spawning Qwen Code CLI (streaming)"
        );

        let mut child = cmd.spawn().map_err(|e| {
            LlmError::Http(format!(
                "Qwen Code CLI not found or failed to start ({}). \
                 Install: npm install -g @qwen-code/qwen-code && qwen auth. \
                 If the CLI is installed in a non-standard location, set \
                 provider_urls.qwen-code in your LibreFang config.toml \
                 (e.g. provider_urls.qwen-code = \"/path/to/qwen\")",
                e
            ))
        })?;

        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| LlmError::Http("No stdout from qwen CLI".to_string()))?;

        // Drain stderr in a background task to prevent deadlock when the
        // subprocess writes more than the OS pipe buffer can hold.
        let stderr = child.stderr.take();
        let stderr_handle = tokio::spawn(async move {
            let mut buf = String::new();
            if let Some(stderr) = stderr {
                let mut reader = tokio::io::BufReader::new(stderr);
                let _ = tokio::io::AsyncReadExt::read_to_string(&mut reader, &mut buf).await;
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

        while let Ok(Some(line)) = lines.next_line().await {
            if line.trim().is_empty() {
                continue;
            }

            match serde_json::from_str::<QwenStreamEvent>(&line) {
                Ok(event) => match event.r#type.as_str() {
                    "content" | "text" | "assistant" | "content_block_delta" => {
                        if let Some(ref content) = event.content {
                            full_text.push_str(content);
                            let _ = tx
                                .send(StreamEvent::TextDelta {
                                    text: content.clone(),
                                })
                                .await;
                        }
                    }
                    "result" | "done" | "complete" => {
                        if let Some(ref result) = event.result {
                            if full_text.is_empty() {
                                full_text = result.clone();
                                let _ = tx
                                    .send(StreamEvent::TextDelta {
                                        text: result.clone(),
                                    })
                                    .await;
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
                            let _ = tx
                                .send(StreamEvent::TextDelta {
                                    text: content.clone(),
                                })
                                .await;
                        }
                    }
                },
                Err(e) => {
                    warn!(line = %line, error = %e, "Non-JSON line from Qwen CLI");
                    full_text.push_str(&line);
                    let _ = tx.send(StreamEvent::TextDelta { text: line }).await;
                }
            }
        }

        let status = child
            .wait()
            .await
            .map_err(|e| LlmError::Http(format!("Qwen CLI wait failed: {e}")))?;

        let stderr_output = stderr_handle.await.unwrap_or_default();

        if !status.success() {
            let code = status.code().unwrap_or(1);
            let detail = if !stderr_output.trim().is_empty() {
                stderr_output.trim().to_string()
            } else if !full_text.is_empty() {
                full_text.clone()
            } else {
                "unknown error".to_string()
            };

            let message = if detail.contains("not authenticated")
                || detail.contains("auth")
                || detail.contains("login")
                || detail.contains("credentials")
            {
                format!("Qwen Code CLI is not authenticated. Run: qwen auth\nDetail: {detail}")
            } else {
                format!("Qwen Code CLI exited with code {code}: {detail}")
            };

            return Err(LlmError::Api {
                status: code as u16,
                message,
            });
        }

        if !stderr_output.trim().is_empty() {
            warn!(stderr = %stderr_output.trim(), "Qwen CLI stderr output");
        }

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

#[async_trait]
impl LlmDriver for QwenCodeDriver {
    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        // `prepared` cleans up its temp dir via `Drop`, so cancellation at
        // any await point below still releases the dir — no explicit
        // cleanup call needed.
        let prepared = Self::build_prompt(&request);
        self.complete_inner(&prepared, &request).await
    }

    async fn stream(
        &self,
        request: CompletionRequest,
        tx: tokio::sync::mpsc::Sender<StreamEvent>,
    ) -> Result<CompletionResponse, LlmError> {
        let prepared = Self::build_prompt(&request);
        self.stream_inner(&prepared, &request, &tx).await
    }
}

/// Check if the Qwen Code CLI is available.
///
/// Returns `true` if the CLI binary is found (via PATH or common install
/// locations) or if Qwen credentials files exist on disk.
pub fn qwen_code_available() -> bool {
    QwenCodeDriver::detect().is_some() || qwen_credentials_exist()
}

/// Check if Qwen credentials exist.
fn qwen_credentials_exist() -> bool {
    if let Some(home) = home_dir() {
        let qwen_dir = home.join(".qwen");
        qwen_dir.join("credentials.json").exists()
            || qwen_dir.join(".credentials.json").exists()
            || qwen_dir.join("auth.json").exists()
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
            model: "qwen-code/qwen3-coder".to_string(),
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

        let prepared = QwenCodeDriver::build_prompt(&request);
        assert!(prepared.text.contains("[System]"));
        assert!(prepared.text.contains("You are helpful."));
        assert!(prepared.text.contains("[User]"));
        assert!(prepared.text.contains("Hello"));
        // Pure-text request must not allocate a temp image dir.
        assert!(prepared.image_dir.is_none());
    }

    #[test]
    fn test_build_prompt_with_inline_image() {
        // Regression: a user message that contains a ContentBlock::Image
        // must produce an `@path` token in the prompt text and a populated
        // `image_dir` so the CLI wrapper can pass --add-dir. Before this
        // fix the driver stripped image blocks silently via text_content().
        use librefang_types::message::{Message, MessageContent};

        // A 1×1 transparent PNG encoded in base64 — minimal valid image.
        let tiny_png = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR4nGNgAAIAAAUAAen63NgAAAAASUVORK5CYII=";

        let request = CompletionRequest {
            model: "qwen-code/qwen-vl-max".to_string(),
            messages: vec![Message {
                role: Role::User,
                content: MessageContent::Blocks(vec![
                    ContentBlock::Text {
                        text: "What's in this image?".to_string(),
                        provider_metadata: None,
                    },
                    ContentBlock::Image {
                        media_type: "image/png".to_string(),
                        data: tiny_png.to_string(),
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

        let prepared = QwenCodeDriver::build_prompt(&request);

        // The text question survives.
        assert!(prepared.text.contains("What's in this image?"));

        // A temp dir was allocated and is exposed via read_dirs().
        let dir = prepared
            .image_dir
            .clone()
            .expect("image_dir must be Some when the request has an image block");
        assert!(dir.exists(), "temp dir must actually exist on disk");
        assert!(
            prepared.read_dirs().any(|d| d == &dir),
            "read_dirs() must include the owned temp dir so --add-dir is passed"
        );

        // The image landed in the temp dir as image-1.png.
        let image_path = dir.join("image-1.png");
        assert!(image_path.exists(), "decoded image file must exist");

        // The prompt text must reference the file via the @path convention
        // so Qwen Code CLI picks it up as a file attachment. On Windows the
        // prefix `\\?\` is stripped by display_cli_path, so assert against
        // that helper rather than raw Display.
        let expected_token = format!("@{}", display_cli_path(&image_path));
        assert!(
            prepared.text.contains(&expected_token),
            "prompt text must contain '{expected_token}' — got: {}",
            prepared.text
        );

        // Dropping `prepared` must remove the temp dir (Drop impl), so
        // cancelled futures don't leak disk space.
        drop(prepared);
        assert!(
            !dir.exists(),
            "Drop must remove the temp dir after the prompt is released"
        );
    }

    #[test]
    fn test_build_prompt_with_image_file_reference() {
        // ImageFile blocks reference a path already on disk; the driver
        // must emit `@path` without creating a temp copy.
        use librefang_types::message::{Message, MessageContent};

        // Create a real file so the existence check passes.
        let tmp = std::env::temp_dir().join(format!(
            "librefang-qwen-imagefile-test-{}.png",
            uuid::Uuid::new_v4()
        ));
        std::fs::write(&tmp, b"fake-png-bytes").expect("write tmp file");

        let request = CompletionRequest {
            model: "qwen-code/qwen-vl-max".to_string(),
            messages: vec![Message {
                role: Role::User,
                content: MessageContent::Blocks(vec![ContentBlock::ImageFile {
                    media_type: "image/png".to_string(),
                    path: tmp.display().to_string(),
                }]),
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

        let prepared = QwenCodeDriver::build_prompt(&request);

        // The emitted `@path` must use the canonicalized absolute path so
        // the CLI subprocess resolves it independent of cwd, with Windows
        // `\\?\` prefix stripped via display_cli_path.
        let canonical = std::fs::canonicalize(&tmp).expect("canonicalize tmp file");
        let expected_token = format!("@{}", display_cli_path(&canonical));
        assert!(
            prepared.text.contains(&expected_token),
            "prompt text must reference the canonical file path — got: {}",
            prepared.text
        );
        // ImageFile doesn't need a temp dir — the driver must not allocate one.
        assert!(
            prepared.image_dir.is_none(),
            "ImageFile block must not trigger temp dir allocation"
        );
        // The parent dir must be whitelisted for --add-dir so the CLI
        // sandbox can actually read the file.
        let parent = canonical.parent().expect("canonical has parent");
        assert!(
            prepared.read_dirs().any(|d| d.as_path() == parent),
            "read_dirs() must include the parent of the ImageFile so --add-dir covers it"
        );

        drop(prepared);
        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn test_model_flag_mapping() {
        assert_eq!(
            QwenCodeDriver::model_flag("qwen-code/qwen3-coder"),
            Some("qwen3-coder".to_string())
        );
        assert_eq!(
            QwenCodeDriver::model_flag("qwen-code/qwen-coder-plus"),
            Some("qwen-coder-plus".to_string())
        );
        assert_eq!(
            QwenCodeDriver::model_flag("qwen-code/qwq-32b"),
            Some("qwq-32b".to_string())
        );
        assert_eq!(
            QwenCodeDriver::model_flag("coder"),
            Some("qwen3-coder".to_string())
        );
        assert_eq!(
            QwenCodeDriver::model_flag("custom-model"),
            Some("custom-model".to_string())
        );
    }

    #[test]
    fn test_new_defaults_to_qwen() {
        let driver = QwenCodeDriver::new(None, true);
        assert_eq!(driver.cli_path, "qwen");
        assert!(driver.skip_permissions);
    }

    #[test]
    fn test_new_with_custom_path() {
        let driver = QwenCodeDriver::new(Some("/usr/local/bin/qwen".to_string()), true);
        assert_eq!(driver.cli_path, "/usr/local/bin/qwen");
    }

    #[test]
    fn test_new_with_empty_path() {
        let driver = QwenCodeDriver::new(Some(String::new()), true);
        assert_eq!(driver.cli_path, "qwen");
    }

    #[test]
    fn test_skip_permissions_disabled() {
        let driver = QwenCodeDriver::new(None, false);
        assert!(!driver.skip_permissions);
    }

    #[test]
    fn test_sensitive_env_list_coverage() {
        assert!(SENSITIVE_ENV_EXACT.contains(&"OPENAI_API_KEY"));
        assert!(SENSITIVE_ENV_EXACT.contains(&"ANTHROPIC_API_KEY"));
        assert!(SENSITIVE_ENV_EXACT.contains(&"GEMINI_API_KEY"));
        assert!(SENSITIVE_ENV_EXACT.contains(&"GROQ_API_KEY"));
        assert!(SENSITIVE_ENV_EXACT.contains(&"DEEPSEEK_API_KEY"));
    }

    #[test]
    fn test_build_args_with_yolo() {
        let driver = QwenCodeDriver::new(None, true);
        let args = driver.build_args("test prompt", "qwen-code/qwen3-coder", false);
        assert!(args.contains(&"--yolo".to_string()));
        assert!(args.contains(&"json".to_string()));
        assert!(args.contains(&"--model".to_string()));
    }

    #[test]
    fn test_build_args_without_yolo() {
        let driver = QwenCodeDriver::new(None, false);
        let args = driver.build_args("test prompt", "qwen-code/qwen3-coder", false);
        assert!(!args.contains(&"--yolo".to_string()));
    }

    #[test]
    fn test_build_args_streaming() {
        let driver = QwenCodeDriver::new(None, true);
        let args = driver.build_args("test prompt", "qwen-code/qwen3-coder", true);
        assert!(args.contains(&"stream-json".to_string()));
        assert!(args.contains(&"--include-partial-messages".to_string()));
    }

    #[test]
    fn test_json_output_deserialization() {
        let json = r#"{"result":"Hello world","usage":{"input_tokens":10,"output_tokens":5}}"#;
        let parsed: QwenJsonOutput = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.result.unwrap(), "Hello world");
        assert_eq!(parsed.usage.unwrap().input_tokens, 10);
    }

    #[test]
    fn test_json_output_content_field() {
        let json = r#"{"content":"Hello from content field"}"#;
        let parsed: QwenJsonOutput = serde_json::from_str(json).unwrap();
        assert!(parsed.result.is_none());
        assert_eq!(parsed.content.unwrap(), "Hello from content field");
    }

    #[test]
    fn test_stream_event_deserialization() {
        let json = r#"{"type":"content","content":"Hello"}"#;
        let event: QwenStreamEvent = serde_json::from_str(json).unwrap();
        assert_eq!(event.r#type, "content");
        assert_eq!(event.content.unwrap(), "Hello");
    }

    #[test]
    fn test_stream_event_result() {
        let json = r#"{"type":"result","result":"Final answer","usage":{"input_tokens":20,"output_tokens":10}}"#;
        let event: QwenStreamEvent = serde_json::from_str(json).unwrap();
        assert_eq!(event.r#type, "result");
        assert_eq!(event.result.unwrap(), "Final answer");
        assert_eq!(event.usage.unwrap().output_tokens, 10);
    }

    #[test]
    fn test_common_cli_paths_contains_standard_locations() {
        let paths = QwenCodeDriver::common_cli_paths("qwen");
        assert!(!paths.is_empty(), "should return at least some candidates");

        // On Unix, /usr/local/bin/qwen should be in the list.
        #[cfg(not(target_os = "windows"))]
        {
            assert!(
                paths.contains(&"/usr/local/bin/qwen".to_string()),
                "should include /usr/local/bin/qwen"
            );
            assert!(
                paths.contains(&"/usr/bin/qwen".to_string()),
                "should include /usr/bin/qwen"
            );
        }

        // Should include ~/.local/bin/qwen
        if let Some(home) = home_dir() {
            let local_bin = home
                .join(".local")
                .join("bin")
                .join("qwen")
                .to_string_lossy()
                .to_string();
            assert!(
                paths.contains(&local_bin),
                "should include ~/.local/bin/qwen"
            );
        }
    }

    #[test]
    fn test_try_cli_nonexistent_binary() {
        // A binary that definitely doesn't exist should return None.
        assert!(QwenCodeDriver::try_cli("__nonexistent_binary_12345__").is_none());
    }

    #[test]
    fn test_which_nonexistent_binary() {
        // `which` for a non-existent binary should return None.
        assert!(QwenCodeDriver::which("__nonexistent_binary_12345__").is_none());
    }
}
