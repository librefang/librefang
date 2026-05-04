//! Integration tests for #3511 — `agent_id` propagation into the HTTP
//! access log via `Response::extensions`.
//!
//! The middleware in `crates/librefang-api/src/middleware.rs::request_logging`
//! reads back an [`AgentIdField`](librefang_api::extensions::AgentIdField)
//! marker after `next.run().await` and emits it as a structured `tracing`
//! field. The test surface here is the marker itself — we boot the route
//! handlers via `tower::oneshot`, hit a path-scoped endpoint, and assert
//! the response carries the marker for the agent we addressed in the path.
//!
//! We do not exercise the `tracing` subscriber: the read of
//! `response.extensions().get::<AgentIdField>()` in the middleware is a
//! one-liner and the only thing that can break it is a handler forgetting
//! to call `with_agent_id` at the end of its happy / error path. Asserting
//! on `extensions` directly is the precise contract.

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use axum::Router;
use librefang_api::extensions::AgentIdField;
use librefang_api::routes::{self, AppState};
use librefang_testing::{MockKernelBuilder, TestAppState};
use librefang_types::agent::AgentId;
use std::sync::Arc;
use tower::ServiceExt;

/// Minimal harness: nest the route module under `/api` exactly as
/// `server.rs` does, so the URI we hit in the test mirrors production.
struct Harness {
    app: Router,
    _test: TestAppState,
}

fn boot_auto_dream() -> Harness {
    let test = TestAppState::with_builder(MockKernelBuilder::new().with_config(|cfg| {
        // Non-LLM provider keeps boot fast and avoids any accidental egress.
        cfg.default_model = librefang_types::config::DefaultModelConfig {
            provider: "ollama".to_string(),
            model: "test-model".to_string(),
            api_key_env: "OLLAMA_API_KEY".to_string(),
            base_url: None,
            message_timeout_secs: 300,
            extra_params: std::collections::HashMap::new(),
            cli_profile_dirs: Vec::new(),
        };
    }));
    let state: Arc<AppState> = test.state.clone();
    let app = Router::new()
        .nest("/api", routes::auto_dream::router())
        .with_state(state);
    Harness { app, _test: test }
}

fn boot_budget() -> Harness {
    let test = TestAppState::with_builder(MockKernelBuilder::new().with_config(|cfg| {
        cfg.default_model = librefang_types::config::DefaultModelConfig {
            provider: "ollama".to_string(),
            model: "test-model".to_string(),
            api_key_env: "OLLAMA_API_KEY".to_string(),
            base_url: None,
            message_timeout_secs: 300,
            extra_params: std::collections::HashMap::new(),
            cli_profile_dirs: Vec::new(),
        };
    }));
    let state: Arc<AppState> = test.state.clone();
    let app = Router::new()
        .nest("/api", routes::budget::router())
        .with_state(state);
    Harness { app, _test: test }
}

/// `PUT /api/auto-dream/agents/{id}/enabled` for an unknown agent returns
/// `404 NOT_FOUND` (the kernel does the lookup), and the handler must
/// still tag the response with the `agent_id` that was successfully
/// parsed from the path. Without the tag the access-log line would say
/// `agent_id=""` even though the routing layer clearly knew which agent
/// the operator was trying to address.
#[tokio::test(flavor = "multi_thread")]
async fn auto_dream_set_enabled_tags_response_with_agent_id_on_404() {
    let h = boot_auto_dream();
    let unknown = AgentId::new();

    let req = Request::builder()
        .method(Method::PUT)
        .uri(format!("/api/auto-dream/agents/{unknown}/enabled"))
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::to_vec(&serde_json::json!({"enabled": true})).unwrap(),
        ))
        .unwrap();

    let resp = h.app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);

    let field = resp
        .extensions()
        .get::<AgentIdField>()
        .expect("AgentIdField must be present even when the agent is not in the registry");
    assert_eq!(field.0, unknown);
}

/// `GET /api/budget/agents/{id}` for an unknown agent returns `404`, but
/// the handler must still tag the response with the resolved `agent_id`
/// — the path was well-formed, the operator's intent is clear, the
/// access log should reflect that.
#[tokio::test(flavor = "multi_thread")]
async fn budget_agent_status_tags_response_with_agent_id_on_404() {
    let h = boot_budget();
    let unknown = AgentId::new();

    let req = Request::builder()
        .method(Method::GET)
        .uri(format!("/api/budget/agents/{unknown}"))
        .body(Body::empty())
        .unwrap();

    let resp = h.app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);

    let field = resp
        .extensions()
        .get::<AgentIdField>()
        .expect("AgentIdField must be present on the 404 response");
    assert_eq!(field.0, unknown);
}

/// Malformed agent id never reaches the handler — the `AgentIdPath`
/// extractor rejects with `400` before the handler runs, so the
/// `with_agent_id` call site never executes. The access-log line for
/// such a request will carry an empty `agent_id` field, which is the
/// documented behavior (no resolved agent, no marker).
#[tokio::test(flavor = "multi_thread")]
async fn auto_dream_set_enabled_no_marker_when_path_is_malformed() {
    let h = boot_auto_dream();

    let req = Request::builder()
        .method(Method::PUT)
        .uri("/api/auto-dream/agents/not-a-uuid/enabled")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::to_vec(&serde_json::json!({"enabled": true})).unwrap(),
        ))
        .unwrap();

    let resp = h.app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    assert!(
        resp.extensions().get::<AgentIdField>().is_none(),
        "marker must NOT be set when the path id failed to parse — \
         the handler never ran, so no agent_id was ever resolved"
    );
}
