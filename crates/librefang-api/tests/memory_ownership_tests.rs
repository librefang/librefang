//! Cross-tenant memory ownership isolation integration tests.
//!
//! Verifies that agent-scoped memory endpoints enforce ownership via
//! `require_agent_access()` → `check_account()`. When tenant-B requests an
//! agent owned by tenant-A, every endpoint must return 404 (not 403, to avoid
//! confirming agent existence).
//!
//! Run: cargo test -p librefang-api --test memory_ownership_tests -- --nocapture

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use librefang_api::routes::AppState;
use librefang_api::server;
use librefang_kernel::LibreFangKernel;
use librefang_types::agent::{
    AgentEntry, AgentId, AgentIdentity, AgentManifest, AgentMode, AgentState,
};
use librefang_types::config::{DefaultModelConfig, KernelConfig};
use std::sync::Arc;
use tower::ServiceExt;

// ---------------------------------------------------------------------------
// Test infrastructure (mirrors account_tests.rs)
// ---------------------------------------------------------------------------

struct MtHarness {
    app: Router,
    state: Arc<AppState>,
    _tmp: tempfile::TempDir,
}

impl Drop for MtHarness {
    fn drop(&mut self) {
        self.state.kernel.shutdown();
    }
}

impl MtHarness {
    /// Send a request through the router. Clones the router internally so the
    /// harness can be reused for multiple requests in the same test.
    async fn send(&self, req: Request<Body>) -> axum::response::Response {
        self.app.clone().oneshot(req).await.unwrap()
    }
}

/// Boot a full router with multi-tenant mode enabled.
async fn start_mt_router() -> MtHarness {
    let tmp = tempfile::tempdir().expect("Failed to create temp dir");

    let config = KernelConfig {
        home_dir: tmp.path().to_path_buf(),
        data_dir: tmp.path().join("data"),
        multi_tenant: true,
        default_model: DefaultModelConfig {
            provider: "ollama".to_string(),
            model: "test-model".to_string(),
            api_key_env: "OLLAMA_API_KEY".to_string(),
            base_url: None,
            message_timeout_secs: 300,
        },
        ..KernelConfig::default()
    };

    let kernel = LibreFangKernel::boot_with_config(config).expect("Kernel should boot");
    let kernel = Arc::new(kernel);
    kernel.set_self_handle();

    let (app, state) = server::build_router(
        kernel,
        "127.0.0.1:0".parse().expect("listen addr should parse"),
    )
    .await;

    MtHarness {
        app,
        state,
        _tmp: tmp,
    }
}

/// Register an agent owned by the given tenant in the kernel registry.
/// Returns the agent ID string so callers can embed it in URLs.
fn register_agent(h: &MtHarness, tenant: &str, name: &str) -> String {
    let agent_id = AgentId::new();
    let entry = AgentEntry {
        id: agent_id,
        account_id: Some(tenant.to_string()),
        name: name.to_string(),
        manifest: AgentManifest::default(),
        state: AgentState::Created,
        mode: AgentMode::default(),
        created_at: chrono::Utc::now(),
        last_active: chrono::Utc::now(),
        parent: None,
        children: vec![],
        session_id: Default::default(),
        source_toml_path: None,
        tags: vec![],
        identity: AgentIdentity::default(),
        onboarding_completed: false,
        onboarding_completed_at: None,
        is_hand: false,
    };
    let _ = h.state.kernel.agent_registry().register(entry);
    agent_id.to_string()
}

// ---------------------------------------------------------------------------
// Test 1: Owner can access their agent's memory (not 404)
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn owner_gets_agent_memory_not_404() {
    let h = start_mt_router().await;
    let agent_id = register_agent(&h, "tenant-a", "owner-agent");

    let resp = h
        .send(
            Request::builder()
                .uri(format!("/api/memory/agents/{agent_id}"))
                .header("x-account-id", "tenant-a")
                .body(Body::empty())
                .unwrap(),
        )
        .await;

    // Owner must NOT get 404. The actual status depends on whether a PM store
    // is configured (200 with data, or 500 if no store), but never 404.
    assert_ne!(
        resp.status(),
        StatusCode::NOT_FOUND,
        "Owner should be able to access their own agent's memory, got 404"
    );
    assert_ne!(
        resp.status(),
        StatusCode::BAD_REQUEST,
        "Owner request with valid header must not be rejected as bad request"
    );
}

// ---------------------------------------------------------------------------
// Test 2: Cross-tenant GET /api/memory/agents/{id} → 404
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn cross_tenant_list_agent_memory_returns_404() {
    let h = start_mt_router().await;
    let agent_id = register_agent(&h, "tenant-a", "private-agent");

    let resp = h
        .send(
            Request::builder()
                .uri(format!("/api/memory/agents/{agent_id}"))
                .header("x-account-id", "tenant-b")
                .body(Body::empty())
                .unwrap(),
        )
        .await;

    assert_eq!(
        resp.status(),
        StatusCode::NOT_FOUND,
        "Cross-tenant GET /api/memory/agents/{{id}} must return 404"
    );
}

