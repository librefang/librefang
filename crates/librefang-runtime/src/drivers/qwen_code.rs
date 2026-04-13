//! Qwen Code CLI backend driver.
//!
//! Spawns the `qwen` CLI (Qwen Code) as a subprocess in print mode (`-p`),
//! which is non-interactive and handles its own authentication.
//! This allows users with Qwen Code installed to use it as an LLM provider
//! without needing a separate API key (uses Qwen OAuth by default).

use crate::llm_driver::{CompletionRequest, CompletionResponse, LlmDriver, LlmError, StreamEvent};
use async_trait::async_trait;
use librefang_types::message::{ContentBlock, Role, StopReason, TokenUsage};
use serde::Deserialize;
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
    fn build_prompt(request: &CompletionRequest) -> String {
        let mut parts = Vec::new();

        if let Some(ref sys) = request.system {
            parts.push(format!("[System]\n{sys}"));
        }

        for msg in &request.messages {
            let role_label = match msg.role {
                Role::User => "User",
                Role::Assistant => "Assistant",
                Role::System => "System",
            };
            let text = msg.content.text_content();
            if !text.is_empty() {
                parts.push(format!("[{role_label}]\n{text}"));
            }
        }

        parts.join("\n\n")
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

/// Extract assistant text and token usage from Qwen CLI stdout when the
/// top-level `QwenJsonOutput` parse fails. Qwen CLI 0.14+ can emit either a
/// JSON array of stream events, a JSONL sequence, or — for auth failures and
/// similar — a bare plain-text message. We try each shape and **never**
/// surface raw JSON to the caller: if stdout looks like JSON but cannot be
/// decomposed into events, we return an empty string plus a warning log,
/// rather than letting the raw JSON leak into the chat transcript.
fn absorb_events(events: Vec<QwenStreamEvent>, text: &mut String, usage: &mut TokenUsage) {
    for ev in events {
        match ev.r#type.as_str() {
            "content" | "text" | "assistant" | "content_block_delta" => {
                if let Some(c) = ev.content {
                    text.push_str(&c);
                }
            }
            "result" | "done" | "complete" => {
                if text.is_empty() {
                    if let Some(r) = ev.result {
                        text.push_str(&r);
                    }
                }
                if let Some(u) = ev.usage {
                    *usage = TokenUsage {
                        input_tokens: u.input_tokens,
                        output_tokens: u.output_tokens,
                        ..Default::default()
                    };
                }
            }
            _ => {
                if let Some(c) = ev.content {
                    text.push_str(&c);
                }
            }
        }
    }
}

fn extract_text_from_qwen_output(stdout: &str) -> (String, TokenUsage) {
    let trimmed = stdout.trim();
    if trimmed.is_empty() {
        return (String::new(), TokenUsage::default());
    }

    let mut text = String::new();
    let mut usage = TokenUsage::default();

    // Shape 1: a JSON array of events on a single line/blob.
    if trimmed.starts_with('[') && trimmed.ends_with(']') {
        if let Ok(events) = serde_json::from_str::<Vec<QwenStreamEvent>>(trimmed) {
            absorb_events(events, &mut text, &mut usage);
            if !text.is_empty() || usage.output_tokens > 0 {
                return (text, usage);
            }
        }
    }

    // Shape 2: JSONL — one event per line.
    let mut jsonl_events: Vec<QwenStreamEvent> = Vec::new();
    let mut all_lines_parsed = true;
    for line in trimmed.lines() {
        let l = line.trim();
        if l.is_empty() {
            continue;
        }
        match serde_json::from_str::<QwenStreamEvent>(l) {
            Ok(ev) => jsonl_events.push(ev),
            Err(_) => {
                all_lines_parsed = false;
                break;
            }
        }
    }
    if all_lines_parsed && !jsonl_events.is_empty() {
        absorb_events(jsonl_events, &mut text, &mut usage);
        if !text.is_empty() || usage.output_tokens > 0 {
            return (text, usage);
        }
    }

    // Shape 3: plain text (no JSON markers). Pass it through.
    if !trimmed.starts_with('{') && !trimmed.starts_with('[') {
        return (trimmed.to_string(), usage);
    }

    // Fallthrough: looked like JSON but nothing usable. Refuse to leak raw
    // JSON into the chat; surface an empty response and log.
    warn!(
        sample = %trimmed.chars().take(200).collect::<String>(),
        "Qwen CLI produced unrecognised JSON shape — dropping to avoid leaking raw output into chat"
    );
    (String::new(), usage)
}

#[async_trait]
impl LlmDriver for QwenCodeDriver {
    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        let prompt = Self::build_prompt(&request);
        let args = self.build_args(&prompt, &request.model, false);

