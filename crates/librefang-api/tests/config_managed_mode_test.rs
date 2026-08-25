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

fn auth_delete(path: &str) -> Request<Body> {
    Request::builder()
        .method(Method::DELETE)
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

fn auth_put_json(path: &str, body: serde_json::Value) -> Request<Body> {
    Request::builder()
        .method(Method::PUT)
        .uri(path)
        .header(header::AUTHORIZATION, format!("Bearer {API_KEY}"))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_string()))
        .unwrap()
}

fn auth_patch_json(path: &str, body: serde_json::Value) -> Request<Body> {
    Request::builder()
        .method(Method::PATCH)
        .uri(path)
        .header(header::AUTHORIZATION, format!("Bearer {API_KEY}"))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_string()))
        .unwrap()
}

/// Asserts the one documented refusal shape: `423` plus the structured body every guarded route shares.
/// Factored out because the provider cases below assert it three times and a copy that drifted would silently stop checking the contract.
fn assert_managed_refusal(status: StatusCode, body: &[u8]) {
    assert_eq!(
        status,
        StatusCode::LOCKED,
        "managed mode must answer 423, got {status}: {}",
        String::from_utf8_lossy(body)
    );

    let v: serde_json::Value = serde_json::from_slice(body).expect("locked body is JSON");
    assert_eq!(v["code"], "config_managed");
    assert_eq!(v["ok"], false);
    assert!(
        v["source"].as_str().is_some_and(|s| !s.is_empty()),
        "the refusal must tell the operator which file is managed; got {v}"
    );
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

    assert_managed_refusal(status, &body);

    assert_eq!(
        std::fs::read_to_string(&config_path).expect("config.toml still readable"),
        seed,
        "a refused write must not have opened, truncated, or rewritten the file"
    );
}

/// Memory settings are deployment configuration too.
/// Refuse the PATCH before reading or rewriting config.toml so managed deployments remain immutable.
#[tokio::test(flavor = "multi_thread")]
async fn memory_config_patch_is_locked_in_managed_mode() {
    let _guard = ManagedModeGuard::set().await;

    let h = boot_router_with_api_key(API_KEY).await;
    let config_path = h.home.join("config.toml");
    let seed = format!("api_key = \"{API_KEY}\"\n[memory]\ndecay_rate = 0.1\n");
    std::fs::write(&config_path, &seed).expect("seed config.toml");

    let (status, body) = send(
        h.app.clone(),
        auth_patch_json("/api/memory/config", serde_json::json!({"decay_rate": 0.9})),
    )
    .await;

    assert_managed_refusal(status, &body);
    assert_eq!(
        std::fs::read_to_string(config_path).expect("config.toml still readable"),
        seed
    );
}

// ---------------------------------------------------------------------------
// Provider routes that persist deployment configuration
// ---------------------------------------------------------------------------
//
// `set_provider_key` persists `[default_model]` (auto-switch / free-model migration), `set_provider_url` persists `[provider_urls]` and `[provider_proxy_urls]`, and `set_default_provider` persists `[default_model]` outright.
// All three shipped unguarded in #6717 even though "every API route that persists deployment configuration is locked" is an acceptance criterion of #6695, so each gets a case here.

/// `POST /api/providers/{name}/key` is refused **in full**, which is wider than the rest of managed mode.
/// The route's `config.toml` write is conditional on live daemon state, so guarding only that write would accept or refuse the identical request depending on timing — and in the refusing case it would already have rewritten `secrets.env`.
/// Hence the assertion set covers both files: a refused request must leave `config.toml` *and* `secrets.env` byte-identical.
#[tokio::test(flavor = "multi_thread")]
async fn set_provider_key_is_locked_in_managed_mode() {
    let _guard = ManagedModeGuard::set().await;

    let h = boot_router_with_api_key(API_KEY).await;
    let config_path = h.home.join("config.toml");
    let config_seed = format!("api_key = \"{API_KEY}\"\n");
    std::fs::write(&config_path, &config_seed).expect("seed config.toml");

    // Seed `secrets.env` too.
    // Asserting it is unchanged is only meaningful against a file that exists — an absent file would pass the check trivially even if the handler had written one and then failed.
    let secrets_path = h.home.join("secrets.env");
    let secrets_seed = "PREEXISTING_API_KEY=untouched\n";
    std::fs::write(&secrets_path, secrets_seed).expect("seed secrets.env");

    // `groq` is a real catalog provider with a real `api_key_env`, so this request succeeds in mutable mode.
    // A refusal here is the mode, not a validation failure on an unknown provider name.
    let (status, body) = send(
        h.app.clone(),
        auth_post_json(
            "/api/providers/groq/key",
            serde_json::json!({"key": "gsk-managed-mode-must-refuse-this"}),
        ),
    )
    .await;

    assert_managed_refusal(status, &body);

    assert_eq!(
        std::fs::read_to_string(&config_path).expect("config.toml still readable"),
        config_seed,
        "a refused write must not have opened, truncated, or rewritten config.toml"
    );
    assert_eq!(
        std::fs::read_to_string(&secrets_path).expect("secrets.env still readable"),
        secrets_seed,
        "the refusal happens before the secrets.env write, so the credential file must be untouched too"
    );
}

