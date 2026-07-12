//! Regression tests for the approvals hardening cluster (audit findings #18, #24).
//!
//! - #18: per-approval TOTP replay prevention must be atomic so a single
//!   valid single-use code cannot authorize more than one concurrent approval
//!   (TOCTOU between `is_totp_code_used` and `record_totp_code_used_for`).
//! - #24: `approve_request` must route the typed `KernelOpError` through the
//!   central status-code map (like `reject_request`), so a missing/expired id
//!   yields 404 rather than a blanket 400.
//!
//! Strategy mirrors `approvals_routes_integration.rs` / `totp_flow_test.rs`:
//! mount `routes::system::router()` under `/api` against a fresh mock kernel
//! and drive it through `tower::ServiceExt::oneshot`, which skips the global
//! auth gate and focuses each test on handler behavior.

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use axum::Router;
use librefang_api::routes::{self, AppState};
use librefang_testing::{MockKernelBuilder, TestAppState};
use librefang_types::approval::{ApprovalRequest, RiskLevel, SecondFactor};
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
    // Seed the pinned in-repo registry fixture so the kernel boots offline and
    // self-contained — without it `boot_with_config` needs a content registry
    // and fails unless `LIBREFANG_REGISTRY_OFFLINE=1` is set in the environment
    // (CI sets it; a bare local `cargo test` does not). Mirrors the
    // fixture-seeding in `audit_relations_acl_test` / `hooks_commands_routes_integration`.
    let test = TestAppState::with_builder(MockKernelBuilder::new().with_registry_fixture());
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
    app: &Router,
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
    let resp = app.clone().oneshot(req).await.unwrap();
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

async fn post(
    app: &Router,
    path: &str,
    body: serde_json::Value,
) -> (StatusCode, serde_json::Value) {
    json_request(app, Method::POST, path, Some(body)).await
}

/// Build a `TOTP` client that exactly mirrors `ApprovalManager::generate_totp_secret`
/// so `generate_current()` produces a code the kernel will accept.
fn totp_for(secret_base32: &str, issuer: &str) -> totp_rs::TOTP {
    use totp_rs::{Algorithm, Secret, TOTP};
    let raw = Secret::Encoded(secret_base32.to_string())
        .to_bytes()
        .expect("decode base32 secret");
    TOTP::new(
        Algorithm::SHA1,
        6,
        1,
        30,
        raw,
        Some(issuer.to_string()),
        String::new(),
    )
    .expect("totp init")
}

fn current_code(h: &Harness) -> String {
    let secret = h
        .state
        .kernel
        .vault_get("totp_secret")
        .expect("totp_secret in vault");
    let issuer = h.state.kernel.approvals().policy().totp_issuer.clone();
    totp_for(&secret, &issuer)
        .generate_current()
        .expect("generate current code")
}

fn make_request(agent: &str, tool: &str) -> ApprovalRequest {
    ApprovalRequest {
        id: Uuid::new_v4(),
        agent_id: agent.to_string(),
        tool_name: tool.to_string(),
        description: format!("test request for {tool}"),
        action_summary: format!("run {tool}"),
        risk_level: RiskLevel::High,
        requested_at: chrono::Utc::now(),
        timeout_secs: 300,
        sender_id: None,
        channel: None,
        chat_id: None,
        route_to: Vec::new(),
        escalation_count: 0,
        session_id: None,
        tool_use_id: None,
    }
}

/// Spawn `ApprovalManager::request_approval` in the background and wait until
/// the request lands in the pending map (hard cap so a bug can't deadlock).
async fn seed_pending(h: &Harness, req: ApprovalRequest) -> Uuid {
    let id = req.id;
    let kernel = Arc::clone(&h.state.kernel);
    tokio::spawn(async move {
        let _ = kernel.approvals().request_approval(req).await;
    });

    // Generous deadline: this test seeds N=24 approvals and runs an 8-worker
    // concurrent storm, so under a loaded runner a spawned `request_approval`
    // task can be scheduled later than the reference test's single seed. The
    // loop returns as soon as the request appears, so the cap only matters when
    // something is genuinely wrong.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while tokio::time::Instant::now() < deadline {
        if h.state.kernel.approvals().get_pending(id).is_some() {
            return id;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    panic!("seeded approval {id} did not appear in pending map within 10s");
}

// ---------------------------------------------------------------------------
// Finding #24 — approve routes typed KernelOpError through the status map
// ---------------------------------------------------------------------------

/// A well-formed but non-existent approval id must resolve to
/// `KernelOpError::AgentNotFound` and surface as 404 — matching
/// `reject_request`. Before the fix, `approve_request` collapsed every
/// resolve failure to 400, so this asserted 400 by mistake.
#[tokio::test(flavor = "multi_thread")]
async fn approve_missing_id_is_not_found() {
    let h = boot();
    let path = format!("/api/approvals/{}/approve", Uuid::new_v4());
    let (status, body) = post(&h.app, &path, serde_json::json!({})).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "got: {body}");
}

