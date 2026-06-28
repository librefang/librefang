//! Codex CLI backend driver.
//!
//! Spawns the `codex` CLI (OpenAI Codex CLI) via the non-interactive `exec`
//! subcommand, which handles its own authentication.
//! This allows users with Codex CLI installed to use it as an LLM provider
//! without needing additional configuration beyond OpenAI credentials.

use crate::llm_driver::{CompletionRequest, CompletionResponse, LlmDriver, LlmError};
use async_trait::async_trait;
use librefang_types::message::{ContentBlock, Role, StopReason, TokenUsage};
use tracing::{debug, warn};

/// Environment variable names to strip from the subprocess to prevent
/// leaking API keys from other providers.
const SENSITIVE_ENV_EXACT: &[&str] = &[
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
/// unless it starts with `OPENAI_` or `CODEX_`.
const SENSITIVE_SUFFIXES: &[&str] = &["_SECRET", "_TOKEN", "_PASSWORD"];

/// LLM driver that delegates to the Codex CLI.
pub struct CodexCliDriver {
    cli_path: String,
    skip_permissions: bool,
    /// When `true` (the default), set `LIBREFANG_AGENT_ID`, `LIBREFANG_SESSION_ID`,
    /// and `LIBREFANG_STEP_ID` env vars on the spawned subprocess so operators can
    /// correlate process-tree entries with LibreFang agent sessions.
    emit_caller_trace_headers: bool,
}

impl CodexCliDriver {
    /// Create a new Codex CLI driver.
    ///
    /// `cli_path` overrides the CLI binary path; defaults to `"codex"` on PATH.
    /// `skip_permissions` adds `--sandbox workspace-write` to the spawned command
    /// so that the CLI runs non-interactively (required for daemon mode).
    pub fn new(cli_path: Option<String>, skip_permissions: bool) -> Self {
        if skip_permissions {
            warn!(
                "Codex CLI driver: workspace-write sandbox enabled. \
                 The CLI will not prompt for tool approvals. \
                 LibreFang's own capability/RBAC system enforces access control."
            );
        }

        Self {
            cli_path: cli_path
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "codex".to_string()),
            skip_permissions,
            emit_caller_trace_headers: true,
        }
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

    /// Detect if the Codex CLI is available on PATH.
    pub fn detect() -> Option<String> {
        let output = std::process::Command::new("codex")
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

    /// Build the CLI arguments for a given request.
    pub fn build_args(&self, prompt: &str, model: &str) -> Vec<String> {
        // LibreFang can run as a desktop app or daemon whose current directory
        // is not a Git repository. Codex otherwise rejects non-interactive
        // execution before processing the prompt.
        let mut args = vec!["exec".to_string(), "--skip-git-repo-check".to_string()];

        if self.skip_permissions {
            args.push("--sandbox".to_string());
            args.push("workspace-write".to_string());
        }

        let model_flag = Self::model_flag(model);
        if let Some(ref m) = model_flag {
            args.push("--model".to_string());
            args.push(m.clone());
        }

        args.push(prompt.to_string());

        args
    }

    /// Build a text prompt from the completion request messages.
    fn build_prompt(request: &CompletionRequest) -> String {
        let mut parts = Vec::new();

        if let Some(ref sys) = request.system {
            parts.push(format!("[System]\n{sys}"));
        }

        for msg in request.messages.iter() {
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

    /// Map a model ID like "codex-cli/o4-mini" to CLI --model flag value.
    fn model_flag(model: &str) -> Option<String> {
        let stripped = model.strip_prefix("codex-cli/").unwrap_or(model);
        match stripped {
            "o4-mini" => Some("o4-mini".to_string()),
            "o3" => Some("o3".to_string()),
            "gpt-4.1" => Some("gpt-4.1".to_string()),
            _ => Some(stripped.to_string()),
        }
    }

    /// Strip ANSI CSI escape sequences (e.g. `\x1b[1m` … `\x1b[0m`) from a
    /// line. codex styles its banner keys with bold when it thinks the stream
    /// is a terminal; we read it from a pipe where that detection is usually
    /// off, but strip defensively so a `--color always` or future detection
    /// change can't break model extraction.
    fn strip_ansi(line: &str) -> String {
        let bytes = line.as_bytes();
        let mut out = String::with_capacity(line.len());
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] == 0x1b {
                // ESC: skip an optional '[' then everything up to and including
                // the final byte in 0x40..=0x7e (the CSI terminator, e.g. 'm').
                i += 1;
                if i < bytes.len() && bytes[i] == b'[' {
                    i += 1;
                    while i < bytes.len() && !(0x40..=0x7e).contains(&bytes[i]) {
                        i += 1;
                    }
                    if i < bytes.len() {
                        i += 1; // consume the terminator byte
                    }
                }
                continue;
            }
            // `line` is valid UTF-8 and ANSI bytes are ASCII; copy this byte's
            // full char to preserve multi-byte content (e.g. workdir paths).
            let ch_len = line[i..].chars().next().map(|c| c.len_utf8()).unwrap_or(1);
            out.push_str(&line[i..i + ch_len]);
            i += ch_len;
        }
        out
    }

    /// Extract the model codex actually resolved, from its stderr startup
    /// banner.
    ///
    /// `codex exec` prints a human-readable preamble to **stderr** (even when
    /// piped to a non-TTY), with a `model:` line, e.g.:
    ///
    /// ```text
    /// OpenAI Codex v0.114.0 (research preview)
    /// --------
    /// workdir: /path/to/repo
    /// model: gpt-5.3-codex
    /// provider: openai
    /// --------
    /// ```
    ///
    /// The `--json` event stream carries no model field (confirmed against
    /// codex-rs `exec_events.rs`; the request to add one was closed
    /// unimplemented), so the banner is the only authoritative source for the
    /// model the CLI actually used (librefang/librefang#6134). The summary is
    /// emitted by codex's `print_config_summary` via `eprintln!`, so it always
    /// lands on stderr regardless of `--json`.
    ///
    /// Tolerant by design: strips ANSI styling, scans for the first line whose
    /// key is `model` (case-insensitive), returns the trimmed value, and
    /// yields `None` on any banner shape we don't recognise so the caller can
    /// fall back to the requested model id. Never panics, never assumes a fixed
    /// line position.
    fn parse_model_from_banner(stderr: &str) -> Option<String> {
        for line in stderr.lines() {
            let clean = Self::strip_ansi(line);
            let trimmed = clean.trim();
            if let Some((key, value)) = trimmed.split_once(':') {
                if key.trim().eq_ignore_ascii_case("model") {
                    let model = value.trim();
                    if !model.is_empty() {
                        return Some(model.to_string());
                    }
                }
            }
        }
        None
    }

    /// Apply security env filtering to a command.
    fn apply_env_filter(cmd: &mut tokio::process::Command) {
        for key in SENSITIVE_ENV_EXACT {
            cmd.env_remove(key);
        }
        for (key, _) in std::env::vars() {
            if key.starts_with("OPENAI_") || key.starts_with("CODEX_") {
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

#[async_trait]
impl LlmDriver for CodexCliDriver {
    #[tracing::instrument(
        name = "llm.complete",
        skip_all,
        fields(provider = "codex_cli", model = %request.model)
    )]
    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        let prompt = Self::build_prompt(&request);
        let args = self.build_args(&prompt, &request.model);

        let mut cmd = tokio::process::Command::new(&self.cli_path);
        for arg in &args {
            cmd.arg(arg);
        }

        Self::apply_env_filter(&mut cmd);
        if self.emit_caller_trace_headers {
            Self::apply_caller_trace_envs(&mut cmd, &request);
        }

        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());

        debug!(cli = %self.cli_path, skip_permissions = self.skip_permissions, "Spawning Codex CLI");

        let output = cmd.output().await.map_err(|e| {
            LlmError::Http(format!(
                "Codex CLI not found or failed to start ({}). \
                 Install: npm install -g @openai/codex",
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
                format!(
                    "Codex CLI is not authenticated. Check your OpenAI credentials.\nDetail: {detail}"
                )
            } else {
                format!("Codex CLI exited with code {code}: {detail}")
            };

            return Err(LlmError::Api {
                status: code as u16,
                message,
                code: None,
            });
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let text = stdout.trim().to_string();

        // Recover the model codex actually resolved from its stderr startup
        // banner (#6134). codex's `--json` events carry no model field, but the
        // banner's `model:` line does — and stderr was already piped above.
        // Stamp it into `actual_model` so kernel-side metering records the real
        // model rather than the requested id. Degrade safely: fall back to the
        // requested model id when the banner can't be parsed so attribution is
        // never empty and the call never breaks.
        let stderr = String::from_utf8_lossy(&output.stderr);
        let resolved_model = match Self::parse_model_from_banner(&stderr) {
            Some(banner_model) => {
                debug!(requested = %request.model, actual = %banner_model, "Codex CLI resolved model");
                banner_model
            }
            None => Self::model_flag(&request.model).unwrap_or_else(|| request.model.clone()),
        };

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
            actual_provider: None,
            actual_model: Some(resolved_model),
        })
    }

    fn family(&self) -> crate::llm_driver::LlmFamily {
        crate::llm_driver::LlmFamily::OpenAi
    }

    fn is_coding_agent(&self) -> bool {
        true
    }
}

/// Check if the Codex CLI is available.
pub fn codex_cli_available() -> bool {
    if super::is_proxied_via_env(&["OPENAI_BASE_URL", "OPENAI_API_BASE"], &["api.openai.com"]) {
        return false;
    }
    CodexCliDriver::detect().is_some() || codex_cli_credentials_exist()
}

/// Check if Codex CLI credentials exist.
fn codex_cli_credentials_exist() -> bool {
    if let Some(home) = home_dir() {
        let codex_dir = home.join(".codex");
        codex_dir.join("auth.json").exists()
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
    fn is_coding_agent_is_true() {
        assert!(CodexCliDriver::new(None, false).is_coding_agent());
    }

    #[test]
    fn test_new_defaults() {
        let driver = CodexCliDriver::new(None, false);
        assert_eq!(driver.cli_path, "codex");
        assert!(!driver.skip_permissions);
    }

    #[test]
    fn test_new_with_custom_path() {
        let driver = CodexCliDriver::new(Some("/usr/local/bin/codex".to_string()), true);
        assert_eq!(driver.cli_path, "/usr/local/bin/codex");
        assert!(driver.skip_permissions);
    }

    #[test]
    fn test_new_with_empty_path() {
        let driver = CodexCliDriver::new(Some(String::new()), false);
        assert_eq!(driver.cli_path, "codex");
    }

    #[test]
    fn test_build_args_with_workspace_write_sandbox() {
        let driver = CodexCliDriver::new(None, true);
        let args = driver.build_args("test prompt", "codex-cli/o4-mini");
        assert_eq!(
            args,
            [
                "exec",
                "--skip-git-repo-check",
                "--sandbox",
                "workspace-write",
                "--model",
                "o4-mini",
                "test prompt",
            ]
        );
    }

    #[test]
    fn test_build_args_without_workspace_write_sandbox() {
        let driver = CodexCliDriver::new(None, false);
        let args = driver.build_args("test prompt", "codex-cli/o3");
        assert_eq!(
            args,
            [
                "exec",
                "--skip-git-repo-check",
                "--model",
                "o3",
                "test prompt",
            ]
        );
    }

    #[test]
    fn test_model_flag_mapping() {
        assert_eq!(
            CodexCliDriver::model_flag("codex-cli/o4-mini"),
            Some("o4-mini".to_string())
        );
        assert_eq!(
            CodexCliDriver::model_flag("codex-cli/o3"),
            Some("o3".to_string())
        );
        assert_eq!(
            CodexCliDriver::model_flag("codex-cli/gpt-4.1"),
            Some("gpt-4.1".to_string())
        );
        assert_eq!(
            CodexCliDriver::model_flag("custom-model"),
            Some("custom-model".to_string())
        );
    }

    #[test]
    fn test_parse_model_from_banner_extracts_model() {
        // Verbatim shape of codex exec's stderr startup banner. The `model:`
        // line sits mid-banner, so extraction must not depend on line position.
        let banner = "OpenAI Codex v0.114.0 (research preview)\n\
--------\n\
workdir: /Users/codex-user/Documents/repo\n\
model: gpt-5.3-codex\n\
provider: openai\n\
approval: never\n\
sandbox: read-only\n\
--------\n";
        assert_eq!(
            CodexCliDriver::parse_model_from_banner(banner),
            Some("gpt-5.3-codex".to_string())
        );
    }

    #[test]
    fn test_parse_model_from_banner_strips_ansi() {
        // codex bolds the banner keys with ANSI when it thinks the stream is a
        // terminal; the parser must see through `\x1b[1mmodel:\x1b[0m`.
        let banner = "\u{1b}[1mworkdir:\u{1b}[0m /repo\n\
\u{1b}[1mmodel:\u{1b}[0m o4-mini\n\
\u{1b}[1mprovider:\u{1b}[0m openai\n";
        assert_eq!(
            CodexCliDriver::parse_model_from_banner(banner),
            Some("o4-mini".to_string())
        );
    }

    #[test]
    fn test_parse_model_from_banner_absent_returns_none() {
        // No `model:` line → None, so the caller falls back to the requested
        // model id rather than fabricating one.
        let no_model = "OpenAI Codex v0.114.0\n\
--------\n\
workdir: /repo\n\
provider: openai\n\
--------\n";
        assert_eq!(CodexCliDriver::parse_model_from_banner(no_model), None);
        assert_eq!(CodexCliDriver::parse_model_from_banner(""), None);
        // A `model:` key with an empty value is also rejected.
        assert_eq!(CodexCliDriver::parse_model_from_banner("model: \n"), None);
    }

    #[test]
    fn test_sensitive_env_list_coverage() {
        assert!(SENSITIVE_ENV_EXACT.contains(&"ANTHROPIC_API_KEY"));
        assert!(SENSITIVE_ENV_EXACT.contains(&"GEMINI_API_KEY"));
        assert!(SENSITIVE_ENV_EXACT.contains(&"GROQ_API_KEY"));
        // OPENAI_API_KEY should NOT be in the strip list (Codex needs it)
        assert!(!SENSITIVE_ENV_EXACT.contains(&"OPENAI_API_KEY"));
    }

    #[test]
    fn test_caller_trace_envs_set_when_flag_on() {
        // apply_caller_trace_envs must set all three vars when all IDs are present.
        let mut cmd = tokio::process::Command::new("echo");
        let request = CompletionRequest {
            model: "codex-cli/o4-mini".to_string(),
            messages: std::sync::Arc::new(vec![]),
            tools: std::sync::Arc::new(vec![]),
            max_tokens: 1,
            temperature: 0.0,
            system: None,
            thinking: None,
            prompt_caching: false,
            cache_ttl: None,
            prompt_cache_strategy: None,
            response_format: None,
            timeout_secs: None,
            extra_body: None,
            agent_id: Some("agent-abc".to_string()),
            session_id: Some("sess-xyz".to_string()),
            step_id: Some("step-001".to_string()),
            reasoning_echo_policy: librefang_types::model_catalog::ReasoningEchoPolicy::default(),

            ..Default::default()
        };
        CodexCliDriver::apply_caller_trace_envs(&mut cmd, &request);
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
        // with_emit_caller_trace_headers(false) records the intent — the actual
        // env injection is skipped in complete() which we can't invoke without
        // a running binary. Verify the flag is stored correctly.
        let driver = CodexCliDriver::new(None, false).with_emit_caller_trace_headers(false);
        assert!(!driver.emit_caller_trace_headers);
    }

    #[test]
    fn test_caller_trace_envs_skips_empty_values() {
        // None / empty IDs must not produce env var entries on the command.
        let mut cmd = tokio::process::Command::new("echo");
        let request = CompletionRequest {
            model: "codex-cli/o4-mini".to_string(),
            messages: std::sync::Arc::new(vec![]),
            tools: std::sync::Arc::new(vec![]),
            max_tokens: 1,
            temperature: 0.0,
            system: None,
            thinking: None,
            prompt_caching: false,
            cache_ttl: None,
            prompt_cache_strategy: None,
            response_format: None,
            timeout_secs: None,
            extra_body: None,
            agent_id: None,
            session_id: Some(String::new()),
            step_id: None,
            reasoning_echo_policy: librefang_types::model_catalog::ReasoningEchoPolicy::default(),

            ..Default::default()
        };
        CodexCliDriver::apply_caller_trace_envs(&mut cmd, &request);
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
}