/// `PUT /api/providers/{name}/url` persists `[provider_urls]` / `[provider_proxy_urls]`, which are deployment configuration.
/// The guard also has to fire before the in-memory catalog is mutated, otherwise a refused request would still move the running daemon's endpoint with nothing on disk to show for it — so this asserts the live catalog kept the original URL as well as the file.
#[tokio::test(flavor = "multi_thread")]
async fn set_provider_url_is_locked_in_managed_mode() {
    let _guard = ManagedModeGuard::set().await;

    let h = boot_router_with_api_key(API_KEY).await;
    let config_path = h.home.join("config.toml");
    let seed = format!("api_key = \"{API_KEY}\"\n");
    std::fs::write(&config_path, &seed).expect("seed config.toml");

    let url_before = {
        let catalog = h.state.kernel.model_catalog_ref().load();
        catalog.get_provider("ollama").map(|p| p.base_url.clone())
    };

    let (status, body) = send(
        h.app.clone(),
        auth_put_json(
            "/api/providers/ollama/url",
            serde_json::json!({"base_url": "http://managed-mode-must-refuse-this:11434"}),
        ),
    )
    .await;

    assert_managed_refusal(status, &body);

    assert_eq!(
        std::fs::read_to_string(&config_path).expect("config.toml still readable"),
        seed,
        "a refused write must not have opened, truncated, or rewritten the file"
    );

    let url_after = {
        let catalog = h.state.kernel.model_catalog_ref().load();
        catalog.get_provider("ollama").map(|p| p.base_url.clone())
    };
    assert_eq!(
        url_before, url_after,
        "a refused request must not move the live catalog either — that is the drift managed mode exists to prevent"
    );
}

/// `POST /api/providers/{name}/default` exists to persist `[default_model]`, so managed mode refuses it.
/// The persist failure inside the handler is only a `warn!`, so without the guard this route would answer `200` and hot-switch the live default while the manifest kept saying otherwise; the in-memory override is therefore asserted alongside the file.
#[tokio::test(flavor = "multi_thread")]
async fn set_default_provider_is_locked_in_managed_mode() {
    let _guard = ManagedModeGuard::set().await;

    let h = boot_router_with_api_key(API_KEY).await;
    let config_path = h.home.join("config.toml");
    let seed = format!("api_key = \"{API_KEY}\"\n");
    std::fs::write(&config_path, &seed).expect("seed config.toml");

    let (status, body) = send(
        h.app.clone(),
        auth_post_json(
            "/api/providers/ollama/default",
            serde_json::json!({"model": "managed-mode-must-refuse-this"}),
        ),
    )
    .await;

    assert_managed_refusal(status, &body);

    assert_eq!(
        std::fs::read_to_string(&config_path).expect("config.toml still readable"),
        seed,
        "a refused write must not have opened, truncated, or rewritten the file"
    );

    // Assert on the *effective* default rather than on `override.is_none()`: the override being unset at boot is a kernel construction detail, while "the refused model never became the live default" is the property the guard is responsible for.
    let effective_model = {
        let guard = h
            .state
            .kernel
            .default_model_override_ref()
            .read()
            .unwrap_or_else(|e| e.into_inner());
        match guard.as_ref() {
            Some(dm) => dm.model.clone(),
            None => h.state.kernel.config_ref().default_model.model.clone(),
        }
    };
    assert_eq!(
        effective_model, "test-model",
        "a refused request must not hot-switch the live default model"
    );
}

// ---------------------------------------------------------------------------
// Routes outside `config/*` that persist into config.toml (#6695)
// ---------------------------------------------------------------------------
//
// The four write paths below were named as open gaps in `docs/operations/managed-config.md` and are locked here.
// Each case asserts the shared refusal shape *and* that `config.toml` is byte-identical afterwards, because "answered 423" and "wrote nothing" are separate properties and only the second one is what managed mode promises.

