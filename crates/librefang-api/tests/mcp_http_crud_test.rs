//! Phase 9 (C-005b): route-level HTTP coverage for the MCP CRUD endpoints
//! against the real full router, asserting writes land in the DB config store
//! (config.toml is never touched) and are visible via GET.
#![cfg(feature = "surreal-backend")]

use axum::body::Body;
use axum::http::{Request, StatusCode};
use librefang_kernel::LibreFangKernel;
use librefang_storage::StorageConfig;
use librefang_types::config::{DefaultModelConfig, KernelConfig};
use std::sync::Arc;
use tower::ServiceExt;

async fn json(resp: axum::response::Response) -> (StatusCode, serde_json::Value) {
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), 1 << 20)
        .await
        .unwrap();
    let v = if bytes.is_empty() {
        serde_json::Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null)
    };
    (status, v)
}

#[tokio::test(flavor = "multi_thread")]
async fn mcp_crud_round_trips_through_the_store() {
    // `oneshot` requests carry no ConnectInfo, so the auth middleware treats
    // them as non-loopback. This is the single test in this binary, so the
    // documented no-auth opt-out is safe to set process-wide here.
    std::env::set_var("LIBREFANG_ALLOW_NO_AUTH", "1");

    let tmp = tempfile::tempdir().unwrap();
    let storage = StorageConfig::embedded_default(tmp.path().join("operational"));
    let config = KernelConfig {
        home_dir: tmp.path().to_path_buf(),
        data_dir: tmp.path().join("data"),
        api_key: String::new(), // no auth → loopback oneshot is allowed
        storage,
        mcp_servers: Vec::new(),
        default_model: DefaultModelConfig {
            provider: "ollama".to_string(),
            model: "test-model".to_string(),
            api_key_env: "OLLAMA_API_KEY".to_string(),
            base_url: None,
            message_timeout_secs: 300,
            extra_params: std::collections::BTreeMap::new(),
            cli_profile_dirs: Vec::new(),
        },
        ..KernelConfig::default()
    };
    let kernel = Arc::new(LibreFangKernel::boot_with_config(config).expect("kernel boots"));
    kernel.set_self_handle();
    let config_path = kernel.home_dir().join("config.toml");

    let (app, _state) =
        librefang_api::server::build_router(kernel, "127.0.0.1:0".parse().unwrap()).await;

    // POST /api/mcp/servers — add a raw entry.
    let add = Request::builder()
        .method("POST")
        .uri("/api/mcp/servers")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::json!({
                "name": "http-server",
                "transport": { "type": "sse", "url": "http://127.0.0.1:9" }
            })
            .to_string(),
        ))
        .unwrap();
    let (status, body) = json(app.clone().oneshot(add).await.unwrap()).await;
    assert_eq!(status, StatusCode::CREATED, "add failed: {body}");
    assert_eq!(body["status"], "added");

    // config.toml must NOT have been written (read-only-safe).
    assert!(
        !config_path.exists(),
        "MCP add must not write config.toml; it should go to the DB store"
    );

    // GET /api/mcp/servers — the new server is listed.
    let get = Request::builder()
        .uri("/api/mcp/servers")
        .body(Body::empty())
        .unwrap();
    let (status, body) = json(app.clone().oneshot(get).await.unwrap()).await;
    assert_eq!(status, StatusCode::OK);
    let names: Vec<&str> = body["configured"]
        .as_array()
        .expect("configured array")
        .iter()
        .filter_map(|s| s["name"].as_str())
        .collect();
    assert!(
        names.contains(&"http-server"),
        "GET must list the stored server; got {body}"
    );

    // DELETE /api/mcp/servers/http-server.
    let del = Request::builder()
        .method("DELETE")
        .uri("/api/mcp/servers/http-server")
        .body(Body::empty())
        .unwrap();
    let (status, body) = json(app.clone().oneshot(del).await.unwrap()).await;
    assert_eq!(status, StatusCode::OK, "delete failed: {body}");

    // GET again — gone.
    let get2 = Request::builder()
        .uri("/api/mcp/servers")
        .body(Body::empty())
        .unwrap();
    let (_, body) = json(app.clone().oneshot(get2).await.unwrap()).await;
    let names: Vec<&str> = body["configured"]
        .as_array()
        .map(|a| a.iter().filter_map(|s| s["name"].as_str()).collect())
        .unwrap_or_default();
    assert!(
        !names.contains(&"http-server"),
        "server must be gone after delete; got {body}"
    );
}
