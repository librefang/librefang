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
//!   sessions don't bleed budget when the agent is despawned. Failure
//!   to delete is logged at WARN; the cached id is restored so a later
//!   `cleanup` retries (avoids leaking workspaces on transient
//!   network blips).
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
//!
//! ## Error sanitization
//!
//! Response bodies that surface in `ExecError` messages are capped at
//! `ERR_BODY_TRUNCATE` bytes and have any `Bearer <token>` substrings
//! stripped before they hit the public message. Full bodies go to
//! `tracing::debug!` only — operators who need to debug a 5xx still
//! have the raw payload, but the value never leaves the daemon's logs.

use crate::tool_exec_backend::{ExecError, ExecOutcome, ExecSpec, ToolExecBackend};
use async_trait::async_trait;
use librefang_types::tool_exec::{BackendKind, DaytonaBackendConfig};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;

/// Cap for response-body fragments that end up inside public
/// `ExecError` messages. Keep small — bodies past this go to
/// debug-level tracing only.
const ERR_BODY_TRUNCATE: usize = 256;

/// Cap for the `tracing::debug!` body — large but bounded so a
/// pathological 1 GiB response can't blow up logs.
const DEBUG_BODY_TRUNCATE: usize = 1024;

/// Daytona backend handle. Cheap to clone — owns the `reqwest::Client`
/// behind an `Arc` and the workspace-id cache behind an `RwLock` so
/// multiple `run_command` calls in flight reuse the same workspace.
pub struct DaytonaBackend {
    cfg: DaytonaBackendConfig,
    agent_id: String,
    client: reqwest::Client,
    /// Workspace-id cache. `RwLock<Option<String>>` so the common
    /// "already initialised" path takes a read lock (fast), and only
    /// the first `ensure_workspace` call (or a post-cleanup retry)
    /// takes the write lock. Double-checked locking keeps the write
    /// branch from racing.
    workspace_id: Arc<RwLock<Option<String>>>,
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
            workspace_id: Arc::new(RwLock::new(None)),
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
    /// Cached for the lifetime of this backend instance unless cleanup
    /// clears it (or fails and re-stores it).
    async fn ensure_workspace(&self) -> Result<String, ExecError> {
        // Fast path: read lock — no allocation, no contention with
        // other concurrent `ensure_workspace` calls once initialised.
        if let Some(id) = self.workspace_id.read().await.as_ref() {
            return Ok(id.clone());
        }

        // Slow path: take write lock and re-check. Another task may
        // have raced ahead while we were waiting.
        let mut guard = self.workspace_id.write().await;
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
        let timeout = Duration::from_secs(self.cfg.timeout_secs);
        let url = self.url("/api/workspaces");
        let send_fut = self.client.post(&url).bearer_auth(&key).json(&body).send();
        let parsed: WorkspaceResponse = tokio::time::timeout(timeout, async {
            let resp = send_fut
                .await
                .map_err(|e| ExecError::Connect(format!("daytona create workspace: {e}")))?;
            if !resp.status().is_success() {
                let status = resp.status();
                let raw = resp.text().await.unwrap_or_default();
                tracing::debug!(
                    %status, body = %truncate_for_log(&raw, DEBUG_BODY_TRUNCATE),
                    "daytona create workspace failed"
                );
                return Err(ExecError::Other(format!(
                    "daytona create workspace HTTP {status}: {}",
                    sanitize_error_body(&raw, ERR_BODY_TRUNCATE)
                )));
            }
            resp.json::<WorkspaceResponse>()
                .await
                .map_err(|e| ExecError::Other(format!("daytona response decode: {e}")))
        })
        .await
        .map_err(|_| {
            ExecError::Timeout(format!("daytona create after {}s", timeout.as_secs()))
        })??;

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
            // Reserved-env keys (LD_PRELOAD, DYLD_*) are scrubbed at
            // the trait boundary — they can never reach a remote shell.
            if crate::tool_exec_backend::is_reserved_env_key(k) {
                tracing::warn!(
                    key = %k,
                    "tool_exec/daytona: dropping reserved env key from remote command"
                );
                continue;
            }
            // #4677 review (refs #3823): refuse to forward the env var
            // that holds the daemon's Daytona auth token. The token
            // never *intentionally* lands on `ExecSpec::env` — Daytona
            // is an HTTP backend, the bearer goes in the
            // `Authorization` header, not in the remote shell — but if
            // an agent puts `cfg.api_key_env` on its env map (either
            // by accident or by design), prefixing it to the remote
            // command line would let a `printenv` / `echo $VAR` tool
            // call exfiltrate it. Drop it explicitly with a warning.
            if k == &self.cfg.api_key_env {
                tracing::warn!(
                    key = %k,
                    "tool_exec/daytona: refusing to forward daytona auth env var \
                     to remote command (would leak bearer token via remote shell)"
                );
                continue;
            }
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

        // Wrap the entire request lifecycle (send + status check + body
        // decode) in one timeout. Mirrors SSH's `timeout(total, do_exec)`
        // pattern; without it a server that streams headers fast but
        // then stalls on the body could block forever.
        let url = self.url(&format!("/api/workspaces/{workspace}/exec"));
        let parsed: ExecResponse = tokio::time::timeout(timeout, async {
            let resp = self
                .client
                .post(&url)
                .bearer_auth(&key)
                .json(&body)
                .send()
                .await
                .map_err(|e| ExecError::Connect(format!("daytona exec: {e}")))?;
            if !resp.status().is_success() {
                let status = resp.status();
                let raw = resp.text().await.unwrap_or_default();
                tracing::debug!(
                    %status, body = %truncate_for_log(&raw, DEBUG_BODY_TRUNCATE),
                    "daytona exec failed"
                );
                return Err(ExecError::Other(format!(
                    "daytona exec HTTP {status}: {}",
                    sanitize_error_body(&raw, ERR_BODY_TRUNCATE)
                )));
            }
            resp.json::<ExecResponse>()
                .await
                .map_err(|e| ExecError::Other(format!("daytona exec decode: {e}")))
        })
        .await
        .map_err(|_| ExecError::Timeout(format!("daytona exec after {}s", timeout.as_secs())))??;

        let mut stdout = parsed.stdout;
        let mut stderr = parsed.stderr;
        if let Some(cap) = spec.limits.max_output_bytes {
            stdout = crate::tool_exec_backend::truncate_to_cap(stdout, cap);
            stderr = crate::tool_exec_backend::truncate_to_cap(stderr, cap);
        }
        Ok(ExecOutcome {
            stdout,
            stderr,
            exit_code: parsed.exit_code,
            backend_id: Some(format!("daytona:{workspace}")),
        })
    }

