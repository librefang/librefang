//! Integration tests for the global / per-agent budget HTTP routes.
//!
//! Refs #3571 — "~80% of registered HTTP routes have no integration test."
//! Slice covered here: the non-user budget surface.
//!
//!   * `GET  /api/budget`               — global budget status snapshot
//!   * `PUT  /api/budget`               — global budget mutation + audit
//!   * `GET  /api/budget/agents`        — per-agent cost ranking
//!   * `GET  /api/budget/agents/{id}`   — single-agent budget detail
//!   * `PUT  /api/budget/agents/{id}`   — single-agent budget mutation
//!   * `GET  /api/usage`                — per-agent usage rollup
//!   * `GET  /api/usage/summary`        — global usage rollup
//!
//! User-budget routes (`/api/budget/users{,/...}`) are already covered in
//! `api_integration_test.rs` and are intentionally out of scope here.
//!
//! These tests boot a real `LibreFangKernel` via `MockKernelBuilder` (no
//! networking, no LLM credentials) and drive the `routes::budget::router()`
//! via `tower::ServiceExt::oneshot` — same pattern as `users_test.rs`.

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use axum::Router;
use librefang_api::routes::{self, AppState};
use librefang_memory::usage::{UsageRecord, UsageStore};
use librefang_testing::{MockKernelBuilder, TestAppState};
use librefang_types::agent::{
    AgentEntry, AgentId, AgentManifest, AgentMode, AgentState, ResourceQuota, SessionId,
};
use std::sync::Arc;
use tower::ServiceExt;

struct Harness {
    app: Router,
    state: Arc<AppState>,
    _test: TestAppState,
}

fn manifest(name: &str) -> AgentManifest {
    AgentManifest {
        name: name.to_string(),
        description: "test agent".to_string(),
        author: "test".to_string(),
        module: "builtin:chat".to_string(),
        ..Default::default()
    }
}

/// Register a synthetic agent directly into the kernel registry so the
/// budget routes have something to enumerate. Returns the new agent id.
fn register_agent(state: &AppState, name: &str, quota: ResourceQuota) -> AgentId {
    let id = AgentId::new();
    let mut m = manifest(name);
    m.resources = quota;
    let entry = AgentEntry {
        id,
        name: name.to_string(),
        manifest: m,
        state: AgentState::Running,
        mode: AgentMode::default(),
        created_at: chrono::Utc::now(),
        last_active: chrono::Utc::now(),
        session_id: SessionId::new(),
        ..Default::default()
    };
    state.kernel.agent_registry().register(entry).unwrap();
    id
}

/// Insert a usage row directly into the SQLite usage store. Bypasses the
/// metering engine's quota gates — the budget read endpoints aggregate
/// straight from `usage_events`, so a raw insert is sufficient and keeps
/// the test independent of provider catalogs and pricing tables.
fn record_usage(state: &AppState, agent_id: AgentId, cost_usd: f64) {
    let store = UsageStore::new(state.kernel.memory_substrate().usage_conn());
    let mut rec = UsageRecord::anonymous(agent_id, "test", "test-model", 100, 200, cost_usd, 0, 10);
    rec.session_id = Some(SessionId::new());
    store.record(&rec).unwrap();
}

async fn boot() -> Harness {
    let test = TestAppState::with_builder(MockKernelBuilder::new().with_config(move |cfg| {
        cfg.default_model = librefang_types::config::DefaultModelConfig {
            provider: "ollama".to_string(),
            model: "test-model".to_string(),
            api_key_env: "OLLAMA_API_KEY".to_string(),
            base_url: None,
            message_timeout_secs: 300,
            extra_params: std::collections::HashMap::new(),
            cli_profile_dirs: Vec::new(),
        };
        cfg.budget = librefang_types::config::BudgetConfig {
            max_hourly_usd: 1.0,
            max_daily_usd: 10.0,
            max_monthly_usd: 100.0,
            alert_threshold: 0.8,
            ..Default::default()
        };
    }));
    let state = test.state.clone();
    let app = Router::new()
        .nest("/api", routes::budget::router())
        .with_state(state.clone());
    Harness {
        app,
        state,
        _test: test,
    }
}

