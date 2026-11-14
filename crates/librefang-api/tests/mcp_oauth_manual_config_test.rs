//! #8670 — the manual OAuth fallback (`[mcp_servers.oauth]`: `auth_url` / `token_url` / `client_id` / `scopes`) must be editable from the dashboard.
//!
//! Two things stood in the way besides the missing form fields.
//! The read routes never returned `oauth`, so a form could not hydrate it.
//! And `PUT` replaces the whole entry, so editing any other field of a server silently dropped a hand-written `oauth` block, exactly as headers did before #8578.
//!
//! An absent `oauth` key now means "unchanged", and an explicit `"oauth": null` clears it.

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use librefang_api::routes::{self, AppState};
use librefang_testing::{MockKernelBuilder, TestAppState};
use librefang_types::config::McpRuntimeStore;
use std::sync::Arc;
use tower::ServiceExt;

const NAME: &str = "oauth-fallback";

struct Harness {
    app: axum::Router,
    state: Arc<AppState>,
    _test: TestAppState,
}

/// DB-backed store so the test can read the persisted entry directly instead of trusting the response.
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

fn json_request(method: Method, uri: &str, body: serde_json::Value) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(uri)
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap()
}

fn get(uri: &str) -> Request<Body> {
    Request::builder()
        .method(Method::GET)
        .uri(uri)
        .body(Body::empty())
        .unwrap()
}

fn oauth_block() -> serde_json::Value {
    serde_json::json!({
        "auth_url": "https://example.invalid/plane-mcp/authorize",
        "token_url": "https://example.invalid/plane-mcp/token",
        "client_id": "client-from-dcr",
        "client_secret_env": "PLANE_CLIENT_SECRET",
        "scopes": ["read"],
        // Serialized even when empty, so the read routes echo it back.
        "user_scopes": [],
    })
}

/// `false` exits immediately, so the connect attempt fails fast while the entry is still persisted.
fn server_body(oauth: Option<serde_json::Value>, timeout_secs: u64) -> serde_json::Value {
    let mut body = serde_json::json!({
        "name": NAME,
        "transport": { "type": "stdio", "command": "false", "args": [] },
        "timeout_secs": timeout_secs,
    });
    if let Some(oauth) = oauth {
        body["oauth"] = oauth;
    }
    body
}

fn stored_oauth(state: &Arc<AppState>) -> Option<librefang_types::config::McpOAuthConfig> {
    let store = librefang_memory::McpConfigStore::new(state.kernel.memory_substrate().pool());
    store
        .get(NAME)
        .expect("store read")
        .expect("entry persisted")
        .oauth
}

async fn add_with_oauth(h: &Harness) {
    let (status, body) = send(
        h.app.clone(),
        json_request(
            Method::POST,
            "/mcp/servers",
            server_body(Some(oauth_block()), 30),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "add should succeed: {body}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn detail_and_list_return_the_manual_oauth_block() {
    let h = boot();
    add_with_oauth(&h).await;

    let (status, body) = send(h.app.clone(), get(&format!("/mcp/servers/{NAME}"))).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let detail: serde_json::Value = serde_json::from_str(&body).expect("detail json");
    assert_eq!(detail["oauth"], oauth_block(), "detail route: {body}");

    let (status, body) = send(h.app.clone(), get("/mcp/servers")).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let list: serde_json::Value = serde_json::from_str(&body).expect("list json");
    let entry = list["configured"]
        .as_array()
        .expect("configured array")
        .iter()
        .find(|s| s["name"] == NAME)
        .unwrap_or_else(|| panic!("server missing from list: {body}"));
    assert_eq!(entry["oauth"], oauth_block(), "list route: {body}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_server_without_oauth_reports_null() {
    let h = boot();
    let (status, body) = send(
        h.app.clone(),
        json_request(Method::POST, "/mcp/servers", server_body(None, 30)),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");

    let (_, body) = send(h.app.clone(), get(&format!("/mcp/servers/{NAME}"))).await;
    let detail: serde_json::Value = serde_json::from_str(&body).expect("detail json");
    assert!(detail["oauth"].is_null(), "expected null oauth: {body}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn put_without_the_oauth_key_keeps_the_stored_block() {
    let h = boot();
    add_with_oauth(&h).await;
    assert!(
        stored_oauth(&h.state).is_some(),
        "precondition: the oauth block must be persisted for this test to mean anything"
    );

    // Edit an unrelated field and omit `oauth`, as the dashboard form does when the operator never touched the OAuth fields.
    let (status, body) = send(
        h.app.clone(),
        json_request(
            Method::PUT,
            &format!("/mcp/servers/{NAME}"),
            server_body(None, 45),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let oauth = stored_oauth(&h.state).expect("oauth block must survive an unrelated edit");
    assert_eq!(oauth.client_id.as_deref(), Some("client-from-dcr"));
    assert_eq!(
        oauth.token_url.as_deref(),
        Some("https://example.invalid/plane-mcp/token")
    );
    assert_eq!(oauth.scopes, vec!["read".to_string()]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn put_with_a_new_oauth_block_replaces_it() {
    let h = boot();
    add_with_oauth(&h).await;

    let replacement = serde_json::json!({
        "auth_url": "https://example.invalid/other/authorize",
        "token_url": "https://example.invalid/other/token",
        "client_id": "other-client",
        "scopes": ["read", "write"],
    });
    let (status, body) = send(
        h.app.clone(),
        json_request(
            Method::PUT,
            &format!("/mcp/servers/{NAME}"),
            server_body(Some(replacement), 30),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let oauth = stored_oauth(&h.state).expect("replacement persisted");
    assert_eq!(oauth.client_id.as_deref(), Some("other-client"));
    assert_eq!(oauth.scopes, vec!["read".to_string(), "write".to_string()]);
    assert!(
        oauth.client_secret_env.is_none(),
        "a replacement is the whole block, not a merge"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn put_with_an_explicit_null_clears_the_oauth_block() {
    let h = boot();
    add_with_oauth(&h).await;

    let (status, body) = send(
        h.app.clone(),
        json_request(
            Method::PUT,
            &format!("/mcp/servers/{NAME}"),
            server_body(Some(serde_json::Value::Null), 30),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(
        stored_oauth(&h.state).is_none(),
        "an explicit null must remove the block"
    );
}
