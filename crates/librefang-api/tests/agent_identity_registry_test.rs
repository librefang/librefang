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
//!
//! Harness: the full production router (`server::build_router`) served on a
//! real loopback socket — NOT the handler-only mock `start_test_server`.
//! The full router wires the auth, rate-limit, idempotency, body-size, and
//! error-envelope layers that the mock silently dropped, so these tests now
//! exercise the same middleware stack a live daemon does. Empty `api_key` +
//! a loopback peer keeps the single-user dev fast-path open; booting the same
//! harness with a non-empty key (`start_full_router("<key>")`) proves the auth
//! layer is active (see `auth_layer_enforced_on_identity_routes`).

use librefang_kernel::LibreFangKernel;
use librefang_types::config::{DefaultModelConfig, KernelConfig};
use std::net::SocketAddr;
use std::sync::Arc;

struct TestServer {
    base_url: String,
    state: Arc<librefang_api::routes::AppState>,
    _tmp: tempfile::TempDir,
}

impl Drop for TestServer {
    fn drop(&mut self) {
        self.state.kernel.shutdown();
    }
}

/// Boot the real kernel and serve the full production router on a random
/// loopback port. `api_key` empty → single-user dev fast-path (loopback
/// peers pass auth without a token); non-empty → token auth is enforced.
async fn start_full_router(api_key: &str) -> TestServer {
    let tmp = tempfile::tempdir().expect("create temp dir");

    // Seed the pinned registry fixture into the temp home_dir so the kernel boots with a populated model catalog (mirrors api_integration_test::start_full_router).
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

    let kernel = Arc::new(LibreFangKernel::boot_with_config(config).expect("kernel should boot"));
    kernel.set_self_handle();

    let (app, state) = librefang_api::server::build_router(
        kernel,
        "127.0.0.1:0".parse().expect("listen addr should parse"),
    )
    .await;

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test server");
    let addr = listener.local_addr().unwrap();

    // SECURITY: `into_make_service_with_connect_info` injects the peer
    // SocketAddr the auth layer reads to classify loopback vs. non-loopback
    // callers — the same wiring production uses (`server.rs`). Without it the
    // empty-key fast-path would fail closed (missing ConnectInfo → 401).
    tokio::spawn(async move {
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .unwrap();
    });

    TestServer {
        base_url: format!("http://{}", addr),
        state,
        _tmp: tmp,
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
    let server = start_full_router("").await;
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
    let server = start_full_router("").await;
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
    let server = start_full_router("").await;
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
    let server = start_full_router("").await;
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
    let server = start_full_router("").await;
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
    let server = start_full_router("").await;
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

/// Real-middleware coverage the mock router could not provide: with a
/// configured `api_key` the full router's auth layer rejects unauthenticated
/// writes to the identity domain (even from loopback — the unconditional
/// loopback bypass was removed, see middleware bug #3558), and accepts the
/// same request once a valid bearer token is supplied. The handler-only
/// `start_test_server` mock had no auth layer at all, so this entire class of
/// regression was invisible to it.
#[tokio::test(flavor = "multi_thread")]
async fn auth_layer_enforced_on_identity_routes() {
    const KEY: &str = "identity-registry-test-key";
    let server = start_full_router(KEY).await;
    let client = reqwest::Client::new();

    // Spawn requires auth — without a token the write is rejected 401, never
    // reaching the handler (so no identity is registered).
    let unauth_spawn = client
        .post(format!("{}/api/agents", server.base_url))
        .json(&serde_json::json!({"manifest_toml": TEST_MANIFEST}))
        .send()
        .await
        .unwrap();
    assert_eq!(
        unauth_spawn.status(),
        401,
        "unauthenticated spawn must be rejected by the auth layer"
    );
    assert!(
        server
            .state
            .kernel
            .agent_identities()
            .get("respawn-target")
            .is_none(),
        "a 401'd spawn must not have registered a canonical UUID"
    );

    // With the correct bearer token the same spawn succeeds and registers.
    let authed_spawn = client
        .post(format!("{}/api/agents", server.base_url))
        .header("authorization", format!("Bearer {KEY}"))
        .json(&serde_json::json!({"manifest_toml": TEST_MANIFEST}))
        .send()
        .await
        .unwrap();
    assert_eq!(
        authed_spawn.status(),
        201,
        "authenticated spawn must succeed through the full router"
    );
    let agent_id = authed_spawn.json::<serde_json::Value>().await.unwrap()["agent_id"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(
        server
            .state
            .kernel
            .agent_identities()
            .get("respawn-target")
            .is_some(),
        "authenticated spawn must register the canonical UUID"
    );

    // Identity-reset is a write too — unauthenticated POST is 401, the binding
    // survives, and the authenticated retry purges it.
    let unauth_reset = client
        .post(format!(
            "{}/api/agents/identities/respawn-target/reset?confirm=true",
            server.base_url
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(
        unauth_reset.status(),
        401,
        "unauthenticated identity reset must be rejected by the auth layer"
    );
    assert!(
        server
            .state
            .kernel
            .agent_identities()
            .get("respawn-target")
            .is_some(),
        "a 401'd reset must not have purged the canonical UUID"
    );

    let authed_reset = client
        .post(format!(
            "{}/api/agents/identities/respawn-target/reset?confirm=true",
            server.base_url
        ))
        .header("authorization", format!("Bearer {KEY}"))
        .send()
        .await
        .unwrap();
    assert_eq!(
        authed_reset.status(),
        200,
        "authenticated identity reset must succeed through the full router"
    );

    // GET /api/agents/identities is NOT in the dashboard-reads allowlist
    // (only the exact `/api/agents` GET is), so a configured key gates it even
    // for reads: unauthenticated → 401, authenticated → 200. This is precisely
    // the enumeration surface the mock router left wide open.
    let unauth_list = client
        .get(format!("{}/api/agents/identities", server.base_url))
        .send()
        .await
        .unwrap();
    assert_eq!(
        unauth_list.status(),
        401,
        "identity listing is not a public read and must require auth when a key is configured"
    );

    let authed_list = client
        .get(format!("{}/api/agents/identities", server.base_url))
        .header("authorization", format!("Bearer {KEY}"))
        .send()
        .await
        .unwrap();
    assert_eq!(
        authed_list.status(),
        200,
        "authenticated identity listing must succeed"
    );

    let _ = agent_id;
}
