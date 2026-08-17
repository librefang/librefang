//! Integration coverage for `POST /api/channels/sidecar/{name}/configure`.

use axum::body::{to_bytes, Body};
use axum::http::{header, Method, Request, StatusCode};
use librefang_api::server;
use librefang_kernel::LibreFangKernel;
use librefang_types::config::{DefaultModelConfig, KernelConfig};
use std::sync::Arc;
use tower::ServiceExt;

const API_KEY: &str = "test-secret-key";

struct RouterHarness {
    app: axum::Router,
    home: std::path::PathBuf,
    _tmp: tempfile::TempDir,
    state: Arc<librefang_api::routes::AppState>,
}

impl Drop for RouterHarness {
    fn drop(&mut self) {
        self.state.kernel.shutdown();
    }
}

async fn boot_router() -> RouterHarness {
    let tmp = tempfile::tempdir().expect("tempdir");
    librefang_kernel::registry_sync::seed_registry_fixture_for_tests(tmp.path());
    let config = KernelConfig {
        home_dir: tmp.path().to_path_buf(),
        data_dir: tmp.path().join("data"),
        api_key: API_KEY.to_string(),
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
    let home = config.home_dir.clone();
    let kernel = Arc::new(LibreFangKernel::boot_with_config(config).expect("kernel boot"));
    kernel.set_self_handle();
    let (app, state) = server::build_router(kernel, "127.0.0.1:0".parse().unwrap()).await;
    RouterHarness {
        app,
        home,
        _tmp: tmp,
        state,
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn configure_rejects_included_sidecar_without_partial_writes() {
    let harness = boot_router().await;
    let config_path = harness.home.join("config.toml");
    let included_path = harness.home.join("channels.toml");
    std::fs::write(&config_path, "include = [\"channels.toml\"]\n").unwrap();
    std::fs::write(
        &included_path,
        "[[sidecar_channels]]\nname = \"telegram\"\ncommand = \"python3\"\n",
    )
    .unwrap();

    let request = Request::builder()
        .method(Method::POST)
        .uri("/api/channels/sidecar/telegram/configure")
        .header(header::AUTHORIZATION, format!("Bearer {API_KEY}"))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            serde_json::json!({"values": {"TELEGRAM_BOT_TOKEN": "secret"}}).to_string(),
        ))
        .unwrap();
    let response = harness.app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let body = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();

    assert_eq!(
        status,
        StatusCode::CONFLICT,
        "{}",
        String::from_utf8_lossy(&body)
    );
    assert_eq!(
        std::fs::read_to_string(&config_path).unwrap(),
        "include = [\"channels.toml\"]\n"
    );
    assert!(!harness.home.join("secrets.env").exists());
}

#[tokio::test(flavor = "multi_thread")]
async fn configure_rolls_back_secrets_when_config_write_fails() {
    let harness = boot_router().await;
    let config_path = harness.home.join("config.toml");
    let secrets_path = harness.home.join("secrets.env");
    let original_config = "sidecar_channels = \"not-an-array\"\n";
    let original_secrets = "TELEGRAM_BOT_TOKEN=old-secret\nKEEP_ME=unchanged\n";
    std::fs::write(&config_path, original_config).unwrap();
    std::fs::write(&secrets_path, original_secrets).unwrap();

    let request = Request::builder()
        .method(Method::POST)
        .uri("/api/channels/sidecar/telegram/configure")
        .header(header::AUTHORIZATION, format!("Bearer {API_KEY}"))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            serde_json::json!({"values": {"TELEGRAM_BOT_TOKEN": "new-secret"}}).to_string(),
        ))
        .unwrap();
    let response = harness.app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let body = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();

    assert_eq!(
        status,
        StatusCode::INTERNAL_SERVER_ERROR,
        "{}",
        String::from_utf8_lossy(&body)
    );
    let response_text = String::from_utf8_lossy(&body);
    assert!(!response_text.contains("not-an-array"));
    assert!(!response_text.contains(&harness.home.display().to_string()));
    assert_eq!(
        std::fs::read_to_string(config_path).unwrap(),
        original_config
    );
    assert_eq!(
        std::fs::read_to_string(secrets_path).unwrap(),
        original_secrets
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn registry_returns_typed_metadata_array() {
    let harness = boot_router().await;
    let channels_dir = harness.home.join("channels");
    std::fs::create_dir_all(&channels_dir).unwrap();
    std::fs::write(
        channels_dir.join("audit-test.toml"),
        "id = \"audit-test\"\nname = \"Audit Test\"\ndescription = \"typed response\"\n",
    )
    .unwrap();

    let request = Request::builder()
        .method(Method::GET)
        .uri("/api/channels/registry")
        .header(header::AUTHORIZATION, format!("Bearer {API_KEY}"))
        .body(Body::empty())
        .unwrap();
    let response = harness.app.clone().oneshot(request).await.unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    let metadata: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let entry = metadata
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["id"] == "audit-test")
        .unwrap();
    assert_eq!(entry["name"], "Audit Test");
    assert_eq!(entry["description"], "typed response");
}
