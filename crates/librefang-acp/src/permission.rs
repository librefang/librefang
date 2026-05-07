//! Bridge from LibreFang `ApprovalRequest` to ACP `session/request_permission`.
//!
//! When a tool needs approval, the kernel fires
//! [`librefang_types::approval::ApprovalEvent::Created`] on the broadcast
//! channel exposed by `ApprovalManager::subscribe()`. This module subscribes,
//! filters by the LibreFang `SessionId` we tracked when our ACP `session/new`
//! ran, and translates each match into a `session/request_permission`
//! request the editor can render in its native modal UI.
//!
//! When the editor user picks an option (or 60s elapses), we feed the
//! decision back via [`AcpKernel::resolve_approval`] so the kernel's
//! [`ApprovalGate`](librefang_types::approval) policy + audit
//! pipeline runs identically to dashboard / TUI / channel approvals.

use std::sync::Arc;
use std::time::Duration;

use agent_client_protocol::schema::{
    PermissionOption, PermissionOptionKind, RequestPermissionOutcome, RequestPermissionRequest,
    RequestPermissionResponse, ToolCallId, ToolCallUpdate, ToolCallUpdateFields,
};
use agent_client_protocol::Client;
use agent_client_protocol::ConnectionTo;
use librefang_types::agent::SessionId as LfSessionId;
use librefang_types::approval::{ApprovalDecision, ApprovalEvent, ApprovalRequest};
use tokio::sync::broadcast;
use tracing::{debug, warn};
use uuid::Uuid;

use crate::events::infer_tool_kind;
use crate::session::SessionStore;
use crate::{AcpError, AcpKernel};

/// 60-second client decision timeout. Mirrors hermes-agent's default
/// (see `acp_adapter/permissions.py`) — long enough for a human to
/// read and click, short enough that a hung editor doesn't pin a
/// pending approval indefinitely.
const PERMISSION_TIMEOUT: Duration = Duration::from_secs(60);

/// Run forever (until the connection closes), forwarding kernel
/// `ApprovalEvent::Created` events into `session/request_permission`
/// requests for the matching ACP session.
pub(crate) async fn run_bridge<K: AcpKernel>(
    kernel: Arc<K>,
    sessions: Arc<SessionStore>,
    cx: ConnectionTo<Client>,
) -> Result<(), agent_client_protocol::Error> {
    let mut rx = kernel.subscribe_approvals();
    debug!("ACP permission bridge: subscribed to approval events");
    loop {
        match rx.recv().await {
            Ok(ApprovalEvent::Created(approval)) => {
                if let Err(e) = dispatch_pending(&kernel, &sessions, &cx, approval).await {
                    warn!(error = %e, "ACP permission bridge: dispatch_pending failed");
                }
            }
            // `Resolved` events are emitted as a courtesy to other
            // subscribers (dashboards / TUI). The ACP side has nothing to
            // do — the resolution either came from us (already handled)
            // or from another surface, in which case the editor just
            // never gets to pick.
            Ok(_) => {}
            Err(broadcast::error::RecvError::Lagged(n)) => {
                // Slow consumer. Re-sync via list_pending isn't strictly
                // required because every prompt re-fires Created on tool
                // approval anyway; just log and keep going.
                warn!(
                    skipped = n,
                    "ACP permission bridge: lagged behind broadcast"
                );
            }
            Err(broadcast::error::RecvError::Closed) => {
                debug!("ACP permission bridge: kernel broadcast closed, exiting");
                break;
            }
        }
    }
    Ok(())
}

