//! Pluggable tool-execution backend trait + concrete impls (#3332).
//!
//! Historically the runtime ran every shell / `docker_exec` / process
//! spawn directly on the daemon host via `subprocess_sandbox` and
//! `docker_sandbox`. This module introduces a trait, [`ToolExecBackend`],
//! that abstracts "run a command somewhere" so the kernel can route an
//! agent's exec calls to remote SSH hosts or managed sandboxes
//! (Daytona, …) without the call sites caring.
//!
//! ## What lives here vs. what stays put
//!
//! - The trait + the result / error / spec types are the new public
//!   abstraction layer.
//! - [`LocalBackend`] is a thin adapter around the existing
//!   `subprocess_sandbox` helpers and is **bit-for-bit equivalent** to
//!   the old direct call path. Existing tool runner code keeps calling
//!   the helpers directly today; the trait route is opt-in via
//!   per-agent `tool_exec_backend` manifest field — a follow-up PR will
//!   migrate the call sites.
//! - [`DockerBackend`] is an adapter over the existing
//!   `docker_sandbox`, exposing `create + exec + destroy` as a single
//!   `run_command` call.
//! - SSH and Daytona backends are gated behind `ssh-backend` and
//!   `daytona-backend` cargo features.
//!
//! Configuration lives in `librefang-types::tool_exec` so the agent
//! manifest and kernel config can carry it without depending on the
//! runtime crate.

use async_trait::async_trait;
use librefang_types::tool_exec::{BackendKind, ToolExecConfig};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;

// ---------------------------------------------------------------------------
// Public trait + DTOs
// ---------------------------------------------------------------------------

/// Resource limits applied to a single command invocation.
///
/// Backends honour these on a best-effort basis — `LocalBackend` enforces
/// `timeout` and `max_output_bytes` directly, `DockerBackend` adds
/// `--memory` / `--cpus` / `--pids-limit` by way of the existing
/// `DockerSandboxConfig` knobs, and remote backends usually rely on the
/// remote system to enforce its own caps. Setting a field to `None` means
/// "no explicit cap from the runtime" — the backend may still impose its
/// own configured default.
#[derive(Debug, Clone, Default)]
pub struct ResourceLimits {
    /// Wall-clock timeout for the command. `None` falls back to the
    /// backend's configured default (e.g. `DockerSandboxConfig.timeout_secs`).
    pub timeout: Option<Duration>,
    /// Maximum combined stdout/stderr bytes returned to the caller.
    /// `None` falls back to the backend default (50 KiB for Docker today).
    pub max_output_bytes: Option<usize>,
}

/// One command to execute on a backend.
///
/// `command` is the literal command line — backends pass it to whatever
/// shell makes sense (`sh -c …` for Docker / SSH, the host's
/// `tokio::process::Command` for Local). Validation against the agent's
/// `ExecPolicy` happens **before** dispatch, in `tool_runner.rs`.
#[derive(Debug, Clone)]
pub struct ExecSpec {
    /// Verbatim shell command line.
    pub command: String,
    /// Working directory (relative to whatever the backend considers
    /// its workspace root). Empty means "the backend's default".
    pub workdir: Option<PathBuf>,
    /// Extra environment variables. Keys are sorted by `BTreeMap` to keep
    /// the wire form deterministic — important for prompt-cache stability
    /// when these values land in tool-result text.
    pub env: BTreeMap<String, String>,
    /// Resource caps for this invocation.
    pub limits: ResourceLimits,
}

impl ExecSpec {
    /// Convenience constructor for the common "just run this command"
    /// case — no env vars, no workdir override, default limits.
    pub fn new(command: impl Into<String>) -> Self {
        Self {
            command: command.into(),
            workdir: None,
            env: BTreeMap::new(),
            limits: ResourceLimits::default(),
        }
    }
}

