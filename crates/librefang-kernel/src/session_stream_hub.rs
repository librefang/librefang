//! Session-scoped fan-out for in-turn `StreamEvent`s.
//!
//! The kernel's streaming entry point (`send_message_streaming_*`) creates a
//! per-turn `mpsc::channel<StreamEvent>` between the agent loop (producer) and
//! the HTTP/SSE handler that originally triggered the turn (consumer). That
//! single-consumer channel is fine for the originating client, but it makes
//! it impossible for any *other* client to observe the same turn — the
//! desktop app cannot watch a conversation the CLI just kicked off, and a
//! reconnecting browser tab cannot resume mid-turn.
//!
//! `SessionStreamHub` is a side-channel keyed by `SessionId`: a
//! `tokio::sync::broadcast` sender per active session. The kernel installs a
//! short forwarder task that drains the producer mpsc, fans each event out
//! to both the original caller (via a non-blocking `try_send`) and the
//! broadcast channel. Any number of late attachers can `subscribe()` and
//! start receiving events from the moment they connect.
//!
//! Hub entries are created lazily on first publish or first subscribe and
//! pruned when no live receivers remain (`gc_idle`). Lossiness is intentional:
//! a slow attacher (including the originating caller) lags rather than
//! backpressuring the producer or starving other attachers.

use dashmap::DashMap;
use librefang_llm_driver::StreamEvent;
use librefang_types::agent::SessionId;
use std::sync::Arc;
use tokio::sync::broadcast;

/// Buffer size for per-session broadcast channels.
///
/// Sized to absorb a few seconds of fast token streaming (Anthropic ~50 tok/s,
/// Groq ~500 tok/s) without lag for typical attachers. Slow attachers that
/// fall behind will see `RecvError::Lagged` and may resync from the next
/// event — broadcast is intentionally lossy here.
const SESSION_BROADCAST_CAPACITY: usize = 1024;

/// Session-scoped event hub for multi-client SSE attach.
#[derive(Debug)]
pub struct SessionStreamHub {
    senders: DashMap<SessionId, broadcast::Sender<StreamEvent>>,
}

impl SessionStreamHub {
    pub fn new() -> Self {
        Self {
            senders: DashMap::new(),
        }
    }

    /// Get (or create) the broadcast sender for a session.
    ///
    /// Always returns a sender — entries are created on first publish so that
    /// late attachers can also call `subscribe(session_id)` before any turn
    /// has run for that session and still receive future events.
    pub fn sender(&self, session_id: SessionId) -> broadcast::Sender<StreamEvent> {
        if let Some(existing) = self.senders.get(&session_id) {
            return existing.clone();
        }
        let entry = self
            .senders
            .entry(session_id)
            .or_insert_with(|| broadcast::channel(SESSION_BROADCAST_CAPACITY).0);
        entry.clone()
    }

    /// Subscribe to events for a session. Creates an empty channel on demand
    /// so attach calls before any producer has published still succeed.
    pub fn subscribe(&self, session_id: SessionId) -> broadcast::Receiver<StreamEvent> {
        self.sender(session_id).subscribe()
    }

    /// Drop entries with no active receivers — bounded memory under churn
    /// (lots of one-shot sessions). Cheap to call periodically; safe to skip.
    pub fn gc_idle(&self) -> usize {
        let stale: Vec<SessionId> = self
            .senders
            .iter()
            .filter(|e| e.value().receiver_count() == 0)
            .map(|e| *e.key())
            .collect();
        let count = stale.len();
        for id in stale {
            // Re-check under the entry lock: a subscriber may have appeared
            // between the snapshot above and this remove. DashMap's
            // remove_if covers that race.
            self.senders.remove_if(&id, |_, v| v.receiver_count() == 0);
        }
        count
    }

    /// Active session count (entries currently retained).
    pub fn active_session_count(&self) -> usize {
        self.senders.len()
    }
}

impl Default for SessionStreamHub {
    fn default() -> Self {
        Self::new()
    }
}