/// approve/reject must agree on the status code for an identical error
/// condition (missing id) — both 404. Pins the two arms against drift.
#[tokio::test(flavor = "multi_thread")]
async fn approve_and_reject_agree_on_missing_id_status() {
    let h = boot();
    let id = Uuid::new_v4();
    let (approve_status, _) = post(
        &h.app,
        &format!("/api/approvals/{id}/approve"),
        serde_json::json!({}),
    )
    .await;
    let (reject_status, _) = post(
        &h.app,
        &format!("/api/approvals/{id}/reject"),
        serde_json::json!({}),
    )
    .await;
    assert_eq!(approve_status, StatusCode::NOT_FOUND);
    assert_eq!(reject_status, StatusCode::NOT_FOUND);
    assert_eq!(approve_status, reject_status);
}

// ---------------------------------------------------------------------------
// Finding #18 — one single-use TOTP code approves at most one action
// ---------------------------------------------------------------------------

/// Fire many concurrent `approve` requests, each for a distinct pending
/// approval but all carrying the *same* single-use TOTP code. The atomic
/// check-and-record critical section must let exactly ONE succeed; every
/// other must be rejected as replay. Without the fix (non-atomic
/// check-then-record), multiple requests read `is_totp_code_used == false`
/// before any records the code and more than one approval succeeds.
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn concurrent_approvals_consume_totp_code_exactly_once() {
    let h = boot();

    // Enroll a TOTP secret (setup stores it in the vault; confirm is not
    // required — `approve_request` reads `totp_secret` directly).
    let (setup_status, _) = post(&h.app, "/api/approvals/totp/setup", serde_json::json!({})).await;
    assert_eq!(setup_status, StatusCode::OK);

    // Require TOTP for the tool under test.
    let mut policy = h.state.kernel.approvals().policy();
    policy.second_factor = SecondFactor::Totp;
    policy.totp_tools = vec!["shell_exec".to_string()];
    h.state.kernel.approvals().update_policy(policy);

    // Seed N distinct pending approvals for that TOTP-gated tool, each under a
    // DISTINCT agent id. `ApprovalManager::request_approval` enforces
    // `MAX_PENDING_PER_AGENT` (5), so seeding all N under one agent would be
    // denied past the cap and never land in the pending map. One approval per
    // agent stays well under the cap and does not affect the invariant under
    // test: the single-use TOTP code is tracked globally by code hash, not per
    // agent, so exactly one concurrent approve may still consume it.
    const N: usize = 24;
    let mut ids = Vec::with_capacity(N);
    for i in 0..N {
        ids.push(seed_pending(&h, make_request(&format!("agent-{i}"), "shell_exec")).await);
    }

    // One fresh, unused code shared by every concurrent request.
    let code = current_code(&h);

    // Fire all approvals concurrently on separate tasks so the synchronous
    // replay check-and-record regions can genuinely overlap across threads.
    let mut handles = Vec::with_capacity(N);
    for id in ids {
        let app = h.app.clone();
        let code = code.clone();
        handles.push(tokio::spawn(async move {
            let (status, _) = post(
                &app,
                &format!("/api/approvals/{id}/approve"),
                serde_json::json!({ "totp_code": code }),
            )
            .await;
            status
        }));
    }

    let mut successes = 0usize;
    for handle in handles {
        if handle.await.unwrap() == StatusCode::OK {
            successes += 1;
        }
    }

    assert_eq!(
        successes, 1,
        "exactly one concurrent approval may consume a single-use TOTP code, got {successes}"
    );
}
