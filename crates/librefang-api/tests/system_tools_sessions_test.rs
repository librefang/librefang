//! Integration tests for the `tools` and `sessions` GET sub-routes mounted
//! by `routes::system::router()`.
//!
//! Scope (issue #3571): exercise read-side endpoints whose handlers live in
//! the 5000-line `routes/system.rs` kitchen-sink and currently ship with no
//! integration coverage. Mutating routes (`/tools/{name}/invoke`,
//! `/sessions/cleanup`) are intentionally out of scope — the former has its
//! own focused harness in `tools_invoke_test.rs`, the latter needs real
//! session state we do not seed here.
//!
//! Endpoints covered:
//!   - `GET  /api/tools`              — list builtin tool definitions
//!   - `GET  /api/tools/{name}`       — single tool lookup, 404 on miss
//!   - `GET  /api/sessions`           — paginated session list (empty + seeded)
//!   - `GET  /api/sessions/search`    — FTS5 search, 400 when `q` missing
//!
//! The harness wires `routes::system::router()` directly under `/api` and
//! drives requests with `tower::ServiceExt::oneshot`. No middleware is
//! installed — auth is exercised separately in `auth_public_allowlist.rs`.

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use axum::Router;
use librefang_api::routes::{self, AppState};
use librefang_testing::{MockKernelBuilder, TestAppState};
use librefang_types::agent::AgentId;
use std::sync::Arc;
use tower::ServiceExt;

struct Harness {
    app: Router,
    state: Arc<AppState>,
    _test: TestAppState,
}

impl Drop for Harness {
    fn drop(&mut self) {
        self.state.kernel.shutdown();
    }
}

