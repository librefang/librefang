//! Focused integration tests for untested auth/config admin flows.
//!
//! Covers dashboard auth helpers plus selected admin-only config/system
//! endpoints through the real router and middleware stack.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use librefang_api::password_hash::{generate_session_token, SessionToken};
use librefang_api::routes::AppState;
use librefang_api::server;
use librefang_kernel::LibreFangKernel;
use librefang_types::config::{DefaultModelConfig, KernelConfig};
use std::collections::HashMap;
use std::sync::Arc;
use tower::ServiceExt;

struct Harness {
    app: Router,
    state: Arc<AppState>,
    _tmp: tempfile::TempDir,
}

impl Drop for Harness {
    fn drop(&mut self) {
        self.state.kernel.shutdown();
    }
}

impl Harness {
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

fn authenticated_request(
    method: axum::http::Method,
    uri: &str,
    api_key: &str,
    account_id: Option<&str>,
) -> Request<Body> {
    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .header("authorization", format!("Bearer {api_key}"));
    if let Some(account_id) = account_id {
        builder = builder.header("x-account-id", account_id);
    }
    builder.body(Body::empty()).unwrap()
}

async fn start_router(mut config: KernelConfig) -> Harness {
    let tmp = tempfile::tempdir().expect("Failed to create temp dir");
    config.home_dir = tmp.path().to_path_buf();
    config.data_dir = tmp.path().join("data");
    if config.default_model.provider.is_empty() {
        config.default_model = DefaultModelConfig {
            provider: "ollama".to_string(),
            model: "test-model".to_string(),
            api_key_env: "OLLAMA_API_KEY".to_string(),
            base_url: None,
            message_timeout_secs: 300,
        };
    }

    let kernel = LibreFangKernel::boot_with_config(config).expect("Kernel should boot");
    let kernel = Arc::new(kernel);
    kernel.set_self_handle();

    let (app, state) = server::build_router(
        kernel,
        "127.0.0.1:0".parse().expect("listen addr should parse"),
    )
    .await;

    Harness {
        app,
        state,
        _tmp: tmp,
    }
}

fn default_config() -> KernelConfig {
    KernelConfig {
        default_model: DefaultModelConfig {
            provider: "ollama".to_string(),
            model: "test-model".to_string(),
            api_key_env: "OLLAMA_API_KEY".to_string(),
            base_url: None,
            message_timeout_secs: 300,
        },
        ..KernelConfig::default()
    }
}

fn persist_sessions(home_dir: &std::path::Path, sessions: &HashMap<String, SessionToken>) {
    std::fs::write(
        home_dir.join("sessions.json"),
        serde_json::to_vec(sessions).expect("serialize sessions"),
    )
    .expect("write sessions");
}

#[tokio::test(flavor = "multi_thread")]
async fn dashboard_check_reports_credentials_mode() {
    let mut config = default_config();
    config.dashboard_user = "admin".to_string();
    config.dashboard_pass = "hunter2-password".to_string();

    let h = start_router(config).await;
    let resp = h
        .send(
            Request::builder()
                .uri("/api/auth/dashboard-check")
                .body(Body::empty())
                .unwrap(),
        )
        .await;

    assert_eq!(resp.status(), StatusCode::OK);
    let json = read_json(resp).await;
    assert_eq!(json["mode"], "credentials");
    assert_eq!(json["username"], "admin");
}

#[tokio::test(flavor = "multi_thread")]
async fn dashboard_login_creates_active_and_persisted_session() {
    let mut config = default_config();
    config.dashboard_user = "admin".to_string();
    config.dashboard_pass = "hunter2-password".to_string();

    let h = start_router(config).await;
    let resp = h
        .send(
            Request::builder()
                .method("POST")
                .uri("/api/auth/dashboard-login")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "username": "admin",
                        "password": "hunter2-password"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await;

    assert_eq!(resp.status(), StatusCode::OK);
    let json = read_json(resp).await;
    let token = json["token"].as_str().expect("token string");
    assert!(!token.is_empty());

    let sessions = h.state.active_sessions.read().await;
    assert!(sessions.contains_key(token));
    drop(sessions);

    let persisted = std::fs::read_to_string(h.state.kernel.home_dir().join("sessions.json"))
        .expect("persisted sessions");
    assert!(persisted.contains(token));
}

#[tokio::test(flavor = "multi_thread")]
async fn change_password_updates_config_and_clears_sessions() {
    let mut config = default_config();
    config.api_key = "secret-api-key".to_string();
    config.dashboard_user = "admin".to_string();
    config.dashboard_pass = "old-password".to_string();

    let h = start_router(config).await;

    let seeded = generate_session_token();
    {
        let mut sessions = h.state.active_sessions.write().await;
        sessions.insert(seeded.token.clone(), seeded.clone());
        persist_sessions(h.state.kernel.home_dir(), &sessions);
    }

    let resp = h
        .send(
            Request::builder()
                .method("POST")
                .uri("/api/auth/change-password")
                .header("authorization", "Bearer secret-api-key")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "current_password": "old-password",
                        "new_password": "new-password-123",
                        "new_username": "admin2"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await;

    assert_eq!(resp.status(), StatusCode::OK);
    let json = read_json(resp).await;
    assert_eq!(json["ok"], true);

    let config_toml = std::fs::read_to_string(h.state.kernel.home_dir().join("config.toml"))
        .expect("config.toml");
    assert!(config_toml.contains("dashboard_user = \"admin2\""));
    assert!(config_toml.contains("dashboard_pass_hash = "));

    let sessions = h.state.active_sessions.read().await;
    assert!(sessions.is_empty(), "sessions should be cleared");
    drop(sessions);

    assert!(
        !h.state.kernel.home_dir().join("sessions.json").exists(),
        "sessions file should be removed"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn security_status_is_admin_only() {
    let mut config = default_config();
    config.multi_tenant = true;
    config.admin_accounts = vec!["tenant-admin".to_string()];

    let h = start_router(config).await;

    let tenant_resp = h
        .send(
            Request::builder()
                .uri("/api/security")
                .header("x-account-id", "tenant-a")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(tenant_resp.status(), StatusCode::FORBIDDEN);

    let admin_resp = h
        .send(
            Request::builder()
                .uri("/api/security")
                .header("x-account-id", "tenant-admin")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(admin_resp.status(), StatusCode::OK);
    let json = read_json(admin_resp).await;
    assert_eq!(json["total_features"], 15);
    assert_eq!(json["configurable"]["auth"]["api_key_set"], false);
}

#[tokio::test(flavor = "multi_thread")]
async fn config_set_persists_nested_value_for_admin() {
    let mut config = default_config();
    config.multi_tenant = true;
    config.admin_accounts = vec!["tenant-admin".to_string()];

    let h = start_router(config).await;
    let resp = h
        .send(
            Request::builder()
                .method("POST")
                .uri("/api/config/set")
                .header("x-account-id", "tenant-admin")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "path": "budget.max_daily_usd",
                        "value": 42.5
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await;

    assert_eq!(resp.status(), StatusCode::OK);
    let json = read_json(resp).await;
    assert!(
        matches!(
            json["status"].as_str(),
            Some("applied") | Some("applied_partial")
        ),
        "unexpected config_set status: {}",
        json["status"]
    );
    assert_eq!(json["path"], "budget.max_daily_usd");

    let config_toml = std::fs::read_to_string(h.state.kernel.home_dir().join("config.toml"))
        .expect("config.toml");
    assert!(config_toml.contains("[budget]"));
    assert!(config_toml.contains("max_daily_usd = 42.5"));
}

#[tokio::test(flavor = "multi_thread")]
async fn migrate_scan_rejects_missing_directory_for_admin() {
    let mut config = default_config();
    config.multi_tenant = true;
    config.admin_accounts = vec!["tenant-admin".to_string()];

    let h = start_router(config).await;
    let resp = h
        .send(
            Request::builder()
                .method("POST")
                .uri("/api/migrate/scan")
                .header("x-account-id", "tenant-admin")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "path": "/definitely/missing/librefang-test-path"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await;

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let json = read_json(resp).await;
    assert_eq!(json["error"], "Directory not found");
}

#[tokio::test(flavor = "multi_thread")]
async fn quick_init_creates_config_for_admin() {
    let mut config = default_config();
    config.multi_tenant = true;
    config.admin_accounts = vec!["tenant-admin".to_string()];

    let h = start_router(config).await;
    let config_path = h.state.kernel.home_dir().join("config.toml");
    assert!(
        !config_path.exists(),
        "test expects config.toml to be absent"
    );

    let resp = h
        .send(
            Request::builder()
                .method("POST")
                .uri("/api/init")
                .header("x-account-id", "tenant-admin")
                .body(Body::empty())
                .unwrap(),
        )
        .await;

    assert_eq!(resp.status(), StatusCode::OK);
    let json = read_json(resp).await;
    assert_eq!(json["status"], "initialized");
    assert!(config_path.exists(), "quick_init should create config.toml");

    let config_toml = std::fs::read_to_string(config_path).expect("config.toml");
    assert!(config_toml.contains("[default_model]"));
    assert!(config_toml.contains("provider = "));
    assert!(config_toml.contains("model = "));
}

#[tokio::test(flavor = "multi_thread")]
async fn privileged_get_routes_require_real_auth_when_api_key_is_configured() {
    let mut config = default_config();
    config.api_key = "secret-api-key".to_string();
    config.admin_accounts = vec!["admin".to_string()];

    let h = start_router(config).await;

    for uri in [
        "/api/status",
        "/api/config",
        "/api/agents",
        "/api/budget/agents",
        "/api/channels",
        "/api/workflows",
        "/api/models",
        "/api/models/aliases",
        "/api/providers",
        "/api/network/status",
    ] {
        let resp = h
            .send(
                Request::builder()
                    .uri(uri)
                    .header("x-account-id", "admin")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await;
        assert_eq!(
            resp.status(),
            StatusCode::UNAUTHORIZED,
            "{uri} should require API auth"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn privileged_get_routes_accept_authenticated_admin_or_tenant_access() {
    let mut config = default_config();
    config.api_key = "secret-api-key".to_string();
    config.admin_accounts = vec!["admin".to_string()];

    let h = start_router(config).await;

    let admin_status = h
        .send(authenticated_request(
            axum::http::Method::GET,
            "/api/status",
            "secret-api-key",
            Some("admin"),
        ))
        .await;
    assert_eq!(admin_status.status(), StatusCode::OK);

    let admin_config = h
        .send(authenticated_request(
            axum::http::Method::GET,
            "/api/config",
            "secret-api-key",
            Some("admin"),
        ))
        .await;
    assert_eq!(admin_config.status(), StatusCode::OK);

    let tenant_channels = h
        .send(authenticated_request(
            axum::http::Method::GET,
            "/api/channels",
            "secret-api-key",
            Some("tenant-a"),
        ))
        .await;
    assert_eq!(tenant_channels.status(), StatusCode::OK);

    let tenant_workflows = h
        .send(authenticated_request(
            axum::http::Method::GET,
            "/api/workflows",
            "secret-api-key",
            Some("tenant-a"),
        ))
        .await;
    assert_eq!(tenant_workflows.status(), StatusCode::OK);

    let tenant_agents = h
        .send(authenticated_request(
            axum::http::Method::GET,
            "/api/agents",
            "secret-api-key",
            Some("tenant-a"),
        ))
        .await;
    assert_eq!(tenant_agents.status(), StatusCode::OK);

    for uri in [
        "/api/models",
        "/api/models/aliases",
        "/api/providers",
        "/api/network/status",
    ] {
        let resp = h
            .send(authenticated_request(
                axum::http::Method::GET,
                uri,
                "secret-api-key",
                Some("admin"),
            ))
            .await;
        assert_eq!(
            resp.status(),
            StatusCode::OK,
            "{uri} should allow authenticated admin access"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn versioned_public_system_routes_match_unversioned_aliases_in_multi_tenant_mode() {
    let mut config = default_config();
    config.multi_tenant = true;

    let h = start_router(config).await;

    for uri in [
        "/api/profiles",
        "/api/v1/profiles",
        "/api/profiles/minimal",
        "/api/v1/profiles/minimal",
        "/api/commands",
        "/api/v1/commands",
    ] {
        let resp = h
            .send(Request::builder().uri(uri).body(Body::empty()).unwrap())
            .await;
        assert_eq!(
            resp.status(),
            StatusCode::OK,
            "{uri} should remain publicly reachable without X-Account-Id"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn versioned_and_unversioned_protected_routes_require_the_same_auth() {
    let mut config = default_config();
    config.api_key = "secret-api-key".to_string();
    config.admin_accounts = vec!["admin".to_string()];

    let h = start_router(config).await;

    for uri in [
        "/api/status",
        "/api/v1/status",
        "/api/agents",
        "/api/v1/agents",
    ] {
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
            StatusCode::UNAUTHORIZED,
            "{uri} should require auth"
        );
    }

    for uri in ["/api/agents", "/api/v1/agents"] {
        let resp = h
            .send(authenticated_request(
                axum::http::Method::GET,
                uri,
                "secret-api-key",
                Some("tenant-a"),
            ))
            .await;
        assert_eq!(
            resp.status(),
            StatusCode::OK,
            "{uri} should allow authenticated tenant access"
        );
    }

    for uri in ["/api/status", "/api/v1/status"] {
        let resp = h
            .send(authenticated_request(
                axum::http::Method::GET,
                uri,
                "secret-api-key",
                Some("admin"),
            ))
            .await;
        assert_eq!(
            resp.status(),
            StatusCode::OK,
            "{uri} should allow authenticated admin access"
        );
    }
}
