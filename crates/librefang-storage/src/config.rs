//! Serialisable configuration types for the storage subsystem.
//!
//! These types are referenced from `librefang-types::KernelConfig` so they
//! must round-trip through TOML and JSON.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Default SurrealDB namespace for librefang's operational tables.
///
/// Codified as a constant so the wizard, settings UI, CLI provisioning, and
/// the runtime all agree.
pub const DEFAULT_NAMESPACE_NAME: &str = "librefang";

/// Default SurrealDB database name (within the librefang namespace).
pub const DEFAULT_DATABASE_NAME: &str = "main";

/// Where the storage backend lives.
///
/// `Embedded` uses the bundled RocksDB engine and is the default chosen by
/// the wizard. `Remote` connects to an external SurrealDB 3.0 instance and
/// can be shared with the Universal Agent Runtime via separate namespaces.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum StorageBackendKind {
    /// Single-process embedded SurrealDB. Cannot be shared between
    /// processes — see surreal-memory-server's CLAUDE.md.
    Embedded {
        /// Filesystem path of the RocksDB directory.
        path: PathBuf,
    },
    /// Remote SurrealDB 3.0 instance reachable over WebSocket or HTTP.
    Remote(RemoteSurrealConfig),
}

impl StorageBackendKind {
    /// Convenience constructor for the wizard's default-embedded path.
    #[must_use]
    pub fn embedded(path: impl Into<PathBuf>) -> Self {
        Self::Embedded { path: path.into() }
    }

    /// `true` when the backend is the embedded RocksDB engine.
    #[must_use]
    pub fn is_embedded(&self) -> bool {
        matches!(self, Self::Embedded { .. })
    }

    /// `true` when the backend is a remote SurrealDB instance.
    #[must_use]
    pub fn is_remote(&self) -> bool {
        matches!(self, Self::Remote(_))
    }
}

/// Connection details for a remote SurrealDB 3.0 instance.
///
/// The same struct is reused by [`crate::config::StorageConfig`] (for
/// librefang's own session) and by `librefang_types::config::UarConfig`
/// (when UAR is configured to reuse the same instance).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RemoteSurrealConfig {
    /// Endpoint URL — `ws://`, `wss://`, `http://`, or `https://`.
    pub url: String,
    /// SurrealDB namespace (top-level tenant).
    pub namespace: String,
    /// SurrealDB database (per-namespace logical store).
    pub database: String,
    /// Username for namespace- or database-level authentication.
    pub username: String,
    /// Name of the environment variable (or `vault:` reference) that holds
    /// the password. We never persist the password itself in config files.
    pub password_env: String,
    /// Skip TLS certificate verification. **Never** enable in production.
    #[serde(default)]
    pub tls_skip_verify: bool,
}

impl RemoteSurrealConfig {
    /// Convenience constructor with the librefang defaults filled in.
    #[must_use]
    pub fn librefang(
        url: impl Into<String>,
        username: impl Into<String>,
        password_env: impl Into<String>,
    ) -> Self {
        Self {
            url: url.into(),
            namespace: DEFAULT_NAMESPACE_NAME.to_string(),
            database: DEFAULT_DATABASE_NAME.to_string(),
            username: username.into(),
            password_env: password_env.into(),
            tls_skip_verify: false,
        }
    }
}

/// Top-level storage configuration on `KernelConfig`.
///
/// Defaults to embedded SurrealDB at `<data_dir>/librefang.surreal`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StorageConfig {
    /// Which backend to use at startup.
    pub backend: StorageBackendKind,
    /// Default namespace when [`Self::backend`] is `Embedded`. For `Remote`
    /// the namespace inside the [`RemoteSurrealConfig`] takes precedence.
    pub namespace: String,
    /// Default database when [`Self::backend`] is `Embedded`.
    pub database: String,
    /// Optional path to a legacy `librefang.db` SQLite file. When set, the
    /// `storage migrate` CLI / `POST /api/storage/migrate` endpoint can
    /// stream rows from it into the active SurrealDB store.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub legacy_sqlite_path: Option<PathBuf>,
}

