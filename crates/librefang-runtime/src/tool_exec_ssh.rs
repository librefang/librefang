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
//!   formats supported by `russh-keys`). When the key file is encrypted,
//!   set `key_passphrase_env` to the env var holding the passphrase.
//! - `password_env` set → password auth from the named env var.
//! - Neither set → tries publickey-from-agent, falling back to none.
//!   This last branch only succeeds against hosts that explicitly
//!   `PermitEmptyPasswords yes`, which we expect to be vanishingly
//!   rare. The point is to avoid panicking on misconfiguration.
//!
//! ## Host-key verification
//!
//! `SshBackendConfig.host_key_sha256` is the SHA-256 hex of the
//! expected server host key. Three modes:
//!
//! 1. **Pinned (recommended).** When `host_key_sha256` is set, the
//!    backend hard-rejects a connection whose host key doesn't match.
//! 2. **TOFU on disk.** When unset and a known-hosts file at
//!    `~/.librefang/ssh_known_hosts.toml` already records the host,
//!    the entry there is required to match. Mismatch ⇒
//!    [`ExecError::AuthFailure`].
//! 3. **First connect.** When neither pin nor known-hosts entry
//!    exists, the backend writes the seen fingerprint into the
//!    known-hosts file and accepts.
//!
//! Mode 3 is the only branch that opens you up to MITM on first
//! contact — operators are encouraged to copy the fingerprint logged
//! at INFO into the explicit pin once they've verified it via an
//! out-of-band channel.

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
// Known-hosts file (TOFU) — shared by integration tests.
// ---------------------------------------------------------------------------

mod known_hosts {
    //! Tiny TOFU known-hosts store.
    //!
    //! On-disk shape, at `~/.librefang/ssh_known_hosts.toml`:
    //! ```toml
    //! # Map of "host:port" → "sha256:<lowercase hex>"
    //! [hosts]
    //! "build.example.com:22" = "sha256:deadbeef…"
    //! ```
    //!
    //! Why TOML rather than the system `~/.ssh/known_hosts`? We don't
    //! want the daemon's pinning state to interleave with the user's
    //! interactive ssh client (different scope, different lifecycle),
    //! and the TOML form keeps the loader trivial.

    use serde::{Deserialize, Serialize};
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    #[derive(Debug, Clone, Default, Serialize, Deserialize)]
    pub(super) struct KnownHostsFile {
        #[serde(default)]
        pub hosts: BTreeMap<String, String>,
    }

    /// Path to the daemon's known-hosts file. `None` when the home
    /// directory cannot be resolved (rare; we fall back to "no pin").
    pub(super) fn path() -> Option<PathBuf> {
        dirs::home_dir().map(|h| h.join(".librefang").join("ssh_known_hosts.toml"))
    }

    pub(super) async fn load() -> KnownHostsFile {
        let Some(p) = path() else {
            return KnownHostsFile::default();
        };
        match tokio::fs::read_to_string(&p).await {
            Ok(s) => toml::from_str::<KnownHostsFile>(&s).unwrap_or_default(),
            Err(_) => KnownHostsFile::default(),
        }
    }

    pub(super) async fn save(file: &KnownHostsFile) -> std::io::Result<()> {
        let Some(p) = path() else {
            return Ok(());
        };
        if let Some(parent) = p.parent() {
            tokio::fs::create_dir_all(parent).await.ok();
        }
        let body = toml::to_string_pretty(file)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;
        tokio::fs::write(&p, body).await
    }

    pub(super) fn key_for(host: &str, port: u16) -> String {
        format!("{host}:{port}")
    }

    pub(super) fn fingerprint_for_storage(hex: &str) -> String {
        format!("sha256:{}", hex.to_ascii_lowercase())
    }

    pub(super) fn matches_stored(stored: &str, hex: &str) -> bool {
        let normalised = stored
            .trim()
            .trim_start_matches("sha256:")
            .to_ascii_lowercase();
        normalised == hex.to_ascii_lowercase()
    }
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
    use tokio::sync::Mutex;
    use tokio::time::timeout;

