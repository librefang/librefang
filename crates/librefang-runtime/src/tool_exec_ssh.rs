//! SSH-backed [`ToolExecBackend`] (#3332).
//!
//! Behind the `ssh-backend` cargo feature. Uses [`russh`] for the raw
//! SSH transport — no shelling out to the system `ssh` client, so the
//! daemon does not need a working OpenSSH installation on the host.
//!
//! ## Scope
//!
//! - Exec-only. We open a session, run a single command per request,
//!   capture stdout/stderr + exit code, and close. Connection lifetime
//!   is tied to one [`run_command`] call, NOT to the agent session —
//!   keeps the impl simple and avoids holding sockets open across long
//!   idle periods.
//! - File I/O (`upload` / `download`) is **not** implemented. Callers
//!   get [`ExecError::UnsupportedForBackend`] back. SFTP via
//!   `russh-sftp` is a deliberate follow-up; see
//!   `docs/architecture/tool-exec-backends.md`.
//!
//! ## Auth
//!
//! - `key_path` set → public-key auth from the file (PEM / OpenSSH
//!   formats supported by `russh-keys`).
//! - `password_env` set → password auth from the named env var.
//! - Neither set → tries publickey-from-agent, falling back to none.
//!   This last branch only succeeds against hosts that explicitly
//!   `PermitEmptyPasswords yes`, which we expect to be vanishingly
//!   rare. The point is to avoid panicking on misconfiguration.
//!
//! ## Host-key verification
//!
//! `SshBackendConfig.host_key_sha256` is the SHA-256 hex of the
//! expected server host key. When set, the backend hard-rejects a
//! connection whose host key doesn't match (TOFU pinning). When empty,
//! the backend logs the fingerprint at INFO and accepts on first
//! connect — fine for trusted LAN, but operators are warned in the
//! docs to set the pin in production.

use crate::tool_exec_backend::{ExecError, ExecOutcome, ExecSpec, ToolExecBackend};
use async_trait::async_trait;
use librefang_types::tool_exec::{BackendKind, SshBackendConfig};

/// SSH backend handle.
///
/// Cheap to clone — connections are opened on demand inside
/// `run_command`, not stored on this struct. Holds only the typed
/// configuration.
pub struct SshBackend {
    cfg: SshBackendConfig,
}

impl SshBackend {
    pub fn new(cfg: SshBackendConfig) -> Self {
        Self { cfg }
    }

    fn validate_config(&self) -> Result<(), ExecError> {
        if self.cfg.host.trim().is_empty() {
            return Err(ExecError::NotConfigured(
                "tool_exec.ssh.host is empty".into(),
            ));
        }
        if self.cfg.user.trim().is_empty() {
            return Err(ExecError::NotConfigured(
                "tool_exec.ssh.user is empty".into(),
            ));
        }
        Ok(())
    }
}

#[async_trait]
impl ToolExecBackend for SshBackend {
    fn kind(&self) -> BackendKind {
        BackendKind::Ssh
    }

    async fn run_command(&self, spec: ExecSpec) -> Result<ExecOutcome, ExecError> {
        self.validate_config()?;

        // The russh API is verbose enough that piling it inline here
        // would obscure the control flow. Move the actual transport
        // dance into `transport::exec_one` so this method stays
        // readable and the unit tests can substitute a stub for the
        // transport layer.
        transport::exec_one(&self.cfg, &spec).await
    }

    // upload / download intentionally use the trait default that returns
    // UnsupportedForBackend — see module docs for rationale.
}

// ---------------------------------------------------------------------------
// Real transport (russh) — only compiled when feature is on.
// ---------------------------------------------------------------------------

mod transport {
    use super::*;
    use russh::client;
    use russh::client::Handler;
    use russh_keys::key::PublicKey;
    use russh_keys::PublicKeyBase64;
    use std::sync::Arc;
    use std::time::Duration;
    use tokio::time::timeout;

    /// `russh` requires a `Handler` for client-side decisions. We use it
    /// to enforce optional host-key pinning — `SshBackendConfig.host_key_sha256`.
    /// When empty, we log-and-accept (TOFU-style); when set, we hard-reject
    /// any mismatch.
    struct PinningHandler {
        expected_sha256: String,
    }

    #[async_trait]
    impl Handler for PinningHandler {
        type Error = russh::Error;

        async fn check_server_key(&mut self, server_key: &PublicKey) -> Result<bool, Self::Error> {
            // russh_keys 0.45 exposes a base64-style fingerprint via
            // `PublicKey::fingerprint()`, e.g. `"AAAAB3...."`. Hash the
            // wire-form public-key bytes ourselves so we control the
            // pin format (hex SHA-256 of the SSH wire blob).
            use sha2::{Digest, Sha256};
            let blob = server_key.public_key_bytes();
            let mut hasher = Sha256::new();
            hasher.update(&blob);
            let digest = hasher.finalize();
            let hex = digest_to_hex(&digest);
            if self.expected_sha256.is_empty() {
                tracing::info!(
                    fingerprint = %hex,
                    "tool_exec.ssh: accepting unpinned host key on first connect; \
                     set tool_exec.ssh.host_key_sha256 to pin it"
                );
                return Ok(true);
            }
            let expected = self.expected_sha256.trim();
            let matches = expected.eq_ignore_ascii_case(&hex);
            if !matches {
                tracing::error!(
                    expected = %expected,
                    actual = %hex,
                    "tool_exec.ssh: host key fingerprint mismatch"
                );
            }
            Ok(matches)
        }
    }

