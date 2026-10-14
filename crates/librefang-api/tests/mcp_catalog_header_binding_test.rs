//! #8578 — secret-backed HTTP headers for MCP catalog entries, through the real `POST/PUT /api/mcp/servers` routes.
//!
//! A catalog entry can declare `[[headers]] name = "Authorization", value = "Bearer ${KEY}"` next to `[[required_env]] name = "KEY"`.
//! Installing it must persist the reference, never the secret, must make the credential resolvable without a daemon restart, and must not lose the binding when the server is later edited through a client that never saw it.

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use librefang_api::routes::{self, AppState};
use librefang_testing::{MockKernelBuilder, TestAppState};
use librefang_types::config::McpRuntimeStore;
use std::sync::Arc;
use tower::ServiceExt;

/// Process-unique so exporting it into the test process environment cannot collide with any other test.
const KEY_NAME: &str = "LIBREFANG_TEST_8578_ROUTE_API_KEY";
const SECRET: &str = "sk_8578_routeSecretMustNeverBePersistedOrServed";
const TEMPLATE_HEADER: &str = "Authorization: Bearer ${LIBREFANG_TEST_8578_ROUTE_API_KEY}";

struct Harness {
    app: axum::Router,
    state: Arc<AppState>,
    _test: TestAppState,
}

/// DB-backed store so the test can read the persisted entry back directly.
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

fn stored_entry(
    state: &Arc<AppState>,
    name: &str,
) -> librefang_types::config::McpServerConfigEntry {
    let store = librefang_memory::McpConfigStore::new(state.kernel.memory_substrate().pool());
    store
        .get(name)
        .expect("store read")
        .expect("entry persisted")
}

/// Write a secret-backed HTTP template into the catalog directory before the kernel boots, so the boot-time catalog load picks it up the way it picks up a registry-synced entry.
/// The URL is `.invalid`, so the background connect after install fails fast without leaving the machine.
fn seed_catalog(home: &std::path::Path) {
    let dir = home.join("mcp").join("catalog");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("remote-8578.toml"),
        format!(
            r#"
id = "remote-8578"
name = "Remote 8578"
description = "Secret-backed Streamable HTTP fixture"
category = "ai"

[transport]
type = "http"
url = "https://mcp.example.invalid/mcp"

[[required_env]]
name = "{KEY_NAME}"
label = "API Key"
help = "Use your own key"

[[headers]]
name = "Authorization"
value = "Bearer ${{{KEY_NAME}}}"
"#
        ),
    )
    .unwrap();
}

/// DB-backed store with the catalog fixture in place at boot.
fn boot_with_catalog() -> Harness {
    let test = TestAppState::with_builder(MockKernelBuilder::new().with_config(|cfg| {
        cfg.mcp_runtime_store = McpRuntimeStore::Db;
        seed_catalog(&cfg.home_dir);
    }));
    let state = test.state.clone();
    let app = routes::skills::router().with_state(state.clone());
    Harness {
        app,
        state,
        _test: test,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn template_install_persists_the_header_reference_and_exports_the_credential() {
    let h = boot_with_catalog();

    let (status, body) = send(
        h.app.clone(),
        json_request(
            Method::POST,
            "/mcp/servers",
            serde_json::json!({
                "template_id": "remote-8578",
                "credentials": { KEY_NAME: SECRET },
            }),
        ),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "template install failed: {body}"
    );
    assert!(
        !body.contains(SECRET),
        "install response leaked the secret: {body}"
    );

    let stored = stored_entry(&h.state, "remote-8578");
    assert_eq!(stored.template_id.as_deref(), Some("remote-8578"));
    assert_eq!(
        stored.headers,
        vec![TEMPLATE_HEADER.to_string()],
        "the installed server must carry the catalog's header binding"
    );
    assert_eq!(stored.env, vec![KEY_NAME.to_string()]);
    let persisted = serde_json::to_string(&stored).unwrap();
    assert!(
        !persisted.contains(SECRET),
        "the persisted server config must hold the reference, not the secret: {persisted}"
    );

    // The connect path resolves `${KEY}` from the daemon environment; the credential has to be there now, not after the next restart.
    assert_eq!(
        std::env::var(KEY_NAME).ok().as_deref(),
        Some(SECRET),
        "the declared credential must be resolvable by the connect that follows the install"
    );

    for uri in ["/mcp/servers", "/mcp/servers/remote-8578"] {
        let (status, body) = send(h.app.clone(), get(uri)).await;
        assert_eq!(status, StatusCode::OK, "GET {uri} failed: {body}");
        assert!(
            !body.contains(SECRET),
            "GET {uri} leaked the secret: {body}"
        );
    }
}

/// Editing a server through a client that hydrated from the read routes — which never return `headers` — must keep its header binding, while an explicit empty list still removes it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn editing_a_server_without_a_headers_key_keeps_its_header_binding() {
    let h = boot();

    let (status, body) = send(
        h.app.clone(),
        json_request(
            Method::POST,
            "/mcp/servers",
            serde_json::json!({
                "name": "bound",
                "transport": { "type": "http", "url": "https://mcp.example.invalid/mcp" },
                "env": [KEY_NAME],
                "headers": [TEMPLATE_HEADER],
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "add failed: {body}");

    // The dashboard's save: every field it was shown, one of them changed, no `headers` key.
    let (status, body) = send(
        h.app.clone(),
        json_request(
            Method::PUT,
            "/mcp/servers/bound",
            serde_json::json!({
                "name": "bound",
                "transport": { "type": "http", "url": "https://mcp.example.invalid/mcp" },
                "timeout_secs": 90,
                "env": [KEY_NAME],
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "update failed: {body}");
    let stored = stored_entry(&h.state, "bound");
    assert_eq!(stored.timeout_secs, 90, "the update must have taken effect");
    assert_eq!(
        stored.headers,
        vec![TEMPLATE_HEADER.to_string()],
        "an omitted `headers` key must leave the stored binding in place"
    );

    // An explicit empty list is a deliberate removal.
    let (status, body) = send(
        h.app.clone(),
        json_request(
            Method::PUT,
            "/mcp/servers/bound",
            serde_json::json!({
                "name": "bound",
                "transport": { "type": "http", "url": "https://mcp.example.invalid/mcp" },
                "env": [KEY_NAME],
                "headers": [],
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "clearing update failed: {body}");
    assert!(
        stored_entry(&h.state, "bound").headers.is_empty(),
        "an explicit empty `headers` list must clear the binding"
    );
}