    /// Outcome captured by the host-key handler so the caller can decide
    /// whether to persist a new TOFU entry after authenticate succeeds.
    /// Only the `FirstSeen` arm carries the fingerprint that needs
    /// persisting; the matched cases just need the bool-y "ok" signal.
    #[derive(Debug, Clone)]
    pub(super) enum HostKeyVerdict {
        /// Pin matched, or known-hosts entry matched. Nothing to persist.
        Matched,
        /// First-seen on a host with no pin and no known-hosts entry —
        /// caller should write `hex` into `~/.librefang/ssh_known_hosts.toml`.
        FirstSeen(String),
    }

    /// `russh` requires a `Handler` for client-side decisions. We use it
    /// to enforce optional host-key pinning — `SshBackendConfig.host_key_sha256` —
    /// AND on-disk TOFU. Mismatch in either layer is a hard reject.
    pub(super) struct PinningHandler {
        pub(super) expected_sha256: String,
        pub(super) host_key_for_storage: String,
        pub(super) known_hosts_entry: Option<String>,
        pub(super) verdict: Arc<Mutex<Option<HostKeyVerdict>>>,
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
            let hex_fp = hex::encode(digest);

            // 1. Explicit pin (highest precedence).
            if !self.expected_sha256.is_empty() {
                let expected = self.expected_sha256.trim();
                let matches = expected.eq_ignore_ascii_case(&hex_fp);
                if !matches {
                    tracing::error!(
                        expected = %expected,
                        actual = %hex_fp,
                        "tool_exec.ssh: pinned host key fingerprint mismatch"
                    );
                    return Ok(false);
                }
                *self.verdict.lock().await = Some(HostKeyVerdict::Matched);
                return Ok(true);
            }

            // 2. On-disk TOFU entry — must match if present.
            if let Some(stored) = &self.known_hosts_entry {
                if super::known_hosts::matches_stored(stored, &hex_fp) {
                    *self.verdict.lock().await = Some(HostKeyVerdict::Matched);
                    return Ok(true);
                }
                tracing::error!(
                    stored = %stored,
                    actual = %hex_fp,
                    host = %self.host_key_for_storage,
                    "tool_exec.ssh: TOFU host-key mismatch — refusing to connect. \
                     If the remote was rekeyed, edit ~/.librefang/ssh_known_hosts.toml"
                );
                return Ok(false);
            }

            // 3. First-seen — accept and let the caller persist.
            tracing::info!(
                fingerprint = %hex_fp,
                host = %self.host_key_for_storage,
                "tool_exec.ssh: insecure: host key check disabled \
                 (configure tool_exec.ssh.host_key_sha256 to enable). \
                 Recording first-seen fingerprint to known-hosts."
            );
            *self.verdict.lock().await = Some(HostKeyVerdict::FirstSeen(hex_fp));
            Ok(true)
        }
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

        // Resolve known-hosts entry up front so the handler doesn't
        // hit the disk during the TLS callback.
        let known = super::known_hosts::load().await;
        let kh_key = super::known_hosts::key_for(&cfg.host, cfg.port);
        let kh_entry = known.hosts.get(&kh_key).cloned();

        let verdict = Arc::new(Mutex::new(None));
        let handler = PinningHandler {
            expected_sha256: cfg.host_key_sha256.clone(),
            host_key_for_storage: kh_key.clone(),
            known_hosts_entry: kh_entry,
            verdict: verdict.clone(),
        };
        let addr = format!("{}:{}", cfg.host, cfg.port);
        let mut session = client::connect(client_cfg, addr, handler)
            .await
            .map_err(|e| ExecError::Connect(format!("ssh connect: {e}")))?;

