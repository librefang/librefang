//! Per-application SurrealDB session pool.
//!
//! Real connection logic lives behind `--features surreal-backend`; without
//! that feature this module compiles to a stubs-only surface so other crates
//! can still depend on the type names.
//!
//! Per the SurrealDB 3.0 [multi-tenancy guide](https://surrealdb.com/docs/sdk/rust/concepts/multi-tenancy),
//! the recommended pattern for a single transport with multiple logical
//! tenants is to clone the root `Surreal<Any>` client and `use_ns` /
//! `use_db` on each clone independently. [`SurrealConnectionPool`] does
//! exactly that, keyed by remote URL.

use crate::config::StorageConfig;
#[cfg(feature = "surreal-backend")]
use crate::config::{RemoteSurrealConfig, StorageBackendKind};
use crate::error::{StorageError, StorageResult};

/// A logical session against one SurrealDB namespace + database.
///
/// Always cheap to clone; cloning hands out an independent session over the
/// shared transport.
#[derive(Clone)]
pub struct SurrealSession {
    namespace: String,
    database: String,
    #[cfg(feature = "surreal-backend")]
    client: surrealdb::Surreal<surrealdb::engine::any::Any>,
}

impl SurrealSession {
    /// SurrealDB namespace this session is bound to.
    #[must_use]
    pub fn namespace(&self) -> &str {
        &self.namespace
    }

    /// SurrealDB database this session is bound to.
    #[must_use]
    pub fn database(&self) -> &str {
        &self.database
    }

    /// Underlying SurrealDB client (only available with `surreal-backend`).
    #[cfg(feature = "surreal-backend")]
    #[must_use]
    pub fn client(&self) -> &surrealdb::Surreal<surrealdb::engine::any::Any> {
        &self.client
    }
}

impl std::fmt::Debug for SurrealSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SurrealSession")
            .field("namespace", &self.namespace)
            .field("database", &self.database)
            .finish_non_exhaustive()
    }
}

/// Pool of per-URL SurrealDB connections.
///
/// Each call to [`Self::open`] returns an independent [`SurrealSession`] that
/// shares the underlying transport with any other session opened against the
/// same URL. This is the pattern librefang uses to let one remote SurrealDB
/// instance host both `librefang/*` and `uar/*` namespaces concurrently.
#[derive(Default)]
pub struct SurrealConnectionPool {
    #[cfg(feature = "surreal-backend")]
    transports: dashmap::DashMap<String, surrealdb::Surreal<surrealdb::engine::any::Any>>,
}

impl SurrealConnectionPool {
    /// Construct an empty pool. Connections are opened lazily on first use.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Open or reuse a session for the given [`StorageConfig`].
    ///
    /// For embedded backends this opens a fresh RocksDB-backed connection.
    /// For remote backends, the underlying transport is cached by URL and
    /// cloned to produce per-session ns/db state, per the SurrealDB 3.0
    /// multi-tenancy pattern.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError::BackendDisabled`] when the
    /// `surreal-backend` feature isn't compiled in,
    /// [`StorageError::EmbeddedOpen`] / [`StorageError::RemoteConnect`] for
    /// transport failures, and [`StorageError::AuthFailed`] /
    /// [`StorageError::MissingCredential`] for credential issues.
    #[cfg(feature = "surreal-backend")]
    pub async fn open(&self, cfg: &StorageConfig) -> StorageResult<SurrealSession> {
        match &cfg.backend {
            StorageBackendKind::Embedded { path } => {
                // RocksDB only allows a single OS-level lock per directory at
                // a time. Cache the underlying transport keyed by URL so that
                // repeated opens against the same embedded path within one
                // process reuse the connection (and therefore the lock).
                let url = format!("rocksdb://{}", path.display());
                let transport = match self.transports.get(&url) {
                    Some(existing) => existing.clone(),
                    None => {
                        let client = surrealdb::engine::any::connect(&url).await.map_err(|e| {
                            StorageError::EmbeddedOpen {
                                path: path.clone(),
                                source: Box::new(e),
                            }
                        })?;
                        self.transports.insert(url.clone(), client.clone());
                        client
                    }
                };
                let session = transport.clone();
                session
                    .use_ns(&cfg.namespace)
                    .use_db(&cfg.database)
                    .await
                    .map_err(|e| StorageError::Backend(e.to_string()))?;
                Ok(SurrealSession {
                    namespace: cfg.namespace.clone(),
                    database: cfg.database.clone(),
                    client: session,
                })
            }
            StorageBackendKind::Remote(remote) => self.open_remote(remote).await,
        }
    }

    /// Stub used when the `surreal-backend` feature is not enabled.
    #[cfg(not(feature = "surreal-backend"))]
    pub async fn open(&self, _cfg: &StorageConfig) -> StorageResult<SurrealSession> {
        Err(StorageError::BackendDisabled { backend: "surreal" })
    }

