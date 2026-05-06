//! Configuration types for the pluggable tool-execution backend.
//!
//! Issue #3332 — adds support for executing tool commands on remote and
//! managed hosts (SSH, Daytona, …) instead of always running them as a
//! subprocess on the local daemon. The trait surface and concrete
//! backend implementations live in the `librefang-runtime` crate; this
//! module is type-only so the agent manifest (`AgentManifest`) and the
//! kernel config (`KernelConfig`) — both rooted in `librefang-types` —
//! can carry per-agent / global selection without pulling in any
//! runtime dependencies.
//!
//! Resolution order (highest priority first):
//! 1. Per-agent override on the manifest (`AgentManifest.tool_exec_backend`).
//! 2. Global override in `config.toml` (`KernelConfig.tool_exec`).
//! 3. Compiled-in default — `BackendKind::Local`.
//!
//! See `docs/architecture/tool-exec-backends.md`.

use serde::{Deserialize, Serialize};

/// Identifier for a tool-execution backend.
///
/// `Local` is the long-standing default — commands run as a subprocess
/// on the daemon host, sandboxed via `subprocess_sandbox`. The other
/// variants select managed or remote hosts; their concrete impls live
/// behind feature flags in the runtime crate.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "lowercase")]
pub enum BackendKind {
    /// Local subprocess (default). Always available.
    #[default]
    Local,
    /// Local Docker container. Routed through the existing `[docker]`
    /// sandbox config — kept as a distinct kind so per-agent selection
    /// can prefer Docker without needing the global `mode = all` flag.
    Docker,
    /// Remote SSH host. Requires the runtime to be built with the
    /// `ssh-backend` feature.
    Ssh,
    /// Daytona managed sandbox. Requires the runtime to be built with
    /// the `daytona-backend` feature.
    Daytona,
}

impl BackendKind {
    /// Human-readable name suitable for logging and error messages.
    pub fn as_str(self) -> &'static str {
        match self {
            BackendKind::Local => "local",
            BackendKind::Docker => "docker",
            BackendKind::Ssh => "ssh",
            BackendKind::Daytona => "daytona",
        }
    }
}

/// Top-level tool-execution backend configuration in `config.toml`.
///
/// ```toml
/// [tool_exec]
/// kind = "local"   # default
///
/// # [tool_exec.ssh]
/// # host = "build.example.com"
/// # user = "agent"
/// # key_path = "/home/me/.ssh/id_ed25519"
///
/// # [tool_exec.daytona]
/// # api_url = "https://app.daytona.io"
/// # api_key_env = "DAYTONA_API_KEY"
/// ```
///
/// `kind` selects the active backend; the matching sub-table carries
/// the connection knobs. Inactive sub-tables are ignored — keeping
/// stale config around is fine.
#[derive(Debug, Clone, Default, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct ToolExecConfig {
    /// Active backend kind. Default: `local`.
    pub kind: BackendKind,
    /// SSH-backend connection knobs. Required when `kind = "ssh"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ssh: Option<SshBackendConfig>,
    /// Daytona-backend knobs. Required when `kind = "daytona"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub daytona: Option<DaytonaBackendConfig>,
}

/// SSH backend connection configuration.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct SshBackendConfig {
    /// Hostname or IP of the remote machine.
    pub host: String,
    /// SSH port. Default: 22.
    pub port: u16,
    /// Login user. Default: empty (must be set when backend is active).
    pub user: String,
    /// Path to a private key on disk. Mutually exclusive with `password_env`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key_path: Option<std::path::PathBuf>,
    /// Environment variable holding the SSH password. Falls back to
    /// passwordless keyless auth if neither this nor `key_path` is set
    /// (useful only for hosts with `~/.ssh/authorized_keys` already
    /// holding the daemon's key).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub password_env: Option<String>,
    /// Optional command timeout in seconds. Default: 60.
    pub timeout_secs: u64,
    /// Optional working directory on the remote host. Empty = remote `$HOME`.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub workdir: String,
    /// SHA-256 hex of the expected server host key. When set, the
    /// backend refuses to connect if the server presents a different
    /// key (TOFU-style pinning). When empty, the backend logs the key
    /// fingerprint and accepts on first connect — fine for local LANs
    /// but should be tightened in production.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub host_key_sha256: String,
}

impl Default for SshBackendConfig {
    fn default() -> Self {
        Self {
            host: String::new(),
            port: 22,
            user: String::new(),
            key_path: None,
            password_env: None,
            timeout_secs: 60,
            workdir: String::new(),
            host_key_sha256: String::new(),
        }
    }
}

/// Daytona-managed-sandbox backend configuration.
///
/// Daytona exposes a REST API for ephemeral developer sandboxes. We
/// authenticate with a bearer token stored in an env var (so the
/// daemon never persists the secret), create a workspace per agent
/// session, and execute commands via the workspace's exec endpoint.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct DaytonaBackendConfig {
    /// Daytona API base URL. Default: `https://app.daytona.io`.
    pub api_url: String,
    /// Environment variable holding the API key.
    /// Default: `DAYTONA_API_KEY`.
    pub api_key_env: String,
    /// Default sandbox image. Default: `ubuntu:22.04`.
    pub image: String,
    /// Per-command timeout in seconds. Default: 120.
    pub timeout_secs: u64,
    /// Workspace name prefix (Daytona-side). Default: `librefang`.
    pub workspace_prefix: String,
}