async fn boot() -> Harness {
    let test = TestAppState::with_builder(MockKernelBuilder::new().with_config(|cfg| {
        cfg.default_model = librefang_types::config::DefaultModelConfig {
            provider: "ollama".into(),
            model: "test-model".into(),
            api_key_env: "OLLAMA_API_KEY".into(),
            base_url: None,
            message_timeout_secs: 300,
            extra_params: std::collections::HashMap::new(),
            cli_profile_dirs: Vec::new(),
        };
    }));

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

async fn get_json(h: &Harness, path: &str) -> (StatusCode, serde_json::Value) {
    let req = Request::builder()
        .method(Method::GET)
        .uri(path)
        .body(Body::empty())
        .unwrap();
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
// /api/tools
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn list_tools_returns_builtins_with_total() {
    let h = boot().await;
    let (status, body) = get_json(&h, "/api/tools").await;
    assert_eq!(status, StatusCode::OK, "{body:?}");

    let tools = body["tools"].as_array().expect("tools array");
    assert!(
        !tools.is_empty(),
        "builtin tool list must not be empty: {body:?}"
    );
    // Every entry must carry the wire shape consumers depend on.
    for t in tools {
        assert!(t["name"].is_string(), "tool missing name: {t:?}");
        assert!(
            t["description"].is_string(),
            "tool missing description: {t:?}"
        );
        assert!(
            t["input_schema"].is_object(),
            "tool input_schema must be object: {t:?}"
        );
    }
    // `total` reflects the `tools` array length verbatim.
    assert_eq!(
        body["total"].as_u64().unwrap_or(0) as usize,
        tools.len(),
        "total must equal tools.len(): {body:?}"
    );

    // `file_read` is part of the builtin set since the handler was first
    // wired — pin it as a smoke marker so a silent regression that returns
    // an empty list still fails the assertions above on `is_empty`, AND a
    // regression that returns a stub list missing the filesystem tools is
    // also caught here.
    let names: Vec<&str> = tools.iter().filter_map(|t| t["name"].as_str()).collect();
    assert!(
        names.contains(&"file_read"),
        "expected `file_read` in builtin tools, got: {names:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn get_tool_returns_definition_for_known_builtin() {
    let h = boot().await;
    let (status, body) = get_json(&h, "/api/tools/file_read").await;
    assert_eq!(status, StatusCode::OK, "{body:?}");
    assert_eq!(body["name"], "file_read");
    assert!(body["description"].is_string());
    assert!(body["input_schema"].is_object());
}

#[tokio::test(flavor = "multi_thread")]
async fn get_tool_returns_404_for_unknown_name() {
    let h = boot().await;
    let (status, body) = get_json(&h, "/api/tools/nope_not_a_real_tool").await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body:?}");
    // The handler funnels through `ApiErrorResponse::not_found` with an i18n
    // template. We only assert there is *some* error string surfaced — the
    // wording is locale-driven.
    assert!(
        body.get("error").is_some(),
        "404 must include an `error` field: {body:?}"
    );
}

// ---------------------------------------------------------------------------
// /api/sessions
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn list_sessions_returns_paginated_envelope() {
    let h = boot().await;
    let (status, body) = get_json(&h, "/api/sessions").await;
    assert_eq!(status, StatusCode::OK, "{body:?}");

    // The handler always returns the paginated envelope. Pinning the shape
    // keeps dashboard consumers honest about reading `sessions`/`total`/
    // `offset`/`limit`. We do NOT assert empty here — `MockKernelBuilder`
    // boots with a seeded agent that already has a canonical session.
    assert!(body["sessions"].is_array(), "{body:?}");
    assert!(body["total"].is_number(), "{body:?}");
    assert_eq!(body["offset"].as_u64().unwrap_or(u64::MAX), 0);
    assert!(
        body["limit"].as_u64().unwrap_or(0) > 0,
        "limit must default to > 0: {body:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn list_sessions_surfaces_seeded_session() {
    let h = boot().await;

    // Capture the baseline `total` BEFORE seeding so the assertion is robust
    // to whatever sessions `MockKernelBuilder` happens to bootstrap.
    let (_, before) = get_json(&h, "/api/sessions").await;
    let baseline = before["total"].as_u64().unwrap_or(0);

    // Seed a session directly through the substrate so the list endpoint
    // has something to return — exercises the non-empty branch of the
    // `list_sessions_paginated` -> handler envelope.
    let agent_id = AgentId(uuid::Uuid::new_v4());
    let seeded_id = h
        .state
        .kernel
        .memory_substrate()
        .create_session(agent_id)
        .expect("seed session")
        .id
        .0
        .to_string();

    let (status, body) = get_json(&h, "/api/sessions").await;
    assert_eq!(status, StatusCode::OK, "{body:?}");
    assert_eq!(
        body["total"].as_u64().unwrap_or(0),
        baseline + 1,
        "total must increment after seeding: {body:?}"
    );
    let arr = body["sessions"].as_array().expect("sessions array");

    // The seeded id must be present in the returned page. The wire field
    // is `session_id` (substrate's `Session::session_id` JSON shape).
    let found = arr.iter().any(|s| {
        s.get("session_id")
            .and_then(|v| v.as_str())
            .map(|id| id == seeded_id)
            .unwrap_or(false)
    });
    assert!(found, "seeded session_id not in page: {arr:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn list_sessions_honours_pagination_query() {
    let h = boot().await;
    // Capture baseline session count before seeding (MockKernelBuilder may
    // bootstrap a canonical session for its seeded agent).
    let (_, before) = get_json(&h, "/api/sessions").await;
    let baseline = before["total"].as_u64().unwrap_or(0);

    let agent_id = AgentId(uuid::Uuid::new_v4());
    let substrate = h.state.kernel.memory_substrate();
    substrate.create_session(agent_id).expect("seed 1");
    substrate.create_session(agent_id).expect("seed 2");

    let (status, body) = get_json(&h, "/api/sessions?limit=1&offset=0").await;
    assert_eq!(status, StatusCode::OK, "{body:?}");
    assert_eq!(body["limit"].as_u64().unwrap_or(0), 1, "{body:?}");
    assert_eq!(body["offset"].as_u64().unwrap_or(u64::MAX), 0, "{body:?}");
    // `total` reflects the full count, independent of the page window.
    assert_eq!(
        body["total"].as_u64().unwrap_or(0),
        baseline + 2,
        "{body:?}"
    );
    assert_eq!(
        body["sessions"].as_array().map(|a| a.len()).unwrap_or(0),
        1,
        "page window must respect ?limit=1: {body:?}"
    );
}

// ---------------------------------------------------------------------------
// /api/sessions/search
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn search_sessions_missing_q_returns_400() {
    let h = boot().await;
    let (status, body) = get_json(&h, "/api/sessions/search").await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body:?}");
    assert!(
        body.get("error").is_some(),
        "400 must include an `error` field: {body:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn search_sessions_empty_q_returns_400() {
    let h = boot().await;
    let (status, body) = get_json(&h, "/api/sessions/search?q=").await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body:?}");
    assert!(
        body.get("error").is_some(),
        "400 must include an `error` field: {body:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn search_sessions_returns_envelope_on_no_match() {
    let h = boot().await;
    // A query that cannot match anything (substrate is empty) — the handler
    // must still return the paginated envelope, not 500.
    let (status, body) = get_json(
        &h,
        "/api/sessions/search?q=zzz_no_such_session_zzz&limit=10&offset=0",
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body:?}");
    assert!(body["results"].is_array(), "{body:?}");
    assert_eq!(body["results"].as_array().unwrap().len(), 0);
    assert_eq!(body["limit"].as_u64().unwrap_or(0), 10, "{body:?}");
    assert_eq!(body["offset"].as_u64().unwrap_or(u64::MAX), 0, "{body:?}");
    // Page is not full → next_offset must be `null`, signalling EOF.
    assert!(
        body["next_offset"].is_null(),
        "next_offset must be null when results.len() < limit: {body:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn search_sessions_malformed_agent_id_is_silently_ignored() {
    // The handler parses `agent_id` with `Uuid::parse_str(...).ok()`, so a
    // garbage value falls through to `None` rather than 400. Pin the
    // behaviour so a future refactor that suddenly starts rejecting it
    // doesn't break dashboard callers that rely on the lenient shape.
    let h = boot().await;
    let (status, body) = get_json(&h, "/api/sessions/search?q=anything&agent_id=not-a-uuid").await;
    assert_eq!(status, StatusCode::OK, "{body:?}");
    assert!(body["results"].is_array(), "{body:?}");
}
