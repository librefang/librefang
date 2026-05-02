//! Integration tests for the non-TOTP approval HTTP routes.
//!
//! Issue #3571 (slice): the approvals family had no integration tests for
//! anything other than the TOTP enrollment flow (already covered by
//! `totp_flow_test.rs`). This file fills in the rest:
//!
//!   - `GET  /api/approvals`                         — list (pending + recent)
//!   - `GET  /api/approvals/count`                   — pending badge counter
//!   - `GET  /api/approvals/{id}`                    — single request
//!   - `POST /api/approvals/{id}/approve`            — approve (no-TOTP path)
//!   - `POST /api/approvals/{id}/reject`             — reject
//!   - `POST /api/approvals/{id}/modify`             — modify-and-retry
//!   - `POST /api/approvals/batch`                   — batch resolve
//!   - `GET  /api/approvals/audit`                   — audit log query
//!   - `GET  /api/approvals/session/{id}`            — list per session
//!   - `POST /api/approvals/session/{id}/approve_all`
//!   - `POST /api/approvals/session/{id}/reject_all`
//!
//! Strategy mirrors `totp_flow_test.rs`: mount `routes::system::router()`
//! directly under `/api` against a fresh `TestAppState` / `MockKernelBuilder`
//! kernel and drive it through `tower::ServiceExt::oneshot`. Mounting the
//! domain router (rather than the full `server::build_router`) keeps the
//! tests focused on handler behavior and avoids the global auth gate.
//!
//! Pending approvals are seeded by spawning the kernel's
//! `ApprovalManager::request_approval` from the test (the production wire-up
//! that `POST /api/approvals` uses internally), then waiting for the request
//! to land in the in-memory map before driving the HTTP surface.

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use axum::Router;
use librefang_api::routes::{self, AppState};
use librefang_testing::{MockKernelBuilder, TestAppState};
use librefang_types::approval::{ApprovalRequest, RiskLevel};
use std::sync::Arc;
use std::time::Duration;
use tower::ServiceExt;
use uuid::Uuid;

struct Harness {
    app: Router,
    state: Arc<AppState>,
    _test: TestAppState,
}

fn boot() -> Harness {
    let test = TestAppState::with_builder(MockKernelBuilder::new());
    let state = test.state.clone();
    let app = Router::new()
        .nest("/api", routes::system::router())
        .with_state(state.clone());
    Harness {
        app,
        state,
        _test: test,
    }
}

async fn json_request(
    h: &Harness,
    method: Method,
    path: &str,
    body: Option<serde_json::Value>,
) -> (StatusCode, serde_json::Value) {
    let mut builder = Request::builder().method(method).uri(path);
    let body_bytes = match body {
        Some(v) => {
            builder = builder.header("content-type", "application/json");
            serde_json::to_vec(&v).unwrap()
        }
        None => Vec::new(),
    };
    let req = builder.body(Body::from(body_bytes)).unwrap();
    let resp = h.app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), 1 << 20)
        .await
        .unwrap();
    let value: serde_json::Value = if bytes.is_empty() {
        serde_json::Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null)
    };
    (status, value)
}

async fn get(h: &Harness, path: &str) -> (StatusCode, serde_json::Value) {
    json_request(h, Method::GET, path, None).await
}

async fn post(h: &Harness, path: &str, body: serde_json::Value) -> (StatusCode, serde_json::Value) {
    json_request(h, Method::POST, path, Some(body)).await
}

/// Build a synthetic `ApprovalRequest` with a long timeout so it stays pending
/// across the test's lifetime. `tool_name` must NOT be in `policy.totp_tools`
/// — the default policy has an empty list, so any plausible name is fine.
fn make_request(agent: &str, tool: &str, session_id: Option<&str>) -> ApprovalRequest {
    ApprovalRequest {
        id: Uuid::new_v4(),
        agent_id: agent.to_string(),
        tool_name: tool.to_string(),
        description: format!("test request for {tool}"),
        action_summary: format!("run {tool}"),
        risk_level: RiskLevel::High,
        requested_at: chrono::Utc::now(),
        // Wide timeout so the request doesn't auto-resolve mid-test. The
        // type clamps to MAX_TIMEOUT_SECS so anything past that is fine.
        timeout_secs: 300,
        sender_id: None,
        channel: None,
        route_to: Vec::new(),
        escalation_count: 0,
        session_id: session_id.map(str::to_string),
    }
}

