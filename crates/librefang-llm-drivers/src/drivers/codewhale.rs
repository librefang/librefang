//! CodeWhale CLI backend driver.
//!
//! Spawns the `codewhale` CLI (an open-source Rust terminal coding agent,
//! DeepSeek-V4-first and OpenAI-compatible) via its non-interactive
//! `exec --json` one-shot mode, which handles its own authentication and
//! provider/model resolution.
//!
//! Like the `codex-cli` driver, CodeWhale resolves its own model — its
//! `/model auto` route re-picks a model per turn — so the model the CLI
//! actually ran can differ from the requested id. The one-shot `--json`
//! summary reports that resolved model in its `model` field, which we surface
//! through [`CompletionResponse::actual_model`] so kernel-side metering
//! records reality rather than the nominated id.
//!
//! We use the **one-shot** path (`exec --json`, not `--auto`) so CodeWhale
//! performs a single model call rather than running its own tool-using agent
//! loop on top of LibreFang's — LibreFang already owns the agent loop and tool
//! surface; here CodeWhale is just the LLM backend.

use crate::llm_driver::{CompletionRequest, CompletionResponse, LlmDriver, LlmError};
use async_trait::async_trait;
use librefang_types::message::{ContentBlock, Role, StopReason, TokenUsage};
use serde::Deserialize;
use tracing::{debug, warn};

/// LLM driver that delegates to the CodeWhale CLI.
pub struct CodeWhaleDriver {
    cli_path: String,
    skip_permissions: bool,
    /// When `true` (the default), set `LIBREFANG_AGENT_ID`,
    /// `LIBREFANG_SESSION_ID`, and `LIBREFANG_STEP_ID` env vars on the spawned
    /// subprocess so operators can correlate process-tree entries with
    /// LibreFang agent sessions.
    emit_caller_trace_headers: bool,
}

/// One-shot summary emitted by `codewhale exec --json`.
///
/// Shape: `{"mode":"one-shot","model":"<id>","success":true,"output":"<text>"}`.
/// `output` carries the assistant text; `model` is the model CodeWhale
/// actually resolved (it may differ from the requested id under `/model auto`).
/// Older / alternate builds may name the text field `result`/`content`/`text`,
/// so we try each. All fields are optional so a partial or future-shaped
/// summary still deserializes and degrades gracefully.
#[derive(Debug, Deserialize, Default)]
struct CodeWhaleExecSummary {
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    output: Option<String>,
    #[serde(default)]
    result: Option<String>,
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    text: Option<String>,
    /// `false` marks a failed run even when the process exits 0.
    #[serde(default = "default_true")]
    success: bool,
    /// Populated on failure.
    #[serde(default)]
    error: Option<String>,
}

fn default_true() -> bool {
    true
}