impl Default for DaytonaBackendConfig {
    fn default() -> Self {
        Self {
            api_url: "https://app.daytona.io".to_string(),
            api_key_env: "DAYTONA_API_KEY".to_string(),
            image: "ubuntu:22.04".to_string(),
            timeout_secs: 120,
            workspace_prefix: "librefang".to_string(),
        }
    }
}

/// Resolve which backend a given agent should use given the manifest
/// override and the global config.
///
/// `manifest_override` mirrors `AgentManifest.tool_exec_backend`;
/// `global_kind` mirrors `ToolExecConfig.kind` from `KernelConfig`.
///
/// Returns the chosen kind. The caller is responsible for materialising
/// the actual backend impl from the matching config sub-table.
pub fn resolve_backend_kind(
    manifest_override: Option<BackendKind>,
    global_kind: BackendKind,
) -> BackendKind {
    manifest_override.unwrap_or(global_kind)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backend_kind_default_is_local() {
        assert_eq!(BackendKind::default(), BackendKind::Local);
    }

    #[test]
    fn backend_kind_as_str_round_trip() {
        for kind in [
            BackendKind::Local,
            BackendKind::Docker,
            BackendKind::Ssh,
            BackendKind::Daytona,
        ] {
            let s = kind.as_str();
            let parsed: BackendKind = serde_json::from_str(&format!("\"{s}\"")).unwrap();
            assert_eq!(parsed, kind);
        }
    }

    #[test]
    fn resolver_prefers_manifest_override() {
        assert_eq!(
            resolve_backend_kind(Some(BackendKind::Ssh), BackendKind::Local),
            BackendKind::Ssh
        );
        assert_eq!(
            resolve_backend_kind(Some(BackendKind::Daytona), BackendKind::Docker),
            BackendKind::Daytona
        );
    }

    #[test]
    fn resolver_falls_back_to_global() {
        assert_eq!(
            resolve_backend_kind(None, BackendKind::Docker),
            BackendKind::Docker
        );
        assert_eq!(
            resolve_backend_kind(None, BackendKind::Ssh),
            BackendKind::Ssh
        );
    }

    #[test]
    fn resolver_default_when_both_unset() {
        // Caller passes BackendKind::default() as global when config didn't
        // specify a kind — must come out as Local.
        assert_eq!(
            resolve_backend_kind(None, BackendKind::default()),
            BackendKind::Local
        );
    }

    #[test]
    fn tool_exec_config_default_is_local_no_subtables() {
        let cfg = ToolExecConfig::default();
        assert_eq!(cfg.kind, BackendKind::Local);
        assert!(cfg.ssh.is_none());
        assert!(cfg.daytona.is_none());
    }

    #[test]
    fn tool_exec_config_toml_roundtrip_local() {
        let cfg = ToolExecConfig::default();
        let toml_str = toml::to_string(&cfg).unwrap();
        let back: ToolExecConfig = toml::from_str(&toml_str).unwrap();
        assert_eq!(back.kind, BackendKind::Local);
    }

    #[test]
    fn tool_exec_config_toml_ssh_section() {
        let toml_str = r#"
            kind = "ssh"
            [ssh]
            host = "build.example.com"
            port = 2222
            user = "agent"
            key_path = "/home/me/.ssh/id_ed25519"
            timeout_secs = 30
        "#;
        let cfg: ToolExecConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(cfg.kind, BackendKind::Ssh);
        let ssh = cfg.ssh.unwrap();
        assert_eq!(ssh.host, "build.example.com");
        assert_eq!(ssh.port, 2222);
        assert_eq!(ssh.user, "agent");
        assert_eq!(ssh.timeout_secs, 30);
        assert_eq!(
            ssh.key_path
                .as_deref()
                .map(|p| p.to_string_lossy().into_owned()),
            Some("/home/me/.ssh/id_ed25519".to_string())
        );
        assert!(ssh.password_env.is_none());
    }

    #[test]
    fn tool_exec_config_toml_daytona_section() {
        let toml_str = r#"
            kind = "daytona"
            [daytona]
            api_url = "https://daytona.example.com"
            api_key_env = "MY_DAYTONA_KEY"
            image = "python:3.12"
            timeout_secs = 90
        "#;
        let cfg: ToolExecConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(cfg.kind, BackendKind::Daytona);
        let dt = cfg.daytona.unwrap();
        assert_eq!(dt.api_url, "https://daytona.example.com");
        assert_eq!(dt.api_key_env, "MY_DAYTONA_KEY");
        assert_eq!(dt.image, "python:3.12");
        assert_eq!(dt.timeout_secs, 90);
        assert_eq!(dt.workspace_prefix, "librefang"); // default kept
    }

    #[test]
    fn ssh_backend_config_default_port_22() {
        let cfg = SshBackendConfig::default();
        assert_eq!(cfg.port, 22);
        assert_eq!(cfg.timeout_secs, 60);
    }

    #[test]
    fn daytona_backend_config_defaults() {
        let cfg = DaytonaBackendConfig::default();
        assert_eq!(cfg.api_url, "https://app.daytona.io");
        assert_eq!(cfg.api_key_env, "DAYTONA_API_KEY");
        assert_eq!(cfg.image, "ubuntu:22.04");
        assert_eq!(cfg.timeout_secs, 120);
        assert_eq!(cfg.workspace_prefix, "librefang");
    }
}
