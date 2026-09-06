//! Regression tests for the live per-user bearer table (`AppState.user_api_keys`) — the only credential list `middleware::auth` compares an `Authorization: Bearer` against.
//!
//! Its invariant is that the table equals the `[[users]]` entries carrying an `api_key_hash` plus every paired device, and the two halves used to drift in opposite directions:
//! `POST /api/config/reload` never rebuilt the table at all, so a bearer revoked by deleting its `[[users]]` block kept authenticating every REST request until the daemon restarted — even though the same edit took effect immediately on the WS and terminal upgrades, which re-derive their table per connection.
//! Any `/api/users` write rebuilt the table from config alone, so an unrelated user or group edit silently de-authenticated every paired mobile device while `GET /api/pairing/devices` went on listing them as paired.
//!
//! Both tests drive the full router (`server::build_router`) rather than a bare domain router: the defect lives in the wiring between a handler and the shared table the auth middleware reads, and a router mounted without that middleware layer cannot observe it.
//! Everything is tempdir-backed (`config.home_dir` = tempdir), so the reloads and `config.toml` rewrites here never touch the real `~/.librefang/config.toml`.

use axum::body::{to_bytes, Body};
use axum::http::{header, Method, Request, StatusCode};
use librefang_api::server;
use librefang_kernel::LibreFangKernel;
use librefang_types::config::{
    DefaultModelConfig, KernelConfig, PairingConfig, ReloadConfig, ReloadMode, UserConfig,
};
use std::sync::Arc;
use tower::ServiceExt;

const MASTER_KEY: &str = "master-secret-key";

struct Harness {
    app: axum::Router,
    home: std::path::PathBuf,
    /// The config the kernel booted with, kept so a test can edit one section and rewrite `config.toml` the way an operator would.
    config: KernelConfig,
    _tmp: tempfile::TempDir,
    state: Arc<librefang_api::routes::AppState>,
}

impl Drop for Harness {
    fn drop(&mut self) {
        self.state.kernel.shutdown();
    }
}

/// Serialize a whole `KernelConfig` to the harness's `config.toml`.
///
/// The file is seeded from the very config the kernel booted with, because every reload in these tests re-reads it: a `config.toml` missing `api_key` would answer the follow-up requests under different auth rules than the ones under test, and one missing `home_dir` would let the reloaded config point at the real daemon home.
fn write_config(home: &std::path::Path, config: &KernelConfig) {
    let rendered = toml::to_string_pretty(config).expect("serialize config");
    std::fs::write(home.join("config.toml"), rendered).expect("write config.toml");
}

