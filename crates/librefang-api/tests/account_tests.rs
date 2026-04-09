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
use futures::{SinkExt, StreamExt};
use librefang_api::routes::AppState;
use librefang_api::server;
use librefang_kernel::workflow::{Workflow, WorkflowId, WorkflowRunId};
use librefang_kernel::LibreFangKernel;
use librefang_types::config::{DefaultModelConfig, KernelConfig};
use librefang_types::workflow_template::{
    ParameterType, TemplateParameter, WorkflowTemplate, WorkflowTemplateStep,
};
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

struct MtHttpHarness {
    base_url: String,
    state: Arc<AppState>,
    _tmp: tempfile::TempDir,
}

impl Drop for MtHarness {
    fn drop(&mut self) {
        self.state.kernel.shutdown();
    }
}

impl Drop for MtHttpHarness {
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

async fn read_json(resp: axum::response::Response) -> serde_json::Value {
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    serde_json::from_slice(&body).unwrap()
}

fn make_agent_entry(name: &str, account_id: Option<&str>) -> librefang_types::agent::AgentEntry {
    use librefang_types::agent::{
        AgentEntry, AgentId, AgentIdentity, AgentManifest, AgentMode, AgentState,
    };

    AgentEntry {
        id: AgentId::new(),
        account_id: account_id.map(str::to_string),
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
    }
}

fn make_workflow(name: &str) -> Workflow {
    Workflow {
        id: WorkflowId::new(),
        name: name.to_string(),
        description: "test workflow".to_string(),
        steps: vec![],
        created_at: chrono::Utc::now(),
        account_id: None,
        layout: None,
    }
}

fn make_template(id: &str, name: &str) -> WorkflowTemplate {
    WorkflowTemplate {
        id: id.to_string(),
        name: name.to_string(),
        description: "test template".to_string(),
        category: None,
        parameters: vec![TemplateParameter {
            name: "subject".to_string(),
            description: None,
            param_type: ParameterType::String,
            default: None,
            required: true,
        }],
        steps: vec![WorkflowTemplateStep {
            name: "step-1".to_string(),
            prompt_template: "Inspect {{subject}}".to_string(),
            agent: None,
            depends_on: vec![],
        }],
        tags: vec![],
        created_at: None,
        i18n: Default::default(),
    }
}

fn first_hand_id(h: &MtHarness) -> String {
    h.state
        .kernel
        .hands()
        .list_definitions()
        .into_iter()
        .next()
        .expect("expected at least one hand definition in test registry")
        .id
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

/// Boot a full router with multi-tenant mode enabled and an admin account configured.
async fn start_mt_router_with_admin(admin_id: &str) -> MtHarness {
    let tmp = tempfile::tempdir().expect("Failed to create temp dir");

    let config = KernelConfig {
        home_dir: tmp.path().to_path_buf(),
        data_dir: tmp.path().join("data"),
        multi_tenant: true,
        admin_accounts: vec![admin_id.to_string()],
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

async fn start_mt_http_router() -> MtHttpHarness {
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

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("Failed to bind test server");
    let addr = listener.local_addr().expect("listener addr");
    tokio::spawn(async move {
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .await
        .unwrap();
    });

    MtHttpHarness {
        base_url: format!("http://{addr}"),
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
// Channels: tenant-owned route isolation
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn mt_channels_are_invisible_across_tenants() {
    let h = start_mt_router().await;

    let configure = h
        .send(
            Request::builder()
                .method("POST")
                .uri("/api/channels/telegram/configure")
                .header("content-type", "application/json")
                .header("x-account-id", "tenant-a")
                .body(Body::from(
                    serde_json::json!({
                        "fields": {
                            "bot_token_env": "tenant-a-token",
                            "default_agent": "assistant-a"
                        }
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await;
    assert_eq!(configure.status(), StatusCode::OK);

    let owner = read_json(
        h.send(
            Request::builder()
                .uri("/api/channels/telegram")
                .header("x-account-id", "tenant-a")
                .body(Body::empty())
                .unwrap(),
        )
        .await,
    )
    .await;
    assert_eq!(owner["configured"], true);
    assert_eq!(owner["has_token"], true);

    let other = read_json(
        h.send(
            Request::builder()
                .uri("/api/channels/telegram")
                .header("x-account-id", "tenant-b")
                .body(Body::empty())
                .unwrap(),
        )
        .await,
    )
    .await;
    assert_eq!(other["configured"], false);
    assert_eq!(other["has_token"], false);

    let default_agent_field = other["fields"]
        .as_array()
        .unwrap()
        .iter()
        .find(|field| field["key"] == "default_agent")
        .unwrap();
    assert!(
        default_agent_field.get("value").is_none(),
        "tenant-b must not see tenant-a channel config values"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn mt_cross_tenant_channel_delete_does_not_remove_owner_config() {
    let h = start_mt_router().await;

    let configure = h
        .send(
            Request::builder()
                .method("POST")
                .uri("/api/channels/telegram/configure")
                .header("content-type", "application/json")
                .header("x-account-id", "tenant-a")
                .body(Body::from(
                    serde_json::json!({
                        "fields": {
                            "bot_token_env": "tenant-a-token",
                            "default_agent": "assistant-a"
                        }
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await;
    assert_eq!(configure.status(), StatusCode::OK);

    let delete = h
        .send(
            Request::builder()
                .method("DELETE")
                .uri("/api/channels/telegram/configure")
                .header("x-account-id", "tenant-b")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(delete.status(), StatusCode::OK);

    let owner = read_json(
        h.send(
            Request::builder()
                .uri("/api/channels/telegram")
                .header("x-account-id", "tenant-a")
                .body(Body::empty())
                .unwrap(),
        )
        .await,
    )
    .await;
    assert_eq!(
        owner["configured"], true,
        "tenant-b delete must not remove tenant-a channel config"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn mt_channel_partial_update_preserves_scoped_secret_mapping_via_route() {
    let h = start_mt_router().await;

    let first = h
        .send(
            Request::builder()
                .method("POST")
                .uri("/api/channels/telegram/configure")
                .header("content-type", "application/json")
                .header("x-account-id", "tenant-a")
                .body(Body::from(
                    serde_json::json!({
                        "fields": {
                            "bot_token_env": "tenant-a-token",
                            "default_agent": "assistant-a"
                        }
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await;
    assert_eq!(first.status(), StatusCode::OK);

    let second = h
        .send(
            Request::builder()
                .method("POST")
                .uri("/api/channels/telegram/configure")
                .header("content-type", "application/json")
                .header("x-account-id", "tenant-a")
                .body(Body::from(
                    serde_json::json!({
                        "fields": {
                            "default_agent": "assistant-b"
                        }
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await;
    assert_eq!(second.status(), StatusCode::OK);

    let config_path = h.state.kernel.home_dir().join("config.toml");
    let parsed: toml::Value =
        toml::from_str(&std::fs::read_to_string(&config_path).unwrap()).unwrap();
    let entry = &parsed["channels"]["telegram"];
    assert_eq!(entry["account_id"].as_str(), Some("tenant-a"));
    assert_eq!(
        entry["bot_token_env"].as_str(),
        Some("TELEGRAM_BOT_TOKEN__ACCOUNT_74_65_6E_61_6E_74_2D_61")
    );
    assert_eq!(entry["default_agent"].as_str(), Some("assistant-b"));
}

#[tokio::test(flavor = "multi_thread")]
async fn mt_channel_reload_keeps_other_tenant_runtime_adapter_registered() {
    let h = start_mt_router().await;

    for (tenant, topic) in [
        ("tenant-a", "tenant-a-topic"),
        ("tenant-b", "tenant-b-topic"),
    ] {
        let resp = h
            .send(
                Request::builder()
                    .method("POST")
                    .uri("/api/channels/ntfy/configure")
                    .header("content-type", "application/json")
                    .header("x-account-id", tenant)
                    .body(Body::from(
                        serde_json::json!({
                            "fields": {
                                "topic": topic,
                                "default_agent": format!("agent-{tenant}")
                            }
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await;
        assert_eq!(resp.status(), StatusCode::OK);
    }

    let keys_before: Vec<String> = h
        .state
        .kernel
        .channel_adapters_ref()
        .iter()
        .map(|entry| entry.key().clone())
        .collect();
    assert!(
        keys_before.iter().any(|key| key == "ntfy:tenant-a"),
        "tenant-a adapter key missing before delete: {keys_before:?}"
    );
    assert!(
        keys_before.iter().any(|key| key == "ntfy:tenant-b"),
        "tenant-b adapter key missing before delete: {keys_before:?}"
    );

    let delete = h
        .send(
            Request::builder()
                .method("DELETE")
                .uri("/api/channels/ntfy/configure")
                .header("x-account-id", "tenant-a")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(delete.status(), StatusCode::OK);

    let keys_after: Vec<String> = h
        .state
        .kernel
        .channel_adapters_ref()
        .iter()
        .map(|entry| entry.key().clone())
        .collect();
    assert!(
        !keys_after.iter().any(|key| key == "ntfy:tenant-a"),
        "tenant-a adapter key should be removed after delete: {keys_after:?}"
    );
    assert!(
        keys_after.iter().any(|key| key == "ntfy:tenant-b"),
        "tenant-b adapter key must survive tenant-a delete/reload: {keys_after:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn mt_conflicting_single_instance_webhook_keeps_existing_runtime_adapter() {
    let h = start_mt_router().await;

    let first = h
        .send(
            Request::builder()
                .method("POST")
                .uri("/api/channels/webhook/configure")
                .header("content-type", "application/json")
                .header("x-account-id", "tenant-a")
                .body(Body::from(
                    serde_json::json!({
                        "fields": {
                            "secret_env": "tenant-a-secret",
                            "default_agent": "agent-tenant-a"
                        }
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await;
    assert_eq!(first.status(), StatusCode::OK);
    let first_json = read_json(first).await;
    assert_eq!(first_json["activated"].as_bool(), Some(true));

    let keys_before: Vec<String> = h
        .state
        .kernel
        .channel_adapters_ref()
        .iter()
        .map(|entry| entry.key().clone())
        .collect();
    assert!(
        keys_before.iter().any(|key| key == "webhook:tenant-a"),
        "tenant-a webhook adapter should be active before conflict: {keys_before:?}"
    );

    let second = h
        .send(
            Request::builder()
                .method("POST")
                .uri("/api/channels/webhook/configure")
                .header("content-type", "application/json")
                .header("x-account-id", "tenant-b")
                .body(Body::from(
                    serde_json::json!({
                        "fields": {
                            "secret_env": "tenant-b-secret",
                            "default_agent": "agent-tenant-b"
                        }
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await;
    assert_eq!(second.status(), StatusCode::OK);
    let second_json = read_json(second).await;
    assert_eq!(second_json["activated"].as_bool(), Some(false));
    assert!(
        second_json["note"]
            .as_str()
            .unwrap_or_default()
            .contains("hot-reload failed"),
        "conflicting webhook config should report explicit reload failure: {second_json:?}"
    );

    let keys_after: Vec<String> = h
        .state
        .kernel
        .channel_adapters_ref()
        .iter()
        .map(|entry| entry.key().clone())
        .collect();
    assert!(
        keys_after.iter().any(|key| key == "webhook:tenant-a"),
        "tenant-a webhook adapter must remain active after conflicting tenant-b configure: {keys_after:?}"
    );
    assert!(
        !keys_after.iter().any(|key| key == "webhook:tenant-b"),
        "conflicting tenant-b webhook adapter must not activate: {keys_after:?}"
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

#[tokio::test(flavor = "multi_thread")]
async fn mt_openai_chat_completions_hides_cross_tenant_agent() {
    let h = start_mt_router().await;
    let entry = make_agent_entry("tenant-a-agent", Some("tenant-a"));
    let agent_id = entry.id;
    let _ = h.state.kernel.agent_registry().register(entry);

    let resp = h
        .send(
            Request::builder()
                .method("POST")
                .uri("/v1/chat/completions")
                .header("content-type", "application/json")
                .header("x-account-id", "tenant-b")
                .body(Body::from(
                    serde_json::json!({
                        "model": agent_id.to_string(),
                        "messages": [{"role": "user", "content": "hello"}]
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await;

    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["error"]["code"], "model_not_found");
}

#[tokio::test(flavor = "multi_thread")]
async fn mt_openai_models_list_filtered_by_tenant() {
    let h = start_mt_router().await;
    let _ = h
        .state
        .kernel
        .agent_registry()
        .register(make_agent_entry("alpha-agent", Some("tenant-a")));
    let _ = h
        .state
        .kernel
        .agent_registry()
        .register(make_agent_entry("beta-agent", Some("tenant-b")));

    let resp = h
        .send(
            Request::builder()
                .uri("/v1/models")
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
    let models = json["data"].as_array().expect("models array");
    assert_eq!(models.len(), 1);
    assert_eq!(models[0]["id"], "librefang:alpha-agent");
}

#[tokio::test(flavor = "multi_thread")]
async fn mt_tenant_workflow_crud_and_run_are_scoped() {
    let h = start_mt_router().await;
    let create = h
        .send(
            Request::builder()
                .method("POST")
                .uri("/api/workflows")
                .header("content-type", "application/json")
                .header("x-account-id", "tenant-a")
                .body(Body::from(
                    serde_json::json!({
                        "name": "tenant-a-workflow",
                        "description": "owned by tenant a",
                        "steps": []
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await;
    assert_eq!(create.status(), StatusCode::CREATED);
    let body = axum::body::to_bytes(create.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let workflow_id = json["workflow_id"]
        .as_str()
        .expect("workflow_id")
        .to_string();

    let workflow = h
        .state
        .kernel
        .workflow_engine()
        .get_workflow(WorkflowId(workflow_id.parse().unwrap()))
        .await
        .expect("workflow should exist");
    assert_eq!(workflow.account_id.as_deref(), Some("tenant-a"));

    let list_a = h
        .send(
            Request::builder()
                .uri("/api/workflows")
                .header("x-account-id", "tenant-a")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(list_a.status(), StatusCode::OK);
    let list_a_json = read_json(list_a).await;
    let workflows = list_a_json["workflows"]
        .as_array()
        .expect("workflows array");
    assert_eq!(workflows.len(), 1);
    assert_eq!(workflows[0]["id"], workflow_id);

    let list_b = h
        .send(
            Request::builder()
                .uri("/api/workflows")
                .header("x-account-id", "tenant-b")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(list_b.status(), StatusCode::OK);
    let list_b_json = read_json(list_b).await;
    let other_workflows = list_b_json["workflows"]
        .as_array()
        .expect("workflows array");
    assert!(other_workflows.is_empty());

    let get_b = h
        .send(
            Request::builder()
                .uri(format!("/api/workflows/{workflow_id}"))
                .header("x-account-id", "tenant-b")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(get_b.status(), StatusCode::NOT_FOUND);

    let run = h
        .send(
            Request::builder()
                .method("POST")
                .uri(format!("/api/workflows/{workflow_id}/run"))
                .header("content-type", "application/json")
                .header("x-account-id", "tenant-a")
                .body(Body::from(
                    serde_json::json!({"input": "hello"}).to_string(),
                ))
                .unwrap(),
        )
        .await;
    assert_eq!(run.status(), StatusCode::OK);
    let run_json = read_json(run).await;
    let run_id = run_json["run_id"].as_str().expect("run_id").to_string();

    let run_record = h
        .state
        .kernel
        .workflow_engine()
        .get_run(WorkflowRunId(uuid::Uuid::parse_str(&run_id).unwrap()))
        .await
        .expect("run should exist");
    assert_eq!(run_record.account_id.as_deref(), Some("tenant-a"));

    let run_get_b = h
        .send(
            Request::builder()
                .uri(format!("/api/workflows/runs/{run_id}"))
                .header("x-account-id", "tenant-b")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(run_get_b.status(), StatusCode::NOT_FOUND);

    let update_b = h
        .send(
            Request::builder()
                .method("PUT")
                .uri(format!("/api/workflows/{workflow_id}"))
                .header("content-type", "application/json")
                .header("x-account-id", "tenant-b")
                .body(Body::from(
                    serde_json::json!({"name": "tenant-b-overwrite"}).to_string(),
                ))
                .unwrap(),
        )
        .await;
    assert_eq!(update_b.status(), StatusCode::NOT_FOUND);

    let dry_run_b = h
        .send(
            Request::builder()
                .method("POST")
                .uri(format!("/api/workflows/{workflow_id}/dry-run"))
                .header("content-type", "application/json")
                .header("x-account-id", "tenant-b")
                .body(Body::from(
                    serde_json::json!({"input": "hello"}).to_string(),
                ))
                .unwrap(),
        )
        .await;
    assert_eq!(dry_run_b.status(), StatusCode::NOT_FOUND);
}

#[tokio::test(flavor = "multi_thread")]
async fn mt_goals_are_tenant_owned() {
    let h = start_mt_router().await;

    let create = h
        .send(
            Request::builder()
                .method("POST")
                .uri("/api/goals")
                .header("content-type", "application/json")
                .header("x-account-id", "tenant-a")
                .body(Body::from(
                    serde_json::json!({
                        "title": "Tenant A goal",
                        "description": "owned by tenant a",
                        "status": "pending",
                        "progress": 10
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await;
    assert_eq!(create.status(), StatusCode::CREATED);
    let create_json = read_json(create).await;
    let goal_id = create_json["id"].as_str().expect("goal id").to_string();
    assert_eq!(create_json["account_id"], "tenant-a");

    let list_a = h
        .send(
            Request::builder()
                .uri("/api/goals")
                .header("x-account-id", "tenant-a")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(list_a.status(), StatusCode::OK);
    let list_a_json = read_json(list_a).await;
    assert_eq!(list_a_json["total"], 1);

    let list_b = h
        .send(
            Request::builder()
                .uri("/api/goals")
                .header("x-account-id", "tenant-b")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(list_b.status(), StatusCode::OK);
    let list_b_json = read_json(list_b).await;
    assert_eq!(list_b_json["total"], 0);

    let get_b = h
        .send(
            Request::builder()
                .uri(format!("/api/goals/{goal_id}"))
                .header("x-account-id", "tenant-b")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(get_b.status(), StatusCode::NOT_FOUND);

    let update_a = h
        .send(
            Request::builder()
                .method("PUT")
                .uri(format!("/api/goals/{goal_id}"))
                .header("content-type", "application/json")
                .header("x-account-id", "tenant-a")
                .body(Body::from(
                    serde_json::json!({
                        "status": "in_progress",
                        "progress": 55
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await;
    assert_eq!(update_a.status(), StatusCode::OK);
    let update_a_json = read_json(update_a).await;
    assert_eq!(update_a_json["status"], "in_progress");
    assert_eq!(update_a_json["progress"], 55);

    let update_b = h
        .send(
            Request::builder()
                .method("PUT")
                .uri(format!("/api/goals/{goal_id}"))
                .header("content-type", "application/json")
                .header("x-account-id", "tenant-b")
                .body(Body::from(
                    serde_json::json!({"status": "completed"}).to_string(),
                ))
                .unwrap(),
        )
        .await;
    assert_eq!(update_b.status(), StatusCode::NOT_FOUND);

    let delete_b = h
        .send(
            Request::builder()
                .method("DELETE")
                .uri(format!("/api/goals/{goal_id}"))
                .header("x-account-id", "tenant-b")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(delete_b.status(), StatusCode::NOT_FOUND);

    let delete_a = h
        .send(
            Request::builder()
                .method("DELETE")
                .uri(format!("/api/goals/{goal_id}"))
                .header("x-account-id", "tenant-a")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(delete_a.status(), StatusCode::OK);
}

#[tokio::test(flavor = "multi_thread")]
async fn mt_missing_account_header_rejected_on_goals() {
    let h = start_mt_router().await;
    let resp = h
        .send(
            Request::builder()
                .uri("/api/goals")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test(flavor = "multi_thread")]
async fn mt_providers_are_tenant_owned() {
    let h = start_mt_router().await;

    let set_key = h
        .send(
            Request::builder()
                .method("POST")
                .uri("/api/providers/openai/key")
                .header("content-type", "application/json")
                .header("x-account-id", "tenant-a")
                .body(Body::from(r#"{"key":"tenant-a-secret"}"#))
                .unwrap(),
        )
        .await;
    assert_eq!(set_key.status(), StatusCode::OK);

    let set_default = h
        .send(
            Request::builder()
                .method("POST")
                .uri("/api/providers/openai/default")
                .header("x-account-id", "tenant-a")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(set_default.status(), StatusCode::OK);

    let provider_a = h
        .send(
            Request::builder()
                .uri("/api/providers/openai")
                .header("x-account-id", "tenant-a")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(provider_a.status(), StatusCode::OK);
    let provider_a_json = read_json(provider_a).await;
    assert_eq!(provider_a_json["tenant"]["configured"], true);
    assert_eq!(provider_a_json["tenant"]["default"], true);

    let provider_b = h
        .send(
            Request::builder()
                .uri("/api/providers/openai")
                .header("x-account-id", "tenant-b")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(provider_b.status(), StatusCode::OK);
    let provider_b_json = read_json(provider_b).await;
    assert_eq!(provider_b_json["tenant"]["configured"], false);
    assert_eq!(provider_b_json["tenant"]["default"], false);

    let delete_b = h
        .send(
            Request::builder()
                .method("DELETE")
                .uri("/api/providers/openai/key")
                .header("x-account-id", "tenant-b")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(delete_b.status(), StatusCode::NOT_FOUND);
}

#[tokio::test(flavor = "multi_thread")]
async fn mt_missing_account_header_rejected_on_providers() {
    let h = start_mt_router().await;
    let resp = h
        .send(
            Request::builder()
                .uri("/api/providers")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test(flavor = "multi_thread")]
async fn mt_provider_catalog_update_requires_admin_account() {
    let h = start_mt_router().await;
    let resp = h
        .send(
            Request::builder()
                .method("POST")
                .uri("/api/catalog/update")
                .header("x-account-id", "tenant-a")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

#[tokio::test(flavor = "multi_thread")]
async fn mt_hand_activation_binds_instance_agent_to_tenant() {
    let h = start_mt_router().await;
    let hand_id = first_hand_id(&h);

    let resp = h
        .send(
            Request::builder()
                .method("POST")
                .uri(format!("/api/hands/{hand_id}/activate"))
                .header("content-type", "application/json")
                .header("x-account-id", "tenant-a")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await;

    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let agent_id = json["agent_id"]
        .as_str()
        .expect("hand activation should return linked agent_id")
        .parse()
        .expect("agent_id should parse");
    let agent = h
        .state
        .kernel
        .agent_registry()
        .get(agent_id)
        .expect("linked agent should exist");

    assert_eq!(agent.account_id.as_deref(), Some("tenant-a"));
}

#[tokio::test(flavor = "multi_thread")]
async fn mt_cross_tenant_hand_instance_access_returns_404() {
    let h = start_mt_router().await;
    let hand_id = first_hand_id(&h);

    let activate = h
        .send(
            Request::builder()
                .method("POST")
                .uri(format!("/api/hands/{hand_id}/activate"))
                .header("content-type", "application/json")
                .header("x-account-id", "tenant-a")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await;

    assert_eq!(activate.status(), StatusCode::OK);
    let body = axum::body::to_bytes(activate.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let instance_id = json["instance_id"]
        .as_str()
        .expect("hand activation should return instance_id");

    let list = h
        .send(
            Request::builder()
                .uri("/api/hands/active")
                .header("x-account-id", "tenant-b")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(list.status(), StatusCode::OK);
    let body = axum::body::to_bytes(list.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let instances = json["instances"].as_array().expect("instances array");
    assert!(
        instances.is_empty(),
        "other tenants must not see active hand instances they do not own"
    );

    let pause = h
        .send(
            Request::builder()
                .method("POST")
                .uri(format!("/api/hands/instances/{instance_id}/pause"))
                .header("x-account-id", "tenant-b")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(pause.status(), StatusCode::NOT_FOUND);
}

#[tokio::test(flavor = "multi_thread")]
async fn mt_cross_tenant_hand_instance_endpoints_return_404() {
    let h = start_mt_router().await;
    let hand_id = first_hand_id(&h);

    let activate = h
        .send(
            Request::builder()
                .method("POST")
                .uri(format!("/api/hands/{hand_id}/activate"))
                .header("content-type", "application/json")
                .header("x-account-id", "tenant-a")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await;

    assert_eq!(activate.status(), StatusCode::OK);
    let body = axum::body::to_bytes(activate.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let instance_id = json["instance_id"]
        .as_str()
        .expect("hand activation should return instance_id");

    let cases = vec![
        (
            "GET",
            format!("/api/hands/instances/{instance_id}/session"),
            None,
        ),
        (
            "GET",
            format!("/api/hands/instances/{instance_id}/status"),
            None,
        ),
        (
            "GET",
            format!("/api/hands/instances/{instance_id}/stats"),
            None,
        ),
        (
            "GET",
            format!("/api/hands/instances/{instance_id}/browser"),
            None,
        ),
        (
            "POST",
            format!("/api/hands/instances/{instance_id}/message"),
            Some(serde_json::json!({"message": "hi"}).to_string()),
        ),
        (
            "DELETE",
            format!("/api/hands/instances/{instance_id}"),
            None,
        ),
    ];

    for (method, uri, body) in cases {
        let mut builder = Request::builder()
            .method(method)
            .uri(&uri)
            .header("x-account-id", "tenant-b");
        if body.is_some() {
            builder = builder.header("content-type", "application/json");
        }
        let resp = h
            .send(builder.body(Body::from(body.unwrap_or_default())).unwrap())
            .await;
        assert_eq!(resp.status(), StatusCode::NOT_FOUND, "{uri} must be hidden");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn mt_cross_tenant_hand_settings_update_returns_404() {
    let h = start_mt_router().await;
    let hand_id = first_hand_id(&h);

    let activate = h
        .send(
            Request::builder()
                .method("POST")
                .uri(format!("/api/hands/{hand_id}/activate"))
                .header("content-type", "application/json")
                .header("x-account-id", "tenant-a")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await;
    assert_eq!(activate.status(), StatusCode::OK);

    let update = h
        .send(
            Request::builder()
                .method("PUT")
                .uri(format!("/api/hands/{hand_id}/settings"))
                .header("content-type", "application/json")
                .header("x-account-id", "tenant-b")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await;

    assert_eq!(update.status(), StatusCode::NOT_FOUND);
}

#[tokio::test(flavor = "multi_thread")]
async fn st_hand_routes_reject_missing_account_id() {
    let h = start_st_router().await;
    let hand_id = first_hand_id(&h);

    let activate = h
        .send(
            Request::builder()
                .method("POST")
                .uri(format!("/api/hands/{hand_id}/activate"))
                .header("content-type", "application/json")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await;
    assert_eq!(
        activate.status(),
        StatusCode::BAD_REQUEST,
        "tenant-facing hand activation must reject missing X-Account-Id even in single-tenant mode"
    );

    let active = h
        .send(
            Request::builder()
                .uri("/api/hands/active")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(
        active.status(),
        StatusCode::BAD_REQUEST,
        "tenant-facing hand listing must reject missing X-Account-Id even in single-tenant mode"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn mt_hand_message_does_not_inject_cross_tenant_attachment() {
    use librefang_types::message::{ContentBlock, MessageContent};

    let h = start_mt_router().await;
    let hand_id = first_hand_id(&h);

    let activate = h
        .send(
            Request::builder()
                .method("POST")
                .uri(format!("/api/hands/{hand_id}/activate"))
                .header("content-type", "application/json")
                .header("x-account-id", "tenant-b")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await;
    assert_eq!(activate.status(), StatusCode::OK);
    let body = axum::body::to_bytes(activate.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let instance_id = json["instance_id"]
        .as_str()
        .expect("hand activation should return instance_id");
    let agent_id: librefang_types::agent::AgentId = json["agent_id"]
        .as_str()
        .expect("hand activation should return agent_id")
        .parse()
        .expect("agent_id should parse");

    let uploader = make_agent_entry("tenant-a-uploader", Some("tenant-a"));
    let upload_agent_id = uploader.id;
    let _ = h.state.kernel.agent_registry().register(uploader);

    let upload = h
        .send(
            Request::builder()
                .method("POST")
                .uri(format!("/api/agents/{upload_agent_id}/upload"))
                .header("x-account-id", "tenant-a")
                .header("content-type", "image/png")
                .header("x-filename", "foreign.png")
                .body(Body::from("not-really-a-png"))
                .unwrap(),
        )
        .await;
    assert_eq!(upload.status(), StatusCode::CREATED);
    let body = axum::body::to_bytes(upload.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let file_id = json["file_id"].as_str().expect("file_id");

    let send = h
        .send(
            Request::builder()
                .method("POST")
                .uri(format!("/api/hands/instances/{instance_id}/message"))
                .header("content-type", "application/json")
                .header("x-account-id", "tenant-b")
                .body(Body::from(
                    serde_json::json!({
                        "message": "/btw ignore this attachment",
                        "attachments": [{
                            "file_id": file_id,
                            "filename": "foreign.png",
                            "content_type": "image/png"
                        }]
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await;

    assert_ne!(send.status(), StatusCode::NOT_FOUND);

    let entry = h
        .state
        .kernel
        .agent_registry()
        .get(agent_id)
        .expect("hand-linked agent should exist");
    let session = h
        .state
        .kernel
        .memory_substrate()
        .get_session(entry.session_id)
        .expect("session lookup should succeed");

    let has_image_block = session
        .as_ref()
        .map(|session| {
            session
                .messages
                .iter()
                .any(|message| match &message.content {
                    MessageContent::Blocks(blocks) => blocks
                        .iter()
                        .any(|block| matches!(block, ContentBlock::Image { .. })),
                    _ => false,
                })
        })
        .unwrap_or(false);

    assert!(
        !has_image_block,
        "foreign tenant upload must not be injected into hand session"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn mt_websocket_message_does_not_inject_cross_tenant_attachment() {
    use librefang_types::message::{ContentBlock, MessageContent};
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;
    use tokio_tungstenite::tungstenite::Message;

    let h = start_mt_http_router().await;
    let agent = make_agent_entry("tenant-b-ws-agent", Some("tenant-b"));
    let agent_id = agent.id;
    let _ = h.state.kernel.agent_registry().register(agent);

    let uploader = make_agent_entry("tenant-a-uploader", Some("tenant-a"));
    let upload_agent_id = uploader.id;
    let _ = h.state.kernel.agent_registry().register(uploader);

    let http = reqwest::Client::new();
    let upload = http
        .post(format!(
            "{}/api/agents/{upload_agent_id}/upload",
            h.base_url
        ))
        .header("x-account-id", "tenant-a")
        .header("content-type", "image/png")
        .header("x-filename", "foreign.png")
        .body("not-really-a-png")
        .send()
        .await
        .expect("upload request should succeed");
    assert_eq!(upload.status(), reqwest::StatusCode::CREATED);
    let json: serde_json::Value = upload.json().await.expect("upload JSON");
    let file_id = json["file_id"].as_str().expect("file_id");

    let ws_url = format!("{}/api/agents/{agent_id}/ws", h.base_url).replace("http://", "ws://");
    let mut request = ws_url
        .into_client_request()
        .expect("websocket request should build");
    request.headers_mut().insert(
        "x-account-id",
        axum::http::HeaderValue::from_static("tenant-b"),
    );
    let (mut socket, _) = tokio_tungstenite::connect_async(request)
        .await
        .expect("websocket connect should succeed");

    let connected = socket
        .next()
        .await
        .expect("connected frame should exist")
        .expect("connected frame should parse");
    let connected_text = connected.into_text().expect("connected text frame");
    let connected_json: serde_json::Value =
        serde_json::from_str(&connected_text).expect("connected payload should be JSON");
    assert_eq!(connected_json["type"], "connected");

    socket
        .send(Message::Text(
            serde_json::json!({
                "type": "message",
                "content": "check foreign upload",
                "attachments": [{
                    "file_id": file_id,
                    "filename": "foreign.png",
                    "content_type": "image/png"
                }]
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("message send should succeed");

    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
    while tokio::time::Instant::now() < deadline {
        match tokio::time::timeout(std::time::Duration::from_millis(500), socket.next()).await {
            Ok(Some(Ok(Message::Text(text)))) => {
                let json: serde_json::Value =
                    serde_json::from_str(&text).expect("ws payload should be JSON");
                let msg_type = json["type"].as_str().unwrap_or_default();
                if matches!(msg_type, "response" | "error" | "silent_complete") {
                    break;
                }
            }
            Ok(Some(Ok(_))) => {}
            Ok(Some(Err(_))) | Ok(None) => break,
            Err(_) => {}
        }
    }

    let entry = h
        .state
        .kernel
        .agent_registry()
        .get(agent_id)
        .expect("ws agent should exist");
    let session = h
        .state
        .kernel
        .memory_substrate()
        .get_session(entry.session_id)
        .expect("session lookup should succeed");

    let has_image_block = session
        .as_ref()
        .map(|session| {
            session
                .messages
                .iter()
                .any(|message| match &message.content {
                    MessageContent::Blocks(blocks) => blocks
                        .iter()
                        .any(|block| matches!(block, ContentBlock::Image { .. })),
                    _ => false,
                })
        })
        .unwrap_or(false);

    assert!(
        !has_image_block,
        "foreign tenant upload must not be injected into websocket session"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn mt_openai_streaming_chat_completions_hides_cross_tenant_agent() {
    let h = start_mt_router().await;
    let entry = make_agent_entry("tenant-a-agent-stream", Some("tenant-a"));
    let agent_id = entry.id;
    let _ = h.state.kernel.agent_registry().register(entry);

    let resp = h
        .send(
            Request::builder()
                .method("POST")
                .uri("/v1/chat/completions")
                .header("content-type", "application/json")
                .header("x-account-id", "tenant-b")
                .body(Body::from(
                    serde_json::json!({
                        "model": agent_id.to_string(),
                        "stream": true,
                        "messages": [{"role": "user", "content": "hello"}]
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await;

    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test(flavor = "multi_thread")]
async fn mt_tenant_can_create_agent_bound_schedule() {
    let h = start_mt_router().await;
    let entry = make_agent_entry("scheduled-agent", Some("tenant-a"));
    let agent_id = entry.id;
    let _ = h.state.kernel.agent_registry().register(entry);

    let resp = h
        .send(
            Request::builder()
                .method("POST")
                .uri("/api/schedules")
                .header("content-type", "application/json")
                .header("x-account-id", "tenant-a")
                .body(Body::from(
                    serde_json::json!({
                        "name": "tenant-agent-schedule",
                        "cron": "0 * * * *",
                        "agent_id": agent_id.to_string(),
                        "message": "scheduled"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await;

    assert_eq!(resp.status(), StatusCode::CREATED);
}

#[tokio::test(flavor = "multi_thread")]
async fn mt_tenant_workflow_schedule_is_scoped() {
    let h = start_mt_router().await;
    let workflow = Workflow {
        account_id: Some("tenant-a".to_string()),
        ..make_workflow("tenant-a-scheduled-workflow")
    };
    let workflow_id = h.state.kernel.register_workflow(workflow).await;

    let create = h
        .send(
            Request::builder()
                .method("POST")
                .uri("/api/schedules")
                .header("content-type", "application/json")
                .header("x-account-id", "tenant-a")
                .body(Body::from(
                    serde_json::json!({
                        "name": "tenant-workflow-schedule",
                        "cron": "0 * * * *",
                        "workflow_id": workflow_id.to_string(),
                        "message": "scheduled workflow input"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await;
    assert_eq!(create.status(), StatusCode::CREATED);
    let create_json = read_json(create).await;
    let schedule_id = create_json["id"].as_str().expect("schedule id");

    let tenant_jobs = h.state.kernel.cron().list_jobs_by_account("tenant-a");
    assert_eq!(tenant_jobs.len(), 1);
    assert_eq!(tenant_jobs[0].account_id.as_deref(), Some("tenant-a"));

    let get_b = h
        .send(
            Request::builder()
                .uri(format!("/api/schedules/{schedule_id}"))
                .header("x-account-id", "tenant-b")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(get_b.status(), StatusCode::NOT_FOUND);

    let run = h
        .send(
            Request::builder()
                .method("POST")
                .uri(format!("/api/schedules/{schedule_id}/run"))
                .header("x-account-id", "tenant-a")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(run.status(), StatusCode::OK);

    let delete_b = h
        .send(
            Request::builder()
                .method("DELETE")
                .uri(format!("/api/schedules/{schedule_id}"))
                .header("x-account-id", "tenant-b")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(delete_b.status(), StatusCode::NOT_FOUND);
}

#[tokio::test(flavor = "multi_thread")]
async fn mt_same_named_workflow_schedules_stay_tenant_isolated() {
    let h = start_mt_router().await;
    let workflow_a = Workflow {
        account_id: Some("tenant-a".to_string()),
        ..make_workflow("shared-workflow-name")
    };
    let workflow_b = Workflow {
        account_id: Some("tenant-b".to_string()),
        ..make_workflow("shared-workflow-name")
    };
    let workflow_a_id = h.state.kernel.register_workflow(workflow_a).await;
    let workflow_b_id = h.state.kernel.register_workflow(workflow_b).await;

    let create_a = h
        .send(
            Request::builder()
                .method("POST")
                .uri("/api/schedules")
                .header("content-type", "application/json")
                .header("x-account-id", "tenant-a")
                .body(Body::from(
                    serde_json::json!({
                        "name": "tenant-a-shared-schedule",
                        "cron": "0 * * * *",
                        "workflow_id": workflow_a_id.to_string(),
                        "message": "tenant a input"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await;
    assert_eq!(create_a.status(), StatusCode::CREATED);
    let create_a_json = read_json(create_a).await;
    let schedule_a_id = create_a_json["id"].as_str().expect("schedule id");

    let create_b = h
        .send(
            Request::builder()
                .method("POST")
                .uri("/api/schedules")
                .header("content-type", "application/json")
                .header("x-account-id", "tenant-b")
                .body(Body::from(
                    serde_json::json!({
                        "name": "tenant-b-shared-schedule",
                        "cron": "0 * * * *",
                        "workflow_id": workflow_b_id.to_string(),
                        "message": "tenant b input"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await;
    assert_eq!(create_b.status(), StatusCode::CREATED);
    let create_b_json = read_json(create_b).await;
    let schedule_b_id = create_b_json["id"].as_str().expect("schedule id");

    let run_a = h
        .send(
            Request::builder()
                .method("POST")
                .uri(format!("/api/schedules/{schedule_a_id}/run"))
                .header("x-account-id", "tenant-a")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(run_a.status(), StatusCode::OK);

    let run_b = h
        .send(
            Request::builder()
                .method("POST")
                .uri(format!("/api/schedules/{schedule_b_id}/run"))
                .header("x-account-id", "tenant-b")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(run_b.status(), StatusCode::OK);

    let runs_a = h
        .state
        .kernel
        .workflow_engine()
        .list_runs_by_account("tenant-a", None)
        .await;
    let runs_b = h
        .state
        .kernel
        .workflow_engine()
        .list_runs_by_account("tenant-b", None)
        .await;
    assert!(runs_a.iter().all(|run| run.workflow_id == workflow_a_id));
    assert!(runs_b.iter().all(|run| run.workflow_id == workflow_b_id));
}

#[tokio::test(flavor = "multi_thread")]
async fn mt_deleting_one_tenant_workflow_only_removes_own_schedules() {
    let h = start_mt_router().await;
    let workflow_a = Workflow {
        account_id: Some("tenant-a".to_string()),
        ..make_workflow("tenant-a-cleanup-workflow")
    };
    let workflow_b = Workflow {
        account_id: Some("tenant-b".to_string()),
        ..make_workflow("tenant-b-cleanup-workflow")
    };
    let workflow_a_id = h.state.kernel.register_workflow(workflow_a).await;
    let workflow_b_id = h.state.kernel.register_workflow(workflow_b).await;

    let schedule_a = read_json(
        h.send(
            Request::builder()
                .method("POST")
                .uri("/api/schedules")
                .header("content-type", "application/json")
                .header("x-account-id", "tenant-a")
                .body(Body::from(
                    serde_json::json!({
                        "name": "tenant-a-cleanup-schedule",
                        "cron": "0 * * * *",
                        "workflow_id": workflow_a_id.to_string(),
                        "message": "tenant a input"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await,
    )
    .await;
    let schedule_a_id = schedule_a["id"].as_str().expect("schedule a id");

    let schedule_b = read_json(
        h.send(
            Request::builder()
                .method("POST")
                .uri("/api/schedules")
                .header("content-type", "application/json")
                .header("x-account-id", "tenant-b")
                .body(Body::from(
                    serde_json::json!({
                        "name": "tenant-b-cleanup-schedule",
                        "cron": "0 * * * *",
                        "workflow_id": workflow_b_id.to_string(),
                        "message": "tenant b input"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await,
    )
    .await;
    let schedule_b_id = schedule_b["id"].as_str().expect("schedule b id");

    let delete_a = h
        .send(
            Request::builder()
                .method("DELETE")
                .uri(format!("/api/workflows/{workflow_a_id}"))
                .header("x-account-id", "tenant-a")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(delete_a.status(), StatusCode::OK);

    assert!(h
        .state
        .kernel
        .cron()
        .list_jobs_by_account("tenant-a")
        .is_empty());
    let tenant_b_jobs = h.state.kernel.cron().list_jobs_by_account("tenant-b");
    assert_eq!(tenant_b_jobs.len(), 1);
    assert_eq!(tenant_b_jobs[0].id.to_string(), schedule_b_id);

    let get_removed = h
        .send(
            Request::builder()
                .uri(format!("/api/schedules/{schedule_a_id}"))
                .header("x-account-id", "tenant-a")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(get_removed.status(), StatusCode::NOT_FOUND);

    let get_b = h
        .send(
            Request::builder()
                .uri(format!("/api/schedules/{schedule_b_id}"))
                .header("x-account-id", "tenant-b")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(get_b.status(), StatusCode::OK);
}

#[tokio::test(flavor = "multi_thread")]
async fn mt_admin_template_instantiation_preserves_request_account_scope() {
    let admin_id = "admin-tenant";
    let h = start_mt_router_with_admin(admin_id).await;
    h.state
        .kernel
        .templates()
        .register(make_template("tmpl-1", "Template Workflow"))
        .await;

    let resp = h
        .send(
            Request::builder()
                .method("POST")
                .uri("/api/workflow-templates/tmpl-1/instantiate")
                .header("content-type", "application/json")
                .header("x-account-id", admin_id)
                .body(Body::from(
                    serde_json::json!({"subject": "tenant-owned"}).to_string(),
                ))
                .unwrap(),
        )
        .await;

    assert_eq!(resp.status(), StatusCode::CREATED);
    let json = read_json(resp).await;
    let workflow_id = WorkflowId(json["workflow_id"].as_str().unwrap().parse().unwrap());

    let workflow = h
        .state
        .kernel
        .workflow_engine()
        .get_workflow(workflow_id)
        .await
        .expect("instantiated workflow");
    assert_eq!(workflow.account_id.as_deref(), Some(admin_id));
}

#[tokio::test(flavor = "multi_thread")]
async fn mt_admin_cron_create_preserves_target_agent_account_scope() {
    let admin_id = "admin-tenant";
    let h = start_mt_router_with_admin(admin_id).await;
    let entry = make_agent_entry("tenant-a-cron-target", Some("tenant-a"));
    let agent_id = entry.id;
    let _ = h.state.kernel.agent_registry().register(entry);

    let resp = h
        .send(
            Request::builder()
                .method("POST")
                .uri("/api/cron/jobs")
                .header("content-type", "application/json")
                .header("x-account-id", admin_id)
                .body(Body::from(
                    serde_json::json!({
                        "agent_id": agent_id.to_string(),
                        "name": "tenant-owned-admin-cron",
                        "schedule": {"kind": "every", "every_secs": 3600},
                        "action": {"kind": "agent_turn", "message": "scheduled by admin"},
                        "delivery": {"kind": "none"},
                        "one_shot": false
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await;

    assert_eq!(resp.status(), StatusCode::CREATED);
    let json = read_json(resp).await;
    let job_id = librefang_types::scheduler::CronJobId(
        uuid::Uuid::parse_str(json["job_id"].as_str().expect("job_id")).unwrap(),
    );
    let job = h.state.kernel.cron().get_job(job_id).expect("cron job");
    assert_eq!(job.account_id.as_deref(), Some("tenant-a"));
}

#[tokio::test(flavor = "multi_thread")]
async fn mt_tenant_trigger_is_scoped() {
    let h = start_mt_router().await;
    let entry = make_agent_entry("trigger-agent", Some("tenant-a"));
    let agent_id = entry.id;
    let _ = h.state.kernel.agent_registry().register(entry);

    let create = h
        .send(
            Request::builder()
                .method("POST")
                .uri("/api/triggers")
                .header("content-type", "application/json")
                .header("x-account-id", "tenant-a")
                .body(Body::from(
                    serde_json::json!({
                        "agent_id": agent_id.to_string(),
                        "pattern": {"system_keyword": {"keyword": "deploy"}},
                        "prompt_template": "Event: {{event}}"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await;
    assert_eq!(create.status(), StatusCode::CREATED);
    let create_json = read_json(create).await;
    let trigger_id = create_json["trigger_id"].as_str().expect("trigger id");

    let tenant_triggers = h.state.kernel.trigger_engine().list_by_account("tenant-a");
    assert_eq!(tenant_triggers.len(), 1);
    assert_eq!(tenant_triggers[0].account_id.as_deref(), Some("tenant-a"));

    let list_b = h
        .send(
            Request::builder()
                .uri("/api/triggers")
                .header("x-account-id", "tenant-b")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(list_b.status(), StatusCode::OK);
    let list_b_json = read_json(list_b).await;
    let triggers = list_b_json["triggers"].as_array().expect("triggers array");
    assert!(triggers.is_empty());

    let delete_b = h
        .send(
            Request::builder()
                .method("DELETE")
                .uri(format!("/api/triggers/{trigger_id}"))
                .header("x-account-id", "tenant-b")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(delete_b.status(), StatusCode::NOT_FOUND);
}

#[tokio::test(flavor = "multi_thread")]
async fn mt_tenant_cannot_access_other_tenant_registered_upload() {
    let h = start_mt_router().await;
    let entry = make_agent_entry("uploader-agent", Some("tenant-a"));
    let agent_id = entry.id;
    let _ = h.state.kernel.agent_registry().register(entry);

    let upload = h
        .send(
            Request::builder()
                .method("POST")
                .uri(format!("/api/agents/{agent_id}/upload"))
                .header("x-account-id", "tenant-a")
                .header("content-type", "text/plain")
                .header("x-filename", "note.txt")
                .body(Body::from("secret"))
                .unwrap(),
        )
        .await;
    assert_eq!(upload.status(), StatusCode::CREATED);
    let body = axum::body::to_bytes(upload.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let file_id = json["file_id"].as_str().expect("file_id");

    let resp = h
        .send(
            Request::builder()
                .uri(format!("/api/uploads/{file_id}"))
                .header("x-account-id", "tenant-b")
                .body(Body::empty())
                .unwrap(),
        )
        .await;

    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test(flavor = "multi_thread")]
async fn mt_unregistered_upload_fallback_is_hidden_from_tenants() {
    let file_id = uuid::Uuid::new_v4().to_string();
    let upload_dir = std::env::temp_dir().join("librefang_uploads");
    std::fs::create_dir_all(&upload_dir).unwrap();
    std::fs::write(upload_dir.join(&file_id), b"fallback-bytes").unwrap();

    let mt = start_mt_router().await;
    let tenant_resp = mt
        .send(
            Request::builder()
                .uri(format!("/api/uploads/{file_id}"))
                .header("x-account-id", "tenant-a")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(tenant_resp.status(), StatusCode::NOT_FOUND);

    let st = start_st_router().await;
    let admin_resp = st
        .send(
            Request::builder()
                .uri(format!("/api/uploads/{file_id}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(admin_resp.status(), StatusCode::OK);
}

#[tokio::test(flavor = "multi_thread")]
async fn mt_media_providers_requires_admin_account() {
    let h = start_mt_router().await;
    let resp = h
        .send(
            Request::builder()
                .uri("/api/media/providers")
                .header("x-account-id", "tenant-a")
                .body(Body::empty())
                .unwrap(),
        )
        .await;

    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

#[tokio::test(flavor = "multi_thread")]
async fn mt_media_providers_allows_admin_account() {
    let h = start_mt_router_with_admin("admin-tenant").await;
    let resp = h
        .send(
            Request::builder()
                .uri("/api/media/providers")
                .header("x-account-id", "admin-tenant")
                .body(Body::empty())
                .unwrap(),
        )
        .await;

    assert_eq!(resp.status(), StatusCode::OK);
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
        let content = item.get("content").and_then(|v| v.as_str()).unwrap_or("");
        assert!(
            !content.contains("hijacked"),
            "Memory content must not be modified by cross-tenant update, got: {content}"
        );
    }
}

// ---------------------------------------------------------------------------
// Missing-account rejection: tenant-facing memory routes must not widen scope
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn test_memory_search_rejects_missing_account_even_without_middleware_gate() {
    let h = start_st_router().await;

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
        StatusCode::BAD_REQUEST,
        "Tenant-facing memory search must reject missing X-Account-Id instead of widening scope"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_memory_list_rejects_missing_account_even_without_middleware_gate() {
    let h = start_st_router().await;

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
        StatusCode::BAD_REQUEST,
        "Tenant-facing memory list must reject missing X-Account-Id instead of widening scope"
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
                .uri(format!(
                    "/api/memory/agents/{agent_id}/search?q=test&limit=5"
                ))
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
    let h = start_mt_router_with_admin("tenant-admin").await;

    let resp = h
        .send(
            Request::builder()
                .uri("/api/memory/config")
                .header("x-account-id", "tenant-admin")
                .body(Body::empty())
                .unwrap(),
        )
        .await;

    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "GET /api/memory/config should return 200 for configured admin, got: {}",
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

// ---------------------------------------------------------------------------
// System endpoints: admin-only (require_admin blocks scoped tenants)
// ---------------------------------------------------------------------------

/// Assert that a system endpoint returns 403 for a scoped tenant.
async fn assert_system_admin_only(h: &MtHarness, method: &str, uri: &str) {
    let req = Request::builder()
        .method(method)
        .uri(uri)
        .header("x-account-id", "tenant-blocked")
        .header("content-type", "application/json")
        .body(Body::empty())
        .unwrap();
    let resp = h.send(req).await;
    assert_eq!(
        resp.status(),
        StatusCode::FORBIDDEN,
        "{method} {uri} must return 403 for scoped tenant, got: {}",
        resp.status()
    );
}

/// Assert that a system endpoint does not return 403 for a configured admin account.
async fn assert_system_admin_allowed(h: &MtHarness, method: &str, uri: &str, account_id: &str) {
    let req = Request::builder()
        .method(method)
        .uri(uri)
        .header("x-account-id", account_id)
        .header("content-type", "application/json")
        .body(Body::empty())
        .unwrap();
    let resp = h.send(req).await;
    assert_ne!(
        resp.status(),
        StatusCode::FORBIDDEN,
        "{method} {uri} must NOT return 403 for configured admin account, got: {}",
        resp.status()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_system_sessions_admin_only() {
    let h = start_mt_router().await;
    assert_system_admin_only(&h, "GET", "/api/sessions").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn test_system_audit_admin_only() {
    let h = start_mt_router().await;
    assert_system_admin_only(&h, "GET", "/api/audit/recent").await;
    assert_system_admin_only(&h, "GET", "/api/audit/verify").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn test_system_tools_admin_only() {
    let h = start_mt_router().await;
    assert_system_admin_only(&h, "GET", "/api/tools").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn test_system_templates_admin_only() {
    let h = start_mt_router().await;
    assert_system_admin_only(&h, "GET", "/api/templates").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn test_system_backup_admin_only() {
    let h = start_mt_router().await;
    assert_system_admin_only(&h, "POST", "/api/backup").await;
    assert_system_admin_only(&h, "GET", "/api/backups").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn test_system_webhooks_admin_only() {
    let h = start_mt_router().await;
    assert_system_admin_only(&h, "GET", "/api/webhooks/events").await;
    assert_system_admin_only(&h, "GET", "/api/webhooks").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn test_system_pairing_admin_only() {
    let h = start_mt_router().await;
    assert_system_admin_only(&h, "GET", "/api/pairing/devices").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn test_system_queue_admin_only() {
    let h = start_mt_router().await;
    assert_system_admin_only(&h, "GET", "/api/queue/status").await;
    assert_system_admin_only(&h, "GET", "/api/tasks/status").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn test_system_registry_admin_only() {
    let h = start_mt_router().await;
    assert_system_admin_only(&h, "GET", "/api/registry/schema").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn test_system_bindings_admin_only() {
    let h = start_mt_router().await;
    assert_system_admin_only(&h, "GET", "/api/bindings").await;
    // POST with valid AgentBinding JSON
    let req = Request::builder()
        .method("POST")
        .uri("/api/bindings")
        .header("x-account-id", "tenant-blocked")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::to_vec(&serde_json::json!({"agent": "test-agent", "match_rule": {}}))
                .unwrap(),
        ))
        .unwrap();
    let resp = h.send(req).await;
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_system_kv_admin_only() {
    let h = start_mt_router().await;
    assert_system_admin_only(
        &h,
        "GET",
        "/api/memory/agents/00000000-0000-0000-0000-000000000000/kv",
    )
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn test_system_approvals_admin_only() {
    let h = start_mt_router().await;
    assert_system_admin_only(&h, "GET", "/api/approvals").await;
    assert_system_admin_only(&h, "GET", "/api/approvals/some-id").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn test_system_admin_passes_with_configured_admin_account() {
    let h = start_mt_router_with_admin("tenant-admin").await;
    assert_system_admin_allowed(&h, "GET", "/api/sessions", "tenant-admin").await;
    assert_system_admin_allowed(&h, "GET", "/api/audit/recent", "tenant-admin").await;
    assert_system_admin_allowed(&h, "GET", "/api/tools", "tenant-admin").await;
    assert_system_admin_allowed(&h, "GET", "/api/backups", "tenant-admin").await;
    assert_system_admin_allowed(&h, "GET", "/api/queue/status", "tenant-admin").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn test_skills_admin_surfaces_return_403_for_tenants() {
    let h = start_mt_router().await;

    for (method, uri, body) in [
        ("GET", "/api/skills", None),
        ("POST", "/api/skills/reload", None),
        (
            "POST",
            "/api/skills/install",
            Some(serde_json::json!({"name": "nonexistent-skill"}).to_string()),
        ),
        ("POST", "/api/hands/reload", None),
    ] {
        let mut req = Request::builder()
            .method(method)
            .uri(uri)
            .header("x-account-id", "tenant-skills");
        if body.is_some() {
            req = req.header("content-type", "application/json");
        }
        let resp = h
            .send(req.body(Body::from(body.unwrap_or_default())).unwrap())
            .await;
        assert_eq!(
            resp.status(),
            StatusCode::FORBIDDEN,
            "{method} {uri} must return 403 for tenant-scoped callers"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn test_system_tier4_public_accessible_by_tenant() {
    let h = start_mt_router().await;
    for uri in &["/api/profiles", "/api/commands", "/api/versions"] {
        let req = Request::builder()
            .uri(*uri)
            .header("x-account-id", "tenant-public")
            .body(Body::empty())
            .unwrap();
        let resp = h.send(req).await;
        assert_ne!(
            resp.status(),
            StatusCode::FORBIDDEN,
            "GET {uri} is Tier 4 Public and must NOT return 403 for tenants, got: {}",
            resp.status()
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn test_system_public_subset_allows_missing_account_header() {
    let h = start_mt_router().await;

    for uri in &[
        "/api/profiles",
        "/api/profiles/minimal",
        "/api/commands",
        "/api/commands/help",
        "/api/versions",
    ] {
        let resp = h
            .send(Request::builder().uri(*uri).body(Body::empty()).unwrap())
            .await;
        assert_ne!(
            resp.status(),
            StatusCode::BAD_REQUEST,
            "GET {uri} should be explicitly public and not require X-Account-Id"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn test_system_non_public_endpoints_reject_missing_account_header() {
    let h = start_mt_router().await;

    for uri in &["/api/templates", "/api/queue/status"] {
        let resp = h
            .send(Request::builder().uri(*uri).body(Body::empty()).unwrap())
            .await;
        assert_eq!(
            resp.status(),
            StatusCode::BAD_REQUEST,
            "GET {uri} must require X-Account-Id because it is not public"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn test_system_tenant_derived_memory_export_is_owned_and_no_fallback() {
    let h = start_mt_router().await;
    let entry = make_agent_entry("memory-owner", Some("tenant-a"));
    let agent_id = entry.id;
    let _ = h.state.kernel.agent_registry().register(entry);

    let owner_resp = h
        .send(
            Request::builder()
                .uri(format!("/api/agents/{agent_id}/memory/export"))
                .header("x-account-id", "tenant-a")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(owner_resp.status(), StatusCode::OK);

    let other_resp = h
        .send(
            Request::builder()
                .uri(format!("/api/agents/{agent_id}/memory/export"))
                .header("x-account-id", "tenant-b")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(other_resp.status(), StatusCode::NOT_FOUND);

    let missing_account = h
        .send(
            Request::builder()
                .uri(format!("/api/agents/{agent_id}/memory/export"))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(missing_account.status(), StatusCode::BAD_REQUEST);
}

// ---------------------------------------------------------------------------
// Admin accounts: configured admin can access admin-guarded endpoints
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn test_admin_account_passes_require_admin_in_multitenant() {
    let h = start_mt_router_with_admin("admin-tenant").await;

    // Admin account should NOT get 403 on admin-guarded endpoints
    for uri in &[
        "/api/sessions",
        "/api/audit/recent",
        "/api/tools",
        "/api/backups",
        "/api/queue/status",
    ] {
        let req = Request::builder()
            .uri(*uri)
            .header("x-account-id", "admin-tenant")
            .body(Body::empty())
            .unwrap();
        let resp = h.send(req).await;
        assert_ne!(
            resp.status(),
            StatusCode::FORBIDDEN,
            "GET {uri} must NOT return 403 for configured admin account, got: {}",
            resp.status()
        );
    }

    // Non-admin tenant should STILL get 403
    for uri in &["/api/sessions", "/api/tools"] {
        let req = Request::builder()
            .uri(*uri)
            .header("x-account-id", "regular-tenant")
            .body(Body::empty())
            .unwrap();
        let resp = h.send(req).await;
        assert_eq!(
            resp.status(),
            StatusCode::FORBIDDEN,
            "GET {uri} must return 403 for non-admin tenant"
        );
    }
}

// ---------------------------------------------------------------------------
// Event webhooks: admin can manage in multi-tenant mode
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn test_event_webhooks_admin_can_manage() {
    let h = start_mt_router_with_admin("admin-tenant").await;

    // Admin can list event webhooks
    let req = Request::builder()
        .uri("/api/webhooks/events")
        .header("x-account-id", "admin-tenant")
        .body(Body::empty())
        .unwrap();
    let resp = h.send(req).await;
    assert_ne!(
        resp.status(),
        StatusCode::FORBIDDEN,
        "GET /api/webhooks/events must NOT return 403 for admin, got: {}",
        resp.status()
    );

    // Admin can create event webhook
    let req = Request::builder()
        .method("POST")
        .uri("/api/webhooks/events")
        .header("x-account-id", "admin-tenant")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::to_vec(&serde_json::json!({
                "url": "https://example.com/hook",
                "events": ["agent.spawned"]
            }))
            .unwrap(),
        ))
        .unwrap();
    let resp = h.send(req).await;
    assert_ne!(
        resp.status(),
        StatusCode::FORBIDDEN,
        "POST /api/webhooks/events must NOT return 403 for admin, got: {}",
        resp.status()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn mt_missing_account_header_rejected_on_usage() {
    let h = start_mt_router().await;
    let resp = h
        .send(
            Request::builder()
                .uri("/api/usage")
                .body(Body::empty())
                .unwrap(),
        )
        .await;

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test(flavor = "multi_thread")]
async fn mt_budget_admin_only_endpoints_return_403_for_non_admin() {
    let h = start_mt_router_with_admin("admin-tenant").await;

    for uri in ["/api/budget", "/api/usage/by-model", "/api/usage/daily"] {
        let resp = h
            .send(
                Request::builder()
                    .uri(uri)
                    .header("x-account-id", "tenant-a")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await;

        assert_eq!(
            resp.status(),
            StatusCode::FORBIDDEN,
            "{uri} must be admin-only"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn mt_budget_usage_and_ranking_are_scoped_to_owned_agents() {
    let h = start_mt_router().await;

    let tenant_a = make_agent_entry("tenant-a-agent", Some("tenant-a"));
    let tenant_b = make_agent_entry("tenant-b-agent", Some("tenant-b"));
    let tenant_a_id = tenant_a.id;
    let tenant_b_id = tenant_b.id;
    let _ = h.state.kernel.agent_registry().register(tenant_a);
    let _ = h.state.kernel.agent_registry().register(tenant_b);

    h.state
        .kernel
        .metering_ref()
        .record(&librefang_memory::usage::UsageRecord {
            agent_id: tenant_a_id,
            model: "model-a".to_string(),
            input_tokens: 10,
            output_tokens: 20,
            cost_usd: 1.25,
            tool_calls: 0,
            latency_ms: 10,
        })
        .unwrap();
    h.state
        .kernel
        .metering_ref()
        .record(&librefang_memory::usage::UsageRecord {
            agent_id: tenant_b_id,
            model: "model-b".to_string(),
            input_tokens: 30,
            output_tokens: 40,
            cost_usd: 9.75,
            tool_calls: 0,
            latency_ms: 10,
        })
        .unwrap();

    let usage_resp = h
        .send(
            Request::builder()
                .uri("/api/usage")
                .header("x-account-id", "tenant-a")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(usage_resp.status(), StatusCode::OK);
    let usage_json = read_json(usage_resp).await;
    let usage_agents = usage_json["agents"].as_array().unwrap();
    assert_eq!(usage_agents.len(), 1);
    assert_eq!(usage_agents[0]["agent_id"], tenant_a_id.to_string());

    let summary_resp = h
        .send(
            Request::builder()
                .uri("/api/usage/summary")
                .header("x-account-id", "tenant-a")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(summary_resp.status(), StatusCode::OK);
    let summary_json = read_json(summary_resp).await;
    assert_eq!(summary_json["total_cost_usd"], 1.25);

    let ranking_resp = h
        .send(
            Request::builder()
                .uri("/api/budget/agents")
                .header("x-account-id", "tenant-a")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(ranking_resp.status(), StatusCode::OK);
    let ranking_json = read_json(ranking_resp).await;
    let ranking_agents = ranking_json["agents"].as_array().unwrap();
    assert_eq!(ranking_agents.len(), 1);
    assert_eq!(ranking_agents[0]["agent_id"], tenant_a_id.to_string());
}

#[tokio::test(flavor = "multi_thread")]
async fn mt_budget_agent_status_returns_404_cross_tenant() {
    let h = start_mt_router().await;

    let tenant_a = make_agent_entry("tenant-a-agent", Some("tenant-a"));
    let tenant_a_id = tenant_a.id;
    let _ = h.state.kernel.agent_registry().register(tenant_a);

    let resp = h
        .send(
            Request::builder()
                .uri(format!("/api/budget/agents/{tenant_a_id}"))
                .header("x-account-id", "tenant-b")
                .body(Body::empty())
                .unwrap(),
        )
        .await;

    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test(flavor = "multi_thread")]
async fn mt_memory_relations_are_invisible_across_tenants() {
    let h = start_mt_router().await;

    let tenant_a = make_agent_entry("tenant-a-memory-agent", Some("tenant-a"));
    let tenant_b = make_agent_entry("tenant-b-memory-agent", Some("tenant-b"));
    let tenant_a_id = tenant_a.id;
    let tenant_b_id = tenant_b.id;
    let _ = h.state.kernel.agent_registry().register(tenant_a);
    let _ = h.state.kernel.agent_registry().register(tenant_b);

    let create_resp = h
        .send(
            Request::builder()
                .method("POST")
                .uri(format!("/api/memory/agents/{tenant_a_id}/relations"))
                .header("x-account-id", "tenant-a")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&serde_json::json!([{
                        "subject": "Alice",
                        "subject_type": "person",
                        "relation": "works_at",
                        "object": "Acme",
                        "object_type": "organization"
                    }]))
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await;
    assert_eq!(create_resp.status(), StatusCode::OK);

    let own_resp = h
        .send(
            Request::builder()
                .uri(format!(
                    "/api/memory/agents/{tenant_a_id}/relations?source=Alice&relation=works_at"
                ))
                .header("x-account-id", "tenant-a")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(own_resp.status(), StatusCode::OK);
    let own_json = read_json(own_resp).await;
    assert_eq!(own_json["count"], 1);

    let cross_agent_resp = h
        .send(
            Request::builder()
                .uri(format!(
                    "/api/memory/agents/{tenant_b_id}/relations?source=Alice&relation=works_at"
                ))
                .header("x-account-id", "tenant-b")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(cross_agent_resp.status(), StatusCode::OK);
    let cross_agent_json = read_json(cross_agent_resp).await;
    assert_eq!(cross_agent_json["count"], 0);

    let wrong_tenant_resp = h
        .send(
            Request::builder()
                .uri(format!(
                    "/api/memory/agents/{tenant_a_id}/relations?source=Alice&relation=works_at"
                ))
                .header("x-account-id", "tenant-b")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(wrong_tenant_resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test(flavor = "multi_thread")]
async fn mt_network_protocol_routes_reject_missing_account_and_forbid_non_admin() {
    let h = start_mt_router_with_admin("admin-tenant").await;

    let missing = h
        .send(
            Request::builder()
                .uri("/.well-known/agent.json")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(missing.status(), StatusCode::BAD_REQUEST);

    let non_admin = h
        .send(
            Request::builder()
                .uri("/.well-known/agent.json")
                .header("x-account-id", "tenant-a")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(non_admin.status(), StatusCode::FORBIDDEN);
}

#[tokio::test(flavor = "multi_thread")]
async fn mt_network_admin_only_endpoints_return_403_for_non_admin() {
    let h = start_mt_router_with_admin("admin-tenant").await;

    for uri in ["/api/network/status", "/api/a2a/agents"] {
        let resp = h
            .send(
                Request::builder()
                    .uri(uri)
                    .header("x-account-id", "tenant-a")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await;

        assert_eq!(
            resp.status(),
            StatusCode::FORBIDDEN,
            "{uri} must be admin-only"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn mt_network_comms_topology_and_events_only_include_owned_agents() {
    let h = start_mt_router().await;

    let mut tenant_a_parent = make_agent_entry("tenant-a-parent", Some("tenant-a"));
    let tenant_a_child = make_agent_entry("tenant-a-child", Some("tenant-a"));
    let tenant_b_child = make_agent_entry("tenant-b-child", Some("tenant-b"));
    let tenant_a_parent_id = tenant_a_parent.id;
    let tenant_a_child_id = tenant_a_child.id;
    let tenant_b_child_id = tenant_b_child.id;
    tenant_a_parent.children = vec![tenant_a_child_id, tenant_b_child_id];

    let _ = h.state.kernel.agent_registry().register(tenant_a_parent);
    let _ = h.state.kernel.agent_registry().register(tenant_a_child);
    let _ = h.state.kernel.agent_registry().register(tenant_b_child);

    h.state.kernel.audit().record(
        &tenant_a_parent_id.to_string(),
        librefang_runtime::audit::AuditAction::AgentMessage,
        "tokens_in=5, tokens_out=7",
        "ok",
    );
    h.state.kernel.audit().record(
        &tenant_b_child_id.to_string(),
        librefang_runtime::audit::AuditAction::AgentMessage,
        "tokens_in=11, tokens_out=13",
        "ok",
    );

    let topology_resp = h
        .send(
            Request::builder()
                .uri("/api/comms/topology")
                .header("x-account-id", "tenant-a")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(topology_resp.status(), StatusCode::OK);
    let topology_json = read_json(topology_resp).await;
    let nodes = topology_json["nodes"].as_array().unwrap();
    assert_eq!(nodes.len(), 2);
    assert!(nodes
        .iter()
        .all(|node| node["id"] != tenant_b_child_id.to_string()));
    let edges = topology_json["edges"].as_array().unwrap();
    assert!(edges
        .iter()
        .all(|edge| edge["to"] != tenant_b_child_id.to_string()));

    let events_resp = h
        .send(
            Request::builder()
                .uri("/api/comms/events")
                .header("x-account-id", "tenant-a")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(events_resp.status(), StatusCode::OK);
    let events_json = read_json(events_resp).await;
    let events = events_json.as_array().unwrap();
    assert!(
        !events.is_empty(),
        "tenant-a should see its own audit activity"
    );
    assert!(events
        .iter()
        .all(|event| event["source_id"] != tenant_b_child_id.to_string()));
}

#[tokio::test(flavor = "multi_thread")]
async fn mt_network_comms_send_returns_404_for_cross_tenant_target() {
    let h = start_mt_router().await;

    let tenant_a = make_agent_entry("tenant-a-agent", Some("tenant-a"));
    let tenant_b = make_agent_entry("tenant-b-agent", Some("tenant-b"));
    let tenant_a_id = tenant_a.id;
    let tenant_b_id = tenant_b.id;
    let _ = h.state.kernel.agent_registry().register(tenant_a);
    let _ = h.state.kernel.agent_registry().register(tenant_b);

    let resp = h
        .send(
            Request::builder()
                .method("POST")
                .uri("/api/comms/send")
                .header("x-account-id", "tenant-a")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&serde_json::json!({
                        "from_agent_id": tenant_a_id.to_string(),
                        "to_agent_id": tenant_b_id.to_string(),
                        "message": "hello",
                        "attachments": [],
                    }))
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await;

    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}
