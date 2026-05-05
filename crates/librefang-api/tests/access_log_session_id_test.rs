//! Integration tests for #3511 — `session_id` propagation into the HTTP
//! access log via `Response::extensions`.
//!
//! The middleware in `crates/librefang-api/src/middleware.rs::request_logging`
//! reads back a [`SessionIdField`](librefang_api::extensions::SessionIdField)
//! marker after `next.run().await` and emits it as a structured `session_id`
//! field on the access-log event.
//!
//! Like the companion `access_log_agent_id_test`, the surface exercised here
//! is the extension marker on the raw [`axum::response::Response`] — we do
//! not instrument a tracing subscriber. What matters is that handlers which
//! know the session_id call `with_session_id` (or the composed
//! `with_session_id(sid, with_agent_id(aid, body))` form) so the middleware
//! can emit a non-empty field.

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use axum::Router;
use librefang_api::extensions::SessionIdField;
use librefang_api::routes::{self, AppState};
use librefang_testing::{MockKernelBuilder, TestAppState};
use librefang_types::agent::AgentId;
use std::sync::Arc;
use tower::ServiceExt;

/// Minimal harness: nest the agents route under `/api` exactly as
/// `server.rs` does so URIs mirror production.
struct Harness {
    app: Router,
    _test: TestAppState,
}

fn boot_agents() -> Harness {
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
        .nest("/api", routes::agents::router())
        .with_state(state);
    Harness { app, _test: test }
}

/// `GET /api/agents/{id}/session` for an existing agent returns the canonical
/// session and the handler tags the response with both `agent_id` and
/// `session_id`.  The access-log middleware reads both markers and can emit
/// them as structured fields — confirmed here by asserting on the extension
/// directly (the precise contract the middleware relies on).
#[tokio::test(flavor = "multi_thread")]
async fn get_agent_session_tags_response_with_session_id() {
    let h = boot_agents();

    // Obtain a real registered agent from the mock kernel.
    let agent_id = {
        let registry = h._test.state.kernel.agent_registry();
        let agents = registry.list();
        assert!(
            !agents.is_empty(),
            "MockKernel must pre-populate at least one agent for this test"
        );
        agents[0].id
    };

    let req = Request::builder()
        .method(Method::GET)
        .uri(format!("/api/agents/{agent_id}/session"))
        .body(Body::empty())
        .unwrap();

    let resp = h.app.clone().oneshot(req).await.unwrap();
    // 200 (session exists or empty) or 404 (agent not wired in mock memory)
    // — either way the handler must have tagged the response.
    assert!(
        resp.status() == StatusCode::OK || resp.status() == StatusCode::NOT_FOUND,
        "unexpected status {}",
        resp.status()
    );

    assert!(
        resp.extensions().get::<SessionIdField>().is_some(),
        "SessionIdField must be present in response extensions for GET /session — \
         the access-log middleware needs it to emit a non-empty session_id field"
    );
}

/// `GET /api/agents/{id}/session` for an unknown agent returns `404` before
/// reaching the session-lookup logic, so no `session_id` is ever resolved and
/// the marker must NOT be present.  The access-log line for such a request
/// will carry `session_id=""`, which is the documented behaviour when no
/// session could be determined.
#[tokio::test(flavor = "multi_thread")]
async fn get_agent_session_no_marker_when_agent_not_found() {
    let h = boot_agents();
    let unknown = AgentId::new();

    let req = Request::builder()
        .method(Method::GET)
        .uri(format!("/api/agents/{unknown}/session"))
        .body(Body::empty())
        .unwrap();

    let resp = h.app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);

    assert!(
        resp.extensions().get::<SessionIdField>().is_none(),
        "SessionIdField must NOT be set when the agent is unknown — \
         no session was ever resolved"
    );
}

/// `GET /api/agents/{id}/sessions/{session_id}/stream` parses both ids from
/// the path.  Even though the SSE stream cannot be driven to completion in a
/// unit test, the handler validates the session binding synchronously before
/// starting the stream; a 404 for a non-existent session still reflects the
/// parsed ids and the response must carry `session_id`.
///
/// Note: the stream endpoint actually tags agent_id+session_id only on the
/// success path (the SSE `Sse::new(...)` response).  For the 404 path the
/// error is returned before the tagging site so the marker is absent — which
/// is also acceptable (no resolved session, no field).  This test documents
/// that contract.
#[tokio::test(flavor = "multi_thread")]
async fn attach_session_stream_404_has_no_session_marker() {
    let h = boot_agents();
    let agent_id = AgentId::new();
    let session_id = librefang_types::agent::SessionId::new();

    let req = Request::builder()
        .method(Method::GET)
        .uri(format!(
            "/api/agents/{agent_id}/sessions/{session_id}/stream"
        ))
        .body(Body::empty())
        .unwrap();

    let resp = h.app.clone().oneshot(req).await.unwrap();
    // Either 404 (agent not found) or 400 (parse failure on mock) — the SSE
    // stream is not reached, so no marker is set.
    assert!(
        resp.status() == StatusCode::NOT_FOUND || resp.status() == StatusCode::BAD_REQUEST,
        "unexpected status {}",
        resp.status()
    );
    // No session was resolved — marker absent is the correct contract.
    assert!(
        resp.extensions().get::<SessionIdField>().is_none(),
        "SessionIdField must NOT be set when the agent/session lookup fails \
         before reaching the stream-start tagging site"
    );
}