/// Wire a producer mpsc through the hub: every event is broadcast to all
/// session subscribers AND queued non-blocking to the originating caller.
///
/// Returns the producer sender (hand this to the agent loop) and the caller
/// receiver (hand this back to the original HTTP/SSE handler). Spawns one
/// short-lived forwarder task that exits when the producer side is dropped,
/// which in turn drops the caller sender so the SSE handler's stream ends
/// naturally.
///
/// Backpressure semantics:
/// - producer → forwarder mpsc(64): same as before; backpressures the agent
///   loop only if the forwarder itself stalls (it never does).
/// - forwarder → caller mpsc(SESSION_BROADCAST_CAPACITY): try_send only —
///   a slow CLI/desktop falls behind by at worst the buffer size before
///   drops begin. Crucially, the forwarder never awaits on the caller, so
///   one slow client cannot choke broadcast subscribers or the agent loop.
/// - forwarder → broadcast: synchronous, never blocks. Broadcast handles
///   slow attachers internally with `Lagged`.
pub fn install_stream_fanout(
    hub: &Arc<SessionStreamHub>,
    session_id: SessionId,
) -> (
    tokio::sync::mpsc::Sender<StreamEvent>,
    tokio::sync::mpsc::Receiver<StreamEvent>,
) {
    use tokio::sync::mpsc::error::TrySendError;

    let (producer_tx, mut producer_rx) = tokio::sync::mpsc::channel::<StreamEvent>(64);
    let (caller_tx, caller_rx) =
        tokio::sync::mpsc::channel::<StreamEvent>(SESSION_BROADCAST_CAPACITY);
    let broadcast_tx = hub.sender(session_id);

    crate::supervised_spawn::spawn_supervised("session_stream_fanout", async move {
        while let Some(event) = producer_rx.recv().await {
            // Best-effort fan-out to attached clients. `broadcast::send`
            // returns Err only when there are zero receivers — fine, just
            // means nobody is attached right now.
            let _ = broadcast_tx.send(event.clone());
            // Non-blocking forward to the originating caller. If the caller
            // dropped its receiver or fell behind by the full buffer, drop
            // the event rather than awaiting — keeps the forwarder loop and
            // hence the agent loop unblocked.
            match caller_tx.try_send(event) {
                Ok(()) | Err(TrySendError::Closed(_)) => {}
                Err(TrySendError::Full(_)) => {
                    tracing::debug!(
                        "originating caller queue full; dropping event from originating client (broadcast subscribers unaffected)"
                    );
                }
            }
        }
    });

    (producer_tx, caller_rx)
}

#[cfg(test)]
mod tests {
    use super::*;
    use librefang_types::agent::AgentId;
    use librefang_types::message::{StopReason, TokenUsage};

    fn fresh_session() -> SessionId {
        SessionId::for_channel(AgentId::new(), "test")
    }

    fn delta(t: &str) -> StreamEvent {
        StreamEvent::TextDelta {
            text: t.to_string(),
        }
    }

    #[tokio::test]
    async fn fanout_forwards_to_both_caller_and_subscriber() {
        let hub = Arc::new(SessionStreamHub::new());
        let session = fresh_session();
        let (producer_tx, mut caller_rx) = install_stream_fanout(&hub, session);
        let mut subscriber = hub.subscribe(session);

        producer_tx.send(delta("hi")).await.unwrap();
        producer_tx.send(delta("there")).await.unwrap();
        drop(producer_tx);

        // Caller side gets both events in order.
        let a = caller_rx.recv().await.unwrap();
        let b = caller_rx.recv().await.unwrap();
        assert!(matches!(a, StreamEvent::TextDelta { text } if text == "hi"));
        assert!(matches!(b, StreamEvent::TextDelta { text } if text == "there"));
        assert!(
            caller_rx.recv().await.is_none(),
            "caller stream should close"
        );

        // Subscriber receives the same sequence.
        let s1 = subscriber.recv().await.unwrap();
        let s2 = subscriber.recv().await.unwrap();
        assert!(matches!(s1, StreamEvent::TextDelta { text } if text == "hi"));
        assert!(matches!(s2, StreamEvent::TextDelta { text } if text == "there"));
    }

    #[tokio::test]
    async fn caller_drop_does_not_stall_broadcast() {
        let hub = Arc::new(SessionStreamHub::new());
        let session = fresh_session();
        let (producer_tx, caller_rx) = install_stream_fanout(&hub, session);
        let mut subscriber = hub.subscribe(session);

        // Originating caller hangs up immediately.
        drop(caller_rx);

        // Producer can still send — forwarder must keep draining and broadcasting.
        producer_tx
            .send(StreamEvent::ContentComplete {
                stop_reason: StopReason::EndTurn,
                usage: TokenUsage::default(),
            })
            .await
            .unwrap();
        let received = subscriber.recv().await.unwrap();
        assert!(matches!(received, StreamEvent::ContentComplete { .. }));
    }

    #[tokio::test]
    async fn multiple_subscribers_each_get_full_stream() {
        let hub = Arc::new(SessionStreamHub::new());
        let session = fresh_session();
        // Subscribe before any producer exists — empty channel preallocated.
        let mut a = hub.subscribe(session);
        let mut b = hub.subscribe(session);
        let (producer_tx, mut caller_rx) = install_stream_fanout(&hub, session);

        producer_tx.send(delta("x")).await.unwrap();
        drop(producer_tx);

        for sub in [&mut a, &mut b] {
            let ev = sub.recv().await.unwrap();
            assert!(matches!(ev, StreamEvent::TextDelta { text } if text == "x"));
        }
        let _ = caller_rx.recv().await;
    }

    #[tokio::test]
    async fn gc_idle_drops_unsubscribed_sessions() {
        let hub = Arc::new(SessionStreamHub::new());
        let session = fresh_session();
        // Touch the session so an entry exists.
        let _ = hub.sender(session);
        assert_eq!(hub.active_session_count(), 1);
        let pruned = hub.gc_idle();
        assert_eq!(pruned, 1);
        assert_eq!(hub.active_session_count(), 0);
    }
}