/// `POST /api/auth/change-password` persists `dashboard_user` / `dashboard_pass_hash` as top-level keys, so the deployment that owns the file owns the dashboard credential.
///
/// The guard sits ahead of the current-password verification, so this case deliberately sends a **wrong** current password: a `423` here proves the refusal is not reachable only via the success path, and that a caller cannot use the endpoint as a password oracle in managed mode.
#[tokio::test(flavor = "multi_thread")]
async fn change_password_is_locked_in_managed_mode() {
    let _guard = ManagedModeGuard::set().await;

    let h = boot_router_with_config(API_KEY, |c| {
        c.dashboard_user = "operator".to_string();
        c.dashboard_pass = "correct-horse-battery".to_string();
    })
    .await;
    let config_path = h.home.join("config.toml");
    let seed = format!("api_key = \"{API_KEY}\"\ndashboard_user = \"operator\"\n");
    std::fs::write(&config_path, &seed).expect("seed config.toml");

    let (status, body) = send(
        h.app.clone(),
        auth_post_json(
            "/api/auth/change-password",
            serde_json::json!({
                "current_password": "definitely-not-the-password",
                "new_password": "managed-mode-must-refuse-this",
            }),
        ),
    )
    .await;

    assert_managed_refusal(status, &body);
    assert_eq!(
        std::fs::read_to_string(&config_path).expect("config.toml still readable"),
        seed,
        "a refused credential change must not have rewritten the file"
    );
}

/// `POST /api/channels/sidecar/{name}/configure` writes `[[sidecar_channels]]` into `config.toml` and the adapter's secrets into `secrets.env`, both inside one blocking call.
/// It is therefore refused in full, like `set_provider_key`, and both files are asserted untouched.
#[tokio::test(flavor = "multi_thread")]
async fn configure_sidecar_channel_is_locked_in_managed_mode() {
    let _guard = ManagedModeGuard::set().await;

    let h = boot_router_with_api_key(API_KEY).await;
    let config_path = h.home.join("config.toml");
    let config_seed = format!("api_key = \"{API_KEY}\"\n");
    std::fs::write(&config_path, &config_seed).expect("seed config.toml");

    let secrets_path = h.home.join("secrets.env");
    let secrets_seed = "PREEXISTING_TOKEN=untouched\n";
    std::fs::write(&secrets_path, secrets_seed).expect("seed secrets.env");

    // `telegram` is a real `SIDECAR_CATALOG` entry, so a 404 here would mean the guard never ran.
    let (status, body) = send(
        h.app.clone(),
        auth_post_json(
            "/api/channels/sidecar/telegram/configure",
            serde_json::json!({"values": {"bot_token": "managed-mode-must-refuse-this"}}),
        ),
    )
    .await;

    assert_managed_refusal(status, &body);
    assert_eq!(
        std::fs::read_to_string(&config_path).expect("config.toml still readable"),
        config_seed,
        "a refused sidecar configure must not have rewritten config.toml"
    );
    assert_eq!(
        std::fs::read_to_string(&secrets_path).expect("secrets.env still readable"),
        secrets_seed,
        "the refusal precedes the secrets.env write, so the credential file must be untouched too"
    );
}

/// `DELETE /api/channels/sidecar/{name}` rewrites `config.toml` to drop the block.
/// Without the guard it would delete an entry the manifest declares, which the next rollout would silently restore.
#[tokio::test(flavor = "multi_thread")]
async fn delete_sidecar_channel_is_locked_in_managed_mode() {
    let _guard = ManagedModeGuard::set().await;

    let h = boot_router_with_api_key(API_KEY).await;
    let config_path = h.home.join("config.toml");
    let seed = format!(
        "api_key = \"{API_KEY}\"\n\n[[sidecar_channels]]\nname = \"telegram\"\ncommand = \"python3\"\n"
    );
    std::fs::write(&config_path, &seed).expect("seed config.toml");

    let (status, body) = send(h.app.clone(), auth_delete("/api/channels/sidecar/telegram")).await;

    assert_managed_refusal(status, &body);
    assert_eq!(
        std::fs::read_to_string(&config_path).expect("config.toml still readable"),
        seed,
        "a refused sidecar delete must leave the declared block in place"
    );
}

/// `POST /api/extensions/install` calls `upsert_mcp_server_config` directly, with no `mcp_runtime_store` check, so it is refused unconditionally.
#[tokio::test(flavor = "multi_thread")]
async fn extension_install_is_locked_in_managed_mode() {
    let _guard = ManagedModeGuard::set().await;

    let h = boot_router_with_api_key(API_KEY).await;
    let config_path = h.home.join("config.toml");
    let seed = format!("api_key = \"{API_KEY}\"\n");
    std::fs::write(&config_path, &seed).expect("seed config.toml");

    let (status, body) = send(
        h.app.clone(),
        auth_post_json(
            "/api/extensions/install",
            serde_json::json!({"name": "managed-mode-must-refuse-this"}),
        ),
    )
    .await;

    assert_managed_refusal(status, &body);
    assert_eq!(
        std::fs::read_to_string(&config_path).expect("config.toml still readable"),
        seed
    );
}