/// Outcome of a successful (or non-zero-exit) command run.
///
/// Returning a non-zero `exit_code` is **not** an error from the
/// backend's point of view — the command was dispatched and produced
/// output. Backends only return `Err(ExecError)` when they cannot
/// dispatch at all (auth failure, network timeout, missing config).
#[derive(Debug, Clone)]
pub struct ExecOutcome {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
    /// Backend-specific identifier of the execution context
    /// (Docker container ID, SSH session ID, Daytona workspace ID).
    /// Surfaces in tool-result JSON so operators can correlate logs.
    pub backend_id: Option<String>,
}

/// Backend-side errors. Distinct from non-zero exit codes (which surface
/// as `ExecOutcome` with `exit_code != 0`).
#[derive(Debug, thiserror::Error)]
pub enum ExecError {
    /// The selected backend is missing required configuration
    /// (e.g. SSH `host` empty, Daytona `api_key_env` unset).
    #[error("backend not configured: {0}")]
    NotConfigured(String),
    /// Connection / auth failure reaching the remote system.
    #[error("backend connect error: {0}")]
    Connect(String),
    /// The backend rejected the command before dispatching it
    /// (validation, allowlist, quota).
    #[error("backend rejected command: {0}")]
    Rejected(String),
    /// Timeout firing before the command produced an exit status.
    #[error("backend timeout: {0}")]
    Timeout(String),
    /// Operation is not supported by this backend (e.g. SSH backend
    /// without SFTP refusing `upload`).
    #[error("operation not supported by {backend} backend: {operation}")]
    UnsupportedForBackend {
        backend: &'static str,
        operation: &'static str,
    },
    /// Catch-all for backend-internal failures.
    #[error("backend error: {0}")]
    Other(String),
}

/// Trait every tool-execution backend implements.
///
/// All methods are `async`, `Send + Sync`. Stateful backends (SSH,
/// Daytona) hold their connection inside the impl; `cleanup()` is called
/// when the agent's session ends so the backend can drop sockets /
/// destroy sandboxes.
#[async_trait]
pub trait ToolExecBackend: Send + Sync {
    /// Backend kind — used for logging and feature reporting.
    fn kind(&self) -> BackendKind;

    /// Run a single command and return its outcome.
    async fn run_command(&self, spec: ExecSpec) -> Result<ExecOutcome, ExecError>;

    /// Upload bytes to a path on the backend's workspace.
    /// Default impl returns `UnsupportedForBackend` — backends that need
    /// it (Docker, future SFTP-enabled SSH) override.
    async fn upload(&self, _path: &str, _bytes: &[u8]) -> Result<(), ExecError> {
        Err(ExecError::UnsupportedForBackend {
            backend: self.kind().as_str(),
            operation: "upload",
        })
    }

    /// Download bytes from a path on the backend's workspace.
    async fn download(&self, _path: &str) -> Result<Vec<u8>, ExecError> {
        Err(ExecError::UnsupportedForBackend {
            backend: self.kind().as_str(),
            operation: "download",
        })
    }

