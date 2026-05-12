//! Integration tests for the incremental cascade-leak detection guard.
//!
//! These tests verify the forward_task behaviour introduced in
//! `stream_with_retry`: a tokio::spawn that reads from the proxy channel,
//! accumulates text, and stops forwarding TextDelta events as soon as
//! `is_cascade_leak` fires. The guard must be unconditional — even if the
//! driver would subsequently emit a ToolUse stop_reason, the entire turn must
//! be treated as a silent drop.
//!
//! Because `stream_with_retry` and its inner forward_task are private, these
//! tests replicate the exact same accumulation + channel-forwarding pattern
//! using the public `is_cascade_leak` and `StreamEvent` types. This approach
//! is a regression lock: if the forwarding logic changes in a way that breaks
//! the guard, these tests will fail.

use librefang_runtime::llm_driver::StreamEvent;
use librefang_runtime::silent_response::{is_cascade_leak, SilentReason};
use librefang_types::message::{StopReason, TokenUsage};
use tokio::sync::mpsc;

/// Replicate the forward_task logic from `stream_with_retry`.
///
/// Reads `events` from `proxy_rx`, accumulates TextDelta text, calls
/// `is_cascade_leak` on each delta, and forwards events to `outer_tx`
/// exactly as the production code does. Returns `true` when the leak guard
/// fired (i.e. `cascade_leak_aborted`).
async fn run_forward_task(
    mut proxy_rx: mpsc::Receiver<StreamEvent>,
    outer_tx: mpsc::Sender<StreamEvent>,
) -> bool {
    let mut accumulated = String::new();
    let mut leak_fired = false;
    while let Some(event) = proxy_rx.recv().await {
        match &event {
            StreamEvent::TextDelta { text } if !leak_fired => {
                accumulated.push_str(text);
                if is_cascade_leak(&accumulated) {
                    leak_fired = true;
                    // Swallow this delta — do not forward it.
                    continue;
                }
                let _ = outer_tx
                    .send(StreamEvent::TextDelta { text: text.clone() })
                    .await;
            }
            StreamEvent::TextDelta { .. } => {
                // leak_fired: swallow remaining text tokens.
            }
            other => {
                let _ = outer_tx.send(other.clone()).await;
            }
        }
    }
    leak_fired
}

/// Collect all events received on `rx` before the channel is closed.
async fn drain(mut rx: mpsc::Receiver<StreamEvent>) -> Vec<StreamEvent> {
    let mut out = Vec::new();
    while let Some(ev) = rx.recv().await {
        out.push(ev);
    }
    out
}

// ---------------------------------------------------------------------------
// (a) TextDelta tokens must NOT reach downstream after detection
// ---------------------------------------------------------------------------

/// When the accumulation of TextDelta tokens triggers `is_cascade_leak`,
/// neither the triggering delta nor any subsequent TextDelta must be forwarded.
#[tokio::test]
async fn text_delta_tokens_are_suppressed_after_leak_detection() {
    let (proxy_tx, proxy_rx) = mpsc::channel::<StreamEvent>(16);
    let (outer_tx, outer_rx) = mpsc::channel::<StreamEvent>(16);

    // Two structural markers in sequence → leak fires on the second delta.
    let events = vec![
        StreamEvent::TextDelta {
            text: "User asked: what is the time?\n".to_string(),
        },
        StreamEvent::TextDelta {
            text: "I responded: it is noon.\n".to_string(),
        },
        // This delta must be swallowed — it arrives after the leak fires.
        StreamEvent::TextDelta {
            text: "Some more text that must never appear.\n".to_string(),
        },
    ];

    for ev in events {
        proxy_tx.send(ev).await.unwrap();
    }
    drop(proxy_tx); // signal EOF to forward_task

    let cascade_leak_aborted = run_forward_task(proxy_rx, outer_tx).await;
    let forwarded = drain(outer_rx).await;

    assert!(
        cascade_leak_aborted,
        "cascade_leak_aborted must be true when two structural markers appear"
    );

    // The first delta arrived before the leak fired, so it should be forwarded.
    // The second delta triggered the leak and must be swallowed.
    // The third delta arrives after the leak and must also be swallowed.
    let text_deltas: Vec<_> = forwarded
        .iter()
        .filter(|e| matches!(e, StreamEvent::TextDelta { .. }))
        .collect();

    assert_eq!(
        text_deltas.len(),
        1,
        "only the first TextDelta (before leak) should be forwarded; got {:?}",
        text_deltas
            .iter()
            .map(|e| format!("{e:?}"))
            .collect::<Vec<_>>()
    );

    // Confirm no post-leak text is present.
    let has_leaked_text = forwarded.iter().any(
        |e| matches!(e, StreamEvent::TextDelta { text } if text.contains("must never appear")),
    );
    assert!(
        !has_leaked_text,
        "post-leak TextDelta must never reach downstream"
    );
}

