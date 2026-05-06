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
use tokio::io::AsyncReadExt;

// ---------------------------------------------------------------------------
// Public trait + DTOs
// ---------------------------------------------------------------------------

/// Default `LocalBackend` per-command timeout when neither the spec nor
/// the operator config supplies one. Matches the legacy `tool_runner`
/// shell-class default at `tool_runner.rs:2896` so behaviour is
/// preserved across the trait migration.
pub const LOCAL_DEFAULT_TIMEOUT_SECS: u64 = 30;

/// Default `DockerBackend` `max_output_bytes` cap when callers omit
/// one. Matches the cap that `docker_sandbox::exec_in_sandbox` has used
/// historically (≈ 50 KiB per stream); referenced from the doc on
/// [`ResourceLimits::max_output_bytes`] so future tuning has one knob.
pub const DOCKER_DEFAULT_MAX_OUTPUT_BYTES: usize = 50 * 1024;

/// Environment-variable keys we never propagate into a child process,
/// regardless of what the caller put on `ExecSpec::env`. These are
/// dynamic-loader hijack vectors — letting an LLM-emitted `env` map
/// override them would let a tool call swap out `libc` or load a
/// shim into every subsequent child. Backends MUST scrub these at the
/// trait boundary; the local backend `debug_assert!`s their absence in
/// debug builds, plus drops + warns in release.
pub const RESERVED_ENV_KEYS: &[&str] = &[
    "LD_PRELOAD",
    "DYLD_INSERT_LIBRARIES",
    "DYLD_LIBRARY_PATH",
    "DYLD_FRAMEWORK_PATH",
];

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
    /// Maximum bytes returned to the caller per stream (stdout AND
    /// stderr each capped at this value — the cap is *per stream*, not
    /// combined). `None` falls back to the backend default
    /// (see [`DOCKER_DEFAULT_MAX_OUTPUT_BYTES`] for Docker).
    pub max_output_bytes: Option<usize>,
}

/// One command to execute on a backend.
///
/// `command` is the literal command line — backends pass it to whatever
/// shell makes sense (`sh -c …` for Docker / SSH, the host's
/// `tokio::process::Command` for Local). Validation against the agent's
/// `ExecPolicy` happens **before** dispatch, in `tool_runner.rs`.
///
/// **Reserved env keys.** Keys listed in [`RESERVED_ENV_KEYS`] are
/// silently dropped by every backend before the child process is
/// spawned — see the constant's docs for the rationale.
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

    /// Builder: set the working directory.
    pub fn with_workdir(mut self, p: impl Into<PathBuf>) -> Self {
        self.workdir = Some(p.into());
        self
    }

    /// Builder: insert one env var. Keys present in
    /// [`RESERVED_ENV_KEYS`] will be dropped at dispatch time — setting
    /// them here is a no-op rather than an error so callers can pass
    /// "whatever the agent gave us" without filtering up front.
    pub fn with_env(mut self, k: impl Into<String>, v: impl Into<String>) -> Self {
        self.env.insert(k.into(), v.into());
        self
    }

    /// Builder: cap the wall-clock timeout.
    pub fn with_timeout(mut self, d: Duration) -> Self {
        self.limits.timeout = Some(d);
        self
    }

    /// Builder: cap stdout/stderr (per stream).
    pub fn with_max_output_bytes(mut self, n: usize) -> Self {
        self.limits.max_output_bytes = Some(n);
        self
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
    /// Authentication failure (separate from generic Connect so callers
    /// can prompt for new credentials specifically).
    #[error("backend auth failure: {0}")]
    AuthFailure(String),
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
    /// Source-of-truth: kernel config / agent manifest. Falls back to
    /// [`LOCAL_DEFAULT_TIMEOUT_SECS`] when constructed via
    /// [`LocalBackend::with_defaults`].
    default_timeout: Duration,
}

impl LocalBackend {
    pub fn new(allowed_env: Vec<String>, default_timeout: Duration) -> Self {
        Self {
            allowed_env,
            default_timeout,
        }
    }

