//! # `librefang-storage`
//!
//! SurrealDB storage foundation for LibreFang.
//!
//! This crate provides the pieces an opt-in SurrealDB backend needs, without
//! changing the default SQLite storage: a serialisable storage configuration,
//! a connection pool that hands out per-tenant sessions over one shared
//! transport, and a versioned migration runner with the operational schema.
//!
//! ## Feature flags
//!
//! - `surreal-backend` (default) — pulls in the `surrealdb` crate pinned at
//!   `=4.0.0`. SurrealDB 4.0 removed RocksDB, so the embedded engine is
//!   SurrealKV (`surrealkv://`).
//!
//! ## Layout
//!
//! - [`config`]: serialisable [`StorageConfig`] / [`StorageBackendKind`] /
//!   [`RemoteSurrealConfig`].
//! - [`error`]: the canonical [`StorageError`].
//! - [`pool`]: the [`SurrealConnectionPool`] that hands out per-app sessions
//!   sharing a single transport.
//! - [`migrations`]: the versioned DDL migrations and their runner.

#![warn(missing_docs)]
#![warn(rust_2018_idioms)]

pub mod config;
pub mod error;
pub mod migrations;
pub mod pool;

pub use config::{
    RemoteSurrealConfig, StorageBackendKind, StorageConfig, DEFAULT_DATABASE_NAME,
    DEFAULT_NAMESPACE_NAME,
};
pub use error::StorageError;
pub use pool::{shared_pool, SurrealConnectionPool, SurrealSession};
