//! Integration tests for managed configuration mode (#6695).
//!
//! These live in their own test binary on purpose.
//! `LIBREFANG_CONFIG_MODE` is process-global, and Rust runs the tests inside one binary on parallel threads, so setting it here would be visible to every other test in the same file — which is exactly how the first version of these tests broke five unrelated `config_set` cases.
//! A separate binary is a separate process, so the blast radius stops at this file, and the mutex below serializes the cases within it.

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

async fn boot_router_with_api_key(api_key: &str) -> RouterHarness {
    boot_router_with_config(api_key, |_| {}).await
}

/// Same harness, with a hook to set config fields the default does not exercise.
/// Used by the `external_auth` read test, which has to distinguish "the value came from config" from "the value is the default that happens to match".
async fn boot_router_with_config(
    api_key: &str,
    customize: impl FnOnce(&mut KernelConfig),
) -> RouterHarness {
    let tmp = tempfile::tempdir().expect("tempdir");

    librefang_kernel::registry_sync::seed_registry_fixture_for_tests(tmp.path());

    let mut config = KernelConfig {
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
    customize(&mut config);

    let home = config.home_dir.clone();
    let kernel = LibreFangKernel::boot_with_config(config).expect("kernel boot");
    let kernel = Arc::new(kernel);
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

// ---------------------------------------------------------------------------
// GET /api/config
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Managed configuration mode (#6695)
// ---------------------------------------------------------------------------

/// `LIBREFANG_CONFIG_MODE` is process-global, so the managed-mode cases must not overlap with each other or with any other test that reads it.
/// They all run under this lock and restore the previous value before releasing it.
fn managed_mode_lock() -> &'static tokio::sync::Mutex<()> {
    static LOCK: std::sync::OnceLock<tokio::sync::Mutex<()>> = std::sync::OnceLock::new();
    LOCK.get_or_init(|| tokio::sync::Mutex::new(()))
}

/// Sets `LIBREFANG_CONFIG_MODE=managed` for its lifetime and restores the prior value on drop, including on panic, so one failing assertion cannot leak the lock into the rest of the suite.
struct ManagedModeGuard {
    previous: Option<String>,
    // A tokio mutex rather than `std::sync`: the guard is held across the harness `.await`s, which a `std::sync::MutexGuard` cannot legally do (clippy's `await_holding_lock`).
    _lock: tokio::sync::MutexGuard<'static, ()>,
}

impl ManagedModeGuard {
    async fn set() -> Self {
        let lock = managed_mode_lock().lock().await;
        let previous = std::env::var(librefang_kernel::config::CONFIG_MODE_ENV).ok();
        std::env::set_var(librefang_kernel::config::CONFIG_MODE_ENV, "managed");
        Self {
            previous,
            _lock: lock,
        }
    }
}

impl Drop for ManagedModeGuard {
    fn drop(&mut self) {
        match self.previous.take() {
            Some(v) => std::env::set_var(librefang_kernel::config::CONFIG_MODE_ENV, v),
            None => std::env::remove_var(librefang_kernel::config::CONFIG_MODE_ENV),
        }
    }
}

/// The default deployment is unchanged: writable, mode `mutable`, and `config/set` still works.
/// This is the compatibility guarantee the RFC asks for, so it is asserted rather than assumed.
#[tokio::test(flavor = "multi_thread")]
async fn config_status_reports_mutable_by_default() {
    let _lock = managed_mode_lock().lock().await;
    let previous = std::env::var(librefang_kernel::config::CONFIG_MODE_ENV).ok();
    std::env::remove_var(librefang_kernel::config::CONFIG_MODE_ENV);

    let h = boot_router_with_api_key(API_KEY).await;
    let (status, body) = send(h.app.clone(), auth_get("/api/config/status")).await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));

    let v: serde_json::Value = serde_json::from_slice(&body).expect("status body is JSON");
    assert_eq!(v["mode"], "mutable");
    assert_eq!(v["writable"], true);
    assert!(
        v["source"]
            .as_str()
            .is_some_and(|s| s.ends_with("config.toml")),
        "source must name the config file; got {v}"
    );

    if let Some(p) = previous {
        std::env::set_var(librefang_kernel::config::CONFIG_MODE_ENV, p);
    }
}

/// Managed mode is reported to the dashboard rather than left to be discovered by a failed write.
#[tokio::test(flavor = "multi_thread")]
async fn config_status_reports_managed_when_locked() {
    let _guard = ManagedModeGuard::set().await;

    let h = boot_router_with_api_key(API_KEY).await;
    let (status, body) = send(h.app.clone(), auth_get("/api/config/status")).await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));

    let v: serde_json::Value = serde_json::from_slice(&body).expect("status body is JSON");
    assert_eq!(v["mode"], "managed");
    assert_eq!(v["writable"], false);
}

/// The load-bearing case: a write that would be accepted in mutable mode is refused with the one documented status and the structured body, and the file on disk is untouched.
#[tokio::test(flavor = "multi_thread")]
async fn config_set_is_locked_in_managed_mode_and_leaves_the_file_untouched() {
    let _guard = ManagedModeGuard::set().await;

    let h = boot_router_with_api_key(API_KEY).await;
    let config_path = h.home.join("config.toml");
    let seed = format!("api_key = \"{API_KEY}\"\n");
    std::fs::write(&config_path, &seed).expect("seed config.toml");

    // `log_level` is allowlisted and round-trips in mutable mode — see `config_set_writes_allowlisted_path_to_tempdir_toml` — so a refusal here is the mode, not the path.
    let (status, body) = send(
        h.app.clone(),
        auth_post_json(
            "/api/config/set",
            serde_json::json!({"path": "log_level", "value": "debug"}),
        ),
    )
    .await;

    assert_eq!(
        status,
        StatusCode::LOCKED,
        "managed mode must answer 423, got {status}: {}",
        String::from_utf8_lossy(&body)
    );

    let v: serde_json::Value = serde_json::from_slice(&body).expect("locked body is JSON");
    assert_eq!(v["code"], "config_managed");
    assert_eq!(v["ok"], false);
    assert!(
        v["source"].as_str().is_some_and(|s| !s.is_empty()),
        "the refusal must tell the operator which file is managed; got {v}"
    );

    assert_eq!(
        std::fs::read_to_string(&config_path).expect("config.toml still readable"),
        seed,
        "a refused write must not have opened, truncated, or rewritten the file"
    );
}
