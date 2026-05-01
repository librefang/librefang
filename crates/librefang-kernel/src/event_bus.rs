//! Event bus — pub/sub with pattern matching and history ring buffer.

use dashmap::DashMap;
use librefang_types::agent::AgentId;
use librefang_types::event::{Event, EventPayload, EventTarget};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::sync::{broadcast, RwLock};
use tracing::{debug, error, warn};

/// Maximum events retained in the history ring buffer.
const HISTORY_SIZE: usize = 1000;

/// The central event bus for inter-agent and system communication.
///
/// Events are wrapped in `Arc<Event>` before broadcast so each subscriber
/// receives a cheap reference-counted handle instead of a deep clone of
/// the (potentially large) payload. See #3380.
pub struct EventBus {
    /// Broadcast channel for all events.
    sender: broadcast::Sender<Arc<Event>>,
    /// Per-agent event channels.
    agent_channels: DashMap<AgentId, broadcast::Sender<Arc<Event>>>,
    /// Event history ring buffer.
    history: Arc<RwLock<VecDeque<Event>>>,
    /// Count of events dropped because the per-agent channel had no active receiver.
    dropped_count: AtomicU64,
    /// Timestamp of the last drop warning log (for rate-limiting).
    last_drop_warn: std::sync::Mutex<std::time::Instant>,
}

/// Maps an `EventPayload` variant to a short label for drop-warning log fields.
fn payload_kind(payload: &EventPayload) -> &'static str {
    match payload {
        EventPayload::Message(_) => "Message",
        EventPayload::ToolResult(_) => "ToolResult",
        EventPayload::MemoryUpdate(_) => "MemoryUpdate",
        EventPayload::Lifecycle(_) => "Lifecycle",
        EventPayload::Network(_) => "Network",
        EventPayload::System(_) => "System",
        EventPayload::ApprovalRequested(_) => "ApprovalRequested",
        EventPayload::ApprovalResolved(_) => "ApprovalResolved",
        EventPayload::Custom(_) => "Custom",
    }
}

impl EventBus {
    /// Create a new event bus.
    pub fn new() -> Self {
        // 4 096-event capacity for the global broadcast channel (up from 1 024).
        // Burst-publishing scenarios (e.g. mass trigger evaluation) can spike far
        // above 1 024 events between scheduler ticks, causing RecvError::Lagged
        // and silently dropping trigger-driving events (issue #3630).
        let (sender, _) = broadcast::channel(4096);
        // Backdate the rate-limit timestamp so the FIRST lag burst after
        // process start is always logged. Initialising to `Instant::now()`
        // would silence the first 10 s of lag — a fresh process that
        // immediately sees backlog would only bump dropped_count and stay
        // quiet, defeating the "make lag visible" goal of #3630.
        // checked_sub: CLOCK_MONOTONIC can be <11 s on boot; fallback forfeits warmup, not correctness.
        let warmup = std::time::Instant::now()
            .checked_sub(std::time::Duration::from_secs(11))
            .unwrap_or_else(std::time::Instant::now);
        Self {
            sender,
            agent_channels: DashMap::new(),
            history: Arc::new(RwLock::new(VecDeque::with_capacity(HISTORY_SIZE))),
            dropped_count: AtomicU64::new(0),
            last_drop_warn: std::sync::Mutex::new(warmup),
        }
    }

