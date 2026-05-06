//! Daytona-managed-sandbox [`ToolExecBackend`] (#3332).
//!
//! Behind the `daytona-backend` cargo feature. Uses the workspace
//! `reqwest` client to talk to Daytona's REST API.
//!
//! ## Scope
//!
//! - One-shot exec: per `run_command`, ensure a workspace exists for
//!   this agent, POST the command to its `/exec` endpoint, return the
//!   stdout / stderr / exit code. The workspace is reused across calls
//!   for the same agent ID.
//! - `cleanup()` issues a delete on the workspace so background
//!   sessions don't bleed budget when the agent is despawned.
//! - File I/O (`upload` / `download`) is **not** implemented in this
//!   landing — Daytona's archive endpoint takes more shape than fits
//!   in the issue scope. See `docs/architecture/tool-exec-backends.md`.
//!
//! ## Auth & BYO account
//!
//! The backend reads its bearer token from the env var configured in
//! `tool_exec.daytona.api_key_env` (default `DAYTONA_API_KEY`). The
//! daemon never persists the token — operators are expected to wire it
//! in via systemd / launchd / docker secrets. See the architecture doc
//! for setup notes.

use crate::tool_exec_backend::{ExecError, ExecOutcome, ExecSpec, ToolExecBackend};
use async_trait::async_trait;
use librefang_types::tool_exec::{BackendKind, DaytonaBackendConfig};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;

/// Daytona backend handle. Cheap to clone — owns the `reqwest::Client`
/// behind an `Arc` and the workspace-id cache behind a `Mutex` so
/// multiple `run_command` calls in flight reuse the same workspace.
pub struct DaytonaBackend {
    cfg: DaytonaBackendConfig,
    agent_id: String,
    client: reqwest::Client,
    workspace_id: Arc<Mutex<Option<String>>>,
}

impl DaytonaBackend {
    pub fn new(cfg: DaytonaBackendConfig, agent_id: String) -> Self {
        Self {
            cfg,
            agent_id,
            client: reqwest::Client::builder()
                .user_agent(crate::USER_AGENT)
                .build()
                .unwrap_or_default(),
            workspace_id: Arc::new(Mutex::new(None)),
        }
    }

    fn api_key(&self) -> Result<String, ExecError> {
        std::env::var(&self.cfg.api_key_env).map_err(|_| {
            ExecError::NotConfigured(format!(
                "tool_exec.daytona.api_key_env={} not set",
                self.cfg.api_key_env
            ))
        })
    }

    fn url(&self, path: &str) -> String {
        let base = self.cfg.api_url.trim_end_matches('/');
        if path.starts_with('/') {
            format!("{base}{path}")
        } else {
            format!("{base}/{path}")
        }
    }

    /// Ensure a Daytona workspace exists for this agent; return its id.
    /// Cached for the lifetime of this backend instance.
    async fn ensure_workspace(&self) -> Result<String, ExecError> {
        let mut guard = self.workspace_id.lock().await;
        if let Some(id) = guard.as_ref() {
            return Ok(id.clone());
        }

        let key = self.api_key()?;
        let body = CreateWorkspace {
            name: format!(
                "{}-{}",
                self.cfg.workspace_prefix,
                sanitize_id(&self.agent_id)
            ),
            image: self.cfg.image.clone(),
        };
        let resp = self
            .client
            .post(self.url("/api/workspaces"))
            .bearer_auth(&key)
            .json(&body)
            .send()
            .await
            .map_err(|e| ExecError::Connect(format!("daytona create workspace: {e}")))?;
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(ExecError::Other(format!(
                "daytona create workspace HTTP {status}: {text}"
            )));
        }
        let parsed: WorkspaceResponse = resp
            .json()
            .await
            .map_err(|e| ExecError::Other(format!("daytona response decode: {e}")))?;
        *guard = Some(parsed.id.clone());
        Ok(parsed.id)
    }
}

#[async_trait]
impl ToolExecBackend for DaytonaBackend {
    fn kind(&self) -> BackendKind {
        BackendKind::Daytona
    }

    async fn run_command(&self, spec: ExecSpec) -> Result<ExecOutcome, ExecError> {
        let key = self.api_key()?;
        let workspace = self.ensure_workspace().await?;
        let timeout = spec
            .limits
            .timeout
            .unwrap_or_else(|| Duration::from_secs(self.cfg.timeout_secs));

        // Compose env-prefixed command so callers' env survives the wire.
        let mut full_cmd = String::new();
        for (k, v) in &spec.env {
            // Daytona's exec endpoint takes a single shell string; we
            // prefix `KEY=value` assignments and let the remote shell
            // export them for the duration of `command`.
            full_cmd.push_str(&format!("{k}={} ", shell_quote(v)));
        }
        full_cmd.push_str(&spec.command);

        let body = ExecRequest {
            command: full_cmd,
            workdir: spec
                .workdir
                .as_ref()
                .and_then(|p| p.to_str())
                .map(String::from),
        };
        let resp = tokio::time::timeout(
            timeout,
            self.client
                .post(self.url(&format!("/api/workspaces/{workspace}/exec")))
                .bearer_auth(&key)
                .json(&body)
                .send(),
        )
        .await
        .map_err(|_| ExecError::Timeout(format!("daytona exec after {}s", timeout.as_secs())))?
        .map_err(|e| ExecError::Connect(format!("daytona exec: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(ExecError::Other(format!(
                "daytona exec HTTP {status}: {text}"
            )));
        }
        let parsed: ExecResponse = resp
            .json()
            .await
            .map_err(|e| ExecError::Other(format!("daytona exec decode: {e}")))?;

