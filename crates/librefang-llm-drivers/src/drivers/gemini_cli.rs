//! Gemini CLI backend driver.
//!
//! Spawns the `gemini` CLI (Google Gemini CLI) as a subprocess in print mode (`-p`),
//! which is non-interactive and handles its own authentication.
//! This allows users with Gemini CLI installed to use it as an LLM provider
//! without needing a separate API key (uses Google OAuth by default).

use crate::llm_driver::{CompletionRequest, CompletionResponse, LlmDriver, LlmError};
use async_trait::async_trait;
use librefang_types::message::{ContentBlock, Role, StopReason, TokenUsage};
use tracing::debug;

/// Environment variable names to strip from the subprocess to prevent
/// leaking API keys from other providers.
const SENSITIVE_ENV_EXACT: &[&str] = &[
    "OPENAI_API_KEY",
    "ANTHROPIC_API_KEY",
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
/// unless it starts with `GEMINI_` or `GOOGLE_`.
const SENSITIVE_SUFFIXES: &[&str] = &["_SECRET", "_TOKEN", "_PASSWORD"];

/// LLM driver that delegates to the Gemini CLI.
pub struct GeminiCliDriver {
    cli_path: String,
    #[allow(dead_code)]
    skip_permissions: bool,
    message_timeout_secs: u64,
    /// When `true` (the default), set `LIBREFANG_AGENT_ID`, `LIBREFANG_SESSION_ID`,
    /// and `LIBREFANG_STEP_ID` env vars on the spawned subprocess so operators can
    /// correlate process-tree entries with LibreFang agent sessions.
    emit_caller_trace_headers: bool,
}

impl GeminiCliDriver {
    /// Create a new Gemini CLI driver.
    ///
    /// `cli_path` overrides the CLI binary path; defaults to `"gemini"` on PATH.
    /// `skip_permissions` is accepted for interface consistency but Gemini CLI
    /// does not have a tool-approval mechanism.
    pub fn new(cli_path: Option<String>, skip_permissions: bool) -> Self {
        Self {
            cli_path: cli_path
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "gemini".to_string()),
            skip_permissions,
            message_timeout_secs: crate::cli_process::DEFAULT_MESSAGE_TIMEOUT_SECS,
            emit_caller_trace_headers: true,
        }
    }

    /// Set the default subprocess deadline.
    /// A per-request timeout overrides it.
    pub fn with_message_timeout(mut self, timeout_secs: u64) -> Self {
        self.message_timeout_secs = timeout_secs;
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

    /// Detect if the Gemini CLI is available on PATH.
    pub fn detect() -> Option<String> {
        let output = std::process::Command::new("gemini")
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
    pub fn build_args(&self, model: &str) -> Vec<String> {
        let mut args = vec!["--output-format".to_string(), "json".to_string()];

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

    /// Map a model ID like "gemini-cli/gemini-2.5-pro" to CLI --model flag value.
    fn model_flag(model: &str) -> Option<String> {
        let stripped = model.strip_prefix("gemini-cli/").unwrap_or(model).trim();
        // Bare id → omit --model so the CLI uses its own configured default.
        if stripped.is_empty() || stripped == "gemini-cli" {
            return None;
        }
        match stripped {
            "gemini-2.5-pro" | "pro" => Some("gemini-2.5-pro".to_string()),
            "gemini-2.5-flash" | "flash" => Some("gemini-2.5-flash".to_string()),
            _ => Some(stripped.to_string()),
        }
    }

    /// Parse the single JSON object produced by `gemini --output-format json`.
    ///
    /// Gemini CLI reports session-wide metrics per model. Aggregate every
    /// model entry because a single agent turn may route work across multiple
    /// models. Thoughts are billable output tokens, matching the Gemini API
    /// driver's metering convention.
    fn parse_json_output(stdout: &str) -> Result<(String, TokenUsage), String> {
        let output: serde_json::Value = serde_json::from_str(stdout)
            .map_err(|error| format!("invalid Gemini CLI JSON output: {error}"))?;

        let text = output
            .get("response")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| "Gemini CLI JSON output did not include a response".to_string())?
            .to_string();

        let mut usage = TokenUsage::default();
        if let Some(models) = output
            .pointer("/stats/models")
            .and_then(serde_json::Value::as_object)
        {
            for model in models.values() {
                let tokens = model.get("tokens").unwrap_or(&serde_json::Value::Null);
                usage.input_tokens = usage.input_tokens.saturating_add(
                    tokens
                        .get("prompt")
                        .and_then(serde_json::Value::as_u64)
                        .unwrap_or(0),
                );
                usage.output_tokens = usage
                    .output_tokens
                    .saturating_add(
                        tokens
                            .get("candidates")
                            .and_then(serde_json::Value::as_u64)
                            .unwrap_or(0),
                    )
                    .saturating_add(
                        tokens
                            .get("thoughts")
                            .and_then(serde_json::Value::as_u64)
                            .unwrap_or(0),
                    );
                usage.cache_read_input_tokens = usage.cache_read_input_tokens.saturating_add(
                    tokens
                        .get("cached")
                        .and_then(serde_json::Value::as_u64)
                        .unwrap_or(0),
                );
            }
        }

        Ok((text, usage))
    }

    /// Apply security env filtering to a command.
    fn apply_env_filter(cmd: &mut tokio::process::Command) {
        for key in SENSITIVE_ENV_EXACT {
            cmd.env_remove(key);
        }
        for (key, _) in std::env::vars() {
            if key.starts_with("GEMINI_") || key.starts_with("GOOGLE_") {
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
impl LlmDriver for GeminiCliDriver {
    #[tracing::instrument(
        name = "llm.complete",
        skip_all,
        fields(provider = "gemini_cli", model = %request.model)
    )]
    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        let prompt = Self::build_prompt(&request);
        let args = self.build_args(&request.model);

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

        debug!(cli = %self.cli_path, "Spawning Gemini CLI");

        let timeout_secs = request.timeout_secs.unwrap_or(self.message_timeout_secs);
        let output = match crate::cli_process::output_with_input_timeout(
            &mut cmd,
            prompt.as_bytes(),
            std::time::Duration::from_secs(timeout_secs),
        )
        .await
        {
            Ok(output) => output,
            Err(crate::cli_process::OutputError::TimedOut) => {
                return Err(crate::cli_process::timeout_error(
                    timeout_secs,
                    "Gemini CLI",
                ));
            }
            Err(crate::cli_process::OutputError::Spawn(e)) => {
                return Err(LlmError::Http(format!(
                    "Gemini CLI not found or failed to start ({e}). \
                     Install the Google Gemini CLI and run: gemini"
                )));
            }
            Err(crate::cli_process::OutputError::Io(e)) => {
                return Err(LlmError::Http(format!(
                    "Gemini CLI subprocess failed after starting: {e}"
                )));
            }
        };

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
            let detail = if !stderr.is_empty() { &stderr } else { &stdout };
            let code = output.status.code().unwrap_or(1);

            // Check quota/rate-limit BEFORE auth — Gemini CLI's error output
            // for quota exhaustion contains "credentials" (from "Loaded cached
            // credentials") which would false-positive the auth check.
            let lower = detail.to_lowercase();
            if lower.contains("exhausted your capacity")
                || lower.contains("quota")
                || lower.contains("rate limit")
                || lower.contains("too many requests")
                || lower.contains("429")
            {
                return Err(LlmError::RateLimited {
                    retry_after_ms: 60_000,
                    message: Some(format!("Gemini quota exhausted: {detail}")),
                });
            }

            let message = if lower.contains("not authenticated") || lower.contains("login required")
            {
                format!("Gemini CLI is not authenticated. Run: gemini auth\nDetail: {detail}")
            } else {
                format!("Gemini CLI exited with code {code}: {detail}")
            };

            return Err(LlmError::Api {
                status: code as u16,
                message,
                code: None,
            });
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let (text, usage) = Self::parse_json_output(&stdout).map_err(|message| LlmError::Api {
            status: 502,
            message,
            code: None,
        })?;

        Ok(CompletionResponse {
            content: vec![ContentBlock::Text {
                text,
                provider_metadata: None,
            }],
            stop_reason: StopReason::EndTurn,
            tool_calls: Vec::new(),
            usage,
            actual_provider: None,
            actual_model: None,
        })
    }

    fn family(&self) -> crate::llm_driver::LlmFamily {
        crate::llm_driver::LlmFamily::Google
    }

    fn is_coding_agent(&self) -> bool {
        true
    }
}

/// Check if the Gemini CLI is available.
pub fn gemini_cli_available() -> bool {
    if super::is_proxied_via_env(
        &["GEMINI_API_BASE", "GOOGLE_AI_BASE_URL"],
        &[
            "generativelanguage.googleapis.com",
            "aiplatform.googleapis.com",
        ],
    ) {
        return false;
    }
    GeminiCliDriver::detect().is_some() || gemini_cli_credentials_exist()
}

/// Check if Gemini CLI credentials exist.
///
/// Only looks for actual credential files. `settings.json` is intentionally
/// excluded: it is a CLI preference file (theme, default model) that is
/// created on first launch even when the user is not logged in, so treating
/// it as proof of authentication marks Gemini as "configured" for anyone who
/// merely installed the CLI.
fn gemini_cli_credentials_exist() -> bool {
    home_dir()
        .map(|h| credentials_in_dir(&h.join(".gemini")))
        .unwrap_or(false)
}

/// Check a given directory for Gemini CLI credential files.
///
/// Recognised filenames:
/// - `oauth_creds.json` — Google Gemini CLI's actual OAuth token file
/// - `credentials.json` / `.credentials.json` — defensive fallbacks
fn credentials_in_dir(dir: &std::path::Path) -> bool {
    dir.join("oauth_creds.json").exists()
        || dir.join("credentials.json").exists()
        || dir.join(".credentials.json").exists()
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
        assert!(GeminiCliDriver::new(None, false).is_coding_agent());
    }

    #[test]
    fn test_new_defaults() {
        let driver = GeminiCliDriver::new(None, false);
        assert_eq!(driver.cli_path, "gemini");
        assert!(!driver.skip_permissions);
        assert_eq!(
            driver.message_timeout_secs,
            crate::cli_process::DEFAULT_MESSAGE_TIMEOUT_SECS
        );
    }

    #[test]
    fn with_message_timeout_overrides_default() {
        let driver = GeminiCliDriver::new(None, false).with_message_timeout(19);
        assert_eq!(driver.message_timeout_secs, 19);
    }

    #[cfg(unix)]
    fn sleeping_cli() -> tempfile::TempPath {
        use std::os::unix::fs::PermissionsExt;

        let mut file = tempfile::NamedTempFile::new().unwrap();
        std::io::Write::write_all(&mut file, b"#!/bin/sh\nsleep 30\n").unwrap();
        let mut permissions = file.as_file().metadata().unwrap().permissions();
        permissions.set_mode(0o700);
        file.as_file().set_permissions(permissions).unwrap();
        // Convert to a `TempPath` (file still on disk, deleted on drop) so
        // this process no longer holds the file open for writing. Spawning
        // the path directly as a subprocess otherwise fails with `ETXTBSY`
        // ("Text file busy") because Linux refuses to exec a file that has
        // a writable fd open anywhere, including in the exec-ing process.
        file.into_temp_path()
    }

    #[cfg(unix)]
    fn json_cli() -> tempfile::TempPath {
        use std::os::unix::fs::PermissionsExt;

        let mut file = tempfile::NamedTempFile::new().unwrap();
        std::io::Write::write_all(
            &mut file,
            br##"#!/bin/sh
printf '%s\n' '{"response":"hello from gemini","stats":{"models":{"gemini-test":{"tokens":{"prompt":42,"candidates":7,"cached":12,"thoughts":3}}}}}'
"##,
        )
        .unwrap();
        let mut permissions = file.as_file().metadata().unwrap().permissions();
        permissions.set_mode(0o700);
        file.as_file().set_permissions(permissions).unwrap();
        file.into_temp_path()
    }

    #[cfg(unix)]
    fn stdin_cli() -> tempfile::TempPath {
        use std::os::unix::fs::PermissionsExt;

        let mut file = tempfile::NamedTempFile::new().unwrap();
        std::io::Write::write_all(
            &mut file,
            br##"#!/bin/sh
case " $* " in
  *private-prompt*) exit 8 ;;
esac
prompt=$(cat)
case "$prompt" in
  *private-prompt*) ;;
  *) exit 9 ;;
esac
printf '%s\n' '{"response":"stdin received","stats":{"models":{"gemini-test":{"tokens":{"prompt":2,"candidates":3}}}}}'
"##,
        )
        .unwrap();
        let mut permissions = file.as_file().metadata().unwrap().permissions();
        permissions.set_mode(0o700);
        file.as_file().set_permissions(permissions).unwrap();
        file.into_temp_path()
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn complete_honors_request_timeout() {
        let cli = sleeping_cli();
        let driver = GeminiCliDriver::new(Some(cli.to_string_lossy().into_owned()), false);
        let request = CompletionRequest {
            model: "gemini-cli".to_string(),
            timeout_secs: Some(0),
            ..Default::default()
        };

        let error = driver.complete(request).await.unwrap_err();

        assert!(matches!(
            error,
            LlmError::TimedOut {
                inactivity_secs: 0,
                ..
            }
        ));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn complete_returns_json_message_and_usage() {
        let cli = json_cli();
        let driver = GeminiCliDriver::new(Some(cli.to_string_lossy().into_owned()), false);
        let response = driver
            .complete(CompletionRequest {
                model: "gemini-cli".to_string(),
                ..Default::default()
            })
            .await
            .unwrap();

        assert!(matches!(
            &response.content[0],
            ContentBlock::Text { text, .. } if text == "hello from gemini"
        ));
        assert_eq!(response.usage.input_tokens, 42);
        assert_eq!(response.usage.output_tokens, 10);
        assert_eq!(response.usage.cache_read_input_tokens, 12);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn complete_pipes_prompt_without_putting_it_in_argv() {
        let cli = stdin_cli();
        let driver = GeminiCliDriver::new(Some(cli.to_string_lossy().into_owned()), false);
        let response = driver
            .complete(CompletionRequest {
                model: "gemini-cli".to_string(),
                system: Some("private-prompt".to_string()),
                ..Default::default()
            })
            .await
            .unwrap();

        assert!(matches!(
            &response.content[0],
            ContentBlock::Text { text, .. } if text == "stdin received"
        ));
    }

    #[test]
    fn test_new_with_custom_path() {
        let driver = GeminiCliDriver::new(Some("/usr/local/bin/gemini".to_string()), true);
        assert_eq!(driver.cli_path, "/usr/local/bin/gemini");
    }

    #[test]
    fn test_new_with_empty_path() {
        let driver = GeminiCliDriver::new(Some(String::new()), false);
        assert_eq!(driver.cli_path, "gemini");
    }

    #[test]
    fn test_build_args() {
        let driver = GeminiCliDriver::new(None, false);
        let args = driver.build_args("gemini-cli/gemini-2.5-pro");
        assert!(!args.contains(&"-p".to_string()));
        assert!(args.contains(&"--model".to_string()));
        assert!(args.contains(&"gemini-2.5-pro".to_string()));
        assert!(args
            .windows(2)
            .any(|pair| pair == ["--output-format", "json"]));
    }

    #[test]
    fn parse_json_output_aggregates_model_usage() {
        let output = r#"{
            "response": "final answer",
            "stats": {"models": {
                "gemini-pro": {"tokens": {"prompt": 100, "candidates": 10, "cached": 80, "thoughts": 20}},
                "gemini-flash": {"tokens": {"prompt": 30, "candidates": 5, "cached": 0, "thoughts": 2}}
            }}
        }"#;

        let (text, usage) = GeminiCliDriver::parse_json_output(output).unwrap();

        assert_eq!(text, "final answer");
        assert_eq!(usage.input_tokens, 130);
        assert_eq!(usage.output_tokens, 37);
        assert_eq!(usage.cache_read_input_tokens, 80);
    }

    #[test]
    fn parse_json_output_rejects_malformed_or_missing_response() {
        assert!(GeminiCliDriver::parse_json_output("not-json").is_err());
        let error = GeminiCliDriver::parse_json_output(r#"{"stats": {}}"#).unwrap_err();
        assert!(error.contains("response"));
    }

    #[test]
    fn test_model_flag_bare_id_yields_none() {
        // Bare provider id / empty → None so `--model` is omitted and the CLI
        // uses its own configured default instead of a rejected placeholder.
        assert_eq!(GeminiCliDriver::model_flag("gemini-cli"), None);
        assert_eq!(GeminiCliDriver::model_flag("gemini-cli/"), None);
        assert_eq!(GeminiCliDriver::model_flag("  "), None);
    }

    #[test]
    fn test_model_flag_mapping() {
        assert_eq!(
            GeminiCliDriver::model_flag("gemini-cli/gemini-2.5-pro"),
            Some("gemini-2.5-pro".to_string())
        );
        assert_eq!(
            GeminiCliDriver::model_flag("gemini-cli/gemini-2.5-flash"),
            Some("gemini-2.5-flash".to_string())
        );
        assert_eq!(
            GeminiCliDriver::model_flag("pro"),
            Some("gemini-2.5-pro".to_string())
        );
        assert_eq!(
            GeminiCliDriver::model_flag("flash"),
            Some("gemini-2.5-flash".to_string())
        );
        assert_eq!(
            GeminiCliDriver::model_flag("custom-model"),
            Some("custom-model".to_string())
        );
    }

    #[test]
    fn test_sensitive_env_list_coverage() {
        assert!(SENSITIVE_ENV_EXACT.contains(&"OPENAI_API_KEY"));
        assert!(SENSITIVE_ENV_EXACT.contains(&"ANTHROPIC_API_KEY"));
        assert!(SENSITIVE_ENV_EXACT.contains(&"GROQ_API_KEY"));
        assert!(SENSITIVE_ENV_EXACT.contains(&"DEEPSEEK_API_KEY"));
    }

    fn make_tmp_dir(label: &str) -> std::path::PathBuf {
        let p = std::env::temp_dir().join(format!(
            "librefang-test-gemini-{label}-{}-{}",
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
        // `settings.json` is the CLI's preference file — it is created the
        // first time `gemini` runs, even when the user never signs in.
        // Treating it as a credential caused LibreFang to auto-mark Gemini
        // as "configured" for anyone who had merely installed the CLI.
        let dir = make_tmp_dir("settings-only");
        std::fs::write(dir.join("settings.json"), "{}").unwrap();
        assert!(
            !credentials_in_dir(&dir),
            "settings.json must not be treated as a Gemini credential"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn oauth_creds_json_is_recognised() {
        let dir = make_tmp_dir("oauth-creds");
        std::fs::write(dir.join("oauth_creds.json"), "{}").unwrap();
        assert!(credentials_in_dir(&dir));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn credentials_json_variants_are_recognised() {
        for name in ["credentials.json", ".credentials.json"] {
            let dir = make_tmp_dir(&format!("creds-{name}"));
            std::fs::write(dir.join(name), "{}").unwrap();
            assert!(credentials_in_dir(&dir), "{name} should be recognised");
            std::fs::remove_dir_all(&dir).unwrap();
        }
    }

    #[test]
    fn empty_dir_has_no_credentials() {
        let dir = make_tmp_dir("empty");
        assert!(!credentials_in_dir(&dir));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn test_caller_trace_envs_set_when_flag_on() {
        // apply_caller_trace_envs must set all three vars when all IDs are present.
        let mut cmd = tokio::process::Command::new("echo");
        let request = CompletionRequest {
            model: "gemini-cli/gemini-2.5-pro".to_string(),
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
        GeminiCliDriver::apply_caller_trace_envs(&mut cmd, &request);
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
        let driver = GeminiCliDriver::new(None, false).with_emit_caller_trace_headers(false);
        assert!(!driver.emit_caller_trace_headers);
    }

    #[test]
    fn test_caller_trace_envs_skips_empty_values() {
        // None / empty IDs must not produce env var entries on the command.
        let mut cmd = tokio::process::Command::new("echo");
        let request = CompletionRequest {
            model: "gemini-cli/gemini-2.5-pro".to_string(),
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
        GeminiCliDriver::apply_caller_trace_envs(&mut cmd, &request);
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
