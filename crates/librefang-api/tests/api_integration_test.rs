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
use librefang_kernel::audit::AuditAction;
use librefang_kernel::LibreFangKernel;
use librefang_testing::{MockKernelBuilder, TestAppState};
use librefang_types::agent::WebSearchAugmentationMode;
use librefang_types::config::{DefaultModelConfig, KernelConfig};
use std::path::PathBuf;
use std::sync::Arc;
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
    start_test_server_with_provider_at(provider, model, api_key_env, None).await
}

/// Start a test server whose default provider points at `base_url`.
///
/// `None` keeps the provider's registry default (the real endpoint), which is
/// what every pre-existing caller wants. `Some(url)` aims the driver at a local
/// stub — a `wiremock::MockServer` speaking the Ollama protocol — so a test can
/// drive a complete agent turn through the production message handler without a
/// provider credential and without touching the network.
async fn start_test_server_with_provider_at(
    provider: &str,
    model: &str,
    api_key_env: &str,
    base_url: Option<&str>,
) -> TestServer {
    let provider = provider.to_string();
    let model = model.to_string();
    let api_key_env = api_key_env.to_string();
    let base_url = base_url.map(str::to_string);
    start_test_server_with_builder(MockKernelBuilder::new().with_config(move |cfg| {
        cfg.default_model.provider = provider;
        cfg.default_model.model = model;
        cfg.default_model.api_key_env = api_key_env;
        cfg.default_model.base_url = base_url;
    }))
    .await
}

/// Boot the shared test router on top of an arbitrary [`MockKernelBuilder`].
///
/// Split out of `start_test_server_with_provider` so a test that needs more
/// than provider/model/base_url — a seeded model catalog, for instance — gets
/// the same router, auth layer, and listener as every other test rather than a
/// hand-rolled copy of them.
async fn start_test_server_with_builder(builder: MockKernelBuilder) -> TestServer {
    let test = TestAppState::with_builder(builder);
    let config_path = test.tmp_path().join("config.toml");
    let test = test.with_config_path(config_path.clone());
    let (state, _tmp, _) = test.into_parts();
    state.kernel.clone().set_self_handle();

    // Exercise the same identity attribution as the production router.
    // With no auth configured, loopback requests receive the synthetic Owner principal that authorizes the daemon's own MCP bridge headers.
    let auth_state = middleware::AuthState {
        api_key_lock: state.api_key_lock.clone(),
        master_key: state.master_key.clone(),
        active_sessions: state.active_sessions.clone(),
        dashboard_auth_enabled: false,
        user_api_keys: state.user_api_keys.clone(),
        require_auth_for_reads: false,
        allow_no_auth: false,
        audit_log: None,
    };

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
            "/api/uploads/{file_id}",
            axum::routing::get(routes::serve_upload),
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
        .layer(axum::middleware::from_fn_with_state(
            auth_state,
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
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .await
        .unwrap();
    });

    TestServer {
        base_url: format!("http://{}", addr),
        config_path,
        state,
        _tmp,
    }
}

async fn start_full_router(api_key: &str) -> FullRouterHarness {
    let tmp = tempfile::tempdir().expect("Failed to create temp dir");

    // Seed the pinned registry fixture into the temp home_dir so the kernel boots with a populated model catalog, offline.
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
        ("/locales/uk.json", "Чат"),
        ("/locales/ko.json", "채팅"),
        ("/locales/pl.json", "Czat"),
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
              model: "anthropic/some-model"
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

// ── /api/migrate path containment (audit: migrate-arbitrary-paths) ──────────
//
// `POST /api/migrate` previously consumed `source_dir` / `target_dir` via
// `PathBuf::from(...)` with NO containment check. Admin-only, but a
// compromised Admin token (leaked CI env, phishing) became a probe for any
// `.exists()` readable as the daemon UID AND a write primitive into any
// `target_dir`. The handler now canonicalizes both paths and requires them to
// fall under an allowlist (currently just `home_dir`). These tests pin that
// contract by hitting the FULL layered router so a future reorder or revert
// is caught.

/// `source_dir = "/etc"` (an existing system path, not under daemon home)
/// must be rejected with 400 — pins the `.exists()` oracle closure.
#[tokio::test(flavor = "multi_thread")]
async fn test_run_migrate_rejects_source_dir_outside_home() {
    let harness = start_full_router("").await;

    let mut request = Request::builder()
        .method("POST")
        .uri("/api/migrate")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::to_vec(&serde_json::json!({
                "source": "openclaw",
                "source_dir": "/etc",
                "target_dir": "",
                "dry_run": true
            }))
            .unwrap(),
        ))
        .unwrap();
    request
        .extensions_mut()
        .insert(axum::extract::ConnectInfo(std::net::SocketAddr::from((
            [127, 0, 0, 1],
            0,
        ))));

    let response = harness.app.clone().oneshot(request).await.unwrap();
    assert_eq!(
        response.status(),
        StatusCode::BAD_REQUEST,
        "source_dir='/etc' must be rejected — outside the allowed migration roots"
    );

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let err_text = serde_json::to_string(&json).unwrap();
    assert!(
        err_text.contains("source_dir") && err_text.contains("allowed"),
        "error must name the rejected field and the allowlist policy: {err_text}"
    );
}

/// `source_dir = "../../../etc"` (a relative traversal) must be rejected after
/// canonicalization — pins that `..`-style escapes don't bypass the check.
#[tokio::test(flavor = "multi_thread")]
async fn test_run_migrate_rejects_source_dir_dotdot_traversal() {
    let harness = start_full_router("").await;

    let mut request = Request::builder()
        .method("POST")
        .uri("/api/migrate")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::to_vec(&serde_json::json!({
                "source": "openclaw",
                "source_dir": "../../../etc",
                "target_dir": "",
                "dry_run": true
            }))
            .unwrap(),
        ))
        .unwrap();
    request
        .extensions_mut()
        .insert(axum::extract::ConnectInfo(std::net::SocketAddr::from((
            [127, 0, 0, 1],
            0,
        ))));

    let response = harness.app.clone().oneshot(request).await.unwrap();
    assert_eq!(
        response.status(),
        StatusCode::BAD_REQUEST,
        "source_dir='../../../etc' must be rejected after canonicalization"
    );
}

/// `target_dir = "/tmp/foo"` (an existing non-home directory). The previous
/// behaviour silently wrote there. Now must be rejected — `target_dir`
/// resolution walks up to find an existing ancestor (`/tmp`), canonicalizes
/// it, and rejects when it's not under `home_dir`.
#[tokio::test(flavor = "multi_thread")]
async fn test_run_migrate_rejects_target_dir_outside_home() {
    let harness = start_full_router("").await;

    // Provide a valid source under home so we isolate the target-dir check.
    let source_dir = harness.state.kernel.home_dir().join("legit-source");
    std::fs::create_dir_all(&source_dir).unwrap();
    std::fs::write(
        source_dir.join("openclaw.json"),
        r#"{ agents: { list: [ { id: "main", name: "Main" } ], defaults: { model: "anthropic/x" } } }"#,
    )
    .unwrap();

    // /tmp/foo-<unique>: nonexistent path under /tmp, which is outside
    // home_dir. The validator walks up to /tmp (existing) and rejects.
    let pid = std::process::id();
    let outside_target = format!("/tmp/librefang-import-outside-{pid}");

    let mut request = Request::builder()
        .method("POST")
        .uri("/api/migrate")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::to_vec(&serde_json::json!({
                "source": "openclaw",
                "source_dir": source_dir.display().to_string(),
                "target_dir": outside_target,
                "dry_run": true
            }))
            .unwrap(),
        ))
        .unwrap();
    request
        .extensions_mut()
        .insert(axum::extract::ConnectInfo(std::net::SocketAddr::from((
            [127, 0, 0, 1],
            0,
        ))));

    let response = harness.app.clone().oneshot(request).await.unwrap();
    assert_eq!(
        response.status(),
        StatusCode::BAD_REQUEST,
        "target_dir outside home_dir must be rejected"
    );

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let err_text = serde_json::to_string(&json).unwrap();
    assert!(
        err_text.contains("target_dir"),
        "error must name the rejected field: {err_text}"
    );

    // And the rejected directory must NOT have been created.
    assert!(
        !std::path::Path::new(&outside_target).exists(),
        "rejected target_dir must not be created on disk"
    );
}

/// Legitimate migration under `home_dir/<sub>` must still succeed — pins that
/// the new check does not break the happy path. Complements
/// `test_run_migrate_uses_daemon_home_when_target_dir_is_empty` by exercising
/// the explicit-target (nonexistent subdir of home) branch of the validator.
#[tokio::test(flavor = "multi_thread")]
async fn test_run_migrate_accepts_paths_inside_home() {
    let harness = start_full_router("").await;

    let source_dir = harness.state.kernel.home_dir().join("containment-source");
    std::fs::create_dir_all(&source_dir).unwrap();
    std::fs::write(
        source_dir.join("openclaw.json"),
        r#"{ agents: { list: [ { id: "main", name: "Main" } ], defaults: { model: "anthropic/x" } } }"#,
    )
    .unwrap();

    // Nonexistent target subdir of home — exercises the
    // canonicalize_nonexistent path (walk up to home, canonicalize, append).
    let target_dir = harness.state.kernel.home_dir().join("containment-target");

    let mut request = Request::builder()
        .method("POST")
        .uri("/api/migrate")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::to_vec(&serde_json::json!({
                "source": "openclaw",
                "source_dir": source_dir.display().to_string(),
                "target_dir": target_dir.display().to_string(),
                "dry_run": true
            }))
            .unwrap(),
        ))
        .unwrap();
    request
        .extensions_mut()
        .insert(axum::extract::ConnectInfo(std::net::SocketAddr::from((
            [127, 0, 0, 1],
            0,
        ))));

    let response = harness.app.clone().oneshot(request).await.unwrap();
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "legitimate migration with both paths under home_dir must still succeed"
    );
}

/// `POST /api/migrate/scan { path: "/etc" }` must be rejected with 400 — pins
/// that the sibling scan endpoint also enforces containment. Without this,
/// the 200-vs-400 `Directory not found` branch of `migrate_scan` is a
/// `.exists()` oracle for arbitrary daemon-UID-readable paths even after
/// `run_migrate` was hardened.
#[tokio::test(flavor = "multi_thread")]
async fn test_migrate_scan_rejects_path_outside_home() {
    let harness = start_full_router("").await;

    let mut request = Request::builder()
        .method("POST")
        .uri("/api/migrate/scan")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::to_vec(&serde_json::json!({ "path": "/etc" })).unwrap(),
        ))
        .unwrap();
    request
        .extensions_mut()
        .insert(axum::extract::ConnectInfo(std::net::SocketAddr::from((
            [127, 0, 0, 1],
            0,
        ))));

    let response = harness.app.clone().oneshot(request).await.unwrap();
    assert_eq!(
        response.status(),
        StatusCode::BAD_REQUEST,
        "migrate_scan path='/etc' must be rejected — outside the allowed migration roots"
    );

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let err_text = serde_json::to_string(&json).unwrap();
    assert!(
        err_text.contains("path") && err_text.contains("allowed"),
        "error must name the rejected field and the allowlist policy: {err_text}"
    );
}

