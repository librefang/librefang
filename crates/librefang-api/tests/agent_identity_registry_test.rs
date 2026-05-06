//! Integration coverage for the canonical agent UUID registry — refs #4614.
//!
//! Verifies:
//! 1. spawn_agent registers a canonical UUID matching the agent's id;
//! 2. DELETE /api/agents/{id} without `?confirm=true` is rejected 409
//!    (preserving the registry); with confirm purges the binding;
//! 3. respawn after a confirmed delete picks up the same deterministic
//!    UUID via `AgentId::from_name` AND re-registers the binding;
//! 4. GET /api/agents/identities surfaces the registry contents;
//! 5. POST /api/agents/identities/{name}/reset gates on `?confirm=true`.
//!
//! These tests catch regressions at the API ↔ kernel ↔ identity-registry
//! boundary on every push (per CLAUDE.md / refs #3721 — integration
//! tests are the canonical replacement for the old curl checklist).

use axum::Router;
use librefang_api::{middleware, routes, ws};
use librefang_testing::{MockKernelBuilder, TestAppState};
use std::sync::Arc;
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;

struct TestServer {
    base_url: String,
    state: Arc<routes::AppState>,
    _tmp: tempfile::TempDir,
}

impl Drop for TestServer {
    fn drop(&mut self) {
        self.state.kernel.shutdown();
    }
}

async fn start_test_server() -> TestServer {
    let test = TestAppState::with_builder(MockKernelBuilder::new().with_config(|cfg| {
        cfg.default_model.provider = "ollama".to_string();
        cfg.default_model.model = "test-model".to_string();
        cfg.default_model.api_key_env = "OLLAMA_API_KEY".to_string();
    }));
    let config_path = test.tmp_path().join("config.toml");
    let test = test.with_config_path(config_path);
    let (state, _tmp, _) = test.into_parts();
    state.kernel.set_self_handle();

    let app = Router::new()
        .route(
            "/api/agents",
            axum::routing::get(routes::list_agents).post(routes::spawn_agent),
        )
        .route(
            "/api/agents/identities",
            axum::routing::get(routes::list_agent_identities),
        )
        .route(
            "/api/agents/identities/{name}/reset",
            axum::routing::post(routes::reset_agent_identity),
        )
        .route(
            "/api/agents/{id}",
            axum::routing::get(routes::get_agent).delete(routes::kill_agent),
        )
        .route("/api/agents/{id}/ws", axum::routing::get(ws::agent_ws))
        .layer(axum::middleware::from_fn(middleware::request_logging))
        .layer(TraceLayer::new_for_http())
        .layer(CorsLayer::permissive())
        .with_state(state.clone());

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test server");
    let addr = listener.local_addr().unwrap();

    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    TestServer {
        base_url: format!("http://{}", addr),
        state,
        _tmp,
    }
}

const TEST_MANIFEST: &str = r#"
name = "respawn-target"
version = "0.1.0"
description = "Integration test agent (refs #4614)"
author = "test"
module = "builtin:chat"

[model]
provider = "ollama"
model = "test-model"
system_prompt = "You are a test agent."

[capabilities]
memory_read = ["*"]
memory_write = ["self.*"]
"#;

