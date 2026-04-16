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
    pub fn build_args(&self, prompt: &str, model: &str) -> Vec<String> {
        let mut args = vec!["-p".to_string(), prompt.to_string()];

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

    /// Map a model ID like "gemini-cli/gemini-2.5-pro" to CLI --model flag value.
    fn model_flag(model: &str) -> Option<String> {
        let stripped = model.strip_prefix("gemini-cli/").unwrap_or(model);
        match stripped {
            "gemini-2.5-pro" | "pro" => Some("gemini-2.5-pro".to_string()),
            "gemini-2.5-flash" | "flash" => Some("gemini-2.5-flash".to_string()),
            _ => Some(stripped.to_string()),
        }
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
    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        let prompt = Self::build_prompt(&request);
        let args = self.build_args(&prompt, &request.model);

        let mut cmd = tokio::process::Command::new(&self.cli_path);
        for arg in &args {
            cmd.arg(arg);
        }

        Self::apply_env_filter(&mut cmd);

        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());

        debug!(cli = %self.cli_path, "Spawning Gemini CLI");

        let output = cmd.output().await.map_err(|e| {
            LlmError::Http(format!(
                "Gemini CLI not found or failed to start ({}). \
                 Install the Google Gemini CLI and run: gemini",
                e
            ))
        })?;

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
            });
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
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
fn gemini_cli_credentials_exist() -> bool {
    if let Some(home) = home_dir() {
        let gemini_dir = home.join(".gemini");
        gemini_dir.join("settings.json").exists() || gemini_dir.join(".credentials.json").exists()
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
    fn test_new_defaults() {
        let driver = GeminiCliDriver::new(None, false);
        assert_eq!(driver.cli_path, "gemini");
        assert!(!driver.skip_permissions);
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
        let args = driver.build_args("test prompt", "gemini-cli/gemini-2.5-pro");
        assert!(args.contains(&"-p".to_string()));
        assert!(args.contains(&"test prompt".to_string()));
        assert!(args.contains(&"--model".to_string()));
        assert!(args.contains(&"gemini-2.5-pro".to_string()));
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
}
