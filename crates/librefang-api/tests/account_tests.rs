//! Multi-tenant account isolation integration tests.
//!
//! Verifies cross-tenant read/write denial across API endpoints when
//! `multi_tenant = true`. Uses `build_router` with the real middleware stack
//! so the `require_account_id` layer is applied.
//!
//! Run: cargo test -p librefang-api --test account_tests -- --nocapture

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use librefang_api::routes::AppState;
use librefang_api::server;
use librefang_kernel::LibreFangKernel;
use librefang_types::config::{DefaultModelConfig, KernelConfig};
use std::sync::Arc;
use tower::ServiceExt;

// ---------------------------------------------------------------------------
// Test infrastructure
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

/// Boot a full router with multi-tenant mode disabled (single-tenant/legacy).
async fn start_st_router() -> MtHarness {
    let tmp = tempfile::tempdir().expect("Failed to create temp dir");

    let config = KernelConfig {
        home_dir: tmp.path().to_path_buf(),
        data_dir: tmp.path().join("data"),
        multi_tenant: false,
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

// ---------------------------------------------------------------------------
// Multi-tenant: missing header is rejected
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn mt_missing_account_header_rejected_on_agents() {
    let h = start_mt_router().await;
    let resp = h
        .send(
            Request::builder()
                .uri("/api/agents")
                .body(Body::empty())
                .unwrap(),
        )
        .await;

    assert_eq!(
        resp.status(),
        StatusCode::BAD_REQUEST,
        "Multi-tenant mode must reject requests without X-Account-Id"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn mt_missing_account_header_rejected_on_config() {
    let h = start_mt_router().await;
    let resp = h
        .send(
            Request::builder()
                .uri("/api/config")
                .body(Body::empty())
                .unwrap(),
        )
        .await;

    assert_eq!(
        resp.status(),
        StatusCode::BAD_REQUEST,
        "Multi-tenant mode must reject requests without X-Account-Id on /config"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn mt_missing_account_header_rejected_on_workflows() {
    let h = start_mt_router().await;
    let resp = h
        .send(
            Request::builder()
                .uri("/api/workflows")
                .body(Body::empty())
                .unwrap(),
        )
        .await;

    assert_eq!(
        resp.status(),
        StatusCode::BAD_REQUEST,
        "Multi-tenant mode must reject requests without X-Account-Id on /workflows"
    );
}

// ---------------------------------------------------------------------------
// Multi-tenant: health and version are still exempt
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn mt_health_endpoint_exempt_from_account_requirement() {
    let h = start_mt_router().await;
    let resp = h
        .send(
            Request::builder()
                .uri("/api/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await;

    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "/api/health must remain accessible without X-Account-Id"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn mt_version_endpoint_exempt_from_account_requirement() {
    let h = start_mt_router().await;
    let resp = h
        .send(
            Request::builder()
                .uri("/api/version")
                .body(Body::empty())
                .unwrap(),
        )
        .await;

    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "/api/version must remain accessible without X-Account-Id"
    );
}

// ---------------------------------------------------------------------------
// Multi-tenant: valid header passes through
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn mt_valid_account_header_passes_on_agents() {
    let h = start_mt_router().await;
    let resp = h
        .send(
            Request::builder()
                .uri("/api/agents")
                .header("x-account-id", "tenant-a")
                .body(Body::empty())
                .unwrap(),
        )
        .await;

    // Should get 200 (empty agent list), not 400 (missing header)
    assert_ne!(
        resp.status(),
        StatusCode::BAD_REQUEST,
        "Valid X-Account-Id should pass the middleware gate"
    );
}

// ---------------------------------------------------------------------------
// Single-tenant: no header required
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn st_no_account_header_passes_when_mt_disabled() {
    let h = start_st_router().await;
    let resp = h
        .send(
            Request::builder()
                .uri("/api/agents")
                .body(Body::empty())
                .unwrap(),
        )
        .await;

    assert_ne!(
        resp.status(),
        StatusCode::BAD_REQUEST,
        "Single-tenant mode should not require X-Account-Id"
    );
}

// ---------------------------------------------------------------------------
// Cross-tenant isolation: agent list filtering
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn mt_agent_list_returns_empty_for_new_tenant() {
    let h = start_mt_router().await;
    let resp = h
        .send(
            Request::builder()
                .uri("/api/agents")
                .header("x-account-id", "tenant-with-no-agents")
                .body(Body::empty())
                .unwrap(),
        )
        .await;

    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

    // Response is PaginatedResponse { items, total, offset, limit }
    let items = json
        .get("items")
        .and_then(|v| v.as_array())
        .expect("Expected items array in paginated response");

    assert!(
        items.is_empty(),
        "New tenant should see no agents, got: {items:?}"
    );
    assert_eq!(json["total"], 0, "Total should be 0 for new tenant");
}

// ---------------------------------------------------------------------------
// Cross-tenant isolation: accessing another tenant's agent returns 404
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn mt_cross_tenant_agent_access_returns_404() {
    let h = start_mt_router().await;

    // Register an agent owned by tenant-a directly in the registry
    use librefang_types::agent::{
        AgentEntry, AgentId, AgentIdentity, AgentManifest, AgentMode, AgentState,
    };
    let agent_id = AgentId::new();
    let entry = AgentEntry {
        id: agent_id,
        account_id: Some("tenant-a".to_string()),
        name: "private-agent".to_string(),
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
    h.state.kernel.agent_registry().register(entry);

    // tenant-b tries to access tenant-a's agent
    let resp = h
        .send(
            Request::builder()
                .uri(&format!("/api/agents/{agent_id}"))
                .header("x-account-id", "tenant-b")
                .body(Body::empty())
                .unwrap(),
        )
        .await;

    assert_eq!(
        resp.status(),
        StatusCode::NOT_FOUND,
        "Cross-tenant agent access must return 404, not 200 or 403"
    );
}

// ---------------------------------------------------------------------------
// Cross-tenant isolation: owner can access their own agent
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn mt_owner_can_access_own_agent() {
    let h = start_mt_router().await;

    use librefang_types::agent::{
        AgentEntry, AgentId, AgentIdentity, AgentManifest, AgentMode, AgentState,
    };
    let agent_id = AgentId::new();
    let entry = AgentEntry {
        id: agent_id,
        account_id: Some("tenant-a".to_string()),
        name: "my-agent".to_string(),
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
    h.state.kernel.agent_registry().register(entry);

    let resp = h
        .send(
            Request::builder()
                .uri(&format!("/api/agents/{agent_id}"))
                .header("x-account-id", "tenant-a")
                .body(Body::empty())
                .unwrap(),
        )
        .await;

    // Should be 200 (found) — not 404 (wrong tenant) or 400 (missing header)
    assert_ne!(
        resp.status(),
        StatusCode::NOT_FOUND,
        "Owner should be able to access their own agent"
    );
    assert_ne!(resp.status(), StatusCode::BAD_REQUEST);
}

// ---------------------------------------------------------------------------
// Cross-tenant isolation: tenant-b cannot see tenant-a's agents in list
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn mt_agent_list_filtered_by_tenant() {
    let h = start_mt_router().await;

    use librefang_types::agent::{
        AgentEntry, AgentId, AgentIdentity, AgentManifest, AgentMode, AgentState,
    };

    // Register agent for tenant-a
    let entry_a = AgentEntry {
        id: AgentId::new(),
        account_id: Some("tenant-a".to_string()),
        name: "alpha-agent".to_string(),
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
    h.state.kernel.agent_registry().register(entry_a);

    // Register agent for tenant-b
    let entry_b = AgentEntry {
        id: AgentId::new(),
        account_id: Some("tenant-b".to_string()),
        name: "beta-agent".to_string(),
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
    h.state.kernel.agent_registry().register(entry_b);

    // tenant-a lists agents — should see only their own
    let resp = h
        .send(
            Request::builder()
                .uri("/api/agents")
                .header("x-account-id", "tenant-a")
                .body(Body::empty())
                .unwrap(),
        )
        .await;

    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

    // Response is PaginatedResponse { items, total, offset, limit }
    let items = json
        .get("items")
        .and_then(|v| v.as_array())
        .expect("Expected items array in paginated response");

    assert_eq!(
        items.len(),
        1,
        "tenant-a should see exactly 1 agent, got {}",
        items.len()
    );

    // Verify it's the right agent
    let name = items[0].get("name").and_then(|v| v.as_str()).unwrap_or("");
    assert_eq!(name, "alpha-agent", "tenant-a should see alpha-agent only");
}

// ---------------------------------------------------------------------------
// Regression: empty/whitespace X-Account-Id is rejected in MT mode
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn mt_empty_account_header_rejected() {
    let h = start_mt_router().await;
    let resp = h
        .send(
            Request::builder()
                .uri("/api/agents")
                .header("x-account-id", "")
                .body(Body::empty())
                .unwrap(),
        )
        .await;

    assert_eq!(
        resp.status(),
        StatusCode::BAD_REQUEST,
        "Empty X-Account-Id must be rejected in multi-tenant mode"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn mt_whitespace_account_header_rejected() {
    let h = start_mt_router().await;
    let resp = h
        .send(
            Request::builder()
                .uri("/api/agents")
                .header("x-account-id", "   ")
                .body(Body::empty())
                .unwrap(),
        )
        .await;

    assert_eq!(
        resp.status(),
        StatusCode::BAD_REQUEST,
        "Whitespace-only X-Account-Id must be rejected in multi-tenant mode"
    );
}
