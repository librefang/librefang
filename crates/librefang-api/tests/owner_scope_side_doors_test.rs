//! Owner-scoping and Admin-gating regressions for read / decide surfaces that bypassed the #6753 agent-ownership rule and the `/api/audit/*` Admin gate.
//!
//! Every request runs through the real domain routers and the real auth middleware, so `AuthenticatedApiUser` extraction is exercised end to end:
//!
//! - `GET /api/logs/stream` streams raw audit ledger rows and must be Admin-only, like `/api/audit/*`.
//! - `GET/POST /api/approvals*` must only show and decide a restricted caller's own agents' approvals, and attribute the decision to the caller.
//! - `GET /api/dashboard/snapshot` must owner-scope its `agents` array like `GET /api/agents`, without leaking one caller's view through the memoization cache.
//! - `GET /api/budget/agents[/{id}]` and `GET /api/usage` must not expose another user's per-agent spend.

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use axum::Router;
use librefang_api::middleware;
use librefang_api::routes::{self, AppState};
use librefang_kernel::auth::UserRole as KernelUserRole;
use librefang_testing::{MockKernelBuilder, TestAppState};
use librefang_types::agent::{AgentId, AgentManifest, UserId};
use librefang_types::approval::{ApprovalRequest, RiskLevel};
use librefang_types::config::UserConfig;
use std::sync::Arc;
use tower::ServiceExt;

const ALICE_KEY: &str = "alice-side-door-key";
const BOB_KEY: &str = "bob-side-door-key";
const VIEWER_KEY: &str = "viewer-side-door-key";
const ADMIN_KEY: &str = "admin-side-door-key";

struct Harness {
    app: Router,
    state: Arc<AppState>,
    _tmp: tempfile::TempDir,
}

impl Drop for Harness {
    fn drop(&mut self) {
        self.state.kernel.shutdown();
    }
}

async fn boot() -> Harness {
    let users = [
        ("Alice", "user", ALICE_KEY),
        ("Bob", "user", BOB_KEY),
        ("Eve", "viewer", VIEWER_KEY),
        ("Admin", "admin", ADMIN_KEY),
    ];
    let mut configs = Vec::new();
    let mut auth_users = Vec::new();
    for (name, role, key) in users {
        let hash = librefang_api::password_hash::hash_password(key).expect("hash test key");
        configs.push(UserConfig {
            name: name.to_string(),
            role: role.to_string(),
            api_key_hash: Some(hash.clone()),
            ..Default::default()
        });
        auth_users.push(middleware::ApiUserAuth {
            name: name.to_string(),
            role: KernelUserRole::from_str_role(role),
            api_key_hash: hash,
            user_id: UserId::from_name(name),
        });
    }

    let test = TestAppState::with_builder(MockKernelBuilder::new().with_config(move |cfg| {
        cfg.api_key = "side-door-master-key".to_string();
        cfg.users = configs;
    }))
    .with_api_key("side-door-master-key")
    .with_user_api_keys(auth_users);
    let (state, tmp, _) = test.into_parts();
    state.kernel.clone().set_self_handle();

    let auth_state = middleware::AuthState {
        api_key_lock: state.api_key_lock.clone(),
        master_key: state.master_key.clone(),
        active_sessions: state.active_sessions.clone(),
        dashboard_auth_enabled: state.dashboard_auth_enabled.clone(),
        user_api_keys: state.user_api_keys.clone(),
        // Production's `derive_require_auth_for_reads` turns this on whenever `[[users]]` are configured; without it the dashboard-read-public budget routes would never populate `AuthenticatedApiUser` and the ownership assertions would pass vacuously.
        require_auth_for_reads: true,
        allow_no_auth: false,
        audit_log: Some(state.kernel.audit().clone()),
    };
    let app = Router::new()
        .nest("/api", routes::system::router())
        .nest("/api", routes::config::router())
        .nest("/api", routes::budget::router())
        .layer(axum::middleware::from_fn_with_state(
            auth_state,
            middleware::auth,
        ))
        .with_state(state.clone());

    Harness {
        app,
        state,
        _tmp: tmp,
    }
}

