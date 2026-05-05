//! Memory substrate for the LibreFang Agent Operating System.
//!
//! Provides a unified memory API over three storage backends:
//! - **Structured store** (SQLite): Key-value pairs, sessions, agent state
//! - **Semantic store**: Text-based search (Phase 1: LIKE matching, Phase 2: Qdrant vectors)
//! - **Knowledge graph** (SQLite): Entities and relations
//!
//! Agents interact with a single `Memory` trait that abstracts over all three stores.
//!
//! ## Proactive Memory (mem0-style API)
//!
//! This module also provides proactive memory capabilities:
//! - `ProactiveMemory`: Unified API (search, add, get, list)
//! - `ProactiveMemoryHooks`: Auto-memorize and auto-retrieve hooks
//! - `ProactiveMemoryStore`: Implementation on top of MemorySubstrate

pub mod chunker;
pub mod consolidation;
pub mod decay;
pub mod http_vector_store;
pub mod idempotency;
pub mod knowledge;
pub mod migration;
pub mod namespace_acl;
pub mod proactive;
pub mod prompt;
pub mod provider;
pub mod roster_store;
pub mod semantic;
pub mod session;
pub mod structured;
pub mod usage;

mod session_store;
mod substrate;
pub use session_store::SessionStore;
pub use substrate::MemorySubstrate;

// Re-export types for convenience
pub use librefang_types::memory::{
    ExtractionResult, MemoryAction, MemoryAddResult, MemoryFilter, MemoryFragment, MemoryId,
    MemoryItem, MemoryLevel, MemorySource, ProactiveMemory, ProactiveMemoryConfig,
    ProactiveMemoryHooks, RelationTriple, VectorSearchResult, VectorStore,
};

// Re-export proactive memory store
pub use namespace_acl::{MemoryNamespaceGuard, NamespaceGate};
pub use proactive::{MemoryExportItem, MemoryStats, ProactiveMemoryStore};
pub use prompt::PromptStore;

// Re-export vector store implementations
pub use http_vector_store::HttpVectorStore;
pub use semantic::SqliteVectorStore;

// Re-export memory provider plugin system
pub use provider::{MemoryError, MemoryManager, MemoryProvider, NullMemoryProvider};