    /// Publish an event to the bus.
    ///
    /// The event is wrapped in `Arc<Event>` so dispatching to N subscribers
    /// performs N cheap atomic ref-count bumps instead of N deep clones of
    /// the payload (#3380).
    pub async fn publish(&self, event: Event) {
        debug!(
            event_id = %event.id,
            source = %event.source,
            kind = payload_kind(&event.payload),
            "Publishing event"
        );

        // Store in history (history needs an owned copy — we keep this clone
        // only at the boundary; the broadcast hot path uses Arc throughout).
        {
            let mut history = self.history.write().await;
            if history.len() >= HISTORY_SIZE {
                history.pop_front();
            }
            history.push_back(event.clone());
        }

        // Wrap once, share via Arc clones — no payload deep-clone per subscriber.
        let event = Arc::new(event);

        // Route to target
        match &event.target {
            EventTarget::Agent(agent_id) => {
                if let Some(sender) = self.agent_channels.get(agent_id) {
                    if sender.send(Arc::clone(&event)).is_err() {
                        let total = self.dropped_count.fetch_add(1, Ordering::Relaxed) + 1;
                        if let Ok(mut last) = self.last_drop_warn.lock() {
                            if last.elapsed() >= std::time::Duration::from_secs(10) {
                                warn!(
                                    agent_id = %agent_id,
                                    event_id = %event.id,
                                    event_kind = payload_kind(&event.payload),
                                    total_dropped = total,
                                    "Event bus: agent has no active receiver, event dropped — check agent health",
                                );
                                *last = std::time::Instant::now();
                            }
                        }
                    }
                }
            }
            EventTarget::Broadcast => {
                if self.sender.send(Arc::clone(&event)).is_err() {
                    debug!(
                        event_id = %event.id,
                        event_kind = payload_kind(&event.payload),
                        "Broadcast event: no global subscribers"
                    );
                }
                let mut agent_drops: u64 = 0;
                for entry in self.agent_channels.iter() {
                    if entry.value().send(Arc::clone(&event)).is_err() {
                        agent_drops += 1;
                    }
                }
                if agent_drops > 0 {
                    let total =
                        self.dropped_count.fetch_add(agent_drops, Ordering::Relaxed) + agent_drops;
                    if let Ok(mut last) = self.last_drop_warn.lock() {
                        if last.elapsed() >= std::time::Duration::from_secs(10) {
                            warn!(
                                dropped = agent_drops,
                                total_dropped = total,
                                event_kind = payload_kind(&event.payload),
                                "Event bus: broadcast reached agents with no active receivers, events dropped",
                            );
                            *last = std::time::Instant::now();
                        }
                    }
                }
            }
            EventTarget::Pattern(_pattern) => {
                if self.sender.send(Arc::clone(&event)).is_err() {
                    debug!(
                        event_id = %event.id,
                        event_kind = payload_kind(&event.payload),
                        "Pattern event: no global subscribers"
                    );
                }
            }
            EventTarget::System => {
                if self.sender.send(Arc::clone(&event)).is_err() {
                    debug!(
                        event_id = %event.id,
                        event_kind = payload_kind(&event.payload),
                        "System event: no global subscribers"
                    );
                }
            }
        }
    }

    /// Subscribe to events for a specific agent.
    ///
    /// **Lagged handling**: callers must match `RecvError::Lagged(n)` and log a
    /// warning, then `continue` — the skipped events are already lost but future
    /// events can still be received.  Exiting on `Lagged` turns a transient
    /// slow-consumer condition into a permanent trigger miss (issue #3630).
    pub fn subscribe_agent(&self, agent_id: AgentId) -> broadcast::Receiver<Arc<Event>> {
        let entry = self.agent_channels.entry(agent_id).or_insert_with(|| {
            // 2 048-event buffer per agent (up from 256).  Trigger-driving events
            // are published in bursts; a deeper queue keeps slow consumers from
            // lagging and silently missing events (issue #3630).
            let (tx, _) = broadcast::channel(2048);
            tx
        });
        entry.subscribe()
    }

    /// Subscribe to all broadcast/system events.
    pub fn subscribe_all(&self) -> broadcast::Receiver<Arc<Event>> {
        self.sender.subscribe()
    }

    /// Get recent event history.
    pub async fn history(&self, limit: usize) -> Vec<Event> {
        let history = self.history.read().await;
        history.iter().rev().take(limit).cloned().collect()
    }

    /// Return the total number of events dropped due to no active receivers
    /// or consumer-side lag (see [`recv_event_skipping_lag`]).
    pub fn dropped_count(&self) -> u64 {
        self.dropped_count.load(Ordering::Relaxed)
    }