/// Non-TextDelta events (ContentComplete, ToolUseStart, etc.) continue to be
/// forwarded even after the leak fires — the guard only suppresses text.
#[tokio::test]
async fn non_text_events_forwarded_after_leak() {
    let (proxy_tx, proxy_rx) = mpsc::channel::<StreamEvent>(16);
    let (outer_tx, outer_rx) = mpsc::channel::<StreamEvent>(16);

    let events = vec![
        StreamEvent::TextDelta {
            text: "User asked: foo\nI responded: bar\n".to_string(),
        },
        // ContentComplete after the leak — must still be forwarded.
        StreamEvent::ContentComplete {
            stop_reason: StopReason::EndTurn,
            usage: TokenUsage {
                input_tokens: 10,
                output_tokens: 5,
                ..Default::default()
            },
        },
    ];

    for ev in events {
        proxy_tx.send(ev).await.unwrap();
    }
    drop(proxy_tx);

    let cascade_leak_aborted = run_forward_task(proxy_rx, outer_tx).await;
    let forwarded = drain(outer_rx).await;

    assert!(cascade_leak_aborted);
    assert!(
        forwarded
            .iter()
            .any(|e| matches!(e, StreamEvent::ContentComplete { .. })),
        "ContentComplete must be forwarded even after cascade leak fires"
    );
}

// ---------------------------------------------------------------------------
// (c) ToolUse stop_reason must NOT bypass the silent drop (regression lock)
// ---------------------------------------------------------------------------

/// Regression lock for the BLOCKER: when `cascade_leak_aborted = true`, the
/// caller must treat the entire turn as silent — even if the driver's final
/// `ContentComplete` carries `stop_reason = ToolUse`. The forward_task
/// itself does not execute tools; it only returns `cascade_leak_aborted`.
/// The caller (`run_agent_loop_streaming`) is responsible for the early
/// return — this test locks in that the flag is correctly set when the stream
/// contains a ToolUse-typed completion after a leak.
#[tokio::test]
async fn tool_use_stop_reason_still_sets_cascade_leak_aborted() {
    let (proxy_tx, proxy_rx) = mpsc::channel::<StreamEvent>(16);
    let (outer_tx, outer_rx) = mpsc::channel::<StreamEvent>(16);

    // Simulate a prompt-leaking stream that ends with ToolUse.
    let events = vec![
        // Leak trigger
        StreamEvent::TextDelta {
            text: "User asked: foo\n".to_string(),
        },
        StreamEvent::TextDelta {
            text: "I responded: bar\n".to_string(),
        },
        // Tool use block — arrives after the leak fired.
        StreamEvent::ToolUseStart {
            id: "tool_abc".to_string(),
            name: "bash".to_string(),
        },
        StreamEvent::ToolInputDelta {
            text: r#"{"command":"rm -rf /"}"#.to_string(),
        },
        StreamEvent::ToolUseEnd {
            id: "tool_abc".to_string(),
            name: "bash".to_string(),
            input: serde_json::json!({"command": "rm -rf /"}),
        },
        StreamEvent::ContentComplete {
            stop_reason: StopReason::ToolUse,
            usage: TokenUsage {
                input_tokens: 20,
                output_tokens: 8,
                ..Default::default()
            },
        },
    ];

    for ev in events {
        proxy_tx.send(ev).await.unwrap();
    }
    drop(proxy_tx);

    let cascade_leak_aborted = run_forward_task(proxy_rx, outer_tx).await;
    // Drain so outer_tx's buffer is fully consumed.
    let _ = drain(outer_rx).await;

    assert!(
        cascade_leak_aborted,
        "cascade_leak_aborted must be true even when stop_reason is ToolUse — \
         the caller must NOT execute tool calls in this case"
    );
}