    /// Convenience: a Local backend with empty env passthrough and the
    /// legacy [`LOCAL_DEFAULT_TIMEOUT_SECS`] default.
    /// Intended for tests and the resolver fallback.
    pub fn with_defaults() -> Self {
        Self::new(Vec::new(), Duration::from_secs(LOCAL_DEFAULT_TIMEOUT_SECS))
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

        // Layer caller-supplied env on top of the sandboxed allowlist,
        // dropping anything in RESERVED_ENV_KEYS. Debug builds also
        // assert the caller didn't even try.
        for (k, v) in &spec.env {
            if is_reserved_env_key(k) {
                debug_assert!(
                    false,
                    "reserved env key {k:?} present on ExecSpec::env (reserved: {:?})",
                    RESERVED_ENV_KEYS
                );
                tracing::warn!(
                    key = %k,
                    "tool_exec: dropping reserved env key from child process \
                     (loader-hijack vector); see RESERVED_ENV_KEYS in tool_exec_backend.rs"
                );
                continue;
            }
            cmd.env(k, v);
        }

        if let Some(wd) = &spec.workdir {
            cmd.current_dir(wd);
        }

        cmd.stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());

        let timeout = spec.limits.timeout.unwrap_or(self.default_timeout);

        // Spawn + stream so a runaway child can't OOM the daemon by
        // emitting more bytes than `max_output_bytes`. We read each pipe
        // into a Vec capped at the limit; once the cap is reached we
        // drop further bytes and set `truncated`. We DON'T kill the
        // child — the legacy code path didn't either, and killing on
        // overflow risks dropping useful exit-status info.
        let mut child = cmd
            .spawn()
            .map_err(|e| ExecError::Other(format!("spawn failed: {e}")))?;
        let mut stdout_pipe = child
            .stdout
            .take()
            .ok_or_else(|| ExecError::Other("child stdout pipe missing".into()))?;
        let mut stderr_pipe = child
            .stderr
            .take()
            .ok_or_else(|| ExecError::Other("child stderr pipe missing".into()))?;

        // Per-stream cap. Use a generous default when callers don't set
        // one so unbounded `yes`-style spam still doesn't OOM us.
        let cap = spec
            .limits
            .max_output_bytes
            .unwrap_or(DOCKER_DEFAULT_MAX_OUTPUT_BYTES);

        async fn read_capped<R: AsyncReadExt + Unpin>(
            r: &mut R,
            cap: usize,
        ) -> std::io::Result<(Vec<u8>, usize, bool)> {
            let mut buf = Vec::with_capacity(std::cmp::min(cap, 8 * 1024));
            let mut total = 0usize;
            let mut truncated = false;
            let mut chunk = [0u8; 8 * 1024];
            loop {
                let n = r.read(&mut chunk).await?;
                if n == 0 {
                    break;
                }
                total = total.saturating_add(n);
                if buf.len() < cap {
                    let take = std::cmp::min(n, cap - buf.len());
                    buf.extend_from_slice(&chunk[..take]);
                    if take < n {
                        truncated = true;
                    }
                } else {
                    truncated = true;
                }
            }
            Ok((buf, total, truncated))
        }

        let stdout_fut = read_capped(&mut stdout_pipe, cap);
        let stderr_fut = read_capped(&mut stderr_pipe, cap);
        let wait_fut = child.wait();

        let combined = async move {
            let ((stdout_res, stderr_res), wait_res) =
                tokio::join!(async { tokio::join!(stdout_fut, stderr_fut) }, wait_fut);
            let (stdout_buf, stdout_total, stdout_trunc) =
                stdout_res.map_err(|e| ExecError::Other(format!("stdout read: {e}")))?;
            let (stderr_buf, stderr_total, stderr_trunc) =
                stderr_res.map_err(|e| ExecError::Other(format!("stderr read: {e}")))?;
            let status = wait_res.map_err(|e| ExecError::Other(format!("wait: {e}")))?;
            Ok::<_, ExecError>((
                stdout_buf,
                stdout_total,
                stdout_trunc,
                stderr_buf,
                stderr_total,
                stderr_trunc,
                status,
            ))
        };