/// Spawn `ApprovalManager::request_approval` in the background and wait
/// until the request lands in the pending map (with a hard cap so a bug in
/// insertion can't deadlock the suite).
async fn seed_pending(h: &Harness, req: ApprovalRequest) -> Uuid {
    let id = req.id;
    let kernel = Arc::clone(&h.state.kernel);
    tokio::spawn(async move {
        // The future resolves when the approval is decided or times out.
        // We don't observe the result here — the caller drives resolution
        // through the HTTP API and asserts on the side effects.
        let _ = kernel.approvals().request_approval(req).await;
    });

    // Poll briefly for the entry to appear. 1s is generous; insertion is
    // synchronous after the spawned task is scheduled.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(1);
    while tokio::time::Instant::now() < deadline {
        if h.state.kernel.approvals().get_pending(id).is_some() {
            return id;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    panic!("seeded approval {id} did not appear in pending map within 1s");
}

// ---------------------------------------------------------------------------
// GET /api/approvals — list & pagination
// ---------------------------------------------------------------------------

/// Empty kernel: list returns the canonical paginated envelope with zero items.
/// Frontend depends on the `total / offset / limit` triple existing even when
/// the buffer is empty (otherwise the dashboard's pagination math NaNs out).
#[tokio::test(flavor = "multi_thread")]
async fn list_empty_returns_paginated_envelope() {
    let h = boot();
    let (status, body) = get(&h, "/api/approvals").await;
    assert_eq!(status, StatusCode::OK, "got: {body}");
    assert_eq!(body["approvals"].as_array().unwrap().len(), 0);
    assert_eq!(body["total"], 0);
    assert_eq!(body["offset"], 0);
    assert!(body["limit"].as_u64().unwrap() > 0, "limit must be > 0");
}

/// A seeded pending request must show up with `status=pending` and the
/// dashboard-shaped aliases (`action`, `created_at`, `agent_name`).
#[tokio::test(flavor = "multi_thread")]
async fn list_includes_pending_with_dashboard_aliases() {
    let h = boot();
    let id = seed_pending(&h, make_request("agent-a", "shell_exec", None)).await;

    let (status, body) = get(&h, "/api/approvals").await;
    assert_eq!(status, StatusCode::OK);
    let items = body["approvals"].as_array().unwrap();
    assert_eq!(items.len(), 1, "exactly one pending request expected");
    let item = &items[0];
    assert_eq!(item["id"], id.to_string());
    assert_eq!(item["status"], "pending");
    assert_eq!(item["tool_name"], "shell_exec");
    // Dashboard aliases — the SPA reads these names, not the canonical ones.
    assert_eq!(item["action"], item["action_summary"]);
    assert_eq!(item["created_at"], item["requested_at"]);
    assert!(item["agent_name"].is_string(), "agent_name alias missing");
}

// ---------------------------------------------------------------------------
// GET /api/approvals/count
// ---------------------------------------------------------------------------

/// Count tracks pending insertions exactly. The dashboard nav badge polls this
/// endpoint and renders the integer verbatim — a wrong type or off-by-one
/// breaks the badge.
#[tokio::test(flavor = "multi_thread")]
async fn count_reflects_pending_total() {
    let h = boot();
    let (_, before) = get(&h, "/api/approvals/count").await;
    assert_eq!(before["pending"], 0);

    seed_pending(&h, make_request("a1", "shell_exec", None)).await;
    seed_pending(&h, make_request("a1", "fs_write", None)).await;

    let (status, after) = get(&h, "/api/approvals/count").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(after["pending"], 2);
}

// ---------------------------------------------------------------------------
// GET /api/approvals/{id}
// ---------------------------------------------------------------------------

/// A non-UUID `{id}` must be rejected with 400, not panicked through to a
/// 500. The handler parses the path with `Uuid::parse_str` before looking it
/// up, so the malformed-id branch is the boundary check this test pins.
#[tokio::test(flavor = "multi_thread")]
async fn get_approval_invalid_uuid_is_bad_request() {
    let h = boot();
    let (status, body) = get(&h, "/api/approvals/not-a-uuid").await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "got: {body}");
    assert!(body["error"].is_string());
}

/// A well-formed UUID that doesn't exist is 404, distinct from 400 — the
/// dashboard distinguishes "request was already resolved" (404) from "client
/// sent garbage" (400) and surfaces different toasts for each.
#[tokio::test(flavor = "multi_thread")]
async fn get_approval_missing_uuid_is_not_found() {
    let h = boot();
    let path = format!("/api/approvals/{}", Uuid::new_v4());
    let (status, body) = get(&h, &path).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "got: {body}");
}

