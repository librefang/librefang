//! HTTP regression for the per-agent knowledge-graph relations endpoint.
//!
//! `GET /api/memory/agents/{id}/relations` previously discarded the path
//! agent id (`Path(_agent_id)`) and queried the graph with no agent scope,
//! so any authenticated caller received up to 100 relation triples across
//! ALL agents regardless of the id in the URL — a cross-agent data leak.
//!
//! This test stores a private triple for two different agents through the
//! real router, then asserts each agent's relations endpoint returns only
//! that agent's triple and never the other's.

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use librefang_api::server;
use librefang_kernel::LibreFangKernel;
use librefang_types::config::{DefaultModelConfig, KernelConfig};
use std::sync::Arc;
use tower::ServiceExt;

struct RouterHarness {
    app: axum::Router,
    _tmp: tempfile::TempDir,
    _state: Arc<librefang_api::routes::AppState>,
}

impl Drop for RouterHarness {
    fn drop(&mut self) {
        self._state.kernel.shutdown();
    }
}

async fn boot_router_with_api_key(api_key: &str) -> RouterHarness {
    let tmp = tempfile::tempdir().expect("tempdir");

    // Seed the pinned registry fixture so the kernel boots with content, offline.
    librefang_kernel::registry_sync::seed_registry_fixture_for_tests(tmp.path());

    let config = KernelConfig {
        home_dir: tmp.path().to_path_buf(),
        data_dir: tmp.path().join("data"),
        api_key: api_key.to_string(),
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

    let kernel = LibreFangKernel::boot_with_config(config).expect("kernel boot");
    let kernel = Arc::new(kernel);
    kernel.set_self_handle();

    let (app, state) = server::build_router(kernel, "127.0.0.1:0".parse().expect("addr")).await;

    RouterHarness {
        app,
        _tmp: tmp,
        _state: state,
    }
}

const TEST_KEY: &str = "test-secret-relations-acl-key";

fn authed_get(path: &str) -> Request<Body> {
    Request::builder()
        .method(Method::GET)
        .uri(path)
        .header("authorization", format!("Bearer {TEST_KEY}"))
        .body(Body::empty())
        .unwrap()
}

fn authed_json(method: Method, path: &str, body: serde_json::Value) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(path)
        .header("authorization", format!("Bearer {TEST_KEY}"))
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap()
}

async fn read_json(resp: axum::response::Response) -> serde_json::Value {
    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .expect("read body");
    serde_json::from_slice(&body).expect("body is JSON")
}

/// Collect every entity name (source + target) appearing in a `matches` array.
fn match_names(body: &serde_json::Value) -> Vec<String> {
    body["matches"]
        .as_array()
        .expect("matches array")
        .iter()
        .flat_map(|m| {
            [
                m["source"]["name"].as_str().unwrap_or("").to_string(),
                m["target"]["name"].as_str().unwrap_or("").to_string(),
            ]
        })
        .collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn relations_endpoint_scopes_to_path_agent_and_never_cross_leaks() {
    let harness = boot_router_with_api_key(TEST_KEY).await;

    // Agent A stores a private triple: Alice works at AcmeCorp.
    let resp = harness
        .app
        .clone()
        .oneshot(authed_json(
            Method::POST,
            "/api/memory/agents/agent-a/relations",
            serde_json::json!([{
                "subject": "Alice",
                "subject_type": "person",
                "relation": "works_at",
                "object": "AcmeCorp",
                "object_type": "organization"
            }]),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // Agent B stores a different private triple: Bob works at GlobexInc.
    let resp = harness
        .app
        .clone()
        .oneshot(authed_json(
            Method::POST,
            "/api/memory/agents/agent-b/relations",
            serde_json::json!([{
                "subject": "Bob",
                "subject_type": "person",
                "relation": "works_at",
                "object": "GlobexInc",
                "object_type": "organization"
            }]),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // Agent B's endpoint must return only Bob→GlobexInc, never Alice→AcmeCorp.
    let resp = harness
        .app
        .clone()
        .oneshot(authed_get("/api/memory/agents/agent-b/relations"))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = read_json(resp).await;
    assert_eq!(
        body["count"],
        serde_json::json!(1),
        "agent B endpoint must return exactly its own triple, got: {body}"
    );
    let names = match_names(&body);
    assert!(
        names.contains(&"Bob".to_string()) && names.contains(&"GlobexInc".to_string()),
        "expected agent B's own triple (Bob/GlobexInc), got: {body}"
    );
    assert!(
        !names.contains(&"Alice".to_string()) && !names.contains(&"AcmeCorp".to_string()),
        "agent B endpoint leaked agent A's triple: {body}"
    );

    // Agent A's endpoint must return only Alice→AcmeCorp, never Bob→GlobexInc.
    let resp = harness
        .app
        .clone()
        .oneshot(authed_get("/api/memory/agents/agent-a/relations"))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = read_json(resp).await;
    assert_eq!(
        body["count"],
        serde_json::json!(1),
        "agent A endpoint must return exactly its own triple, got: {body}"
    );
    let names = match_names(&body);
    assert!(
        names.contains(&"Alice".to_string()) && names.contains(&"AcmeCorp".to_string()),
        "expected agent A's own triple (Alice/AcmeCorp), got: {body}"
    );
    assert!(
        !names.contains(&"Bob".to_string()) && !names.contains(&"GlobexInc".to_string()),
        "agent A endpoint leaked agent B's triple: {body}"
    );

    // An empty path id must be rejected rather than silently scanning all agents.
    let resp = harness
        .app
        .clone()
        .oneshot(authed_get("/api/memory/agents/%20/relations"))
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::BAD_REQUEST,
        "whitespace-only agent id must be a 400, not an unscoped scan"
    );
}