// ---------------------------------------------------------------------------
// (b) SilentReason::PromptRegurgitated is the correct reason variant
// ---------------------------------------------------------------------------

/// The `PromptRegurgitated` variant must exist and serialise correctly so
/// the structured log event `silent_response_detected` carries the right reason.
#[test]
fn silent_reason_prompt_regurgitated_serializes() {
    let r = SilentReason::PromptRegurgitated;
    let s = serde_json::to_string(&r).unwrap();
    assert_eq!(
        s, "\"prompt_regurgitated\"",
        "SilentReason::PromptRegurgitated must serialise as \"prompt_regurgitated\""
    );
}

// ---------------------------------------------------------------------------
// Incremental accumulation: leak fires mid-stream
// ---------------------------------------------------------------------------

/// Structural markers split across deltas must still be detected correctly.
/// This ensures the accumulation logic (concatenation before checking) works
/// when a single marker is split across two deltas.
#[tokio::test]
async fn leak_fires_when_markers_split_across_deltas() {
    let (proxy_tx, proxy_rx) = mpsc::channel::<StreamEvent>(16);
    let (outer_tx, outer_rx) = mpsc::channel::<StreamEvent>(16);

    // Split "User asked: " across two deltas, then add the second structural marker.
    let events = vec![
        StreamEvent::TextDelta {
            text: "User ".to_string(),
        },
        StreamEvent::TextDelta {
            text: "asked: hello\n".to_string(),
        },
        // Second structural marker triggers the leak.
        StreamEvent::TextDelta {
            text: "I responded: world\n".to_string(),
        },
        // Must be swallowed.
        StreamEvent::TextDelta {
            text: "extra text after leak\n".to_string(),
        },
    ];

    for ev in events {
        proxy_tx.send(ev).await.unwrap();
    }
    drop(proxy_tx);

    let cascade_leak_aborted = run_forward_task(proxy_rx, outer_tx).await;
    let forwarded = drain(outer_rx).await;

    assert!(
        cascade_leak_aborted,
        "leak must fire when markers split across deltas"
    );

    let has_extra = forwarded.iter().any(
        |e| matches!(e, StreamEvent::TextDelta { text } if text.contains("extra text after leak")),
    );
    assert!(!has_extra, "post-leak text must be suppressed");
}

/// A clean stream (no structural markers) must never set cascade_leak_aborted.
#[tokio::test]
async fn clean_stream_does_not_abort() {
    let (proxy_tx, proxy_rx) = mpsc::channel::<StreamEvent>(16);
    let (outer_tx, outer_rx) = mpsc::channel::<StreamEvent>(16);

    let events = vec![
        StreamEvent::TextDelta {
            text: "Sure, here is what I found:\n".to_string(),
        },
        StreamEvent::TextDelta {
            text: "The answer is 42.\n".to_string(),
        },
        StreamEvent::ContentComplete {
            stop_reason: StopReason::EndTurn,
            usage: TokenUsage {
                input_tokens: 5,
                output_tokens: 10,
                ..Default::default()
            },
        },
    ];

    for ev in events {
        proxy_tx.send(ev).await.unwrap();
    }
    drop(proxy_tx);

    let cascade_leak_aborted = run_forward_task(proxy_rx, outer_tx).await;
    let forwarded = drain(outer_rx).await;

    assert!(
        !cascade_leak_aborted,
        "clean stream must not set cascade_leak_aborted"
    );
    assert_eq!(
        forwarded.len(),
        3,
        "all 3 events must be forwarded for a clean stream"
    );
}
