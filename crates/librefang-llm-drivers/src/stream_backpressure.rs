//! Helpers for propagating receiver-disconnect as backpressure on streaming
//! LLM drivers.
//!
//! Streaming drivers run an SSE/byte loop that fans out `StreamEvent`s through
//! an `mpsc::Sender<StreamEvent>`. If the consumer drops the receiver mid-flight
//! (client disconnect, request cancel, ancestor task aborted), every subsequent
//! `tx.send(...).await` fails. Without checking, the driver keeps reading the
//! upstream provider's full SSE response — burning tokens, memory, and time
//! for output nobody will read (#3769).
//!
//! Pattern used by every streaming driver:
//!
//! ```ignore
//! let mut receiver_dropped = false;
//! while let Some(chunk) = byte_stream.next().await {
//!     if receiver_dropped { break; }
//!     // ... parse SSE ...
//!     emit!(receiver_dropped, tx, StreamEvent::TextDelta { text });
//! }
//! ```
//!
//! The `emit!` macro below sets `receiver_dropped = true` on send failure but
//! does **not** short-circuit the surrounding line/event parser — the outer
//! byte-stream loop bails on the next iteration. This avoids tearing through
//! complex per-driver branch structure and keeps accumulator state coherent
//! for the final `CompletionResponse`.

/// Forward a `StreamEvent` to the consumer, marking the stream cancelled if
/// the receiver has dropped. The `$flag` ident must be a `bool` mutable in
/// scope; the outer byte-stream loop is expected to check it and break.
///
/// Logs at `debug!` (not `warn!`) the first time the receiver disconnects —
/// consumer disconnect is normal (user navigated away, request cancelled),
/// not an error condition.
#[macro_export]
macro_rules! send_or_mark_dropped {
    ($flag:ident, $tx:expr, $event:expr) => {{
        if !$flag {
            if $tx.send($event).await.is_err() {
                tracing::debug!("stream consumer disconnected; aborting upstream LLM stream");
                $flag = true;
            }
        }
    }};
}

#[cfg(test)]
mod tests {
    use crate::llm_driver::StreamEvent;
    use librefang_types::message::{StopReason, TokenUsage};
    use tokio::sync::mpsc;

    /// Simulates the driver loop: a stream of events, but the receiver is
    /// dropped after the first one. The flag must flip and the loop must
    /// exit on the next iteration instead of running to completion.
    #[tokio::test]
    async fn drops_set_flag_and_break_outer_loop() {
        let (tx, mut rx) = mpsc::channel::<StreamEvent>(1);

        let driver = tokio::spawn(async move {
            let mut receiver_dropped = false;
            let mut sent = 0usize;
            // Pretend upstream gives us 1000 chunks. A buggy driver would
            // emit all 1000; the fixed driver bails after the first send
            // failure.
            for i in 0..1000 {
                if receiver_dropped {
                    break;
                }
                send_or_mark_dropped!(
                    receiver_dropped,
                    tx,
                    StreamEvent::TextDelta {
                        text: format!("chunk {i}"),
                    }
                );
                if !receiver_dropped {
                    sent += 1;
                }
            }
            // Best-effort final send — byte loop is done, nothing to abort
            // even if the receiver has dropped.
            let _ = tx
                .send(StreamEvent::ContentComplete {
                    stop_reason: StopReason::EndTurn,
                    usage: TokenUsage::default(),
                })
                .await;
            (sent, receiver_dropped)
        });

        // Consume one event, then drop the receiver to simulate client gone.
        let _ = rx.recv().await;
        drop(rx);

        let (sent, dropped) = driver.await.expect("driver task should finish");
        assert!(
            sent < 10,
            "driver kept sending after receiver dropped: sent={sent}"
        );
        assert!(dropped, "receiver_dropped flag must be set");
    }

    /// Receiver stays alive: every event flows through, no flag flip.
    #[tokio::test]
    async fn live_receiver_gets_every_event() {
        let (tx, mut rx) = mpsc::channel::<StreamEvent>(8);

        let driver = tokio::spawn(async move {
            let mut receiver_dropped = false;
            for i in 0..5 {
                send_or_mark_dropped!(
                    receiver_dropped,
                    tx,
                    StreamEvent::TextDelta {
                        text: format!("chunk {i}"),
                    }
                );
            }
            receiver_dropped
        });

        let mut received = 0;
        while rx.recv().await.is_some() {
            received += 1;
        }

        let dropped = driver.await.unwrap();
        assert!(!dropped, "receiver was alive — flag must stay false");
        assert_eq!(received, 5);
    }
}
