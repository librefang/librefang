//! Agent subsystem — registries, lifecycle, lock maps, and per-agent
//! decision traces.
//!
//! Bundles the thirteen agent-side fields that previously sat as a
//! flat cluster on `LibreFangKernel`. Inner names are kept verbatim so
//! the migration is purely mechanical (`self.registry` →
//! `self.agents.registry`, `self.scheduler` → `self.agents.scheduler`,
//! …).

use std::sync::Arc;

use dashmap::DashMap;
use librefang_runtime::interrupt::SessionInterrupt;
use librefang_types::agent::{AgentId, SessionId};
use librefang_types::tool::DecisionTrace;
use uuid::Uuid;

use super::super::RunningTask;
use crate::agent_identity_registry::AgentIdentityRegistry;
use crate::capabilities::CapabilityManager;
use crate::registry::AgentRegistry;
use crate::scheduler::AgentScheduler;
use crate::supervisor::Supervisor;

/// Agent registries + scheduler + supervisor + lock maps + traces —
/// see module docs.
pub struct AgentSubsystem {
    /// Agent registry.
    pub(crate) registry: AgentRegistry,
    /// Canonical agent UUID registry (refs #4614).
    pub(crate) agent_identities: Arc<AgentIdentityRegistry>,
    /// Capability manager.
    pub(crate) capabilities: CapabilityManager,
    /// Agent scheduler.
    pub(crate) scheduler: AgentScheduler,
    /// Process supervisor.
    pub(crate) supervisor: Supervisor,
    /// Tracks running agent loops for cancellation + observability.
    /// Keyed by `(agent, session)` so concurrent loops on the same
    /// agent each retain their own abort handle.
    pub(crate) running_tasks: DashMap<(AgentId, SessionId), RunningTask>,
    /// Tracks per-(agent, session) interrupts so `stop_*_run` can
    /// signal `cancel()` in addition to aborting the tokio task.
    pub(crate) session_interrupts: DashMap<(AgentId, SessionId), SessionInterrupt>,
    /// Per-agent message locks — serializes LLM calls for the same
    /// agent to prevent session corruption when multiple messages
    /// arrive concurrently.
    pub(crate) agent_msg_locks: DashMap<AgentId, Arc<tokio::sync::Mutex<()>>>,
    /// Per-session message locks — used instead of `agent_msg_locks`
    /// when a caller supplies an explicit `session_id_override`.
    pub(crate) session_msg_locks: DashMap<SessionId, Arc<tokio::sync::Mutex<()>>>,
    /// Per-agent invocation semaphore — caps concurrent **trigger
    /// dispatch** fires to a single agent.
    pub(crate) agent_concurrency: DashMap<AgentId, Arc<tokio::sync::Semaphore>>,
    /// Per-hand-instance lock serializing runtime-override mutations.
    pub(crate) hand_runtime_override_locks: DashMap<Uuid, Arc<std::sync::Mutex<()>>>,
    /// Per-agent decision traces from the most recent message exchange.
    pub(crate) decision_traces: DashMap<AgentId, Vec<DecisionTrace>>,
    /// Per-agent fire-and-forget background tasks (skill reviews, owner
    /// notifications, …) that hold semaphore permits or spend tokens
    /// on behalf of a specific agent.
    pub(crate) agent_watchers:
        DashMap<AgentId, Arc<std::sync::Mutex<Vec<tokio::task::JoinHandle<()>>>>>,
}

impl AgentSubsystem {
    pub(crate) fn new(
        agent_identities: Arc<AgentIdentityRegistry>,
        supervisor: Supervisor,
    ) -> Self {
        Self {
            registry: AgentRegistry::new(),
            agent_identities,
            capabilities: CapabilityManager::new(),
            scheduler: AgentScheduler::new(),
            supervisor,
            running_tasks: DashMap::new(),
            session_interrupts: DashMap::new(),
            agent_msg_locks: DashMap::new(),
            session_msg_locks: DashMap::new(),
            agent_concurrency: DashMap::new(),
            hand_runtime_override_locks: DashMap::new(),
            decision_traces: DashMap::new(),
            agent_watchers: DashMap::new(),
        }
    }

    /// Agent registry handle.
    #[inline]
    pub fn registry_ref(&self) -> &AgentRegistry {
        &self.registry
    }

    /// Canonical agent UUID registry (refs #4614).
    #[inline]
    pub fn identities_ref(&self) -> &Arc<AgentIdentityRegistry> {
        &self.agent_identities
    }

    /// Agent scheduler handle.
    #[inline]
    pub fn scheduler_ref(&self) -> &AgentScheduler {
        &self.scheduler
    }

    /// Process supervisor handle.
    #[inline]
    pub fn supervisor_ref(&self) -> &Supervisor {
        &self.supervisor
    }

    /// Per-agent decision-trace storage.
    #[inline]
    pub fn traces(&self) -> &DashMap<AgentId, Vec<DecisionTrace>> {
        &self.decision_traces
    }
}