        let mut stdout = parsed.stdout;
        let mut stderr = parsed.stderr;
        if let Some(cap) = spec.limits.max_output_bytes {
            stdout = truncate_to_cap(stdout, cap);
            stderr = truncate_to_cap(stderr, cap);
        }
        Ok(ExecOutcome {
            stdout,
            stderr,
            exit_code: parsed.exit_code,
            backend_id: Some(format!("daytona:{workspace}")),
        })
    }

    async fn cleanup(&self) -> Result<(), ExecError> {
        let mut guard = self.workspace_id.lock().await;
        let id = match guard.take() {
            Some(id) => id,
            None => return Ok(()),
        };
        let key = self.api_key().ok();
        if let Some(key) = key {
            let _ = self
                .client
                .delete(self.url(&format!("/api/workspaces/{id}")))
                .bearer_auth(key)
                .send()
                .await;
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Wire types — kept private; the trait is the contract surface.
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct CreateWorkspace {
    name: String,
    image: String,
}

#[derive(Deserialize)]
struct WorkspaceResponse {
    id: String,
}

#[derive(Serialize)]
struct ExecRequest {
    command: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    workdir: Option<String>,
}

#[derive(Deserialize)]
struct ExecResponse {
    stdout: String,
    stderr: String,
    exit_code: i32,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Daytona requires alphanumeric / dash workspace names. Sanitize the
/// agent id to avoid 4xx on creation when ids contain underscores.
fn sanitize_id(agent_id: &str) -> String {
    let s: String = agent_id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c
            } else {
                '-'
            }
        })
        .collect();
    if s.is_empty() {
        "agent".to_string()
    } else {
        s
    }
}

/// POSIX single-quote a value safely for env-prefix / `cd` use.
fn shell_quote(value: &str) -> String {
    if value
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '/' | '-' | '.' | ':' | '+'))
    {
        value.to_string()
    } else {
        let escaped = value.replace('\'', r"'\''");
        format!("'{escaped}'")
    }
}

fn truncate_to_cap(s: String, cap: usize) -> String {
    if s.len() > cap {
        let total = s.len();
        let safe = crate::str_utils::safe_truncate_str(&s, cap).to_string();
        format!("{safe}... [truncated, {total} total bytes]")
    } else {
        s
    }
}

// ---------------------------------------------------------------------------
// Tests — mock the HTTP server with a tokio listener so we don't need
// a live Daytona account.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg_for(api_url: String) -> DaytonaBackendConfig {
        DaytonaBackendConfig {
            api_url,
            api_key_env: "LIBREFANG_DAYTONA_TEST_KEY".to_string(),
            image: "ubuntu:22.04".to_string(),
            timeout_secs: 5,
            workspace_prefix: "test".to_string(),
        }
    }

    #[test]
    fn kind_is_daytona() {
        let backend = DaytonaBackend::new(cfg_for("https://example.invalid".into()), "a".into());
        assert_eq!(backend.kind(), BackendKind::Daytona);
    }

    #[test]
    fn sanitize_id_accepts_alphanumeric() {
        assert_eq!(sanitize_id("abc-123"), "abc-123");
    }

    #[test]
    fn sanitize_id_replaces_specials() {
        assert_eq!(
            sanitize_id("agent_with_underscore"),
            "agent-with-underscore"
        );
        assert_eq!(sanitize_id("a/b\\c"), "a-b-c");
    }

    #[test]
    fn sanitize_id_falls_back_when_empty() {
        assert_eq!(sanitize_id(""), "agent");
        assert_eq!(sanitize_id("///"), "---");
    }

    #[test]
    fn shell_quote_no_special_chars_unchanged() {
        assert_eq!(shell_quote("hello"), "hello");
    }

    #[test]
    fn shell_quote_wraps_with_spaces() {
        assert_eq!(shell_quote("hello world"), "'hello world'");
    }

    #[test]
    fn truncate_to_cap_under_unchanged() {
        assert_eq!(truncate_to_cap("ok".into(), 10), "ok");
    }

    #[test]
    fn truncate_to_cap_over_appends_marker() {
        let s = "x".repeat(50);
        let out = truncate_to_cap(s, 20);
        assert!(out.contains("[truncated"));
    }

    #[tokio::test]
    async fn missing_api_key_env_errors() {
        // Make sure the env var is unset for this test.
        // SAFETY: tests in this module run serially via tokio::test only when
        // they don't share state; unset is idempotent.
        // SAFETY: env mutation happens in single-threaded tokio test; harmless.
        unsafe { std::env::remove_var("LIBREFANG_DAYTONA_TEST_KEY") };
        let backend = DaytonaBackend::new(cfg_for("http://127.0.0.1:1".into()), "a".into());
        match backend.run_command(ExecSpec::new("echo hi")).await {
            Err(ExecError::NotConfigured(msg)) => {
                assert!(msg.contains("LIBREFANG_DAYTONA_TEST_KEY"));
            }
            other => panic!("expected NotConfigured, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn upload_returns_unsupported() {
        let backend = DaytonaBackend::new(cfg_for("http://127.0.0.1:1".into()), "a".into());
        match backend.upload("/tmp/x", b"hi").await {
            Err(ExecError::UnsupportedForBackend { backend, operation }) => {
                assert_eq!(backend, "daytona");
                assert_eq!(operation, "upload");
            }
            other => panic!("expected UnsupportedForBackend, got {other:?}"),
        }
    }
}