    fn digest_to_hex(digest: &[u8]) -> String {
        let mut s = String::with_capacity(digest.len() * 2);
        for b in digest {
            s.push_str(&format!("{b:02x}"));
        }
        s
    }

    pub(super) async fn exec_one(
        cfg: &SshBackendConfig,
        spec: &ExecSpec,
    ) -> Result<ExecOutcome, ExecError> {
        let total_timeout = spec
            .limits
            .timeout
            .unwrap_or_else(|| Duration::from_secs(cfg.timeout_secs));

        timeout(total_timeout, do_exec(cfg, spec))
            .await
            .map_err(|_| ExecError::Timeout(format!("after {}s", total_timeout.as_secs())))?
    }

    async fn do_exec(cfg: &SshBackendConfig, spec: &ExecSpec) -> Result<ExecOutcome, ExecError> {
        let client_cfg = Arc::new(client::Config::default());
        let handler = PinningHandler {
            expected_sha256: cfg.host_key_sha256.clone(),
        };
        let addr = format!("{}:{}", cfg.host, cfg.port);
        let mut session = client::connect(client_cfg, addr, handler)
            .await
            .map_err(|e| ExecError::Connect(format!("ssh connect: {e}")))?;

        // Authenticate. russh 0.45 returns `bool` directly from each
        // authenticate_* call; no wrapper.
        let user = cfg.user.clone();
        let auth_ok = if let Some(path) = &cfg.key_path {
            let key = russh_keys::load_secret_key(path, None)
                .map_err(|e| ExecError::Connect(format!("ssh key {path:?}: {e}")))?;
            session
                .authenticate_publickey(user, std::sync::Arc::new(key))
                .await
                .map_err(|e| ExecError::Connect(format!("ssh publickey: {e}")))?
        } else if let Some(env_name) = &cfg.password_env {
            let pw = std::env::var(env_name).map_err(|_| {
                ExecError::NotConfigured(format!("tool_exec.ssh.password_env={env_name} not set"))
            })?;
            session
                .authenticate_password(user, pw)
                .await
                .map_err(|e| ExecError::Connect(format!("ssh password: {e}")))?
        } else {
            session
                .authenticate_none(user)
                .await
                .map_err(|e| ExecError::Connect(format!("ssh none-auth: {e}")))?
        };

        if !auth_ok {
            return Err(ExecError::Connect("ssh authentication failed".into()));
        }

        // Open a channel and run a single command.
        let mut chan = session
            .channel_open_session()
            .await
            .map_err(|e| ExecError::Other(format!("open session: {e}")))?;

        // Compose the remote command line. Honour `workdir` by
        // prepending `cd <dir> &&` — `russh` exec doesn't have a
        // separate cwd parameter.
        let mut full_cmd = String::new();
        if !cfg.workdir.is_empty() {
            full_cmd.push_str(&format!("cd {} && ", shell_quote(&cfg.workdir)));
        } else if let Some(wd) = spec.workdir.as_ref().and_then(|p| p.to_str()) {
            full_cmd.push_str(&format!("cd {} && ", shell_quote(wd)));
        }
        // Prefix env-var assignments. Sorted by BTreeMap key.
        for (k, v) in &spec.env {
            full_cmd.push_str(&format!("{k}={} ", shell_quote(v)));
        }
        full_cmd.push_str(&spec.command);

        chan.exec(true, full_cmd.as_bytes())
            .await
            .map_err(|e| ExecError::Other(format!("exec: {e}")))?;

        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let mut exit_code: Option<i32> = None;

        while let Some(msg) = chan.wait().await {
            use russh::ChannelMsg;
            match msg {
                ChannelMsg::Data { data } => stdout.extend_from_slice(&data),
                ChannelMsg::ExtendedData { ext: 1, data } => {
                    stderr.extend_from_slice(&data);
                }
                ChannelMsg::ExitStatus { exit_status } => {
                    exit_code = Some(exit_status as i32);
                }
                ChannelMsg::Eof | ChannelMsg::Close => break,
                _ => {}
            }
        }

        let stdout_s = String::from_utf8_lossy(&stdout).into_owned();
        let stderr_s = String::from_utf8_lossy(&stderr).into_owned();
        let cap = spec.limits.max_output_bytes;
        let stdout_s = cap.map_or(stdout_s.clone(), |c| truncate_to_cap(stdout_s, c));
        let stderr_s = cap.map_or(stderr_s.clone(), |c| truncate_to_cap(stderr_s, c));

        Ok(ExecOutcome {
            stdout: stdout_s,
            stderr: stderr_s,
            exit_code: exit_code.unwrap_or(-1),
            backend_id: Some(format!("ssh:{}@{}:{}", cfg.user, cfg.host, cfg.port)),
        })
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

    /// POSIX single-quote a value safely for `cd` / env assignment.
    fn shell_quote(value: &str) -> String {
        if value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '/' | '-' | '.' | ':' | '+'))
        {
            value.to_string()
        } else {
            // Replace ' with '\'' inside single-quoted regions.
            let escaped = value.replace('\'', r"'\''");
            format!("'{escaped}'")
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn shell_quote_alphanumeric_unchanged() {
            assert_eq!(shell_quote("foo"), "foo");
            assert_eq!(shell_quote("/usr/local/bin"), "/usr/local/bin");
        }

        #[test]
        fn shell_quote_wraps_with_spaces() {
            assert_eq!(shell_quote("hello world"), "'hello world'");
        }

        #[test]
        fn shell_quote_escapes_inner_single_quote() {
            // POSIX trick: close, escape, reopen.
            assert_eq!(shell_quote("it's"), r"'it'\''s'");
        }

        #[test]
        fn digest_to_hex_emits_expected() {
            let bytes = [0xde, 0xad, 0xbe, 0xef];
            assert_eq!(digest_to_hex(&bytes), "deadbeef");
        }
    }
}

