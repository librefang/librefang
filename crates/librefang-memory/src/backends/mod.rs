//! Concrete storage backend implementations.
//!
//! Phase 5–9 of the `surrealdb-storage-swap` plan introduce SurrealDB-backed
//! implementations behind the `surreal-backend` Cargo feature. The legacy
//! [`crate::MemorySubstrate`] (rusqlite) keeps its trait impls in
//! [`crate::backend`] and remains available when callers opt into
//! `sqlite-backend`.

#[cfg(feature = "surreal-backend")]
pub mod shared;
#[cfg(feature = "surreal-backend")]
pub mod surreal;
#[cfg(feature = "surreal-backend")]
pub mod surreal_device;
#[cfg(feature = "surreal-backend")]
pub mod surreal_knowledge;
#[cfg(feature = "surreal-backend")]
pub mod surreal_kv;
#[cfg(feature = "surreal-backend")]
pub mod surreal_proactive;
#[cfg(feature = "surreal-backend")]
pub mod surreal_prompt;
#[cfg(feature = "surreal-backend")]
pub mod surreal_semantic;
#[cfg(feature = "surreal-backend")]
pub mod surreal_session;
#[cfg(feature = "surreal-backend")]
pub mod surreal_task;
#[cfg(feature = "surreal-backend")]
pub mod surreal_usage;

#[cfg(feature = "surreal-backend")]
pub use shared::open_shared_memory_storage;
#[cfg(feature = "surreal-backend")]
pub use surreal::SurrealMemoryBackend;
#[cfg(feature = "surreal-backend")]
pub use surreal_device::SurrealDeviceStore;
#[cfg(feature = "surreal-backend")]
pub use surreal_knowledge::SurrealKnowledgeBackend;
#[cfg(feature = "surreal-backend")]
pub use surreal_kv::SurrealKvBackend;
#[cfg(feature = "surreal-backend")]
pub use surreal_proactive::SurrealProactiveMemoryBackend;
#[cfg(feature = "surreal-backend")]
pub use surreal_prompt::SurrealPromptStore;
#[cfg(feature = "surreal-backend")]
pub use surreal_semantic::SurrealSemanticBackend;
#[cfg(feature = "surreal-backend")]
pub use surreal_session::SurrealSessionBackend;
#[cfg(feature = "surreal-backend")]
pub use surreal_task::SurrealTaskBackend;
#[cfg(feature = "surreal-backend")]
pub use surreal_usage::SurrealUsageStore;