/// Happy path: seeded request is fetchable and serializes the dashboard shape.
#[tokio::test(flavor = "multi_thread")]
async fn get_approval_returns_seeded_request() {
    let h = boot();
    let id = seed_pending(&h, make_request("agent-x", "shell_exec", None)).await;

    let (status, body) = get(&h, &format!("/api/approvals/{id}")).await;
    assert_eq!(status, StatusCode::OK, "got: {body}");
    assert_eq!(body["id"], id.to_string());
    assert_eq!(body["tool_name"], "shell_exec");
    assert_eq!(body["status"], "pending");
}

// ---------------------------------------------------------------------------
// POST /api/approvals/{id}/approve & /reject & /modify
// ---------------------------------------------------------------------------

/// Approve removes the request from `pending` and pushes it onto `recent`
/// with `status=approved`. We assert via the list endpoint so we exercise
/// the full read-after-write contract the dashboard depends on.
#[tokio::test(flavor = "multi_thread")]
async fn approve_resolves_pending_to_approved() {
    let h = boot();
    let id = seed_pending(&h, make_request("a", "shell_exec", None)).await;

    let (status, body) = post(
        &h,
        &format!("/api/approvals/{id}/approve"),
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "got: {body}");
    assert_eq!(body["status"], "approved");

    // Pending count must have dropped to zero, and the list now shows it as
    // resolved (not pending).
    let (_, count) = get(&h, "/api/approvals/count").await;
    assert_eq!(count["pending"], 0);
    let (_, list) = get(&h, "/api/approvals").await;
    let item = &list["approvals"].as_array().unwrap()[0];
    assert_eq!(item["id"], id.to_string());
    assert_eq!(item["status"], "approved");
}

