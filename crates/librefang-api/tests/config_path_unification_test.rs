//! The daemon resolves `config.toml` exactly once, and every surface reads that one answer (#6695).
//!
//! Before this, the kernel loaded the file through `default_config_path()` — which honours `LIBREFANG_CONFIG_PATH` — while the API layer re-derived it as `home_dir().join("config.toml")`.
//! With the two in agreement only by coincidence, a Kubernetes ConfigMap mounted outside `LIBREFANG_HOME` produced a daemon that read the manifest and wrote somewhere else, so `GET /api/config/status` named a file that `POST /api/config/set` never touched.
//!
//! These tests live in their own binary because they set `LIBREFANG_HOME` and `LIBREFANG_CONFIG_PATH`, which are process-global.
//! A separate binary is a separate process, so the blast radius stops at this file, and the mutex below serializes the cases within it.

use axum::body::{to_bytes, Body};
use axum::http::{header, Method, Request, StatusCode};
use librefang_api::server;
use librefang_kernel::LibreFangKernel;
use librefang_types::config::{DefaultModelConfig, KernelConfig};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tower::ServiceExt;

const API_KEY: &str = "test-secret-key";

fn env_lock() -> &'static tokio::sync::Mutex<()> {
    static LOCK: std::sync::OnceLock<tokio::sync::Mutex<()>> = std::sync::OnceLock::new();
    LOCK.get_or_init(|| tokio::sync::Mutex::new(()))
}

/// Points `LIBREFANG_HOME` (and optionally `LIBREFANG_CONFIG_PATH`) at test-owned directories for its lifetime, restoring the previous values on drop — including on panic, so one failing assertion cannot leak the lock into the rest of the file.
struct EnvGuard {
    previous_home: Option<String>,
    previous_config_path: Option<String>,
    // A tokio mutex rather than `std::sync`: the guard is held across the harness `.await`s, which a `std::sync::MutexGuard` cannot legally do (clippy's `await_holding_lock`).
    _lock: tokio::sync::MutexGuard<'static, ()>,
}

impl EnvGuard {
    async fn set(home: &Path, config_path: Option<&Path>) -> Self {
        let lock = env_lock().lock().await;
        let previous_home = std::env::var("LIBREFANG_HOME").ok();
        let previous_config_path = std::env::var(librefang_kernel::config::CONFIG_PATH_ENV).ok();
        std::env::set_var("LIBREFANG_HOME", home);
        match config_path {
            Some(p) => std::env::set_var(librefang_kernel::config::CONFIG_PATH_ENV, p),
            None => std::env::remove_var(librefang_kernel::config::CONFIG_PATH_ENV),
        }
        Self {
            previous_home,
            previous_config_path,
            _lock: lock,
        }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        match self.previous_home.take() {
            Some(v) => std::env::set_var("LIBREFANG_HOME", v),
            None => std::env::remove_var("LIBREFANG_HOME"),
        }
        match self.previous_config_path.take() {
            Some(v) => std::env::set_var(librefang_kernel::config::CONFIG_PATH_ENV, v),
            None => std::env::remove_var(librefang_kernel::config::CONFIG_PATH_ENV),
        }
    }
}

struct RouterHarness {
    app: axum::Router,
    kernel: Arc<LibreFangKernel>,
}

impl Drop for RouterHarness {
    fn drop(&mut self) {
        self.kernel.shutdown();
    }
}

