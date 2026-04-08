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
    let _ = h.state.kernel.agent_registry().register(entry);

    // tenant-b tries to access tenant-a's agent
    let resp = h
        .send(
            Request::builder()
                .uri(format!("/api/agents/{agent_id}"))
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
    let _ = h.state.kernel.agent_registry().register(entry);

    let resp = h
        .send(
            Request::builder()
                .uri(format!("/api/agents/{agent_id}"))
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
    let _ = h.state.kernel.agent_registry().register(entry_a);

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
    let _ = h.state.kernel.agent_registry().register(entry_b);

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

// ---------------------------------------------------------------------------
// Memory isolation: cross-tenant denial — search
// ---------------------------------------------------------------------------

/// Register an agent owned by the given tenant in the kernel registry.
///
/// Returns the agent ID string so callers can reference it.
fn register_tenant_agent(h: &MtHarness, tenant: &str, agent_name: &str) -> String {
    use librefang_types::agent::{
        AgentEntry, AgentId, AgentIdentity, AgentManifest, AgentMode, AgentState,
    };

    let agent_id = AgentId::new();
    let entry = AgentEntry {
        id: agent_id,
        account_id: Some(tenant.to_string()),
        name: agent_name.to_string(),
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

/// Insert a test memory directly into the semantic store for a given agent/tenant.
///
/// Bypasses LLM extraction by writing straight to SQLite via MemorySubstrate.
/// The metadata contains `account_id` so scoped queries can filter on it.
/// Returns the memory ID as a string.
fn insert_test_memory(h: &MtHarness, agent_id_str: &str, tenant: &str, content: &str) -> String {
    use librefang_types::memory::MemorySource;
    use std::collections::HashMap;

    let agent_id: librefang_types::agent::AgentId = agent_id_str
        .parse()
        .expect("agent_id_str should be a valid UUID");

    let mut metadata = HashMap::new();
    metadata.insert(
        "account_id".to_string(),
        serde_json::Value::String(tenant.to_string()),
    );

    let mem_id = h
        .state
        .kernel
        .memory_substrate()
        .remember_with_embedding(
            agent_id,
            content,
            MemorySource::UserProvided,
            "user",
            metadata,
            None, // no embedding — uses LIKE fallback
        )
        .expect("insert_test_memory should succeed");

    mem_id.to_string()
}

#[tokio::test(flavor = "multi_thread")]
async fn test_memory_search_scoped_by_tenant() {
    let h = start_mt_router().await;

    let agent_a = register_tenant_agent(&h, "tenant-a", "agent-alpha");
    let _agent_b = register_tenant_agent(&h, "tenant-b", "agent-beta");

    // Seed a real memory for tenant-a so the store is NOT empty.
    insert_test_memory(&h, &agent_a, "tenant-a", "alpha secret knowledge");

    // tenant-b searches — should NOT see tenant-a's memory.
    let resp = h
        .send(
            Request::builder()
                .uri("/api/memory/search?q=alpha+secret&limit=10")
                .header("x-account-id", "tenant-b")
                .body(Body::empty())
                .unwrap(),
        )
        .await;

    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "search_scoped should return 200, got: {}",
        resp.status()
    );
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let items = json
        .get("memories")
        .and_then(|v| v.as_array())
        .expect("Expected memories array");
    assert!(
        items.is_empty(),
        "tenant-b must not see tenant-a's memories via search, got: {items:?}"
    );

    // tenant-a searches — SHOULD see its own memory.
    let resp_a = h
        .send(
            Request::builder()
                .uri("/api/memory/search?q=alpha+secret&limit=10")
                .header("x-account-id", "tenant-a")
                .body(Body::empty())
                .unwrap(),
        )
        .await;

    assert_eq!(resp_a.status(), StatusCode::OK);
    let body_a = axum::body::to_bytes(resp_a.into_body(), usize::MAX)
        .await
        .unwrap();
    let json_a: serde_json::Value = serde_json::from_slice(&body_a).unwrap();
    let items_a = json_a
        .get("memories")
        .and_then(|v| v.as_array())
        .expect("Expected memories array");
    assert!(
        !items_a.is_empty(),
        "tenant-a should see its own memory via search, got empty results"
    );
}

// ---------------------------------------------------------------------------
// Memory isolation: cross-tenant denial — list
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn test_memory_list_scoped_by_tenant() {
    let h = start_mt_router().await;

    let agent_a = register_tenant_agent(&h, "tenant-a", "agent-alpha");
    let _agent_b = register_tenant_agent(&h, "tenant-b", "agent-beta");

    // Seed a real memory for tenant-a so the store is NOT empty.
    insert_test_memory(&h, &agent_a, "tenant-a", "tenant-a private note");

    // tenant-b lists — should NOT see tenant-a's memory.
    let resp = h
        .send(
            Request::builder()
                .uri("/api/memory")
                .header("x-account-id", "tenant-b")
                .body(Body::empty())
                .unwrap(),
        )
        .await;

    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "list_by_account should return 200, got: {}",
        resp.status()
    );
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let items = json
        .get("memories")
        .and_then(|v| v.as_array())
        .expect("Expected memories array");
    // tenant-b should see zero memories — the only memory belongs to tenant-a
    assert!(
        items.is_empty(),
        "tenant-b must not see tenant-a's memories in list, got {} items: {items:?}",
        items.len()
    );

    // tenant-a lists — SHOULD see its own memory.
    let resp_a = h
        .send(
            Request::builder()
                .uri("/api/memory")
                .header("x-account-id", "tenant-a")
                .body(Body::empty())
                .unwrap(),
        )
        .await;

    assert_eq!(resp_a.status(), StatusCode::OK);
    let body_a = axum::body::to_bytes(resp_a.into_body(), usize::MAX)
        .await
        .unwrap();
    let json_a: serde_json::Value = serde_json::from_slice(&body_a).unwrap();
    let items_a = json_a
        .get("memories")
        .and_then(|v| v.as_array())
        .expect("Expected memories array");
    assert!(
        !items_a.is_empty(),
        "tenant-a should see its own memory via list, got empty results"
    );
}

// ---------------------------------------------------------------------------
// Memory isolation: cross-tenant denial — delete
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn test_memory_delete_cross_tenant_denied() {
    let h = start_mt_router().await;

    let agent_a = register_tenant_agent(&h, "tenant-a", "agent-alpha");

    // Insert a REAL memory owned by tenant-a's agent.
    let mem_id = insert_test_memory(&h, &agent_a, "tenant-a", "do not delete me");

    // tenant-b tries to delete tenant-a's memory.
    // The handler calls require_memory_access() which finds the owning agent,
    // then check_account() rejects because tenant-b != tenant-a → 404.
    let resp = h
        .send(
            Request::builder()
                .method("DELETE")
                .uri(format!("/api/memory/items/{mem_id}"))
                .header("x-account-id", "tenant-b")
                .body(Body::empty())
                .unwrap(),
        )
        .await;

    assert_eq!(
        resp.status(),
        StatusCode::NOT_FOUND,
        "Cross-tenant delete must return 404 (ownership denied), got: {}",
        resp.status()
    );

    // Verify the memory still exists by having tenant-a search for it.
    let resp_check = h
        .send(
            Request::builder()
                .uri("/api/memory/search?q=do+not+delete&limit=10")
                .header("x-account-id", "tenant-a")
                .body(Body::empty())
                .unwrap(),
        )
        .await;

    assert_eq!(resp_check.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp_check.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let items = json
        .get("memories")
        .and_then(|v| v.as_array())
        .expect("Expected memories array");
    assert!(
        !items.is_empty(),
        "Memory should still exist after cross-tenant delete attempt"
    );
}

// ---------------------------------------------------------------------------
// Memory isolation: cross-tenant denial — update
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn test_memory_update_cross_tenant_denied() {
    let h = start_mt_router().await;

    let agent_a = register_tenant_agent(&h, "tenant-a", "agent-alpha");

    // Insert a REAL memory owned by tenant-a's agent.
    let mem_id = insert_test_memory(&h, &agent_a, "tenant-a", "original content");

    // tenant-b tries to update tenant-a's memory.
    // The handler calls require_memory_access() → check_account() → 404.
    let resp = h
        .send(
            Request::builder()
                .method("PUT")
                .uri(format!("/api/memory/items/{mem_id}"))
                .header("x-account-id", "tenant-b")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({"content": "hijacked"}).to_string(),
                ))
                .unwrap(),
        )
        .await;

    assert_eq!(
        resp.status(),
        StatusCode::NOT_FOUND,
        "Cross-tenant memory update must return 404 (ownership denied), got: {}",
        resp.status()
    );

    // Verify the content was NOT changed by having tenant-a search for the original.
    let resp_check = h
        .send(
            Request::builder()
                .uri("/api/memory/search?q=original+content&limit=10")
                .header("x-account-id", "tenant-a")
                .body(Body::empty())
                .unwrap(),
        )
        .await;

    assert_eq!(resp_check.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp_check.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let items = json
        .get("memories")
        .and_then(|v| v.as_array())
        .expect("Expected memories array");
    assert!(
        !items.is_empty(),
        "Original memory should still exist after cross-tenant update attempt"
    );
    // Verify none of the results contain the hijacked content
    for item in items {
        let content = item
            .get("content")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        assert!(
            !content.contains("hijacked"),
            "Memory content must not be modified by cross-tenant update, got: {content}"
        );
    }
}

// ---------------------------------------------------------------------------
// Admin fallback: admin (no account header) sees all memories
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn test_memory_search_admin_sees_all() {
    // Use single-tenant router so admin requests (no X-Account-Id) are not
    // blocked by the require_account_id middleware.
    let h = start_st_router().await;

    let agent_a = register_tenant_agent(&h, "tenant-a", "agent-alpha");
    let agent_b = register_tenant_agent(&h, "tenant-b", "agent-beta");

    // Seed memories for BOTH tenants so the store is non-empty.
    insert_test_memory(&h, &agent_a, "tenant-a", "alpha admin searchable");
    insert_test_memory(&h, &agent_b, "tenant-b", "beta admin searchable");

    // Admin request — no X-Account-Id header.
    // In single-tenant mode, the handler gets AccountId(None) and dispatches
    // to search_all() (unscoped). This verifies the admin code path works.
    let resp = h
        .send(
            Request::builder()
                .uri("/api/memory/search?q=admin+searchable&limit=50")
                .body(Body::empty())
                .unwrap(),
        )
        .await;

    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "Admin search should succeed (200), got: {}",
        resp.status()
    );

    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let items = json
        .get("memories")
        .and_then(|v| v.as_array())
        .expect("Expected memories array");
    // Admin should see memories from BOTH tenants (at least 2).
    assert!(
        items.len() >= 2,
        "Admin search should return memories from both tenants, got {} items: {items:?}",
        items.len()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_memory_list_admin_sees_all() {
    let h = start_st_router().await;

    let agent_a = register_tenant_agent(&h, "tenant-a", "agent-alpha");
    let agent_b = register_tenant_agent(&h, "tenant-b", "agent-beta");

    // Seed memories for BOTH tenants so the store is non-empty.
    insert_test_memory(&h, &agent_a, "tenant-a", "alpha listed memory");
    insert_test_memory(&h, &agent_b, "tenant-b", "beta listed memory");

    // Admin request — no X-Account-Id header.
    // In single-tenant mode, the handler gets AccountId(None) and dispatches
    // to list_all() (unscoped).
    let resp = h
        .send(
            Request::builder()
                .uri("/api/memory")
                .body(Body::empty())
                .unwrap(),
        )
        .await;

    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "Admin list should succeed (200), got: {}",
        resp.status()
    );

    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

    // Admin uses list_all() — verify response structure is valid
    assert!(
        json.get("total").is_some(),
        "Admin list response should contain 'total' field, got: {json}"
    );

    let total = json["total"].as_u64().unwrap_or(0);
    assert!(
        total >= 2,
        "Admin list should see memories from both tenants, got total={total}"
    );

    let items = json
        .get("memories")
        .and_then(|v| v.as_array())
        .expect("Expected memories array");
    assert!(
        items.len() >= 2,
        "Admin list should return memories from both tenants, got {} items",
        items.len()
    );
}

// ---------------------------------------------------------------------------
// Agent ownership: agent-scoped endpoints verify ownership
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn test_memory_agent_endpoints_require_ownership() {
    let h = start_mt_router().await;

    use librefang_types::agent::{
        AgentEntry, AgentId, AgentIdentity, AgentManifest, AgentMode, AgentState,
    };

    // Register agent owned by tenant-a
    let agent_id = AgentId::new();
    let entry = AgentEntry {
        id: agent_id,
        account_id: Some("tenant-a".to_string()),
        name: "owned-agent".to_string(),
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

    // tenant-b tries agent-scoped memory list
    let resp_list = h
        .send(
            Request::builder()
                .uri(format!("/api/memory/agents/{agent_id}"))
                .header("x-account-id", "tenant-b")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(
        resp_list.status(),
        StatusCode::NOT_FOUND,
        "Agent list must return 404 for non-owner tenant"
    );

    // tenant-b tries agent-scoped memory search
    let resp_search = h
        .send(
            Request::builder()
                .uri(format!("/api/memory/agents/{agent_id}/search?q=test&limit=5"))
                .header("x-account-id", "tenant-b")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(
        resp_search.status(),
        StatusCode::NOT_FOUND,
        "Agent search must return 404 for non-owner tenant"
    );

    // tenant-b tries agent-scoped memory stats
    let resp_stats = h
        .send(
            Request::builder()
                .uri(format!("/api/memory/agents/{agent_id}/stats"))
                .header("x-account-id", "tenant-b")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(
        resp_stats.status(),
        StatusCode::NOT_FOUND,
        "Agent stats must return 404 for non-owner tenant"
    );

    // tenant-a (owner) should NOT get 404
    let resp_owner = h
        .send(
            Request::builder()
                .uri(format!("/api/memory/agents/{agent_id}"))
                .header("x-account-id", "tenant-a")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_ne!(
        resp_owner.status(),
        StatusCode::NOT_FOUND,
        "Owner should be able to access their agent's memories"
    );
}

// ---------------------------------------------------------------------------
// Admin-only endpoints: tenants get 403
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn test_memory_cleanup_admin_only() {
    let h = start_mt_router().await;

    let resp = h
        .send(
            Request::builder()
                .method("POST")
                .uri("/api/memory/cleanup")
                .header("x-account-id", "tenant-a")
                .body(Body::empty())
                .unwrap(),
        )
        .await;

    assert_eq!(
        resp.status(),
        StatusCode::FORBIDDEN,
        "POST /api/memory/cleanup must return 403 for tenants, got: {}",
        resp.status()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_memory_decay_admin_only() {
    let h = start_mt_router().await;

    let resp = h
        .send(
            Request::builder()
                .method("POST")
                .uri("/api/memory/decay")
                .header("x-account-id", "tenant-a")
                .body(Body::empty())
                .unwrap(),
        )
        .await;

    assert_eq!(
        resp.status(),
        StatusCode::FORBIDDEN,
        "POST /api/memory/decay must return 403 for tenants, got: {}",
        resp.status()
    );
}

// ---------------------------------------------------------------------------
// Config endpoint: accessible by admin
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn test_memory_config_accessible() {
    let h = start_st_router().await;

    // GET /api/memory/config — admin (no account header in ST mode)
    let resp = h
        .send(
            Request::builder()
                .uri("/api/memory/config")
                .body(Body::empty())
                .unwrap(),
        )
        .await;

    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "GET /api/memory/config should return 200 for admin, got: {}",
        resp.status()
    );

    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

    // Should contain proactive_memory config keys
    assert!(
        json.get("proactive_memory").is_some(),
        "Config response should contain proactive_memory section, got: {json}"
    );
}
