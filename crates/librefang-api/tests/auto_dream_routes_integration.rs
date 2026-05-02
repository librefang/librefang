//! Integration tests for the `/api/auto-dream/*` HTTP surface.
//!
//! Refs #3571 — "~80% of registered HTTP routes have no integration test".
//! This file covers the four endpoints registered in
//! `crates/librefang-api/src/routes/auto_dream.rs`:
//!
//!   * `GET  /api/auto-dream/status`
//!   * `POST /api/auto-dream/agents/{id}/trigger`
//!   * `POST /api/auto-dream/agents/{id}/abort`
//!   * `PUT  /api/auto-dream/agents/{id}/enabled`
//!
//! We follow the same `tower::oneshot` + `MockKernelBuilder` + `TestAppState`
//! recipe used by `users_test.rs`. The router is mounted under `/api`
//! exactly the way `server.rs` mounts it.
//!
//! Scope notes:
//! - Trigger happy-path (an actual dream run) would require a real LLM
//!   driver and a fully-spawned agent, neither of which the in-process test
//!   harness provides. By default `auto_dream.enabled = false`, so
//!   `trigger_manual` short-circuits with `fired=false, reason="auto-dream
//!   is disabled in config"` before any LLM dispatch — that path is safe
//!   to exercise. We also cover the "agent not found" validation branch.
//! - Abort happy-path requires an in-flight manual dream; we cover the
//!   "nothing in flight" branch and the invalid-id branch only.

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use axum::Router;
use librefang_api::routes::{self, AppState};
use librefang_testing::{MockKernelBuilder, TestAppState};
use std::sync::Arc;
use tower::ServiceExt;

struct Harness {
    app: Router,
    _state: Arc<AppState>,
    _test: TestAppState,
}

async fn boot() -> Harness {
    boot_with(|_| {}).await
}