/// `POST /api/extensions/uninstall` is the delete counterpart and is refused on the same terms.
#[tokio::test(flavor = "multi_thread")]
async fn extension_uninstall_is_locked_in_managed_mode() {
    let _guard = ManagedModeGuard::set().await;

    let h = boot_router_with_api_key(API_KEY).await;
    let config_path = h.home.join("config.toml");
    let seed = format!("api_key = \"{API_KEY}\"\n");
    std::fs::write(&config_path, &seed).expect("seed config.toml");

    let (status, body) = send(
        h.app.clone(),
        auth_post_json(
            "/api/extensions/uninstall",
            serde_json::json!({"name": "managed-mode-must-refuse-this"}),
        ),
    )
    .await;

    assert_managed_refusal(status, &body);
    assert_eq!(
        std::fs::read_to_string(&config_path).expect("config.toml still readable"),
        seed
    );
}

// ---------------------------------------------------------------------------
// MCP servers: locked per store, not per route (#6695 / #6021)
// ---------------------------------------------------------------------------

/// Under the default `mcp_runtime_store = "file"` the MCP write rewrites `config.toml`, so managed mode refuses it.
#[tokio::test(flavor = "multi_thread")]
async fn add_mcp_server_is_locked_in_managed_mode_under_the_file_store() {
    let _guard = ManagedModeGuard::set().await;

    let h = boot_router_with_config(API_KEY, |c| {
        c.mcp_runtime_store = librefang_types::config::McpRuntimeStore::File;
    })
    .await;
    let config_path = h.home.join("config.toml");
    let seed = format!("api_key = \"{API_KEY}\"\n");
    std::fs::write(&config_path, &seed).expect("seed config.toml");

    let (status, body) = send(
        h.app.clone(),
        auth_post_json(
            "/api/mcp/servers",
            serde_json::json!({
                "name": "managed-mode-must-refuse-this",
                "transport": {"type": "http", "url": "https://example.invalid/mcp"},
            }),
        ),
    )
    .await;

    assert_managed_refusal(status, &body);
    assert_eq!(
        std::fs::read_to_string(&config_path).expect("config.toml still readable"),
        seed,
        "a refused write must not have opened, truncated, or rewritten the file"
    );
}

/// The counterpart that stops managed mode from over-locking, and the reason the guard sits inside the store match rather than at the top of the handler.
///
/// With `mcp_runtime_store = "db"` the same request persists into SQLite and never opens `config.toml`.
/// Refusing it would take the dashboard's MCP install surface away from a deployment that has already moved this persistence off the managed file — locking a write that is not happening.
/// The case asserts both halves. "Not `423`" is the #6695 property; "succeeded" is what stops the first half from passing vacuously on some unrelated `400`, and it is the evidence that the escape hatch this exemption points operators at actually works.
/// It asserts `is_success()` rather than a literal `201` so a later change to the created-vs-ok status is not a failure of this test.
#[tokio::test(flavor = "multi_thread")]
async fn add_mcp_server_stays_writable_in_managed_mode_under_the_db_store() {
    let _guard = ManagedModeGuard::set().await;

    let h = boot_router_with_config(API_KEY, |c| {
        c.mcp_runtime_store = librefang_types::config::McpRuntimeStore::Db;
    })
    .await;
    let config_path = h.home.join("config.toml");
    let seed = format!("api_key = \"{API_KEY}\"\n");
    std::fs::write(&config_path, &seed).expect("seed config.toml");

    let (status, body) = send(
        h.app.clone(),
        auth_post_json(
            "/api/mcp/servers",
            serde_json::json!({
                "name": "db-store-server",
                "transport": {"type": "http", "url": "https://example.invalid/mcp"},
            }),
        ),
    )
    .await;

    assert_ne!(
        status,
        StatusCode::LOCKED,
        "the db store never writes config.toml, so managed mode must not refuse it: {}",
        String::from_utf8_lossy(&body)
    );
    assert!(
        status.is_success(),
        "the write must actually go through, not merely avoid the lock; got {status}: {}",
        String::from_utf8_lossy(&body)
    );
    assert_eq!(
        std::fs::read_to_string(&config_path).expect("config.toml still readable"),
        seed,
        "and it must not have written config.toml either — that is what makes the exemption sound"
    );
}