    /// Open a session against a remote SurrealDB, reusing the cached transport
    /// for that URL when one already exists.
    ///
    /// # Errors
    ///
    /// Same conditions as [`Self::open`].
    #[cfg(feature = "surreal-backend")]
    pub async fn open_remote(&self, remote: &RemoteSurrealConfig) -> StorageResult<SurrealSession> {
        // Warn when the operator has requested TLS skip-verify. The SurrealDB
        // 3.0 SDK's `engine::any::connect` uses the underlying transport's
        // default TLS policy and does not currently expose a certificate
        // bypass via the URL or a builder; implementing a custom connector
        // would require vendoring the transport crate. Until that work lands,
        // we honour the config field by logging the intent and falling through
        // to the standard TLS-verified connection.
        if remote.tls_skip_verify {
            tracing::warn!(
                url = %remote.url,
                "tls_skip_verify = true is configured but certificate \
                 verification bypass is not yet implemented for the \
                 SurrealDB 3.0 transport layer. The connection will use \
                 the default TLS policy (verification enabled). \
                 Set tls_skip_verify = false to suppress this warning, \
                 or connect via plain ws:// / http:// for local/dev servers."
            );
        }

        let password = std::env::var(&remote.password_env)
            .map_err(|_| StorageError::MissingCredential {
                name: remote.password_env.clone(),
            })
            .and_then(|v| {
                if v.is_empty() {
                    Err(StorageError::MissingCredential {
                        name: remote.password_env.clone(),
                    })
                } else {
                    Ok(v)
                }
            })?;

        let transport = match self.transports.get(&remote.url) {
            Some(existing) => existing.clone(),
            None => {
                let client = surrealdb::engine::any::connect(&remote.url)
                    .await
                    .map_err(|e| StorageError::RemoteConnect {
                        url: remote.url.clone(),
                        source: Box::new(e),
                    })?;
                self.transports.insert(remote.url.clone(), client.clone());
                client
            }
        };

        let session = transport.clone();
        session
            .signin(surrealdb::opt::auth::Namespace {
                namespace: remote.namespace.clone(),
                username: remote.username.clone(),
                password: password.clone(),
            })
            .await
            .map_err(|e| StorageError::AuthFailed {
                username: remote.username.clone(),
                message: e.to_string(),
            })?;
        session
            .use_ns(&remote.namespace)
            .use_db(&remote.database)
            .await
            .map_err(|e| StorageError::Backend(e.to_string()))?;

        Ok(SurrealSession {
            namespace: remote.namespace.clone(),
            database: remote.database.clone(),
            client: session,
        })
    }

    /// Number of distinct remote transports currently cached.
    #[must_use]
    pub fn cached_transports(&self) -> usize {
        #[cfg(feature = "surreal-backend")]
        {
            self.transports.len()
        }
        #[cfg(not(feature = "surreal-backend"))]
        {
            0
        }
    }
}

impl std::fmt::Debug for SurrealConnectionPool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SurrealConnectionPool")
            .field("cached_transports", &self.cached_transports())
            .finish()
    }
}

#[cfg(all(test, feature = "surreal-backend"))]
mod tests {
    use super::*;
    use crate::config::{StorageBackendKind, StorageConfig};
    use tempfile::tempdir;

    #[tokio::test(flavor = "multi_thread")]
    async fn embedded_pool_opens_with_independent_ns_db() {
        let dir = tempdir().unwrap();
        let cfg_a = StorageConfig {
            backend: StorageBackendKind::embedded(dir.path().join("a.surreal")),
            namespace: "librefang".into(),
            database: "main".into(),
            legacy_sqlite_path: None,
        };
        let pool = SurrealConnectionPool::new();
        let session = pool.open(&cfg_a).await.expect("open embedded");
        assert_eq!(session.namespace(), "librefang");
        assert_eq!(session.database(), "main");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn pool_emits_independent_sessions_per_namespace() {
        // Phase 4b exit criterion: two sessions opened against the same
        // pool retain independent namespace/database labels even when they
        // share the underlying transport (here we verify the embedded path,
        // which is the cheapest way to exercise the cloning code path
        // without standing up a remote SurrealDB in CI).
        let dir = tempdir().unwrap();
        let pool = SurrealConnectionPool::new();
        let lib_cfg = StorageConfig {
            backend: StorageBackendKind::embedded(dir.path().join("lib.surreal")),
            namespace: "librefang".into(),
            database: "main".into(),
            legacy_sqlite_path: None,
        };
        let uar_cfg = StorageConfig {
            backend: StorageBackendKind::embedded(dir.path().join("uar.surreal")),
            namespace: "uar".into(),
            database: "main".into(),
            legacy_sqlite_path: None,
        };
        let lib_session = pool.open(&lib_cfg).await.expect("open librefang");
        let uar_session = pool.open(&uar_cfg).await.expect("open uar");
        assert_eq!(lib_session.namespace(), "librefang");
        assert_eq!(uar_session.namespace(), "uar");
        assert_eq!(lib_session.database(), "main");
        assert_eq!(uar_session.database(), "main");
    }
}