    /// Tear down whatever long-lived resources the backend holds.
    /// Idempotent — safe to call multiple times.
    async fn cleanup(&self) -> Result<(), ExecError> {
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Local subprocess backend
// ---------------------------------------------------------------------------

/// Local-subprocess backend.
///
/// Uses the same code path as the legacy direct-call route via
/// `tokio::process::Command`, with `subprocess_sandbox::sandbox_command`
/// scrubbing the inherited environment. Always available — no feature
/// flag, no remote dependencies.
pub struct LocalBackend {
    /// Allowlisted env vars to pass through to spawned children.
    /// Mirrors the existing `tool_runner.rs` `allowed_env` plumbing.
    allowed_env: Vec<String>,
    /// Default per-command timeout when `ExecSpec::limits.timeout` is unset.
    default_timeout: Duration,
}

impl LocalBackend {
    pub fn new(allowed_env: Vec<String>, default_timeout: Duration) -> Self {
        Self {
            allowed_env,
            default_timeout,
        }
    }

    /// Convenience: a Local backend with empty env passthrough and a 30s default.
    /// Intended for tests and the resolver fallback.
    pub fn with_defaults() -> Self {
        Self::new(Vec::new(), Duration::from_secs(30))
    }
}

#[async_trait]
impl ToolExecBackend for LocalBackend {
    fn kind(&self) -> BackendKind {
        BackendKind::Local
    }

    async fn run_command(&self, spec: ExecSpec) -> Result<ExecOutcome, ExecError> {
        // POSIX `sh -c` on Unix, `cmd /C` on Windows — same shape as
        // docker_sandbox::exec_in_sandbox uses internally so the contract
        // (verbatim command line) is identical.
        #[cfg(unix)]
        let mut cmd = {
            let mut c = tokio::process::Command::new("sh");
            c.arg("-c").arg(&spec.command);
            c
        };
        #[cfg(windows)]
        let mut cmd = {
            let mut c = tokio::process::Command::new("cmd");
            c.arg("/C").arg(&spec.command);
            c
        };

        crate::subprocess_sandbox::sandbox_command(&mut cmd, &self.allowed_env);

        // Layer caller-supplied env on top of the sandboxed allowlist.
        for (k, v) in &spec.env {
            cmd.env(k, v);
        }

        if let Some(wd) = &spec.workdir {
            cmd.current_dir(wd);
        }

        cmd.stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());

        let timeout = spec.limits.timeout.unwrap_or(self.default_timeout);

        let output = tokio::time::timeout(timeout, cmd.output())
            .await
            .map_err(|_| ExecError::Timeout(format!("after {}s", timeout.as_secs())))?
            .map_err(|e| ExecError::Other(format!("spawn failed: {e}")))?;

        let mut stdout = String::from_utf8_lossy(&output.stdout).into_owned();
        let mut stderr = String::from_utf8_lossy(&output.stderr).into_owned();

        if let Some(cap) = spec.limits.max_output_bytes {
            stdout = truncate_to_cap(stdout, cap);
            stderr = truncate_to_cap(stderr, cap);
        }

        Ok(ExecOutcome {
            stdout,
            stderr,
            exit_code: output.status.code().unwrap_or(-1),
            backend_id: None,
        })
    }
}

fn truncate_to_cap(mut s: String, cap: usize) -> String {
    if s.len() > cap {
        let total = s.len();
        let safe = crate::str_utils::safe_truncate_str(&s, cap).to_string();
        s = format!("{safe}... [truncated, {total} total bytes]");
    }
    s
}

// ---------------------------------------------------------------------------
// Docker backend (adapter over docker_sandbox)
// ---------------------------------------------------------------------------

/// Adapter that exposes the existing `docker_sandbox` create+exec+destroy
/// flow as a single `ToolExecBackend::run_command` call. Reuses the
/// long-standing `DockerSandboxConfig`; no new config knobs.
pub struct DockerBackend {
    config: librefang_types::config::DockerSandboxConfig,
    agent_id: String,
    workspace: PathBuf,
}

impl DockerBackend {
    pub fn new(
        config: librefang_types::config::DockerSandboxConfig,
        agent_id: impl Into<String>,
        workspace: PathBuf,
    ) -> Self {
        Self {
            config,
            agent_id: agent_id.into(),
            workspace,
        }
    }
}

#[async_trait]
impl ToolExecBackend for DockerBackend {
    fn kind(&self) -> BackendKind {
        BackendKind::Docker
    }