async fn dispatch_pending<K: AcpKernel>(
    kernel: &Arc<K>,
    sessions: &Arc<SessionStore>,
    cx: &ConnectionTo<Client>,
    approval: ApprovalRequest,
) -> Result<(), AcpError> {
    // Skip approvals not tagged with a session_id — they originated from
    // a non-ACP surface (e.g. a workflow trigger) and have no place to
    // surface in the editor.
    let Some(lf_id_str) = approval.session_id.as_deref() else {
        return Ok(());
    };
    let Ok(lf_uuid) = Uuid::parse_str(lf_id_str) else {
        return Ok(());
    };
    let lf_id = LfSessionId(lf_uuid);

    // Map the LibreFang session id back to its ACP counterpart. If we
    // don't have one, the approval is for a session we don't own —
    // another surface (or a parallel ACP server) will handle it.
    let Some(acp_id) = sessions.find_by_librefang_id(&lf_id) else {
        return Ok(());
    };

    let req_id = approval.id;
    let title = if approval.action_summary.is_empty() {
        approval.tool_name.clone()
    } else {
        format!("{}: {}", approval.tool_name, approval.action_summary)
    };
    let tool_call = ToolCallUpdate::new(
        ToolCallId::new(req_id.to_string()),
        ToolCallUpdateFields::new()
            .title(title)
            .kind(infer_tool_kind(&approval.tool_name)),
    );
    let options = vec![
        PermissionOption::new("allow_once", "Allow once", PermissionOptionKind::AllowOnce),
        PermissionOption::new(
            "allow_always",
            "Allow always",
            PermissionOptionKind::AllowAlways,
        ),
        PermissionOption::new("reject_once", "Deny", PermissionOptionKind::RejectOnce),
        PermissionOption::new(
            "reject_always",
            "Deny always",
            PermissionOptionKind::RejectAlways,
        ),
    ];

    let perm_req = RequestPermissionRequest::new(acp_id, tool_call, options);
    let sent = cx.send_request(perm_req);

    // Forward the response onto a oneshot we can race against the 60s
    // timeout. The closure registered with `on_receiving_result` runs
    // on the connection's task; sending into a oneshot is cheap.
    let (tx, rx) = tokio::sync::oneshot::channel::<
        Result<RequestPermissionResponse, agent_client_protocol::Error>,
    >();
    sent.on_receiving_result(async move |result| {
        let _ = tx.send(result);
        Ok(())
    })
    .map_err(AcpError::Transport)?;

    // Serialize approvals — wait inline before processing the next
    // broadcast event. Phase 1 prefers correctness over throughput;
    // pending approvals queue in the broadcast (capacity 256). The
    // serial path keeps tests deterministic and avoids spawning
    // detached tasks whose runtime context (LocalSet vs not) can
    // diverge between production and test harnesses.
    let (decision, remember) = match tokio::time::timeout(PERMISSION_TIMEOUT, rx).await {
        Ok(Ok(Ok(resp))) => decision_from_outcome(resp.outcome),
        Ok(Ok(Err(e))) => {
            warn!(error = %e, request_id = %req_id, "ACP request_permission transport error");
            (ApprovalDecision::Denied, false)
        }
        Ok(Err(_recv_err)) => {
            warn!(request_id = %req_id, "ACP request_permission: response channel dropped");
            (ApprovalDecision::Denied, false)
        }
        Err(_elapsed) => {
            debug!(request_id = %req_id, "ACP request_permission timed out, denying");
            (ApprovalDecision::Denied, false)
        }
    };

    // Persist "always" choices so future tool requests for the same
    // (agent_id, tool_name) skip the editor entirely. Done before the
    // resolve so the cache is populated by the time any concurrent
    // tool call queries `requires_approval_with_context_for`.
    if remember {
        if let Err(e) = kernel
            .remember_decision(&approval.agent_id, &approval.tool_name, decision.clone())
            .await
        {
            warn!(error = %e, request_id = %req_id,
                  "ACP permission bridge: remember_decision failed");
        }
    }

    if let Err(e) = kernel
        .resolve_approval(req_id, decision, Some("acp".into()))
        .await
    {
        warn!(error = %e, request_id = %req_id, "ACP permission bridge: resolve_approval failed");
    }

    Ok(())
}

/// Translate ACP's [`RequestPermissionOutcome`] into LibreFang's
/// [`ApprovalDecision`] plus a `remember_always` flag (#3313).
///
/// * `allow_once` / `reject_once` / `Cancelled` → `(decision, false)` —
///   one-shot, no persistence.
/// * `allow_always` / `reject_always` → `(decision, true)` — caller
///   should also persist via `AcpKernel::remember_decision` so future
///   `(agent_id, tool_name)` calls short-circuit.
fn decision_from_outcome(outcome: RequestPermissionOutcome) -> (ApprovalDecision, bool) {
    match outcome {
        RequestPermissionOutcome::Selected(selected) => {
            let id: &str = &selected.option_id.0;
            let approved = id.starts_with("allow");
            let remember = id.ends_with("_always");
            let decision = if approved {
                ApprovalDecision::Approved
            } else {
                ApprovalDecision::Denied
            };
            (decision, remember)
        }
        // Cancellation = client wants to abort this turn; deny so the
        // tool execution path bails out cleanly. Don't remember.
        RequestPermissionOutcome::Cancelled => (ApprovalDecision::Denied, false),
        // ACP marks the outcome enum `#[non_exhaustive]`; any future
        // variant defaults to deny without remembering for safety.
        _ => (ApprovalDecision::Denied, false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_client_protocol::schema::{PermissionOptionId, SelectedPermissionOutcome};

    fn outcome(id: &'static str) -> RequestPermissionOutcome {
        RequestPermissionOutcome::Selected(SelectedPermissionOutcome::new(PermissionOptionId::new(
            id,
        )))
    }

    #[test]
    fn allow_once_is_approved_no_remember() {
        assert_eq!(
            decision_from_outcome(outcome("allow_once")),
            (ApprovalDecision::Approved, false)
        );
    }

    #[test]
    fn allow_always_is_approved_with_remember() {
        assert_eq!(
            decision_from_outcome(outcome("allow_always")),
            (ApprovalDecision::Approved, true)
        );
    }

    #[test]
    fn reject_once_is_denied_no_remember() {
        assert_eq!(
            decision_from_outcome(outcome("reject_once")),
            (ApprovalDecision::Denied, false)
        );
    }

    #[test]
    fn reject_always_is_denied_with_remember() {
        assert_eq!(
            decision_from_outcome(outcome("reject_always")),
            (ApprovalDecision::Denied, true)
        );
    }

    #[test]
    fn cancelled_is_denied_no_remember() {
        assert_eq!(
            decision_from_outcome(RequestPermissionOutcome::Cancelled),
            (ApprovalDecision::Denied, false)
        );
    }

    #[test]
    fn unknown_id_is_denied_no_remember() {
        assert_eq!(
            decision_from_outcome(outcome("frobnicate")),
            (ApprovalDecision::Denied, false)
        );
    }
}
