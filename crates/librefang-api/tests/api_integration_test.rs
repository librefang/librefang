//! Real HTTP integration tests for the LibreFang API.
//!
//! These tests boot a real kernel, start a real axum HTTP server on a random
//! port, and hit actual endpoints with reqwest.  No mocking.
//!
//! Tests that require an LLM API call are gated behind GROQ_API_KEY.
//!
//! Run: cargo test -p librefang-api --test api_integration_test -- --nocapture

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use librefang_api::middleware;
use librefang_api::routes::{self, AppState};
use librefang_api::server;
use librefang_api::ws;
use librefang_kernel::LibreFangKernel;
use librefang_runtime::audit::AuditAction;
use librefang_types::config::{DefaultModelConfig, KernelConfig};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;
use tower::ServiceExt;
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;

// ---------------------------------------------------------------------------
// Test infrastructure
// ---------------------------------------------------------------------------

struct TestServer {
    base_url: String,
    config_path: PathBuf,
    state: Arc<AppState>,
    _tmp: tempfile::TempDir,
}

struct FullRouterHarness {
    app: Router,
    state: Arc<AppState>,
    _tmp: tempfile::TempDir,
}

impl Drop for TestServer {
    fn drop(&mut self) {
        self.state.kernel.shutdown();
    }
}

impl Drop for FullRouterHarness {
    fn drop(&mut self) {
        self.state.kernel.shutdown();
    }
}

/// Start a test server using ollama as default provider (no API key needed).
/// This lets the kernel boot without any real LLM credentials.
/// Tests that need actual LLM calls should use `start_test_server_with_llm()`.
async fn start_test_server() -> TestServer {
    start_test_server_with_provider("ollama", "test-model", "OLLAMA_API_KEY").await
}

/// Start a test server with Groq as the LLM provider (requires GROQ_API_KEY).
async fn start_test_server_with_llm() -> TestServer {
    start_test_server_with_provider("groq", "llama-3.3-70b-versatile", "GROQ_API_KEY").await
}

async fn start_test_server_with_provider(
    provider: &str,
    model: &str,
    api_key_env: &str,
) -> TestServer {
    let tmp = tempfile::tempdir().expect("Failed to create temp dir");

    let config = KernelConfig {
        home_dir: tmp.path().to_path_buf(),
        data_dir: tmp.path().join("data"),
        default_model: DefaultModelConfig {
            provider: provider.to_string(),
            model: model.to_string(),
            api_key_env: api_key_env.to_string(),
            base_url: None,
            message_timeout_secs: 300,
            extra_params: std::collections::HashMap::new(),
            cli_profile_dirs: Vec::new(),
        },
        ..KernelConfig::default()
    };
    let config_path = tmp.path().join("config.toml");
    std::fs::write(&config_path, toml::to_string_pretty(&config).unwrap())
        .expect("Failed to write test config");

    let kernel = LibreFangKernel::boot_with_config(config).expect("Kernel should boot");
    let kernel = Arc::new(kernel);
    kernel.set_self_handle();

    let state = Arc::new(AppState {
        kernel,
        started_at: Instant::now(),
        peer_registry: None,
        bridge_manager: tokio::sync::Mutex::new(None),
        channels_config: tokio::sync::RwLock::new(Default::default()),
        shutdown_notify: Arc::new(tokio::sync::Notify::new()),
        clawhub_cache: dashmap::DashMap::new(),
        skillhub_cache: dashmap::DashMap::new(),
        provider_probe_cache: librefang_runtime::provider_health::ProbeCache::new(),
        webhook_store: librefang_api::webhook_store::WebhookStore::load(std::env::temp_dir().join(
            format!("librefang-test-webhooks-{}.json", uuid::Uuid::new_v4()),
        )),
        active_sessions: Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
        #[cfg(feature = "telemetry")]
        prometheus_handle: None,
        media_drivers: librefang_runtime::media::MediaDriverCache::new(),
        webhook_router: Arc::new(tokio::sync::RwLock::new(Arc::new(axum::Router::new()))),
        api_key_lock: Arc::new(tokio::sync::RwLock::new(String::new())),
        provider_test_cache: dashmap::DashMap::new(),
        config_write_lock: tokio::sync::Mutex::new(()),
    });

    let app = Router::new()
        .route("/api/health", axum::routing::get(routes::health))
        .route("/api/status", axum::routing::get(routes::status))
        .route(
            "/api/config/reload",
            axum::routing::post(routes::config_reload),
        )
        .route(
            "/api/agents",
            axum::routing::get(routes::list_agents).post(routes::spawn_agent),
        )
        .route(
            "/api/agents/{id}/message",
            axum::routing::post(routes::send_message),
        )
        .route(
            "/api/agents/{id}/session",
            axum::routing::get(routes::get_agent_session),
        )
        .route(
            "/api/agents/{id}/metrics",
            axum::routing::get(routes::agent_metrics),
        )
        .route(
            "/api/agents/{id}/logs",
            axum::routing::get(routes::agent_logs),
        )
        .route("/api/agents/{id}/ws", axum::routing::get(ws::agent_ws))
        .route(
            "/api/agents/{id}",
            axum::routing::delete(routes::kill_agent),
        )
        .route(
            "/api/triggers",
            axum::routing::get(routes::list_triggers).post(routes::create_trigger),
        )
        .route(
            "/api/triggers/{id}",
            axum::routing::delete(routes::delete_trigger),
        )
        .route(
            "/api/workflows",
            axum::routing::get(routes::list_workflows).post(routes::create_workflow),
        )
        .route(
            "/api/workflows/{id}/run",
            axum::routing::post(routes::run_workflow),
        )
        .route(
            "/api/workflows/{id}/runs",
            axum::routing::get(routes::list_workflow_runs),
        )
        .route("/api/tools", axum::routing::get(routes::list_tools))
        .route("/api/tools/{name}", axum::routing::get(routes::get_tool))
        .route("/mcp", axum::routing::post(routes::mcp_http))
        .route("/api/shutdown", axum::routing::post(routes::shutdown))
        .layer(axum::middleware::from_fn(middleware::request_logging))
        .layer(TraceLayer::new_for_http())
        .layer(CorsLayer::permissive())
        .with_state(state.clone());

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("Failed to bind test server");
    let addr = listener.local_addr().unwrap();

    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    TestServer {
        base_url: format!("http://{}", addr),
        config_path,
        state,
        _tmp: tmp,
    }
}

