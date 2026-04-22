//! Event bus — pub/sub with pattern matching and history ring buffer.

use dashmap::DashMap;
use librefang_types::agent::AgentId;
use librefang_types::event::{Event, EventTarget};
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
    /// Count of events dropped due to full channels.
    dropped_count: AtomicU64,
    /// Timestamp of the last drop warning log (for rate-limiting).
    last_drop_warn: std::sync::Mutex<std::time::Instant>,
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
    pub async fn publish(&self, event: Event) {
        debug!(
            event_id = %event.id,
            source = %event.source,
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
        let mut drops: u64 = 0;
        match &event.target {
            EventTarget::Agent(agent_id) => {
                if let Some(sender) = self.agent_channels.get(agent_id) {
                    if sender.send(event.clone()).is_err() {
                        drops += 1;
                    }
                }
            }
            EventTarget::Broadcast => {
                if self.sender.send(event.clone()).is_err() {
                    drops += 1;
                }
                for entry in self.agent_channels.iter() {
                    if entry.value().send(event.clone()).is_err() {
                        drops += 1;
                    }
                }
            }
            EventTarget::Pattern(_pattern) => {
                // Phase 1: broadcast to all for pattern matching
                if self.sender.send(event.clone()).is_err() {
                    drops += 1;
                }
            }
            EventTarget::System => {
                if self.sender.send(event.clone()).is_err() {
                    drops += 1;
                }
            }
        }

        if drops > 0 {
            let total = self.dropped_count.fetch_add(drops, Ordering::Relaxed) + drops;
            // Rate-limit warning logs to once per 10 seconds.
            if let Ok(mut last) = self.last_drop_warn.lock() {
                if last.elapsed() >= std::time::Duration::from_secs(10) {
                    warn!(
                        dropped = drops,
                        total_dropped = total,
                        "Event bus: channel full, events dropped"
                    );
                    *last = std::time::Instant::now();
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