impl CodeWhaleDriver {
    /// Create a new CodeWhale CLI driver.
    ///
    /// `cli_path` overrides the CLI binary path; defaults to `"codewhale"` on
    /// PATH. `skip_permissions` runs the CLI non-interactively (required for
    /// daemon mode) — CodeWhale's one-shot `exec` does not prompt, so this is
    /// informational parity with the other CLI drivers.
    pub fn new(cli_path: Option<String>, skip_permissions: bool) -> Self {
        Self {
            cli_path: cli_path
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "codewhale".to_string()),
            skip_permissions,
            emit_caller_trace_headers: true,
        }
    }

    /// Control whether caller-trace env vars are injected into the spawned
    /// subprocess.
    pub fn with_emit_caller_trace_headers(mut self, emit: bool) -> Self {
        self.emit_caller_trace_headers = emit;
        self
    }

    /// Inject caller-trace env vars into a subprocess command when the flag is
    /// on. Empty / `None` values are skipped.
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

    /// Detect if the CodeWhale CLI is available on PATH.
    pub fn detect() -> Option<String> {
        let output = std::process::Command::new("codewhale")
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

    /// Build the CLI arguments for a request.
    ///
    /// One-shot JSON mode: `exec --json [--model <m>] <prompt>`. The prompt is
    /// the trailing positional; CodeWhale's `exec` declares it with
    /// `allow_hyphen_values`, so a prompt that starts with `-` is not parsed as
    /// a flag.
    pub fn build_args(&self, prompt: &str, model: &str) -> Vec<String> {
        let mut args = vec!["exec".to_string(), "--json".to_string()];

        if let Some(m) = Self::model_flag(model) {
            args.push("--model".to_string());
            args.push(m);
        }

        args.push(prompt.to_string());
        args
    }

    /// Map a model id like `codewhale/deepseek-v4-pro` to the `--model` value.
    ///
    /// Returns `None` for the bare provider id (`codewhale`) or an empty
    /// string, so CodeWhale falls back to its configured default model rather
    /// than being handed a non-model token.
    fn model_flag(model: &str) -> Option<String> {
        let stripped = model.strip_prefix("codewhale/").unwrap_or(model);
        let stripped = stripped.trim();
        if stripped.is_empty() || stripped == "codewhale" {
            None
        } else {
            Some(stripped.to_string())
        }
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

    /// Extract assistant text from a parsed summary, trying the documented
    /// `output` field first then alternate field names.
    fn summary_text(summary: &CodeWhaleExecSummary) -> String {
        summary
            .output
            .clone()
            .or_else(|| summary.result.clone())
            .or_else(|| summary.content.clone())
            .or_else(|| summary.text.clone())
            .unwrap_or_default()
    }
}

#[async_trait]
impl LlmDriver for CodeWhaleDriver {
    #[tracing::instrument(
        name = "llm.complete",
        skip_all,
        fields(provider = "codewhale", model = %request.model)
    )]
    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        let prompt = Self::build_prompt(&request);
        let args = self.build_args(&prompt, &request.model);

        let mut cmd = tokio::process::Command::new(&self.cli_path);
        for arg in &args {
            cmd.arg(arg);
        }

        // CodeWhale is multi-provider (DeepSeek, OpenAI, OpenRouter, NVIDIA, …)
        // and reads whichever provider key is configured, so — like the aider
        // driver — we do not strip provider keys from the environment.
        if self.emit_caller_trace_headers {
            Self::apply_caller_trace_envs(&mut cmd, &request);
        }

        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());

        debug!(cli = %self.cli_path, skip_permissions = self.skip_permissions, "Spawning CodeWhale CLI");

        let output = cmd.output().await.map_err(|e| {
            LlmError::Http(format!(
                "CodeWhale CLI not found or failed to start ({e}). \
                 Install: npm install -g codewhale"
            ))
        })?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);

        if !output.status.success() {
            let detail = if !stderr.trim().is_empty() {
                stderr.trim()
            } else {
                stdout.trim()
            };
            let code = output.status.code().unwrap_or(1);

            let message = if detail.contains("not authenticated")
                || detail.contains("auth")
                || detail.contains("login")
                || detail.contains("credentials")
                || detail.contains("api key")
                || detail.contains("API key")
            {
                format!(
                    "CodeWhale CLI is not authenticated. Run: codewhale auth set --provider deepseek\nDetail: {detail}"
                )
            } else {
                format!("CodeWhale CLI exited with code {code}: {detail}")
            };

            return Err(LlmError::Api {
                status: code as u16,
                message,
                code: None,
            });
        }

        // Parse the one-shot `--json` summary. Recover the resolved model from
        // its `model` field (#6134-style actual-model reporting); fall back to
        // the requested id when the summary lacks one so attribution is never
        // empty.
        let trimmed = stdout.trim();
        if let Ok(summary) = serde_json::from_str::<CodeWhaleExecSummary>(trimmed) {
            let text = Self::summary_text(&summary);

            if !summary.success {
                let detail = summary.error.unwrap_or_else(|| {
                    if text.is_empty() {
                        "CodeWhale reported failure".to_string()
                    } else {
                        text.clone()
                    }
                });
                return Err(LlmError::Api {
                    status: 1,
                    message: format!("CodeWhale run failed: {detail}"),
                    code: None,
                });
            }

            let resolved_model = match summary.model {
                Some(m) if !m.trim().is_empty() => {
                    debug!(requested = %request.model, actual = %m, "CodeWhale resolved model");
                    m
                }
                _ => Self::model_flag(&request.model).unwrap_or_else(|| request.model.clone()),
            };

            return Ok(CompletionResponse {
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
            });
        }

        // Fallback: stdout was not the expected JSON summary. Treat it as plain
        // text and fall back to the requested model id for attribution.
        warn!("CodeWhale CLI output was not a JSON summary; treating stdout as plain text");
        let resolved_model =
            Self::model_flag(&request.model).unwrap_or_else(|| request.model.clone());
        Ok(CompletionResponse {
            content: vec![ContentBlock::Text {
                text: trimmed.to_string(),
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
        // CodeWhale is DeepSeek-first and OpenAI-wire-compatible.
        crate::llm_driver::LlmFamily::OpenAi
    }

    fn is_coding_agent(&self) -> bool {
        true
    }
}

/// Check if the CodeWhale CLI is available (binary on PATH or credentials
/// exist).
pub fn codewhale_available() -> bool {
    CodeWhaleDriver::detect().is_some() || codewhale_credentials_exist()
}

/// Check if CodeWhale credentials/config exist.
fn codewhale_credentials_exist() -> bool {
    if let Some(home) = home_dir() {
        let dir = home.join(".codewhale");
        dir.join("config.toml").exists() || dir.join("auth.json").exists()
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
        assert!(CodeWhaleDriver::new(None, false).is_coding_agent());
    }

    #[test]
    fn test_new_defaults() {
        let driver = CodeWhaleDriver::new(None, false);
        assert_eq!(driver.cli_path, "codewhale");
        assert!(!driver.skip_permissions);
    }

    #[test]
    fn test_new_with_empty_path() {
        let driver = CodeWhaleDriver::new(Some(String::new()), false);
        assert_eq!(driver.cli_path, "codewhale");
    }

    #[test]
    fn test_build_args_one_shot_json() {
        let driver = CodeWhaleDriver::new(None, true);
        let args = driver.build_args("hello prompt", "codewhale/deepseek-v4-pro");
        assert_eq!(args.first().map(String::as_str), Some("exec"));
        assert!(args.contains(&"--json".to_string()));
        assert!(args.contains(&"--model".to_string()));
        assert!(args.contains(&"deepseek-v4-pro".to_string()));
        assert_eq!(args.last().map(String::as_str), Some("hello prompt"));
    }

    #[test]
    fn test_model_flag_strips_prefix_and_defaults() {
        assert_eq!(
            CodeWhaleDriver::model_flag("codewhale/deepseek-v4-pro"),
            Some("deepseek-v4-pro".to_string())
        );
        assert_eq!(
            CodeWhaleDriver::model_flag("mimo-v2.5-pro"),
            Some("mimo-v2.5-pro".to_string())
        );
        // Bare provider id / empty → None so CodeWhale uses its own default.
        assert_eq!(CodeWhaleDriver::model_flag("codewhale"), None);
        assert_eq!(CodeWhaleDriver::model_flag(""), None);
    }

    #[test]
    fn test_parse_summary_extracts_model_and_output() {
        let json =
            r#"{"mode":"one-shot","model":"deepseek-v4-pro","success":true,"output":"hi there"}"#;
        let summary: CodeWhaleExecSummary = serde_json::from_str(json).unwrap();
        assert_eq!(summary.model.as_deref(), Some("deepseek-v4-pro"));
        assert!(summary.success);
        assert_eq!(CodeWhaleDriver::summary_text(&summary), "hi there");
    }

    #[test]
    fn test_parse_summary_failure_flag() {
        let json = r#"{"mode":"one-shot","success":false,"error":"boom"}"#;
        let summary: CodeWhaleExecSummary = serde_json::from_str(json).unwrap();
        assert!(!summary.success);
        assert_eq!(summary.error.as_deref(), Some("boom"));
    }

    #[test]
    fn test_summary_text_field_fallbacks() {
        // Alternate builds may use `result`/`content`/`text` instead of `output`.
        let s = CodeWhaleExecSummary {
            result: Some("from result".to_string()),
            ..Default::default()
        };
        assert_eq!(CodeWhaleDriver::summary_text(&s), "from result");
    }
}