async fn boot_with<F>(mutate: F) -> Harness
where
    F: FnOnce(&mut librefang_types::config::KernelConfig) + 'static,
{
    let test = TestAppState::with_builder(MockKernelBuilder::new().with_config(move |cfg| {
        // A non-LLM provider keeps the kernel boot fast and avoids any
        // accidental network egress if a downstream code path tries to
        // resolve the default driver.
        cfg.default_model = librefang_types::config::DefaultModelConfig {
            provider: "ollama".to_string(),
            model: "test-model".to_string(),
            api_key_env: "OLLAMA_API_KEY".to_string(),
            base_url: None,
            message_timeout_secs: 300,
            extra_params: std::collections::HashMap::new(),
            cli_profile_dirs: Vec::new(),
        };
        mutate(cfg);
    }));

    let state = test.state.clone();
    let app = Router::new()
        .nest("/api", routes::auto_dream::router())
        .with_state(state.clone());

    Harness {
        app,
        _state: state,
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

// ---------------------------------------------------------------------------
// GET /api/auto-dream/status
// ---------------------------------------------------------------------------

/// Default-config status — global toggle off, empty agent list. Pins the
/// public response shape the dashboard depends on.
#[tokio::test(flavor = "multi_thread")]
async fn auto_dream_status_default_shape() {
    let h = boot().await;
    let (status, body) = json_request(&h, Method::GET, "/api/auto-dream/status", None).await;
    assert_eq!(status, StatusCode::OK, "{body:?}");
    // Top-level fields must exist with the documented types.
    assert_eq!(body["enabled"], false, "default global toggle is off");
    assert!(body["min_hours"].is_number(), "{body:?}");
    assert!(body["min_sessions"].is_number(), "{body:?}");
    assert!(body["check_interval_secs"].is_number(), "{body:?}");
    assert!(body["lock_dir"].is_string(), "{body:?}");
    assert!(body["agents"].is_array(), "{body:?}");
    // The mock kernel seeds a default agent; each row must carry the
    // documented per-agent fields the dashboard renders.
    let agents = body["agents"].as_array().expect("agents array");
    assert!(!agents.is_empty(), "expected at least the default agent");
    let row = &agents[0];
    for key in [
        "agent_id",
        "agent_name",
        "auto_dream_enabled",
        "last_consolidated_at_ms",
        "sessions_since_last",
        "effective_min_hours",
        "effective_min_sessions",
        "lock_path",
        "can_abort",
    ] {
        assert!(
            row.get(key).is_some(),
            "agent row missing `{key}` field: {row:?}"
        );
    }
    // Default-seeded agents are not opted in; can_abort is always false
    // when no dream is in flight.
    assert_eq!(row["auto_dream_enabled"], false, "{row:?}");
    assert_eq!(row["can_abort"], false, "{row:?}");
}

/// Status reflects the live `[auto_dream]` config block. Ensures the route
/// reads from `config_snapshot()` rather than a stale clone.
#[tokio::test(flavor = "multi_thread")]
async fn auto_dream_status_reflects_config_overrides() {
    let h = boot_with(|cfg| {
        cfg.auto_dream.enabled = true;
        cfg.auto_dream.min_hours = 12.0;
        cfg.auto_dream.min_sessions = 3;
        cfg.auto_dream.check_interval_secs = 3600;
    })
    .await;
    let (status, body) = json_request(&h, Method::GET, "/api/auto-dream/status", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["enabled"], true);
    assert_eq!(body["min_hours"], 12.0);
    assert_eq!(body["min_sessions"], 3);
    assert_eq!(body["check_interval_secs"], 3600);
}

// ---------------------------------------------------------------------------
// POST /api/auto-dream/agents/{id}/trigger
// ---------------------------------------------------------------------------

/// Malformed agent id is a 400 with a JSON error body. This is the cheapest
/// validation guard and the most likely thing to regress when the handler is
/// refactored to use a typed extractor.
#[tokio::test(flavor = "multi_thread")]
async fn auto_dream_trigger_invalid_id_is_400() {
    let h = boot().await;
    let (status, body) = json_request(
        &h,
        Method::POST,
        "/api/auto-dream/agents/not-a-uuid/trigger",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body:?}");
    assert_eq!(body["error"], "invalid agent id");
}

/// Globally disabled auto-dream short-circuits before any LLM dispatch and
/// returns `fired=false` with a specific reason. This is what protects the
/// trigger endpoint from being a foot-gun when an operator hits it before
/// flipping the global toggle on.
#[tokio::test(flavor = "multi_thread")]
async fn auto_dream_trigger_returns_disabled_when_global_off() {
    let h = boot().await; // default: auto_dream.enabled = false
    let some_id = librefang_types::agent::AgentId::new();
    let (status, body) = json_request(
        &h,
        Method::POST,
        &format!("/api/auto-dream/agents/{some_id}/trigger"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body:?}");
    assert_eq!(body["fired"], false);
    assert_eq!(body["agent_id"], some_id.to_string());
    assert!(body["task_id"].is_null());
    assert_eq!(body["reason"], "auto-dream is disabled in config");
}

/// Even with the global toggle on, an unknown agent id resolves to
/// `fired=false, reason="agent not found"`. This is the safe path we can
/// test without spawning a real agent — it avoids ever reaching the LLM
/// dispatch branch.
#[tokio::test(flavor = "multi_thread")]
async fn auto_dream_trigger_unknown_agent_returns_not_found_reason() {
    let h = boot_with(|cfg| {
        cfg.auto_dream.enabled = true;
    })
    .await;
    let some_id = librefang_types::agent::AgentId::new();
    let (status, body) = json_request(
        &h,
        Method::POST,
        &format!("/api/auto-dream/agents/{some_id}/trigger"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body:?}");
    assert_eq!(body["fired"], false);
    assert_eq!(body["agent_id"], some_id.to_string());
    assert_eq!(body["reason"], "agent not found");
}

// ---------------------------------------------------------------------------
// POST /api/auto-dream/agents/{id}/abort
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn auto_dream_abort_invalid_id_is_400() {
    let h = boot().await;
    let (status, body) = json_request(
        &h,
        Method::POST,
        "/api/auto-dream/agents/not-a-uuid/abort",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body:?}");
    assert_eq!(body["error"], "invalid agent id");
}

/// No dream is ever in-flight in this test binary, so abort must surface
/// the documented "nothing to abort" reason rather than 404 / 500. Pins
/// the contract dashboards rely on for stale-button cleanup.
#[tokio::test(flavor = "multi_thread")]
async fn auto_dream_abort_with_no_in_flight_dream_returns_aborted_false() {
    let h = boot().await;
    let some_id = librefang_types::agent::AgentId::new();
    let (status, body) = json_request(
        &h,
        Method::POST,
        &format!("/api/auto-dream/agents/{some_id}/abort"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body:?}");
    assert_eq!(body["aborted"], false);
    assert_eq!(body["agent_id"], some_id.to_string());
    assert!(
        body["reason"]
            .as_str()
            .unwrap_or("")
            .contains("no abort-capable dream"),
        "unexpected reason: {body:?}"
    );
}

// ---------------------------------------------------------------------------
// PUT /api/auto-dream/agents/{id}/enabled
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn auto_dream_set_enabled_invalid_id_is_400() {
    let h = boot().await;
    let (status, body) = json_request(
        &h,
        Method::PUT,
        "/api/auto-dream/agents/not-a-uuid/enabled",
        Some(serde_json::json!({"enabled": true})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body:?}");
    assert_eq!(body["error"], "invalid agent id");
}

/// Well-formed UUID for an agent that doesn't exist returns 404 from the
/// underlying `update_auto_dream_enabled` registry call.
#[tokio::test(flavor = "multi_thread")]
async fn auto_dream_set_enabled_unknown_agent_is_404() {
    let h = boot().await;
    let some_id = librefang_types::agent::AgentId::new();
    let (status, body) = json_request(
        &h,
        Method::PUT,
        &format!("/api/auto-dream/agents/{some_id}/enabled"),
        Some(serde_json::json!({"enabled": true})),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body:?}");
    assert!(
        !body["error"].as_str().unwrap_or("").is_empty(),
        "expected an error string: {body:?}"
    );
}

/// Missing `enabled` field in the JSON body is a deserialization failure
/// and must be a 4xx (axum's `Json` extractor rejects with 400/422), not a
/// 500 / panic. Pins the validation contract for the toggle endpoint.
#[tokio::test(flavor = "multi_thread")]
async fn auto_dream_set_enabled_missing_field_rejects_with_4xx() {
    let h = boot().await;
    let some_id = librefang_types::agent::AgentId::new();
    let (status, _body) = json_request(
        &h,
        Method::PUT,
        &format!("/api/auto-dream/agents/{some_id}/enabled"),
        Some(serde_json::json!({})),
    )
    .await;
    assert!(
        status.is_client_error(),
        "missing `enabled` field must be a 4xx, got {status}"
    );
    assert_ne!(
        status,
        StatusCode::INTERNAL_SERVER_ERROR,
        "must not be a 500 — bad input is a client error"
    );
}