    async fn cleanup(&self) -> Result<(), ExecError> {
        // Take the cached id under write lock so a parallel `run_command`
        // doesn't see a half-deleted workspace.
        let id = {
            let mut guard = self.workspace_id.write().await;
            match guard.take() {
                Some(id) => id,
                None => return Ok(()),
            }
        };
        let key = match self.api_key() {
            Ok(k) => k,
            Err(_) => {
                // No key means nothing we can do remotely; drop the cache
                // and return Ok so cleanup is idempotent on torn-down envs.
                return Ok(());
            }
        };
        let url = self.url(&format!("/api/workspaces/{id}"));
        match self.client.delete(&url).bearer_auth(&key).send().await {
            Ok(resp) if resp.status().is_success() => Ok(()),
            Ok(resp) => {
                let status = resp.status();
                let raw = resp.text().await.unwrap_or_default();
                tracing::warn!(
                    workspace = %id,
                    %status,
                    body = %truncate_for_log(&raw, DEBUG_BODY_TRUNCATE),
                    "daytona cleanup non-2xx; restoring cached id so a future cleanup retries"
                );
                // Re-store so a later cleanup attempt retries the
                // delete instead of silently leaking the workspace.
                *self.workspace_id.write().await = Some(id);
                Ok(())
            }
            Err(e) => {
                tracing::warn!(
                    workspace = %id,
                    error = %e,
                    "daytona cleanup transport error; restoring cached id"
                );
                *self.workspace_id.write().await = Some(id);
                Ok(())
            }
        }
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
///
/// Polish: collapse runs of `-` into a single dash, trim leading and
/// trailing dashes, and fall back to `"agent"` if the result is empty.
fn sanitize_id(agent_id: &str) -> String {
    let mut out = String::with_capacity(agent_id.len());
    let mut last_dash = false;
    for c in agent_id.chars() {
        let safe = if c.is_ascii_alphanumeric() || c == '-' {
            c
        } else {
            '-'
        };
        if safe == '-' {
            if last_dash {
                continue;
            }
            last_dash = true;
        } else {
            last_dash = false;
        }
        out.push(safe);
    }
    let trimmed = out.trim_matches('-');
    if trimmed.is_empty() {
        "agent".to_string()
    } else {
        trimmed.to_string()
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

/// Truncate a body for `tracing::debug!` use. Doesn't strip Bearer
/// tokens — debug logs are operator-controlled and the operator owns
/// the token anyway. Public-facing messages MUST go through
/// `sanitize_error_body`.
fn truncate_for_log(s: &str, cap: usize) -> String {
    if s.len() > cap {
        format!(
            "{}... [+{} bytes]",
            crate::str_utils::safe_truncate_str(s, cap),
            s.len() - cap
        )
    } else {
        s.to_string()
    }
}

/// Strip `Bearer <token>` substrings AND truncate.
///
/// Public error messages never contain raw response bodies past `cap`
/// chars; full body goes to `tracing::debug!` only. We do simple
/// substring scanning rather than pulling in the `regex` crate — the
/// pattern is fixed (`Bearer ` followed by non-whitespace).
fn sanitize_error_body(s: &str, cap: usize) -> String {
    let mut out = String::with_capacity(s.len().min(cap + 32));
    let mut rest = s;
    while let Some(idx) = rest.find("Bearer ") {
        out.push_str(&rest[..idx]);
        out.push_str("Bearer <redacted>");
        // Skip past the Bearer prefix, then to the next whitespace.
        let after = &rest[idx + "Bearer ".len()..];
        let advance = after.find(char::is_whitespace).unwrap_or(after.len());
        rest = &after[advance..];
    }
    out.push_str(rest);
    if out.len() > cap {
        let total = out.len();
        let safe = crate::str_utils::safe_truncate_str(&out, cap).to_string();
        format!("{safe}... [truncated, {total} total bytes]")
    } else {
        out
    }
}

// ---------------------------------------------------------------------------
// Tests — mock the HTTP server with a tokio listener so we don't need
// a live Daytona account.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg_for(api_url: String, api_key_env: String) -> DaytonaBackendConfig {
        DaytonaBackendConfig {
            api_url,
            api_key_env,
            image: "ubuntu:22.04".to_string(),
            timeout_secs: 5,
            workspace_prefix: "test".to_string(),
        }
    }

    #[test]
    fn kind_is_daytona() {
        let backend = DaytonaBackend::new(
            cfg_for(
                "https://example.invalid".into(),
                "LIBREFANG_DAYTONA_TEST_KEY_KIND".into(),
            ),
            "a".into(),
        );
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
    fn sanitize_id_collapses_runs() {
        assert_eq!(sanitize_id("---abc---"), "abc");
        assert_eq!(sanitize_id("a/b/c"), "a-b-c");
        assert_eq!(sanitize_id("a___b"), "a-b");
    }

    #[test]
    fn sanitize_id_falls_back_when_empty() {
        assert_eq!(sanitize_id(""), "agent");
        assert_eq!(sanitize_id("///"), "agent");
        assert_eq!(sanitize_id("---"), "agent");
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
    fn truncate_for_log_under_cap_unchanged() {
        assert_eq!(truncate_for_log("ok", 100), "ok");
    }

    #[test]
    fn truncate_for_log_over_cap_appends_marker() {
        let s = "x".repeat(2000);
        let out = truncate_for_log(&s, 100);
        assert!(out.contains("[+1900 bytes]"), "got: {out}");
    }

    #[test]
    fn sanitize_error_body_redacts_bearer() {
        let raw = "denied: Bearer dt_pat_abc123def something happened";
        let out = sanitize_error_body(raw, 200);
        assert!(out.contains("Bearer <redacted>"), "got: {out}");
        assert!(!out.contains("dt_pat_abc123def"), "got: {out}");
    }

    #[test]
    fn sanitize_error_body_truncates_long_bodies() {
        let raw = "x".repeat(2000);
        let out = sanitize_error_body(&raw, 256);
        assert!(out.contains("[truncated"), "got: {out}");
        // 256 chars + marker — overall length is bounded.
        assert!(out.len() < 2000, "should truncate; got len={}", out.len());
    }

    #[test]
    fn sanitize_error_body_handles_multiple_bearers() {
        let raw = "Bearer aaa Bearer bbb done";
        let out = sanitize_error_body(raw, 200);
        assert!(!out.contains("aaa"));
        assert!(!out.contains("bbb"));
        assert_eq!(out.matches("Bearer <redacted>").count(), 2);
    }

    /// L5: this test MUST use a unique env-var name so it cannot
    /// collide with other tests in the module that also touch
    /// `LIBREFANG_DAYTONA_TEST_KEY*`. Removed the `unsafe` block by
    /// using a name no other test references.
    #[tokio::test]
    async fn missing_api_key_env_errors() {
        const VAR: &str = "LIBREFANG_DAYTONA_TEST_KEY_MISSING_API_KEY_ENV";
        // SAFETY: The env-var name is unique to this test, and we
        // only `remove_var`. Even with parallel test execution, no
        // other test reads or writes this name.
        unsafe { std::env::remove_var(VAR) };
        let backend =
            DaytonaBackend::new(cfg_for("http://127.0.0.1:1".into(), VAR.into()), "a".into());
        match backend.run_command(ExecSpec::new("echo hi")).await {
            Err(ExecError::NotConfigured(msg)) => {
                assert!(msg.contains(VAR));
            }
            other => panic!("expected NotConfigured, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn upload_returns_unsupported() {
        let backend = DaytonaBackend::new(
            cfg_for(
                "http://127.0.0.1:1".into(),
                "LIBREFANG_DAYTONA_TEST_KEY_UPLOAD".into(),
            ),
            "a".into(),
        );
        match backend.upload("/tmp/x", b"hi").await {
            Err(ExecError::UnsupportedForBackend { backend, operation }) => {
                assert_eq!(backend, "daytona");
                assert_eq!(operation, "upload");
            }
            other => panic!("expected UnsupportedForBackend, got {other:?}"),
        }
    }
}