async fn start_full_router(api_key: &str) -> FullRouterHarness {
    let tmp = tempfile::tempdir().expect("Failed to create temp dir");

    // Sync registry content into the temp home_dir so the kernel boots
    // with a populated model catalog.
    librefang_runtime::registry_sync::sync_registry(
        tmp.path(),
        librefang_runtime::registry_sync::DEFAULT_CACHE_TTL_SECS,
        "",
    );

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
            extra_params: std::collections::HashMap::new(),
            cli_profile_dirs: Vec::new(),
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

    FullRouterHarness {
        app,
        state,
        _tmp: tmp,
    }
}

/// Manifest that uses ollama (no API key required, won't make real LLM calls).
const TEST_MANIFEST: &str = r#"
name = "test-agent"
version = "0.1.0"
description = "Integration test agent"
author = "test"
module = "builtin:chat"

[model]
provider = "ollama"
model = "test-model"
system_prompt = "You are a test agent. Reply concisely."

[capabilities]
tools = ["file_read"]
memory_read = ["*"]
memory_write = ["self.*"]
"#;

/// Manifest that uses Groq for real LLM tests.
const LLM_MANIFEST: &str = r#"
name = "test-agent"
version = "0.1.0"
description = "Integration test agent"
author = "test"
module = "builtin:chat"

[model]
provider = "groq"
model = "llama-3.3-70b-versatile"
system_prompt = "You are a test agent. Reply concisely."

[capabilities]
tools = ["file_read"]
memory_read = ["*"]
memory_write = ["self.*"]
"#;

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn test_health_endpoint() {
    let server = start_test_server().await;
    let client = reqwest::Client::new();

    let resp = client
        .get(format!("{}/api/health", server.base_url))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);

    // Middleware injects x-request-id
    assert!(resp.headers().contains_key("x-request-id"));

    let body: serde_json::Value = resp.json().await.unwrap();
    // Public health endpoint returns minimal info (redacted for security)
    assert_eq!(body["status"], "ok");
    assert!(body["version"].is_string());
    // Detailed fields should NOT appear in public health endpoint
    assert!(body["database"].is_null());
    assert!(body["agent_count"].is_null());
}