fn spawn_authored(state: &AppState, author: &str) -> AgentId {
    state
        .kernel
        .spawn_agent_typed(AgentManifest {
            name: format!("side-door-{}", uuid::Uuid::new_v4()),
            source_template: None,
            author: author.to_string(),
            ..AgentManifest::default()
        })
        .expect("spawn authored test agent")
}

fn submit_approval(state: &AppState, agent_id: AgentId, session_id: Option<&str>) -> uuid::Uuid {
    let id = uuid::Uuid::new_v4();
    state
        .kernel
        .approvals()
        .submit_manual_request(ApprovalRequest {
            id,
            agent_id: agent_id.to_string(),
            tool_name: "shell_exec".to_string(),
            description: "run command".to_string(),
            action_summary: "rm -rf ~/data".to_string(),
            risk_level: RiskLevel::High,
            requested_at: chrono::Utc::now(),
            timeout_secs: 600,
            sender_id: None,
            channel: None,
            chat_id: None,
            route_to: Vec::new(),
            escalation_count: 0,
            session_id: session_id.map(str::to_string),
            tool_use_id: None,
        })
        .expect("submit manual approval")
}

async fn send(
    h: &Harness,
    method: Method,
    path: &str,
    bearer: &str,
    body: Option<serde_json::Value>,
) -> (StatusCode, Option<String>, serde_json::Value) {
    let mut builder = Request::builder()
        .method(method)
        .uri(path)
        .header("authorization", format!("Bearer {bearer}"));
    let body = match body {
        Some(value) => {
            builder = builder.header("content-type", "application/json");
            Body::from(serde_json::to_vec(&value).expect("serialize request"))
        }
        None => Body::empty(),
    };
    let resp = h
        .app
        .clone()
        .oneshot(builder.body(body).expect("build request"))
        .await
        .expect("route response");
    let status = resp.status();
    let content_type = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    if content_type
        .as_deref()
        .is_some_and(|ct| ct.starts_with("text/event-stream"))
    {
        // The SSE body never ends; the headers are all the test needs.
        return (status, content_type, serde_json::Value::Null);
    }
    let bytes = axum::body::to_bytes(resp.into_body(), 4 << 20)
        .await
        .expect("read body");
    let value = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
    (status, content_type, value)
}

fn contains_id(items: &serde_json::Value, key: &str, id: &str) -> bool {
    items
        .as_array()
        .expect("array payload")
        .iter()
        .any(|item| item[key].as_str() == Some(id))
}