    /// Record that the consumer side dropped `n` events due to lag, and emit
    /// an error log if the rate-limit window has expired. Use this from any
    /// loop that receives from a [`broadcast::Receiver`] returned by
    /// [`Self::subscribe_agent`] or [`Self::subscribe_all`] when it sees
    /// [`broadcast::error::RecvError::Lagged`] — silent lag drops would
    /// otherwise hide missed triggers (issue #3630).
    pub fn record_consumer_lag(&self, n: u64, context: &'static str) {
        let total = self.dropped_count.fetch_add(n, Ordering::Relaxed) + n;
        if let Ok(mut last) = self.last_drop_warn.lock() {
            if last.elapsed() >= std::time::Duration::from_secs(10) {
                error!(
                    lagged = n,
                    total_dropped = total,
                    context = context,
                    "Event bus: consumer lagged behind broadcast queue, events dropped — \
                     receiver should be drained faster or buffer increased",
                );
                *last = std::time::Instant::now();
            }
        }
    }

    /// Remove an agent's channel when it's terminated.
    pub fn unsubscribe_agent(&self, agent_id: AgentId) {
        self.agent_channels.remove(&agent_id);
    }

    /// Remove channels for agents that no longer exist in the registry.
    pub fn gc_stale_channels(&self, live_agents: &std::collections::HashSet<AgentId>) -> usize {
        let stale: Vec<AgentId> = self
            .agent_channels
            .iter()
            .filter(|entry| !live_agents.contains(entry.key()))
            .map(|entry| *entry.key())
            .collect();
        let count = stale.len();
        for id in stale {
            self.agent_channels.remove(&id);
        }
        count
    }
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new()
    }
}

/// Receive the next event from a broadcast receiver, treating
/// [`broadcast::error::RecvError::Lagged`] as a "skip and report" condition
/// rather than terminating the consumer. Lagged drops are routed through
/// [`EventBus::record_consumer_lag`] so they show up as `error!` logs and
/// in `dropped_count()`. Returns `None` if the channel is closed.
///
/// Without this helper, callers that exit on `Lagged` turn a transient
/// burst into a permanent miss; callers that ignore `Lagged` lose triggers
/// silently (issue #3630).
pub async fn recv_event_skipping_lag(
    rx: &mut broadcast::Receiver<Arc<Event>>,
    bus: &EventBus,
    context: &'static str,
) -> Option<Arc<Event>> {
    loop {
        match rx.recv().await {
            Ok(event) => return Some(event),
            Err(broadcast::error::RecvError::Lagged(n)) => {
                bus.record_consumer_lag(n, context);
                continue;
            }
            Err(broadcast::error::RecvError::Closed) => return None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use librefang_types::event::{EventPayload, SystemEvent};

    #[tokio::test]
    async fn test_publish_and_history() {
        let bus = EventBus::new();
        let agent_id = AgentId::new();
        let event = Event::new(
            agent_id,
            EventTarget::System,
            EventPayload::System(SystemEvent::KernelStarted),
        );
        bus.publish(event).await;
        let history = bus.history(10).await;
        assert_eq!(history.len(), 1);
    }

    #[tokio::test]
    async fn record_consumer_lag_increments_dropped_count() {
        let bus = EventBus::new();
        assert_eq!(bus.dropped_count(), 0);
        bus.record_consumer_lag(7, "test");
        assert_eq!(bus.dropped_count(), 7);
        bus.record_consumer_lag(3, "test");
        assert_eq!(bus.dropped_count(), 10);
    }

    /// Warmup regression guard: first lag burst must advance `last_drop_warn` (#3630).
    #[tokio::test]
    async fn first_lag_burst_after_construction_advances_warn_timestamp() {
        let bus = EventBus::new();
        let before = *bus.last_drop_warn.lock().unwrap();
        bus.record_consumer_lag(1, "test_first_burst");
        let after = *bus.last_drop_warn.lock().unwrap();
        assert!(
            after > before,
            "first lag burst must advance last_drop_warn — warmup regression"
        );
    }

    #[tokio::test]
    async fn test_agent_subscribe() {
        let bus = EventBus::new();
        let agent_id = AgentId::new();
        let mut rx = bus.subscribe_agent(agent_id);

        let event = Event::new(
            AgentId::new(),
            EventTarget::Agent(agent_id),
            EventPayload::System(SystemEvent::HealthCheck {
                status: "ok".to_string(),
            }),
        );
        bus.publish(event).await;

        let received = rx.recv().await.unwrap();
        match &received.payload {
            EventPayload::System(SystemEvent::HealthCheck { status }) => {
                assert_eq!(status, "ok");
            }
            other => panic!("Expected HealthCheck payload, got {:?}", other),
        }
    }
}
