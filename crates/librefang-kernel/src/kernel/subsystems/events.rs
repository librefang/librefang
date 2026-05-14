//! Event subsystem — buses, mid-turn injection channels, sticky
//! routing state, and the GC-task idempotency guard for the session
//! stream hub.
//!
//! Bundles eight event/routing handles that previously sat as a flat
//! cluster on `LibreFangKernel`. Inner names are kept verbatim so the
//! migration is purely mechanical.

use std::collections::HashMap;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use dashmap::DashMap;
use librefang_types::agent::{AgentId, SessionId};
use librefang_types::task::{TaskHandle, TaskId};
use librefang_types::tool::AgentLoopSignal;
use parking_lot::Mutex;

use crate::event_bus::EventBus;
use crate::session_lifecycle::SessionLifecycleBus;
use crate::session_stream_hub::SessionStreamHub;

/// Registry entry for an in-flight async task (#4983).
///
/// The kernel keeps one of these per registered `TaskId` for as long as
/// the underlying operation is running. On terminal completion the entry
/// is removed and a `TaskCompletionEvent` is injected into the
/// `(agent_id, session_id)` injection channel via the existing #956
/// mid-turn path.
#[derive(Debug, Clone)]
pub(crate) struct PendingTask {
    /// The handle the kernel returned to the caller. Carries the
    /// `TaskKind` (workflow run id, delegation target, …) so the
    /// completion event can be built without re-deriving correlation
    /// state.
    pub handle: TaskHandle,
    /// Agent that registered the task. The completion event is
    /// addressed to this agent's session.
    pub agent_id: AgentId,
    /// Session that registered the task — the originating turn's
    /// session. Pairs with `agent_id` as the injection-channel key.
    pub session_id: SessionId,
}

/// Focused event-bus + injection-channel API.
pub trait EventSubsystemApi: Send + Sync {
    /// Top-level event bus handle.
    fn event_bus_ref(&self) -> &EventBus;
    /// Cloneable session-lifecycle bus.
    fn lifecycle_bus(&self) -> Arc<SessionLifecycleBus>;
    /// Per-(agent, session) injection senders map.
    fn injection_senders_ref(
        &self,
    ) -> &DashMap<(AgentId, SessionId), tokio::sync::mpsc::Sender<AgentLoopSignal>>;
}

/// Event buses + injection channels + routing cluster — see module docs.
pub struct EventSubsystem {
    /// Event bus.
    pub(crate) event_bus: EventBus,
    /// Session lifecycle event bus (push-based pub/sub for
    /// session-scoped events).
    pub(crate) session_lifecycle_bus: Arc<SessionLifecycleBus>,
    /// Per-session stream-event hub for multi-client SSE attach.
    pub(crate) session_stream_hub: Arc<SessionStreamHub>,
    /// Per-(agent, session) mid-turn injection senders.
    pub(crate) injection_senders:
        DashMap<(AgentId, SessionId), tokio::sync::mpsc::Sender<AgentLoopSignal>>,
    /// Per-(agent, session) injection receivers, created alongside
    /// senders and consumed by the agent loop.
    pub(crate) injection_receivers: DashMap<
        (AgentId, SessionId),
        Arc<tokio::sync::Mutex<tokio::sync::mpsc::Receiver<AgentLoopSignal>>>,
    >,
    /// Sticky assistant routing per conversation.
    pub(crate) assistant_routes:
        DashMap<String, (super::super::AssistantRouteTarget, std::time::Instant)>,
    /// Consecutive-mismatch counters for `StickyHeuristic` auto-routing.
    pub(crate) route_divergence: DashMap<String, u32>,
    /// Idempotency guard for the session-stream-hub idle GC task.
    pub(crate) session_stream_hub_gc_started: AtomicBool,
    /// Async task tracker (#4983). Stores pending tasks (workflow runs,
    /// agent delegations, …) so the kernel can inject a
    /// `TaskCompletionEvent` into the originating session when the
    /// underlying operation finishes. `HashMap` is intentional — the
    /// registry is keyed by `TaskId` and accessed via exact-key lookup
    /// only; it is **not** iterated to build any LLM-bound prompt, so
    /// the #3298 deterministic-ordering rule does not apply. Wrapped in
    /// `parking_lot::Mutex` so the "lookup, remove, then send" sequence
    /// in `complete_async_task` can be expressed atomically without
    /// holding a DashMap shard guard across the `try_send` boundary.
    pub(crate) async_tasks: Mutex<HashMap<TaskId, PendingTask>>,
}

impl EventSubsystem {
    pub(crate) fn new() -> Self {
        Self {
            event_bus: EventBus::new(),
            session_lifecycle_bus: Arc::new(SessionLifecycleBus::new(256)),
            session_stream_hub: Arc::new(SessionStreamHub::new()),
            injection_senders: DashMap::new(),
            injection_receivers: DashMap::new(),
            assistant_routes: DashMap::new(),
            route_divergence: DashMap::new(),
            session_stream_hub_gc_started: AtomicBool::new(false),
            async_tasks: Mutex::new(HashMap::new()),
        }
    }
}

impl EventSubsystemApi for EventSubsystem {
    #[inline]
    fn event_bus_ref(&self) -> &EventBus {
        &self.event_bus
    }

    #[inline]
    fn lifecycle_bus(&self) -> Arc<SessionLifecycleBus> {
        Arc::clone(&self.session_lifecycle_bus)
    }

    #[inline]
    fn injection_senders_ref(
        &self,
    ) -> &DashMap<(AgentId, SessionId), tokio::sync::mpsc::Sender<AgentLoopSignal>> {
        &self.injection_senders
    }
}