#[tokio::test(flavor = "multi_thread")]
async fn logs_stream_is_admin_only_like_audit_routes() {
    let h = boot().await;

    for key in [VIEWER_KEY, BOB_KEY] {
        let (status, _, _) = send(&h, Method::GET, "/api/logs/stream", key, None).await;
        assert_eq!(
            status,
            StatusCode::FORBIDDEN,
            "non-admin key must not stream the audit ledger"
        );
    }
    let denied = h.state.kernel.audit().recent(20);
    assert!(
        denied.iter().any(|entry| {
            matches!(
                entry.action,
                librefang_kernel::audit::AuditAction::PermissionDenied
            ) && entry.detail.contains("role viewer")
        }),
        "the denial must be recorded in the audit log with the caller's role"
    );

    let (status, content_type, _) =
        send(&h, Method::GET, "/api/logs/stream", ADMIN_KEY, None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(content_type
        .as_deref()
        .is_some_and(|ct| ct.starts_with("text/event-stream")));
}

#[tokio::test(flavor = "multi_thread")]
async fn approvals_are_owner_scoped_for_non_admin_callers() {
    let h = boot().await;
    let bob_agent = spawn_authored(&h.state, "Bob");
    let session = format!("side-door-session-{}", uuid::Uuid::new_v4());
    let approval = submit_approval(&h.state, bob_agent, Some(&session));
    let aid = approval.to_string();

    // Alice sees none of Bob's approvals.
    let (status, _, body) = send(&h, Method::GET, "/api/approvals", ALICE_KEY, None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(!contains_id(&body["approvals"], "id", &aid));
    assert_eq!(body["total"], 0);
    let (_, _, body) = send(&h, Method::GET, "/api/approvals/count", ALICE_KEY, None).await;
    assert_eq!(body["pending"], 0);
    let (status, _, _) = send(
        &h,
        Method::GET,
        &format!("/api/approvals/{aid}"),
        ALICE_KEY,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (_, _, body) = send(
        &h,
        Method::GET,
        &format!("/api/approvals/session/{session}"),
        ALICE_KEY,
        None,
    )
    .await;
    assert_eq!(body["count"], 0);

    // Alice cannot decide Bob's approval through any action route.
    let (status, _, _) = send(
        &h,
        Method::POST,
        &format!("/api/approvals/{aid}/approve"),
        ALICE_KEY,
        Some(serde_json::json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _, _) = send(
        &h,
        Method::POST,
        &format!("/api/approvals/{aid}/reject"),
        ALICE_KEY,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _, _) = send(
        &h,
        Method::POST,
        &format!("/api/approvals/{aid}/modify"),
        ALICE_KEY,
        Some(serde_json::json!({"feedback": "no"})),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _, body) = send(
        &h,
        Method::POST,
        "/api/approvals/batch",
        ALICE_KEY,
        Some(serde_json::json!({"ids": [aid], "decision": "approve"})),
    )
    .await;
    assert_eq!(status, StatusCode::MULTI_STATUS);
    assert_eq!(body["results"][0]["status"], "error");
    let (_, _, body) = send(
        &h,
        Method::POST,
        &format!("/api/approvals/session/{session}/approve_all"),
        ALICE_KEY,
        Some(serde_json::json!({})),
    )
    .await;
    assert_eq!(body["resolved"], 0);
    let (_, _, body) = send(
        &h,
        Method::POST,
        &format!("/api/approvals/session/{session}/reject_all"),
        ALICE_KEY,
        None,
    )
    .await;
    assert_eq!(body["resolved"], 0);
    assert!(
        h.state.kernel.approvals().get_pending(approval).is_some(),
        "a non-owner must not be able to resolve the approval"
    );

    // Bob (the owner) sees and approves it, attributed to him.
    let (_, _, body) = send(&h, Method::GET, "/api/approvals", BOB_KEY, None).await;
    assert!(contains_id(&body["approvals"], "id", &aid));
    let (status, _, _) = send(
        &h,
        Method::POST,
        &format!("/api/approvals/{aid}/approve"),
        BOB_KEY,
        Some(serde_json::json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, _, body) = send(
        &h,
        Method::GET,
        &format!("/api/approvals/{aid}"),
        ADMIN_KEY,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["decided_by"], "Bob");

    // The decided record stays hidden from Alice, including in the persistent audit trail.
    let (status, _, _) = send(
        &h,
        Method::GET,
        &format!("/api/approvals/{aid}"),
        ALICE_KEY,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (_, _, body) = send(&h, Method::GET, "/api/approvals/audit", ALICE_KEY, None).await;
    assert!(!contains_id(&body["items"], "request_id", &aid));
    assert_eq!(body["total"], 0);
    let (_, _, body) = send(
        &h,
        Method::GET,
        &format!("/api/approvals/audit?agent_id={bob_agent}"),
        ALICE_KEY,
        None,
    )
    .await;
    assert_eq!(body["total"], 0);
    let (_, _, body) = send(&h, Method::GET, "/api/approvals/audit", BOB_KEY, None).await;
    assert!(contains_id(&body["items"], "request_id", &aid));
    let (_, _, body) = send(&h, Method::GET, "/api/approvals/audit", ADMIN_KEY, None).await;
    assert!(contains_id(&body["items"], "request_id", &aid));
}

#[tokio::test(flavor = "multi_thread")]
async fn dashboard_snapshot_agents_are_owner_scoped_without_cache_leak() {
    let h = boot().await;
    let alice_agent = spawn_authored(&h.state, "Alice").to_string();
    let bob_agent = spawn_authored(&h.state, "Bob").to_string();

    let (status, _, body) = send(&h, Method::GET, "/api/dashboard/snapshot", BOB_KEY, None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(contains_id(&body["agents"], "id", &bob_agent));
    assert!(!contains_id(&body["agents"], "id", &alice_agent));

    // Within the memoization TTL: the Admin must still get the full list, so the filtered view was not cached.
    let (_, _, body) = send(&h, Method::GET, "/api/dashboard/snapshot", ADMIN_KEY, None).await;
    assert!(contains_id(&body["agents"], "id", &bob_agent));
    assert!(contains_id(&body["agents"], "id", &alice_agent));

    let (_, _, body) = send(&h, Method::GET, "/api/dashboard/snapshot", ALICE_KEY, None).await;
    assert!(contains_id(&body["agents"], "id", &alice_agent));
    assert!(!contains_id(&body["agents"], "id", &bob_agent));
}

#[tokio::test(flavor = "multi_thread")]
async fn budget_and_usage_routes_enforce_owner_read() {
    let h = boot().await;
    let alice_agent = spawn_authored(&h.state, "Alice");
    let idle_alice_agent = spawn_authored(&h.state, "Alice");
    let aid = alice_agent.to_string();
    for (input, output, cost) in [(100, 50, 0.25), (10, 5, 0.5)] {
        h.state
            .kernel
            .memory_substrate()
            .usage()
            .record(&librefang_memory::usage::UsageRecord {
                agent_id: alice_agent,
                model: "test-model".to_string(),
                input_tokens: input,
                output_tokens: output,
                cost_usd: cost,
                tool_calls: 1,
                ..Default::default()
            })
            .expect("record usage");
    }

    let detail = format!("/api/budget/agents/{aid}");
    let (status, _, _) = send(&h, Method::GET, &detail, BOB_KEY, None).await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "Bob must not read Alice's agent spend"
    );
    let (status, _, _) = send(&h, Method::GET, &detail, ALICE_KEY, None).await;
    assert_eq!(status, StatusCode::OK);
    let (status, _, _) = send(&h, Method::GET, &detail, ADMIN_KEY, None).await;
    assert_eq!(status, StatusCode::OK);

    for path in ["/api/budget/agents", "/api/usage"] {
        let (status, _, body) = send(&h, Method::GET, path, BOB_KEY, None).await;
        assert_eq!(status, StatusCode::OK);
        assert!(
            !contains_id(&body["items"], "agent_id", &aid),
            "{path} leaked Alice's agent to Bob"
        );
        let (_, _, body) = send(&h, Method::GET, path, ALICE_KEY, None).await;
        assert!(
            contains_id(&body["items"], "agent_id", &aid),
            "{path} must still show Alice her own agent"
        );
        let (_, _, body) = send(&h, Method::GET, path, ADMIN_KEY, None).await;
        assert!(
            contains_id(&body["items"], "agent_id", &aid),
            "{path} must show Admin every agent"
        );
    }

    // `/api/usage` keeps its exact per-agent sums and zero-fills an idle agent after the switch to one grouped query.
    let (_, _, body) = send(&h, Method::GET, "/api/usage", ALICE_KEY, None).await;
    let items = body["items"].as_array().expect("usage items");
    let used = items
        .iter()
        .find(|item| item["agent_id"].as_str() == Some(aid.as_str()))
        .expect("used agent row");
    assert_eq!(used["call_count"], 2);
    assert_eq!(used["input_tokens"], 110);
    assert_eq!(used["output_tokens"], 55);
    assert_eq!(used["tool_calls"], 2);
    assert!((used["total_cost_usd"].as_f64().unwrap() - 0.75).abs() < 1e-9);
    let idle = items
        .iter()
        .find(|item| item["agent_id"].as_str() == Some(idle_alice_agent.to_string().as_str()))
        .expect("idle agent row");
    assert_eq!(idle["call_count"], 0);
    assert_eq!(idle["total_cost_usd"], 0.0);
}