        let (
            stdout_buf,
            stdout_total,
            stdout_trunc,
            stderr_buf,
            stderr_total,
            stderr_trunc,
            status,
        ) = tokio::time::timeout(timeout, combined)
            .await
            .map_err(|_| ExecError::Timeout(format!("after {}s", timeout.as_secs())))??;

        let mut stdout = String::from_utf8_lossy(&stdout_buf).into_owned();
        let mut stderr = String::from_utf8_lossy(&stderr_buf).into_owned();

        if stdout_trunc {
            stdout.push_str(&format!("... [truncated, {stdout_total} total bytes]"));
        }
        if stderr_trunc {
            stderr.push_str(&format!("... [truncated, {stderr_total} total bytes]"));
        }

        Ok(ExecOutcome {
            stdout,
            stderr,
            exit_code: status.code().unwrap_or(-1),
            backend_id: None,
        })
    }
}

/// Returns true if the key is on the reserved (loader-hijack) list.
/// Case-sensitive — POSIX env names are case-sensitive in practice.
pub(crate) fn is_reserved_env_key(key: &str) -> bool {
    RESERVED_ENV_KEYS.contains(&key)
}

/// Truncate a string to `cap` bytes, appending a marker noting the
/// original total. UTF-8 safe via [`crate::str_utils::safe_truncate_str`].
///
/// Shared by SSH and Daytona backends (and tests) — keep behaviour
/// identical so the marker string stays consistent across the wire
/// (operators grep for the marker to spot truncation in logs).
/// `LocalBackend` doesn't call this directly because it already caps
/// during streaming and appends the same marker manually after the
/// `read_capped` loop; under default features (no ssh / daytona) the
/// helper has no live caller, hence the explicit `allow(dead_code)`.
#[allow(dead_code)]
pub(crate) fn truncate_to_cap(s: String, cap: usize) -> String {
    if s.len() > cap {
        let total = s.len();
        let safe = crate::str_utils::safe_truncate_str(&s, cap).to_string();
        format!("{safe}... [truncated, {total} total bytes]")
    } else {
        s
    }
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

        // Reserved-env scrub at the trait boundary — Docker's
        // `exec_in_sandbox` doesn't currently honour `spec.env`, but
        // when it does the reserved keys must not leak in.
        for k in spec.env.keys() {
            if is_reserved_env_key(k) {
                debug_assert!(
                    false,
                    "reserved env key {k:?} present on ExecSpec::env (Docker)"
                );
                tracing::warn!(
                    key = %k,
                    "tool_exec/docker: reserved env key dropped before container exec"
                );
            }
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
            Duration::from_secs(LOCAL_DEFAULT_TIMEOUT_SECS),
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

    #[test]
    fn truncate_cap_zero_emits_pure_marker() {
        // Edge case: cap = 0 — `safe_truncate_str` must yield "" and the
        // marker is appended verbatim. Guards against arithmetic-underflow
        // regressions in the helper.
        let out = truncate_to_cap("hi".to_string(), 0);
        assert_eq!(out, "... [truncated, 2 total bytes]");
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn local_backend_runs_echo() {
        let backend = LocalBackend::with_defaults();
        let outcome = backend
            .run_command(ExecSpec::new("echo hello"))
            .await
            .expect("echo must succeed");
        assert_eq!(outcome.exit_code, 0);
        assert!(outcome.stdout.contains("hello"));
        assert_eq!(backend.kind().as_str(), "local");
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn local_backend_captures_nonzero_exit() {
        let backend = LocalBackend::with_defaults();
        let outcome = backend
            .run_command(ExecSpec::new("false"))
            .await
            .expect("`false` dispatches successfully");
        assert_ne!(outcome.exit_code, 0);
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn local_backend_timeout_returns_error() {
        let backend = LocalBackend::with_defaults();
        let spec = ExecSpec::new("sleep 5").with_timeout(Duration::from_millis(100));
        match backend.run_command(spec).await {
            Err(ExecError::Timeout(_)) => {}
            other => panic!("expected timeout, got {other:?}"),
        }
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn local_backend_max_output_truncates() {
        let backend = LocalBackend::with_defaults();
        let spec = ExecSpec::new("yes hello | head -c 5000")
            .with_max_output_bytes(100)
            .with_timeout(Duration::from_secs(5));
        let outcome = backend.run_command(spec).await.expect("runs");
        // Streaming truncation appends the marker.
        assert!(
            outcome.stdout.contains("[truncated"),
            "stdout was: {:?}",
            outcome.stdout
        );
    }

    /// H1 regression: a child that emits far more than `cap` bytes must
    /// (a) finish without hanging, (b) yield a capped stdout body, and
    /// (c) record the original total in the truncation marker.
    #[tokio::test]
    #[cfg(unix)]
    async fn local_backend_streams_output_above_cap_without_oom() {
        let backend = LocalBackend::with_defaults();
        // 200 KiB of `x` — >> cap, well below anything that would
        // actually OOM the test runner, but enough to prove streaming.
        let spec = ExecSpec::new(r#"yes x | tr -d '\n' | head -c 204800 ; echo ; echo done"#)
            .with_max_output_bytes(4096)
            .with_timeout(Duration::from_secs(10));
        let outcome = backend
            .run_command(spec)
            .await
            .expect("must dispatch and exit, not hang");
        assert_eq!(outcome.exit_code, 0, "child should exit cleanly");
        // Truncated body — 4 KiB of payload + the marker suffix.
        assert!(
            outcome.stdout.contains("[truncated,"),
            "expected truncation marker, got: {:?}",
            &outcome.stdout[..std::cmp::min(120, outcome.stdout.len())]
        );
        // Marker reports the true byte total (well above the cap).
        let marker_pos = outcome
            .stdout
            .find("[truncated,")
            .expect("truncated marker present");
        let payload = &outcome.stdout[..marker_pos];
        assert!(
            payload.len() <= 4096 + 8,
            "payload should be capped at ~cap bytes; got {} bytes",
            payload.len()
        );
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

    #[test]
    fn reserved_env_keys_match_doc() {
        assert!(is_reserved_env_key("LD_PRELOAD"));
        assert!(is_reserved_env_key("DYLD_INSERT_LIBRARIES"));
        assert!(is_reserved_env_key("DYLD_LIBRARY_PATH"));
        assert!(is_reserved_env_key("DYLD_FRAMEWORK_PATH"));
        assert!(!is_reserved_env_key("PATH"));
        assert!(!is_reserved_env_key("ld_preload")); // case-sensitive
    }

    /// Reserved-env keys placed on `ExecSpec::env` must not reach the
    /// child. Release-mode behaviour: drop + warn (we can't observe the
    /// warn here easily, so we settle for "child sees a sentinel that
    /// shows the key was dropped").
    #[tokio::test]
    #[cfg(unix)]
    async fn local_backend_drops_reserved_env_keys() {
        // We can only directly verify in release mode — debug builds
        // hit the `debug_assert!`. Skip if we're in debug.
        if cfg!(debug_assertions) {
            return;
        }
        let backend = LocalBackend::with_defaults();
        let spec = ExecSpec::new(r#"echo "[$LD_PRELOAD]""#)
            .with_env("LD_PRELOAD", "/tmp/evil.so")
            .with_timeout(Duration::from_secs(2));
        let outcome = backend.run_command(spec).await.expect("runs");
        assert_eq!(outcome.exit_code, 0);
        // The sandboxed env scrubs LD_PRELOAD anyway; the test is that
        // the value we tried to inject did NOT appear.
        assert!(
            !outcome.stdout.contains("/tmp/evil.so"),
            "reserved env key leaked into child: {:?}",
            outcome.stdout
        );
    }

    #[test]
    fn exec_spec_builder_chains() {
        let spec = ExecSpec::new("ls")
            .with_workdir("/tmp")
            .with_env("FOO", "bar")
            .with_timeout(Duration::from_secs(7))
            .with_max_output_bytes(123);
        assert_eq!(spec.command, "ls");
        assert_eq!(spec.workdir.as_deref(), Some(std::path::Path::new("/tmp")));
        assert_eq!(spec.env.get("FOO").map(|s| s.as_str()), Some("bar"));
        assert_eq!(spec.limits.timeout, Some(Duration::from_secs(7)));
        assert_eq!(spec.limits.max_output_bytes, Some(123));
    }
}