async fn spawn_one(server: &TestServer, manifest: &str) -> String {
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/api/agents", server.base_url))
        .json(&serde_json::json!({"manifest_toml": manifest}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 201, "spawn must return 201");
    let body: serde_json::Value = resp.json().await.unwrap();
    body["agent_id"].as_str().unwrap().to_string()
}

#[tokio::test(flavor = "multi_thread")]
async fn spawn_registers_canonical_uuid() {
    let server = start_test_server().await;
    let agent_id = spawn_one(&server, TEST_MANIFEST).await;

    let recorded = server
        .state
        .kernel
        .agent_identities()
        .get("respawn-target")
        .expect("registry must record the canonical UUID");
    assert_eq!(recorded.to_string(), agent_id);
}

#[tokio::test(flavor = "multi_thread")]
async fn delete_without_confirm_returns_409_and_preserves_identity() {
    let server = start_test_server().await;
    let agent_id = spawn_one(&server, TEST_MANIFEST).await;

    let client = reqwest::Client::new();
    let resp = client
        .delete(format!("{}/api/agents/{}", server.base_url, agent_id))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 409, "bare DELETE must be 409 (refs #4614)");
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["code"], "delete_confirmation_required");
    let err = body["error"]["message"]
        .as_str()
        .or_else(|| body["message"].as_str())
        .unwrap_or_default();
    assert!(
        err.contains("canonical UUID") && err.contains("cannot be undone"),
        "warning text must mention canonical UUID + data-loss; got: {err}"
    );
    assert!(
        server
            .state
            .kernel
            .agent_identities()
            .get("respawn-target")
            .is_some(),
        "rejected DELETE must NOT purge the canonical UUID"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn delete_with_confirm_purges_identity_and_respawn_recovers_uuid() {
    let server = start_test_server().await;
    let first_id = spawn_one(&server, TEST_MANIFEST).await;

    let client = reqwest::Client::new();
    let resp = client
        .delete(format!(
            "{}/api/agents/{}?confirm=true",
            server.base_url, first_id
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["status"], "killed");
    assert_eq!(body["identity_purged"], true);
    assert!(
        server
            .state
            .kernel
            .agent_identities()
            .get("respawn-target")
            .is_none(),
        "confirmed DELETE must purge the canonical UUID"
    );

    // Respawn re-derives via `AgentId::from_name` and re-registers the
    // binding. The v5 derivation is deterministic for a fixed name, so
    // the recovered UUID equals the original — sessions / memories tied
    // to the prior life cycle were already removed by `kill_agent`'s
    // `memory.remove_agent` call. The point of the registry is to
    // survive *non-explicit* lifecycle resets (panic restart, hot
    // reload, manifest reload).
    let second_id = spawn_one(&server, TEST_MANIFEST).await;
    assert_eq!(
        first_id, second_id,
        "deterministic from_name yields the same UUID after a clean re-register"
    );
    assert!(
        server
            .state
            .kernel
            .agent_identities()
            .get("respawn-target")
            .is_some(),
        "fresh spawn must re-register the canonical UUID"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn delete_invalid_uuid_short_circuits_400_before_confirm_check() {
    // Refs #4614: malformed UUID is still 400 (not 409), since the
    // parse failure happens before the confirm check fires.
    let server = start_test_server().await;
    let client = reqwest::Client::new();
    let resp = client
        .delete(format!("{}/api/agents/not-a-uuid", server.base_url))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);
}

#[tokio::test(flavor = "multi_thread")]
async fn list_identities_returns_registered_entries() {
    let server = start_test_server().await;
    let agent_id = spawn_one(&server, TEST_MANIFEST).await;

    let client = reqwest::Client::new();
    let resp = client
        .get(format!("{}/api/agents/identities", server.base_url))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    let arr = body.as_array().expect("response must be a JSON array");
    let row = arr
        .iter()
        .find(|r| r["name"] == "respawn-target")
        .expect("registry must contain the spawned agent");
    assert_eq!(row["canonical_uuid"].as_str().unwrap(), agent_id);
    assert!(row["created_at"].as_str().is_some());
}

#[tokio::test(flavor = "multi_thread")]
async fn reset_identity_endpoint_gates_on_confirm() {
    let server = start_test_server().await;
    let _agent_id = spawn_one(&server, TEST_MANIFEST).await;
    let client = reqwest::Client::new();

    // Bare reset → 409
    let resp = client
        .post(format!(
            "{}/api/agents/identities/respawn-target/reset",
            server.base_url
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 409);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["code"], "reset_identity_unconfirmed");

    // Confirmed reset → 200, binding gone
    let resp = client
        .post(format!(
            "{}/api/agents/identities/respawn-target/reset?confirm=true",
            server.base_url
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["status"], "reset");
    assert!(body["previous_canonical_uuid"].as_str().is_some());
    assert!(server
        .state
        .kernel
        .agent_identities()
        .get("respawn-target")
        .is_none());

    // Reset on a now-missing name → 404
    let resp = client
        .post(format!(
            "{}/api/agents/identities/respawn-target/reset?confirm=true",
            server.base_url
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 404);
}
