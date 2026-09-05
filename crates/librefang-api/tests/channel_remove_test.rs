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
    boot_router_with_sidecars(Vec::new()).await
}

/// Boot with `sidecar_channels` already in the kernel's *in-memory* config,
/// which is what a running daemon holds and what `GET /api/channels` reports —
/// independently of whatever config.toml says at any later moment.
async fn boot_router_with_sidecars(
    sidecar_channels: Vec<librefang_types::config::SidecarChannelConfig>,
) -> RouterHarness {
    let tmp = tempfile::tempdir().expect("tempdir");
    librefang_kernel::registry_sync::seed_registry_fixture_for_tests(tmp.path());
    let config = KernelConfig {
        home_dir: tmp.path().to_path_buf(),
        data_dir: tmp.path().join("data"),
        api_key: API_KEY.to_string(),
        sidecar_channels,
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

const EMAIL_BLOCK: &str = "[[sidecar_channels]]\n\
     name = \"email\"\n\
     channel_type = \"email\"\n\
     command = \"python3\"\n\
     args = [\"-m\", \"librefang.sidecar.adapters.email\"]\n";

/// A sidecar declared in an `include = [...]` file is a fully live channel:
/// the kernel merges included files into the running config, so it spawns,
/// it supervises, and `list_channels` renders it as `configured` — which is
/// the only state in which the dashboard offers the delete button. Deleting
/// used to rewrite the root config.toml only, find nothing, and answer
/// "404 no configured sidecar channel named email" for a channel that was
/// running at that moment.
#[tokio::test(flavor = "multi_thread")]
async fn delete_removes_sidecar_declared_in_an_included_file() {
    let h = boot_router().await;
    let config_path = h.home.join("config.toml");
    let included_path = h.home.join("channels.toml");
    std::fs::write(&config_path, "include = [\"channels.toml\"]\n").expect("seed config.toml");
    std::fs::write(&included_path, EMAIL_BLOCK).expect("seed channels.toml");

    let (status, body) = send(h.app.clone(), auth_delete("/api/channels/sidecar/email")).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "body: {}",
        String::from_utf8_lossy(&body)
    );
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["status"], "removed");

    let written = std::fs::read_to_string(&included_path).expect("included file still present");
    assert!(
        !written.contains("[[sidecar_channels]]") && !written.contains("name = \"email\""),
        "block must be gone from the included file: {written}"
    );
    assert_eq!(
        std::fs::read_to_string(&config_path).expect("root config still present"),
        "include = [\"channels.toml\"]\n",
        "the root config must be left untouched"
    );

    let (status, _) = send(h.app.clone(), auth_delete("/api/channels/sidecar/email")).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "second delete must 404");
}

/// Reconcile path: the daemon is running a sidecar whose `[[sidecar_channels]]`
/// block is no longer on disk. The dashboard reads `configured` from the live
/// in-memory config, so the card — and its delete button — are still there,
/// while the delete found nothing to strip and answered
/// "404 no configured sidecar channel named email" for a channel whose child
/// process was alive. Observed on the rodela deployment on 2026-08-24: config
/// carried telegram only, `GET /api/channels` reported email as configured,
/// supervised and connected, and its adapter process was running.
///
/// The delete must complete: nothing to remove on disk is not "does not
/// exist", and the reload that follows is what stops the orphaned child.
#[tokio::test(flavor = "multi_thread")]
async fn delete_reconciles_a_live_sidecar_that_is_no_longer_on_disk() {
    // Deliberately unspawnable (and `restart = false`, so the supervisor does
    // not retry): the point of the test is that a channel must be deletable
    // regardless of whether its sidecar is up.
    let email: librefang_types::config::SidecarChannelConfig = toml::from_str(
        "name = \"email\"\n\
         channel_type = \"email\"\n\
         command = \"/nonexistent/librefang-test-email-sidecar\"\n\
         restart = false\n",
    )
    .expect("parse sidecar entry");
    let h = boot_router_with_sidecars(vec![email]).await;
    // config.toml never declared it — the file is the post-edit state.
    std::fs::write(h.home.join("config.toml"), "# no sidecar_channels\n").expect("seed config");

    let (status, body) = send(h.app.clone(), auth_delete("/api/channels/sidecar/email")).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "a live channel must be deletable even with no block left on disk; body: {}",
        String::from_utf8_lossy(&body)
    );
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["status"], "removed");

    // Reconciled: the reload dropped it from the live config, so the channel
    // really is gone and a repeat delete is a genuine 404.
    let (status, _) = send(h.app.clone(), auth_delete("/api/channels/sidecar/email")).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "second delete must 404");
}

#[tokio::test(flavor = "multi_thread")]
async fn delete_unknown_sidecar_404s() {
    let h = boot_router().await;
    let (status, _) = send(h.app.clone(), auth_delete("/api/channels/sidecar/nope")).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

/// A name declared in the root config.toml AND an included file at once: the
/// early-returning walk this test replaces stripped only the root block,
/// reported `removed`, and the reload re-merged the survivor from the include,
/// so the channel came back — a fresh version of the bug this handler fixes.
/// Both blocks must go in one delete, and a repeat delete must be a real 404.
#[tokio::test(flavor = "multi_thread")]
async fn delete_strips_a_sidecar_declared_in_root_and_include_at_once() {
    let h = boot_router().await;
    let config_path = h.home.join("config.toml");
    let included_path = h.home.join("channels.toml");
    // `include` must precede the first table header: written after
    // TELEGRAM_BLOCK it lands under `[sidecar_channels.env]`, not at the
    // document root, and the included file is never scanned.
    std::fs::write(
        &config_path,
        format!("include = [\"channels.toml\"]\n{TELEGRAM_BLOCK}"),
    )
    .expect("seed config.toml");
    std::fs::write(&included_path, TELEGRAM_BLOCK).expect("seed channels.toml");

    let (status, body) = send(h.app.clone(), auth_delete("/api/channels/sidecar/telegram")).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "body: {}",
        String::from_utf8_lossy(&body)
    );
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["status"], "removed");

    let root = std::fs::read_to_string(&config_path).expect("root config still present");
    let included = std::fs::read_to_string(&included_path).expect("included file still present");
    assert!(
        !root.contains("[[sidecar_channels]]") && !root.contains("name = \"telegram\""),
        "block must be gone from the root config: {root}"
    );
    assert!(
        !included.contains("[[sidecar_channels]]") && !included.contains("name = \"telegram\""),
        "block must be gone from the included file: {included}"
    );

    let (status, _) = send(h.app.clone(), auth_delete("/api/channels/sidecar/telegram")).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "second delete must 404");
}
