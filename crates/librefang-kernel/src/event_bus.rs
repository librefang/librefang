//! Event bus — pub/sub with pattern matching and history ring buffer.

use dashmap::DashMap;
use librefang_types::agent::AgentId;
use librefang_types::event::{Event, EventPayload, EventTarget};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::sync::{broadcast, RwLock};
use tracing::{debug, warn};

/// Maximum events retained in the history ring buffer.
const HISTORY_SIZE: usize = 1000;

/// The central event bus for inter-agent and system communication.
pub struct EventBus {
    /// Broadcast channel for all events.
    sender: broadcast::Sender<Event>,
    /// Per-agent event channels.
    agent_channels: DashMap<AgentId, broadcast::Sender<Event>>,
    /// Event history ring buffer.
    history: Arc<RwLock<VecDeque<Event>>>,
    /// Count of events where the intended recipient agent had no active
    /// receiver.  Incremented when an agent-targeted send finds the
    /// per-agent broadcast channel with no live subscribers (bug #3793).
    ///
    /// Note: tokio `broadcast::Sender::send` returns `Err` only when
    /// there are *no receivers* — the channel is a ring buffer that never
    /// rejects the sender; slow receivers get `RecvError::Lagged` instead.
    /// We therefore track the "no-receiver" condition as a drop because
    /// the event would not reach any consumer.
    dropped_count: AtomicU64,
    /// Timestamp of the last drop warning log (for rate-limiting).
    last_drop_warn: std::sync::Mutex<std::time::Instant>,
}

/// Return a short human-readable label for an `EventPayload` variant.
/// Used in drop-warning log fields so operators can identify which event
/// types are being silently lost without decoding the full payload.
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
        let (sender, _) = broadcast::channel(1024);
        Self {
            sender,
            agent_channels: DashMap::new(),
            history: Arc::new(RwLock::new(VecDeque::with_capacity(HISTORY_SIZE))),
            dropped_count: AtomicU64::new(0),
            last_drop_warn: std::sync::Mutex::new(std::time::Instant::now()),
        }
    }

    /// Publish an event to the bus.
    ///
    /// # Drop semantics (bug #3793)
    ///
    /// `tokio::broadcast` is a *ring buffer*: the sender never blocks and
    /// never returns an error because the channel is "full".  Instead, slow
    /// receivers are skipped with `RecvError::Lagged`.  `Sender::send`
    /// returns `Err` **only** when there are zero active receivers.
    ///
    /// We therefore distinguish two cases:
    ///
    /// * **Global broadcast / pattern / system channels** — it is normal for
    ///   these to have no subscribers during early boot or when no agent is
    ///   listening.  A failed send is logged at `debug` level only.
    /// * **Per-agent channels** — if a channel entry exists but has no
    ///   receiver, the agent has disconnected without cleaning up.  This is
    ///   a genuine drop that warrants a `warn!` with event-type information
    ///   so operators can diagnose which event kinds are lost.
    pub async fn publish(&self, event: Event) {
        debug!(
            event_id = %event.id,
            source = %event.source,
            kind = payload_kind(&event.payload),
            "Publishing event"
        );

        // Store in history
        {
            let mut history = self.history.write().await;
            if history.len() >= HISTORY_SIZE {
                history.pop_front();
            }
            history.push_back(event.clone());
        }

        // Route to target
        match &event.target {
            EventTarget::Agent(agent_id) => {
                if let Some(sender) = self.agent_channels.get(agent_id) {
                    // Per-agent channel: Err means no active receiver for this
                    // specific agent — the event is genuinely lost.
                    if sender.send(event.clone()).is_err() {
                        let total =
                            self.dropped_count.fetch_add(1, Ordering::Relaxed) + 1;
                        // Rate-limit to at most one warning per 10 seconds.
                        if let Ok(mut last) = self.last_drop_warn.lock() {
                            if last.elapsed() >= std::time::Duration::from_secs(10) {
                                warn!(
                                    agent_id = %agent_id,
                                    event_id = %event.id,
                                    event_kind = payload_kind(&event.payload),
                                    total_dropped = total,
                                    "Event bus: agent has no active receiver, event dropped — \
                                     consider increasing queue capacity or checking agent health",
                                );
                                *last = std::time::Instant::now();
                            }
                        }
                    }
                }
            }
            EventTarget::Broadcast => {
                // Global broadcast: no-receiver is expected when no system
                // subscriber is registered; log at debug only.
                if self.sender.send(event.clone()).is_err() {
                    debug!(
                        event_id = %event.id,
                        event_kind = payload_kind(&event.payload),
                        "Broadcast event: no global subscribers"
                    );
                }
                let mut agent_drops: u64 = 0;
                for entry in self.agent_channels.iter() {
                    if entry.value().send(event.clone()).is_err() {
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
                                "Event bus: broadcast reached agents with no active receivers, \
                                 events dropped — consider increasing queue capacity",
                            );
                            *last = std::time::Instant::now();
                        }
                    }
                }
            }
            EventTarget::Pattern(_pattern) => {
                // No-receiver on the pattern channel is non-critical.
                if self.sender.send(event.clone()).is_err() {
                    debug!(
                        event_id = %event.id,
                        event_kind = payload_kind(&event.payload),
                        "Pattern event: no global subscribers"
                    );
                }
            }
            EventTarget::System => {
                // No-receiver on the system channel is non-critical.
                if self.sender.send(event.clone()).is_err() {
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
    pub fn subscribe_agent(&self, agent_id: AgentId) -> broadcast::Receiver<Event> {
        let entry = self.agent_channels.entry(agent_id).or_insert_with(|| {
            let (tx, _) = broadcast::channel(256);
            tx
        });
        entry.subscribe()
    }

    /// Subscribe to all broadcast/system events.
    pub fn subscribe_all(&self) -> broadcast::Receiver<Event> {
        self.sender.subscribe()
    }

    /// Get recent event history.
    pub async fn history(&self, limit: usize) -> Vec<Event> {
        let history = self.history.read().await;
        history.iter().rev().take(limit).cloned().collect()
    }

    /// Return the total number of events dropped due to full channels.
    pub fn dropped_count(&self) -> u64 {
        self.dropped_count.load(Ordering::Relaxed)
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
        match received.payload {
            EventPayload::System(SystemEvent::HealthCheck { status }) => {
                assert_eq!(status, "ok");
            }
            other => panic!("Expected HealthCheck payload, got {:?}", other),
        }
    }
}