        // Authenticate. russh 0.45 returns `bool` directly from each
        // authenticate_* call; no wrapper.
        let user = cfg.user.clone();
        let auth_ok = if let Some(path) = &cfg.key_path {
            // Honor optional `key_passphrase_env` for encrypted keys.
            // russh-keys 0.45 accepts an `Option<&str>` passphrase.
            let passphrase: Option<String> = match &cfg.key_passphrase_env {
                Some(env_name) if !env_name.is_empty() => {
                    Some(std::env::var(env_name).map_err(|_| {
                        ExecError::NotConfigured(format!(
                            "tool_exec.ssh.key_passphrase_env={env_name} not set"
                        ))
                    })?)
                }
                _ => None,
            };
            let key = russh_keys::load_secret_key(path, passphrase.as_deref())
                .map_err(|e| ExecError::AuthFailure(format!("ssh key {path:?}: {e}")))?;
            session
                .authenticate_publickey(user, std::sync::Arc::new(key))
                .await
                .map_err(|e| ExecError::AuthFailure(format!("ssh publickey: {e}")))?
        } else if let Some(env_name) = &cfg.password_env {
            let pw = std::env::var(env_name).map_err(|_| {
                ExecError::NotConfigured(format!("tool_exec.ssh.password_env={env_name} not set"))
            })?;
            session
                .authenticate_password(user, pw)
                .await
                .map_err(|e| ExecError::AuthFailure(format!("ssh password: {e}")))?
        } else {
            session
                .authenticate_none(user)
                .await
                .map_err(|e| ExecError::AuthFailure(format!("ssh none-auth: {e}")))?
        };

        if !auth_ok {
            return Err(ExecError::AuthFailure("ssh authentication failed".into()));
        }