async fn request(
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

// ---------------------------------------------------------------------------
// GET /api/budget — happy path on a fresh kernel
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn budget_status_returns_configured_limits_with_zero_spend() {
    let h = boot().await;
    let (status, body) = request(&h, Method::GET, "/api/budget", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["hourly_limit"], 1.0);
    assert_eq!(body["daily_limit"], 10.0);
    assert_eq!(body["monthly_limit"], 100.0);
    assert_eq!(body["alert_threshold"], 0.8);
    assert_eq!(body["hourly_spend"], 0.0);
    assert_eq!(body["daily_spend"], 0.0);
    assert_eq!(body["monthly_spend"], 0.0);
}

// ---------------------------------------------------------------------------
// PUT /api/budget — read-after-write + alias key acceptance
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn budget_put_then_get_reflects_update() {
    let h = boot().await;
    let (status, body) = request(
        &h,
        Method::PUT,
        "/api/budget",
        Some(serde_json::json!({
            "max_hourly_usd": 2.5,
            "max_daily_usd": 25.0,
            "max_monthly_usd": 250.0,
            "alert_threshold": 0.5,
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "put: {body:?}");
    assert_eq!(body["hourly_limit"], 2.5);
    assert_eq!(body["alert_threshold"], 0.5);

    let (status, body) = request(&h, Method::GET, "/api/budget", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["hourly_limit"], 2.5);
    assert_eq!(body["daily_limit"], 25.0);
    assert_eq!(body["monthly_limit"], 250.0);
}

#[tokio::test(flavor = "multi_thread")]
async fn budget_put_accepts_response_shape_aliases() {
    // The handler accepts both the config-side key (`max_hourly_usd`) and
    // the GET-response key (`hourly_limit`) so a read-modify-write client
    // that pipes GET into PUT works without renaming. Lock that in.
    let h = boot().await;
    let (status, body) = request(
        &h,
        Method::PUT,
        "/api/budget",
        Some(serde_json::json!({
            "hourly_limit": 9.0,
            "daily_limit": 90.0,
            "monthly_limit": 900.0,
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "put alias: {body:?}");
    assert_eq!(body["hourly_limit"], 9.0);
    assert_eq!(body["daily_limit"], 90.0);
    assert_eq!(body["monthly_limit"], 900.0);
}

#[tokio::test(flavor = "multi_thread")]
async fn budget_put_clamps_alert_threshold_to_unit_range() {
    let h = boot().await;
    let (_, body) = request(
        &h,
        Method::PUT,
        "/api/budget",
        Some(serde_json::json!({"alert_threshold": 5.0})),
    )
    .await;
    assert_eq!(body["alert_threshold"], 1.0);

    let (_, body) = request(
        &h,
        Method::PUT,
        "/api/budget",
        Some(serde_json::json!({"alert_threshold": -0.5})),
    )
    .await;
    assert_eq!(body["alert_threshold"], 0.0);
}

#[tokio::test(flavor = "multi_thread")]
async fn budget_put_with_empty_object_is_noop() {
    // No fields = no mutation. The handler is permissive (unlike the
    // user-budget PUT) so an empty body just round-trips current state.
    let h = boot().await;
    let (status, body) = request(&h, Method::PUT, "/api/budget", Some(serde_json::json!({}))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["hourly_limit"], 1.0);
    assert_eq!(body["daily_limit"], 10.0);
}

// ---------------------------------------------------------------------------
// GET /api/budget/agents — ranking shape, empty + populated
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn agent_budget_ranking_is_empty_with_no_usage() {
    let h = boot().await;
    let _ = register_agent(&h.state, "alpha", ResourceQuota::default());
    let (status, body) = request(&h, Method::GET, "/api/budget/agents", None).await;
    assert_eq!(status, StatusCode::OK);
    // Agents with zero spend are filtered out — the ranking is "top
    // spenders", not "every registered agent".
    assert_eq!(body["agents"].as_array().unwrap().len(), 0);
    assert_eq!(body["total"], 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn agent_budget_ranking_lists_agents_with_recorded_spend() {
    let h = boot().await;
    let quota = ResourceQuota {
        max_cost_per_hour_usd: 1.0,
        max_cost_per_day_usd: 10.0,
        max_cost_per_month_usd: 100.0,
        ..Default::default()
    };
    let alpha = register_agent(&h.state, "alpha", quota.clone());
    let _beta = register_agent(&h.state, "beta", quota);
    record_usage(&h.state, alpha, 0.42);

    let (status, body) = request(&h, Method::GET, "/api/budget/agents", None).await;
    assert_eq!(status, StatusCode::OK);
    let agents = body["agents"].as_array().unwrap();
    assert_eq!(agents.len(), 1, "only alpha has spend: {body:?}");
    let row = &agents[0];
    assert_eq!(row["agent_id"], alpha.to_string());
    assert_eq!(row["name"], "alpha");
    assert!(
        (row["daily_cost_usd"].as_f64().unwrap() - 0.42).abs() < 1e-9,
        "row: {row:?}"
    );
    assert_eq!(row["hourly_limit"], 1.0);
    assert_eq!(row["daily_limit"], 10.0);
    assert_eq!(row["monthly_limit"], 100.0);
    assert!(row["max_llm_tokens_per_hour"].is_number());
    assert_eq!(body["total"], 1);
}

// ---------------------------------------------------------------------------
// GET /api/budget/agents/{id} — happy path, bad id, missing agent
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn agent_budget_status_returns_pct_and_limits() {
    let h = boot().await;
    let quota = ResourceQuota {
        max_cost_per_hour_usd: 2.0,
        max_cost_per_day_usd: 20.0,
        max_cost_per_month_usd: 200.0,
        ..Default::default()
    };
    let id = register_agent(&h.state, "solo", quota);
    record_usage(&h.state, id, 1.0);

    let path = format!("/api/budget/agents/{id}");
    let (status, body) = request(&h, Method::GET, &path, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["agent_id"], id.to_string());
    assert_eq!(body["agent_name"], "solo");
    assert_eq!(body["hourly"]["limit"], 2.0);
    assert_eq!(body["daily"]["limit"], 20.0);
    assert_eq!(body["monthly"]["limit"], 200.0);
    // 1.0 spend / 20.0 daily limit = 0.05
    let daily_pct = body["daily"]["pct"].as_f64().unwrap();
    assert!(
        (daily_pct - 0.05).abs() < 1e-9,
        "daily pct = {daily_pct}, body = {body:?}"
    );
    assert!(body["tokens"]["used"].is_number());
}

#[tokio::test(flavor = "multi_thread")]
async fn agent_budget_status_rejects_invalid_id_with_400() {
    let h = boot().await;
    let (status, body) = request(&h, Method::GET, "/api/budget/agents/not-a-uuid", None).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        body["error"]
            .as_str()
            .unwrap_or("")
            .to_lowercase()
            .contains("invalid"),
        "body: {body:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn agent_budget_status_returns_404_for_unknown_agent() {
    let h = boot().await;
    let path = format!("/api/budget/agents/{}", AgentId::new());
    let (status, _) = request(&h, Method::GET, &path, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

// ---------------------------------------------------------------------------
// PUT /api/budget/agents/{id} — read-after-write + invalid payloads
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn update_agent_budget_then_get_reflects_new_limits() {
    let h = boot().await;
    let id = register_agent(&h.state, "movable", ResourceQuota::default());
    let path = format!("/api/budget/agents/{id}");

    let (status, _) = request(
        &h,
        Method::PUT,
        &path,
        Some(serde_json::json!({
            "max_cost_per_hour_usd": 4.0,
            "max_cost_per_day_usd": 40.0,
            "max_cost_per_month_usd": 400.0,
            "max_llm_tokens_per_hour": 5000,
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, body) = request(&h, Method::GET, &path, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["hourly"]["limit"], 4.0);
    assert_eq!(body["daily"]["limit"], 40.0);
    assert_eq!(body["monthly"]["limit"], 400.0);
    assert_eq!(body["tokens"]["limit"], 5000);
}

#[tokio::test(flavor = "multi_thread")]
async fn update_agent_budget_rejects_empty_body_with_400() {
    let h = boot().await;
    let id = register_agent(&h.state, "stubborn", ResourceQuota::default());
    let path = format!("/api/budget/agents/{id}");
    let (status, body) = request(&h, Method::PUT, &path, Some(serde_json::json!({}))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        body["error"]
            .as_str()
            .unwrap_or("")
            .contains("at least one"),
        "body: {body:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn update_agent_budget_rejects_unknown_agent_with_404() {
    let h = boot().await;
    let path = format!("/api/budget/agents/{}", AgentId::new());
    let (status, _) = request(
        &h,
        Method::PUT,
        &path,
        Some(serde_json::json!({"max_cost_per_hour_usd": 1.0})),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

// ---------------------------------------------------------------------------
// /api/usage and /api/usage/summary — aggregation sanity
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn usage_stats_lists_each_registered_agent() {
    let h = boot().await;
    let id = register_agent(&h.state, "scribe", ResourceQuota::default());
    record_usage(&h.state, id, 0.10);
    record_usage(&h.state, id, 0.05);

    let (status, body) = request(&h, Method::GET, "/api/usage", None).await;
    assert_eq!(status, StatusCode::OK);
    let agents = body["agents"].as_array().unwrap();
    // The kernel may auto-register internal agents (system hands etc.) so we
    // locate our scribe by id rather than asserting the total count — what
    // we're verifying here is that recorded usage is rolled up onto the row
    // for the registered agent, not the size of the registry.
    let row = agents
        .iter()
        .find(|r| r["agent_id"] == id.to_string())
        .unwrap_or_else(|| panic!("scribe row missing from /api/usage: {body:?}"));
    assert_eq!(row["name"], "scribe");
    assert_eq!(row["call_count"], 2);
    assert_eq!(row["input_tokens"], 200);
    assert_eq!(row["output_tokens"], 400);
    assert!(
        (row["total_cost_usd"].as_f64().unwrap() - 0.15).abs() < 1e-9,
        "row: {row:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn usage_summary_aggregates_across_agents() {
    let h = boot().await;
    let a = register_agent(&h.state, "a", ResourceQuota::default());
    let b = register_agent(&h.state, "b", ResourceQuota::default());
    record_usage(&h.state, a, 0.25);
    record_usage(&h.state, b, 0.75);

    let (status, body) = request(&h, Method::GET, "/api/usage/summary", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["call_count"], 2);
    assert!(
        (body["total_cost_usd"].as_f64().unwrap() - 1.0).abs() < 1e-9,
        "body: {body:?}"
    );
    // Each record contributes 100 input + 200 output tokens.
    assert_eq!(body["total_input_tokens"], 200);
    assert_eq!(body["total_output_tokens"], 400);
}
