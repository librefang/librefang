//! Concrete storage backends for the kernel crate.
//!
//! Phase 6 of the `surrealdb-storage-swap` plan introduces the
//! SurrealDB-backed [`crate::storage_backends::TotpLockoutBackend`]; the
//! rusqlite path continues to live inline inside
//! [`crate::approval::ApprovalManager`] until the manager is refactored
//! to depend on the trait in Phase 7.

#[cfg(feature = "surreal-backend")]
pub mod surreal_approval;

#[cfg(feature = "surreal-backend")]
pub use surreal_approval::SurrealTotpLockoutBackend;