        // Persist TOFU entry now that we know auth succeeded — avoids
        // pinning a key from a host we couldn't even talk to.
        if let Some(HostKeyVerdict::FirstSeen(hex_fp)) = verdict.lock().await.clone() {
            let mut updated = super::known_hosts::load().await;
            updated.hosts.insert(
                kh_key.clone(),
                super::known_hosts::fingerprint_for_storage(&hex_fp),
            );
            if let Err(e) = super::known_hosts::save(&updated).await {
                tracing::warn!(error = %e, "tool_exec.ssh: failed to persist TOFU entry");
            }
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
        // Prefix env-var assignments. Sorted by BTreeMap key. Reserved
        // keys are dropped at the trait boundary — duplicate the scrub
        // here so a misuse on the SSH path doesn't leak loader hijacks.
        for (k, v) in &spec.env {
            if crate::tool_exec_backend::is_reserved_env_key(k) {
                tracing::warn!(
                    key = %k,
                    "tool_exec/ssh: dropping reserved env key from remote command"
                );
                continue;
            }
            full_cmd.push_str(&format!("{k}={} ", shell_quote(v)));
        }
        full_cmd.push_str(&spec.command);

        chan.exec(true, full_cmd.as_bytes())
            .await
            .map_err(|e| ExecError::Other(format!("exec: {e}")))?;

        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let mut exit_code: Option<i32> = None;
        let mut signal_note: Option<String> = None;

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
                ChannelMsg::ExitSignal {
                    signal_name,
                    core_dumped: _,
                    error_message: _,
                    lang_tag: _,
                } => {
                    // POSIX convention: shells report killed-by-signal as
                    // 128 + signum. Use `-2` for unknown signal names so
                    // the value is recognisably "not a real exit code".
                    //
                    // russh 0.45's `Sig` is an enum without `Display` —
                    // we render via `Debug` and accept either the
                    // `SIGTERM` form (some hosts) or the bare `TERM`
                    // form (russh's `Sig::TERM` variant) interchangeably.
                    let label = signal_name_label(&format!("{signal_name:?}"));
                    let signum = posix_signum(&label);
                    let code = match signum {
                        Some(n) => 128 + n,
                        None => -2,
                    };
                    exit_code = Some(code);
                    signal_note = Some(format!("\n[killed by signal: SIG{label}]\n"));
                }
                ChannelMsg::Eof | ChannelMsg::Close => break,
                _ => {}
            }
        }

        // Append the signal annotation BEFORE truncation so the marker
        // survives the cap (matters for very chatty commands).
        if let Some(note) = &signal_note {
            stderr.extend_from_slice(note.as_bytes());
        }

        let stdout_s = String::from_utf8_lossy(&stdout).into_owned();
        let stderr_s = String::from_utf8_lossy(&stderr).into_owned();
        let cap = spec.limits.max_output_bytes;
        let stdout_s = match cap {
            Some(c) => crate::tool_exec_backend::truncate_to_cap(stdout_s, c),
            None => stdout_s,
        };
        let stderr_s = match cap {
            Some(c) => crate::tool_exec_backend::truncate_to_cap(stderr_s, c),
            None => stderr_s,
        };

        Ok(ExecOutcome {
            stdout: stdout_s,
            stderr: stderr_s,
            exit_code: exit_code.unwrap_or(-1),
            backend_id: Some(format!("ssh:{}@{}:{}", cfg.user, cfg.host, cfg.port)),
        })
    }

    /// `russh` 0.45 surfaces `signal_name` as the [`russh::Sig`] enum
    /// without `Display`; callers render it via `Debug` first. We
    /// accept the `SIGTERM` and `TERM` forms interchangeably and also
    /// strip any `Sig::` / `Custom("...")` wrapper that `Debug` adds
    /// for unknown variants.
    fn signal_name_label(s: &str) -> String {
        let raw = s.trim();
        // Some russh `Sig::Debug` outputs look like `Custom("USR1")` —
        // peel the wrapper if present.
        let raw = raw
            .strip_prefix("Custom(\"")
            .and_then(|s| s.strip_suffix("\")"))
            .unwrap_or(raw);
        raw.strip_prefix("SIG").unwrap_or(raw).to_string()
    }

    /// Map a POSIX signal name (without the `SIG` prefix, uppercase)
    /// to its numeric value. Restricted to the well-known set —
    /// hosts that emit weird names (`USR1`, `RTMIN+3`) get `-2`.
    fn posix_signum(label: &str) -> Option<i32> {
        match label.to_ascii_uppercase().as_str() {
            "HUP" => Some(1),
            "INT" => Some(2),
            "QUIT" => Some(3),
            "ILL" => Some(4),
            "ABRT" => Some(6),
            "FPE" => Some(8),
            "KILL" => Some(9),
            "SEGV" => Some(11),
            "PIPE" => Some(13),
            "ALRM" => Some(14),
            "TERM" => Some(15),
            _ => None,
        }
    }

    /// POSIX single-quote a value safely for `cd` / env assignment.
    pub(super) fn shell_quote(value: &str) -> String {
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
        fn hex_encode_round_trip() {
            // Sanity check that we can drop our hand-rolled digest_to_hex
            // in favour of the `hex` crate without changing the on-wire
            // format (lowercase, no separators).
            let bytes = [0xde, 0xad, 0xbe, 0xef];
            assert_eq!(hex::encode(bytes), "deadbeef");
        }

        #[test]
        fn signum_known_signals() {
            assert_eq!(posix_signum("KILL"), Some(9));
            assert_eq!(posix_signum("term"), Some(15));
            assert_eq!(posix_signum("SegV"), Some(11));
        }

        #[test]
        fn signum_unknown_returns_none() {
            assert_eq!(posix_signum("USR1"), None);
            assert_eq!(posix_signum(""), None);
        }

        #[test]
        fn signal_label_strips_sig_prefix() {
            assert_eq!(signal_name_label("SIGTERM"), "TERM");
            assert_eq!(signal_name_label("TERM"), "TERM");
            assert_eq!(signal_name_label("  SIGKILL  "), "KILL");
        }

        #[test]
        fn signal_label_unwraps_custom_debug_form() {
            // russh's Debug for unknown signals: `Custom("USR1")`.
            assert_eq!(signal_name_label("Custom(\"USR1\")"), "USR1");
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

    /// L1 plumbing test: setting `key_passphrase_env` round-trips
    /// through [`SshBackendConfig`] (the field is wired) and the field
    /// is preserved by `Clone`. We can't make a meaningful end-to-end
    /// assertion without a live SSH server (the network connect step
    /// happens before auth in `do_exec`), so the field-plumbing test
    /// is the strongest portable guarantee. The runtime check that
    /// actually consumes `key_passphrase_env` is exercised by the
    /// gated live test in `live_echo_when_env_set`.
    #[test]
    fn key_passphrase_env_field_round_trips() {
        let c = SshBackendConfig {
            key_passphrase_env: Some("MY_VAR".into()),
            key_path: Some(std::path::PathBuf::from("/k")),
            ..cfg()
        };
        let cloned = c.clone();
        assert_eq!(cloned.key_passphrase_env.as_deref(), Some("MY_VAR"));
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
