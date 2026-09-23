//! Real-time audit log streaming (SSE) endpoint (#3749 11/N).

use super::AppState;
use axum::extract::{Query, State};
use axum::response::IntoResponse;
use librefang_kernel::audit::AuditAction;
use std::collections::HashMap;
use std::sync::Arc;

type SseEventResult = Result<axum::response::sse::Event, std::convert::Infallible>;

pub fn router() -> axum::Router<Arc<AppState>> {
    axum::Router::new().route("/logs/stream", axum::routing::get(logs_stream))
}

/// GET /api/logs/stream — SSE endpoint for real-time audit log streaming.
///
/// Streams new audit entries as Server-Sent Events. Accepts optional query
/// parameters for filtering:
///   - `level`  — filter by classified level (info, warn, error, debug), classified from the entry's `outcome` (leading `error` / `fail` / `denied`, `warn`, `debug`), falling back to the action so `PermissionDenied` and `BudgetExceeded` are errors; each event carries the classification as `level`
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

    let level_filter = params
        .get("level")
        .map(|level| level.to_ascii_lowercase())
        .unwrap_or_default();
    let text_filter = params
        .get("filter")
        .cloned()
        .unwrap_or_default()
        .to_lowercase();

    let (tx, rx) = tokio::sync::mpsc::channel::<SseEventResult>(256);

    // Subscribe to the kernel shutdown signal so this detached task can
    // exit promptly on daemon shutdown. Without this the only loop exit
    // is `tx.send` returning `Err` (client disconnect), so the spawned
    // task holds the `Arc<AppState>` (and therefore the whole kernel
    // graph via `state.kernel`) until the OS tears the socket down —
    // pinning the entire `AppState` for as long as any dashboard tab
    // keeps an SSE channel open (#5144).
    let mut shutdown_rx = state.kernel.supervisor_ref().subscribe();

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
            tokio::select! {
                _ = tokio::time::sleep(std::time::Duration::from_secs(1)) => {}
                _ = tx.closed() => {
                    return; // Client disconnected while the audit log was quiet.
                }
                _ = shutdown_rx.changed() => {
                    if *shutdown_rx.borrow() {
                        return; // Kernel shutting down — drop Arc<AppState>.
                    }
                    continue;
                }
            }

            let entries = if first_poll {
                // First connect: cap the backfill so a long-running
                // daemon's audit log doesn't dump megabytes onto a
                // freshly-opened EventSource.
                state.kernel.audit().recent(200)
            } else {
                state.kernel.audit().since_seq(last_seq)
            };

            for entry in &entries {
                let level = classify_audit_level(&entry.action, &entry.outcome);
                if !level_filter.is_empty() && level != level_filter {
                    continue;
                }

                let action_str = format!("{:?}", entry.action);

                // Apply text filter
                if !text_filter.is_empty() {
                    let action_matches = action_str.to_ascii_lowercase().contains(&text_filter);
                    let detail_matches = contains_lowercase_filter(&entry.detail, &text_filter);
                    let agent_matches = if action_matches || detail_matches {
                        false
                    } else {
                        contains_lowercase_filter(&entry.agent_id.to_string(), &text_filter)
                    };
                    if !action_matches && !detail_matches && !agent_matches {
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
                    "level": level,
                    "hash": entry.hash,
                });
                let data = match serde_json::to_string(&json) {
                    Ok(data) => data,
                    Err(error) => {
                        tracing::warn!(error = %error, seq = entry.seq, "Failed to serialize audit SSE event");
                        continue;
                    }
                };
                if tx.send(Ok(Event::default().data(data))).await.is_err() {
                    return; // Client disconnected
                }
            }

            // Update tracking state
            update_poll_cursor(
                &mut first_poll,
                &mut last_seq,
                entries.last().map(|e| e.seq),
            );
        }
    });

    // Watchdog: the forwarder handle was previously dropped, so a panic in
    // the poll loop left the EventSource hung with no server trace. Watch it
    // and log on panic (#5137). The watchdog ends naturally when the
    // forwarder returns (client disconnect) — no leak.
    tokio::spawn(async move {
        match forwarder.await {
            Ok(()) => {}
            Err(error) if error.is_panic() => {
                tracing::error!(
                    error = %error,
                    "SSE log forwarder task panicked; EventSource stalled"
                );
            }
            Err(error) => {
                tracing::warn!(
                    error = %error,
                    "SSE log forwarder task was cancelled"
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

/// Classify an audit entry into a level (error, warn, debug, info).
///
/// The outcome decides first, because it is the field that records whether the action went wrong: `AuditEntry.outcome` is "ok", "denied" or an error message, while the action name is the same string whether a `ToolInvoke` succeeded or failed (#8270).
/// An outcome leading with `error` / `fail` / `denied` is an error, one leading with `warn` is a warning, one leading with `debug` is debug.
/// Any other outcome falls back to the action, and only the actions that are failures by definition classify as errors that way — a refusal (`PermissionDenied`) or a spend cap being hit (`BudgetExceeded`) is exactly what an operator filtering for `error` is looking for.
/// `AgentKill` is deliberately not among them: a kill is an intentional operation, and its outcome says whether it failed.
///
/// The dashboard's `auditLogLevel` (`dashboard/src/pages/LogsPage.tsx`) implements the same rule, so the Logs page badge and this stream's `level` filter agree on every entry; the two test tables pin that.
fn classify_audit_level(action: &AuditAction, outcome: &str) -> &'static str {
    let outcome = outcome.trim_start();
    let leads_with = |prefix: &str| {
        outcome
            .as_bytes()
            .get(..prefix.len())
            .is_some_and(|head| head.eq_ignore_ascii_case(prefix.as_bytes()))
    };
    if leads_with("error") || leads_with("fail") || leads_with("denied") {
        return "error";
    }
    if leads_with("warn") {
        return "warn";
    }
    if leads_with("debug") {
        return "debug";
    }
    // Exhaustive on purpose: a new variant has to be placed on one side of this line before it compiles.
    match action {
        AuditAction::PermissionDenied | AuditAction::BudgetExceeded => "error",
        AuditAction::ToolInvoke
        | AuditAction::CapabilityCheck
        | AuditAction::AgentSpawn
        | AuditAction::AgentKill
        | AuditAction::AgentMessage
        | AuditAction::MemoryAccess
        | AuditAction::FileAccess
        | AuditAction::NetworkAccess
        | AuditAction::ShellExec
        | AuditAction::AuthAttempt
        | AuditAction::WireConnect
        | AuditAction::ConfigChange
        | AuditAction::DreamConsolidation
        | AuditAction::UserLogin
        | AuditAction::RoleChange
        | AuditAction::RetentionTrim
        | AuditAction::A2aDiscovered
        | AuditAction::A2aTrusted
        | AuditAction::ChainReanchored
        | AuditAction::McpConnect => "info",
    }
}

/// Test a pre-lowercased filter without allocating for the common ASCII case.
fn contains_lowercase_filter(haystack: &str, lowercase_filter: &str) -> bool {
    if lowercase_filter.is_empty() {
        return true;
    }
    if lowercase_filter.is_ascii() {
        haystack
            .as_bytes()
            .windows(lowercase_filter.len())
            .any(|window| window.eq_ignore_ascii_case(lowercase_filter.as_bytes()))
    } else {
        haystack.to_lowercase().contains(lowercase_filter)
    }
}

fn update_poll_cursor(first_poll: &mut bool, last_seq: &mut u64, newest_seq: Option<u64>) {
    if let Some(seq) = newest_seq {
        *last_seq = seq;
        *first_poll = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_initial_poll_keeps_bounded_backfill_mode() {
        let mut first_poll = true;
        let mut last_seq = 0;

        update_poll_cursor(&mut first_poll, &mut last_seq, None);

        assert!(first_poll);
        assert_eq!(last_seq, 0);
    }

    #[test]
    fn populated_initial_poll_establishes_cursor() {
        let mut first_poll = true;
        let mut last_seq = 0;

        update_poll_cursor(&mut first_poll, &mut last_seq, Some(42));

        assert!(!first_poll);
        assert_eq!(last_seq, 42);
    }

    #[tokio::test]
    async fn sender_detects_a_quiet_client_disconnect() {
        let (tx, rx) = tokio::sync::mpsc::channel::<SseEventResult>(1);
        drop(rx);

        tokio::time::timeout(std::time::Duration::from_millis(100), tx.closed())
            .await
            .expect("sender should observe a dropped SSE receiver");
    }

    /// Every `AuditAction` variant. Kept next to the classifier's exhaustive `match`, which fails to compile when a variant is added; add it here too so the exhaustive test below keeps covering the whole enum.
    const ALL_ACTIONS: [AuditAction; 22] = [
        AuditAction::ToolInvoke,
        AuditAction::CapabilityCheck,
        AuditAction::AgentSpawn,
        AuditAction::AgentKill,
        AuditAction::AgentMessage,
        AuditAction::MemoryAccess,
        AuditAction::FileAccess,
        AuditAction::NetworkAccess,
        AuditAction::ShellExec,
        AuditAction::AuthAttempt,
        AuditAction::WireConnect,
        AuditAction::ConfigChange,
        AuditAction::DreamConsolidation,
        AuditAction::UserLogin,
        AuditAction::RoleChange,
        AuditAction::PermissionDenied,
        AuditAction::BudgetExceeded,
        AuditAction::RetentionTrim,
        AuditAction::A2aDiscovered,
        AuditAction::A2aTrusted,
        AuditAction::ChainReanchored,
        AuditAction::McpConnect,
    ];

    /// The same table is pinned in the dashboard's `LogsPage.test.tsx` (`auditLogLevel`), so the SSE `level` filter and the Logs page badge give one answer per entry (#8270).
    #[test]
    fn audit_level_reads_the_outcome_first_with_an_action_fallback() {
        let cases: &[(AuditAction, &str, &str)] = &[
            (AuditAction::ToolInvoke, "ok", "info"),
            (AuditAction::ToolInvoke, "error: boom", "error"),
            (
                AuditAction::ToolInvoke,
                "  Error: padded and capitalised",
                "error",
            ),
            (AuditAction::ConfigChange, "failed", "error"),
            (AuditAction::DreamConsolidation, "fail", "error"),
            (AuditAction::DreamConsolidation, "aborted", "info"),
            (AuditAction::McpConnect, "error: connect failed", "error"),
            (AuditAction::PermissionDenied, "denied", "error"),
            (AuditAction::PermissionDenied, "ok", "error"),
            (AuditAction::BudgetExceeded, "ok", "error"),
            (AuditAction::BudgetExceeded, "", "error"),
            (AuditAction::CapabilityCheck, "denied", "error"),
            (AuditAction::AgentKill, "ok", "info"),
            (AuditAction::AgentKill, "error: agent not found", "error"),
            (AuditAction::ToolInvoke, "warning: nearing limit", "warn"),
            (AuditAction::ToolInvoke, "debug: cache miss", "debug"),
            (AuditAction::PermissionDenied, "warn: soft deny", "warn"),
            (AuditAction::AgentSpawn, "completed", "info"),
            (AuditAction::ToolInvoke, "", "info"),
        ];
        for (action, outcome, expected) in cases {
            assert_eq!(
                classify_audit_level(action, outcome),
                *expected,
                "action={action:?} outcome={outcome:?}"
            );
        }
    }

    #[test]
    fn only_failure_by_definition_actions_are_errors_when_the_outcome_is_ok() {
        let not_info: Vec<(String, &str)> = ALL_ACTIONS
            .iter()
            .map(|action| (format!("{action:?}"), classify_audit_level(action, "ok")))
            .filter(|(_, level)| *level != "info")
            .collect();
        assert_eq!(
            not_info,
            [
                ("PermissionDenied".to_string(), "error"),
                ("BudgetExceeded".to_string(), "error"),
            ]
        );
    }

    #[test]
    fn text_filter_avoids_allocation_for_ascii_and_supports_unicode() {
        assert!(contains_lowercase_filter("Agent FAILED safely", "fail"));
        assert!(!contains_lowercase_filter("Agent completed", "fail"));
        assert!(contains_lowercase_filter("Überprüfung", "über"));
    }
}
