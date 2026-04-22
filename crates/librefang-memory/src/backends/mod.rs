//! Concrete [`crate::MemoryBackend`] implementations.
//!
//! Phase 5 of the `surrealdb-storage-swap` plan introduces the SurrealDB
//! backend behind the `surreal-backend` Cargo feature. The legacy
//! [`crate::MemorySubstrate`] (rusqlite) keeps its inherent
//! [`crate::MemoryBackend`] impl in [`crate::backend`] and remains available
//! when callers opt into `sqlite-backend`.

#[cfg(feature = "surreal-backend")]
pub mod surreal;

#[cfg(feature = "surreal-backend")]
pub use surreal::SurrealMemoryBackend;
