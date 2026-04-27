//! Session lifecycle event bus.
//!
//! Narrow, push-based pub/sub for session-scoped events: created, turn
//! started, turn completed, turn failed, agent terminated.
//!
//! Modeled after openclaw's `src/sessions/session-lifecycle-events.ts`.
//!
//! This bus is intentionally separate from [`crate::event_bus::EventBus`],
//! which carries broader inter-agent and system events. Subsystems that only
//! care about session lifecycle (triggers, observability, audit) can subscribe
//! here without paying the cost of the wider event stream.
//!
//! Publishing is fire-and-forget — `publish` never blocks the kernel and
//! silently drops the event if no subscriber is listening.

use librefang_types::agent::{AgentId, SessionId};
use tokio::sync::broadcast;

/// Default capacity for the broadcast channel ring buffer.
const DEFAULT_CAPACITY: usize = 256;

/// Lifecycle events emitted by the kernel for each session/turn.
#[derive(Debug, Clone)]
pub enum SessionLifecycleEvent {
    /// A new session row was created in memory (first turn for that sid).
    SessionCreated {
        agent_id: AgentId,
        session_id: SessionId,
    },
    /// An agent loop turn started for a session.
    TurnStarted {
        agent_id: AgentId,
        session_id: SessionId,
    },
    /// A turn completed (success).
    TurnCompleted {
        agent_id: AgentId,
        session_id: SessionId,
        message_count: usize,
    },
    /// A turn failed (loop returned an error or panicked).
    TurnFailed {
        agent_id: AgentId,
        session_id: SessionId,
        error: String,
    },
    /// An agent was terminated; its sessions are no longer active.
    AgentTerminated { agent_id: AgentId, reason: String },
}

/// Broadcast bus for [`SessionLifecycleEvent`].
///
/// Holds a single tokio `broadcast::Sender`. Subscribers obtain a
/// `broadcast::Receiver` via [`SessionLifecycleBus::subscribe`]. When the
/// channel buffer overflows for a slow subscriber, the standard tokio
/// broadcast `Lagged` error is delivered on `recv` — callers should handle
/// it explicitly.
#[derive(Debug)]
pub struct SessionLifecycleBus {
    sender: broadcast::Sender<SessionLifecycleEvent>,
}

impl SessionLifecycleBus {
    /// Create a new bus with the given broadcast channel capacity.
    pub fn new(capacity: usize) -> Self {
        let (sender, _) = broadcast::channel(capacity.max(1));
        Self { sender }
    }

    /// Publish an event. Best-effort — drops silently when no subscribers.
    ///
    /// Never blocks: tokio's `broadcast::Sender::send` returns immediately,
    /// and we ignore the `SendError` raised when the receiver count is zero.
    pub fn publish(&self, event: SessionLifecycleEvent) {
        let _ = self.sender.send(event);
    }

    /// Subscribe to lifecycle events.
    pub fn subscribe(&self) -> broadcast::Receiver<SessionLifecycleEvent> {
        self.sender.subscribe()
    }

    /// Number of currently active subscribers.
    pub fn receiver_count(&self) -> usize {
        self.sender.receiver_count()
    }
}

impl Default for SessionLifecycleBus {
    fn default() -> Self {
        Self::new(DEFAULT_CAPACITY)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn matches_turn_started(
        ev: &SessionLifecycleEvent,
        expected_agent: AgentId,
        expected_session: SessionId,
    ) -> bool {
        matches!(
            ev,
            SessionLifecycleEvent::TurnStarted { agent_id, session_id }
                if *agent_id == expected_agent && *session_id == expected_session
        )
    }

    #[tokio::test]
    async fn publish_then_subscribe_receives_event() {
        let bus = SessionLifecycleBus::new(16);
        let mut rx = bus.subscribe();

        let agent_id = AgentId::new();
        let session_id = SessionId::new();
        bus.publish(SessionLifecycleEvent::TurnStarted {
            agent_id,
            session_id,
        });

        let received = rx.recv().await.expect("receive event");
        assert!(matches_turn_started(&received, agent_id, session_id));
    }

    #[tokio::test]
    async fn multiple_subscribers_all_receive_event() {
        let bus = SessionLifecycleBus::new(16);
        let mut rx1 = bus.subscribe();
        let mut rx2 = bus.subscribe();
        let mut rx3 = bus.subscribe();

        let agent_id = AgentId::new();
        let session_id = SessionId::new();
        bus.publish(SessionLifecycleEvent::SessionCreated {
            agent_id,
            session_id,
        });

        for rx in [&mut rx1, &mut rx2, &mut rx3] {
            let ev = rx.recv().await.expect("receive event");
            assert!(matches!(
                ev,
                SessionLifecycleEvent::SessionCreated { agent_id: a, session_id: s }
                    if a == agent_id && s == session_id
            ));
        }
    }

    #[tokio::test]
    async fn publish_with_no_subscribers_is_no_op() {
        let bus = SessionLifecycleBus::new(16);
        assert_eq!(bus.receiver_count(), 0);

        // No panic, no error returned to caller.
        bus.publish(SessionLifecycleEvent::AgentTerminated {
            agent_id: AgentId::new(),
            reason: "test".to_string(),
        });

        assert_eq!(bus.receiver_count(), 0);
    }

    #[tokio::test]
    async fn dropping_subscriber_does_not_break_others() {
        let bus = SessionLifecycleBus::new(16);
        let mut rx_keep = bus.subscribe();
        let rx_drop = bus.subscribe();
        assert_eq!(bus.receiver_count(), 2);

        drop(rx_drop);
        assert_eq!(bus.receiver_count(), 1);

        let agent_id = AgentId::new();
        let session_id = SessionId::new();
        bus.publish(SessionLifecycleEvent::TurnCompleted {
            agent_id,
            session_id,
            message_count: 5,
        });

        let received = rx_keep.recv().await.expect("receive event");
        assert!(matches!(
            received,
            SessionLifecycleEvent::TurnCompleted {
                agent_id: a,
                session_id: s,
                message_count: 5,
            } if a == agent_id && s == session_id
        ));
    }
}