#[tokio::test(flavor = "multi_thread")]
async fn test_status_endpoint() {
    let server = start_test_server().await;
    let client = reqwest::Client::new();

    let resp = client
        .get(format!("{}/api/status", server.base_url))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["status"], "running");
    assert_eq!(body["agent_count"], 1); // default assistant auto-spawned
    assert!(body["uptime_seconds"].is_number());
    assert_eq!(body["default_provider"], "ollama");
    assert_eq!(body["agents"].as_array().unwrap().len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_build_router_exposes_versioned_api_aliases() {
    let harness = start_full_router("").await;

    let health = harness
        .app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(health.status(), StatusCode::OK);
    assert_eq!(health.headers()["x-api-version"], "v1");

    let versioned_health = harness
        .app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(versioned_health.status(), StatusCode::OK);
    assert_eq!(versioned_health.headers()["x-api-version"], "v1");

    let versions = harness
        .app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/versions")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(versions.status(), StatusCode::OK);

    let body = axum::body::to_bytes(versions.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["current"], "v1");
    assert!(json["supported"]
        .as_array()
        .unwrap()
        .contains(&serde_json::json!("v1")));
}

#[tokio::test(flavor = "multi_thread")]
async fn test_build_router_path_version_beats_unknown_accept_header() {
    let harness = start_full_router("").await;

    let response = harness
        .app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/health")
                .header("accept", "application/vnd.librefang.v99+json")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["x-api-version"], "v1");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_build_router_serves_dashboard_locales() {
    let harness = start_full_router("").await;

    for (path, expected_chat) in [
        ("/locales/en.json", "Chat"),
        ("/locales/zh-CN.json", "对话"),
        ("/locales/ja.json", "チャット"),
    ] {
        let response = harness
            .app
            .clone()
            .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers()["content-type"],
            "application/json; charset=utf-8"
        );

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["nav"]["chat"], expected_chat);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn test_build_router_providers_marks_local_providers() {
    let harness = start_full_router("").await;

    let response = harness
        .app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/providers")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let providers = json["providers"].as_array().unwrap();
    // Ollama is always in the registry and must be marked as a local provider.
    let ollama = providers
        .iter()
        .find(|provider| provider["id"] == "ollama")
        .expect("ollama provider should be present");

    assert_eq!(ollama["is_local"], serde_json::json!(true));
}

#[tokio::test(flavor = "multi_thread")]
async fn test_build_router_unauthorized_responses_include_api_version_header() {
    let harness = start_full_router("secret").await;

    let response = harness
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/agents")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(response.headers()["x-api-version"], "v1");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_run_migrate_uses_daemon_home_when_target_dir_is_empty() {
    let harness = start_full_router("").await;

    let source_dir = harness.state.kernel.home_dir().join("openclaw-source");
    std::fs::create_dir_all(&source_dir).unwrap();
    std::fs::write(
        source_dir.join("openclaw.json"),
        r#"{
          agents: {
            list: [
              { id: "main", name: "Main Agent" }
            ],
            defaults: {
              model: "anthropic/claude-sonnet-4-20250514"
            }
          }
        }"#,
    )
    .unwrap();

    let request = Request::builder()
        .method("POST")
        .uri("/api/migrate")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::to_vec(&serde_json::json!({
                "source": "openclaw",
                "source_dir": source_dir.display().to_string(),
                "target_dir": "",
                "dry_run": false
            }))
            .unwrap(),
        ))
        .unwrap();

    let response = harness.app.clone().oneshot(request).await.unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["status"], "completed");
    assert_eq!(json["dry_run"], false);

    let config_path = harness.state.kernel.home_dir().join("config.toml");
    // Migrate writes to <home>/agents/ but the daemon relocates the dirs to
    // the canonical workspaces/agents/ layout immediately after migration.
    let agent_path = harness
        .state
        .kernel
        .home_dir()
        .join("workspaces")
        .join("agents")
        .join("main")
        .join("agent.toml");
    let report_path = harness.state.kernel.home_dir().join("migration_report.md");

    assert!(
        config_path.exists(),
        "config.toml should be written to daemon home"
    );
    assert!(
        agent_path.exists(),
        "agent.toml should be written to daemon home"
    );
    assert!(
        report_path.exists(),
        "migration_report.md should be written to daemon home"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_config_reload_hot_reloads_proxy_changes() {
    let server = start_test_server().await;
    let client = reqwest::Client::new();

    let mut config: toml::Value =
        toml::from_str(&std::fs::read_to_string(&server.config_path).unwrap()).unwrap();
    let table = config.as_table_mut().unwrap();
    table.insert(
        "home_dir".to_string(),
        toml::Value::String(server.state.kernel.home_dir().display().to_string()),
    );
    table.insert(
        "data_dir".to_string(),
        toml::Value::String(server.state.kernel.data_dir().display().to_string()),
    );
    table.insert(
        "proxy".to_string(),
        toml::Value::Table(toml::map::Map::from_iter([(
            "http_proxy".to_string(),
            toml::Value::String("http://proxy.example.com:8080".to_string()),
        )])),
    );
    std::fs::write(
        &server.config_path,
        toml::to_string_pretty(&config).unwrap(),
    )
    .unwrap();

    let resp = client
        .post(format!("{}/api/config/reload", server.base_url))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);

    let body: serde_json::Value = resp.json().await.unwrap();
    // Proxy is now hot-reloadable — should NOT require restart
    assert_eq!(
        body["restart_required"], false,
        "proxy changes should be hot-reloaded, not require restart: {body}"
    );
    assert!(
        body["hot_actions_applied"]
            .as_array()
            .map(|a| a.iter().any(|v| v.as_str() == Some("ReloadProxy")))
            .unwrap_or(false),
        "ReloadProxy should be in hot_actions_applied: {body}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_spawn_list_kill_agent() {
    let server = start_test_server().await;
    let client = reqwest::Client::new();

    // --- Spawn ---
    let resp = client
        .post(format!("{}/api/agents", server.base_url))
        .json(&serde_json::json!({"manifest_toml": TEST_MANIFEST}))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 201);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["name"], "test-agent");
    let agent_id = body["agent_id"].as_str().unwrap().to_string();
    assert!(!agent_id.is_empty());

    // --- List (2 agents: default assistant + test-agent) ---
    let resp = client
        .get(format!("{}/api/agents", server.base_url))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    let agents = body["items"].as_array().unwrap();
    assert_eq!(agents.len(), 2);
    let test_agent = agents.iter().find(|a| a["name"] == "test-agent").unwrap();
    assert_eq!(test_agent["id"], agent_id);
    assert_eq!(test_agent["model_provider"], "ollama");

    // --- Kill ---
    let resp = client
        .delete(format!("{}/api/agents/{}", server.base_url, agent_id))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["status"], "killed");

    // --- List (only default assistant remains) ---
    let resp = client
        .get(format!("{}/api/agents", server.base_url))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    let agents = body["items"].as_array().unwrap();
    assert_eq!(agents.len(), 1);
    assert_eq!(agents[0]["name"], "assistant");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_agent_session_empty() {
    let server = start_test_server().await;
    let client = reqwest::Client::new();

    // Spawn agent
    let resp = client
        .post(format!("{}/api/agents", server.base_url))
        .json(&serde_json::json!({"manifest_toml": TEST_MANIFEST}))
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = resp.json().await.unwrap();
    let agent_id = body["agent_id"].as_str().unwrap();

    // Session should be empty — no messages sent yet
    let resp = client
        .get(format!(
            "{}/api/agents/{}/session",
            server.base_url, agent_id
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["message_count"], 0);
    assert_eq!(body["messages"].as_array().unwrap().len(), 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_agent_monitoring_endpoints() {
    let server = start_test_server().await;
    let client = reqwest::Client::new();

    let resp = client
        .post(format!("{}/api/agents", server.base_url))
        .json(&serde_json::json!({"manifest_toml": TEST_MANIFEST}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 201);
    let body: serde_json::Value = resp.json().await.unwrap();
    let agent_id = body["agent_id"].as_str().unwrap().to_string();

    server.state.kernel.audit().record(
        &agent_id,
        AuditAction::AgentMessage,
        "exact match target",
        "custom_error",
    );
    server.state.kernel.audit().record(
        &agent_id,
        AuditAction::AgentMessage,
        "should not match substring filter",
        "not_custom_error",
    );

    let resp = client
        .get(format!(
            "{}/api/agents/{}/metrics",
            server.base_url, agent_id
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let metrics: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(metrics["agent_id"], agent_id);
    assert!(metrics["token_usage"].is_object());
    assert!(metrics["tool_calls"].is_object());
    assert!(metrics.get("avg_response_time_ms").is_some());

    let resp = client
        .get(format!(
            "{}/api/agents/{}/logs?level=custom_error&n=10",
            server.base_url, agent_id
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let logs: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(logs["count"], 1);
    assert_eq!(logs["logs"].as_array().unwrap().len(), 1);
    assert_eq!(logs["logs"][0]["outcome"], "custom_error");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_send_message_with_llm() {
    if std::env::var("GROQ_API_KEY").is_err() {
        eprintln!("GROQ_API_KEY not set, skipping LLM integration test");
        return;
    }

    let server = start_test_server_with_llm().await;
    let client = reqwest::Client::new();

    // Spawn
    let resp = client
        .post(format!("{}/api/agents", server.base_url))
        .json(&serde_json::json!({"manifest_toml": LLM_MANIFEST}))
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = resp.json().await.unwrap();
    let agent_id = body["agent_id"].as_str().unwrap().to_string();

    // Send message through the real HTTP endpoint → kernel → Groq LLM
    let resp = client
        .post(format!(
            "{}/api/agents/{}/message",
            server.base_url, agent_id
        ))
        .json(&serde_json::json!({"message": "Say hello in exactly 3 words."}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    let response_text = body["response"].as_str().unwrap();
    assert!(
        !response_text.is_empty(),
        "LLM response should not be empty"
    );
    assert!(body["input_tokens"].as_u64().unwrap() > 0);
    assert!(body["output_tokens"].as_u64().unwrap() > 0);

    // Session should now have messages
    let resp = client
        .get(format!(
            "{}/api/agents/{}/session",
            server.base_url, agent_id
        ))
        .send()
        .await
        .unwrap();
    let session: serde_json::Value = resp.json().await.unwrap();
    assert!(session["message_count"].as_u64().unwrap() > 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_workflow_crud() {
    let server = start_test_server().await;
    let client = reqwest::Client::new();

    // Spawn agent for workflow
    let resp = client
        .post(format!("{}/api/agents", server.base_url))
        .json(&serde_json::json!({"manifest_toml": TEST_MANIFEST}))
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = resp.json().await.unwrap();
    let agent_name = body["name"].as_str().unwrap().to_string();

    // Create workflow
    let resp = client
        .post(format!("{}/api/workflows", server.base_url))
        .json(&serde_json::json!({
            "name": "test-workflow",
            "description": "Integration test workflow",
            "steps": [
                {
                    "name": "step1",
                    "agent_name": agent_name,
                    "prompt": "Echo: {{input}}",
                    "mode": "sequential",
                    "timeout_secs": 30
                }
            ]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 201);
    let body: serde_json::Value = resp.json().await.unwrap();
    let workflow_id = body["workflow_id"].as_str().unwrap().to_string();
    assert!(!workflow_id.is_empty());

    // List workflows
    let resp = client
        .get(format!("{}/api/workflows", server.base_url))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    let workflows = body["workflows"].as_array().unwrap();
    assert_eq!(workflows.len(), 1);
    assert_eq!(workflows[0]["name"], "test-workflow");
    assert_eq!(workflows[0]["steps"], 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_trigger_crud() {
    let server = start_test_server().await;
    let client = reqwest::Client::new();

    // Spawn agent for trigger
    let resp = client
        .post(format!("{}/api/agents", server.base_url))
        .json(&serde_json::json!({"manifest_toml": TEST_MANIFEST}))
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = resp.json().await.unwrap();
    let agent_id = body["agent_id"].as_str().unwrap().to_string();

    // Create trigger (Lifecycle pattern — simplest variant)
    let resp = client
        .post(format!("{}/api/triggers", server.base_url))
        .json(&serde_json::json!({
            "agent_id": agent_id,
            "pattern": "lifecycle",
            "prompt_template": "Handle: {{event}}",
            "max_fires": 5
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 201);
    let body: serde_json::Value = resp.json().await.unwrap();
    let trigger_id = body["trigger_id"].as_str().unwrap().to_string();
    assert_eq!(body["agent_id"], agent_id);

    // List triggers (unfiltered)
    let resp = client
        .get(format!("{}/api/triggers", server.base_url))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    let triggers = body["triggers"].as_array().unwrap();
    assert_eq!(triggers.len(), 1);
    assert_eq!(triggers[0]["agent_id"], agent_id);
    assert_eq!(triggers[0]["enabled"], true);
    assert_eq!(triggers[0]["max_fires"], 5);

    // List triggers (filtered by agent_id)
    let resp = client
        .get(format!(
            "{}/api/triggers?agent_id={}",
            server.base_url, agent_id
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    let triggers = body["triggers"].as_array().unwrap();
    assert_eq!(triggers.len(), 1);

    // Delete trigger
    let resp = client
        .delete(format!("{}/api/triggers/{}", server.base_url, trigger_id))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);

    // List triggers (should be empty)
    let resp = client
        .get(format!("{}/api/triggers", server.base_url))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    let triggers = body["triggers"].as_array().unwrap();
    assert_eq!(triggers.len(), 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_invalid_agent_id_returns_400() {
    let server = start_test_server().await;
    let client = reqwest::Client::new();

    // Send message to invalid ID
    let resp = client
        .post(format!("{}/api/agents/not-a-uuid/message", server.base_url))
        .json(&serde_json::json!({"message": "hello"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert!(body["error"].as_str().unwrap().contains("Invalid"));

    // Kill invalid ID
    let resp = client
        .delete(format!("{}/api/agents/not-a-uuid", server.base_url))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);

    // Session for invalid ID
    let resp = client
        .get(format!("{}/api/agents/not-a-uuid/session", server.base_url))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_kill_nonexistent_agent_returns_404() {
    let server = start_test_server().await;
    let client = reqwest::Client::new();

    let fake_id = uuid::Uuid::new_v4();
    let resp = client
        .delete(format!("{}/api/agents/{}", server.base_url, fake_id))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 404);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_spawn_invalid_manifest_returns_400() {
    let server = start_test_server().await;
    let client = reqwest::Client::new();

    let resp = client
        .post(format!("{}/api/agents", server.base_url))
        .json(&serde_json::json!({"manifest_toml": "this is {{ not valid toml"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert!(body["error"].as_str().unwrap().contains("Invalid manifest"));
}

#[tokio::test(flavor = "multi_thread")]
async fn test_request_id_header_is_uuid() {
    let server = start_test_server().await;
    let client = reqwest::Client::new();

    let resp = client
        .get(format!("{}/api/health", server.base_url))
        .send()
        .await
        .unwrap();

    let request_id = resp
        .headers()
        .get("x-request-id")
        .expect("x-request-id header should be present");
    let id_str = request_id.to_str().unwrap();
    assert!(
        uuid::Uuid::parse_str(id_str).is_ok(),
        "x-request-id should be a valid UUID, got: {}",
        id_str
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_multiple_agents_lifecycle() {
    let server = start_test_server().await;
    let client = reqwest::Client::new();

    // Spawn 3 agents
    let mut ids = Vec::new();
    for i in 0..3 {
        let manifest = format!(
            r#"
name = "agent-{i}"
version = "0.1.0"
description = "Multi-agent test {i}"
author = "test"
module = "builtin:chat"

[model]
provider = "ollama"
model = "test-model"
system_prompt = "Agent {i}."

[capabilities]
memory_read = ["*"]
memory_write = ["self.*"]
"#
        );

        let resp = client
            .post(format!("{}/api/agents", server.base_url))
            .json(&serde_json::json!({"manifest_toml": manifest}))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 201);
        let body: serde_json::Value = resp.json().await.unwrap();
        ids.push(body["agent_id"].as_str().unwrap().to_string());
    }

    // List should show 4 (3 spawned + default assistant)
    let resp = client
        .get(format!("{}/api/agents", server.base_url))
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = resp.json().await.unwrap();
    let agents = body["items"].as_array().unwrap();
    assert_eq!(agents.len(), 4);

    // Status should agree
    let resp = client
        .get(format!("{}/api/status", server.base_url))
        .send()
        .await
        .unwrap();
    let status: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(status["agent_count"], 4);

    // Kill one
    let resp = client
        .delete(format!("{}/api/agents/{}", server.base_url, ids[1]))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);

    // List should show 3 (2 spawned + default assistant)
    let resp = client
        .get(format!("{}/api/agents", server.base_url))
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = resp.json().await.unwrap();
    let agents = body["items"].as_array().unwrap();
    assert_eq!(agents.len(), 3);

    // Kill the rest
    for id in [&ids[0], &ids[2]] {
        client
            .delete(format!("{}/api/agents/{}", server.base_url, id))
            .send()
            .await
            .unwrap();
    }

    // List should have only default assistant
    let resp = client
        .get(format!("{}/api/agents", server.base_url))
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = resp.json().await.unwrap();
    let agents = body["items"].as_array().unwrap();
    assert_eq!(agents.len(), 1);
}

// ---------------------------------------------------------------------------
// Agent list filtering, pagination, and sorting tests
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn test_agent_list_paginated_response_format() {
    let server = start_test_server().await;
    let client = reqwest::Client::new();

    // Default list should return paginated object with items, total, offset, limit
    let resp = client
        .get(format!("{}/api/agents", server.base_url))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert!(
        body["items"].is_array(),
        "Response should have 'items' array"
    );
    assert!(
        body["total"].is_number(),
        "Response should have 'total' number"
    );
    assert!(
        body["offset"].is_number(),
        "Response should have 'offset' number"
    );
    // limit should be null when not specified
    assert!(
        body["limit"].is_null(),
        "limit should be null when not specified"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_agent_list_invalid_sort_returns_400() {
    let server = start_test_server().await;
    let client = reqwest::Client::new();

    let resp = client
        .get(format!("{}/api/agents?sort=invalid_field", server.base_url))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);
    let body: serde_json::Value = resp.json().await.unwrap();
    let error = body["error"].as_str().unwrap();
    assert!(
        error.contains("Invalid sort field"),
        "Error should mention invalid sort field, got: {}",
        error
    );
    assert!(error.contains("invalid_field"));
}

#[tokio::test(flavor = "multi_thread")]
async fn test_agent_list_valid_sort_fields() {
    let server = start_test_server().await;
    let client = reqwest::Client::new();

    // All valid sort fields should return 200
    for field in &["name", "created_at", "last_active", "state"] {
        let resp = client
            .get(format!("{}/api/agents?sort={}", server.base_url, field))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 200, "Sort by '{}' should return 200", field);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn test_agent_list_limit_clamped_to_max() {
    let server = start_test_server().await;
    let client = reqwest::Client::new();

    // Request with limit > 100 should be clamped
    let resp = client
        .get(format!("{}/api/agents?limit=9999", server.base_url))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    // limit in response should be clamped to 100
    assert_eq!(body["limit"].as_u64().unwrap(), 100);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_agent_list_pagination() {
    let server = start_test_server().await;
    let client = reqwest::Client::new();

    // Spawn 2 extra agents
    for i in 0..2 {
        let manifest = format!(
            r#"
name = "page-agent-{i}"
module = "builtin:chat"

[model]
provider = "ollama"
model = "test-model"
system_prompt = "Agent {i}."
"#
        );
        client
            .post(format!("{}/api/agents", server.base_url))
            .json(&serde_json::json!({"manifest_toml": manifest}))
            .send()
            .await
            .unwrap();
    }

    // Get first page with limit=1
    let resp = client
        .get(format!("{}/api/agents?limit=1&offset=0", server.base_url))
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = resp.json().await.unwrap();
    let items = body["items"].as_array().unwrap();
    assert_eq!(items.len(), 1, "Should return exactly 1 item");
    assert!(
        body["total"].as_u64().unwrap() >= 3,
        "Total should include all agents"
    );

    // Get second page
    let resp = client
        .get(format!("{}/api/agents?limit=1&offset=1", server.base_url))
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = resp.json().await.unwrap();
    let items2 = body["items"].as_array().unwrap();
    assert_eq!(items2.len(), 1, "Second page should return 1 item");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_agent_list_text_search() {
    let server = start_test_server().await;
    let client = reqwest::Client::new();

    let manifest = r#"
name = "unique-searchable-agent"
description = "A very special description for testing search"
module = "builtin:chat"

[model]
provider = "ollama"
model = "test-model"
system_prompt = "Test."
"#;
    client
        .post(format!("{}/api/agents", server.base_url))
        .json(&serde_json::json!({"manifest_toml": manifest}))
        .send()
        .await
        .unwrap();

    // Search by name
    let resp = client
        .get(format!(
            "{}/api/agents?q=unique-searchable",
            server.base_url
        ))
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = resp.json().await.unwrap();
    let items = body["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["name"], "unique-searchable-agent");

    // Search with no match
    let resp = client
        .get(format!(
            "{}/api/agents?q=nonexistent-xyz-agent",
            server.base_url
        ))
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = resp.json().await.unwrap();
    let items = body["items"].as_array().unwrap();
    assert!(
        items.is_empty(),
        "No agents should match non-existent query"
    );
}

// ---------------------------------------------------------------------------
// Auth integration tests
// ---------------------------------------------------------------------------

/// Start a test server with Bearer-token authentication enabled.
async fn start_test_server_with_auth(api_key: &str) -> TestServer {
    let tmp = tempfile::tempdir().expect("Failed to create temp dir");

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
            extra_params: std::collections::HashMap::new(),
            cli_profile_dirs: Vec::new(),
        },
        ..KernelConfig::default()
    };
    let config_path = tmp.path().join("config.toml");
    std::fs::write(&config_path, toml::to_string_pretty(&config).unwrap())
        .expect("Failed to write test config");

    let kernel = LibreFangKernel::boot_with_config(config).expect("Kernel should boot");
    let kernel = Arc::new(kernel);
    kernel.set_self_handle();

    let api_key_lock = std::sync::Arc::new(tokio::sync::RwLock::new(
        kernel.config_ref().api_key.clone(),
    ));

    let state = Arc::new(AppState {
        kernel,
        started_at: Instant::now(),
        peer_registry: None,
        bridge_manager: tokio::sync::Mutex::new(None),
        channels_config: tokio::sync::RwLock::new(Default::default()),
        shutdown_notify: Arc::new(tokio::sync::Notify::new()),
        clawhub_cache: dashmap::DashMap::new(),
        skillhub_cache: dashmap::DashMap::new(),
        provider_probe_cache: librefang_runtime::provider_health::ProbeCache::new(),
        webhook_store: librefang_api::webhook_store::WebhookStore::load(std::env::temp_dir().join(
            format!("librefang-test-webhooks-{}.json", uuid::Uuid::new_v4()),
        )),
        active_sessions: Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
        #[cfg(feature = "telemetry")]
        prometheus_handle: None,
        media_drivers: librefang_runtime::media::MediaDriverCache::new(),
        webhook_router: Arc::new(tokio::sync::RwLock::new(Arc::new(axum::Router::new()))),
        api_key_lock: api_key_lock.clone(),
        provider_test_cache: dashmap::DashMap::new(),
        config_write_lock: tokio::sync::Mutex::new(()),
    });

    let api_key_state = middleware::AuthState {
        api_key_lock,
        active_sessions: state.active_sessions.clone(),
        dashboard_auth_enabled: false,
        user_api_keys: Arc::new(Vec::new()),
        require_auth_for_reads: false,
    };

    let app = Router::new()
        .route("/api/health", axum::routing::get(routes::health))
        .route("/api/status", axum::routing::get(routes::status))
        .route(
            "/api/agents",
            axum::routing::get(routes::list_agents).post(routes::spawn_agent),
        )
        .route(
            "/api/agents/{id}/message",
            axum::routing::post(routes::send_message),
        )
        .route(
            "/api/agents/{id}/session",
            axum::routing::get(routes::get_agent_session),
        )
        .route(
            "/api/agents/{id}/metrics",
            axum::routing::get(routes::agent_metrics),
        )
        .route(
            "/api/agents/{id}/logs",
            axum::routing::get(routes::agent_logs),
        )
        .route("/api/agents/{id}/ws", axum::routing::get(ws::agent_ws))
        .route(
            "/api/agents/{id}",
            axum::routing::delete(routes::kill_agent),
        )
        .route(
            "/api/triggers",
            axum::routing::get(routes::list_triggers).post(routes::create_trigger),
        )
        .route(
            "/api/triggers/{id}",
            axum::routing::delete(routes::delete_trigger),
        )
        .route(
            "/api/workflows",
            axum::routing::get(routes::list_workflows).post(routes::create_workflow),
        )
        .route(
            "/api/workflows/{id}/run",
            axum::routing::post(routes::run_workflow),
        )
        .route(
            "/api/workflows/{id}/runs",
            axum::routing::get(routes::list_workflow_runs),
        )
        .route("/api/shutdown", axum::routing::post(routes::shutdown))
        .layer(axum::middleware::from_fn_with_state(
            api_key_state,
            middleware::auth,
        ))
        .layer(axum::middleware::from_fn(middleware::request_logging))
        .layer(TraceLayer::new_for_http())
        .layer(CorsLayer::permissive())
        .with_state(state.clone());

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("Failed to bind test server");
    let addr = listener.local_addr().unwrap();

    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    TestServer {
        base_url: format!("http://{}", addr),
        config_path,
        state,
        _tmp: tmp,
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn test_auth_health_is_public() {
    let server = start_test_server_with_auth("secret-key-123").await;
    let client = reqwest::Client::new();

    // /api/health should be accessible without auth
    let resp = client
        .get(format!("{}/api/health", server.base_url))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_auth_rejects_no_token() {
    let server = start_test_server_with_auth("secret-key-123").await;
    let client = reqwest::Client::new();

    // Protected endpoint without auth header → 401
    // Note: /api/status is public (dashboard needs it), so use a protected endpoint
    let resp = client
        .get(format!("{}/api/commands", server.base_url))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 401);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert!(body["error"].as_str().unwrap().contains("Missing"));
}

#[tokio::test(flavor = "multi_thread")]
async fn test_auth_rejects_wrong_token() {
    let server = start_test_server_with_auth("secret-key-123").await;
    let client = reqwest::Client::new();

    // Wrong bearer token → 401
    // Note: /api/status is public (dashboard needs it), so use a protected endpoint
    let resp = client
        .get(format!("{}/api/commands", server.base_url))
        .header("authorization", "Bearer wrong-key")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 401);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert!(body["error"].as_str().unwrap().contains("Invalid"));
}

#[tokio::test(flavor = "multi_thread")]
async fn test_auth_accepts_correct_token() {
    let server = start_test_server_with_auth("secret-key-123").await;
    let client = reqwest::Client::new();

    // Correct bearer token → 200
    let resp = client
        .get(format!("{}/api/status", server.base_url))
        .header("authorization", "Bearer secret-key-123")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["status"], "running");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_auth_disabled_when_no_key() {
    // Empty API key = auth disabled
    let server = start_test_server().await;
    let client = reqwest::Client::new();

    // Protected endpoint accessible without auth when no key is configured
    let resp = client
        .get(format!("{}/api/status", server.base_url))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
}

// ---------------------------------------------------------------------------
// Tool endpoints
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn test_list_tools() {
    let server = start_test_server().await;
    let client = reqwest::Client::new();

    let resp = client
        .get(format!("{}/api/tools", server.base_url))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);

    let body: serde_json::Value = resp.json().await.unwrap();
    assert!(body["tools"].is_array());
    assert!(body["total"].as_u64().unwrap() > 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_get_tool_found() {
    let server = start_test_server().await;
    let client = reqwest::Client::new();

    // First list tools to get a known tool name
    let resp = client
        .get(format!("{}/api/tools", server.base_url))
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = resp.json().await.unwrap();
    let first_tool_name = body["tools"][0]["name"].as_str().unwrap().to_string();

    // Now fetch that specific tool
    let resp = client
        .get(format!("{}/api/tools/{}", server.base_url, first_tool_name))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);

    let tool: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(tool["name"].as_str().unwrap(), first_tool_name);
    assert!(tool["description"].is_string());
    assert!(tool["input_schema"].is_object());
}

#[tokio::test(flavor = "multi_thread")]
async fn test_get_tool_not_found() {
    let server = start_test_server().await;
    let client = reqwest::Client::new();

    let resp = client
        .get(format!(
            "{}/api/tools/nonexistent_tool_xyz",
            server.base_url
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 404);

    let body: serde_json::Value = resp.json().await.unwrap();
    assert!(body["error"].as_str().unwrap().contains("not found"));
}

// ---------------------------------------------------------------------------
// Test: /api/hands/active enriched response (Task 1 of chat-picker plan)
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn list_active_hands_includes_definition_metadata() {
    use std::collections::{BTreeMap, HashMap};

    let harness = start_full_router("").await;

    // Install a fresh hand definition with a known name + icon.
    let toml_content = r#"
id = "test-grouping-hand"
name = "Test Grouping Hand"
description = "Hand fixture for chat picker grouping integration test"
category = "productivity"
icon = "🧪"

[agent]
name = "test-agent"
description = "Coordinator role for the test grouping hand"
system_prompt = "You are a test agent."

[dashboard]
metrics = []
"#;
    harness
        .state
        .kernel
        .hands()
        .install_from_content(toml_content, "")
        .expect("install_from_content should succeed");

    // Activate the hand to get an instance, then attach two roles by hand.
    // (The kernel normally spawns agents; here we simulate that with set_agents
    // so the test does not depend on the spawner subsystem.)
    let instance = harness
        .state
        .kernel
        .hands()
        .activate("test-grouping-hand", HashMap::new())
        .expect("activate should succeed");

    let main_id = librefang_types::agent::AgentId::new();
    let linter_id = librefang_types::agent::AgentId::new();
    let mut agent_ids = BTreeMap::new();
    agent_ids.insert("main".to_string(), main_id);
    agent_ids.insert("linter".to_string(), linter_id);
    harness
        .state
        .kernel
        .hands()
        .set_agents(instance.instance_id, agent_ids, Some("main".to_string()))
        .expect("set_agents should succeed");

    // Hit the endpoint via the in-process router.
    let response = harness
        .app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/hands/active")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("router.oneshot should succeed");
    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read body");
    let json: serde_json::Value = serde_json::from_slice(&body).expect("response is JSON");
    let instances = json["instances"].as_array().expect("instances array");
    let hand = instances
        .iter()
        .find(|i| i["hand_id"] == "test-grouping-hand")
        .expect("our hand must appear in the active list");

    // Existing fields — regression guard.
    assert_eq!(hand["hand_id"], "test-grouping-hand");
    assert!(hand["agent_id"].is_string(), "legacy agent_id must remain");
    assert!(
        hand["agent_name"].is_string(),
        "legacy agent_name must remain"
    );

    // NEW fields from this plan.
    assert_eq!(
        hand["hand_name"], "Test Grouping Hand",
        "hand_name must be exposed from definition"
    );
    assert_eq!(
        hand["hand_icon"], "🧪",
        "hand_icon must be exposed from definition"
    );
    assert_eq!(
        hand["coordinator_role"], "main",
        "coordinator_role must be exposed"
    );

    let agent_ids_obj = hand["agent_ids"]
        .as_object()
        .expect("agent_ids must be a JSON object");
    assert_eq!(agent_ids_obj.len(), 2, "agent_ids must contain both roles");
    assert_eq!(agent_ids_obj["main"], main_id.to_string());
    assert_eq!(agent_ids_obj["linter"], linter_id.to_string());
}

// ── issue #2699: `/mcp` must rehydrate caller context from the
// `X-LibreFang-Agent-Id` header so CLI drivers (claude-code) can call
// workspace/cron/media tools without every invocation failing.

/// Manifest that grants `cron_list` — needed to exercise the caller-
/// identity path on the `/mcp` endpoint. `TEST_MANIFEST` only grants
/// `file_read`, which would be rejected by the allowed-tools filter
/// that the fix correctly activates.
const MCP_TEST_MANIFEST: &str = r#"
name = "mcp-test-agent"
version = "0.1.0"
description = "Integration test agent for /mcp bridge"
author = "test"
module = "builtin:chat"

[model]
provider = "ollama"
model = "test-model"
system_prompt = "You are a test agent."

[capabilities]
tools = ["cron_list", "cron_create", "cron_cancel"]
memory_read = ["*"]
memory_write = ["self.*"]
"#;

async fn call_mcp_cron_list(
    server: &TestServer,
    agent_header: Option<&str>,
) -> (reqwest::StatusCode, serde_json::Value) {
    let client = reqwest::Client::new();
    let mut req = client
        .post(format!("{}/mcp", server.base_url))
        .json(&serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": {"name": "cron_list", "arguments": {}},
        }));
    if let Some(id) = agent_header {
        req = req.header("X-LibreFang-Agent-Id", id);
    }
    let resp = req.send().await.expect("mcp request send");
    let status = resp.status();
    let body: serde_json::Value = resp.json().await.expect("mcp body parse");
    (status, body)
}

#[tokio::test(flavor = "multi_thread")]
async fn test_mcp_http_rehydrates_caller_context_from_agent_header() {
    // Regression guard for issue #2699 — before the fix, the /mcp
    // endpoint hardcoded `caller_agent_id = None`, so tools that
    // require an agent identity (cron_*, file_*, media_*, schedule_*)
    // failed with a generic error even when the call actually came
    // from the CLI spawned by a registered agent.
    let server = start_test_server().await;

    // Spawn an agent with cron_* in its capabilities.tools.
    let client = reqwest::Client::new();
    let spawn_resp = client
        .post(format!("{}/api/agents", server.base_url))
        .json(&serde_json::json!({"manifest_toml": MCP_TEST_MANIFEST}))
        .send()
        .await
        .unwrap();
    assert_eq!(spawn_resp.status(), 201);
    let spawn_body: serde_json::Value = spawn_resp.json().await.unwrap();
    let agent_id = spawn_body["agent_id"].as_str().unwrap().to_string();

    // No header → cron_list must refuse with the "Agent ID required"
    // error the tool surfaces when caller_agent_id is None.
    let (status, body) = call_mcp_cron_list(&server, None).await;
    assert_eq!(status, 200);
    let content = body["result"]["content"][0]["text"].as_str().unwrap_or("");
    let is_error = body["result"]["isError"].as_bool().unwrap_or(false);
    assert!(
        is_error,
        "cron_list without caller_agent_id must surface an error; got content={content}"
    );
    assert!(
        content.contains("Agent ID required") || content.contains("agent_id"),
        "unexpected error text without header: {content}"
    );

    // With the header → cron_list resolves the agent, passes the
    // allowed-tools check, and returns an empty list. This is the
    // path Claude Code CLI takes after the fix.
    let (status, body) = call_mcp_cron_list(&server, Some(&agent_id)).await;
    assert_eq!(status, 200);
    let is_error = body["result"]["isError"].as_bool().unwrap_or(false);
    let content = body["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or("")
        .to_string();
    assert!(
        !is_error,
        "cron_list with X-LibreFang-Agent-Id must succeed; got error content={content}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_mcp_http_invalid_agent_header_falls_back_to_unauthenticated() {
    // An unparseable or unknown agent ID must degrade gracefully to
    // the unauthenticated path (same behaviour as no header) rather
    // than 500-ing. Keeps external MCP clients working even if a
    // misconfigured bridge stuffs a garbage ID into the header.
    let server = start_test_server().await;

    let (status, body) = call_mcp_cron_list(&server, Some("not-a-uuid")).await;
    assert_eq!(status, 200);
    let is_error = body["result"]["isError"].as_bool().unwrap_or(false);
    assert!(
        is_error,
        "invalid header must still yield the unauthenticated error path"
    );

    // Well-formed UUID but not a registered agent — same deal.
    let (status, body) =
        call_mcp_cron_list(&server, Some("00000000-0000-0000-0000-000000000000")).await;
    assert_eq!(status, 200);
    let is_error = body["result"]["isError"].as_bool().unwrap_or(false);
    assert!(
        is_error,
        "unknown agent ID must still yield the unauthenticated error path"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_mcp_http_unrestricted_agent_can_call_any_tool() {
    // Regression guard: a manifest with `capabilities.tools = []`
    // (or no [capabilities] section at all — same result) means
    // "unrestricted" on the direct agent-loop path. The bridge must
    // match that semantics. A naive implementation that passes the
    // raw `manifest.capabilities.tools` as `allowed_tools` would
    // produce `Some([])`, which `execute_tool` reads as "deny all"
    // and every tool invoked through the bridge would return
    // "Permission denied: agent does not have capability to use tool
    // 'cron_list'" even though the direct path allows everything.
    //
    // The bridge must resolve the allowed-tool set the same way
    // `kernel::send_message` does: `kernel.available_tools(id)` +
    // `entry.mode.filter_tools(...)`.
    const UNRESTRICTED_MANIFEST: &str = r#"
name = "unrestricted-test-agent"
version = "0.1.0"
description = "Agent with no tool restrictions"
author = "test"
module = "builtin:chat"

[model]
provider = "ollama"
model = "test-model"
system_prompt = "You are a test agent."
"#;

    let server = start_test_server().await;

    let client = reqwest::Client::new();
    let spawn_resp = client
        .post(format!("{}/api/agents", server.base_url))
        .json(&serde_json::json!({"manifest_toml": UNRESTRICTED_MANIFEST}))
        .send()
        .await
        .unwrap();
    assert_eq!(spawn_resp.status(), 201);
    let spawn_body: serde_json::Value = spawn_resp.json().await.unwrap();
    let agent_id = spawn_body["agent_id"].as_str().unwrap().to_string();

    let (status, body) = call_mcp_cron_list(&server, Some(&agent_id)).await;
    assert_eq!(status, 200);
    let is_error = body["result"]["isError"].as_bool().unwrap_or(false);
    let content = body["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or("")
        .to_string();
    assert!(
        !is_error,
        "unrestricted agent must be able to call cron_list through the bridge; got content={content}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_mcp_http_enforces_agent_tool_allowlist() {
    // The caller-context rehydration must ALSO propagate the agent's
    // `capabilities.tools` allowlist so the bridge can't be used to
    // privilege-escalate: if the agent didn't have a tool in its
    // manifest, invoking it through `/mcp` with the agent's own ID
    // must still be rejected. (TEST_MANIFEST only grants `file_read`,
    // so `cron_list` must be denied.)
    let server = start_test_server().await;

    let client = reqwest::Client::new();
    let spawn_resp = client
        .post(format!("{}/api/agents", server.base_url))
        .json(&serde_json::json!({"manifest_toml": TEST_MANIFEST}))
        .send()
        .await
        .unwrap();
    assert_eq!(spawn_resp.status(), 201);
    let spawn_body: serde_json::Value = spawn_resp.json().await.unwrap();
    let agent_id = spawn_body["agent_id"].as_str().unwrap().to_string();

    let (status, body) = call_mcp_cron_list(&server, Some(&agent_id)).await;
    assert_eq!(status, 200);
    let is_error = body["result"]["isError"].as_bool().unwrap_or(false);
    let content = body["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or("")
        .to_string();
    assert!(
        is_error,
        "cron_list must be denied for an agent whose manifest omits it; got content={content}"
    );
    assert!(
        content.contains("Permission denied") || content.contains("capability"),
        "denial must mention permission/capability; got: {content}"
    );
}
