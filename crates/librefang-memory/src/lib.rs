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

pub mod consolidation;
pub mod knowledge;
pub mod migration;
pub mod proactive;
pub mod semantic;
pub mod session;
pub mod structured;
pub mod usage;

mod substrate;
pub use substrate::MemorySubstrate;

// Re-export types for convenience
pub use librefang_types::memory::{
    ExtractionResult, MemoryAction, MemoryAddResult, MemoryFilter, MemoryFragment, MemoryId,
    MemoryItem, MemoryLevel, MemorySource, ProactiveMemory, ProactiveMemoryConfig,
    ProactiveMemoryHooks,
};

// Re-export proactive memory store
pub use proactive::{MemoryStats, ProactiveMemoryStore};
