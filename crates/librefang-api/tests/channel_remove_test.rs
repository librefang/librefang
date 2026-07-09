//! Integration test for `DELETE /api/channels/sidecar/{name}` (channel removal).
//!
//! Tempdir-backed kernel so the config.toml rewrite lands in the sandbox.
//! The block is written to disk after boot, so the kernel's in-memory config
//! never carried the channel — removing it yields no `ReloadChannels` action,
//! keeping the test free of sidecar-spawn side effects.

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
    let (app, state) = server::build_router(kernel, "127.0.0.1:0".parse().expect("addr")).await;
    RouterHarness {
        app,
        home,
        _tmp: tmp,
        state,
    }
}

async fn send(app: axum::Router, req: Request<Body>) -> (StatusCode, Vec<u8>) {
    let resp = app.oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap()
        .to_vec();
    (status, bytes)
}

fn auth_delete(path: &str) -> Request<Body> {
    Request::builder()
        .method(Method::DELETE)
        .uri(path)
        .header(header::AUTHORIZATION, format!("Bearer {API_KEY}"))
        .body(Body::empty())
        .unwrap()
}

const TELEGRAM_BLOCK: &str = "[[sidecar_channels]]\n\
     name = \"telegram\"\n\
     channel_type = \"telegram\"\n\
     command = \"python3\"\n\
     args = [\"-m\", \"librefang.sidecar.adapters.telegram\"]\n\
     \n\
     [sidecar_channels.env]\n\
     ALLOWED_USERS = \"1,2\"\n";

#[tokio::test(flavor = "multi_thread")]
async fn delete_removes_configured_sidecar_then_404s_on_repeat() {
    let h = boot_router().await;
    let config_path = h.home.join("config.toml");
    std::fs::write(&config_path, TELEGRAM_BLOCK).expect("seed config.toml");

    let (status, body) = send(h.app.clone(), auth_delete("/api/channels/sidecar/telegram")).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "body: {}",
        String::from_utf8_lossy(&body)
    );
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["status"], "removed");

    let written = std::fs::read_to_string(&config_path).expect("config.toml still present");
    assert!(
        !written.contains("[[sidecar_channels]]") && !written.contains("name = \"telegram\""),
        "block must be gone: {written}"
    );

    let (status, _) = send(h.app.clone(), auth_delete("/api/channels/sidecar/telegram")).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "second delete must 404");
}

#[tokio::test(flavor = "multi_thread")]
async fn delete_unknown_sidecar_404s() {
    let h = boot_router().await;
    let (status, _) = send(h.app.clone(), auth_delete("/api/channels/sidecar/nope")).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}