/// End-to-end coverage for the global `enforce_json_body_depth` middleware
/// wired into `server::build_router` (PR #5412). Unit tests in
/// `middleware.rs` exercise the layer against a `Router::new()` stub; this
/// drives a request through the FULL layered router so a missing wiring (or a
/// reorder that puts the guard behind a body-consuming layer) is caught.
///
/// Asserts both directions:
///   (a) a body nested deeper than `MAX_JSON_BODY_DEPTH` (32) is rejected with
///       400 carrying the standard `validation_error` shape — proving the
///       middleware actually fires before the handler; and
///   (b) a normal shallow JSON body still reaches the handler and succeeds —
///       proving the buffer-and-reconstruct path does not corrupt normal
///       traffic.
#[tokio::test(flavor = "multi_thread")]
async fn test_build_router_rejects_deeply_nested_json_body() {
    let harness = start_full_router("").await;

    // Build `[[[ … 1 … ]]]` with a leaf so each bracket contributes a level.
    // 40 > MAX_JSON_BODY_DEPTH (32), so the guard must reject it. Empty
    // arrays would have depth 0 (no items pushed), hence the inner `1`.
    const DEPTH: usize = 40;
    let deep_body = format!("{}1{}", "[".repeat(DEPTH), "]".repeat(DEPTH));

    let mut request = Request::builder()
        .method("POST")
        .uri("/api/migrate")
        .header("content-type", "application/json")
        .body(Body::from(deep_body))
        .unwrap();
    // Empty api_key + loopback ConnectInfo => the request clears auth, so the
    // 400 we observe comes from the depth guard, not the auth layer above it.
    request
        .extensions_mut()
        .insert(axum::extract::ConnectInfo(std::net::SocketAddr::from((
            [127, 0, 0, 1],
            0,
        ))));

    let response = harness.app.clone().oneshot(request).await.unwrap();
    assert_eq!(
        response.status(),
        StatusCode::BAD_REQUEST,
        "deeply nested JSON must be rejected by the global depth guard"
    );

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    // Standard `ApiErrorResponse` shape produced by `ValidationError`. The
    // message text distinguishes the depth guard from the handler's own
    // `Json<MigrateRequest>` deserialization rejection (which would not carry
    // the `validation_error` code).
    assert_eq!(json["code"], "validation_error");
    // The depth message surfaces in both the top-level `message` and the
    // nested `error.message` of the standard `ApiErrorResponse` envelope; the
    // text distinguishes the guard from any handler-level rejection.
    assert!(
        json["message"]
            .as_str()
            .unwrap_or_default()
            .contains("nesting depth"),
        "error body should name the depth violation, got: {json}"
    );

    // (b) A shallow, well-formed JSON body for the same endpoint still flows
    //     through the buffer+reconstruct path and reaches the handler.
    let source_dir = harness.state.kernel.home_dir().join("depth-ok-source");
    std::fs::create_dir_all(&source_dir).unwrap();
    std::fs::write(
        source_dir.join("openclaw.json"),
        r#"{ agents: { list: [ { id: "main", name: "Main" } ], defaults: { model: "anthropic/x" } } }"#,
    )
    .unwrap();

    let mut shallow = Request::builder()
        .method("POST")
        .uri("/api/migrate")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::to_vec(&serde_json::json!({
                "source": "openclaw",
                "source_dir": source_dir.display().to_string(),
                "target_dir": "",
                "dry_run": true
            }))
            .unwrap(),
        ))
        .unwrap();
    shallow
        .extensions_mut()
        .insert(axum::extract::ConnectInfo(std::net::SocketAddr::from((
            [127, 0, 0, 1],
            0,
        ))));

    let shallow_resp = harness.app.clone().oneshot(shallow).await.unwrap();
    assert_eq!(
        shallow_resp.status(),
        StatusCode::OK,
        "shallow JSON must still reach the handler after the depth guard buffers it"
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
    // Refs #4614: DELETE requires `?confirm=true` so canonical UUID
    // purge is gated behind explicit operator intent.
    let resp = client
        .delete(format!(
            "{}/api/agents/{}?confirm=true",
            server.base_url, agent_id
        ))
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
    assert!(
        body.get("label").is_some(),
        "empty session must include label"
    );
    assert!(body["label"].is_null());
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
    let error: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(error["error"]["message"], "Invalid session ID");

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
async fn test_get_agent_session_reuses_materialized_history_images() {
    use base64::Engine as _;
    use librefang_types::agent::AgentId;
    use librefang_types::message::{ContentBlock, Message, MessageContent, Role};

    let server = start_test_server().await;
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/api/agents", server.base_url))
        .json(&serde_json::json!({"manifest_toml": TEST_MANIFEST}))
        .send()
        .await
        .unwrap();
    let spawned: serde_json::Value = resp.json().await.unwrap();
    let agent_id: AgentId = spawned["agent_id"].as_str().unwrap().parse().unwrap();

    let substrate = server.state.kernel.memory_substrate();
    let mut session = substrate.create_session(agent_id).unwrap();
    session.messages.push(Message {
        role: Role::User,
        content: MessageContent::Blocks(vec![ContentBlock::Image {
            media_type: "image/png".to_string(),
            data: base64::engine::general_purpose::STANDARD.encode(b"history image"),
        }]),
        pinned: false,
        timestamp: None,
    });
    substrate.save_session(&session).unwrap();

    let url = format!(
        "{}/api/agents/{}/session?session_id={}",
        server.base_url, agent_id, session.id.0
    );
    let first: serde_json::Value = client.get(&url).send().await.unwrap().json().await.unwrap();
    let second: serde_json::Value = client.get(&url).send().await.unwrap().json().await.unwrap();
    let first_id = first["messages"][0]["images"][0]["file_id"]
        .as_str()
        .unwrap();
    let second_id = second["messages"][0]["images"][0]["file_id"]
        .as_str()
        .unwrap();
    assert_eq!(first_id, second_id);

    let download = client
        .get(format!("{}/api/uploads/{first_id}", server.base_url))
        .send()
        .await
        .unwrap();
    assert_eq!(download.status(), StatusCode::OK);
    assert_eq!(download.bytes().await.unwrap().as_ref(), b"history image");

    let upload_dir = server
        .state
        .kernel
        .config_ref()
        .channels
        .effective_file_download_dir();
    let image_path = upload_dir.join(librefang_types::media::on_disk_name(
        first_id,
        "image/png",
        "",
    ));
    assert_eq!(std::fs::read(&image_path).unwrap(), b"history image");
    std::fs::remove_file(image_path).unwrap();
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
    let workflows = body["items"].as_array().unwrap();
    assert_eq!(workflows.len(), 1);
    assert_eq!(body["total"].as_u64().unwrap(), 1);
    assert_eq!(workflows[0]["name"], "test-workflow");
    assert_eq!(workflows[0]["steps"], 1);

    // Run-aggregate fields are present on every list entry. The workflow
    // has never run, so the dashboard expects last_run/success_rate to be
    // explicitly null (not missing) and run_count to be zero — UI relies
    // on this to render the "no runs" placeholder.
    assert_eq!(workflows[0]["run_count"], 0);
    assert!(
        workflows[0]["last_run"].is_null(),
        "last_run must be null before any run"
    );
    assert!(
        workflows[0]["success_rate"].is_null(),
        "success_rate must be null before any terminal run"
    );
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
    assert!(body["error"]["message"]
        .as_str()
        .unwrap()
        .contains("Invalid"));

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
async fn test_kill_nonexistent_agent_is_idempotent() {
    // #3509: DELETE on a valid UUID that doesn't exist no longer returns 404
    // — that status is now reserved for the malformed-UUID case alone, and
    // a kill of an already-absent agent succeeds with a tagged body so
    // retried/replayed deletes by clients (network blip, dashboard double
    // click) don't surface a phantom error.
    let server = start_test_server().await;
    let client = reqwest::Client::new();

    let fake_id = uuid::Uuid::new_v4();
    // Refs #4614: confirm required, but the idempotent-already-gone
    // shortcut still applies and yields 200 OK.
    let resp = client
        .delete(format!(
            "{}/api/agents/{}?confirm=true",
            server.base_url, fake_id
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["status"], "already-deleted");
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

    // Kill one (refs #4614: confirm required)
    let resp = client
        .delete(format!(
            "{}/api/agents/{}?confirm=true",
            server.base_url, ids[1]
        ))
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

    // Kill the rest (refs #4614: confirm required)
    for id in [&ids[0], &ids[2]] {
        client
            .delete(format!(
                "{}/api/agents/{}?confirm=true",
                server.base_url, id
            ))
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
    // Audit: agent-list-limit-none-unbounded. `limit` is now always a
    // finite server-applied cap (DEFAULT_AGENT_LIST_LIMIT = 500), never
    // null, so an unspecified `limit` can no longer return an
    // unpaginated collection.
    assert_eq!(
        body["limit"], 500,
        "limit should report the server-applied default cap (500) when not specified"
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
    // limit in response should be clamped to MAX_AGENT_LIST_LIMIT = 500
    // (the cap was bumped from 100 to 500 by a subsequent change to
    // routes/agents/mod.rs (MAX_AGENT_LIST_LIMIT); updating the assertion to track the code).
    assert_eq!(body["limit"].as_u64().unwrap(), 500);
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
    let api_key_owned = api_key.to_string();
    let test = TestAppState::with_builder(MockKernelBuilder::new().with_config(move |cfg| {
        cfg.api_key = api_key_owned;
    }))
    .with_api_key(api_key);
    let config_path = test.tmp_path().join("config.toml");
    let test = test.with_config_path(config_path.clone());
    let (state, _tmp, _) = test.into_parts();
    state.kernel.clone().set_self_handle();

    let api_key_state = middleware::AuthState {
        api_key_lock: state.api_key_lock.clone(),
        master_key: state.master_key.clone(),
        active_sessions: state.active_sessions.clone(),
        dashboard_auth_enabled: false,
        user_api_keys: state.user_api_keys.clone(),
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
        _tmp,
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

// These tests run against the **full router** (`start_full_router`) so the
// `/api/tools` reads flow through the production middleware stack: auth,
// rate-limit, body-size, and request-logging. The old `start_test_server`
// mock harness mounted no auth layer at all on `/api/tools`, so the
// authentication boundary on these endpoints went completely untested
// (first slice of the mock-router -> full-router migration tracked in
// docs/issues/integration-tests-mock-router.md). `/api/tools` and
// `/api/tools/{name}` are NOT in any `PUBLIC_ROUTES_*` allowlist
// (middleware.rs), so once an api_key is configured an unauthenticated read
// must be rejected — `test_list_tools_requires_auth` pins that contract,
// which the mock harness silently allowed through.
const TOOLS_API_KEY: &str = "tools-cluster-test-key";

#[tokio::test(flavor = "multi_thread")]
async fn test_list_tools() {
    let harness = start_full_router(TOOLS_API_KEY).await;

    let response = harness
        .app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/tools")
                .header("authorization", format!("Bearer {TOOLS_API_KEY}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert!(body["tools"].is_array());
    assert!(body["total"].as_u64().unwrap() > 0);
}

/// Coverage the mock harness could not provide: `/api/tools` is not in any
/// public allowlist, so with an api_key configured an unauthenticated read
/// must be rejected by the auth middleware (401), not served.
#[tokio::test(flavor = "multi_thread")]
async fn test_list_tools_requires_auth() {
    let harness = start_full_router(TOOLS_API_KEY).await;

    let response = harness
        .app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/tools")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    let wrong_token = harness
        .app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/tools")
                .header("authorization", "Bearer wrong-key")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(wrong_token.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_get_tool_found() {
    let harness = start_full_router(TOOLS_API_KEY).await;

    // First list tools to get a known tool name.
    let list_response = harness
        .app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/tools")
                .header("authorization", format!("Bearer {TOOLS_API_KEY}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(list_response.status(), StatusCode::OK);
    let list_bytes = axum::body::to_bytes(list_response.into_body(), usize::MAX)
        .await
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(&list_bytes).unwrap();
    let first_tool_name = body["tools"][0]["name"].as_str().unwrap().to_string();

    // Now fetch that specific tool.
    let tool_response = harness
        .app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/tools/{first_tool_name}"))
                .header("authorization", format!("Bearer {TOOLS_API_KEY}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(tool_response.status(), StatusCode::OK);
    let tool_bytes = axum::body::to_bytes(tool_response.into_body(), usize::MAX)
        .await
        .unwrap();
    let tool: serde_json::Value = serde_json::from_slice(&tool_bytes).unwrap();
    assert_eq!(tool["name"].as_str().unwrap(), first_tool_name);
    assert!(tool["description"].is_string());
    assert!(tool["input_schema"].is_object());
}

#[tokio::test(flavor = "multi_thread")]
async fn test_get_tool_not_found() {
    let harness = start_full_router(TOOLS_API_KEY).await;

    let response = harness
        .app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/tools/nonexistent_tool_xyz")
                .header("authorization", format!("Bearer {TOOLS_API_KEY}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert!(body["error"]["message"]
        .as_str()
        .unwrap()
        .contains("not found"));
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
    let instances = json["items"].as_array().expect("items array");
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

/// #6117: a `channel_send` call routed through the `/mcp` bridge must honour the cross-chat guard using the peer scope forwarded on the bridge headers (`X-LibreFang-Current-Peer-Jid` / `-Channel` / `-Chat-Id`).
/// Before the fix `mcp_http` hardcoded sender_id/channel/chat_id to `None`, so the guard had no peer to validate against and a mis-targeted recipient leaked.
const MCP_CHANNEL_MANIFEST: &str = r#"
name = "mcp-channel-agent"
version = "0.1.0"
description = "Integration test agent for /mcp channel_send guard"
author = "test"
module = "builtin:chat"

[model]
provider = "ollama"
model = "test-model"
system_prompt = "You are a test agent."

[capabilities]
tools = ["channel_send"]
"#;

async fn call_mcp_channel_send(
    server: &TestServer,
    agent_id: &str,
    recipient: &str,
    peer_headers: Option<(&str, &str)>,
) -> serde_json::Value {
    let client = reqwest::Client::new();
    let mut req = client
        .post(format!("{}/mcp", server.base_url))
        .header("X-LibreFang-Agent-Id", agent_id)
        .json(&serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": {
                "name": "channel_send",
                "arguments": {"channel": "whatsapp", "recipient": recipient, "message": "hi"},
            },
        }));
    if let Some((peer, channel)) = peer_headers {
        req = req
            .header("X-LibreFang-Current-Peer-Jid", peer)
            .header("X-LibreFang-Current-Channel", channel);
    }
    let resp = req.send().await.expect("mcp request send");
    assert_eq!(resp.status(), 200);
    resp.json().await.expect("mcp body parse")
}

/// #6443 bridge parity: `channel_send` with an explicit `account_id`, routed through `/mcp` with the turn's account scope forwarded on `X-LibreFang-Current-Account-Id` (plus the #6117 peer scope so the recipient guard context matches the in-process path).
async fn call_mcp_channel_send_with_account(
    server: &TestServer,
    agent_id: &str,
    explicit_account: &str,
    account_header: Option<&str>,
) -> serde_json::Value {
    let client = reqwest::Client::new();
    let mut req = client
        .post(format!("{}/mcp", server.base_url))
        .header("X-LibreFang-Agent-Id", agent_id)
        .header("X-LibreFang-Current-Peer-Jid", "owner-jid")
        .header("X-LibreFang-Current-Channel", "whatsapp")
        .json(&serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": {
                "name": "channel_send",
                "arguments": {
                    "channel": "whatsapp",
                    "recipient": "owner-jid",
                    "message": "hi",
                    "account_id": explicit_account,
                },
            },
        }));
    if let Some(account) = account_header {
        req = req.header("X-LibreFang-Current-Account-Id", account);
    }
    let resp = req.send().await.expect("mcp request send");
    assert_eq!(resp.status(), 200);
    resp.json().await.expect("mcp body parse")
}

#[tokio::test(flavor = "multi_thread")]
async fn test_mcp_http_channel_send_cross_chat_guard_uses_peer_headers() {
    let server = start_test_server().await;
    let client = reqwest::Client::new();
    let spawn = client
        .post(format!("{}/api/agents", server.base_url))
        .json(&serde_json::json!({"manifest_toml": MCP_CHANNEL_MANIFEST}))
        .send()
        .await
        .unwrap();
    assert_eq!(spawn.status(), 201);
    let agent_id = spawn.json::<serde_json::Value>().await.unwrap()["agent_id"]
        .as_str()
        .unwrap()
        .to_string();

    // Inbound turn from `owner-jid` on `whatsapp`; the model targets a stranger
    // on the SAME channel → the guard must reject it before any send.
    let body = call_mcp_channel_send(
        &server,
        &agent_id,
        "stranger-jid",
        Some(("owner-jid", "whatsapp")),
    )
    .await;
    let content = body["result"]["content"][0]["text"].as_str().unwrap_or("");
    assert!(
        body["result"]["isError"].as_bool().unwrap_or(false),
        "cross-chat send must be an error: {content}"
    );
    assert!(
        content.contains("Cross-chat dispatch is forbidden"),
        "expected the cross-chat guard message, got: {content}"
    );

    // Same mis-targeted send WITHOUT peer headers (e.g. an external MCP client):
    // the guard no-ops, so the failure — if any — must NOT be the guard's.
    let body = call_mcp_channel_send(&server, &agent_id, "stranger-jid", None).await;
    let content = body["result"]["content"][0]["text"].as_str().unwrap_or("");
    assert!(
        !content.contains("Cross-chat dispatch is forbidden"),
        "without peer headers the guard must not fire: {content}"
    );
}

/// #6443: a `channel_send` with an explicit `account_id` routed through the `/mcp` bridge must honour the cross-account guard using the account scope forwarded on `X-LibreFang-Current-Account-Id`.
/// Before the parity fix `mcp_http` never read the header (the driver never sent it), so a subprocess-driver agent could dispatch through another tenant's bot account even though the in-process path (#6449) already rejected it.
#[tokio::test(flavor = "multi_thread")]
async fn test_mcp_http_channel_send_cross_account_guard_uses_account_header() {
    let server = start_test_server().await;
    let client = reqwest::Client::new();
    let spawn = client
        .post(format!("{}/api/agents", server.base_url))
        .json(&serde_json::json!({"manifest_toml": MCP_CHANNEL_MANIFEST}))
        .send()
        .await
        .unwrap();
    assert_eq!(spawn.status(), 201);
    let agent_id = spawn.json::<serde_json::Value>().await.unwrap()["agent_id"]
        .as_str()
        .unwrap()
        .to_string();

    // Turn arrived on `tenant-a`; the model explicitly targets `tenant-b` on
    // the SAME channel → the guard must reject it before any send.
    let body =
        call_mcp_channel_send_with_account(&server, &agent_id, "tenant-b", Some("tenant-a")).await;
    let content = body["result"]["content"][0]["text"].as_str().unwrap_or("");
    assert!(
        body["result"]["isError"].as_bool().unwrap_or(false),
        "cross-account send must be an error: {content}"
    );
    assert!(
        content.contains("Cross-account (cross-tenant) dispatch is forbidden"),
        "expected the cross-account guard message, got: {content}"
    );

    // Matching account → the guard passes; the failure — if any — must NOT be
    // the guard's (the test kernel has no real whatsapp adapter registered).
    let body =
        call_mcp_channel_send_with_account(&server, &agent_id, "tenant-a", Some("tenant-a")).await;
    let content = body["result"]["content"][0]["text"].as_str().unwrap_or("");
    assert!(
        !content.contains("Cross-account (cross-tenant) dispatch is forbidden"),
        "matching account must not trip the guard: {content}"
    );

    // No account header (external MCP client / out-of-band turn): the guard
    // no-ops even for an explicit foreign account, preserving pre-parity
    // behaviour for clients that never carried account scope.
    let body = call_mcp_channel_send_with_account(&server, &agent_id, "tenant-b", None).await;
    let content = body["result"]["content"][0]["text"].as_str().unwrap_or("");
    assert!(
        !content.contains("Cross-account (cross-tenant) dispatch is forbidden"),
        "without the account header the guard must not fire: {content}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_mcp_http_invalid_agent_header_is_rejected() {
    // A client-supplied agent identity must never degrade to the header-less path.
    // Otherwise a forged or stale identity could silently bypass the agent-scoped authorization contract.
    let server = start_test_server().await;

    let (status, body) = call_mcp_cron_list(&server, Some("not-a-uuid")).await;
    assert_eq!(status, 200);
    assert_eq!(body["error"]["code"], -32001, "{body}");
    assert!(body.get("result").is_none(), "{body}");

    // A well-formed but unknown UUID must fail closed too.
    let (status, body) =
        call_mcp_cron_list(&server, Some("00000000-0000-0000-0000-000000000000")).await;
    assert_eq!(status, 200);
    assert_eq!(body["error"]["code"], -32001, "{body}");
    assert!(body.get("result").is_none(), "{body}");
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
    use librefang_kernel::llm_driver::StreamEvent;
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

#[tokio::test(flavor = "multi_thread")]
async fn test_attach_session_stream_accepts_websocket_upgrade() {
    use futures::StreamExt as _;
    use librefang_kernel::llm_driver::{StreamEvent, PHASE_RESPONSE_COMPLETE};
    use librefang_types::message::{StopReason, TokenUsage};
    use std::time::Duration;
    use tokio_tungstenite::tungstenite::{client::IntoClientRequest, Message};

    let server = start_test_server().await;
    let client = reqwest::Client::new();
    let response = client
        .post(format!("{}/api/agents", server.base_url))
        .json(&serde_json::json!({ "manifest_toml": TEST_MANIFEST }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::CREATED);
    let body: serde_json::Value = response.json().await.unwrap();
    let agent_id_text = body["agent_id"].as_str().unwrap();
    let agent_id = agent_id_text.parse().unwrap();
    let session_id = server
        .state
        .kernel
        .agent_registry()
        .get(agent_id)
        .unwrap()
        .session_id;
    let url = format!(
        "{}/api/agents/{agent_id_text}/sessions/{session_id}/stream",
        server.base_url.replacen("http://", "ws://", 1)
    );

    let mut request = url.into_client_request().unwrap();
    request.headers_mut().insert(
        "Sec-WebSocket-Protocol",
        "bearer.test-token".parse().unwrap(),
    );
    let (mut socket, response) = tokio_tungstenite::connect_async(request).await.unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::SWITCHING_PROTOCOLS);
    assert_eq!(
        response
            .headers()
            .get("Sec-WebSocket-Protocol")
            .and_then(|value| value.to_str().ok()),
        Some("bearer.test-token")
    );

    let sender = server.state.kernel.session_stream_hub().sender(session_id);
    sender
        .send(StreamEvent::TextDelta {
            text: "hello-websocket-attach".to_string(),
        })
        .expect("WebSocket subscriber should be attached");

    let message = tokio::time::timeout(Duration::from_secs(2), socket.next())
        .await
        .expect("WebSocket event should arrive")
        .expect("WebSocket should remain open")
        .expect("WebSocket frame should be valid");
    let Message::Text(text) = message else {
        panic!("expected a text frame, got {message:?}");
    };
    let envelope: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(envelope["type"], "chunk");
    assert_eq!(envelope["content"], "hello-websocket-attach");
    assert_eq!(envelope["done"], false);

    sender
        .send(StreamEvent::ContentComplete {
            stop_reason: StopReason::ToolUse,
            usage: TokenUsage {
                input_tokens: 3,
                output_tokens: 2,
                ..TokenUsage::default()
            },
        })
        .expect("intermediate completion should reach the subscriber");
    sender
        .send(StreamEvent::TextDelta {
            text: "after-tool".to_string(),
        })
        .expect("stream should remain attached after an intermediate completion");

    let message = tokio::time::timeout(Duration::from_secs(2), socket.next())
        .await
        .expect("post-tool event should arrive")
        .expect("WebSocket should remain open")
        .expect("WebSocket frame should be valid");
    let Message::Text(text) = message else {
        panic!("expected a text frame, got {message:?}");
    };
    let envelope: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(envelope["type"], "chunk");
    assert_eq!(envelope["content"], "after-tool");

    sender
        .send(StreamEvent::ContentComplete {
            stop_reason: StopReason::EndTurn,
            usage: TokenUsage {
                input_tokens: 4,
                output_tokens: 6,
                ..TokenUsage::default()
            },
        })
        .expect("final completion usage should reach the subscriber");
    sender
        .send(StreamEvent::PhaseChange {
            phase: PHASE_RESPONSE_COMPLETE.to_string(),
            detail: None,
        })
        .expect("response completion should reach the subscriber");
    let message = tokio::time::timeout(Duration::from_secs(2), socket.next())
        .await
        .expect("terminal event should arrive")
        .expect("WebSocket should deliver the terminal frame")
        .expect("WebSocket terminal frame should be valid");
    let Message::Text(text) = message else {
        panic!("expected a terminal text frame, got {message:?}");
    };
    let envelope: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(envelope["type"], "done");
    assert_eq!(envelope["done"], true);
    assert_eq!(envelope["usage"]["input_tokens"], 7);
    assert_eq!(envelope["usage"]["output_tokens"], 8);

    tokio::time::timeout(Duration::from_secs(2), async {
        while let Some(message) = socket.next().await {
            if matches!(message, Ok(Message::Close(_))) {
                break;
            }
        }
    })
    .await
    .expect("server should close the completed session stream");
    tokio::time::timeout(Duration::from_secs(2), async {
        while sender.receiver_count() != 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("client close should release the session stream subscription");
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

    librefang_kernel::registry_sync::seed_registry_fixture_for_tests(tmp.path());

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
            extra_params: std::collections::BTreeMap::new(),
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

    let api_key_owned = api_key.to_string();
    let test = TestAppState::with_builder(MockKernelBuilder::new().with_config(move |cfg| {
        cfg.api_key = api_key_owned;
        cfg.users = user_configs;
    }))
    .with_api_key(api_key)
    .with_user_api_keys(api_user_records);

    let config_path = test.tmp_path().join("config.toml");
    let test = test.with_config_path(config_path.clone());
    let (state, tmp, _) = test.into_parts();

    let api_key_state = middleware::AuthState {
        api_key_lock: state.api_key_lock.clone(),
        master_key: state.master_key.clone(),
        active_sessions: state.active_sessions.clone(),
        dashboard_auth_enabled: false,
        user_api_keys: state.user_api_keys.clone(),
        require_auth_for_reads: false,
        // Anonymous-rejection tests rely on this — we synthesize requests
        // without a Bearer header and need them to flow through to the
        // in-handler `require_admin` gate (where they get 403'd). Without
        // `allow_no_auth = true` the middleware would 401 first.
        allow_no_auth: true,
        audit_log: Some(state.kernel.audit().clone()),
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
        .nest("/api", routes::agents::router())
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
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .await
        .unwrap();
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
    assert!(body["items"].is_array(), "response must carry items[]");
    assert!(body["total"].is_number(), "response must carry total");
    assert!(body["offset"].is_number(), "response must carry offset");
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
        first_line, "seq,timestamp,agent_id,action,detail,outcome,user_id,channel,hash,prev_hash",
        "CSV header row schema must remain stable for downstream parsers; \
         `prev_hash` is the last column so a verifier can replay the chain"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_user_budget_detail_marks_missing_limits_unenforced() {
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
        serde_json::json!(false),
        "a user without configured limits must not be reported as enforced"
    );
    // Defensive: the spend numerics must also be present so the
    // dashboard can render even on an empty database.
    assert!(body["hourly"]["spend"].is_number());
    assert!(body["daily"]["spend"].is_number());
    assert!(body["monthly"]["spend"].is_number());
    assert!(body["alert_breach"].is_boolean());
}

#[tokio::test(flavor = "multi_thread")]
async fn test_user_budget_detail_returns_404_for_unknown_user() {
    let server =
        start_test_server_with_rbac_users("any-key", vec![("Alice", "admin", "alice-admin-key")])
            .await;
    let resp = reqwest::Client::new()
        .get(format!("{}/api/budget/users/Unknown", server.base_url))
        .header("authorization", "Bearer alice-admin-key")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_user_budget_detail_fails_closed_when_storage_is_unavailable() {
    // #6971 — `user_budget_detail` reads `query_user_hourly` /
    // `query_user_daily` / `query_user_monthly` and, before that fix,
    // `.unwrap_or(0.0)`'d a query failure into a silent zero-spend
    // response. Dropping the backing table forces every one of those
    // three queries to fail, so a regression here would report success
    // with fabricated zero spend numbers instead of failing closed.
    let server =
        start_test_server_with_rbac_users("any-key", vec![("Alice", "admin", "alice-admin-key")])
            .await;
    let conn = server.state.kernel.memory_substrate().pool().get().unwrap();
    conn.execute("DROP TABLE usage_events", []).unwrap();
    drop(conn);

    let client = reqwest::Client::new();
    let resp = client
        .get(format!("{}/api/budget/users/Alice", server.base_url))
        .header("authorization", "Bearer alice-admin-key")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 500);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["code"], "usage_query_failed");
    assert_eq!(body["message"], "Internal server error");
    assert!(!body.to_string().contains("usage_events"), "body: {body}");
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

    let api_key_owned = api_key.to_string();
    let test = TestAppState::with_builder(MockKernelBuilder::new().with_config(move |cfg| {
        cfg.api_key = api_key_owned;
        cfg.users = user_configs;
    }))
    .with_api_key(api_key)
    .with_user_api_keys(api_user_records);

    let config_path = test.tmp_path().join("config.toml");
    let test = test.with_config_path(config_path.clone());
    let (state, tmp, _) = test.into_parts();

    let api_key_state = middleware::AuthState {
        api_key_lock: state.api_key_lock.clone(),
        master_key: state.master_key.clone(),
        active_sessions: state.active_sessions.clone(),
        dashboard_auth_enabled: false,
        user_api_keys: state.user_api_keys.clone(),
        require_auth_for_reads: false,
        allow_no_auth: true,
        audit_log: Some(state.kernel.audit().clone()),
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

#[tokio::test(flavor = "multi_thread")]
async fn test_authz_check_returns_allow_for_permitted_tool() {
    let alice = UserConfig {
        name: "Alice".to_string(),
        role: "admin".to_string(),
        ..Default::default()
    };
    let bob = UserConfig {
        name: "Bob".to_string(),
        role: "user".to_string(),
        tool_policy: Some(UserToolPolicy {
            allowed_tools: vec!["web_search".to_string()],
            denied_tools: vec![],
        }),
        ..Default::default()
    };
    let server = start_test_server_with_full_user_configs(
        "any-key",
        vec![(alice, "alice-admin-key"), (bob, "bob-key")],
    )
    .await;
    let client = reqwest::Client::new();

    let body: serde_json::Value = client
        .get(format!(
            "{}/api/authz/check?user=Bob&action=web_search",
            server.base_url
        ))
        .header("authorization", "Bearer alice-admin-key")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(body["decision"], "allow");
    assert_eq!(body["allowed"], true);
    assert!(body["reason"].is_null());
    // `scope` advertises that this is a user-policy-only decision and
    // does NOT consult per-agent ToolPolicy or channel_rules. Operators
    // and any future dashboard consumer rely on this marker to display
    // the "runtime gate may differ" disclaimer.
    assert_eq!(
        body["scope"], "user_policy_only",
        "scope must mark the decision as user-policy-only — see authz.rs::check docstring"
    );
}

/// Regression for #3231 follow-up: a typical query must always carry
/// `scope: "user_policy_only"` so callers can render the disclaimer
/// that the runtime gate may still deny or require approval (per-agent
/// ToolPolicy + ApprovalPolicy.channel_rules are not consulted by this
/// endpoint).
#[tokio::test(flavor = "multi_thread")]
async fn test_authz_check_response_advertises_user_policy_only_scope() {
    let alice = UserConfig {
        name: "Alice".to_string(),
        role: "admin".to_string(),
        ..Default::default()
    };
    let bob = UserConfig {
        name: "Bob".to_string(),
        role: "user".to_string(),
        tool_policy: Some(UserToolPolicy {
            allowed_tools: vec!["web_search".to_string()],
            denied_tools: vec!["shell_exec".to_string()],
        }),
        ..Default::default()
    };
    let server = start_test_server_with_full_user_configs(
        "any-key",
        vec![(alice, "alice-admin-key"), (bob, "bob-key")],
    )
    .await;
    let client = reqwest::Client::new();

    // Allow case.
    let allow: serde_json::Value = client
        .get(format!(
            "{}/api/authz/check?user=Bob&action=web_search&channel=api",
            server.base_url
        ))
        .header("authorization", "Bearer alice-admin-key")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(allow["decision"], "allow");
    assert_eq!(
        allow["scope"], "user_policy_only",
        "allow path must carry scope marker"
    );

    // Deny case — scope marker must travel with every decision class.
    let deny: serde_json::Value = client
        .get(format!(
            "{}/api/authz/check?user=Bob&action=shell_exec",
            server.base_url
        ))
        .header("authorization", "Bearer alice-admin-key")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(deny["decision"], "deny");
    assert_eq!(
        deny["scope"], "user_policy_only",
        "deny path must carry scope marker"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_authz_check_returns_deny_for_blocked_tool() {
    let alice = UserConfig {
        name: "Alice".to_string(),
        role: "admin".to_string(),
        ..Default::default()
    };
    let bob = UserConfig {
        name: "Bob".to_string(),
        role: "user".to_string(),
        tool_policy: Some(UserToolPolicy {
            allowed_tools: vec![],
            denied_tools: vec!["shell_exec".to_string()],
        }),
        ..Default::default()
    };
    let server = start_test_server_with_full_user_configs(
        "any-key",
        vec![(alice, "alice-admin-key"), (bob, "bob-key")],
    )
    .await;
    let client = reqwest::Client::new();

    let body: serde_json::Value = client
        .get(format!(
            "{}/api/authz/check?user=Bob&action=shell_exec",
            server.base_url
        ))
        .header("authorization", "Bearer alice-admin-key")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(body["decision"], "deny");
    assert_eq!(body["allowed"], false);
    assert!(body["reason"].as_str().unwrap_or("").contains("shell_exec"));
    assert_eq!(body["scope"], "user_policy_only");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_authz_check_unknown_user_returns_404() {
    let alice = UserConfig {
        name: "Alice".to_string(),
        role: "admin".to_string(),
        ..Default::default()
    };
    let server =
        start_test_server_with_full_user_configs("any-key", vec![(alice, "alice-admin-key")]).await;
    let client = reqwest::Client::new();
    let resp = client
        .get(format!(
            "{}/api/authz/check?user=Nobody&action=web_search",
            server.base_url
        ))
        .header("authorization", "Bearer alice-admin-key")
        .send()
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        404,
        "unknown user must surface as 404 — silent guest fallback would mask config gaps"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_authz_check_viewer_caller_rejected_403() {
    let alice = UserConfig {
        name: "Alice".to_string(),
        role: "admin".to_string(),
        ..Default::default()
    };
    let viewer = UserConfig {
        name: "Vince".to_string(),
        role: "viewer".to_string(),
        ..Default::default()
    };
    let server = start_test_server_with_full_user_configs(
        "any-key",
        vec![(alice, "alice-admin-key"), (viewer, "vince-key")],
    )
    .await;
    let client = reqwest::Client::new();
    let resp = client
        .get(format!(
            "{}/api/authz/check?user=Alice&action=web_search",
            server.base_url
        ))
        .header("authorization", "Bearer vince-key")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 403, "Viewer must not query authz/check");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_authz_check_rejects_anonymous() {
    let alice = UserConfig {
        name: "Alice".to_string(),
        role: "admin".to_string(),
        ..Default::default()
    };
    let server =
        start_test_server_with_full_user_configs("any-key", vec![(alice, "alice-admin-key")]).await;
    let client = reqwest::Client::new();
    let resp = client
        .get(format!(
            "{}/api/authz/check?user=Alice&action=web_search",
            server.base_url
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        401,
        "anonymous /api/authz/check must be 401'd at the middleware (same as other admin endpoints)"
    );
}
/// Round-trip: PUT a budget, GET reflects the new limits, DELETE clears
/// it back to "no cap" (limit = 0 in the response).
#[tokio::test(flavor = "multi_thread")]
async fn test_user_budget_put_get_delete_round_trip() {
    let server =
        start_test_server_with_rbac_users("any-key", vec![("Alice", "admin", "alice-admin-key")])
            .await;
    let client = reqwest::Client::new();
    let url = format!("{}/api/budget/users/Alice", server.base_url);

    // Initial GET — no cap configured, all limits are 0.
    let initial: serde_json::Value = client
        .get(&url)
        .header("authorization", "Bearer alice-admin-key")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(initial["hourly"]["limit"], serde_json::json!(0.0));

    // PUT a real cap.
    let put_resp = client
        .put(&url)
        .header("authorization", "Bearer alice-admin-key")
        .json(&serde_json::json!({
            "max_hourly_usd": 1.5,
            "max_daily_usd": 12.0,
            "max_monthly_usd": 100.0,
            "alert_threshold": 0.75,
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(put_resp.status(), 200, "PUT should accept the upsert");

    // Issue #3832: the success body is the canonical UserBudgetConfig — no
    // `{"status":"ok","budget":...}` ack envelope. Dashboard mutations rely on
    // this to `setQueryData` without a follow-up GET.
    let put_body: serde_json::Value = put_resp.json().await.unwrap();
    assert!(
        put_body.get("status").is_none(),
        "PUT body must not carry the legacy ack envelope: {put_body:?}"
    );
    assert!(
        put_body.get("budget").is_none(),
        "PUT body must be the bare UserBudgetConfig, not nested under `budget`: {put_body:?}"
    );
    assert_eq!(put_body["max_hourly_usd"], serde_json::json!(1.5));
    assert_eq!(put_body["max_daily_usd"], serde_json::json!(12.0));
    assert_eq!(put_body["max_monthly_usd"], serde_json::json!(100.0));
    assert_eq!(put_body["alert_threshold"], serde_json::json!(0.75));

    // GET should reflect the new caps.
    let after_put: serde_json::Value = client
        .get(&url)
        .header("authorization", "Bearer alice-admin-key")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(after_put["hourly"]["limit"], serde_json::json!(1.5));
    assert_eq!(after_put["daily"]["limit"], serde_json::json!(12.0));
    assert_eq!(after_put["monthly"]["limit"], serde_json::json!(100.0));
    assert_eq!(after_put["alert_threshold"], serde_json::json!(0.75));

    // DELETE clears.
    let del_resp = client
        .delete(&url)
        .header("authorization", "Bearer alice-admin-key")
        .send()
        .await
        .unwrap();
    assert_eq!(del_resp.status(), 204, "DELETE should clear the cap");

    // GET again — back to limit = 0.
    let after_delete: serde_json::Value = client
        .get(&url)
        .header("authorization", "Bearer alice-admin-key")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(
        after_delete["hourly"]["limit"],
        serde_json::json!(0.0),
        "DELETE must reset limit back to 0 (no cap)"
    );
}

/// Pin the new audit-detail diff format introduced by review item #21
/// follow-up: every PUT to `/api/budget/users/{user}` must emit a
/// ConfigChange row whose `detail` carries `old → new` for every field,
/// and whose `user_id` is the calling admin (not the target user, not
/// the literal `system`). Without this an audit reader has to correlate
/// multiple rows to reconstruct what was rotated; a single row should
/// be self-describing.
#[tokio::test(flavor = "multi_thread")]
async fn test_user_budget_put_audit_records_old_new_diff_and_caller() {
    let server =
        start_test_server_with_rbac_users("any-key", vec![("Alice", "admin", "alice-admin-key")])
            .await;
    let client = reqwest::Client::new();
    let url = format!("{}/api/budget/users/Alice", server.base_url);

    // First PUT — old budget is `none` (no prior cap).
    let _ = client
        .put(&url)
        .header("authorization", "Bearer alice-admin-key")
        .json(&serde_json::json!({
            "max_hourly_usd": 1.0,
            "max_daily_usd": 10.0,
            "max_monthly_usd": 100.0,
            "alert_threshold": 0.8,
        }))
        .send()
        .await
        .unwrap();

    // Second PUT — bumps hourly 1.0 → 5.0 so the diff is unambiguous.
    let _ = client
        .put(&url)
        .header("authorization", "Bearer alice-admin-key")
        .json(&serde_json::json!({
            "max_hourly_usd": 5.0,
            "max_daily_usd": 10.0,
            "max_monthly_usd": 100.0,
            "alert_threshold": 0.8,
        }))
        .send()
        .await
        .unwrap();

    // Pull recent audit rows, filter to ConfigChange.
    let q: serde_json::Value = client
        .get(format!(
            "{}/api/audit/query?action=ConfigChange",
            server.base_url
        ))
        .header("authorization", "Bearer alice-admin-key")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let entries = q["items"].as_array().expect("items[] present");
    assert!(
        !entries.is_empty(),
        "ConfigChange audit row must be emitted by /api/budget/users/{{user}} PUT"
    );

    // Newest first: the second PUT's row carries the 1.0→5.0 transition.
    let bump = entries
        .iter()
        .find(|e| {
            e["detail"]
                .as_str()
                .map(|s| s.contains("hourly: 1→5"))
                .unwrap_or(false)
        })
        .expect("audit detail must record old→new diff (e.g. hourly: 1→5)");
    let detail = bump["detail"].as_str().unwrap();
    assert!(
        detail.starts_with("user_budget updated for "),
        "detail prefix pins forensic search: {detail}"
    );
    assert!(detail.contains("→"), "diff arrow must be present: {detail}");

    // Caller attribution: user_id must be the admin (Alice), not the
    // target user (also Alice in this test — but distinguishable from
    // `null` which would mean anonymous).
    let alice = librefang_types::agent::UserId::from_name("Alice").to_string();
    assert_eq!(
        bump["user_id"].as_str().unwrap_or(""),
        alice,
        "audit row must attribute the action to the calling admin"
    );
    assert_eq!(
        bump["agent_id"].as_str().unwrap_or(""),
        "system",
        "config-mutation rows are not agent-scoped — agent_id stays 'system'"
    );

    // The first PUT's row had old=none → new=1: walk the array for it.
    let first = entries
        .iter()
        .find(|e| {
            e["detail"]
                .as_str()
                .map(|s| s.contains("hourly: none→1"))
                .unwrap_or(false)
        })
        .expect("first PUT must render old=none → new=1");
    assert!(first["detail"]
        .as_str()
        .unwrap()
        .contains("alert: none→0.8"));
}

/// Validation: PUT with negative or out-of-range values is rejected
/// before touching disk. Each case is a full-shape payload with exactly
/// one offending field — proves the per-field validators fire, not just
/// the "missing key" gate.
#[tokio::test(flavor = "multi_thread")]
async fn test_user_budget_put_rejects_invalid_payload() {
    let server =
        start_test_server_with_rbac_users("any-key", vec![("Alice", "admin", "alice-admin-key")])
            .await;
    let client = reqwest::Client::new();
    let url = format!("{}/api/budget/users/Alice", server.base_url);

    let base = serde_json::json!({
        "max_hourly_usd": 1.0,
        "max_daily_usd": 10.0,
        "max_monthly_usd": 100.0,
        "alert_threshold": 0.8,
    });
    let mut cases = Vec::new();
    for (field, value) in [
        ("max_hourly_usd", serde_json::json!(-1.0)),
        ("alert_threshold", serde_json::json!(1.5)),
        ("alert_threshold", serde_json::json!(-0.1)),
    ] {
        let mut body = base.clone();
        body[field] = value;
        cases.push(body);
    }

    for bad in cases {
        let resp = client
            .put(&url)
            .header("authorization", "Bearer alice-admin-key")
            .json(&bad)
            .send()
            .await
            .unwrap();
        assert_eq!(
            resp.status(),
            400,
            "expected 400 for invalid payload {bad:?}"
        );
    }
}

/// Regression: a partial body must NOT be accepted as an upsert. Without
/// this guard, `UserBudgetConfig`'s `#[serde(default)]` would silently
/// fill missing fields with `0.0` / `0.8` and clear an existing cap on
/// the windows the caller didn't mention.
#[tokio::test(flavor = "multi_thread")]
async fn test_user_budget_put_rejects_partial_payload() {
    let server =
        start_test_server_with_rbac_users("any-key", vec![("Alice", "admin", "alice-admin-key")])
            .await;
    let client = reqwest::Client::new();
    let url = format!("{}/api/budget/users/Alice", server.base_url);

    // Seed a real cap so we can confirm it stays put when a partial PUT
    // gets rejected.
    let put_full = client
        .put(&url)
        .header("authorization", "Bearer alice-admin-key")
        .json(&serde_json::json!({
            "max_hourly_usd": 2.0,
            "max_daily_usd": 20.0,
            "max_monthly_usd": 200.0,
            "alert_threshold": 0.6,
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(put_full.status(), 200);

    // Each partial body omits at least one required key and must be
    // rejected with 400.
    for partial in [
        serde_json::json!({"max_hourly_usd": 5.0}),
        serde_json::json!({"max_daily_usd": 50.0, "max_monthly_usd": 500.0}),
        serde_json::json!({}),
        // Wrong type should also 400, not be coerced to 0.
        serde_json::json!({
            "max_hourly_usd": "1.0",
            "max_daily_usd": 10.0,
            "max_monthly_usd": 100.0,
            "alert_threshold": 0.8,
        }),
    ] {
        let resp = client
            .put(&url)
            .header("authorization", "Bearer alice-admin-key")
            .json(&partial)
            .send()
            .await
            .unwrap();
        assert_eq!(
            resp.status(),
            400,
            "expected 400 for partial / wrong-typed payload {partial:?}"
        );
    }

    // Original cap survived all rejected partials.
    let after: serde_json::Value = client
        .get(&url)
        .header("authorization", "Bearer alice-admin-key")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(after["hourly"]["limit"], serde_json::json!(2.0));
    assert_eq!(after["daily"]["limit"], serde_json::json!(20.0));
    assert_eq!(after["monthly"]["limit"], serde_json::json!(200.0));
    assert_eq!(after["alert_threshold"], serde_json::json!(0.6));
}

/// Authz: a non-admin caller is rejected even when the URL is well-formed.
/// The body is intentionally partial — the admin gate must fire before
/// body validation, so a viewer's request never reaches the parser.
#[tokio::test(flavor = "multi_thread")]
async fn test_user_budget_put_rejects_viewer_with_403() {
    let server = start_test_server_with_rbac_users(
        "any-key",
        vec![
            ("Alice", "admin", "alice-admin-key"),
            ("Bob", "viewer", "bob-viewer-key"),
        ],
    )
    .await;
    let client = reqwest::Client::new();
    let resp = client
        .put(format!("{}/api/budget/users/Alice", server.base_url))
        .header("authorization", "Bearer bob-viewer-key")
        .json(&serde_json::json!({"max_hourly_usd": 1.0}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 403, "Viewer must not write user budget");
}

/// 404 path: PUT against an unknown user surfaces a real error (not a
/// silent insert). Requires a full-shape body so the request reaches the
/// persist step (a partial body would be 400'd earlier).
#[tokio::test(flavor = "multi_thread")]
async fn test_user_budget_put_unknown_user_returns_404() {
    let server =
        start_test_server_with_rbac_users("any-key", vec![("Alice", "admin", "alice-admin-key")])
            .await;
    let client = reqwest::Client::new();
    let resp = client
        .put(format!("{}/api/budget/users/NonExistent", server.base_url))
        .header("authorization", "Bearer alice-admin-key")
        .json(&serde_json::json!({
            "max_hourly_usd": 1.0,
            "max_daily_usd": 10.0,
            "max_monthly_usd": 100.0,
            "alert_threshold": 0.8,
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 404);
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

// ---------------------------------------------------------------------------
// API-key rotation owner-only gate (RBAC follow-up to #3054 / M3 / M6)
// ---------------------------------------------------------------------------

/// Pins the owner-only gate on `POST /api/users/{name}/rotate-key`.
/// Without `is_owner_only_write` covering the rotation path, an Admin
/// per-user API key could rotate any other user's key — including the
/// Owner's — and lock everyone else out. This is the same blast radius
/// as user create/delete, which is why all of `/api/users/*` non-GET goes
/// through the `Owner` gate at `middleware.rs:109`.
#[tokio::test(flavor = "multi_thread")]
async fn users_rotate_key_admin_returns_403() {
    let alice = UserConfig {
        name: "Alice".to_string(),
        role: "admin".to_string(),
        channel_bindings: std::collections::HashMap::new(),
        api_key_hash: None, // populated by helper
        ..Default::default()
    };
    let bob = UserConfig {
        name: "Bob".to_string(),
        role: "user".to_string(),
        channel_bindings: std::collections::HashMap::new(),
        api_key_hash: None,
        ..Default::default()
    };
    let server = start_test_server_with_full_user_configs(
        "any-key",
        vec![(alice, "alice-admin-key"), (bob, "bob-user-key")],
    )
    .await;
    let client = reqwest::Client::new();

    // Admin attempting to rotate Bob's key — must be rejected by the
    // middleware's owner-only gate before the handler is even invoked.
    let resp = client
        .post(format!("{}/api/users/Bob/rotate-key", server.base_url))
        .header("authorization", "Bearer alice-admin-key")
        .send()
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        403,
        "Admin must NOT be able to rotate another user's key — only Owner. \
         If this returns 200, an admin can self-promote by rotating the owner's key."
    );

    // Bob attempting to self-rotate is also rejected — same gate. The
    // self-service "rotate my own key" workflow is intentionally NOT
    // supported through this endpoint; users should ask an Owner. This
    // matches the kernel's `Action::ManageUsers` posture.
    let resp = client
        .post(format!("{}/api/users/Bob/rotate-key", server.base_url))
        .header("authorization", "Bearer bob-user-key")
        .send()
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        403,
        "Non-Owner users (incl. self-rotate) must be rejected"
    );
}

/// Oversize uploads must be rejected at the wire by the route-local
/// `RequestBodyLimitLayer` (sized to `max_upload_size_bytes`) wrapping the
/// upload sub-router in `server.rs::build_router`, NOT only by the handler's
/// after-the-fact `body.len()` check. A body one byte over the configured
/// cap must yield `413 PAYLOAD_TOO_LARGE` before the handler runs.
#[tokio::test(flavor = "multi_thread")]
async fn test_upload_over_limit_returns_413() {
    let harness = start_full_router("").await;

    // `start_full_router` boots with `KernelConfig::default()`, so the upload
    // cap is the compiled default (`default_max_upload_size_bytes` = 10 MiB).
    let limit = harness.state.kernel.config_ref().max_upload_size_bytes;
    let oversize = vec![0u8; limit + 1];

    let mut request = Request::builder()
        .method("POST")
        .uri("/api/agents/any-agent/upload")
        .header("content-type", "text/plain")
        .header("x-filename", "huge.txt")
        // Real HTTP clients (reqwest, browsers) always send Content-Length for a
        // known-size body. With it set, RequestBodyLimitLayer short-circuits to a
        // clean 413 before reading the body. (Absent it, the layer instead wraps
        // the body in a `Limited` stream that errors mid-read, which axum's Bytes
        // extractor surfaces as 400 — not the wire-level cap we are asserting.)
        .header("content-length", oversize.len().to_string())
        .body(Body::from(oversize))
        .unwrap();
    // oneshot bypasses axum's connection layer, so inject a loopback
    // ConnectInfo to clear the keyless-auth fail-closed branch (same pattern
    // as test_run_migrate_uses_daemon_home_when_target_dir_is_empty).
    request
        .extensions_mut()
        .insert(axum::extract::ConnectInfo(std::net::SocketAddr::from((
            [127, 0, 0, 1],
            0,
        ))));

    let response = harness.app.clone().oneshot(request).await.unwrap();

    assert_eq!(
        response.status(),
        StatusCode::PAYLOAD_TOO_LARGE,
        "upload one byte over max_upload_size_bytes must be rejected with 413 \
         by the route-local RequestBodyLimitLayer"
    );
}

/// #6530 round-trip: a file uploaded via `POST /upload` is persisted on disk as
/// `<uuid>.<ext>`, but the client-facing `file_id` stays a bare UUID. Serving
/// it back by that bare `file_id` must still find the file — this proves the
/// write-side `on_disk_name` matches the read-side reconstruction in
/// `serve_upload`. Covers a typed content type (PNG → `.png`) and the generic
/// `application/octet-stream` (no extension → bare `<uuid>`) so both arms of
/// `on_disk_name` are exercised end to end.
#[tokio::test(flavor = "multi_thread")]
async fn test_upload_roundtrip_serves_by_bare_file_id() {
    let harness = start_full_router("").await;

    async fn upload_then_serve(
        harness: &FullRouterHarness,
        content_type: &str,
        filename: &str,
        body: Vec<u8>,
    ) {
        // The upload route validates the agent-id *format* (parses as a UUID);
        // it does not require the agent to exist, so a fresh UUID is enough.
        let agent_id = uuid::Uuid::new_v4();
        let mut up = Request::builder()
            .method("POST")
            .uri(format!("/api/agents/{agent_id}/upload"))
            .header("content-type", content_type)
            .header("x-filename", filename)
            .header("content-length", body.len().to_string())
            .body(Body::from(body.clone()))
            .unwrap();
        up.extensions_mut()
            .insert(axum::extract::ConnectInfo(std::net::SocketAddr::from((
                [127, 0, 0, 1],
                0,
            ))));
        let up_resp = harness.app.clone().oneshot(up).await.unwrap();
        let up_status = up_resp.status();
        let bytes = axum::body::to_bytes(up_resp.into_body(), usize::MAX)
            .await
            .unwrap();
        assert_eq!(
            up_status,
            StatusCode::CREATED,
            "upload of {content_type} must succeed, got {up_status}: {}",
            String::from_utf8_lossy(&bytes)
        );
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        let file_id = json["file_id"].as_str().expect("file_id in response");
        // The client-facing id is a bare UUID (the traversal/owner guards parse it).
        assert!(
            uuid::Uuid::parse_str(file_id).is_ok(),
            "file_id must stay a bare UUID, got {file_id}"
        );
        let upload_dir = harness
            .state
            .kernel
            .config_ref()
            .channels
            .effective_file_download_dir();
        let persisted_meta: serde_json::Value = serde_json::from_slice(
            &std::fs::read(upload_dir.join(format!(".{file_id}.meta.json"))).unwrap(),
        )
        .unwrap();
        assert_eq!(persisted_meta["filename"], filename);
        assert_eq!(persisted_meta["content_type"], content_type);
        assert!(
            persisted_meta["uploaded_by"].is_string(),
            "full-router uploads must persist the injected local owner"
        );

        let mut serve = Request::builder()
            .method("GET")
            .uri(format!("/api/uploads/{file_id}"))
            .body(Body::empty())
            .unwrap();
        serve
            .extensions_mut()
            .insert(axum::extract::ConnectInfo(std::net::SocketAddr::from((
                [127, 0, 0, 1],
                0,
            ))));
        let serve_resp = harness.app.clone().oneshot(serve).await.unwrap();
        assert_eq!(
            serve_resp.status(),
            StatusCode::OK,
            "serve_upload must find the {content_type} file by its bare id"
        );
        let served = axum::body::to_bytes(serve_resp.into_body(), usize::MAX)
            .await
            .unwrap();
        assert_eq!(
            served.as_ref(),
            body.as_slice(),
            "served bytes must match the uploaded bytes"
        );
    }

    // PNG → persisted as `<uuid>.png`, served by bare id. The bare-uuid arm of
    // on_disk_name (generic content type) is covered by the media.rs unit tests;
    // application/octet-stream is not an allowed upload content type so it can't
    // exercise that arm through this route.
    upload_then_serve(
        &harness,
        "image/png",
        "shot.png",
        b"\x89PNG\r\n\x1a\nHELLO-6530".to_vec(),
    )
    .await;
    // text/plain → persisted as `<uuid>.txt`; also round-trips by bare id.
    upload_then_serve(&harness, "text/plain", "note.txt", b"hello-6530".to_vec()).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn test_upload_owner_metadata_survives_registry_miss() {
    let server = start_test_server_with_rbac_users(
        "any-key",
        vec![
            ("Alice", "user", "alice-user-key"),
            ("Eve", "viewer", "eve-viewer-key"),
            ("Admin", "admin", "admin-key"),
        ],
    )
    .await;
    let file_id = uuid::Uuid::new_v4().to_string();
    let upload_dir = server
        .state
        .kernel
        .config_ref()
        .channels
        .effective_file_download_dir();
    std::fs::create_dir_all(&upload_dir).unwrap();
    std::fs::write(upload_dir.join(format!("{file_id}.pdf")), b"private upload").unwrap();
    std::fs::write(
        upload_dir.join(format!(".{file_id}.meta.json")),
        serde_json::to_vec(&serde_json::json!({
            "filename": "private.pdf",
            "content_type": "application/pdf",
            "uploaded_by": librefang_types::agent::UserId::from_name("Alice"),
        }))
        .unwrap(),
    )
    .unwrap();

    let client = reqwest::Client::new();
    let url = format!("{}/api/uploads/{file_id}", server.base_url);
    let owner = client
        .get(&url)
        .header("authorization", "Bearer alice-user-key")
        .send()
        .await
        .unwrap();
    assert_eq!(owner.status(), StatusCode::OK);
    assert_eq!(owner.bytes().await.unwrap().as_ref(), b"private upload");

    let stranger = client
        .get(&url)
        .header("authorization", "Bearer eve-viewer-key")
        .send()
        .await
        .unwrap();
    assert_eq!(stranger.status(), StatusCode::FORBIDDEN);

    let admin = client
        .get(&url)
        .header("authorization", "Bearer admin-key")
        .send()
        .await
        .unwrap();
    assert_eq!(admin.status(), StatusCode::OK);

    let unknown_id = uuid::Uuid::new_v4().to_string();
    std::fs::write(
        upload_dir.join(format!("{unknown_id}.png")),
        b"legacy unknown owner",
    )
    .unwrap();
    let unknown_url = format!("{}/api/uploads/{unknown_id}", server.base_url);
    let stranger = client
        .get(&unknown_url)
        .header("authorization", "Bearer eve-viewer-key")
        .send()
        .await
        .unwrap();
    assert_eq!(stranger.status(), StatusCode::FORBIDDEN);
    let admin = client
        .get(&unknown_url)
        .header("authorization", "Bearer admin-key")
        .send()
        .await
        .unwrap();
    assert_eq!(admin.status(), StatusCode::OK);

    // Daemon-generated browser/image-tool artifacts carry an explicit shared sidecar.
    // Ordinary authenticated users must be able to consume their URL.
    let shared_id = uuid::Uuid::new_v4().to_string();
    std::fs::write(upload_dir.join(format!("{shared_id}.png")), b"shared image").unwrap();
    std::fs::write(
        librefang_types::media::upload_metadata_path(&upload_dir, &shared_id),
        serde_json::to_vec(&librefang_types::media::UploadMetadata {
            filename: "generated.png".to_string(),
            content_type: "image/png".to_string(),
            uploaded_by: None,
        })
        .unwrap(),
    )
    .unwrap();
    let shared = client
        .get(format!("{}/api/uploads/{shared_id}", server.base_url))
        .header("authorization", "Bearer eve-viewer-key")
        .send()
        .await
        .unwrap();
    assert_eq!(shared.status(), StatusCode::OK);
    assert_eq!(shared.bytes().await.unwrap().as_ref(), b"shared image");
}

#[tokio::test(flavor = "multi_thread")]
async fn message_routes_reject_foreign_upload_attachments() {
    // The agent is authored by Eve on purpose. The owner-scoping guard added in
    // #6753 runs ahead of attachment resolution in `send_message`, so a caller
    // who cannot reach the agent at all is turned away with 404
    // `agent_not_found` (covered by `non_owner_cannot_message_another_users_agent`
    // in `owner_scoped_routes_integration.rs`) and never reaches the check this
    // test is about. Eve owning the agent while Alice owns the upload is what
    // isolates the attachment-ownership boundary.
    const EVE_MANIFEST: &str = r#"
name = "eve-agent"
version = "0.1.0"
description = "Integration test agent owned by Eve"
author = "Eve"
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

    let server = start_test_server_with_rbac_users(
        "any-key",
        vec![
            ("Alice", "user", "alice-user-key"),
            ("Eve", "user", "eve-user-key"),
        ],
    )
    .await;
    let client = reqwest::Client::new();
    let spawn = client
        .post(format!("{}/api/agents", server.base_url))
        .header("authorization", "Bearer any-key")
        .json(&serde_json::json!({"manifest_toml": EVE_MANIFEST}))
        .send()
        .await
        .unwrap();
    assert_eq!(spawn.status(), StatusCode::CREATED);
    let spawn_body: serde_json::Value = spawn.json().await.unwrap();
    let agent_id = spawn_body["agent_id"].as_str().unwrap();

    let file_id = uuid::Uuid::new_v4().to_string();
    let upload_dir = server
        .state
        .kernel
        .config_ref()
        .channels
        .effective_file_download_dir();
    std::fs::create_dir_all(&upload_dir).unwrap();
    std::fs::write(upload_dir.join(format!("{file_id}.txt")), b"Alice's secret").unwrap();
    std::fs::write(
        librefang_types::media::upload_metadata_path(&upload_dir, &file_id),
        serde_json::to_vec(&librefang_types::media::UploadMetadata {
            filename: "secret.txt".to_string(),
            content_type: "text/plain".to_string(),
            uploaded_by: Some(librefang_types::agent::UserId::from_name("Alice")),
        })
        .unwrap(),
    )
    .unwrap();
    let body = serde_json::json!({
        "message": "summarize the attachment",
        "attachments": [{
            "file_id": file_id,
            "filename": "secret.txt",
            "content_type": "text/plain"
        }]
    });

    for suffix in ["message", "message/stream"] {
        let response = client
            .post(format!(
                "{}/api/agents/{agent_id}/{suffix}",
                server.base_url
            ))
            .header("authorization", "Bearer eve-user-key")
            .json(&body)
            .send()
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            StatusCode::FORBIDDEN,
            "foreign attachment must be rejected before dispatch on {suffix}"
        );
        let json: serde_json::Value = response.json().await.unwrap();
        assert_eq!(json["code"], "upload_access_denied");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn no_auth_http_upload_can_be_attached_over_websocket() {
    use futures::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::Message;

    let harness = start_full_router("").await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = harness.app.clone();
    let server_task = tokio::spawn(async move {
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .await
        .unwrap();
    });
    let client = reqwest::Client::new();
    let base_url = format!("http://{addr}");

    let spawn = client
        .post(format!("{base_url}/api/agents"))
        .json(&serde_json::json!({"manifest_toml": TEST_MANIFEST}))
        .send()
        .await
        .unwrap();
    assert_eq!(spawn.status(), StatusCode::CREATED);
    let spawn_json: serde_json::Value = spawn.json().await.unwrap();
    let agent_id = spawn_json["agent_id"].as_str().unwrap();

    let upload = client
        .post(format!("{base_url}/api/agents/{agent_id}/upload"))
        .header("content-type", "text/plain")
        .header("x-filename", "note.txt")
        .body("no-auth ws attachment")
        .send()
        .await
        .unwrap();
    assert_eq!(upload.status(), StatusCode::CREATED);
    let upload_json: serde_json::Value = upload.json().await.unwrap();
    let file_id = upload_json["file_id"].as_str().unwrap();

    let (mut socket, _) =
        tokio_tungstenite::connect_async(format!("ws://{addr}/api/agents/{agent_id}/ws"))
            .await
            .unwrap();
    let connected = socket.next().await.unwrap().unwrap();
    assert!(connected.to_text().unwrap().contains("connected"));
    socket
        .send(Message::Text(
            serde_json::json!({
                "type": "message",
                "content": "read the attachment",
                "message_id": "no-auth-upload",
                "attachments": [{
                    "file_id": file_id,
                    "filename": "note.txt",
                    "content_type": "text/plain"
                }]
            })
            .to_string()
            .into(),
        ))
        .await
        .unwrap();

    // Attachment injection happens before provider dispatch.
    // Poll the real session state rather than sleeping for a guessed amount of time.
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        let session = client
            .get(format!("{base_url}/api/agents/{agent_id}/session"))
            .send()
            .await
            .unwrap();
        let body = session.text().await.unwrap();
        if body.contains("no-auth ws attachment") {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "no-auth WS must inject the root-owned HTTP upload; last session body: {body}"
        );
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }

    socket.close(None).await.unwrap();
    server_task.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn websocket_rejects_foreign_attachment_with_structured_error() {
    use futures::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::{client::IntoClientRequest, Message};

    let server = start_test_server_with_rbac_users(
        "any-key",
        vec![
            ("Alice", "user", "alice-user-key"),
            ("Eve", "user", "eve-user-key"),
        ],
    )
    .await;
    let client = reqwest::Client::new();
    let spawn = client
        .post(format!("{}/api/agents", server.base_url))
        .header("authorization", "Bearer any-key")
        .json(&serde_json::json!({"manifest_toml": TEST_MANIFEST}))
        .send()
        .await
        .unwrap();
    assert_eq!(spawn.status(), StatusCode::CREATED);
    let spawn_json: serde_json::Value = spawn.json().await.unwrap();
    let agent_id = spawn_json["agent_id"].as_str().unwrap();

    let file_id = uuid::Uuid::new_v4().to_string();
    let upload_dir = server
        .state
        .kernel
        .config_ref()
        .channels
        .effective_file_download_dir();
    std::fs::create_dir_all(&upload_dir).unwrap();
    std::fs::write(
        upload_dir.join(format!("{file_id}.txt")),
        b"Alice's WS secret",
    )
    .unwrap();
    std::fs::write(
        librefang_types::media::upload_metadata_path(&upload_dir, &file_id),
        serde_json::to_vec(&librefang_types::media::UploadMetadata {
            filename: "secret.txt".to_string(),
            content_type: "text/plain".to_string(),
            uploaded_by: Some(librefang_types::agent::UserId::from_name("Alice")),
        })
        .unwrap(),
    )
    .unwrap();

    let mut request = format!(
        "{}/api/agents/{agent_id}/ws",
        server.base_url.replacen("http://", "ws://", 1)
    )
    .into_client_request()
    .unwrap();
    request
        .headers_mut()
        .insert("authorization", "Bearer eve-user-key".parse().unwrap());
    let (mut socket, _) = tokio_tungstenite::connect_async(request).await.unwrap();
    let connected = socket.next().await.unwrap().unwrap();
    assert!(connected.to_text().unwrap().contains("connected"));
    socket
        .send(Message::Text(
            serde_json::json!({
                "type": "message",
                "content": "read the attachment",
                "message_id": "foreign-upload-turn",
                "attachments": [{
                    "file_id": file_id,
                    "filename": "secret.txt",
                    "content_type": "text/plain"
                }]
            })
            .to_string()
            .into(),
        ))
        .await
        .unwrap();
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(2);
    let denied = loop {
        let frame = tokio::time::timeout_at(deadline, socket.next())
            .await
            .expect("attachment denial frame must be immediate")
            .unwrap()
            .unwrap();
        let json: serde_json::Value = serde_json::from_str(frame.to_text().unwrap()).unwrap();
        if json["code"] == "upload_access_denied" {
            break json;
        }
    };
    assert_eq!(denied["type"], "error");
    assert_eq!(denied["code"], "upload_access_denied");
    assert_eq!(denied["message_id"], "foreign-upload-turn");
    assert!(denied["content"].as_str().unwrap().contains("authorized"));
    assert!(denied.get("error").is_none());
}

#[tokio::test(flavor = "multi_thread")]
async fn hand_message_handler_rejects_foreign_upload_attachment() {
    let harness = start_full_router("").await;
    let mut spawn_request = Request::builder()
        .method("POST")
        .uri("/api/agents")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::to_vec(&serde_json::json!({"manifest_toml": TEST_MANIFEST})).unwrap(),
        ))
        .unwrap();
    spawn_request
        .extensions_mut()
        .insert(axum::extract::ConnectInfo(std::net::SocketAddr::from((
            [127, 0, 0, 1],
            0,
        ))));
    let spawn = harness.app.clone().oneshot(spawn_request).await.unwrap();
    assert_eq!(spawn.status(), StatusCode::CREATED);
    let spawn_body = axum::body::to_bytes(spawn.into_body(), usize::MAX)
        .await
        .unwrap();
    let spawn_json: serde_json::Value = serde_json::from_slice(&spawn_body).unwrap();
    let agent_id: librefang_types::agent::AgentId =
        spawn_json["agent_id"].as_str().unwrap().parse().unwrap();

    let hand_id = harness
        .state
        .kernel
        .hands()
        .list_definitions()
        .into_iter()
        .next()
        .expect("test registry must contain a hand definition")
        .id;
    let instance = harness
        .state
        .kernel
        .hands()
        .activate(&hand_id, std::collections::HashMap::new())
        .unwrap();
    harness
        .state
        .kernel
        .hands()
        .set_agent(instance.instance_id, agent_id)
        .unwrap();

    let file_id = uuid::Uuid::new_v4().to_string();
    let upload_dir = harness
        .state
        .kernel
        .config_ref()
        .channels
        .effective_file_download_dir();
    std::fs::create_dir_all(&upload_dir).unwrap();
    std::fs::write(
        upload_dir.join(format!("{file_id}.txt")),
        b"Alice's hand secret",
    )
    .unwrap();
    std::fs::write(
        librefang_types::media::upload_metadata_path(&upload_dir, &file_id),
        serde_json::to_vec(&librefang_types::media::UploadMetadata {
            filename: "secret.txt".to_string(),
            content_type: "text/plain".to_string(),
            uploaded_by: Some(librefang_types::agent::UserId::from_name("Alice")),
        })
        .unwrap(),
    )
    .unwrap();

    let app = Router::new()
        .route(
            "/api/hands/instances/{id}/message",
            axum::routing::post(routes::hand_send_message),
        )
        .with_state(harness.state.clone());
    let mut request = Request::builder()
        .method("POST")
        .uri(format!(
            "/api/hands/instances/{}/message",
            instance.instance_id
        ))
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::to_vec(&serde_json::json!({
                "message": "read the attachment",
                "attachments": [{
                    "file_id": file_id,
                    "filename": "secret.txt",
                    "content_type": "text/plain"
                }]
            }))
            .unwrap(),
        ))
        .unwrap();
    request
        .extensions_mut()
        .insert(middleware::AuthenticatedApiUser {
            name: "Eve".to_string(),
            role: middleware::UserRole::User,
            user_id: librefang_types::agent::UserId::from_name("Eve"),
        });
    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

// ---------------------------------------------------------------------------
// Per-user LLM provider credentials (#6460 Follow-up B)
//
// Owner-gated CRUD over `/api/users/{name}/provider-keys[/{provider}]`.
// These run against the full auth middleware stack (owner-only write gate
// + the Owner gate on the list GET) so the security posture is exercised
// end-to-end, not just the handler body.
// ---------------------------------------------------------------------------

/// Owner can PUT a provider key and then see the provider NAME (only) come
/// back from the list GET. The list must never surface the secret value.
#[tokio::test(flavor = "multi_thread")]
async fn users_provider_keys_put_then_list_shows_name_not_secret() {
    let server = start_test_server_with_rbac_users(
        "any-key",
        vec![
            ("Owner1", "owner", "owner-key"),
            ("Alice", "user", "alice-key"),
        ],
    )
    .await;
    let client = reqwest::Client::new();
    let secret = "sk-super-secret-value-do-not-leak";

    // PUT Alice's anthropic key as Owner.
    let resp = client
        .put(format!(
            "{}/api/users/Alice/provider-keys/anthropic",
            server.base_url
        ))
        .header("authorization", "Bearer owner-key")
        .json(&serde_json::json!({ "api_key": secret }))
        .send()
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        200,
        "Owner must be allowed to set a user's provider key"
    );
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["provider"], "anthropic");
    // The write response must not echo the key.
    let body_str = body.to_string();
    assert!(
        !body_str.contains(secret),
        "PUT response leaked the secret key value: {body_str}"
    );

    // GET the list as Owner — name is present, value is not.
    let resp = client
        .get(format!("{}/api/users/Alice/provider-keys", server.base_url))
        .header("authorization", "Bearer owner-key")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200, "Owner must be allowed to list keys");
    let text = resp.text().await.unwrap();
    assert!(
        text.contains("anthropic"),
        "list must show the provider name: {text}"
    );
    assert!(
        !text.contains(secret),
        "list GET leaked the secret key value: {text}"
    );
}

/// Owner can DELETE a provider key; a subsequent list no longer shows it,
/// and the delete response reports whether a value actually existed.
#[tokio::test(flavor = "multi_thread")]
async fn users_provider_keys_delete_removes_entry() {
    let server = start_test_server_with_rbac_users(
        "any-key",
        vec![
            ("Owner1", "owner", "owner-key"),
            ("Alice", "user", "alice-key"),
        ],
    )
    .await;
    let client = reqwest::Client::new();

    // Seed a key.
    let resp = client
        .put(format!(
            "{}/api/users/Alice/provider-keys/openai",
            server.base_url
        ))
        .header("authorization", "Bearer owner-key")
        .json(&serde_json::json!({ "api_key": "sk-openai-xyz" }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);

    // Delete it — `removed` must be true.
    let resp = client
        .delete(format!(
            "{}/api/users/Alice/provider-keys/openai",
            server.base_url
        ))
        .header("authorization", "Bearer owner-key")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200, "Owner must be allowed to delete a key");
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(
        body["removed"], true,
        "delete must report the existing key was removed: {body}"
    );

    // List is now empty for Alice.
    let resp = client
        .get(format!("{}/api/users/Alice/provider-keys", server.base_url))
        .header("authorization", "Bearer owner-key")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    let providers = body["providers"].as_array().unwrap();
    assert!(
        providers.is_empty(),
        "provider list must be empty after delete: {body}"
    );
}

/// Non-Owner (Admin) callers are rejected on every provider-key route by
/// the middleware gate — PUT / DELETE via `is_owner_only_write`, and the
/// list GET via `min_role_for_privileged_get`. An Admin must never be able
/// to read or write another user's upstream provider credentials.
#[tokio::test(flavor = "multi_thread")]
async fn users_provider_keys_reject_non_owner() {
    let server = start_test_server_with_rbac_users(
        "any-key",
        vec![
            ("Owner1", "owner", "owner-key"),
            ("Alice", "admin", "alice-admin-key"),
        ],
    )
    .await;
    let client = reqwest::Client::new();

    // Admin PUT — 403.
    let resp = client
        .put(format!(
            "{}/api/users/Alice/provider-keys/anthropic",
            server.base_url
        ))
        .header("authorization", "Bearer alice-admin-key")
        .json(&serde_json::json!({ "api_key": "sk-nope" }))
        .send()
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        403,
        "Admin must NOT be able to set a provider key — Owner only"
    );

    // Admin DELETE — 403.
    let resp = client
        .delete(format!(
            "{}/api/users/Alice/provider-keys/anthropic",
            server.base_url
        ))
        .header("authorization", "Bearer alice-admin-key")
        .send()
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        403,
        "Admin must NOT be able to delete a provider key — Owner only"
    );

    // Admin GET (list) — 403 via the Owner gate on the read.
    let resp = client
        .get(format!("{}/api/users/Alice/provider-keys", server.base_url))
        .header("authorization", "Bearer alice-admin-key")
        .send()
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        403,
        "Admin must NOT be able to enumerate another user's providers — Owner only"
    );

    // Sanity: the Owner IS allowed the same GET, proving the 403 is a role
    // gate and not a broken route.
    let resp = client
        .get(format!("{}/api/users/Alice/provider-keys", server.base_url))
        .header("authorization", "Bearer owner-key")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200, "Owner must be allowed to list keys");
}

/// An invalid provider segment is rejected with 400 by the handler's
/// `validate_provider` guard — an unknown provider name never reaches the
/// vault. (An empty or `/`-containing segment cannot route to the handler
/// at all, so the reachable invalid case is the unknown-name one.)
#[tokio::test(flavor = "multi_thread")]
async fn users_provider_keys_invalid_provider_rejected() {
    let server = start_test_server_with_rbac_users(
        "any-key",
        vec![
            ("Owner1", "owner", "owner-key"),
            ("Alice", "user", "alice-key"),
        ],
    )
    .await;
    let client = reqwest::Client::new();

    let resp = client
        .put(format!(
            "{}/api/users/Alice/provider-keys/not-a-real-provider",
            server.base_url
        ))
        .header("authorization", "Bearer owner-key")
        .json(&serde_json::json!({ "api_key": "sk-whatever" }))
        .send()
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        400,
        "an unknown provider name must be rejected with 400, not stored"
    );

    // An empty api_key for a valid provider is also a 400.
    let resp = client
        .put(format!(
            "{}/api/users/Alice/provider-keys/anthropic",
            server.base_url
        ))
        .header("authorization", "Bearer owner-key")
        .json(&serde_json::json!({ "api_key": "" }))
        .send()
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        400,
        "an empty api_key must be rejected with 400"
    );
}

/// Managing provider keys for a user that does not exist yields 404, so an
/// Owner typo never seeds an orphaned vault entry keyed to a phantom user.
#[tokio::test(flavor = "multi_thread")]
async fn users_provider_keys_unknown_user_404() {
    let server =
        start_test_server_with_rbac_users("any-key", vec![("Owner1", "owner", "owner-key")]).await;
    let client = reqwest::Client::new();

    let resp = client
        .put(format!(
            "{}/api/users/Ghost/provider-keys/anthropic",
            server.base_url
        ))
        .header("authorization", "Bearer owner-key")
        .json(&serde_json::json!({ "api_key": "sk-ghost" }))
        .send()
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        404,
        "setting a key for an unknown user must 404"
    );

    let resp = client
        .get(format!("{}/api/users/Ghost/provider-keys", server.base_url))
        .header("authorization", "Bearer owner-key")
        .send()
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        404,
        "listing keys for an unknown user must 404"
    );
}

// ---------------------------------------------------------------------------
// Explicit session targeting on POST /api/agents/{id}/message — issue #7605
//
// `librefang message --session-id <UUID>` forwards its argument as
// `MessageRequest.session_id`. These tests pin the server-side contract the
// flag depends on: two ids address two histories, and no id keeps the
// canonical-session behaviour every existing caller relies on.
//
// A wiremock server speaking the native Ollama protocol (`POST /api/chat`)
// stands in for the provider, so a complete turn — session load, LLM call,
// history write — runs without credentials or network access. Asserting on a
// *failed* turn would prove nothing: no session write happens before the
// provider call, so an ignored `session_id` would look identical.
// ---------------------------------------------------------------------------

/// Model catalog carrying an `ollama` provider that needs no key and lives at
/// `base_url`. Without it the message handler's provider-auth gate short-
/// circuits with 412 before any session resolution happens.
fn ollama_stub_catalog(base_url: &str) -> librefang_testing::CatalogSeed {
    use librefang_types::model_catalog::{AuthStatus, ProviderInfo};

    let (mut providers, mut models) = librefang_testing::test_catalog_baseline();
    providers.push(ProviderInfo {
        id: "ollama".to_string(),
        display_name: "Ollama (wiremock stub)".to_string(),
        api_key_env: "OLLAMA_API_KEY".to_string(),
        base_url: base_url.to_string(),
        key_required: false,
        auth_status: AuthStatus::NotRequired,
        model_count: 1,
        ..ProviderInfo::default()
    });
    let mut entry = models[0].clone();
    entry.id = "test-model".to_string();
    entry.display_name = "Ollama test model".to_string();
    entry.provider = "ollama".to_string();
    models.push(entry);
    (providers, models)
}

/// Boot a test server whose default provider is a wiremock endpoint that
/// answers every `POST /api/chat` with a one-word assistant turn.
async fn start_test_server_with_stub_llm() -> (TestServer, wiremock::MockServer) {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let llm = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/chat"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "model": "test-model",
            "message": { "role": "assistant", "content": "ack" },
            "done": true,
            "done_reason": "stop",
            "prompt_eval_count": 7,
            "eval_count": 2,
        })))
        .mount(&llm)
        .await;

    let uri = llm.uri();
    let config_uri = uri.clone();
    let server = start_test_server_with_builder(
        MockKernelBuilder::new()
            .with_config(move |cfg| {
                cfg.default_model.provider = "ollama".to_string();
                cfg.default_model.model = "test-model".to_string();
                cfg.default_model.api_key_env = "OLLAMA_API_KEY".to_string();
                cfg.default_model.base_url = Some(config_uri);
                // Keep the turn to a single provider round trip. Proactive
                // memory would add retrieval and extraction work that is
                // orthogonal to session routing (and is the subject of the
                // second half of #7605, tracked separately).
                cfg.proactive_memory.enabled = false;
            })
            .with_catalog_seed(ollama_stub_catalog(&uri)),
    )
    .await;
    (server, llm)
}

async fn spawn_stub_agent(server: &TestServer) -> String {
    let resp = reqwest::Client::new()
        .post(format!("{}/api/agents", server.base_url))
        .json(&serde_json::json!({ "manifest_toml": TEST_MANIFEST }))
        .send()
        .await
        .expect("spawn request");
    assert_eq!(resp.status().as_u16(), 201, "spawn must succeed");
    let body: serde_json::Value = resp.json().await.expect("spawn body");
    body["agent_id"]
        .as_str()
        .expect("agent_id in spawn body")
        .to_string()
}

/// `POST /api/agents/{id}/message`, optionally pinned to `session_id`.
async fn post_agent_message(
    server: &TestServer,
    agent_id: &str,
    text: &str,
    session_id: Option<&str>,
) -> (u16, serde_json::Value) {
    let mut payload = serde_json::json!({ "message": text });
    if let Some(sid) = session_id {
        payload["session_id"] = serde_json::Value::String(sid.to_string());
    }
    let resp = reqwest::Client::new()
        .post(format!(
            "{}/api/agents/{}/message",
            server.base_url, agent_id
        ))
        .json(&payload)
        .send()
        .await
        .expect("message request");
    let status = resp.status().as_u16();
    let body: serde_json::Value = resp.json().await.unwrap_or(serde_json::Value::Null);
    (status, body)
}

/// `GET /api/agents/{id}/session`, optionally pinned to `session_id`.
/// Returns the resolved session id and the flattened message texts.
async fn get_agent_session_texts(
    server: &TestServer,
    agent_id: &str,
    session_id: Option<&str>,
) -> (String, Vec<String>) {
    let url = match session_id {
        Some(sid) => format!(
            "{}/api/agents/{}/session?session_id={}",
            server.base_url, agent_id, sid
        ),
        None => format!("{}/api/agents/{}/session", server.base_url, agent_id),
    };
    let resp = reqwest::Client::new()
        .get(url)
        .send()
        .await
        .expect("session request");
    assert_eq!(resp.status().as_u16(), 200, "session read must succeed");
    let body: serde_json::Value = resp.json().await.expect("session body");
    let texts = body["messages"]
        .as_array()
        .map(|ms| {
            ms.iter()
                .map(|m| m["content"].as_str().unwrap_or_default().to_string())
                .collect()
        })
        .unwrap_or_default();
    (
        body["session_id"]
            .as_str()
            .expect("session_id in body")
            .to_string(),
        texts,
    )
}

/// Two turns addressed with different explicit `session_id`s must land in two
/// separate histories: neither session may contain the other's text.
///
/// This is the property `librefang message --session-id` exists to give
/// scripted callers. Before #7605 the CLI had no way to send the field, so N
/// unrelated end-users of one public agent were interleaved into a single
/// canonical transcript — visitor A's message sitting in the model context of
/// visitor B's turn.
#[tokio::test(flavor = "multi_thread")]
async fn test_message_explicit_session_ids_land_in_distinct_sessions() {
    let (server, _llm) = start_test_server_with_stub_llm().await;
    let agent_id = spawn_stub_agent(&server).await;

    const SESSION_A: &str = "11111111-1111-4111-8111-111111111111";
    const SESSION_B: &str = "22222222-2222-4222-8222-222222222222";

    let (status_a, body_a) =
        post_agent_message(&server, &agent_id, "visitor A secret", Some(SESSION_A)).await;
    assert_eq!(
        status_a, 200,
        "session-pinned turn A must succeed: {body_a}"
    );

    let (status_b, body_b) =
        post_agent_message(&server, &agent_id, "visitor B secret", Some(SESSION_B)).await;
    assert_eq!(
        status_b, 200,
        "session-pinned turn B must succeed: {body_b}"
    );

    let (resolved_a, texts_a) = get_agent_session_texts(&server, &agent_id, Some(SESSION_A)).await;
    let (resolved_b, texts_b) = get_agent_session_texts(&server, &agent_id, Some(SESSION_B)).await;

    assert_eq!(
        resolved_a, SESSION_A,
        "session A must resolve to its own id"
    );
    assert_eq!(
        resolved_b, SESSION_B,
        "session B must resolve to its own id"
    );

    assert!(
        texts_a.iter().any(|t| t.contains("visitor A secret")),
        "session A must hold its own turn: {texts_a:?}"
    );
    assert!(
        !texts_a.iter().any(|t| t.contains("visitor B secret")),
        "session A must NOT hold session B's turn: {texts_a:?}"
    );
    assert!(
        texts_b.iter().any(|t| t.contains("visitor B secret")),
        "session B must hold its own turn: {texts_b:?}"
    );
    assert!(
        !texts_b.iter().any(|t| t.contains("visitor A secret")),
        "session B must NOT hold session A's turn: {texts_b:?}"
    );
}

/// Two turns addressed with the *same* explicit `session_id` must accumulate
/// in one history — the continuity half of the contract, which `--incognito`
/// cannot provide because it suppresses session writes entirely.
#[tokio::test(flavor = "multi_thread")]
async fn test_message_same_explicit_session_id_keeps_continuity() {
    let (server, _llm) = start_test_server_with_stub_llm().await;
    let agent_id = spawn_stub_agent(&server).await;

    const SESSION: &str = "33333333-3333-4333-8333-333333333333";

    for text in ["first turn text", "second turn text"] {
        let (status, body) = post_agent_message(&server, &agent_id, text, Some(SESSION)).await;
        assert_eq!(status, 200, "pinned turn must succeed: {body}");
    }

    let (resolved, texts) = get_agent_session_texts(&server, &agent_id, Some(SESSION)).await;
    assert_eq!(resolved, SESSION);
    assert!(
        texts.iter().any(|t| t.contains("first turn text"))
            && texts.iter().any(|t| t.contains("second turn text")),
        "both turns must share the pinned session: {texts:?}"
    );
}

/// Backward compatibility: omitting `session_id` must keep resolving the
/// agent's canonical session, exactly as before the flag existed.
///
/// This is the assertion that protects existing users of `librefang message`
/// and of the REST endpoint. It also pins that the canonical session stays
/// clean of the pinned conversations: a turn addressed to an explicit session
/// must not also be appended to the canonical one.
#[tokio::test(flavor = "multi_thread")]
async fn test_message_without_session_id_keeps_canonical_session() {
    let (server, _llm) = start_test_server_with_stub_llm().await;
    let agent_id = spawn_stub_agent(&server).await;

    const SESSION_PINNED: &str = "44444444-4444-4444-8444-444444444444";

    // The canonical session id, as the daemon reports it before any traffic.
    let (canonical_before, _) = get_agent_session_texts(&server, &agent_id, None).await;
    assert_ne!(
        canonical_before, SESSION_PINNED,
        "the pinned id must not accidentally be the canonical one"
    );

    // A pinned turn, then an unpinned one.
    let (status, body) =
        post_agent_message(&server, &agent_id, "pinned only", Some(SESSION_PINNED)).await;
    assert_eq!(status, 200, "pinned turn must succeed: {body}");

    let (status, body) = post_agent_message(&server, &agent_id, "canonical only", None).await;
    assert_eq!(status, 200, "unpinned turn must succeed: {body}");
    // #5199: the handler echoes the resolved session id only when the caller
    // did not pin one — which is how a CLI caller learns which session it got.
    assert_eq!(
        body["session_id"].as_str(),
        Some(canonical_before.as_str()),
        "unpinned turn must report the canonical session: {body}"
    );

    let (canonical_after, canonical_texts) =
        get_agent_session_texts(&server, &agent_id, None).await;
    assert_eq!(
        canonical_after, canonical_before,
        "an unpinned turn must not move the canonical session"
    );
    assert!(
        canonical_texts.iter().any(|t| t.contains("canonical only")),
        "canonical session must hold the unpinned turn: {canonical_texts:?}"
    );
    assert!(
        !canonical_texts.iter().any(|t| t.contains("pinned only")),
        "canonical session must NOT hold the pinned turn: {canonical_texts:?}"
    );
}

/// A session id owned by a *different* agent must be refused, not read. The
/// CLI cannot validate ownership locally, so this guard is what keeps a
/// mistyped-but-well-formed UUID from surfacing another agent's history.
#[tokio::test(flavor = "multi_thread")]
async fn test_message_rejects_session_id_owned_by_another_agent() {
    let (server, _llm) = start_test_server_with_stub_llm().await;
    let agent_a = spawn_stub_agent(&server).await;

    const SESSION_A: &str = "55555555-5555-4555-8555-555555555555";
    let (status, body) =
        post_agent_message(&server, &agent_a, "agent A turn", Some(SESSION_A)).await;
    assert_eq!(status, 200, "pinned turn must succeed: {body}");

    // A second agent under the same daemon, addressing agent A's session.
    const MANIFEST_B: &str = r#"
name = "test-agent-session-b"
version = "0.1.0"
description = "Integration test agent"
author = "test"
module = "builtin:chat"

[model]
provider = "ollama"
model = "test-model"
system_prompt = "You are a test agent. Reply concisely."
"#;
    let resp = reqwest::Client::new()
        .post(format!("{}/api/agents", server.base_url))
        .json(&serde_json::json!({ "manifest_toml": MANIFEST_B }))
        .send()
        .await
        .expect("spawn B");
    assert_eq!(resp.status().as_u16(), 201);
    let body: serde_json::Value = resp.json().await.expect("spawn B body");
    let agent_b = body["agent_id"].as_str().expect("agent_id").to_string();

    let (status, body) =
        post_agent_message(&server, &agent_b, "agent B probe", Some(SESSION_A)).await;
    assert_ne!(
        status, 200,
        "agent B must not be able to drive agent A's session: {body}"
    );

    // Agent A's history is untouched by the rejected probe.
    let (_, texts_a) = get_agent_session_texts(&server, &agent_a, Some(SESSION_A)).await;
    assert!(
        !texts_a.iter().any(|t| t.contains("agent B probe")),
        "cross-agent probe must not append to the target session: {texts_a:?}"
    );
}

/// A malformed `session_id` is a 400 with a stable machine code, not a 422 and
/// not a silent fallback to the canonical session. The CLI rejects bad UUIDs
/// before sending, so this covers every other caller of the endpoint.
#[tokio::test(flavor = "multi_thread")]
async fn test_message_rejects_malformed_session_id() {
    let (server, _llm) = start_test_server_with_stub_llm().await;
    let agent_id = spawn_stub_agent(&server).await;

    let (status, body) = post_agent_message(&server, &agent_id, "hello", Some("not-a-uuid")).await;
    assert_eq!(status, 400, "malformed session_id must be a 400: {body}");
    assert_eq!(
        body["code"].as_str(),
        Some("invalid_session_id"),
        "error code must be stable for scripted callers: {body}"
    );
}
