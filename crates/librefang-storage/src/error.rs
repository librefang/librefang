//! Canonical error type for storage backends.

use std::path::PathBuf;
use thiserror::Error;

/// Errors returned by storage backends and the connection pool.
///
/// All backend-specific errors must be mapped into one of these variants so
/// call sites can pattern-match on the error category without depending on
/// the underlying driver crate.
#[derive(Debug, Error)]
pub enum StorageError {
    /// The configured backend is not compiled into this build.
    #[error("storage backend '{backend}' is not enabled in this build")]
    BackendDisabled {
        /// Human-readable backend name (`"surreal"` or `"sqlite"`).
        backend: &'static str,
    },

    /// Failed to open or initialise an embedded data directory.
    #[error("failed to open embedded storage at {path}: {source}")]
    EmbeddedOpen {
        /// Embedded data directory.
        path: PathBuf,
        /// Underlying cause.
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },

    /// Failed to connect to a remote SurrealDB instance.
    #[error("failed to connect to remote storage at {url}: {source}")]
    RemoteConnect {
        /// Remote endpoint URL (without credentials).
        url: String,
        /// Underlying cause.
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },

    /// Authentication against the storage backend failed.
    #[error("authentication failed for user '{username}': {message}")]
    AuthFailed {
        /// User attempting to sign in.
        username: String,
        /// Backend-supplied detail (already redacted of password material).
        message: String,
    },

    /// Required environment variable for credentials was unset or empty.
    #[error("credential env var '{name}' is unset or empty")]
    MissingCredential {
        /// Name of the env var that should hold the secret.
        name: String,
    },

    /// Generic backend failure (query, decode, transport, etc.).
    #[error("storage backend error: {0}")]
    Backend(String),

    /// Configuration is internally inconsistent.
    #[error("invalid storage configuration: {0}")]
    InvalidConfig(String),
}

impl StorageError {
    /// `true` when the error indicates a transient / retryable failure (network,
    /// transient lock contention, etc.). Used by callers that wrap idempotent
    /// operations in retry loops.
    #[must_use]
    pub fn is_transient(&self) -> bool {
        matches!(self, Self::RemoteConnect { .. })
    }

    /// `true` when the error indicates a configuration problem the user must
    /// fix (missing credential, unknown backend, malformed URL, etc.).
    #[must_use]
    pub fn is_config(&self) -> bool {
        matches!(
            self,
            Self::BackendDisabled { .. } | Self::MissingCredential { .. } | Self::InvalidConfig(_)
        )
    }
}

/// Convenience alias for `Result<T, StorageError>`.
pub type StorageResult<T> = Result<T, StorageError>;
