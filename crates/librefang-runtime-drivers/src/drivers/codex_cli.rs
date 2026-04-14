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
}

impl CodexCliDriver {
    /// Create a new Codex CLI driver.
    ///
    /// `cli_path` overrides the CLI binary path; defaults to `"codex"` on PATH.
    /// `skip_permissions` adds `--full-auto` to the spawned command so that the CLI
    /// runs non-interactively (required for daemon mode).
    pub fn new(cli_path: Option<String>, skip_permissions: bool) -> Self {
        if skip_permissions {
            warn!(
                "Codex CLI driver: --full-auto enabled. \
                 The CLI will not prompt for tool approvals. \
                 LibreFang's own capability/RBAC system enforces access control."
            );
        }

        Self {
            cli_path: cli_path
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "codex".to_string()),
            skip_permissions,
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
        let mut args = vec!["exec".to_string()];

        if self.skip_permissions {
            args.push("--full-auto".to_string());
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

/// Check if the Codex CLI is available.
pub fn codex_cli_available() -> bool {
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
    fn test_build_args_with_full_auto() {
        let driver = CodexCliDriver::new(None, true);
        let args = driver.build_args("test prompt", "codex-cli/o4-mini");
        assert_eq!(args.first().map(String::as_str), Some("exec"));
        assert!(args.contains(&"test prompt".to_string()));
        assert!(args.contains(&"--full-auto".to_string()));
        assert!(args.contains(&"--model".to_string()));
        assert!(args.contains(&"o4-mini".to_string()));
    }

    #[test]
    fn test_build_args_without_full_auto() {
        let driver = CodexCliDriver::new(None, false);
        let args = driver.build_args("test prompt", "codex-cli/o3");
        assert!(!args.contains(&"--full-auto".to_string()));
        assert_eq!(args.first().map(String::as_str), Some("exec"));
        assert!(!args.contains(&"-q".to_string()));
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
    fn test_sensitive_env_list_coverage() {
        assert!(SENSITIVE_ENV_EXACT.contains(&"ANTHROPIC_API_KEY"));
        assert!(SENSITIVE_ENV_EXACT.contains(&"GEMINI_API_KEY"));
        assert!(SENSITIVE_ENV_EXACT.contains(&"GROQ_API_KEY"));
        // OPENAI_API_KEY should NOT be in the strip list (Codex needs it)
        assert!(!SENSITIVE_ENV_EXACT.contains(&"OPENAI_API_KEY"));
    }
}
