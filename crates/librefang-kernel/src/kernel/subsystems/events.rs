//! Event subsystem — buses, mid-turn injection channels, sticky
//! routing state, and the GC-task idempotency guard for the session
//! stream hub.
//!
//! Bundles eight event/routing handles that previously sat as a flat
//! cluster on `LibreFangKernel`. Inner names are kept verbatim so the
//! migration is purely mechanical.

use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use dashmap::DashMap;
use librefang_types::agent::{AgentId, SessionId};
use librefang_types::tool::AgentLoopSignal;

use crate::event_bus::EventBus;
use crate::session_lifecycle::SessionLifecycleBus;
use crate::session_stream_hub::SessionStreamHub;

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
        }
    }
}
