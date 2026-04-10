//! Integration tests for the plugin management API routes.
//!
//! These tests boot a real kernel and axum router, then exercise the public
//! `/api/plugins*` surface through HTTP requests.
//!
//! Run: cargo test -p librefang-api --test plugins_routes_tests -- --nocapture

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use librefang_api::routes::AppState;
use librefang_api::server;
use librefang_kernel::LibreFangKernel;
use librefang_types::config::{DefaultModelConfig, KernelConfig};
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

async fn start_router(admin_accounts: Vec<&str>) -> Harness {
    let tmp = tempfile::tempdir().expect("Failed to create temp dir");
    let config = KernelConfig {
        home_dir: tmp.path().to_path_buf(),
        data_dir: tmp.path().join("data"),
        multi_tenant: true,
        admin_accounts: admin_accounts.into_iter().map(str::to_string).collect(),
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

    Harness {
        app,
        state,
        _tmp: tmp,
    }
}

async fn read_json(resp: axum::response::Response) -> serde_json::Value {
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    serde_json::from_slice(&body).unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn plugins_list_rejects_missing_account_id() {
    let h = start_router(vec!["admin"]).await;

    let resp = h
        .send(
            Request::builder()
                .uri("/api/plugins")
                .body(Body::empty())
                .unwrap(),
        )
        .await;

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test(flavor = "multi_thread")]
async fn plugins_list_rejects_non_admin_account() {
    let h = start_router(vec!["admin"]).await;

    let resp = h
        .send(
            Request::builder()
                .uri("/api/plugins")
                .header("x-account-id", "tenant-a")
                .body(Body::empty())
                .unwrap(),
        )
        .await;

    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

#[tokio::test(flavor = "multi_thread")]
async fn plugins_list_returns_expected_shape_for_admin() {
    let h = start_router(vec!["admin"]).await;

    let resp = h
        .send(
            Request::builder()
                .uri("/api/plugins")
                .header("x-account-id", "admin")
                .body(Body::empty())
                .unwrap(),
        )
        .await;

    assert_eq!(resp.status(), StatusCode::OK);
    let json = read_json(resp).await;
    assert!(json["plugins"].is_array(), "unexpected response: {json}");
    assert!(json["total"].is_number(), "unexpected response: {json}");
    assert!(
        json["plugins_dir"].is_string(),
        "unexpected response: {json}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn plugins_get_missing_returns_404() {
    let h = start_router(vec!["admin"]).await;

    let resp = h
        .send(
            Request::builder()
                .uri("/api/plugins/no-such-plugin")
                .header("x-account-id", "admin")
                .body(Body::empty())
                .unwrap(),
        )
        .await;

    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test(flavor = "multi_thread")]
async fn plugins_install_rejects_missing_name_for_registry_source() {
    let h = start_router(vec!["admin"]).await;

    let resp = h
        .send(
            Request::builder()
                .method("POST")
                .uri("/api/plugins/install")
                .header("content-type", "application/json")
                .header("x-account-id", "admin")
                .body(Body::from(
                    serde_json::json!({"source": "registry"}).to_string(),
                ))
                .unwrap(),
        )
        .await;

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test(flavor = "multi_thread")]
async fn plugins_install_rejects_invalid_source() {
    let h = start_router(vec!["admin"]).await;

    let resp = h
        .send(
            Request::builder()
                .method("POST")
                .uri("/api/plugins/install")
                .header("content-type", "application/json")
                .header("x-account-id", "admin")
                .body(Body::from(
                    serde_json::json!({"source": "bogus"}).to_string(),
                ))
                .unwrap(),
        )
        .await;

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test(flavor = "multi_thread")]
async fn plugins_uninstall_rejects_missing_name() {
    let h = start_router(vec!["admin"]).await;

    let resp = h
        .send(
            Request::builder()
                .method("POST")
                .uri("/api/plugins/uninstall")
                .header("content-type", "application/json")
                .header("x-account-id", "admin")
                .body(Body::from(serde_json::json!({}).to_string()))
                .unwrap(),
        )
        .await;

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test(flavor = "multi_thread")]
async fn plugins_scaffold_rejects_missing_name() {
    let h = start_router(vec!["admin"]).await;

    let resp = h
        .send(
            Request::builder()
                .method("POST")
                .uri("/api/plugins/scaffold")
                .header("content-type", "application/json")
                .header("x-account-id", "admin")
                .body(Body::from(serde_json::json!({}).to_string()))
                .unwrap(),
        )
        .await;

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test(flavor = "multi_thread")]
async fn plugins_doctor_returns_report_shape_for_admin() {
    let h = start_router(vec!["admin"]).await;

    let resp = h
        .send(
            Request::builder()
                .uri("/api/plugins/doctor")
                .header("x-account-id", "admin")
                .body(Body::empty())
                .unwrap(),
        )
        .await;

    assert_eq!(resp.status(), StatusCode::OK);
    let json = read_json(resp).await;
    assert!(json["runtimes"].is_array(), "unexpected response: {json}");
    assert!(json["plugins"].is_array(), "unexpected response: {json}");
}

#[tokio::test(flavor = "multi_thread")]
async fn plugins_registries_returns_registry_list_for_admin() {
    let h = start_router(vec!["admin"]).await;

    let resp = h
        .send(
            Request::builder()
                .uri("/api/plugins/registries")
                .header("x-account-id", "admin")
                .body(Body::empty())
                .unwrap(),
        )
        .await;

    assert_eq!(resp.status(), StatusCode::OK);
    let json = read_json(resp).await;
    let registries = json["registries"].as_array().expect("registries array");
    assert!(!registries.is_empty(), "unexpected response: {json}");
    let first = &registries[0];
    assert!(first["name"].is_string(), "unexpected response: {json}");
    assert!(
        first["github_repo"].is_string(),
        "unexpected response: {json}"
    );
    assert!(first["plugins"].is_array(), "unexpected response: {json}");
}