/// Boot a real kernel over a tempdir home, seed `config.toml` from that same config, and mount the full router with its auth middleware.
async fn boot(customize: impl FnOnce(&mut KernelConfig)) -> Harness {
    let tmp = tempfile::tempdir().expect("tempdir");

    librefang_kernel::registry_sync::seed_registry_fixture_for_tests(tmp.path());

    let mut config = KernelConfig {
        home_dir: tmp.path().to_path_buf(),
        data_dir: tmp.path().join("data"),
        api_key: MASTER_KEY.to_string(),
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
    write_config(&home, &config);

    let kernel = Arc::new(LibreFangKernel::boot_with_config(config.clone()).expect("kernel boot"));
    kernel.set_self_handle();

    let (app, state) = server::build_router(kernel, "127.0.0.1:0".parse().expect("addr")).await;

    Harness {
        app,
        home,
        config,
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

fn bearer_get(path: &str, token: &str) -> Request<Body> {
    Request::builder()
        .method(Method::GET)
        .uri(path)
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap()
}

fn bearer_post(path: &str, token: &str, body: Option<serde_json::Value>) -> Request<Body> {
    let builder = Request::builder()
        .method(Method::POST)
        .uri(path)
        .header(header::AUTHORIZATION, format!("Bearer {token}"));
    match body {
        Some(value) => builder
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(serde_json::to_vec(&value).unwrap()))
            .unwrap(),
        None => builder.body(Body::empty()).unwrap(),
    }
}

fn anon_post(path: &str, body: serde_json::Value) -> Request<Body> {
    Request::builder()
        .method(Method::POST)
        .uri(path)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap()
}

fn json(bytes: &[u8]) -> serde_json::Value {
    serde_json::from_slice(bytes).unwrap_or(serde_json::Value::Null)
}

/// Revoking a per-user API key the documented way — delete the `[[users]]` block from `config.toml`, `POST /api/config/reload` — must stop that bearer authenticating over REST.
///
/// Before the fix the reload returned 200, the WS and terminal upgrades honoured the revocation (they call `configured_user_api_keys(&auth_snapshot())` per connection), and every `/api/*` request kept accepting the deleted bearer with its full role until the daemon was restarted.
#[tokio::test(flavor = "multi_thread")]
async fn config_reload_revokes_a_user_bearer_deleted_from_config_toml() {
    const ALICE_BEARER: &str = "alice-bearer-token";
    let alice_hash = librefang_api::password_hash::hash_device_token(ALICE_BEARER);

    let h = boot(move |cfg| {
        cfg.users = vec![UserConfig {
            name: "alice".to_string(),
            role: "admin".to_string(),
            api_key_hash: Some(alice_hash),
            ..UserConfig::default()
        }];
    })
    .await;

    // Baseline. Without it a post-reload 401 could just mean the fixture never authenticated in the first place.
    let (status, body) = send(h.app.clone(), bearer_get("/api/users", ALICE_BEARER)).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "the seeded [[users]] bearer must authenticate before the revocation; got: {}",
        String::from_utf8_lossy(&body)
    );

    // The operator's revocation: alice's block leaves `config.toml`, then the daemon is asked to reload.
    let mut revoked = h.config.clone();
    revoked.users.clear();
    write_config(&h.home, &revoked);

    let (status, body) = send(
        h.app.clone(),
        bearer_post("/api/config/reload", MASTER_KEY, None),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "reload must succeed; got: {}",
        String::from_utf8_lossy(&body)
    );

    let (status, _) = send(h.app.clone(), bearer_get("/api/users", ALICE_BEARER)).await;
    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "a bearer whose [[users]] block was deleted must stop authenticating the moment the reload \
         succeeds — otherwise revoking an offboarded or leaked key needs a daemon restart"
    );

    // The same reload must not have taken the master credential down with it.
    let (status, _) = send(h.app.clone(), bearer_get("/api/users", MASTER_KEY)).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "the master api_key must survive the reload that revoked the per-user bearer"
    );
}

/// A paired mobile device's bearer must survive a `/api/users` write it has nothing to do with.
///
/// Before the fix `persist_identity_sections` replaced the whole table with the config-derived rows, and `build_api_user_records` can only produce `[[users]]` entries — so creating one user (or editing a group, or rotating someone's key) dropped every `device:{id}` row and 401'd every paired phone until the daemon restarted, with no log line and no visible change in `GET /api/pairing/devices`.
#[tokio::test(flavor = "multi_thread")]
async fn users_write_keeps_paired_device_bearer_authenticating() {
    let h = boot(|cfg| {
        cfg.pairing = PairingConfig {
            enabled: true,
            public_base_url: Some("https://daemon.example.com".to_string()),
            ..PairingConfig::default()
        };
    })
    .await;

    let (status, body) = send(
        h.app.clone(),
        bearer_post("/api/pairing/request", MASTER_KEY, None),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "pairing request failed: {}",
        String::from_utf8_lossy(&body)
    );
    let token = json(&body)["token"]
        .as_str()
        .expect("pairing token")
        .to_string();

    let (status, body) = send(
        h.app.clone(),
        anon_post(
            "/api/pairing/complete",
            serde_json::json!({"token": token, "display_name": "iPhone", "platform": "ios"}),
        ),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "pairing completion failed: {}",
        String::from_utf8_lossy(&body)
    );
    let device_bearer = json(&body)["api_key"]
        .as_str()
        .expect("device bearer")
        .to_string();

    let (status, body) = send(h.app.clone(), bearer_get("/api/users", &device_bearer)).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "a freshly paired device must authenticate; got: {}",
        String::from_utf8_lossy(&body)
    );

    // A user-management write that says nothing about devices.
    let (status, body) = send(
        h.app.clone(),
        bearer_post(
            "/api/users",
            MASTER_KEY,
            Some(serde_json::json!({"name": "bob", "role": "user"})),
        ),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "creating a user failed: {}",
        String::from_utf8_lossy(&body)
    );

    let (status, _) = send(h.app.clone(), bearer_get("/api/users", &device_bearer)).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "the paired device's bearer must still authenticate after an unrelated /api/users write — \
         a table rebuilt from config alone drops every device row and de-authenticates every \
         paired phone until the daemon restarts"
    );
}

