//! Real-time audit log streaming (SSE) endpoint (#3749 11/N).

use super::AppState;
use axum::extract::{Query, State};
use axum::response::IntoResponse;
use std::collections::HashMap;
use std::sync::Arc;

pub fn router() -> axum::Router<Arc<AppState>> {
    axum::Router::new().route("/logs/stream", axum::routing::get(logs_stream))
}

/// GET /api/logs/stream — SSE endpoint for real-time audit log streaming.
///
/// Streams new audit entries as Server-Sent Events. Accepts optional query
/// parameters for filtering:
///   - `level`  — filter by classified level (info, warn, error)
///   - `filter` — text substring filter across action/detail/agent_id
///
/// A heartbeat ping is sent every 15 seconds to keep the connection alive.
/// The endpoint polls the audit log every second. The first poll sends up
/// to the most recent 200 entries as a bounded backfill (prevents a flood
/// when subscribing against a long-running daemon); every subsequent poll
/// uses a cursor (`since_seq`) so bursts faster than the previous
/// `recent(200)` sliding window are no longer silently truncated.
#[utoipa::path(get, path = "/api/logs/stream", tag = "system", responses((status = 200, description = "SSE log stream")))]
pub async fn logs_stream(
    State(state): State<Arc<AppState>>,
    Query(params): Query<HashMap<String, String>>,
) -> axum::response::Response {
    use axum::response::sse::{Event, KeepAlive, Sse};

    let level_filter = params.get("level").cloned().unwrap_or_default();
    let text_filter = params
        .get("filter")
        .cloned()
        .unwrap_or_default()
        .to_lowercase();

    let (tx, rx) = tokio::sync::mpsc::channel::<
        Result<axum::response::sse::Event, std::convert::Infallible>,
    >(256);

    let forwarder = tokio::spawn(async move {
        // Cursor-based polling: `last_seq == 0` triggers the bounded
        // backfill on the very first iteration, then every subsequent
        // poll asks for entries strictly newer than the last delivered
        // seq. This is the fix for the dropped-burst bug — the previous
        // implementation always called `recent(200)` and skipped via
        // `entry.seq <= last_seq`, so a burst > 200 entries within one
        // poll interval would silently drop the oldest entries in that
        // burst when `recent`'s sliding window scrolled past them.
        let mut last_seq: u64 = 0;
        let mut first_poll = true;

        loop {
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;

            let entries = if first_poll {
                // First connect: cap the backfill so a long-running
                // daemon's audit log doesn't dump megabytes onto a
                // freshly-opened EventSource.
                state.kernel.audit().recent(200)
            } else {
                state.kernel.audit().since_seq(last_seq)
            };

            for entry in &entries {
                let action_str = format!("{:?}", entry.action);

                // Apply level filter
                if !level_filter.is_empty() {
                    let classified = classify_audit_level(&action_str);
                    if classified != level_filter {
                        continue;
                    }
                }

                // Apply text filter
                if !text_filter.is_empty() {
                    let haystack = format!("{} {} {}", action_str, entry.detail, entry.agent_id)
                        .to_lowercase();
                    if !haystack.contains(&text_filter) {
                        continue;
                    }
                }

                let json = serde_json::json!({
                    "seq": entry.seq,
                    "timestamp": entry.timestamp,
                    "agent_id": entry.agent_id,
                    "action": action_str,
                    "detail": entry.detail,
                    "outcome": entry.outcome,
                    "hash": entry.hash,
                });
                let data = serde_json::to_string(&json).unwrap_or_default();
                if tx.send(Ok(Event::default().data(data))).await.is_err() {
                    return; // Client disconnected
                }
            }

            // Update tracking state
            if let Some(last) = entries.last() {
                last_seq = last.seq;
            }
            first_poll = false;
        }
    });

    // Watchdog: the forwarder handle was previously dropped, so a panic in
    // the poll loop left the EventSource hung with no server trace. Watch it
    // and log on panic (#5137). The watchdog ends naturally when the
    // forwarder returns (client disconnect) — no leak.
    tokio::spawn(async move {
        if let Err(e) = forwarder.await {
            if e.is_panic() {
                tracing::error!(
                    error = %e,
                    "SSE log forwarder task panicked; EventSource stalled"
                );
            }
        }
    });

    let rx_stream = tokio_stream::wrappers::ReceiverStream::new(rx);
    Sse::new(rx_stream)
        .keep_alive(
            KeepAlive::new()
                .interval(std::time::Duration::from_secs(15))
                .text("ping"),
        )
        .into_response()
}

/// Classify an audit action string into a level (info, warn, error).
fn classify_audit_level(action: &str) -> &'static str {
    let a = action.to_lowercase();
    if a.contains("error") || a.contains("fail") || a.contains("crash") || a.contains("denied") {
        "error"
    } else if a.contains("warn") || a.contains("block") || a.contains("kill") {
        "warn"
    } else {
        "info"
    }
}
