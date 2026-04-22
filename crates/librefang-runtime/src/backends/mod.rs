//! Concrete backend implementations of the storage traits in
//! [`crate::storage_backends`].
//!
//! Phase 6 of the `surrealdb-storage-swap` plan adds the SurrealDB-backed
//! [`AuditStore`](crate::storage_backends::AuditStore) and
//! [`TraceBackend`](crate::storage_backends::TraceBackend) implementations
//! here. The legacy rusqlite implementations continue to live as inherent
//! impls on [`crate::audit::AuditLog`] and [`crate::trace_store::TraceStore`]
//! and pick up the trait blanket impls in `storage_backends`.

#[cfg(feature = "surreal-backend")]
pub mod surreal_audit;

#[cfg(feature = "surreal-backend")]
pub mod surreal_trace;

#[cfg(feature = "surreal-backend")]
pub use surreal_audit::SurrealAuditStore;

#[cfg(feature = "surreal-backend")]
pub use surreal_trace::SurrealTraceBackend;
