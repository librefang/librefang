//! #6630 — `GET /api/mcp/servers` (and the `{name}` detail route) must never serialize MCP environment *values*.
//!
//! `McpServerConfigEntry::env` is documented as a list of variable names to pass through, but the supported representation also accepts an inline `KEY=VALUE`, so an operator can put a live credential there.
//! Both read routes used to return the raw list.
//! The report covers the Viewer-role case; it is worse than that — `/api/mcp/servers` sits in `PUBLIC_ROUTES_DASHBOARD_READS`, so with `require_auth_for_reads` unset (the default) an unauthenticated caller reads it too.
//!
//! Redacting the read side alone would have been a data-loss bug worse than the disclosure, because `McpServersPage` hydrates its edit form from the list response and submits every field back on save.
//! So the write path merges a bare `NAME` against what is stored, and that round-trip is pinned here too.

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use librefang_api::routes::{self, AppState};
use librefang_testing::{MockKernelBuilder, TestAppState};
use librefang_types::config::McpRuntimeStore;
use std::sync::Arc;
use tower::ServiceExt;

/// Distinctive enough that a substring search over a whole response body is a meaningful assertion — no chance of a coincidental match.
const SENTINEL: &str = "ghp_S3nt1nelMustNeverAppearInAnyResponseBody";
const ENV_NAME: &str = "GITHUB_PERSONAL_ACCESS_TOKEN";
/// A second variable with no inline value: the name-only form must survive untouched, which is what the dashboard's notice ("referenced by name only") describes.
const PLAIN_ENV_NAME: &str = "GITHUB_API_URL";

struct Harness {
    app: axum::Router,
    state: Arc<AppState>,
    _test: TestAppState,
}

/// DB-backed store so the test can read the persisted entry directly and prove the inline value is still there after a round-trip.
fn boot() -> Harness {
    let test = TestAppState::with_builder(MockKernelBuilder::new().with_config(|cfg| {
        cfg.mcp_runtime_store = McpRuntimeStore::Db;
    }));
    let state = test.state.clone();
    let app = routes::skills::router().with_state(state.clone());
    Harness {
        app,
        state,
        _test: test,
    }
}

async fn send(app: axum::Router, req: Request<Body>) -> (StatusCode, String) {
    let resp = app.oneshot(req).await.expect("router response");
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), 1 << 20)
        .await
        .expect("read body");
    (status, String::from_utf8_lossy(&bytes).to_string())
}

fn get(uri: &str) -> Request<Body> {
    Request::builder()
        .method(Method::GET)
        .uri(uri)
        .body(Body::empty())
        .unwrap()
}

fn put_json(uri: &str, body: serde_json::Value) -> Request<Body> {
    Request::builder()
        .method(Method::PUT)
        .uri(uri)
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap()
}

/// `false` exits immediately, so the connect attempt fails fast while the entry is still persisted and added to the effective set.
fn add_server_with_inline_secret(name: &str) -> Request<Body> {
    let body = serde_json::json!({
        "name": name,
        "transport": { "type": "stdio", "command": "false", "args": [] },
        "env": [format!("{ENV_NAME}={SENTINEL}"), PLAIN_ENV_NAME],
    });
    Request::builder()
        .method(Method::POST)
        .uri("/mcp/servers")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap()
}

