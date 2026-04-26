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
use librefang_types::agent::WebSearchAugmentationMode;
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
            "/api/agents/{id}/sessions/{session_id}/trajectory",
            axum::routing::get(routes::export_session_trajectory),
        )
        .route(
            "/api/agents/{id}/sessions/{session_id}/stream",
            axum::routing::get(routes::attach_session_stream),
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

    let mut request = Request::builder()
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
    // Simulate a loopback connection so the unauth-fail-closed branch
    // (when api_key is empty) treats this oneshot as a localhost caller
    // rather than a non-loopback origin. Production gets ConnectInfo from
    // axum's connection layer; oneshot bypasses that, so we inject it.
    request
        .extensions_mut()
        .insert(axum::extract::ConnectInfo(std::net::SocketAddr::from((
            [127, 0, 0, 1],
            0,
        ))));

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

/// Regression test for the cross-agent session-read guard added in PR #3071.
///
/// `GET /api/agents/{A}/session?session_id={B's session}` MUST NOT return
/// agent B's history under agent A's id — otherwise one agent id can read
/// another agent's conversation by guessing a session UUID.
///
/// Also verifies the malformed-uuid case returns 400 (typed query param
/// validation) and that passing the agent's own session_id round-trips.
#[tokio::test(flavor = "multi_thread")]
async fn test_get_agent_session_rejects_cross_agent_session_id() {
    let server = start_test_server().await;
    let client = reqwest::Client::new();

    // Spawn agent A.
    let resp = client
        .post(format!("{}/api/agents", server.base_url))
        .json(&serde_json::json!({"manifest_toml": TEST_MANIFEST}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 201);
    let body_a: serde_json::Value = resp.json().await.unwrap();
    let agent_a = body_a["agent_id"].as_str().unwrap().to_string();

    // Spawn agent B (distinct name so the manifest validates).
    const TEST_MANIFEST_B: &str = r#"
name = "test-agent-b"
version = "0.1.0"
description = "Integration test agent B"
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
    let resp = client
        .post(format!("{}/api/agents", server.base_url))
        .json(&serde_json::json!({"manifest_toml": TEST_MANIFEST_B}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 201);
    let body_b: serde_json::Value = resp.json().await.unwrap();
    let agent_b = body_b["agent_id"].as_str().unwrap().to_string();

    // Discover B's session id (canonical-active).
    let resp = client
        .get(format!(
            "{}/api/agents/{}/session",
            server.base_url, agent_b
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let b_session: serde_json::Value = resp.json().await.unwrap();
    let b_session_id = b_session["session_id"].as_str().unwrap().to_string();

    // Cross-agent read: A's id with B's session_id → 404 (the guard).
    let resp = client
        .get(format!(
            "{}/api/agents/{}/session?session_id={}",
            server.base_url, agent_a, b_session_id
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::NOT_FOUND,
        "cross-agent session read must be rejected"
    );

    // Malformed UUID → 400 (typed serde validation).
    let resp = client
        .get(format!(
            "{}/api/agents/{}/session?session_id=not-a-uuid",
            server.base_url, agent_a
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

    // Same-agent round-trip: A's id with A's own session_id → 200.
    let resp = client
        .get(format!(
            "{}/api/agents/{}/session",
            server.base_url, agent_a
        ))
        .send()
        .await
        .unwrap();
    let a_session: serde_json::Value = resp.json().await.unwrap();
    let a_session_id = a_session["session_id"].as_str().unwrap().to_string();
    let resp = client
        .get(format!(
            "{}/api/agents/{}/session?session_id={}",
            server.base_url, agent_a, a_session_id
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["session_id"].as_str().unwrap(), a_session_id);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_agent_session_trajectory_export_empty() {
    let server = start_test_server().await;
    let client = reqwest::Client::new();

    // Spawn agent
    let resp = client
        .post(format!("{}/api/agents", server.base_url))
        .json(&serde_json::json!({"manifest_toml": TEST_MANIFEST}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 201);
    let body: serde_json::Value = resp.json().await.unwrap();
    let agent_id = body["agent_id"].as_str().unwrap().to_string();

    // Read session to discover session_id.
    let resp = client
        .get(format!(
            "{}/api/agents/{}/session",
            server.base_url, agent_id
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let session_body: serde_json::Value = resp.json().await.unwrap();
    let session_id = session_body["session_id"].as_str().unwrap().to_string();

    // Default (json) format
    let resp = client
        .get(format!(
            "{}/api/agents/{}/sessions/{}/trajectory",
            server.base_url, agent_id, session_id
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let ct = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    assert!(ct.starts_with("application/json"), "got content-type: {ct}");
    let disp = resp
        .headers()
        .get("content-disposition")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    assert!(
        disp.contains("trajectory-") && disp.contains(".json"),
        "got disposition: {disp}"
    );
    let bundle: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(bundle["schema_version"], 1);
    assert_eq!(bundle["metadata"]["agent_id"], agent_id);
    assert_eq!(bundle["metadata"]["session_id"], session_id);
    assert_eq!(bundle["metadata"]["model"], "test-model");
    assert_eq!(bundle["metadata"]["provider"], "ollama");
    assert!(bundle["metadata"]["system_prompt_sha256"].is_string());
    assert!(bundle["metadata"]["librefang_version"].is_string());
    assert_eq!(bundle["metadata"]["message_count"], 0);
    assert!(bundle["messages"].as_array().unwrap().is_empty());

    // jsonl format
    let resp = client
        .get(format!(
            "{}/api/agents/{}/sessions/{}/trajectory?format=jsonl",
            server.base_url, agent_id, session_id
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let ct = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    assert!(
        ct.starts_with("application/x-ndjson"),
        "got content-type: {ct}"
    );
    let body_text = resp.text().await.unwrap();
    let lines: Vec<&str> = body_text.lines().collect();
    // empty session → only metadata header line
    assert_eq!(lines.len(), 1, "expected 1 line, got {}", lines.len());
    assert!(lines[0].contains("\"kind\":\"metadata\""));
}

#[tokio::test(flavor = "multi_thread")]
async fn test_agent_session_trajectory_404_on_unknown_session() {
    let server = start_test_server().await;
    let client = reqwest::Client::new();

    // Spawn an agent so we have a valid agent_id
    let resp = client
        .post(format!("{}/api/agents", server.base_url))
        .json(&serde_json::json!({"manifest_toml": TEST_MANIFEST}))
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = resp.json().await.unwrap();
    let agent_id = body["agent_id"].as_str().unwrap().to_string();

    // Random valid-shape session UUID that doesn't exist.
    let bogus = uuid::Uuid::new_v4().to_string();
    let resp = client
        .get(format!(
            "{}/api/agents/{}/sessions/{}/trajectory",
            server.base_url, agent_id, bogus
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 404);
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
        agent_id.clone(),
        AuditAction::AgentMessage,
        "exact match target",
        "custom_error",
    );
    server.state.kernel.audit().record(
        agent_id.clone(),
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
        // Tests synthesize requests without ConnectInfo, so opt in to the
        // open-server path to keep them green.
        allow_no_auth: true,
        audit_log: None,
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
            "/api/agents/{id}/sessions/{session_id}/trajectory",
            axum::routing::get(routes::export_session_trajectory),
        )
        .route(
            "/api/agents/{id}/sessions/{session_id}/stream",
            axum::routing::get(routes::attach_session_stream),
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

#[tokio::test(flavor = "multi_thread")]
async fn hand_runtime_config_patch_supports_tristate_and_404() {
    use std::collections::HashMap;

    let harness = start_full_router("secret-key-123").await;

    let instance = match harness
        .state
        .kernel
        .activate_hand("apitester", HashMap::new())
    {
        Ok(inst) => inst,
        Err(e) if e.to_string().contains("unsatisfied requirements") => {
            eprintln!("Skipping test: {e}");
            return;
        }
        Err(e) => panic!("apitester hand should activate: {e}"),
    };
    let agent_id = instance.agent_id().expect("apitester agent id");

    let set_response = harness
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method("PATCH")
                .uri(format!("/api/agents/{agent_id}/hand-runtime-config"))
                .header("authorization", "Bearer secret-key-123")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "model": "patched-model",
                        "provider": "patched-provider",
                        "api_key_env": "PATCHED_API_KEY_ENV",
                        "base_url": "https://patched.invalid/v1",
                        "max_tokens": 777,
                        "temperature": 0.7,
                        "web_search_augmentation": "always"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .expect("set override request should succeed");
    assert_eq!(set_response.status(), StatusCode::OK);

    let set_entry = harness
        .state
        .kernel
        .agent_registry()
        .get(agent_id)
        .expect("apitester entry after set");
    assert_eq!(set_entry.manifest.model.model, "patched-model");
    assert_eq!(set_entry.manifest.model.provider, "patched-provider");
    assert_eq!(
        set_entry.manifest.model.api_key_env.as_deref(),
        Some("PATCHED_API_KEY_ENV")
    );
    assert_eq!(
        set_entry.manifest.model.base_url.as_deref(),
        Some("https://patched.invalid/v1")
    );
    assert_eq!(set_entry.manifest.model.max_tokens, 777);
    assert!((set_entry.manifest.model.temperature - 0.7).abs() < 1e-6);
    assert_eq!(
        set_entry.manifest.web_search_augmentation,
        WebSearchAugmentationMode::Always
    );

    let preserve_response = harness
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method("PATCH")
                .uri(format!("/api/agents/{agent_id}/hand-runtime-config"))
                .header("authorization", "Bearer secret-key-123")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "model": "preserved-model",
                        "api_key_env": null,
                        "base_url": null
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .expect("preserve override request should succeed");
    assert_eq!(preserve_response.status(), StatusCode::OK);

    let preserved_entry = harness
        .state
        .kernel
        .agent_registry()
        .get(agent_id)
        .expect("apitester entry after preserve");
    assert_eq!(preserved_entry.manifest.model.model, "preserved-model");
    assert_eq!(
        preserved_entry.manifest.model.api_key_env.as_deref(),
        Some("PATCHED_API_KEY_ENV"),
        "missing api_key_env field must leave prior override unchanged"
    );
    assert_eq!(
        preserved_entry.manifest.model.base_url.as_deref(),
        Some("https://patched.invalid/v1"),
        "missing base_url field must leave prior override unchanged"
    );

    let clear_response = harness
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method("PATCH")
                .uri(format!("/api/agents/{agent_id}/hand-runtime-config"))
                .header("authorization", "Bearer secret-key-123")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "api_key_env": "   ",
                        "base_url": ""
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .expect("clear override request should succeed");
    assert_eq!(clear_response.status(), StatusCode::OK);

    let cleared_entry = harness
        .state
        .kernel
        .agent_registry()
        .get(agent_id)
        .expect("apitester entry after clear");
    assert_ne!(
        cleared_entry.manifest.model.api_key_env.as_deref(),
        Some("PATCHED_API_KEY_ENV"),
        "empty api_key_env must clear the prior runtime override"
    );
    assert_ne!(
        cleared_entry.manifest.model.base_url.as_deref(),
        Some("https://patched.invalid/v1"),
        "empty base_url must clear the prior runtime override"
    );
    assert_eq!(
        cleared_entry.manifest.model.model, "preserved-model",
        "clearing nullable fields must not disturb unrelated non-nullable overrides"
    );

    let not_found = harness
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method("PATCH")
                .uri(format!(
                    "/api/agents/{}/hand-runtime-config",
                    librefang_types::agent::AgentId::new()
                ))
                .header("authorization", "Bearer secret-key-123")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::json!({"model": "x"}).to_string()))
                .unwrap(),
        )
        .await
        .expect("404 request should complete");
    assert_eq!(not_found.status(), StatusCode::NOT_FOUND);
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

// ---------------------------------------------------------------------------
// Multi-client session attach (GET /api/agents/{id}/sessions/{sid}/stream)
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn test_attach_session_stream_404_for_unknown_agent() {
    let server = start_test_server().await;
    let client = reqwest::Client::new();

    let bogus_agent = uuid::Uuid::new_v4();
    let bogus_session = uuid::Uuid::new_v4();

    let resp = client
        .get(format!(
            "{}/api/agents/{}/sessions/{}/stream",
            server.base_url, bogus_agent, bogus_session
        ))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), reqwest::StatusCode::NOT_FOUND);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_attach_session_stream_fans_out_to_multiple_clients() {
    use futures::StreamExt as _;
    use librefang_runtime::llm_driver::StreamEvent;
    use std::time::Duration;

    let server = start_test_server().await;
    let client = reqwest::Client::new();

    // Spawn an agent (ollama, no LLM call needed).
    let resp = client
        .post(format!("{}/api/agents", server.base_url))
        .json(&serde_json::json!({ "manifest_toml": TEST_MANIFEST }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 201);
    let body: serde_json::Value = resp.json().await.unwrap();
    let agent_id_str = body["agent_id"].as_str().unwrap().to_string();
    let agent_id: librefang_types::agent::AgentId = agent_id_str.parse().unwrap();

    // Pull the agent's canonical session id from the registry — the attach
    // route validates the session belongs to the agent.
    let session_id = server
        .state
        .kernel
        .agent_registry()
        .get(agent_id)
        .unwrap()
        .session_id;

    let url = format!(
        "{}/api/agents/{}/sessions/{}/stream",
        server.base_url, agent_id_str, session_id
    );

    // Helper that opens an SSE attach connection and reads until it sees a
    // complete SSE frame (one `\n\n` boundary) or the timeout elapses. Returns
    // the bytes accumulated so the test can assert on the published payload.
    async fn read_first_frame(client: reqwest::Client, url: String) -> String {
        let resp = client.get(url).send().await.unwrap();
        assert_eq!(resp.status(), reqwest::StatusCode::OK);
        let mut buf: Vec<u8> = Vec::new();
        let mut stream = resp.bytes_stream();
        let _ = tokio::time::timeout(Duration::from_secs(2), async {
            while let Some(Ok(chunk)) = stream.next().await {
                buf.extend_from_slice(&chunk);
                if buf.windows(2).any(|w| w == b"\n\n") {
                    return;
                }
            }
        })
        .await;
        String::from_utf8_lossy(&buf).to_string()
    }

    let attacher_a = tokio::spawn(read_first_frame(client.clone(), url.clone()));
    let attacher_b = tokio::spawn(read_first_frame(client.clone(), url.clone()));

    // Wait until both attachers have completed `subscribe()` inside the
    // handler before publishing — broadcast is fire-and-forget for events
    // that arrive with zero subscribers, so a sleep-based wait would be
    // racy on slow CI. Poll receiver_count until it reaches 2.
    let hub = server.state.kernel.session_stream_hub();
    let sender = hub.sender(session_id);
    let waited = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if sender.receiver_count() >= 2 {
                return;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await;
    assert!(
        waited.is_ok(),
        "both attachers should subscribe within 5s; receiver_count={}",
        sender.receiver_count()
    );

    let receiver_count = sender
        .send(StreamEvent::TextDelta {
            text: "hello-multiattach".to_string(),
        })
        .expect("at least one receiver should be attached");
    assert!(
        receiver_count >= 2,
        "expected both attachers to be subscribed before publish; got {receiver_count}"
    );

    let body_a = attacher_a.await.unwrap();
    let body_b = attacher_b.await.unwrap();

    assert!(
        body_a.contains("hello-multiattach"),
        "client A body should contain published event: {body_a}"
    );
    assert!(
        body_b.contains("hello-multiattach"),
        "client B body should contain published event: {body_b}"
    );
}

// ---------------------------------------------------------------------------
// Memory endpoint regression tests for issue #3070:
// When `[proactive_memory] enabled = false`, GET /api/memory and
// GET /api/memory/stats must return 200 with `proactive_enabled: false`,
// not 500. Disabled is a config state, not a server error.
// ---------------------------------------------------------------------------

/// Build a router harness with `proactive_memory.enabled` toggleable.
async fn start_full_router_with_proactive(enabled: bool) -> FullRouterHarness {
    let tmp = tempfile::tempdir().expect("Failed to create temp dir");

    librefang_runtime::registry_sync::sync_registry(
        tmp.path(),
        librefang_runtime::registry_sync::DEFAULT_CACHE_TTL_SECS,
        "",
    );

    let proactive = librefang_types::memory::ProactiveMemoryConfig {
        enabled,
        ..librefang_types::memory::ProactiveMemoryConfig::default()
    };

    let config = KernelConfig {
        home_dir: tmp.path().to_path_buf(),
        data_dir: tmp.path().join("data"),
        default_model: DefaultModelConfig {
            provider: "ollama".to_string(),
            model: "test-model".to_string(),
            api_key_env: "OLLAMA_API_KEY".to_string(),
            base_url: None,
            message_timeout_secs: 300,
            extra_params: std::collections::HashMap::new(),
            cli_profile_dirs: Vec::new(),
        },
        proactive_memory: proactive,
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

/// Build a GET request to `uri` and inject loopback `ConnectInfo` so the
/// auth middleware treats it as a localhost caller (matching production
/// dev-UX semantics). Without this, oneshot tests have no `ConnectInfo`
/// extension and the fail-closed branch returns 401 for non-public paths.
fn loopback_get(uri: &str) -> Request<Body> {
    let mut request = Request::builder().uri(uri).body(Body::empty()).unwrap();
    request
        .extensions_mut()
        .insert(axum::extract::ConnectInfo(std::net::SocketAddr::from((
            [127, 0, 0, 1],
            0,
        ))));
    request
}

#[tokio::test(flavor = "multi_thread")]
async fn test_memory_list_returns_200_when_proactive_disabled() {
    let harness = start_full_router_with_proactive(false).await;

    let response = harness
        .app
        .clone()
        .oneshot(loopback_get("/api/memory"))
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        StatusCode::OK,
        "/api/memory must not 500 when proactive memory is disabled"
    );

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

    assert_eq!(json["proactive_enabled"], serde_json::json!(false));
    assert_eq!(json["total"], serde_json::json!(0));
    assert!(
        json["memories"].as_array().is_some_and(|a| a.is_empty()),
        "memories must be an empty array, got {}",
        json["memories"]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_memory_stats_returns_200_when_proactive_disabled() {
    let harness = start_full_router_with_proactive(false).await;

    let response = harness
        .app
        .clone()
        .oneshot(loopback_get("/api/memory/stats"))
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        StatusCode::OK,
        "/api/memory/stats must not 500 when proactive memory is disabled"
    );

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

    assert_eq!(json["proactive_enabled"], serde_json::json!(false));
    assert!(
        json["stats"].is_null(),
        "stats must be null when disabled, got {}",
        json["stats"]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_memory_list_includes_proactive_enabled_when_enabled() {
    let harness = start_full_router_with_proactive(true).await;

    let response = harness
        .app
        .clone()
        .oneshot(loopback_get("/api/memory"))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

    // When enabled the legacy fields stay intact and `proactive_enabled: true`
    // is added so the dashboard can branch on a single field.
    assert_eq!(json["proactive_enabled"], serde_json::json!(true));
    assert!(json["memories"].is_array(), "memories must be an array");
    assert!(json["total"].is_number(), "total must be a number");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_memory_stats_includes_proactive_enabled_when_enabled() {
    let harness = start_full_router_with_proactive(true).await;

    let response = harness
        .app
        .clone()
        .oneshot(loopback_get("/api/memory/stats"))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

    assert_eq!(json["proactive_enabled"], serde_json::json!(true));
    // Existing fields remain present; we only assert their types so we don't
    // couple to a specific empty-database snapshot.
    assert!(json["total"].is_number() || json["total"].is_null());
}

// ───────────────────────────────────────────────────────────────────────
// RBAC M5 — admin-only audit/budget endpoints
//
// These pin the contract for the four new HTTP endpoints
// (`/api/audit/query`, `/api/audit/export`, `/api/budget/users`,
// `/api/budget/users/{id}`):
//
//   1. Anonymous callers (loopback / `LIBREFANG_ALLOW_NO_AUTH=1`) MUST be
//      rejected — even on a no-auth deployment, the audit chain is too
//      sensitive to expose without an admin api_key.
//   2. Sub-Admin authenticated callers MUST be rejected by the in-handler
//      `require_admin` gate. Middleware lets every authenticated GET
//      through regardless of role; the in-handler check is the only thing
//      stopping a Viewer from reading the chain. This test exists so a
//      future refactor that drops the in-handler gate surfaces as a
//      failure rather than as silent privacy exposure.
//   3. CSV export MUST emit the documented `Content-Type` and
//      `Content-Disposition` so the dashboard download flow keeps working.
//   4. `/api/budget/users/{id}` MUST surface `enforced: false` so the M6
//      dashboard can warn users that `alert_breach` is informational
//      until the M5-followup per-user budget enforcement lands.
// ───────────────────────────────────────────────────────────────────────

use librefang_kernel::auth::UserRole as KernelUserRole;
use librefang_types::config::UserConfig;

/// Build a test server with RBAC users wired into both `KernelConfig.users`
/// and `AuthState.user_api_keys`, plus the audit + budget routes from
/// `routes::audit::router()` / `routes::budget::router()`.
///
/// Each tuple is `(name, role_str, api_key)`. The api_key hash is
/// computed via `librefang_api::password_hash::hash_password` so the
/// auth middleware accepts the corresponding `Bearer` header.
///
/// Audit log handle is plumbed into `AuthState.audit_log` so denials
/// from the in-handler `require_admin` are recorded — same as
/// production (`server.rs::build_router`).
async fn start_test_server_with_rbac_users(
    api_key: &str,
    users: Vec<(&str, &str, &str)>,
) -> TestServer {
    let tmp = tempfile::tempdir().expect("Failed to create temp dir");

    let mut user_configs: Vec<UserConfig> = Vec::with_capacity(users.len());
    let mut api_user_records: Vec<middleware::ApiUserAuth> = Vec::with_capacity(users.len());
    for (name, role_str, key) in &users {
        let hash =
            librefang_api::password_hash::hash_password(key).expect("password hash should succeed");
        user_configs.push(UserConfig {
            name: (*name).to_string(),
            role: (*role_str).to_string(),
            channel_bindings: std::collections::HashMap::new(),
            api_key_hash: Some(hash.clone()),
            ..Default::default()
        });
        api_user_records.push(middleware::ApiUserAuth {
            name: (*name).to_string(),
            role: KernelUserRole::from_str_role(role_str),
            api_key_hash: hash,
            user_id: librefang_types::agent::UserId::from_name(name),
        });
    }

    let config = KernelConfig {
        home_dir: tmp.path().to_path_buf(),
        data_dir: tmp.path().join("data"),
        api_key: api_key.to_string(),
        users: user_configs,
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

    let audit_log = kernel.audit().clone();

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
        user_api_keys: Arc::new(api_user_records),
        require_auth_for_reads: false,
        // Anonymous-rejection tests rely on this — we synthesize requests
        // without a Bearer header and need them to flow through to the
        // in-handler `require_admin` gate (where they get 403'd). Without
        // `allow_no_auth = true` the middleware would 401 first.
        allow_no_auth: true,
        audit_log: Some(audit_log),
    };

    let app = Router::new()
        // Wire the admin-gated RBAC routes under `/api/` — sufficient for
        // these tests. Other RBAC layers (channel bindings, tool policy)
        // are exercised by the kernel-level tests. The authz router is
        // mounted alongside audit/budget so the effective-permissions
        // tests can hit it through the same auth middleware. The users
        // router is mounted so M3 (#3205) policy PUT/GET tests run the
        // full middleware stack (owner-only gate).
        .nest("/api", routes::audit::router())
        .nest("/api", routes::budget::router())
        .nest("/api", routes::authz::router())
        .nest("/api", routes::users::router())
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
async fn test_audit_query_rejects_anonymous() {
    // Anonymous (no Bearer header) callers MUST be denied — the audit
    // chain is too sensitive to leak. With both `api_key` and
    // `user_api_keys` configured, `allow_no_auth` no longer short-circuits
    // (see middleware.rs:501-526) so the request is rejected at the
    // middleware layer with 401 BEFORE reaching the in-handler
    // `require_admin` gate. That's an earlier, stricter rejection — the
    // safety property ("anonymous cannot read audit") still holds.
    let server =
        start_test_server_with_rbac_users("any-key", vec![("Alice", "admin", "alice-admin-key")])
            .await;
    let client = reqwest::Client::new();
    let resp = client
        .get(format!("{}/api/audit/query", server.base_url))
        .send()
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        401,
        "anonymous /api/audit/query must be rejected at the middleware (401)"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_audit_query_rejects_viewer_admin_returns_200() {
    // Pins the in-handler `require_admin` gate. The middleware lets a
    // Viewer GET through (`user_role_allows_request` returns true for
    // every GET); only the handler-side `require_admin` stops it. A
    // refactor that drops that gate must be caught here, NOT in
    // production.
    let server = start_test_server_with_rbac_users(
        "any-key",
        vec![
            ("Alice", "admin", "alice-admin-key"),
            ("Eve", "viewer", "eve-viewer-key"),
        ],
    )
    .await;
    let client = reqwest::Client::new();

    // Viewer → 403
    let viewer = client
        .get(format!("{}/api/audit/query", server.base_url))
        .header("authorization", "Bearer eve-viewer-key")
        .send()
        .await
        .unwrap();
    assert_eq!(
        viewer.status(),
        403,
        "Viewer must be denied at the in-handler require_admin gate"
    );

    // Admin → 200 with valid JSON shape
    let admin = client
        .get(format!("{}/api/audit/query", server.base_url))
        .header("authorization", "Bearer alice-admin-key")
        .send()
        .await
        .unwrap();
    assert_eq!(admin.status(), 200, "Admin must be allowed");
    let body: serde_json::Value = admin.json().await.unwrap();
    // Shape contract: `entries` array, `count`, `limit` fields all
    // present even on an empty audit log.
    assert!(body["entries"].is_array(), "response must carry entries[]");
    assert!(body["count"].is_number(), "response must carry count");
    assert!(body["limit"].is_number(), "response must carry limit");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_audit_export_csv_emits_documented_headers() {
    // The dashboard download flow keys off `Content-Type: text/csv` and
    // a filename in `Content-Disposition`. Pin both so a future refactor
    // of `stream_csv` doesn't silently break the download.
    let server =
        start_test_server_with_rbac_users("any-key", vec![("Alice", "admin", "alice-admin-key")])
            .await;
    let client = reqwest::Client::new();
    let resp = client
        .get(format!("{}/api/audit/export?format=csv", server.base_url))
        .header("authorization", "Bearer alice-admin-key")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200, "Admin CSV export must succeed");
    let ct = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert!(
        ct.starts_with("text/csv"),
        "CSV export must set text/csv; got {ct:?}"
    );
    let cd = resp
        .headers()
        .get("content-disposition")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert!(
        cd.contains("attachment") && cd.contains("audit.csv"),
        "Content-Disposition must trigger an `audit.csv` download; got {cd:?}"
    );
    // First line must be the documented header row, regardless of how
    // many entries the chain holds.
    let text = resp.text().await.unwrap();
    let first_line = text.lines().next().unwrap_or("");
    assert_eq!(
        first_line, "seq,timestamp,agent_id,action,detail,outcome,user_id,channel,hash",
        "CSV header row schema must remain stable for downstream parsers"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_user_budget_detail_includes_enforced_true() {
    // Per-user budget enforcement landed in commit 4a00a646 ("RBAC M5 —
    // wire per-user budget enforcement"): AuthManager::budget_for,
    // MeteringEngine::check_user_budget, and the post-call arm in
    // kernel::execute_llm_agent now actually deny over-budget calls.
    // `/api/budget/users/{id}` reports `enforced: true` accordingly.
    let server =
        start_test_server_with_rbac_users("any-key", vec![("Alice", "admin", "alice-admin-key")])
            .await;
    let client = reqwest::Client::new();
    let resp = client
        .get(format!("{}/api/budget/users/Alice", server.base_url))
        .header("authorization", "Bearer alice-admin-key")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(
        body["enforced"],
        serde_json::json!(true),
        "enforced flag must be `true` now that per-user budget enforcement is wired"
    );
    // Defensive: the spend numerics must also be present so the
    // dashboard can render even on an empty database.
    assert!(body["hourly"]["spend"].is_number());
    assert!(body["daily"]["spend"].is_number());
    assert!(body["monthly"]["spend"].is_number());
    assert!(body["alert_breach"].is_boolean());
}

// ───────────────────────────────────────────────────────────────────────
// Effective-permissions snapshot — `/api/authz/effective/{user_id}`
//
// Pins:
//   1. Admin GET returns 200 with every documented section populated for
//      a user that was seeded with non-default tool_policy / memory_access
//      / budget. Catches a regression where the kernel-side getter starts
//      collapsing slices to None or the route serialiser drops fields.
//   2. Viewer GET is rejected at the in-handler `require_admin` gate
//      (403). The middleware lets Viewer through GETs by default — only
//      the handler stops them.
//   3. Unknown user IDs return 404 (NOT a synthesised "guest defaults"
//      payload — the simulator's job is to show what's configured).
//   4. Anonymous (no Bearer header) is rejected — same model as audit.
// ───────────────────────────────────────────────────────────────────────

use librefang_types::user_policy::{
    ChannelToolPolicy, UserMemoryAccess, UserToolCategories, UserToolPolicy,
};

/// Variant of `start_test_server_with_rbac_users` that lets the caller
/// inject pre-built `UserConfig` rows so per-user policy fields
/// (`tool_policy`, `memory_access`, `budget`, …) can be seeded for
/// the effective-permissions tests.
async fn start_test_server_with_full_user_configs(
    api_key: &str,
    users: Vec<(UserConfig, &str)>,
) -> TestServer {
    let tmp = tempfile::tempdir().expect("Failed to create temp dir");

    let mut user_configs: Vec<UserConfig> = Vec::with_capacity(users.len());
    let mut api_user_records: Vec<middleware::ApiUserAuth> = Vec::with_capacity(users.len());
    for (cfg, key) in &users {
        let hash =
            librefang_api::password_hash::hash_password(key).expect("password hash should succeed");
        let mut cfg = cfg.clone();
        cfg.api_key_hash = Some(hash.clone());
        api_user_records.push(middleware::ApiUserAuth {
            name: cfg.name.clone(),
            role: KernelUserRole::from_str_role(&cfg.role),
            api_key_hash: hash,
            user_id: librefang_types::agent::UserId::from_name(&cfg.name),
        });
        user_configs.push(cfg);
    }

    let config = KernelConfig {
        home_dir: tmp.path().to_path_buf(),
        data_dir: tmp.path().join("data"),
        api_key: api_key.to_string(),
        users: user_configs,
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

    let audit_log = kernel.audit().clone();

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
        user_api_keys: Arc::new(api_user_records),
        require_auth_for_reads: false,
        allow_no_auth: true,
        audit_log: Some(audit_log),
    };

    let app = Router::new()
        .nest("/api", routes::audit::router())
        .nest("/api", routes::budget::router())
        .nest("/api", routes::authz::router())
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
async fn test_effective_permissions_admin_returns_200_with_full_payload() {
    // Seed Alice with non-default values for every per-user RBAC slice
    // so the snapshot must surface each one. A regression that drops a
    // slice from the response (e.g. forgetting to expose `budget` or
    // collapsing `memory_access` to `None`) will fail one of the
    // assertions below.
    let mut alice_bindings = std::collections::HashMap::new();
    alice_bindings.insert("telegram".to_string(), "555111".to_string());
    alice_bindings.insert("discord".to_string(), "8001".to_string());

    let mut alice_channel_rules = std::collections::HashMap::new();
    alice_channel_rules.insert(
        "telegram".to_string(),
        ChannelToolPolicy {
            allowed_tools: vec!["web_*".to_string()],
            denied_tools: vec!["shell_*".to_string()],
        },
    );

    let alice = UserConfig {
        name: "Alice".to_string(),
        role: "admin".to_string(),
        channel_bindings: alice_bindings,
        api_key_hash: None,
        budget: Some(librefang_types::config::UserBudgetConfig {
            max_hourly_usd: 1.0,
            max_daily_usd: 10.0,
            max_monthly_usd: 100.0,
            alert_threshold: 0.75,
        }),
        tool_policy: Some(UserToolPolicy {
            allowed_tools: vec!["read_*".to_string(), "list_*".to_string()],
            denied_tools: vec!["dangerous_tool".to_string()],
        }),
        tool_categories: Some(UserToolCategories {
            allowed_groups: vec!["safe".to_string()],
            denied_groups: vec!["destructive".to_string()],
        }),
        memory_access: Some(UserMemoryAccess {
            readable_namespaces: vec!["proactive".to_string(), "kv:*".to_string()],
            writable_namespaces: vec!["kv:*".to_string()],
            pii_access: true,
            export_allowed: false,
            delete_allowed: true,
        }),
        channel_tool_rules: alice_channel_rules,
    };

    let server =
        start_test_server_with_full_user_configs("any-key", vec![(alice, "alice-admin-key")]).await;
    let client = reqwest::Client::new();
    let resp = client
        .get(format!("{}/api/authz/effective/Alice", server.base_url))
        .header("authorization", "Bearer alice-admin-key")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200, "Admin must receive 200");
    let body: serde_json::Value = resp.json().await.unwrap();

    // Identity fields
    assert_eq!(body["name"], "Alice");
    assert_eq!(body["role"], "admin");
    assert!(
        body["user_id"].is_string() && !body["user_id"].as_str().unwrap().is_empty(),
        "user_id must be a non-empty stringified UUID"
    );

    // Per-user tool policy round-trip
    assert_eq!(
        body["tool_policy"]["allowed_tools"],
        serde_json::json!(["read_*", "list_*"])
    );
    assert_eq!(
        body["tool_policy"]["denied_tools"],
        serde_json::json!(["dangerous_tool"])
    );

    // Tool categories
    assert_eq!(
        body["tool_categories"]["allowed_groups"],
        serde_json::json!(["safe"])
    );
    assert_eq!(
        body["tool_categories"]["denied_groups"],
        serde_json::json!(["destructive"])
    );

    // Memory access (PII flag is the load-bearing one for the dashboard
    // badge — pin it)
    assert_eq!(body["memory_access"]["pii_access"], serde_json::json!(true));
    assert_eq!(
        body["memory_access"]["readable_namespaces"],
        serde_json::json!(["proactive", "kv:*"])
    );
    assert_eq!(
        body["memory_access"]["writable_namespaces"],
        serde_json::json!(["kv:*"])
    );
    assert_eq!(
        body["memory_access"]["export_allowed"],
        serde_json::json!(false)
    );
    assert_eq!(
        body["memory_access"]["delete_allowed"],
        serde_json::json!(true)
    );

    // Budget
    assert_eq!(body["budget"]["max_hourly_usd"], serde_json::json!(1.0));
    assert_eq!(body["budget"]["max_daily_usd"], serde_json::json!(10.0));
    assert_eq!(body["budget"]["max_monthly_usd"], serde_json::json!(100.0));
    assert_eq!(body["budget"]["alert_threshold"], serde_json::json!(0.75));

    // Channel rules
    assert_eq!(
        body["channel_tool_rules"]["telegram"]["allowed_tools"],
        serde_json::json!(["web_*"])
    );
    assert_eq!(
        body["channel_tool_rules"]["telegram"]["denied_tools"],
        serde_json::json!(["shell_*"])
    );

    // Channel bindings (cross-platform identity)
    assert_eq!(
        body["channel_bindings"]["telegram"],
        serde_json::json!("555111")
    );
    assert_eq!(
        body["channel_bindings"]["discord"],
        serde_json::json!("8001")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_effective_permissions_viewer_rejected_403() {
    // Pins the in-handler `require_admin` gate. The middleware lets
    // Viewer GET through; only the handler stops them with 403. A
    // refactor that drops that gate must surface here, not in
    // production where the leak would be silent.
    let alice = UserConfig {
        name: "Alice".to_string(),
        role: "admin".to_string(),
        ..Default::default()
    };
    let eve = UserConfig {
        name: "Eve".to_string(),
        role: "viewer".to_string(),
        ..Default::default()
    };
    let server = start_test_server_with_full_user_configs(
        "any-key",
        vec![(alice, "alice-admin-key"), (eve, "eve-viewer-key")],
    )
    .await;
    let client = reqwest::Client::new();
    let resp = client
        .get(format!("{}/api/authz/effective/Alice", server.base_url))
        .header("authorization", "Bearer eve-viewer-key")
        .send()
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        403,
        "Viewer must be denied by the in-handler require_admin gate"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_effective_permissions_unknown_user_404() {
    // Unknown user → 404 with a useful message. We deliberately do NOT
    // synthesise "guest defaults" — the simulator's job is to show what
    // an admin configured, not to invent inputs that no AuthManager
    // entry actually carries.
    let alice = UserConfig {
        name: "Alice".to_string(),
        role: "admin".to_string(),
        ..Default::default()
    };
    let server =
        start_test_server_with_full_user_configs("any-key", vec![(alice, "alice-admin-key")]).await;
    let client = reqwest::Client::new();
    let resp = client
        .get(format!("{}/api/authz/effective/Nobody", server.base_url))
        .header("authorization", "Bearer alice-admin-key")
        .send()
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        404,
        "Unknown user must be 404, not a synthesised guest payload"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_effective_permissions_rejects_anonymous() {
    // Anonymous (no Bearer header) callers MUST be denied. Same
    // contract as `/api/audit/query` — the snapshot exposes per-user
    // policy and channel bindings, which is too sensitive to leak even
    // on loopback. With both `api_key` and `user_api_keys` configured,
    // the middleware short-circuits to 401 before reaching the handler;
    // either status code (401 / 403) is an acceptable rejection.
    let alice = UserConfig {
        name: "Alice".to_string(),
        role: "admin".to_string(),
        ..Default::default()
    };
    let server =
        start_test_server_with_full_user_configs("any-key", vec![(alice, "alice-admin-key")]).await;
    let client = reqwest::Client::new();
    let resp = client
        .get(format!("{}/api/authz/effective/Alice", server.base_url))
        .send()
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        401,
        "anonymous /api/authz/effective must be rejected at the middleware (401)"
    );
}

/// Pins the "raw Option" discrimination on the snapshot. A user that
/// declared `tool_policy: None` (omitted in TOML) and a user that
/// declared `tool_policy: Some(UserToolPolicy::default())` (explicit
/// empty allow/deny lists) MUST surface distinctly in the JSON:
/// `null` vs `{"allowed_tools": [], "denied_tools": []}`. Same for
/// `tool_categories` and `memory_access`.
///
/// Regression: an earlier draft collapsed both shapes to `None` by
/// comparing the resolved struct to its `Default::default()` after
/// `populate`'s `unwrap_or_default()`. That made the "Configured /
/// Not configured" badge in the simulator silently lie about
/// explicit-empty configs. This test fails closed if `populate`
/// drops the raw `Option<...>` again.
#[tokio::test(flavor = "multi_thread")]
async fn test_effective_permissions_distinguishes_none_from_empty() {
    let bare = UserConfig {
        name: "Bare".to_string(),
        role: "user".to_string(),
        // tool_policy / tool_categories / memory_access default to None.
        ..Default::default()
    };
    let explicit_empty = UserConfig {
        name: "Empty".to_string(),
        role: "user".to_string(),
        tool_policy: Some(UserToolPolicy::default()),
        tool_categories: Some(UserToolCategories::default()),
        memory_access: Some(UserMemoryAccess::default()),
        ..Default::default()
    };
    let admin = UserConfig {
        name: "Alice".to_string(),
        role: "admin".to_string(),
        ..Default::default()
    };
    let server = start_test_server_with_full_user_configs(
        "any-key",
        vec![
            (admin, "alice-admin-key"),
            (bare, "bare-key"),
            (explicit_empty, "empty-key"),
        ],
    )
    .await;
    let client = reqwest::Client::new();

    let bare_body: serde_json::Value = client
        .get(format!("{}/api/authz/effective/Bare", server.base_url))
        .header("authorization", "Bearer alice-admin-key")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(
        bare_body["tool_policy"].is_null(),
        "tool_policy must be null when UserConfig.tool_policy = None, got {:?}",
        bare_body["tool_policy"]
    );
    assert!(
        bare_body["tool_categories"].is_null(),
        "tool_categories must be null when omitted"
    );
    assert!(
        bare_body["memory_access"].is_null(),
        "memory_access must be null when omitted"
    );

    let empty_body: serde_json::Value = client
        .get(format!("{}/api/authz/effective/Empty", server.base_url))
        .header("authorization", "Bearer alice-admin-key")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(
        empty_body["tool_policy"].is_object(),
        "tool_policy must be an object (not null) when UserConfig.tool_policy = Some(default), got {:?}",
        empty_body["tool_policy"]
    );
    assert_eq!(
        empty_body["tool_policy"]["allowed_tools"],
        serde_json::json!([])
    );
    assert_eq!(
        empty_body["tool_policy"]["denied_tools"],
        serde_json::json!([])
    );
    assert!(empty_body["tool_categories"].is_object());
    assert!(empty_body["memory_access"].is_object());
}

/// RBAC M3 (#3205) follow-up — `PUT /api/users/{name}/policy` rewrites the
/// caller's authorization surface, so it must travel through the same
/// owner-only gate as `POST /api/users` and `DELETE /api/users/{name}`.
/// An Admin api key has to be 403'd here; otherwise an Admin could
/// silently grant themselves `denied_tools = []` and bypass downstream
/// per-user denials.
#[tokio::test(flavor = "multi_thread")]
async fn users_policy_put_owner_only() {
    let server = start_test_server_with_rbac_users(
        "any-key",
        vec![
            ("Owner1", "owner", "owner-key"),
            ("Alice", "admin", "alice-admin-key"),
        ],
    )
    .await;
    let client = reqwest::Client::new();

    // Admin is denied at the middleware before reaching the handler.
    let resp = client
        .put(format!("{}/api/users/Alice/policy", server.base_url))
        .header("authorization", "Bearer alice-admin-key")
        .json(&serde_json::json!({
            "tool_policy": { "allowed_tools": ["web_*"], "denied_tools": [] }
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        403,
        "Admin must be denied PUT /api/users/{{name}}/policy by the owner-only middleware gate"
    );

    // Owner sails through. Used to confirm the route IS wired — without
    // this assertion a 404 from a missing route would also produce a 4xx
    // and pretend the gate was working.
    let resp = client
        .put(format!("{}/api/users/Alice/policy", server.base_url))
        .header("authorization", "Bearer owner-key")
        .json(&serde_json::json!({
            "tool_policy": { "allowed_tools": ["web_*"], "denied_tools": [] }
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        200,
        "Owner must be allowed to upsert per-user policy"
    );
}
