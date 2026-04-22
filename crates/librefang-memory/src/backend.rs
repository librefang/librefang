//! `MemoryBackend` trait — the seam Phase 5 of the `surrealdb-storage-swap`
//! plan uses to swap between the legacy SQLite [`crate::MemorySubstrate`] and
//! a SurrealDB-backed implementation built on top of `surreal-memory`.
//!
//! The trait surface is intentionally narrow at this phase — it covers the
//! handful of operations the kernel reaches for through its
//! `KernelHandle::memory()` accessor. Phase 5 will widen the surface as the
//! Surreal implementation reaches feature parity with [`MemorySubstrate`].
//!
//! Today the only implementor is [`crate::MemorySubstrate`] (compiled from
//! the rusqlite stack); Phase 5 will add `SurrealMemoryBackend` behind the
//! `surreal-backend` feature.

use crate::MemorySubstrate;
use async_trait::async_trait;
use librefang_types::agent::{AgentEntry, AgentId};
use librefang_types::error::LibreFangResult;

/// Backend-agnostic interface for the memory subsystem.
///
/// Phase 4 deliberately exposes only the always-needed agent persistence
/// methods so the kernel can compile against the trait without owning the
/// concrete substrate. The full memory surface (knowledge graph, proactive
/// memory, vector search, sessions, usage telemetry) will follow in
/// Phase 5 once the Surreal implementation lands.
#[async_trait]
pub trait MemoryBackend: Send + Sync {
    /// Persist an agent definition.
    fn save_agent(&self, entry: &AgentEntry) -> LibreFangResult<()>;

    /// Look up a single agent by id.
    fn load_agent(&self, agent_id: AgentId) -> LibreFangResult<Option<AgentEntry>>;

    /// Remove an agent and cascade-delete its sessions.
    fn remove_agent(&self, agent_id: AgentId) -> LibreFangResult<()>;

    /// Load every persisted agent definition.
    fn load_all_agents(&self) -> LibreFangResult<Vec<AgentEntry>>;
}

#[async_trait]
impl MemoryBackend for MemorySubstrate {
    fn save_agent(&self, entry: &AgentEntry) -> LibreFangResult<()> {
        MemorySubstrate::save_agent(self, entry)
    }

    fn load_agent(&self, agent_id: AgentId) -> LibreFangResult<Option<AgentEntry>> {
        MemorySubstrate::load_agent(self, agent_id)
    }

    fn remove_agent(&self, agent_id: AgentId) -> LibreFangResult<()> {
        MemorySubstrate::remove_agent(self, agent_id)
    }

    fn load_all_agents(&self) -> LibreFangResult<Vec<AgentEntry>> {
        MemorySubstrate::load_all_agents(self)
    }
}