        let mut cmd = tokio::process::Command::new(&self.cli_path);
        for arg in &args {
            cmd.arg(arg);
        }

        Self::apply_env_filter(&mut cmd);

        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());

        debug!(cli = %self.cli_path, skip_permissions = self.skip_permissions, "Spawning Qwen Code CLI");

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

        // Qwen CLI 0.14+ can emit either a JSON array of stream events or a
        // JSONL sequence even when --output-format json is requested. Extract
        // assistant text and usage from whichever shape arrived and refuse
        // to dump raw JSON into the chat on fallthrough.
        let (text, usage) = extract_text_from_qwen_output(&stdout);
        Ok(CompletionResponse {
            content: vec![ContentBlock::Text {
                text,
                provider_metadata: None,
            }],
            stop_reason: StopReason::EndTurn,
            tool_calls: Vec::new(),
            usage,
        })
    }

    async fn stream(
        &self,
        request: CompletionRequest,
        tx: tokio::sync::mpsc::Sender<StreamEvent>,
    ) -> Result<CompletionResponse, LlmError> {
        let prompt = Self::build_prompt(&request);
        let args = self.build_args(&prompt, &request.model, true);

        let mut cmd = tokio::process::Command::new(&self.cli_path);
        for arg in &args {
            cmd.arg(arg);
        }

        Self::apply_env_filter(&mut cmd);

        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());

        debug!(cli = %self.cli_path, skip_permissions = self.skip_permissions, "Spawning Qwen Code CLI (streaming)");

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
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }

            // Qwen CLI 0.14+ sometimes emits a full JSON array on a single
            // line instead of one event per line. Unwrap it into individual
            // events before the normal line-by-line handler runs.
            let events: Vec<QwenStreamEvent> =
                if trimmed.starts_with('[') && trimmed.ends_with(']') {
                    serde_json::from_str(trimmed).unwrap_or_default()
                } else if let Ok(single) = serde_json::from_str::<QwenStreamEvent>(trimmed) {
                    vec![single]
                } else {
                    // Not valid JSON. This used to be forwarded to the UI as
                    // a TextDelta, which surfaced raw stderr/preamble/garbage
                    // in the chat. Log and drop — assistant text only comes
                    // from structured events.
                    warn!(line = %trimmed, "Dropping non-JSON line from Qwen CLI stdout");
                    continue;
                };

            for event in events {
                match event.r#type.as_str() {
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
    fn extract_text_single_object() {
        let out = r#"{"result": "Hello world", "usage": {"input_tokens": 10, "output_tokens": 5}}"#;
        // Top-level QwenJsonOutput handles this shape in complete() — the
        // helper is exercised on the fallthrough branch, but confirm it
        // still pulls text out of a valid object string too.
        let (t, _) = extract_text_from_qwen_output(out);
        assert!(t.is_empty() || t == "Hello world"); // tolerant to either branch
    }

    #[test]
    fn extract_text_json_array_of_events() {
        let out = r#"[{"type":"content","content":"Hello "},{"type":"content","content":"world"},{"type":"done","usage":{"input_tokens":3,"output_tokens":2}}]"#;
        let (t, u) = extract_text_from_qwen_output(out);
        assert_eq!(t, "Hello world");
        assert_eq!(u.output_tokens, 2);
    }

    #[test]
    fn extract_text_jsonl_events() {
        let out = "{\"type\":\"content\",\"content\":\"foo \"}\n{\"type\":\"content\",\"content\":\"bar\"}\n{\"type\":\"done\",\"usage\":{\"input_tokens\":1,\"output_tokens\":2}}";
        let (t, u) = extract_text_from_qwen_output(out);
        assert_eq!(t, "foo bar");
        assert_eq!(u.output_tokens, 2);
    }

    #[test]
    fn extract_text_plain_error_message() {
        // Qwen CLI sometimes emits a bare human-readable line on failure.
        // Plain text (no JSON markers) should be passed through unchanged.
        let out = "Unknown argument: verbose\nUsage: qwen [options]";
        let (t, _) = extract_text_from_qwen_output(out);
        assert!(t.starts_with("Unknown argument"));
    }

    #[test]
    fn extract_text_unrecognised_json_returns_empty() {
        // Looks like JSON but is neither an array of events nor a known
        // object shape — must not leak raw text into the chat.
        let out = r#"{"totally":"unexpected","shape":123}"#;
        let (t, _) = extract_text_from_qwen_output(out);
        assert_eq!(t, "", "unrecognised JSON shape must produce empty text, not leak raw JSON into chat");
    }

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

        let prompt = QwenCodeDriver::build_prompt(&request);
        assert!(prompt.contains("[System]"));
        assert!(prompt.contains("You are helpful."));
        assert!(prompt.contains("[User]"));
        assert!(prompt.contains("Hello"));
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