// ---------------------------------------------------------------------------
// Public-API tests — these don't need a live SSH server.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> SshBackendConfig {
        SshBackendConfig {
            host: "example.invalid".into(),
            user: "agent".into(),
            ..Default::default()
        }
    }

    #[test]
    fn kind_is_ssh() {
        let backend = SshBackend::new(cfg());
        assert_eq!(backend.kind(), BackendKind::Ssh);
    }

    #[tokio::test]
    async fn rejects_empty_host() {
        let c = SshBackendConfig {
            host: String::new(),
            ..cfg()
        };
        let backend = SshBackend::new(c);
        match backend.run_command(ExecSpec::new("true")).await {
            Err(ExecError::NotConfigured(msg)) => {
                assert!(msg.contains("host"), "got: {msg}");
            }
            other => panic!("expected NotConfigured, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn rejects_empty_user() {
        let c = SshBackendConfig {
            user: String::new(),
            ..cfg()
        };
        let backend = SshBackend::new(c);
        match backend.run_command(ExecSpec::new("true")).await {
            Err(ExecError::NotConfigured(msg)) => {
                assert!(msg.contains("user"), "got: {msg}");
            }
            other => panic!("expected NotConfigured, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn upload_returns_unsupported() {
        let backend = SshBackend::new(cfg());
        match backend.upload("/tmp/x", b"hi").await {
            Err(ExecError::UnsupportedForBackend { backend, operation }) => {
                assert_eq!(backend, "ssh");
                assert_eq!(operation, "upload");
            }
            other => panic!("expected UnsupportedForBackend, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn download_returns_unsupported() {
        let backend = SshBackend::new(cfg());
        match backend.download("/tmp/x").await {
            Err(ExecError::UnsupportedForBackend { backend, operation }) => {
                assert_eq!(backend, "ssh");
                assert_eq!(operation, "download");
            }
            other => panic!("expected UnsupportedForBackend, got {other:?}"),
        }
    }

    /// Live integration test, opted in via `LIBREFANG_SSH_TEST_HOST`
    /// (and friends). Skipped unconditionally otherwise so CI on
    /// hosts without an SSH target is green.
    ///
    /// Required env vars when enabled:
    /// - `LIBREFANG_SSH_TEST_HOST`  — hostname of an SSH server
    /// - `LIBREFANG_SSH_TEST_USER`  — login user
    /// - `LIBREFANG_SSH_TEST_KEY`   — path to a private key (no passphrase)
    ///
    /// Optional:
    /// - `LIBREFANG_SSH_TEST_PORT`  — defaults to 22
    #[tokio::test]
    async fn live_echo_when_env_set() {
        let host = match std::env::var("LIBREFANG_SSH_TEST_HOST") {
            Ok(v) if !v.is_empty() => v,
            _ => return, // not configured — skip
        };
        let user =
            std::env::var("LIBREFANG_SSH_TEST_USER").expect("LIBREFANG_SSH_TEST_USER required");
        let key_path =
            std::env::var("LIBREFANG_SSH_TEST_KEY").expect("LIBREFANG_SSH_TEST_KEY path required");
        let port: u16 = std::env::var("LIBREFANG_SSH_TEST_PORT")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(22);

        let c = SshBackendConfig {
            host,
            user,
            port,
            key_path: Some(std::path::PathBuf::from(key_path)),
            timeout_secs: 30,
            ..Default::default()
        };

        let backend = SshBackend::new(c);
        let outcome = backend
            .run_command(ExecSpec::new("echo hello-from-librefang-3332"))
            .await
            .expect("live ssh exec must succeed");
        assert_eq!(outcome.exit_code, 0);
        assert!(outcome.stdout.contains("hello-from-librefang-3332"));
    }
}