/// Serialise a bootable config to `config_path` and boot the daemon the way production does: `LibreFangKernel::boot(None)`, which resolves the path from the environment rather than being handed one.
async fn boot_router(home: &Path, config_path: &Path) -> RouterHarness {
    librefang_kernel::registry_sync::seed_registry_fixture_for_tests(home);

    let config = KernelConfig {
        home_dir: home.to_path_buf(),
        data_dir: home.join("data"),
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
    if let Some(parent) = config_path.parent() {
        std::fs::create_dir_all(parent).expect("create config dir");
    }
    std::fs::write(
        config_path,
        toml::to_string_pretty(&config).expect("serialize config"),
    )
    .expect("write config");

    let kernel = Arc::new(LibreFangKernel::boot(None).expect("kernel boot"));
    kernel.set_self_handle();

    let (app, _state) =
        server::build_router(kernel.clone(), "127.0.0.1:0".parse().expect("addr")).await;

    RouterHarness { app, kernel }
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

fn auth_get(path: &str) -> Request<Body> {
    Request::builder()
        .method(Method::GET)
        .uri(path)
        .header(header::AUTHORIZATION, format!("Bearer {API_KEY}"))
        .body(Body::empty())
        .unwrap()
}

fn auth_post_json(path: &str, body: serde_json::Value) -> Request<Body> {
    Request::builder()
        .method(Method::POST)
        .uri(path)
        .header(header::AUTHORIZATION, format!("Bearer {API_KEY}"))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_string()))
        .unwrap()
}

/// Read `source` out of `GET /api/config/status` — the API layer's answer to "which file is the configuration".
async fn api_source(app: &axum::Router) -> PathBuf {
    let (status, body) = send(app.clone(), auth_get("/api/config/status")).await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let v: serde_json::Value = serde_json::from_slice(&body).expect("status body is JSON");
    PathBuf::from(
        v["source"]
            .as_str()
            .unwrap_or_else(|| panic!("status body has no `source`: {v}")),
    )
}

/// Under a relocated `LIBREFANG_HOME` the kernel and the API must name the same file — byte for byte, not merely "something ending in config.toml".
#[tokio::test(flavor = "multi_thread")]
async fn api_and_kernel_agree_on_config_path_under_relocated_home() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let home = tmp.path().to_path_buf();
    let _guard = EnvGuard::set(&home, None).await;

    let config_path = home.join("config.toml");
    let h = boot_router(&home, &config_path).await;

    assert_eq!(
        h.kernel.config_path(),
        config_path.as_path(),
        "the kernel must record the path it actually loaded"
    );
    assert_eq!(
        api_source(&h.app).await,
        config_path,
        "GET /api/config/status must report the kernel's path, not a second resolution"
    );
}

/// `LIBREFANG_CONFIG_PATH` relocates the file out of the home directory entirely — the shape a ConfigMap mount takes.
///
/// The filename is deliberately not `config.toml`: the write path used to reject anything else as an "invalid config file path", a check that was meaningful when the path came from a request and was pure obstruction once it came from the operator's own environment.
#[tokio::test(flavor = "multi_thread")]
async fn config_set_writes_the_file_the_daemon_loaded() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let home = tmp.path().join("home");
    let mount = tmp.path().join("etc-librefang");
    std::fs::create_dir_all(&home).expect("create home");
    let config_path = mount.join("librefang.toml");
    let _guard = EnvGuard::set(&home, Some(&config_path)).await;

    let h = boot_router(&home, &config_path).await;

    assert_eq!(
        h.kernel.config_path(),
        config_path.as_path(),
        "LIBREFANG_CONFIG_PATH must survive onto the kernel"
    );
    assert_eq!(api_source(&h.app).await, config_path);

    // `language` rather than, say, `log_level`: it is hot-reloadable without a `LogLevelReloader`, so the live value moving is evidence that the reload re-read the relocated file rather than evidence about the reload classification.
    let (status, body) = send(
        h.app.clone(),
        auth_post_json(
            "/api/config/set",
            serde_json::json!({"path": "language", "value": "ja"}),
        ),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "config/set must accept a relocated config file: {}",
        String::from_utf8_lossy(&body)
    );

    let written = std::fs::read_to_string(&config_path).expect("mounted config still readable");
    assert!(
        written.contains("language = \"ja\""),
        "the write must land in the file the daemon loaded; got:\n{written}"
    );
    assert!(
        !home.join("config.toml").exists(),
        "nothing may be written to $LIBREFANG_HOME/config.toml — that is the split this fix closes"
    );

    // And the round trip closes: the reload after the write re-read the same file.
    assert_eq!(
        h.kernel.config_ref().language,
        "ja",
        "the reload after a write must have re-read the relocated file"
    );
}
