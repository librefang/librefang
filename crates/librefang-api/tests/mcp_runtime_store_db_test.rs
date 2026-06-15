//! #6113 — `mcp_runtime_store = "db"` routes `POST/DELETE /api/mcp/servers`
//! writes to the SQLite `mcp_server_configs` table instead of `config.toml`,
//! and the read path lists DB-backed servers via the effective set (file
//! `[[mcp_servers]]` merged with the DB overlay).
//!
//! This is what lets MCP servers be managed at runtime when `config.toml` is
//! read-only — the Kubernetes ConfigMap motivation behind #6021. The default
//! (`file`) behaviour is covered by the existing config.toml-backed paths and
//! is intentionally not exercised here.

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use librefang_api::routes::{self, AppState};
use librefang_testing::{MockKernelBuilder, TestAppState};
use librefang_types::config::McpRuntimeStore;
use std::sync::Arc;
use tower::ServiceExt;

struct Harness {
    app: axum::Router,
    state: Arc<AppState>,
    _test: TestAppState,
}

/// Boot a real kernel with `mcp_runtime_store = Db` and mount the skills
/// router (which owns `/mcp/servers`).
fn boot_db_mode() -> Harness {
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

async fn body_json(resp: axum::response::Response) -> serde_json::Value {
    let bytes = axum::body::to_bytes(resp.into_body(), 1 << 20)
        .await
        .expect("read body");
    serde_json::from_slice(&bytes).unwrap_or(serde_json::json!(null))
}

fn add_server_request(name: &str) -> Request<Body> {
    // `false` is a stdio command that exits immediately; the connect attempt
    // fails fast and gracefully, but the entry is still persisted and added to
    // the effective set (which `reload_mcp_servers` updates before connecting).
    let body = serde_json::json!({
        "name": name,
        "transport": { "type": "stdio", "command": "false", "args": [] },
    });
    Request::builder()
        .method(Method::POST)
        .uri("/mcp/servers")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn db_mode_post_persists_to_store_not_config_toml_and_lists_via_effective_set() {
    let h = boot_db_mode();
    let home = h.state.kernel.home_dir().to_path_buf();

    let resp = h
        .app
        .clone()
        .oneshot(add_server_request("db-srv"))
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::CREATED,
        "add should succeed in db mode"
    );

    // The SQLite store holds the row.
    let store = librefang_memory::McpConfigStore::new(h.state.kernel.memory_substrate().pool());
    assert!(
        store.get("db-srv").unwrap().is_some(),
        "entry must be persisted to the mcp_server_configs table"
    );

    // config.toml was NOT written with the server (the whole point of db mode).
    let cfg_toml = home.join("config.toml");
    if cfg_toml.exists() {
        let txt = std::fs::read_to_string(&cfg_toml).unwrap();
        assert!(
            !txt.contains("db-srv"),
            "db mode must not write the server to config.toml, got:\n{txt}"
        );
    }

    // GET lists it via the effective set (file + DB overlay).
    let req = Request::builder()
        .method(Method::GET)
        .uri("/mcp/servers")
        .body(Body::empty())
        .unwrap();
    let resp = h.app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    let listed = json
        .get("configured")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .any(|s| s.get("name").and_then(|n| n.as_str()) == Some("db-srv"))
        })
        .unwrap_or(false);
    assert!(
        listed,
        "a DB-backed server must be visible in the listing, got:\n{json}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn db_mode_delete_removes_from_store() {
    let h = boot_db_mode();

    let resp = h
        .app
        .clone()
        .oneshot(add_server_request("doomed"))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);

    let store = librefang_memory::McpConfigStore::new(h.state.kernel.memory_substrate().pool());
    assert!(store.get("doomed").unwrap().is_some());

    let req = Request::builder()
        .method(Method::DELETE)
        .uri("/mcp/servers/doomed")
        .body(Body::empty())
        .unwrap();
    let resp = h.app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK, "delete should succeed");

    assert!(
        store.get("doomed").unwrap().is_none(),
        "delete must remove the row from the DB store"
    );
}