fn stored_env(state: &Arc<AppState>, name: &str) -> Vec<String> {
    let store = librefang_memory::McpConfigStore::new(state.kernel.memory_substrate().pool());
    store
        .get(name)
        .expect("store read")
        .expect("entry persisted")
        .env
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn list_and_detail_never_disclose_inline_env_values() {
    let h = boot();
    let (status, _) = send(h.app.clone(), add_server_with_inline_secret("leaky")).await;
    assert_eq!(status, StatusCode::CREATED, "add should succeed");

    // Precondition: the inline value really is stored, so the assertions below are about the response and not about the value never existing.
    assert!(
        stored_env(&h.state, "leaky").contains(&format!("{ENV_NAME}={SENTINEL}")),
        "precondition: the inline value must be persisted for this test to mean anything"
    );

    for uri in ["/mcp/servers", "/mcp/servers/leaky"] {
        let (status, body) = send(h.app.clone(), get(uri)).await;
        assert_eq!(status, StatusCode::OK, "GET {uri} failed: {body}");
        assert!(
            !body.contains(SENTINEL),
            "GET {uri} disclosed the inline env value:\n{body}"
        );
        // The name is the useful, non-secret half and must survive — the dashboard renders it and an operator needs to know what the server expects.
        assert!(
            body.contains(ENV_NAME),
            "GET {uri} must still report the variable NAME:\n{body}"
        );
        assert!(
            body.contains(PLAIN_ENV_NAME),
            "GET {uri} must still report name-only entries:\n{body}"
        );
        // Guard the specific shape a naive fix produces: `KEY=***` still round-trips into the stored config and destroys the real value.
        assert!(
            !body.contains(&format!("{ENV_NAME}=")),
            "GET {uri} must return the bare name, not a masked `KEY=...` form \
             (a masked value round-trips back into the config):\n{body}"
        );
    }
}

/// The other half of the contract: a client that hydrates its form from the redacted list response and submits every field back must not wipe the inline value it was never shown.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn round_tripping_the_redacted_env_preserves_the_stored_inline_value() {
    let h = boot();
    let (status, _) = send(h.app.clone(), add_server_with_inline_secret("roundtrip")).await;
    assert_eq!(status, StatusCode::CREATED);

    // Read back exactly what a client sees, then submit it verbatim with one unrelated field changed — the dashboard's save flow.
    let (_, list) = send(h.app.clone(), get("/mcp/servers/roundtrip")).await;
    let detail: serde_json::Value = serde_json::from_str(&list).expect("detail is JSON");
    let env_from_response = detail
        .get("env")
        .and_then(|e| e.as_array())
        .expect("env array in detail response")
        .clone();
    assert_eq!(
        env_from_response,
        serde_json::json!([ENV_NAME, PLAIN_ENV_NAME])
            .as_array()
            .unwrap()
            .clone(),
        "the client sees names only"
    );

    let (status, body) = send(
        h.app.clone(),
        put_json(
            "/mcp/servers/roundtrip",
            serde_json::json!({
                "name": "roundtrip",
                "transport": { "type": "stdio", "command": "false", "args": [] },
                "timeout_secs": 99,
                "env": env_from_response,
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "update failed: {body}");

    let stored = stored_env(&h.state, "roundtrip");
    assert!(
        stored.contains(&format!("{ENV_NAME}={SENTINEL}")),
        "the inline value must survive a round-trip through the redacted \
         response — otherwise redaction silently destroys the credential and \
         the server stops connecting for a reason the UI cannot explain. \
         stored: {stored:?}"
    );
    assert!(
        stored.iter().any(|e| e == PLAIN_ENV_NAME),
        "name-only entries must survive unchanged. stored: {stored:?}"
    );
}

/// A submitted `NAME=value` is an explicit change and must win over what is stored, otherwise an operator could never rotate an inline credential.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_explicitly_submitted_value_overrides_the_stored_one() {
    let h = boot();
    let (status, _) = send(h.app.clone(), add_server_with_inline_secret("rotate")).await;
    assert_eq!(status, StatusCode::CREATED);

    let (status, body) = send(
        h.app.clone(),
        put_json(
            "/mcp/servers/rotate",
            serde_json::json!({
                "name": "rotate",
                "transport": { "type": "stdio", "command": "false", "args": [] },
                "env": [format!("{ENV_NAME}=ghp_rotated_value"), PLAIN_ENV_NAME],
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "update failed: {body}");

    let stored = stored_env(&h.state, "rotate");
    assert!(
        stored.contains(&format!("{ENV_NAME}=ghp_rotated_value")),
        "an explicit new value must replace the stored one. stored: {stored:?}"
    );
    assert!(
        !stored.iter().any(|e| e.contains(SENTINEL)),
        "the old value must be gone after an explicit rotation. stored: {stored:?}"
    );
}

/// A name dropped from the submission is an explicit removal, not something to restore from the stored entry.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn omitting_a_name_removes_it_rather_than_restoring_it() {
    let h = boot();
    let (status, _) = send(h.app.clone(), add_server_with_inline_secret("remove")).await;
    assert_eq!(status, StatusCode::CREATED);

    let (status, body) = send(
        h.app.clone(),
        put_json(
            "/mcp/servers/remove",
            serde_json::json!({
                "name": "remove",
                "transport": { "type": "stdio", "command": "false", "args": [] },
                "env": [PLAIN_ENV_NAME],
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "update failed: {body}");

    let stored = stored_env(&h.state, "remove");
    assert!(
        !stored.iter().any(|e| e.starts_with(ENV_NAME)),
        "a name absent from the submission must be removed, not merged back. \
         stored: {stored:?}"
    );
    assert_eq!(stored, vec![PLAIN_ENV_NAME.to_string()]);
}