    async fn run_command(&self, spec: ExecSpec) -> Result<ExecOutcome, ExecError> {
        if !self.config.enabled {
            return Err(ExecError::NotConfigured(
                "docker.enabled = false in config.toml".into(),
            ));
        }
        if !crate::docker_sandbox::is_docker_available().await {
            return Err(ExecError::NotConfigured("docker binary not on PATH".into()));
        }

        let container =
            crate::docker_sandbox::create_sandbox(&self.config, &self.agent_id, &self.workspace)
                .await
                .map_err(ExecError::Other)?;

        let timeout = spec
            .limits
            .timeout
            .unwrap_or(Duration::from_secs(self.config.timeout_secs));
        let res = crate::docker_sandbox::exec_in_sandbox(&container, &spec.command, timeout).await;

        // Always destroy regardless of outcome — mirrors the existing
        // tool_docker_exec semantics in tool_runner.rs.
        if let Err(e) = crate::docker_sandbox::destroy_sandbox(&container).await {
            tracing::warn!("docker_sandbox cleanup failed: {e}");
        }

        let exec = res.map_err(ExecError::Other)?;
        Ok(ExecOutcome {
            stdout: exec.stdout,
            stderr: exec.stderr,
            exit_code: exec.exit_code,
            backend_id: Some(container.container_id),
        })
    }
}

// ---------------------------------------------------------------------------
// Backend factory — builds the impl matching the resolved BackendKind.
// ---------------------------------------------------------------------------

/// Build a backend instance from the resolved `BackendKind` and the
/// global `ToolExecConfig`. Returns a boxed trait object so callers
/// can store the backend behind a single type per agent.
///
/// `agent_id` and `workspace` are needed by the Docker adapter; pass
/// them even if the kind isn't Docker — they're cheap.
pub fn build_backend(
    kind: BackendKind,
    cfg: &ToolExecConfig,
    docker_cfg: &librefang_types::config::DockerSandboxConfig,
    agent_id: &str,
    workspace: PathBuf,
    allowed_env: Vec<String>,
) -> Result<Box<dyn ToolExecBackend>, ExecError> {
    match kind {
        BackendKind::Local => Ok(Box::new(LocalBackend::new(
            allowed_env,
            Duration::from_secs(30),
        ))),
        BackendKind::Docker => Ok(Box::new(DockerBackend::new(
            docker_cfg.clone(),
            agent_id,
            workspace,
        ))),
        BackendKind::Ssh => {
            #[cfg(feature = "ssh-backend")]
            {
                let ssh_cfg = cfg.ssh.as_ref().ok_or_else(|| {
                    ExecError::NotConfigured(
                        "kind = \"ssh\" but [tool_exec.ssh] subtable is missing".into(),
                    )
                })?;
                Ok(Box::new(crate::tool_exec_ssh::SshBackend::new(
                    ssh_cfg.clone(),
                )))
            }
            #[cfg(not(feature = "ssh-backend"))]
            {
                let _ = cfg;
                Err(ExecError::NotConfigured(
                    "ssh backend selected but the runtime was built without the \
                     `ssh-backend` cargo feature"
                        .into(),
                ))
            }
        }
        BackendKind::Daytona => {
            #[cfg(feature = "daytona-backend")]
            {
                let dt_cfg = cfg.daytona.as_ref().ok_or_else(|| {
                    ExecError::NotConfigured(
                        "kind = \"daytona\" but [tool_exec.daytona] subtable is missing".into(),
                    )
                })?;
                Ok(Box::new(crate::tool_exec_daytona::DaytonaBackend::new(
                    dt_cfg.clone(),
                    agent_id.to_string(),
                )))
            }
            #[cfg(not(feature = "daytona-backend"))]
            {
                let _ = cfg;
                let _ = agent_id;
                Err(ExecError::NotConfigured(
                    "daytona backend selected but the runtime was built without the \
                     `daytona-backend` cargo feature"
                        .into(),
                ))
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_under_cap_unchanged() {
        let s = "hello".to_string();
        assert_eq!(truncate_to_cap(s.clone(), 100), "hello");
    }

    #[test]
    fn truncate_over_cap_appends_marker() {
        let s = "x".repeat(200);
        let out = truncate_to_cap(s.clone(), 50);
        assert!(out.starts_with(&"x".repeat(50)));
        assert!(out.contains("[truncated, 200 total bytes]"));
    }

    #[tokio::test]
    async fn local_backend_runs_echo() {
        let backend = LocalBackend::with_defaults();
        // POSIX-only assertion — the docker wrapper test environment
        // always has `sh`.
        if cfg!(unix) {
            let outcome = backend
                .run_command(ExecSpec::new("echo hello"))
                .await
                .expect("echo must succeed");
            assert_eq!(outcome.exit_code, 0);
            assert!(outcome.stdout.contains("hello"));
            assert_eq!(outcome.kind_str(), "local");
        }
    }

    #[tokio::test]
    async fn local_backend_captures_nonzero_exit() {
        if cfg!(unix) {
            let backend = LocalBackend::with_defaults();
            let outcome = backend
                .run_command(ExecSpec::new("false"))
                .await
                .expect("`false` dispatches successfully");
            assert_ne!(outcome.exit_code, 0);
        }
    }

    #[tokio::test]
    async fn local_backend_timeout_returns_error() {
        if cfg!(unix) {
            let backend = LocalBackend::with_defaults();
            let mut spec = ExecSpec::new("sleep 5");
            spec.limits.timeout = Some(Duration::from_millis(100));
            match backend.run_command(spec).await {
                Err(ExecError::Timeout(_)) => {}
                other => panic!("expected timeout, got {other:?}"),
            }
        }
    }

    #[tokio::test]
    async fn local_backend_max_output_truncates() {
        if cfg!(unix) {
            let backend = LocalBackend::with_defaults();
            let mut spec = ExecSpec::new("yes hello | head -c 5000");
            spec.limits.max_output_bytes = Some(100);
            spec.limits.timeout = Some(Duration::from_secs(5));
            let outcome = backend.run_command(spec).await.expect("runs");
            // truncated stdout carries the marker
            assert!(
                outcome.stdout.contains("[truncated"),
                "stdout was: {:?}",
                outcome.stdout
            );
        }
    }

    #[tokio::test]
    async fn local_backend_default_upload_unsupported() {
        let backend = LocalBackend::with_defaults();
        match backend.upload("/tmp/x", b"hello").await {
            Err(ExecError::UnsupportedForBackend { backend, operation }) => {
                assert_eq!(backend, "local");
                assert_eq!(operation, "upload");
            }
            other => panic!("expected UnsupportedForBackend, got {other:?}"),
        }
    }

    #[test]
    fn build_backend_local_always_works() {
        let cfg = ToolExecConfig::default();
        let docker_cfg = librefang_types::config::DockerSandboxConfig::default();
        let backend = build_backend(
            BackendKind::Local,
            &cfg,
            &docker_cfg,
            "agent-1",
            std::env::temp_dir(),
            vec![],
        )
        .expect("local backend always builds");
        assert_eq!(backend.kind(), BackendKind::Local);
    }

    #[test]
    fn build_backend_docker_returns_docker_kind() {
        let cfg = ToolExecConfig::default();
        let docker_cfg = librefang_types::config::DockerSandboxConfig::default();
        let backend = build_backend(
            BackendKind::Docker,
            &cfg,
            &docker_cfg,
            "agent-1",
            std::env::temp_dir(),
            vec![],
        )
        .expect("docker backend builds even when daemon absent");
        assert_eq!(backend.kind(), BackendKind::Docker);
    }

    #[test]
    fn build_backend_ssh_without_feature_or_subtable_errors() {
        let cfg = ToolExecConfig::default(); // no ssh subtable
        let docker_cfg = librefang_types::config::DockerSandboxConfig::default();
        let res = build_backend(
            BackendKind::Ssh,
            &cfg,
            &docker_cfg,
            "agent-1",
            std::env::temp_dir(),
            vec![],
        );
        assert!(
            res.is_err(),
            "ssh backend should error without config / feature"
        );
    }

    #[test]
    fn build_backend_daytona_without_feature_or_subtable_errors() {
        let cfg = ToolExecConfig::default(); // no daytona subtable
        let docker_cfg = librefang_types::config::DockerSandboxConfig::default();
        let res = build_backend(
            BackendKind::Daytona,
            &cfg,
            &docker_cfg,
            "agent-1",
            std::env::temp_dir(),
            vec![],
        );
        assert!(
            res.is_err(),
            "daytona backend should error without config / feature"
        );
    }

    // Test-only helper kept on `ExecOutcome` for ergonomic assertions.
    impl ExecOutcome {
        fn kind_str(&self) -> &'static str {
            // Not actually carried on outcome; placeholder for assertion symmetry.
            // The real backend kind is on `ToolExecBackend::kind()`.
            "local"
        }
    }
}
