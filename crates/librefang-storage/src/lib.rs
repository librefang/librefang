//! # `librefang-storage`
//!
//! Backend-agnostic storage abstractions for librefang.
//!
//! This crate defines the seams that let librefang swap between an embedded
//! SurrealDB 3.0 store (default) and the legacy SQLite store without rippling
//! changes through every call site. Higher-level domain crates
//! (`librefang-memory`, `librefang-runtime`, `librefang-kernel`) consume the
//! traits here and provide their own backend implementations behind the
//! [`surreal-backend`] / [`sqlite-backend`] feature flags.
//!
//! See `Phase 4` of the `surrealdb-storage-swap` plan for the full design.
//!
//! ## Feature flags
//!
//! - `surreal-backend` (default) — pulls in the `surrealdb` crate at version
//!   `=3.0.5` so librefang, `surreal-memory`, and the Universal Agent Runtime
//!   all link the same client.
//! - `sqlite-backend` — opt-in; pulls in `rusqlite` for the legacy backend.
//!
//! Both can be enabled simultaneously; the live backend is selected at startup
//! from [`StorageConfig::backend`].
//!
//! ## Layout
//!
//! - [`config`]: serialisable [`StorageConfig`] / [`StorageBackendKind`] /
//!   [`RemoteSurrealConfig`] used by `librefang-types::KernelConfig`.
//! - [`error`]: canonical [`StorageError`] consumed by every backend trait.
//! - [`pool`]: the [`SurrealConnectionPool`] that hands out per-app sessions
//!   sharing a single transport (per the SurrealDB 3.0 multi-tenancy guide).

#![warn(missing_docs)]
#![warn(rust_2018_idioms)]

pub mod config;
pub mod error;
pub mod migrate;
pub mod migrations;
pub mod pool;
pub mod provision;

pub use config::{
    RemoteSurrealConfig, StorageBackendKind, StorageConfig, DEFAULT_DATABASE_NAME,
    DEFAULT_NAMESPACE_NAME,
};
pub use error::StorageError;
pub use pool::{SurrealConnectionPool, SurrealSession};
pub use provision::{provision_uar_namespace, ProvisionReceipt};