impl StorageConfig {
    /// Build the wizard default: embedded SurrealDB inside `data_dir`.
    #[must_use]
    pub fn embedded_default(data_dir: impl Into<PathBuf>) -> Self {
        let mut path = data_dir.into();
        path.push("librefang.surreal");
        Self {
            backend: StorageBackendKind::embedded(path),
            namespace: DEFAULT_NAMESPACE_NAME.to_string(),
            database: DEFAULT_DATABASE_NAME.to_string(),
            legacy_sqlite_path: None,
        }
    }

    /// Effective namespace for the librefang session (remote overrides
    /// embedded when present).
    #[must_use]
    pub fn effective_namespace(&self) -> &str {
        match &self.backend {
            StorageBackendKind::Remote(remote) => remote.namespace.as_str(),
            StorageBackendKind::Embedded { .. } => self.namespace.as_str(),
        }
    }

    /// Effective database for the librefang session.
    #[must_use]
    pub fn effective_database(&self) -> &str {
        match &self.backend {
            StorageBackendKind::Remote(remote) => remote.database.as_str(),
            StorageBackendKind::Embedded { .. } => self.database.as_str(),
        }
    }
}

impl Default for StorageConfig {
    fn default() -> Self {
        // Conservative default: a relative path so callers that build a
        // `KernelConfig::default()` don't accidentally pick up someone
        // else's data dir. The wizard rewrites this with the real
        // `data_dir` on first run.
        Self::embedded_default(PathBuf::from(".librefang"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_default_uses_namespace_constants() {
        let cfg = StorageConfig::embedded_default("/var/lib/librefang");
        assert_eq!(cfg.namespace, DEFAULT_NAMESPACE_NAME);
        assert_eq!(cfg.database, DEFAULT_DATABASE_NAME);
        assert!(cfg.backend.is_embedded());
        assert!(!cfg.backend.is_remote());
    }

    #[test]
    fn remote_overrides_namespace_database() {
        let cfg = StorageConfig {
            backend: StorageBackendKind::Remote(RemoteSurrealConfig {
                url: "wss://surreal.example.com".into(),
                namespace: "tenant_a".into(),
                database: "prod".into(),
                username: "lf_app".into(),
                password_env: "LF_DB_PASS".into(),
                tls_skip_verify: false,
            }),
            namespace: "ignored".into(),
            database: "ignored".into(),
            legacy_sqlite_path: None,
        };
        assert_eq!(cfg.effective_namespace(), "tenant_a");
        assert_eq!(cfg.effective_database(), "prod");
    }

    #[test]
    fn round_trip_through_json() {
        let cfg = StorageConfig::embedded_default("/tmp/librefang");
        let s = serde_json::to_string(&cfg).unwrap();
        let back: StorageConfig = serde_json::from_str(&s).unwrap();
        assert_eq!(cfg, back);
    }

    #[test]
    fn round_trip_embedded_through_toml() {
        // Phase 4b exit criterion: `StorageConfig` round-trips through TOML so
        // it can live verbatim inside `~/.librefang/config.toml`.
        let cfg = StorageConfig::embedded_default("/var/lib/librefang");
        let s = toml::to_string(&cfg).unwrap();
        let back: StorageConfig = toml::from_str(&s).unwrap();
        assert_eq!(cfg, back);
    }

    #[test]
    fn round_trip_remote_through_toml() {
        let cfg = StorageConfig {
            backend: StorageBackendKind::Remote(RemoteSurrealConfig::librefang(
                "wss://surreal.example.com",
                "lf_app",
                "LF_DB_PASS",
            )),
            namespace: DEFAULT_NAMESPACE_NAME.into(),
            database: DEFAULT_DATABASE_NAME.into(),
            legacy_sqlite_path: None,
        };
        let s = toml::to_string(&cfg).unwrap();
        let back: StorageConfig = toml::from_str(&s).unwrap();
        assert_eq!(cfg, back);
    }

    #[test]
    fn round_trip_remote_through_json() {
        let cfg = StorageConfig {
            backend: StorageBackendKind::Remote(RemoteSurrealConfig::librefang(
                "wss://surreal.example.com",
                "lf_app",
                "LF_DB_PASS",
            )),
            namespace: DEFAULT_NAMESPACE_NAME.into(),
            database: DEFAULT_DATABASE_NAME.into(),
            legacy_sqlite_path: None,
        };
        let s = serde_json::to_string(&cfg).unwrap();
        let back: StorageConfig = serde_json::from_str(&s).unwrap();
        assert_eq!(cfg, back);
    }
}