// ---------------------------------------------------------------------------
// Test 3: Cross-tenant DELETE /api/memory/agents/{id} → 404
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn cross_tenant_delete_agent_memory_returns_404() {
    let h = start_mt_router().await;
    let agent_id = register_agent(&h, "tenant-a", "delete-target");

    let resp = h
        .send(
            Request::builder()
                .method("DELETE")
                .uri(format!("/api/memory/agents/{agent_id}"))
                .header("x-account-id", "tenant-b")
                .body(Body::empty())
                .unwrap(),
        )
        .await;

    assert_eq!(
        resp.status(),
        StatusCode::NOT_FOUND,
        "Cross-tenant DELETE /api/memory/agents/{{id}} must return 404"
    );
}

// ---------------------------------------------------------------------------
// Test 4: Cross-tenant GET /api/memory/agents/{id}/search?q=test → 404
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn cross_tenant_search_agent_memory_returns_404() {
    let h = start_mt_router().await;
    let agent_id = register_agent(&h, "tenant-a", "search-target");

    let resp = h
        .send(
            Request::builder()
                .uri(format!(
                    "/api/memory/agents/{agent_id}/search?q=test&limit=5"
                ))
                .header("x-account-id", "tenant-b")
                .body(Body::empty())
                .unwrap(),
        )
        .await;

    assert_eq!(
        resp.status(),
        StatusCode::NOT_FOUND,
        "Cross-tenant GET /api/memory/agents/{{id}}/search must return 404"
    );
}

// ---------------------------------------------------------------------------
// Test 5: Cross-tenant GET /api/memory/agents/{id}/stats → 404
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn cross_tenant_stats_agent_memory_returns_404() {
    let h = start_mt_router().await;
    let agent_id = register_agent(&h, "tenant-a", "stats-target");

    let resp = h
        .send(
            Request::builder()
                .uri(format!("/api/memory/agents/{agent_id}/stats"))
                .header("x-account-id", "tenant-b")
                .body(Body::empty())
                .unwrap(),
        )
        .await;

    assert_eq!(
        resp.status(),
        StatusCode::NOT_FOUND,
        "Cross-tenant GET /api/memory/agents/{{id}}/stats must return 404"
    );
}

// ---------------------------------------------------------------------------
// Test 6: Cross-tenant POST /api/memory/agents/{id}/consolidate → 404
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn cross_tenant_consolidate_agent_memory_returns_404() {
    let h = start_mt_router().await;
    let agent_id = register_agent(&h, "tenant-a", "consolidate-target");

    let resp = h
        .send(
            Request::builder()
                .method("POST")
                .uri(format!("/api/memory/agents/{agent_id}/consolidate"))
                .header("x-account-id", "tenant-b")
                .body(Body::empty())
                .unwrap(),
        )
        .await;

    assert_eq!(
        resp.status(),
        StatusCode::NOT_FOUND,
        "Cross-tenant POST /api/memory/agents/{{id}}/consolidate must return 404"
    );
}

// ---------------------------------------------------------------------------
// Test 7: Cross-tenant GET /api/memory/agents/{id}/count → 404
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn cross_tenant_count_agent_memory_returns_404() {
    let h = start_mt_router().await;
    let agent_id = register_agent(&h, "tenant-a", "count-target");

    let resp = h
        .send(
            Request::builder()
                .uri(format!("/api/memory/agents/{agent_id}/count"))
                .header("x-account-id", "tenant-b")
                .body(Body::empty())
                .unwrap(),
        )
        .await;

    assert_eq!(
        resp.status(),
        StatusCode::NOT_FOUND,
        "Cross-tenant GET /api/memory/agents/{{id}}/count must return 404"
    );
}

// ---------------------------------------------------------------------------
// Test 8: Cross-tenant GET /api/memory/agents/{id}/export → 404
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn cross_tenant_export_agent_memory_returns_404() {
    let h = start_mt_router().await;
    let agent_id = register_agent(&h, "tenant-a", "export-target");

    let resp = h
        .send(
            Request::builder()
                .uri(format!("/api/memory/agents/{agent_id}/export"))
                .header("x-account-id", "tenant-b")
                .body(Body::empty())
                .unwrap(),
        )
        .await;

    assert_eq!(
        resp.status(),
        StatusCode::NOT_FOUND,
        "Cross-tenant GET /api/memory/agents/{{id}}/export must return 404"
    );
}

// ---------------------------------------------------------------------------
// Test 9: Cross-tenant POST /api/memory/agents/{id}/import → 404
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn cross_tenant_import_agent_memory_returns_404() {
    let h = start_mt_router().await;
    let agent_id = register_agent(&h, "tenant-a", "import-target");

    let resp = h
        .send(
            Request::builder()
                .method("POST")
                .uri(format!("/api/memory/agents/{agent_id}/import"))
                .header("x-account-id", "tenant-b")
                .header("content-type", "application/json")
                .body(Body::from("[]"))
                .unwrap(),
        )
        .await;

    assert_eq!(
        resp.status(),
        StatusCode::NOT_FOUND,
        "Cross-tenant POST /api/memory/agents/{{id}}/import must return 404"
    );
}
