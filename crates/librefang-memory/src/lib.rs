//! Memory subsystem for the LibreFang Agent Operating System.
//!
//! ## Storage backends
//!
//! Two backend families are available, selected at compile time via Cargo features:
//!
//! ### SurrealDB (default — `surreal-backend` feature)
//!
//! All production-grade backends use SurrealDB 3.0 with HNSW vector indexes,
//! BM25 full-text search, and parameterised SurrealQL queries.  The concrete
//! types are:
//!
//! | Backend | Type |
//! |---|---|
//! | Semantic / vector search | [`SurrealSemanticBackend`] (HNSW v5 + BM25 hybrid) |
//! | Agent registry | `SurrealMemoryBackend` |
//! | Sessions | `SurrealSessionBackend` |
//! | Key-value store | `SurrealKvBackend` |
//! | Task queue | `SurrealTaskBackend` |
//! | Proactive memory | `SurrealProactiveMemoryBackend` |
//! | Prompt management | `SurrealPromptStore` |
//! | Device pairing | `SurrealDeviceStore` |
//! | Usage metering | `SurrealUsageStore` |
//! | Knowledge graph | `SurrealKnowledgeBackend` |
//!
//! ### SQLite (upstream compatibility — `sqlite-backend` feature)
//!
//! [`MemorySubstrate`] provides the legacy monolithic SQLite implementation.
//! All backend traits are implemented for it so existing code continues to
//! compile unchanged when only the `sqlite-backend` feature is active.
//!
//! ## Proactive Memory (mem0-style API)
//!
//! - `ProactiveMemory`: Unified API (search, add, get, list)
//! - `ProactiveMemoryHooks`: Auto-memorize and auto-retrieve hooks
//! - `ProactiveMemoryStore`: Implementation on top of MemorySubstrate

pub mod chunker;
pub mod consolidation;
pub mod decay;
pub mod http_vector_store;
pub mod knowledge;
pub mod migration;
pub mod proactive;
pub mod prompt;
pub mod provider;
pub mod semantic;
pub mod session;
pub mod structured;
pub mod usage;

mod backend;
pub mod backends;
mod substrate;
pub use backend::{
    DeviceBackend, KnowledgeBackend, KvBackend, MemoryBackend, ProactiveMemoryBackend,
    PromptBackend, SemanticBackend, SessionBackend, TaskBackend, UsageBackend,
};
#[cfg(feature = "surreal-backend")]
pub use backends::open_shared_memory_storage;
#[cfg(feature = "surreal-backend")]
pub use backends::SurrealDeviceStore;
#[cfg(feature = "surreal-backend")]
pub use backends::SurrealKnowledgeBackend;
#[cfg(feature = "surreal-backend")]
pub use backends::SurrealKvBackend;
#[cfg(feature = "surreal-backend")]
pub use backends::SurrealMemoryBackend;
#[cfg(feature = "surreal-backend")]
pub use backends::SurrealProactiveMemoryBackend;
#[cfg(feature = "surreal-backend")]
pub use backends::SurrealPromptStore;
#[cfg(feature = "surreal-backend")]
pub use backends::SurrealSemanticBackend;
#[cfg(feature = "surreal-backend")]
pub use backends::SurrealSessionBackend;
#[cfg(feature = "surreal-backend")]
pub use backends::SurrealTaskBackend;
#[cfg(feature = "surreal-backend")]
pub use backends::SurrealUsageStore;
pub use substrate::MemorySubstrate;

// Re-export types for convenience
pub use librefang_types::memory::{
    ExtractionResult, MemoryAction, MemoryAddResult, MemoryFilter, MemoryFragment, MemoryId,
    MemoryItem, MemoryLevel, MemorySource, ProactiveMemory, ProactiveMemoryConfig,
    ProactiveMemoryHooks, RelationTriple, VectorSearchResult, VectorStore,
};

// Re-export proactive memory store
pub use proactive::{MemoryExportItem, MemoryStats, ProactiveMemoryStore};
pub use prompt::PromptStore;

// Re-export vector store implementations
pub use http_vector_store::HttpVectorStore;
pub use semantic::SqliteVectorStore;

// Re-export memory provider plugin system
pub use provider::{MemoryError, MemoryManager, MemoryProvider, NullMemoryProvider};