/// Approve with an invalid UUID path segment is 400 (not 404, not 500).
#[tokio::test(flavor = "multi_thread")]
async fn approve_invalid_uuid_is_bad_request() {
    let h = boot();
    let (status, _) = post(&h, "/api/approvals/junk/approve", serde_json::json!({})).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

/// Reject moves a pending request into `recent` with `status=rejected`.
#[tokio::test(flavor = "multi_thread")]
async fn reject_resolves_pending_to_rejected() {
    let h = boot();
    let id = seed_pending(&h, make_request("a", "shell_exec", None)).await;

    let (status, body) = post(
        &h,
        &format!("/api/approvals/{id}/reject"),
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "got: {body}");
    assert_eq!(body["status"], "rejected");

    let (_, list) = get(&h, "/api/approvals").await;
    let item = &list["approvals"].as_array().unwrap()[0];
    assert_eq!(item["status"], "rejected");
}

/// Reject of an unknown UUID is 404, not 500.
#[tokio::test(flavor = "multi_thread")]
async fn reject_missing_id_is_not_found() {
    let h = boot();
    let id = Uuid::new_v4();
    let (status, _) = post(
        &h,
        &format!("/api/approvals/{id}/reject"),
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

/// Modify-and-retry transitions to `modify_and_retry` in the recent buffer.
#[tokio::test(flavor = "multi_thread")]
async fn modify_resolves_pending_with_feedback() {
    let h = boot();
    let id = seed_pending(&h, make_request("a", "shell_exec", None)).await;

    let (status, body) = post(
        &h,
        &format!("/api/approvals/{id}/modify"),
        serde_json::json!({ "feedback": "use --dry-run instead" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "got: {body}");
    assert_eq!(body["status"], "modified");

    let (_, list) = get(&h, "/api/approvals").await;
    let item = &list["approvals"].as_array().unwrap()[0];
    assert_eq!(item["status"], "modify_and_retry");
}

// ---------------------------------------------------------------------------
// POST /api/approvals/batch
// ---------------------------------------------------------------------------

/// Batch approve resolves every listed pending UUID and reports per-id status.
#[tokio::test(flavor = "multi_thread")]
async fn batch_approve_resolves_all() {
    let h = boot();
    let id1 = seed_pending(&h, make_request("a", "shell_exec", None)).await;
    let id2 = seed_pending(&h, make_request("a", "fs_write", None)).await;

    let (status, body) = post(
        &h,
        "/api/approvals/batch",
        serde_json::json!({
            "ids": [id1.to_string(), id2.to_string()],
            "decision": "approve",
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "got: {body}");
    let results = body["results"].as_array().unwrap();
    assert_eq!(results.len(), 2);
    for r in results {
        assert_eq!(r["status"], "ok");
        assert_eq!(r["decision"], "approved");
    }

    let (_, count) = get(&h, "/api/approvals/count").await;
    assert_eq!(count["pending"], 0);
}

/// An unknown decision string is rejected with 400 — the handler must not
/// silently default to approve/reject on garbage input.
#[tokio::test(flavor = "multi_thread")]
async fn batch_invalid_decision_is_bad_request() {
    let h = boot();
    let (status, body) = post(
        &h,
        "/api/approvals/batch",
        serde_json::json!({ "ids": [], "decision": "yolo" }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "got: {body}");
}

/// Invalid UUIDs in the batch are reported per-item, not as a global 400 —
/// the dashboard surfaces these inline next to each row.
#[tokio::test(flavor = "multi_thread")]
async fn batch_reports_invalid_uuid_per_item() {
    let h = boot();
    let (status, body) = post(
        &h,
        "/api/approvals/batch",
        serde_json::json!({
            "ids": ["not-a-uuid"],
            "decision": "approve",
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "got: {body}");
    let results = body["results"].as_array().unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0]["status"], "error");
    assert!(results[0]["message"]
        .as_str()
        .unwrap()
        .contains("invalid UUID"));
}

/// Batch over the documented 100-item cap is rejected up front so a hostile
/// or buggy client can't ask the kernel to resolve thousands of UUIDs in one
/// request.
#[tokio::test(flavor = "multi_thread")]
async fn batch_oversize_is_bad_request() {
    let h = boot();
    let ids: Vec<String> = (0..101).map(|_| Uuid::new_v4().to_string()).collect();
    let (status, body) = post(
        &h,
        "/api/approvals/batch",
        serde_json::json!({ "ids": ids, "decision": "approve" }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "got: {body}");
    assert!(body["error"]
        .as_str()
        .unwrap_or_default()
        .contains("batch size"));
}

// ---------------------------------------------------------------------------
// GET /api/approvals/audit
// ---------------------------------------------------------------------------

/// Audit endpoint returns the canonical `PaginatedResponse{items,total,offset,limit}`
/// envelope (#3842) even when nothing has been resolved yet.
#[tokio::test(flavor = "multi_thread")]
async fn audit_empty_returns_envelope() {
    let h = boot();
    let (status, body) = get(&h, "/api/approvals/audit").await;
    assert_eq!(status, StatusCode::OK, "got: {body}");
    assert!(body["items"].is_array());
    assert!(body["total"].is_number());
    assert!(body["offset"].is_number());
    assert!(body["limit"].is_number());
}

// ---------------------------------------------------------------------------
// Per-session list / approve_all / reject_all
// ---------------------------------------------------------------------------

/// Session list with a brand-new session id returns an empty payload that
/// still includes the count and `has_pending=false` flags the dashboard
/// keys off.
#[tokio::test(flavor = "multi_thread")]
async fn list_for_session_empty_session_returns_empty_envelope() {
    let h = boot();
    let (status, body) = get(&h, "/api/approvals/session/sess-empty").await;
    assert_eq!(status, StatusCode::OK, "got: {body}");
    assert_eq!(body["session_id"], "sess-empty");
    assert_eq!(body["count"], 0);
    assert_eq!(body["has_pending"], false);
    assert_eq!(body["pending"].as_array().unwrap().len(), 0);
}

/// A whitespace-only session_id is 400 — the path validator must guard
/// against `/api/approvals/session/%20`-style probes.
#[tokio::test(flavor = "multi_thread")]
async fn list_for_session_whitespace_id_is_bad_request() {
    let h = boot();
    let (status, _) = get(&h, "/api/approvals/session/%20%20").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

/// `approve_all` for a session resolves every matching pending request and
/// reports the count; `count` afterwards reflects the drop.
#[tokio::test(flavor = "multi_thread")]
async fn approve_all_for_session_resolves_only_matching_session() {
    let h = boot();
    seed_pending(&h, make_request("a", "shell_exec", Some("sess-A"))).await;
    seed_pending(&h, make_request("a", "fs_write", Some("sess-A"))).await;
    // A request on a different session must NOT be resolved by approve_all
    // for sess-A.
    seed_pending(&h, make_request("a", "shell_exec", Some("sess-B"))).await;

    let (status, body) = post(
        &h,
        "/api/approvals/session/sess-A/approve_all",
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "got: {body}");
    assert_eq!(body["session_id"], "sess-A");
    assert_eq!(body["resolved"], 2);
    assert_eq!(body["decision"], "approved");

    // Only the sess-B request should remain pending.
    let (_, count) = get(&h, "/api/approvals/count").await;
    assert_eq!(count["pending"], 1);
}

/// `approve_all` with an `expected_count` that no longer matches the actual
/// pending set returns 409 — the optimistic-concurrency guard the SPA uses
/// to avoid silently approving a fresh high-risk request that landed between
/// "operator viewed list" and "operator clicked approve".
#[tokio::test(flavor = "multi_thread")]
async fn approve_all_for_session_expected_count_mismatch_is_conflict() {
    let h = boot();
    seed_pending(&h, make_request("a", "shell_exec", Some("sess-X"))).await;

    let (status, body) = post(
        &h,
        "/api/approvals/session/sess-X/approve_all",
        serde_json::json!({ "expected_count": 99 }),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "got: {body}");
    assert_eq!(body["pending_count"], 1);
    assert_eq!(body["expected_count"], 99);
}

/// `reject_all` mirrors `approve_all`: drops every pending entry on the
/// session and reports the resolved count.
#[tokio::test(flavor = "multi_thread")]
async fn reject_all_for_session_resolves_pending() {
    let h = boot();
    seed_pending(&h, make_request("a", "shell_exec", Some("sess-R"))).await;
    seed_pending(&h, make_request("a", "fs_write", Some("sess-R"))).await;

    let (status, body) = post(
        &h,
        "/api/approvals/session/sess-R/reject_all",
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "got: {body}");
    assert_eq!(body["resolved"], 2);
    assert_eq!(body["decision"], "rejected");

    let (_, count) = get(&h, "/api/approvals/count").await;
    assert_eq!(count["pending"], 0);
}