/// The other direction of the same invariant: a reload the kernel declined to apply must not republish the table either.
///
/// `POST /api/users/{name}/rotate-key` writes the new hash to `config.toml` and publishes the new table immediately, without waiting for the reload to be honoured — which is what keeps a revocation from surviving a failed reload.
/// Under `[reload] mode = "off"` the kernel deliberately never swaps the freshly-read config in, so `auth_snapshot()` goes on answering with the boot-time `[[users]]` and the pre-rotation hash.
/// A reload path that rebuilt the bearer table unconditionally would therefore read that stale snapshot and put the rotated-away key back into service — turning the fix for the revocation hole into a resurrection hole.
/// `refresh_auth_tables` is gated on `ReloadPlan::config_stored`, the kernel's own record of whether the swap happened, and this test is what holds that gate in place.
#[tokio::test(flavor = "multi_thread")]
async fn reload_that_the_kernel_did_not_apply_does_not_resurrect_a_rotated_key() {
    const OLD_BEARER: &str = "carol-old-bearer-token";
    let carol_hash = librefang_api::password_hash::hash_device_token(OLD_BEARER);

    let h = boot(move |cfg| {
        cfg.reload = ReloadConfig {
            mode: ReloadMode::Off,
            ..ReloadConfig::default()
        };
        cfg.users = vec![UserConfig {
            name: "carol".to_string(),
            role: "admin".to_string(),
            api_key_hash: Some(carol_hash),
            ..UserConfig::default()
        }];
    })
    .await;

    let (status, body) = send(h.app.clone(), bearer_get("/api/users", OLD_BEARER)).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "the seeded [[users]] bearer must authenticate before the rotation; got: {}",
        String::from_utf8_lossy(&body)
    );

    let (status, body) = send(
        h.app.clone(),
        bearer_post("/api/users/carol/rotate-key", MASTER_KEY, None),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "rotating carol's key failed: {}",
        String::from_utf8_lossy(&body)
    );

    let (status, _) = send(h.app.clone(), bearer_get("/api/users", OLD_BEARER)).await;
    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "rotation must revoke the old bearer immediately, without waiting for a reload the mode forbids"
    );

    let (status, body) = send(
        h.app.clone(),
        bearer_post("/api/config/reload", MASTER_KEY, None),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "reload must still answer 200 under mode = \"off\"; got: {}",
        String::from_utf8_lossy(&body)
    );
    let body = json(&body);
    assert_eq!(
        body["config_applied"],
        serde_json::json!(false),
        "mode = \"off\" must report that nothing was applied: {body}"
    );
    assert_eq!(
        body["hot_actions_applied"],
        serde_json::json!([]),
        "hot_actions_applied must name only actions that ran, and under mode = \"off\" none did: {body}"
    );

    let (status, _) = send(h.app.clone(), bearer_get("/api/users", OLD_BEARER)).await;
    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "a reload the kernel declined to apply must not rebuild the bearer table from the stale \
         config it refused to swap in — that would resurrect the key the rotation just revoked"
    );
}
