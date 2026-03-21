//! Aider CLI backend driver.
//!
//! Spawns the `aider` CLI (Aider AI coding assistant) as a subprocess in
//! non-interactive mode (`--message`), which handles its own LLM provider
//! authentication via standard environment variables (OPENAI_API_KEY,
//! ANTHROPIC_API_KEY, etc.).

use crate::llm_driver::{CompletionRequest, CompletionResponse, LlmDriver, LlmError};
use async_trait::async_trait;
use librefang_types::message::{ContentBlock, Role, StopReason, TokenUsage};
use tracing::debug;

/// LLM driver that delegates to the Aider CLI.
pub struct AiderDriver {
    cli_path: String,
    #[allow(dead_code)]
    skip_permissions: bool,
}

impl AiderDriver {
    /// Create a new Aider driver.
    ///
    /// `cli_path` overrides the CLI binary path; defaults to `"aider"` on PATH.
    /// `skip_permissions` is accepted for interface consistency; Aider uses
    /// `--yes-always` which is always included in print mode args.
    pub fn new(cli_path: Option<String>, skip_permissions: bool) -> Self {
        Self {
            cli_path: cli_path
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "aider".to_string()),
            skip_permissions,
        }
    }

    /// Detect if the Aider CLI is available on PATH.
    pub fn detect() -> Option<String> {
        let output = std::process::Command::new("aider")
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
        let mut args = vec![
            "--message".to_string(),
            prompt.to_string(),
            "--yes-always".to_string(),
            "--no-auto-commits".to_string(),
            "--no-git".to_string(),
        ];

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

    /// Map a model ID like "aider/sonnet" to CLI --model flag value.
    /// Strips the "aider/" prefix and passes through.
    fn model_flag(model: &str) -> Option<String> {
        let stripped = model.strip_prefix("aider/").unwrap_or(model);
        Some(stripped.to_string())
    }
}

#[async_trait]
impl LlmDriver for AiderDriver {
    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        let prompt = Self::build_prompt(&request);
        let args = self.build_args(&prompt, &request.model);

        let mut cmd = tokio::process::Command::new(&self.cli_path);
        for arg in &args {
            cmd.arg(arg);
        }

        // Aider needs all provider API keys — don't strip any
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());

        debug!(cli = %self.cli_path, "Spawning Aider CLI");

        let output = cmd.output().await.map_err(|e| {
            LlmError::Http(format!(
                "Aider CLI not found or failed to start ({}). \
                 Install: pip install aider-chat",
                e
            ))
        })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
            let detail = if !stderr.is_empty() { &stderr } else { &stdout };
            let code = output.status.code().unwrap_or(1);

            let message = if detail.contains("not authenticated")
                || detail.contains("api key")
                || detail.contains("API key")
                || detail.contains("credentials")
            {
                format!(
                    "Aider CLI authentication failed. Ensure your LLM provider API keys \
                     are set (e.g. OPENAI_API_KEY, ANTHROPIC_API_KEY).\nDetail: {detail}"
                )
            } else {
                format!("Aider CLI exited with code {code}: {detail}")
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

/// Check if the Aider CLI is available.
pub fn aider_available() -> bool {
    AiderDriver::detect().is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_defaults() {
        let driver = AiderDriver::new(None, false);
        assert_eq!(driver.cli_path, "aider");
        assert!(!driver.skip_permissions);
    }

    #[test]
    fn test_new_with_custom_path() {
        let driver = AiderDriver::new(Some("/usr/local/bin/aider".to_string()), true);
        assert_eq!(driver.cli_path, "/usr/local/bin/aider");
    }

    #[test]
    fn test_new_with_empty_path() {
        let driver = AiderDriver::new(Some(String::new()), false);
        assert_eq!(driver.cli_path, "aider");
    }

    #[test]
    fn test_build_args() {
        let driver = AiderDriver::new(None, false);
        let args = driver.build_args("test prompt", "aider/sonnet");
        assert!(args.contains(&"--message".to_string()));
        assert!(args.contains(&"test prompt".to_string()));
        assert!(args.contains(&"--yes-always".to_string()));
        assert!(args.contains(&"--no-auto-commits".to_string()));
        assert!(args.contains(&"--no-git".to_string()));
        assert!(args.contains(&"--model".to_string()));
        assert!(args.contains(&"sonnet".to_string()));
    }

    #[test]
    fn test_model_flag_mapping() {
        assert_eq!(
            AiderDriver::model_flag("aider/sonnet"),
            Some("sonnet".to_string())
        );
        assert_eq!(
            AiderDriver::model_flag("aider/gpt-4o"),
            Some("gpt-4o".to_string())
        );
        assert_eq!(
            AiderDriver::model_flag("custom-model"),
            Some("custom-model".to_string())
        );
    }
}
