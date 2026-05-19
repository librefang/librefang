//! Integration tests for the `/api/channels/*` REST surface.
//!
//! Channels were called out in #3571 as part of the ~80% of HTTP routes
//! with no integration coverage. This file pins the read-mostly contract
//! plus the error-shape boundaries that the dashboard relies on:
//!
//! - `GET /api/channels` — list, with `total` / `configured_count`
//!   summary and the per-row `configured` flag flipping when a channel is
//!   seeded into `KernelConfig`.
//! - `GET /api/channels/{name}` — happy path round-trips registry
//!   metadata; unknown name returns the unified `ApiErrorResponse` 404.
//! - `GET /api/channels/registry` — file-system probe under
//!   `kernel.home_dir()/channels`; must return a valid JSON value (array
//!   or object) and never 500 on a missing dir.
//! - `POST /api/channels/{name}/configure` — validation surface only:
//!   404 for unknown channel, 400 when the JSON body is missing the
//!   required `fields` object. We deliberately do NOT exercise the
//!   happy path — it mutates `~/.librefang/secrets.env` and process-wide
//!   env vars (`std::env::set_var`), which would race with parallel tests.
//! - `DELETE /api/channels/{name}/configure` — 404 unknown channel.
//! - `POST /api/channels/{name}/test` — 404 unknown channel; for a known
//!   channel with no env credentials, returns 412 Precondition Failed
//!   with the unified `ApiErrorResponse` envelope (`{"error": "Missing
//!   required env vars: …"}`). Migrated from the legacy
//!   `{"status": "error", "message": …}` shape in #3505.

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use axum::Router;
use librefang_api::routes::{self, AppState};
use librefang_testing::{MockKernelBuilder, TestAppState};
use librefang_types::config::{ChannelsConfig, DiscordConfig, OneOrMany, SidecarChannelConfig};
use std::path::Path;
use std::sync::Arc;
use tower::ServiceExt;

/// Single test-wide serialisation lock for anything that touches
/// process-global state used by the `/api/channels/*` handlers:
///   1. `LIBREFANG_HOME` (via `std::env::set_var` — process-global) —
///      consumed by handlers added in #4865 that read `[channels]` from
///      disk under the `config_write_lock`, and by the sidecar
///      `/configure` flow which writes to `secrets.env` / `config.toml`.
///   2. The process-static sidecar schema cache (`SIDECAR_SCHEMA_CACHE`
///      in `routes::channels`) — the seeded-cache test would otherwise
///      race the empty-cache discovery tests and one would see the
///      other's `fields[]`.
///
/// Originally split as two mutexes (`ENV_LOCK` + `SIDECAR_CACHE_LOCK`),
/// which raced because both protected the same `LIBREFANG_HOME` env var
/// from different test paths (`DiskHomeGuard` vs `boot_with_temp_home`).
/// Consolidated into one lock so the invariant — "tests mutating
/// process-wide state run serially" — is enforced once at the source.
///
/// Tests that only exercise validation paths (unknown channel, missing
/// field) fail-fast before reaching `librefang_home()` and don't need
/// to hold this; they can run in parallel as before.
static CHANNELS_PROCESS_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Drop guard that points `LIBREFANG_HOME` at a tempdir for the
/// duration of a test and restores the previous value on drop. Must be
/// constructed only while `CHANNELS_PROCESS_LOCK` is held.
///
/// **Footgun for future tests:** `std::env::set_var` is process-global.
/// Any new test in this binary that boots a server exercising a
/// disk-touching handler (anything reaching `librefang_home()` — i.e.
/// any of the `/configure`, `/instances`, or QR flow handlers) MUST
/// acquire `CHANNELS_PROCESS_LOCK` before constructing this guard,
/// otherwise it will race with the disk-roundtrip tests below and see
/// the tempdir's `config.toml` instead of `~/.librefang`. Tests that
/// only exercise validation paths (unknown channel, missing field)
/// fail-fast before reaching `librefang_home()` and are safe without
/// the lock.
struct DiskHomeGuard {
    // `tmp` is held purely for RAII so the directory survives for the
    // life of the guard. With kernel-authoritative home_dir resolution
    // (codex review fixes #1/#7), tests no longer read disk from
    // `tmp.path()` — they target `state.kernel.home_dir()` directly.
    // We still set `LIBREFANG_HOME` to a sane tempdir so any code that
    // happens to call `std::env::var("LIBREFANG_HOME")` (e.g. the
    // kernel's `reload_config` path that mirrors production) doesn't
    // see the developer's real `~/.librefang`.
    #[allow(dead_code)]
    tmp: tempfile::TempDir,
    prev: Option<String>,
}

impl DiskHomeGuard {
    fn new() -> Self {
        let prev = std::env::var("LIBREFANG_HOME").ok();
        let tmp = tempfile::tempdir().expect("tempdir");
        // SAFETY: serialised via `CHANNELS_PROCESS_LOCK`. Caller holds the lock.
        unsafe {
            std::env::set_var("LIBREFANG_HOME", tmp.path());
        }
        Self { tmp, prev }
    }
}

impl Drop for DiskHomeGuard {
    fn drop(&mut self) {
        // SAFETY: same reasoning as `new`.
        unsafe {
            match &self.prev {
                Some(v) => std::env::set_var("LIBREFANG_HOME", v),
                None => std::env::remove_var("LIBREFANG_HOME"),
            }
        }
    }
}

/// Write a `config.toml` containing one `[[channels.discord]]` per pair.
/// Used by the disk-roundtrip tests below.
///
/// Pins `config_version` to the current value so the kernel's
/// `load_config()` migration path doesn't kick in and rewrite the
/// minimal fixture with a full canonical config dump on first read —
/// that rewrite would clobber test seeds (e.g. drop the
/// `bot_token_env = "..."` lines we want to assert against).
fn write_discord_instances(home: &Path, instances: &[&str]) {
    let mut content = format!(
        "config_version = {}\n",
        librefang_types::config::CONFIG_VERSION
    );
    for env_name in instances {
        content.push_str("[[channels.discord]]\n");
        content.push_str(&format!("bot_token_env = \"{env_name}\"\n\n"));
    }
    std::fs::write(home.join("config.toml"), content).expect("write config.toml");
}

struct Harness {
    app: Router,
    _state: Arc<AppState>,
    _test: TestAppState,
}

async fn boot() -> Harness {
    boot_with_channels(ChannelsConfig::default()).await
}

async fn boot_with_channels(channels: ChannelsConfig) -> Harness {
    let test = TestAppState::with_builder(MockKernelBuilder::new().with_config(move |cfg| {
        cfg.channels = channels.clone();
    }));
    let state = test.state.clone();
    let app = Router::new()
        .nest("/api", routes::channels::router())
        .with_state(state.clone());
    Harness {
        app,
        _state: state,
        _test: test,
    }
}

/// Harness + a `LIBREFANG_HOME` drop-guard wired to the same tempdir the
/// MockKernel uses as `home_dir_boot`. The sidecar-configure handler reads
/// `LIBREFANG_HOME` to resolve `secrets.env` / `config.toml`, and the
/// kernel's `reload_config()` reads `home_dir_boot.join("config.toml")` —
/// pointing the env var at the kernel's own tempdir keeps the two paths
/// in sync so the write and the subsequent reload see the same file.
///
/// Callers MUST hold `CHANNELS_PROCESS_LOCK` for the lifetime of the
/// returned `TempHomeHarness` because `std::env::set_var` is
/// process-global and the schema-cache seed is process-static.
struct TempHomeHarness {
    h: Harness,
    home: std::path::PathBuf,
    prev: Option<String>,
}

impl TempHomeHarness {
    fn home_dir(&self) -> &Path {
        &self.home
    }
}

impl Drop for TempHomeHarness {
    fn drop(&mut self) {
        // SAFETY: serialised via `CHANNELS_PROCESS_LOCK` held by the caller.
        unsafe {
            match &self.prev {
                Some(v) => std::env::set_var("LIBREFANG_HOME", v),
                None => std::env::remove_var("LIBREFANG_HOME"),
            }
        }
    }
}

impl std::ops::Deref for TempHomeHarness {
    type Target = Harness;
    fn deref(&self) -> &Harness {
        &self.h
    }
}

async fn boot_with_temp_home() -> TempHomeHarness {
    let test = TestAppState::with_builder(MockKernelBuilder::new());
    let home = test.tmp_path().to_path_buf();
    let state = test.state.clone();
    let app = Router::new()
        .nest("/api", routes::channels::router())
        .with_state(state.clone());
    let prev = std::env::var("LIBREFANG_HOME").ok();
    // SAFETY: serialised via `CHANNELS_PROCESS_LOCK` held by the caller.
    unsafe {
        std::env::set_var("LIBREFANG_HOME", &home);
    }
    TempHomeHarness {
        h: Harness {
            app,
            _state: state,
            _test: test,
        },
        home,
        prev,
    }
}

async fn json_request(
    h: &Harness,
    method: Method,
    path: &str,
    body: Option<serde_json::Value>,
) -> (StatusCode, serde_json::Value) {
    let mut builder = Request::builder().method(method).uri(path);
    let body_bytes = match body {
        Some(v) => {
            builder = builder.header("content-type", "application/json");
            serde_json::to_vec(&v).unwrap()
        }
        None => Vec::new(),
    };
    let req = builder.body(Body::from(body_bytes)).unwrap();
    let resp = h.app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), 1 << 20)
        .await
        .unwrap();
    let value: serde_json::Value = if bytes.is_empty() {
        serde_json::Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null)
    };
    (status, value)
}

// ---------------------------------------------------------------------------
// GET /api/channels
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn channels_list_returns_full_registry_with_zero_configured() {
    let h = boot().await;
    let (status, body) = json_request(&h, Method::GET, "/api/channels", None).await;
    assert_eq!(status, StatusCode::OK);

    let total = body["total"].as_u64().expect("total must be a number");
    let arr = body["items"].as_array().expect("items must be array");
    assert_eq!(total as usize, arr.len(), "total must match items.len()");
    assert!(total > 0, "registry must be non-empty");
    // Canonical PaginatedResponse envelope (#3842).
    assert_eq!(body["offset"], 0, "offset must be 0: {body}");
    assert!(body["limit"].is_null(), "limit must be null: {body}");
    assert_eq!(
        body["configured_count"], 0,
        "no channels seeded, configured_count must be 0: {body}"
    );

    // Every row must carry the dashboard's render contract.
    for row in arr {
        assert!(row["name"].is_string(), "missing name: {row}");
        assert!(row["display_name"].is_string(), "missing display_name");
        assert!(row["fields"].is_array(), "fields must be array");
        assert_eq!(
            row["configured"], false,
            "row {} should be unconfigured: {row}",
            row["name"]
        );
    }

    // Discord MUST be present — it's the canonical adapter.
    let discord = arr
        .iter()
        .find(|r| r["name"] == "discord")
        .expect("discord must appear in registry");
    assert_eq!(discord["configured"], false);
}

#[tokio::test(flavor = "multi_thread")]
async fn channels_list_flips_configured_flag_when_seeded() {
    let channels = ChannelsConfig {
        discord: OneOrMany(vec![DiscordConfig::default()]),
        ..ChannelsConfig::default()
    };
    let h = boot_with_channels(channels).await;

    let (status, body) = json_request(&h, Method::GET, "/api/channels", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body["configured_count"], 1,
        "exactly one channel was seeded: {body}"
    );

    let arr = body["items"].as_array().expect("array");
    let discord = arr
        .iter()
        .find(|r| r["name"] == "discord")
        .expect("discord row");
    assert_eq!(
        discord["configured"], true,
        "seeded discord must report configured=true: {discord}"
    );

    // Other rows must NOT be flipped just because discord is configured.
    let slack = arr
        .iter()
        .find(|r| r["name"] == "slack")
        .expect("slack row");
    assert_eq!(slack["configured"], false);
}

// ---------------------------------------------------------------------------
// GET /api/channels/{name}
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn channels_get_returns_metadata_for_known_channel() {
    let h = boot().await;
    let (status, body) = json_request(&h, Method::GET, "/api/channels/discord", None).await;
    assert_eq!(status, StatusCode::OK, "{body:?}");
    assert_eq!(body["name"], "discord");
    assert_eq!(body["display_name"], "Discord");
    assert!(body["fields"].is_array());
    assert_eq!(body["configured"], false);
}

#[tokio::test(flavor = "multi_thread")]
async fn channels_get_unknown_returns_404_with_unified_error() {
    let h = boot().await;
    let (status, body) = json_request(&h, Method::GET, "/api/channels/nope-not-real", None).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body:?}");
    let err = body["error"]["message"].as_str().unwrap_or("");
    assert!(
        err.contains("Unknown channel"),
        "error must mention 'Unknown channel': {body:?}"
    );
}

// ---------------------------------------------------------------------------
// GET /api/channels/registry
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn channels_registry_returns_json_even_with_no_dir() {
    // The harness's tmp home does not contain a `channels/` subdir, so the
    // runtime loader falls back to its empty default. The endpoint must
    // still return 200 with a valid JSON document — never 500.
    let h = boot().await;
    let (status, body) = json_request(&h, Method::GET, "/api/channels/registry", None).await;
    assert_eq!(status, StatusCode::OK, "{body:?}");
    assert!(
        body.is_array() || body.is_object(),
        "registry must be array or object, got: {body:?}"
    );
}

// ---------------------------------------------------------------------------
// POST /api/channels/{name}/configure  (validation paths only)
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn channels_configure_unknown_channel_returns_404() {
    let h = boot().await;
    let (status, body) = json_request(
        &h,
        Method::POST,
        "/api/channels/not-a-real-channel/configure",
        Some(serde_json::json!({"fields": {}})),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body:?}");
    assert!(
        body["error"]["message"]
            .as_str()
            .unwrap_or("")
            .contains("Unknown channel"),
        "error must mention 'Unknown channel': {body:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn channels_configure_missing_fields_object_returns_400() {
    let h = boot().await;
    // `fields` is required and must be a JSON object.
    let (status, body) = json_request(
        &h,
        Method::POST,
        "/api/channels/discord/configure",
        Some(serde_json::json!({"not_fields": "oops"})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body:?}");
    assert!(
        body["error"]["message"]
            .as_str()
            .unwrap_or("")
            .contains("fields"),
        "error must mention 'fields': {body:?}"
    );
}

// ---------------------------------------------------------------------------
// DELETE /api/channels/{name}/configure
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn channels_remove_unknown_channel_returns_404() {
    let h = boot().await;
    let (status, body) = json_request(
        &h,
        Method::DELETE,
        "/api/channels/not-a-real-channel/configure",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body:?}");
    assert!(
        body["error"]["message"]
            .as_str()
            .unwrap_or("")
            .contains("Unknown channel"),
        "error must mention 'Unknown channel': {body:?}"
    );
}

// ---------------------------------------------------------------------------
// POST /api/channels/{name}/test
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn channels_test_unknown_channel_returns_404() {
    let h = boot().await;
    let (status, body) = json_request(
        &h,
        Method::POST,
        "/api/channels/not-a-real-channel/test",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body:?}");
    // Post-#3505: error responses use the canonical `ApiErrorResponse`
    // envelope (`{"error": …}`). The pre-migration `{"status": "error",
    // "message": …}` shape no longer appears.
    // Post-#3639: `error` is a nested object with a `message` field.
    assert_eq!(body["error"]["message"], "Unknown channel");
    assert!(
        body.get("status").is_none(),
        "legacy `status` field must be gone post-#3505: {body}"
    );
}

// ---------------------------------------------------------------------------
// Per-instance endpoints (#4837)
//
// The legacy `/configure` endpoints treat every channel as a single
// `[channels.<name>]` table. The new `/instances` endpoints let the
// dashboard manage `[[channels.<name>]]` array entries — supporting two
// Discord bots, three Slack workspaces, etc. on the same channel type.
//
// As with `/configure`, we deliberately do NOT exercise happy-path WRITES
// here. POST/PUT mutate `~/.librefang/secrets.env` and process-wide env
// vars (`std::env::set_var`), which would race with parallel tests. The
// underlying TOML write logic is covered by the unit tests in
// `routes::skills::tests::{append,update,remove}_channel_instance_*`.
// What we DO cover here:
//   - GET /instances reflects the seeded `OneOrMany<T>` from KernelConfig
//   - All four routes 404 on unknown channel
//   - POST/PUT 400 on missing `fields`
//   - PUT/DELETE 404 when the instance index is out of range
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn channels_instances_list_empty_when_unconfigured() {
    let h = boot().await;
    let (status, body) =
        json_request(&h, Method::GET, "/api/channels/discord/instances", None).await;
    assert_eq!(status, StatusCode::OK, "{body:?}");
    assert_eq!(body["channel"], "discord");
    assert_eq!(body["total"], 0);
    let arr = body["items"].as_array().expect("items must be array");
    assert!(arr.is_empty(), "no instances seeded → items must be empty");
}

#[tokio::test(flavor = "multi_thread")]
async fn channels_instances_list_returns_seeded_instances() {
    // Seed two discord instances and assert both come through with
    // their configured fields and `index` values.
    let channels = ChannelsConfig {
        discord: OneOrMany(vec![
            DiscordConfig {
                bot_token_env: "TG_SUPPORT".into(),
                ..DiscordConfig::default()
            },
            DiscordConfig {
                bot_token_env: "TG_OPS".into(),
                ..DiscordConfig::default()
            },
        ]),
        ..ChannelsConfig::default()
    };
    let h = boot_with_channels(channels).await;

    let (status, body) =
        json_request(&h, Method::GET, "/api/channels/discord/instances", None).await;
    assert_eq!(status, StatusCode::OK, "{body:?}");
    assert_eq!(body["total"], 2, "two instances seeded: {body}");
    let arr = body["items"].as_array().expect("items array");
    assert_eq!(arr.len(), 2);
    assert_eq!(arr[0]["index"], 0);
    assert_eq!(arr[1]["index"], 1);
    assert_eq!(arr[0]["config"]["bot_token_env"], "TG_SUPPORT");
    assert_eq!(arr[1]["config"]["bot_token_env"], "TG_OPS");
    // Each instance must carry the field schema so the dashboard can
    // render the form without an extra `/api/channels/{name}` round-trip.
    assert!(
        arr[0]["fields"].is_array(),
        "fields schema must travel with each instance"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn channels_instances_list_unknown_channel_returns_404() {
    let h = boot().await;
    let (status, body) =
        json_request(&h, Method::GET, "/api/channels/not-a-real/instances", None).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body:?}");
    assert!(
        body["error"]["message"]
            .as_str()
            .unwrap_or("")
            .contains("Unknown channel"),
        "error must mention 'Unknown channel': {body}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn channels_create_instance_unknown_channel_returns_404() {
    let h = boot().await;
    let (status, body) = json_request(
        &h,
        Method::POST,
        "/api/channels/not-a-real/instances",
        Some(serde_json::json!({"fields": {}})),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn channels_create_instance_missing_fields_returns_400() {
    let h = boot().await;
    let (status, body) = json_request(
        &h,
        Method::POST,
        "/api/channels/discord/instances",
        Some(serde_json::json!({"not_fields": "oops"})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body:?}");
    assert!(
        body["error"]["message"]
            .as_str()
            .unwrap_or("")
            .contains("fields"),
        "error must mention 'fields': {body}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn channels_update_instance_unknown_channel_returns_404() {
    let h = boot().await;
    let (status, _body) = json_request(
        &h,
        Method::PUT,
        "/api/channels/not-a-real/instances/0",
        Some(serde_json::json!({"fields": {}})),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test(flavor = "multi_thread")]
async fn channels_update_instance_out_of_range_returns_404() {
    // Seed one instance, then try to PUT index 7. The handler must reject
    // because the index is out of range. The body carries a placeholder
    // signature — the post-#4865 PUT requires the CAS field, but a stale
    // value here is fine since the range check fires first under the
    // write lock.
    let channels = ChannelsConfig {
        discord: OneOrMany(vec![DiscordConfig::default()]),
        ..ChannelsConfig::default()
    };
    let h = boot_with_channels(channels).await;
    let (status, body) = json_request(
        &h,
        Method::PUT,
        "/api/channels/discord/instances/7",
        Some(serde_json::json!({
            "fields": {"default_agent": "x"},
            "signature": "stale-signature",
        })),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body:?}");
    assert!(
        body["error"]["message"]
            .as_str()
            .unwrap_or("")
            .contains("out of range"),
        "error must mention 'out of range': {body}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn channels_update_instance_missing_signature_returns_400() {
    // The post-#4865 PUT requires a `signature` body field for CAS so the
    // server can reject writes that target an instance which has been
    // moved or modified since the client read it. A missing signature is
    // a clean 400 — the handler must not silently fall through to a write.
    let channels = ChannelsConfig {
        discord: OneOrMany(vec![DiscordConfig::default()]),
        ..ChannelsConfig::default()
    };
    let h = boot_with_channels(channels).await;
    let (status, body) = json_request(
        &h,
        Method::PUT,
        "/api/channels/discord/instances/0",
        Some(serde_json::json!({"fields": {"default_agent": "x"}})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body:?}");
    assert!(
        body["error"]["message"]
            .as_str()
            .unwrap_or("")
            .contains("signature"),
        "error must call out the missing 'signature' field: {body}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn channels_update_instance_missing_fields_returns_400() {
    let channels = ChannelsConfig {
        discord: OneOrMany(vec![DiscordConfig::default()]),
        ..ChannelsConfig::default()
    };
    let h = boot_with_channels(channels).await;
    let (status, body) = json_request(
        &h,
        Method::PUT,
        "/api/channels/discord/instances/0",
        Some(serde_json::json!({"not_fields": "oops"})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn channels_delete_instance_unknown_channel_returns_404() {
    let h = boot().await;
    let (status, _body) = json_request(
        &h,
        Method::DELETE,
        "/api/channels/not-a-real/instances/0",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test(flavor = "multi_thread")]
async fn channels_delete_instance_out_of_range_returns_404() {
    let channels = ChannelsConfig {
        discord: OneOrMany(vec![DiscordConfig::default()]),
        ..ChannelsConfig::default()
    };
    let h = boot_with_channels(channels).await;
    // Post-#4865 DELETE requires `?signature=` for CAS — a stale value
    // here is fine, the range check fires first under the write lock.
    let (status, body) = json_request(
        &h,
        Method::DELETE,
        "/api/channels/discord/instances/3?signature=stale",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body:?}");
    assert!(
        body["error"]["message"]
            .as_str()
            .unwrap_or("")
            .contains("out of range"),
        "error must mention 'out of range': {body}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn channels_delete_instance_missing_signature_returns_400() {
    // The post-#4865 DELETE requires `?signature=` query parameter for
    // CAS. Without it the handler must reject with 400 before touching
    // disk — silently deleting based on an index alone is the bug class
    // the CAS token closes off.
    let channels = ChannelsConfig {
        discord: OneOrMany(vec![DiscordConfig::default()]),
        ..ChannelsConfig::default()
    };
    let h = boot_with_channels(channels).await;
    let (status, body) = json_request(
        &h,
        Method::DELETE,
        "/api/channels/discord/instances/0",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body:?}");
    assert!(
        body["error"]["message"]
            .as_str()
            .unwrap_or("")
            .contains("signature"),
        "error must call out the missing 'signature' query parameter: {body}"
    );
}

// ---------------------------------------------------------------------------
// Per-instance CAS round-trip (#4865)
// ---------------------------------------------------------------------------
//
// These tests own `LIBREFANG_HOME` (via `CHANNELS_PROCESS_LOCK` + `DiskHomeGuard`) so
// they can seed an actual `config.toml` and drive the post-#4865 handler
// flow that re-reads disk under the `config_write_lock`. Cheaper unit-
// level coverage of the same primitives lives next to the helpers in
// `routes::channels::instance_helper_tests` and `routes::skills::tests`;
// these guard the HTTP-layer wiring.

#[tokio::test(flavor = "multi_thread")]
async fn channels_update_instance_signature_mismatch_returns_409() {
    let _lock = CHANNELS_PROCESS_LOCK.lock().await;
    let guard = DiskHomeGuard::new();

    let h = boot_with_channels(ChannelsConfig {
        discord: OneOrMany(vec![DiscordConfig {
            bot_token_env: "TG_DISK_A".into(),
            ..DiscordConfig::default()
        }]),
        ..ChannelsConfig::default()
    })
    .await;
    // Seed disk into the kernel's authoritative home_dir — the
    // handlers resolve paths via `state.kernel.home_dir()` (codex
    // review fixes #1/#7), so writing into `guard.home()` would miss.
    // `DiskHomeGuard` is still held so any code that happens to call
    // `std::env::var("LIBREFANG_HOME")` (e.g. the kernel's own
    // `reload_config` path) sees a sane value.
    let kernel_home = h._state.kernel.home_dir().to_path_buf();
    write_discord_instances(&kernel_home, &["TG_DISK_A"]);
    let _ = &guard; // keep the env guard alive for the whole test

    // PUT idx=0 with a deliberately stale signature. After #4865 the
    // handler re-reads disk, recomputes the signature for the current
    // disk-side instance, and rejects on mismatch with 409 Conflict.
    let (status, body) = json_request(
        &h,
        Method::PUT,
        "/api/channels/discord/instances/0",
        Some(serde_json::json!({
            "fields": { "default_agent": "smoke" },
            "signature": "0".repeat(64),
        })),
    )
    .await;

    assert_eq!(
        status,
        StatusCode::CONFLICT,
        "stale signature must yield 409, not 500/200: {body:?}"
    );
    assert!(
        body["error"]["message"]
            .as_str()
            .unwrap_or("")
            .contains("modified or moved"),
        "error must explain the conflict so the dashboard can surface a refresh prompt: {body}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn channels_delete_instance_signature_mismatch_returns_409() {
    let _lock = CHANNELS_PROCESS_LOCK.lock().await;
    let guard = DiskHomeGuard::new();

    let h = boot_with_channels(ChannelsConfig {
        discord: OneOrMany(vec![
            DiscordConfig {
                bot_token_env: "TG_DISK_B".into(),
                ..DiscordConfig::default()
            },
            DiscordConfig {
                bot_token_env: "TG_DISK_C".into(),
                ..DiscordConfig::default()
            },
        ]),
        ..ChannelsConfig::default()
    })
    .await;
    let kernel_home = h._state.kernel.home_dir().to_path_buf();
    write_discord_instances(&kernel_home, &["TG_DISK_B", "TG_DISK_C"]);
    let _ = &guard;

    let (status, body) = json_request(
        &h,
        Method::DELETE,
        "/api/channels/discord/instances/0?signature=ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
        None,
    )
    .await;

    assert_eq!(
        status,
        StatusCode::CONFLICT,
        "stale signature on DELETE must yield 409, not silently delete: {body:?}"
    );
    // Disk must be untouched — both instances still present after the
    // rejected delete.
    let raw = std::fs::read_to_string(kernel_home.join("config.toml")).expect("read config.toml");
    assert!(
        raw.contains("TG_DISK_B"),
        "rejected DELETE must leave instance 0 intact: {raw}"
    );
    assert!(
        raw.contains("TG_DISK_C"),
        "rejected DELETE must leave instance 1 intact: {raw}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn channels_update_instance_round_trips_real_signature() {
    let _lock = CHANNELS_PROCESS_LOCK.lock().await;
    let guard = DiskHomeGuard::new();

    let h = boot_with_channels(ChannelsConfig {
        discord: OneOrMany(vec![DiscordConfig {
            bot_token_env: "TG_DISK_D".into(),
            ..DiscordConfig::default()
        }]),
        ..ChannelsConfig::default()
    })
    .await;
    let kernel_home = h._state.kernel.home_dir().to_path_buf();
    write_discord_instances(&kernel_home, &["TG_DISK_D"]);
    let _ = &guard;

    // GET the list to obtain the server-computed signature for the row
    // we're about to update.
    let (list_status, list_body) =
        json_request(&h, Method::GET, "/api/channels/discord/instances", None).await;
    assert_eq!(list_status, StatusCode::OK);
    let signature = list_body["items"][0]["signature"]
        .as_str()
        .expect("list must surface a per-item signature post-#4865")
        .to_string();
    assert_eq!(signature.len(), 64, "signature must be sha-256 hex");

    // Echo it back on PUT — the handler must accept this round-trip.
    // (Failure here is a regression on the canonical-JSON ↔ disk-reread
    // invariant documented in `canonical_json` / `read_disk_channels`.)
    let (status, body) = json_request(
        &h,
        Method::PUT,
        "/api/channels/discord/instances/0",
        Some(serde_json::json!({
            "fields": { "default_agent": "rotated" },
            "signature": signature,
        })),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "round-tripped signature must be accepted: {body:?}"
    );

    // Disk now reflects the new agent name.
    let raw = std::fs::read_to_string(kernel_home.join("config.toml")).expect("read config.toml");
    assert!(
        raw.contains("default_agent = \"rotated\""),
        "PUT must have written the new field to disk: {raw}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn channels_update_instance_clear_secrets_drops_orphan_env_var() {
    let _lock = CHANNELS_PROCESS_LOCK.lock().await;
    let guard = DiskHomeGuard::new();

    let h = boot_with_channels(ChannelsConfig {
        discord: OneOrMany(vec![DiscordConfig {
            bot_token_env: "TG_LONELY".into(),
            ..DiscordConfig::default()
        }]),
        ..ChannelsConfig::default()
    })
    .await;
    let kernel_home = h._state.kernel.home_dir().to_path_buf();
    write_discord_instances(&kernel_home, &["TG_LONELY"]);
    // Prime `secrets.env` with the env var the instance is pointing at,
    // so we can assert the cleanup loop actually removed it.
    std::fs::write(kernel_home.join("secrets.env"), "TG_LONELY=fake-token\n")
        .expect("seed secrets.env");
    let _ = &guard;

    let (_, list_body) =
        json_request(&h, Method::GET, "/api/channels/discord/instances", None).await;
    let signature = list_body["items"][0]["signature"]
        .as_str()
        .expect("signature must be present")
        .to_string();

    // PUT with `clear_secrets` listing the secret key. The instance's
    // `bot_token_env` ref must drop, AND because no sibling references
    // `TG_LONELY` the env-var line must be scrubbed from `secrets.env`.
    let (status, body) = json_request(
        &h,
        Method::PUT,
        "/api/channels/discord/instances/0",
        Some(serde_json::json!({
            "fields": { "default_agent": "no-auth" },
            "signature": signature,
            "clear_secrets": ["bot_token_env"],
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body:?}");

    let cfg = std::fs::read_to_string(kernel_home.join("config.toml")).expect("read config.toml");
    assert!(
        !cfg.contains("bot_token_env"),
        "cleared secret ref must be dropped from the rebuilt instance: {cfg}"
    );
    let secrets =
        std::fs::read_to_string(kernel_home.join("secrets.env")).expect("read secrets.env");
    assert!(
        !secrets.contains("TG_LONELY"),
        "orphan env-var line must be scrubbed when no sibling references it: {secrets}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn channels_update_instance_clear_secrets_preserves_shared_env_var() {
    let _lock = CHANNELS_PROCESS_LOCK.lock().await;
    let guard = DiskHomeGuard::new();
    // Two instances, BOTH pointing at the same env var (a possible
    // user setup if they hand-edited secrets.env). Clearing one
    // instance's ref must NOT remove the env var, since the sibling
    // is still using it.

    let h = boot_with_channels(ChannelsConfig {
        discord: OneOrMany(vec![
            DiscordConfig {
                bot_token_env: "TG_SHARED".into(),
                ..DiscordConfig::default()
            },
            DiscordConfig {
                bot_token_env: "TG_SHARED".into(),
                ..DiscordConfig::default()
            },
        ]),
        ..ChannelsConfig::default()
    })
    .await;
    let kernel_home = h._state.kernel.home_dir().to_path_buf();
    write_discord_instances(&kernel_home, &["TG_SHARED", "TG_SHARED"]);
    std::fs::write(kernel_home.join("secrets.env"), "TG_SHARED=fake\n").expect("seed secrets.env");
    let _ = &guard;

    let (_, list_body) =
        json_request(&h, Method::GET, "/api/channels/discord/instances", None).await;
    let signature = list_body["items"][0]["signature"]
        .as_str()
        .expect("signature")
        .to_string();

    let (status, body) = json_request(
        &h,
        Method::PUT,
        "/api/channels/discord/instances/0",
        Some(serde_json::json!({
            "fields": { "default_agent": "no-auth" },
            "signature": signature,
            "clear_secrets": ["bot_token_env"],
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body:?}");

    let secrets =
        std::fs::read_to_string(kernel_home.join("secrets.env")).expect("read secrets.env");
    assert!(
        secrets.contains("TG_SHARED"),
        "shared env var must survive — sibling instance still references it: {secrets}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn channels_list_includes_instance_count() {
    // The dashboard's card subtitle ("Discord · 2 bots") depends on
    // `instance_count` riding alongside the existing `configured` flag.
    let channels = ChannelsConfig {
        discord: OneOrMany(vec![
            DiscordConfig::default(),
            DiscordConfig::default(),
            DiscordConfig::default(),
        ]),
        ..ChannelsConfig::default()
    };
    let h = boot_with_channels(channels).await;
    let (status, body) = json_request(&h, Method::GET, "/api/channels", None).await;
    assert_eq!(status, StatusCode::OK);
    let arr = body["items"].as_array().expect("items");
    let discord = arr
        .iter()
        .find(|r| r["name"] == "discord")
        .expect("discord row");
    assert_eq!(
        discord["instance_count"], 3,
        "discord seeded with 3 instances must report instance_count=3: {discord}"
    );
    let slack = arr
        .iter()
        .find(|r| r["name"] == "slack")
        .expect("slack row");
    assert_eq!(
        slack["instance_count"], 0,
        "slack untouched must report instance_count=0: {slack}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn channels_test_known_channel_with_no_creds_reports_missing_env() {
    // #3507 reshaped this handler so the HTTP status reflects the actual
    // outcome — `412 Precondition Failed` for missing credentials.
    // Previously this returned 200 + body diagnostic, which made
    // `fetch().ok` lie to clients. #3505 then migrated the body shape
    // from the ad-hoc `{"status": "error", "message": …}` form to the
    // canonical `ApiErrorResponse` envelope (`{"error": …}`).
    //
    // We deliberately do not assert on a specific env var name to avoid
    // coupling the test to the registry; we only require the handler to
    // signal "missing required env vars" so a refactor that drops the
    // env-presence check trips this assertion.
    //
    // Note: this assertion is only meaningful while the test process has
    // not exported `DISCORD_BOT_TOKEN`. The test harness never sets it,
    // and other tests in this file never call `set_var`, so the
    // pre-condition holds.
    if std::env::var("DISCORD_BOT_TOKEN").is_ok() {
        eprintln!(
            "skipping channels_test_known_channel_with_no_creds_reports_missing_env: \
             DISCORD_BOT_TOKEN is set in the environment"
        );
        return;
    }

    let h = boot().await;
    let (status, body) = json_request(&h, Method::POST, "/api/channels/discord/test", None).await;
    assert_eq!(status, StatusCode::PRECONDITION_FAILED, "{body:?}");
    let msg = body["error"]["message"].as_str().unwrap_or("");
    assert!(
        msg.contains("Missing required env vars"),
        "error must call out missing env vars: {body}"
    );
    assert!(
        body.get("status").is_none(),
        "legacy `status` field must be gone post-#3505: {body}"
    );
}

// ---------------------------------------------------------------------------
// Sidecar-backed channels stay visible on /api/channels (#5241 / #5224)
// ---------------------------------------------------------------------------

/// `SidecarChannelConfig` has no `Default` and many serde-defaulted
/// fields — build it from JSON so the test stays robust to new fields.
fn sidecar_telegram() -> SidecarChannelConfig {
    serde_json::from_value(serde_json::json!({
        "name": "telegram",
        "command": "python3",
        "args": ["-m", "librefang.sidecar.adapters.telegram"],
        "channel_type": "telegram",
    }))
    .expect("valid SidecarChannelConfig")
}

async fn boot_with_sidecar(sidecar: Vec<SidecarChannelConfig>) -> Harness {
    let test = TestAppState::with_builder(MockKernelBuilder::new().with_config(move |cfg| {
        cfg.sidecar_channels = sidecar.clone();
    }));
    let state = test.state.clone();
    let app = Router::new()
        .nest("/api", routes::channels::router())
        .with_state(state.clone());
    Harness {
        app,
        _state: state,
        _test: test,
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn channels_list_includes_configured_sidecar_channels() {
    // Regression: telegram / ntfy migrated out-of-process were removed
    // from CHANNEL_REGISTRY and silently vanished from the dashboard
    // channels page. A configured [[sidecar_channels]] entry must still
    // surface as a channel row so the operator view stays consistent
    // regardless of in-process vs sidecar (#5241 / #5224).
    let h = boot_with_sidecar(vec![sidecar_telegram()]).await;
    let (status, body) = json_request(&h, Method::GET, "/api/channels", None).await;
    assert_eq!(status, StatusCode::OK);

    let arr = body["items"].as_array().expect("items");
    let tg = arr
        .iter()
        .find(|r| r["name"] == "telegram")
        .expect("configured sidecar telegram must appear in /api/channels");
    assert_eq!(
        tg["configured"], true,
        "a declared [[sidecar_channels]] is configured: {tg}"
    );
    assert_eq!(tg["category"], "sidecar");
    assert_eq!(tg["setup_type"], "sidecar");
    // config.toml-managed — no editable dashboard form fields (the page
    // renders it as a configured/online card, not a broken setup form).
    assert_eq!(
        tg["fields"].as_array().map(|a| a.len()),
        Some(0),
        "sidecar row must carry no editable fields: {tg}"
    );
    // Counts toward the dashboard's "X configured" sub-line.
    assert!(body["configured_count"].as_u64().unwrap_or(0) >= 1);
    // Exactly one telegram row — the in-process ChannelMeta is gone, so
    // the sidecar row must not be shadowed or duplicated.
    assert_eq!(
        arr.iter().filter(|r| r["name"] == "telegram").count(),
        1,
        "exactly one telegram row (the sidecar one)"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn channels_list_without_sidecar_surfaces_discovery_catalog() {
    // With nothing configured under [[sidecar_channels]], the first-party
    // SDK adapters (telegram, ntfy) must still appear as unconfigured
    // catalog rows — otherwise the Add picker has no way to surface them
    // after the out-of-process migration (#5241 / #5224). Operators who
    // have never touched config.toml saw telegram/ntfy vanish from the
    // dashboard entirely before this; the discovery rows close that gap.
    let _g = CHANNELS_PROCESS_LOCK.lock().await;
    librefang_api::routes::channels::__test_seed_sidecar_schema_cache(&[]);
    let h = boot().await;
    let (status, body) = json_request(&h, Method::GET, "/api/channels", None).await;
    assert_eq!(status, StatusCode::OK);
    let arr = body["items"].as_array().expect("items");

    for kind in ["telegram", "ntfy"] {
        let row = arr
            .iter()
            .find(|r| r["name"] == kind)
            .unwrap_or_else(|| panic!("discovery row for sidecar `{kind}` must appear"));
        assert_eq!(
            row["configured"], false,
            "discovery row must be unconfigured: {row}"
        );
        assert_eq!(row["category"], "sidecar");
        assert_eq!(row["setup_type"], "sidecar");
        assert_eq!(
            row["fields"].as_array().map(|a| a.len()),
            Some(0),
            "catalog row has no editable form fields: {row}"
        );
        // Picker uses display_name; must be non-empty.
        assert!(
            row["display_name"].as_str().is_some_and(|s| !s.is_empty()),
            "discovery row needs a display name: {row}"
        );
    }

    // Unconfigured catalog rows must not bump the configured counter —
    // they represent "available to set up", not "already set up".
    let configured_count = body["configured_count"].as_u64().unwrap_or(0);
    let sidecar_unconfigured = arr
        .iter()
        .filter(|r| r["category"] == "sidecar" && r["configured"] == false)
        .count() as u64;
    assert!(
        configured_count + sidecar_unconfigured <= arr.len() as u64,
        "configured_count must not include discovery rows"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn channels_list_discovery_rows_carry_form_fields_when_schema_cached() {
    // Pre-populate the schema cache with a synthetic telegram schema so the
    // test runs deterministically without depending on `pip install -e
    // sdk/python` on every CI box.
    let _g = CHANNELS_PROCESS_LOCK.lock().await;
    librefang_api::routes::channels::__test_seed_sidecar_schema_cache(&[(
        "telegram",
        librefang_api::routes::sidecar_describe::SidecarSchema {
            name: "telegram".into(),
            display_name: "Telegram".into(),
            description: "Telegram Bot API adapter".into(),
            fields: vec![
                librefang_api::routes::sidecar_describe::SidecarSchemaField {
                    key: "TELEGRAM_BOT_TOKEN".into(),
                    label: "Bot Token".into(),
                    field_type: "secret".into(),
                    required: true,
                    placeholder: "123:ABC".into(),
                    advanced: false,
                    options: None,
                },
            ],
        },
    )]);

    let h = boot().await;
    let (_status, body) = json_request(&h, Method::GET, "/api/channels", None).await;
    let arr = body["items"].as_array().expect("items");
    let tg = arr
        .iter()
        .find(|r| r["name"] == "telegram")
        .expect("telegram row");
    let fields = tg["fields"].as_array().expect("fields[]");
    assert!(
        !fields.is_empty(),
        "discovery row must carry fields when cached: {tg}"
    );
    assert_eq!(fields[0]["key"], "TELEGRAM_BOT_TOKEN");
    assert_eq!(fields[0]["type"], "secret");
    assert_eq!(fields[0]["required"], true);

    // Clear the cache so sibling tests asserting empty fields are not
    // polluted by this seed (the cache is process-static).
    librefang_api::routes::channels::__test_seed_sidecar_schema_cache(&[]);
}

#[tokio::test(flavor = "multi_thread")]
async fn channels_list_discovery_row_hidden_when_kind_configured() {
    // Discovery row for `telegram` must yield to a configured
    // `[[sidecar_channels]]` entry of the same kind — regardless of the
    // local alias the operator picked for `name`. Otherwise the page
    // would render both a "telegram (configured)" row and a "telegram
    // (set me up)" row simultaneously.
    let aliased: SidecarChannelConfig = serde_json::from_value(serde_json::json!({
        "name": "my_alerts",
        "command": "python3",
        "args": ["-m", "librefang.sidecar.adapters.telegram"],
        "channel_type": "telegram",
    }))
    .expect("valid SidecarChannelConfig");
    let h = boot_with_sidecar(vec![aliased]).await;
    let (status, body) = json_request(&h, Method::GET, "/api/channels", None).await;
    assert_eq!(status, StatusCode::OK);
    let arr = body["items"].as_array().expect("items");

    // The aliased configured row appears under its local name.
    assert!(
        arr.iter()
            .any(|r| r["name"] == "my_alerts" && r["configured"] == true),
        "configured aliased telegram row must appear"
    );
    // No discovery row for telegram — it has been "covered".
    assert!(
        !arr.iter().any(|r| r["name"] == "telegram"),
        "discovery row for `telegram` must be suppressed when channel_type=telegram is configured"
    );
    // ntfy discovery row is still there (independent kind, not configured).
    assert!(
        arr.iter()
            .any(|r| r["name"] == "ntfy" && r["configured"] == false),
        "ntfy discovery row remains when only telegram is configured"
    );
}

// ---------------------------------------------------------------------------
// POST /api/channels/sidecar/{name}/configure
// ---------------------------------------------------------------------------

/// Build a synthetic telegram schema with one required secret + one
/// optional list field. Used by the configure-sidecar tests below so they
/// don't depend on `pip install -e sdk/python` being available on CI.
fn telegram_schema_with_required_secret() -> librefang_api::routes::sidecar_describe::SidecarSchema
{
    librefang_api::routes::sidecar_describe::SidecarSchema {
        name: "telegram".into(),
        display_name: "Telegram".into(),
        description: "Telegram Bot API adapter".into(),
        fields: vec![
            librefang_api::routes::sidecar_describe::SidecarSchemaField {
                key: "TELEGRAM_BOT_TOKEN".into(),
                label: "Bot Token".into(),
                field_type: "secret".into(),
                required: true,
                placeholder: "".into(),
                advanced: false,
                options: None,
            },
            librefang_api::routes::sidecar_describe::SidecarSchemaField {
                key: "ALLOWED_USERS".into(),
                label: "Allowed Users".into(),
                field_type: "list".into(),
                required: false,
                placeholder: "".into(),
                advanced: false,
                options: None,
            },
        ],
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn configure_sidecar_writes_secret_to_env_and_nonsecret_to_toml() {
    let _g = CHANNELS_PROCESS_LOCK.lock().await;
    librefang_api::routes::channels::__test_seed_sidecar_schema_cache(&[(
        "telegram",
        telegram_schema_with_required_secret(),
    )]);

    let h = boot_with_temp_home().await;
    let body = serde_json::json!({
        "values": {
            "TELEGRAM_BOT_TOKEN": "secret-123",
            "ALLOWED_USERS": "1,2,3",
        }
    });
    let (status, resp) = json_request(
        &h,
        Method::POST,
        "/api/channels/sidecar/telegram/configure",
        Some(body),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "response: {resp}");
    assert_eq!(resp["status"], "saved");

    // Plan Risk #3 regression — the success response must NEVER echo the
    // secret value or contain a `values` key. Any future refactor that
    // re-includes the request payload in the response will trip these.
    let resp_str = resp.to_string();
    assert!(
        !resp_str.contains("secret-123"),
        "response must not echo the secret value: {resp_str}"
    );
    assert!(
        resp.get("values").is_none(),
        "response must not include a `values` field: {resp_str}"
    );

    // Verify side effects on disk.
    let home = h.home_dir();
    let secrets = std::fs::read_to_string(home.join("secrets.env")).expect("secrets.env exists");
    assert!(
        secrets.contains("TELEGRAM_BOT_TOKEN=secret-123"),
        "secret must land in secrets.env: {secrets}"
    );
    assert!(
        !secrets.contains("ALLOWED_USERS"),
        "non-secret fields must NOT land in secrets.env: {secrets}"
    );

    let toml = std::fs::read_to_string(home.join("config.toml")).expect("config.toml exists");
    assert!(toml.contains("[[sidecar_channels]]"), "toml: {toml}");
    assert!(toml.contains("name = \"telegram\""), "toml: {toml}");
    assert!(
        toml.contains("ALLOWED_USERS = \"1,2,3\""),
        "non-secret must land under [sidecar_channels.env]: {toml}"
    );
    assert!(
        !toml.contains("TELEGRAM_BOT_TOKEN"),
        "secrets must NOT leak into config.toml: {toml}"
    );

    // T4.1: sidecar_channels diff must emit ReloadChannels so the bridge
    // re-inits without a daemon restart.
    assert!(
        resp["hot_actions_applied"]
            .as_array()
            .is_some_and(|a| a.iter().any(|v| v == "ReloadChannels")),
        "expected ReloadChannels in hot_actions_applied: {resp}"
    );

    // Plan Risk #5: with no shell-env shadow set, `shadowed_secrets`
    // must be present and empty. (The field is always emitted, even on
    // the happy path — the dashboard relies on that to short-circuit
    // its warning toast.)
    let shadowed = resp["shadowed_secrets"]
        .as_array()
        .expect("shadowed_secrets must be an array: {resp}");
    assert!(
        shadowed.is_empty(),
        "no shell-env shadow expected here, but got: {resp}"
    );

    // T4.2: prove the bridge actually re-spawned, not just that the kernel's
    // in-memory config was reloaded. `reload_config()` clears
    // `mesh.channel_adapters` (see `config_reload_ops.rs::246-256`), and only
    // the handler-side follow-up `channel_bridge::reload_channels_from_disk`
    // re-populates it by re-entering `start_channel_bridge_with_config`. So
    // the presence of a telegram entry in `channel_adapters_ref()` after the
    // save returns is a direct regression test for the broken-chain bug:
    // without (A) the map stays empty and this assertion fires.
    //
    // (Asserting `configured: true` via GET /api/channels would not catch
    // this: that flag reads `kernel.config_ref().sidecar_channels`, which
    // `reload_config()` updates on its own — independent of bridge spawn.)
    let adapters = h._state.kernel.channel_adapters_ref();
    assert!(
        adapters.contains_key("telegram"),
        "telegram adapter must be registered in channel_adapters after sidecar configure; \
         saw keys: {:?}",
        adapters.iter().map(|e| e.key().clone()).collect::<Vec<_>>()
    );

    // Cross-check the operator-facing view: GET /api/channels must also
    // surface the row as configured (this passes once `reload_config()`
    // ran, regardless of bridge spawn — included as a defence-in-depth
    // check that the kernel's in-memory config matches what's on disk).
    let (list_status, list_body) = json_request(&h, Method::GET, "/api/channels", None).await;
    assert_eq!(list_status, StatusCode::OK, "list response: {list_body}");
    let items = list_body["items"]
        .as_array()
        .expect("items must be an array");
    let telegram = items
        .iter()
        .find(|r| r["name"] == "telegram")
        .expect("telegram row must appear after configure");
    assert_eq!(
        telegram["configured"], true,
        "telegram must be configured=true after sidecar save: {telegram}"
    );

    // Clear cache so sibling tests are not polluted by this seed.
    librefang_api::routes::channels::__test_seed_sidecar_schema_cache(&[]);
}

// Plan Risk #5 regression. If the operator already exported the secret
// key in their shell before launching the daemon, the dotenv loader's
// priority order (system env > vault > .env > secrets.env) means the
// shell value out-ranks whatever we write to `~/.librefang/secrets.env`.
// The save mechanically succeeds, but the new value never reaches the
// sidecar child. We surface this condition via `shadowed_secrets` so the
// dashboard can warn the operator before they spend an hour chasing it.
#[tokio::test(flavor = "multi_thread")]
async fn configure_sidecar_warns_on_shell_env_shadow() {
    // Hold the channels-process lock so no other test mutates `LIBREFANG_HOME`
    // or the process env while we toggle TELEGRAM_BOT_TOKEN.
    let _g = CHANNELS_PROCESS_LOCK.lock().await;
    librefang_api::routes::channels::__test_seed_sidecar_schema_cache(&[(
        "telegram",
        telegram_schema_with_required_secret(),
    )]);

    // SAFETY: serialised via `CHANNELS_PROCESS_LOCK` held by the caller —
    // no other test reads or mutates env vars concurrently.
    unsafe { std::env::set_var("TELEGRAM_BOT_TOKEN", "shell-set-val") };

    let h = boot_with_temp_home().await;
    let body = serde_json::json!({
        "values": {
            "TELEGRAM_BOT_TOKEN": "form-val",
            "ALLOWED_USERS": "1",
        }
    });
    let (status, resp) = json_request(
        &h,
        Method::POST,
        "/api/channels/sidecar/telegram/configure",
        Some(body),
    )
    .await;
    // The save itself must still succeed — shadowing is advisory.
    assert_eq!(status, StatusCode::OK, "response: {resp}");
    assert_eq!(resp["status"], "saved");

    let shadowed = resp["shadowed_secrets"]
        .as_array()
        .expect("shadowed_secrets must be an array");
    assert!(
        shadowed.iter().any(|v| v == "TELEGRAM_BOT_TOKEN"),
        "shadowed_secrets must include TELEGRAM_BOT_TOKEN: {resp}"
    );

    // SAFETY: same lock guarantees as the set above.
    unsafe { std::env::remove_var("TELEGRAM_BOT_TOKEN") };
    librefang_api::routes::channels::__test_seed_sidecar_schema_cache(&[]);
}

#[tokio::test(flavor = "multi_thread")]
async fn configure_sidecar_missing_required_returns_400() {
    let _g = CHANNELS_PROCESS_LOCK.lock().await;
    librefang_api::routes::channels::__test_seed_sidecar_schema_cache(&[(
        "telegram",
        telegram_schema_with_required_secret(),
    )]);

    let h = boot_with_temp_home().await;
    let body = serde_json::json!({ "values": { "ALLOWED_USERS": "1" } });
    let (status, resp) = json_request(
        &h,
        Method::POST,
        "/api/channels/sidecar/telegram/configure",
        Some(body),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "response: {resp}");
    assert!(
        resp.to_string().contains("TELEGRAM_BOT_TOKEN"),
        "error body must name the missing field: {resp}"
    );
    // No disk side effect when validation rejects.
    assert!(
        !h.home_dir().join("secrets.env").exists(),
        "secrets.env must not be created on validation failure"
    );
    assert!(
        !h.home_dir().join("config.toml").exists(),
        "config.toml must not be created on validation failure"
    );

    librefang_api::routes::channels::__test_seed_sidecar_schema_cache(&[]);
}

#[tokio::test(flavor = "multi_thread")]
async fn configure_sidecar_unknown_name_returns_404() {
    let _g = CHANNELS_PROCESS_LOCK.lock().await;
    librefang_api::routes::channels::__test_seed_sidecar_schema_cache(&[]);

    let h = boot_with_temp_home().await;
    let body = serde_json::json!({ "values": {} });
    let (status, _resp) = json_request(
        &h,
        Method::POST,
        "/api/channels/sidecar/nonexistent/configure",
        Some(body),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

/// When a deployment keeps `[[sidecar_channels]]` in a file referenced
/// from `include = [...]`, this endpoint must NOT write a fresh
/// root-level array that would silently shadow the included one after
/// the kernel merges them. The save is refused with 409 and an error
/// message pointing the operator at the file that owns the existing
/// block.
#[tokio::test(flavor = "multi_thread")]
async fn configure_sidecar_refuses_when_include_owns_sidecars() {
    let _g = CHANNELS_PROCESS_LOCK.lock().await;
    librefang_api::routes::channels::__test_seed_sidecar_schema_cache(&[(
        "telegram",
        telegram_schema_with_required_secret(),
    )]);

    let h = boot_with_temp_home().await;
    let home = h.home_dir();
    std::fs::write(home.join("config.toml"), "include = [\"sidecars.toml\"]\n").unwrap();
    std::fs::write(
        home.join("sidecars.toml"),
        "[[sidecar_channels]]\nname=\"ntfy\"\ncommand=\"python3\"\nargs=[\"-m\",\"librefang.sidecar.adapters.ntfy\"]\n",
    )
    .unwrap();
    let body = serde_json::json!({ "values": { "TELEGRAM_BOT_TOKEN": "x" } });
    let (status, resp) = json_request(
        &h,
        Method::POST,
        "/api/channels/sidecar/telegram/configure",
        Some(body),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "response: {resp}");
    let body = resp.to_string();
    assert!(
        body.contains("include"),
        "error must mention the include shadow: {body}"
    );
    assert!(
        body.contains("sidecars.toml"),
        "error must name the included file owning the existing array: {body}"
    );

    // No side effects: the configure handler must NOT have written a root-level
    // `[[sidecar_channels]]` block onto the include-only config.toml.
    let after = std::fs::read_to_string(home.join("config.toml")).unwrap();
    assert!(
        !after.contains("[[sidecar_channels]]"),
        "config.toml must remain include-only on 409: {after}"
    );
    // secrets.env must NOT have been written either — we refuse the entire op.
    assert!(
        !home.join("secrets.env").exists(),
        "secrets.env must not be created when the save is refused with 409"
    );

    librefang_api::routes::channels::__test_seed_sidecar_schema_cache(&[]);
}

/// `included_files_with_sidecars` uses a substring match for
/// `[[sidecar_channels]]` rather than parsing the included file's TOML.
/// That conservative heuristic deliberately produces false positives —
/// an included file that only MENTIONS the string in a comment will
/// still trigger 409. This test documents that limitation: the
/// alternative (full TOML parse of every included file) trades a
/// minor operator surprise for a meaningful complexity / correctness
/// cliff (the include resolver would need to mirror the kernel's
/// recursive include + cycle-detection logic). 409 with the include
/// path in the error message is a recoverable state — the operator
/// either removes the comment or edits the included file directly,
/// as the 409 message instructs.
#[tokio::test(flavor = "multi_thread")]
async fn configure_sidecar_refuses_even_on_commented_sidecar_string() {
    let _g = CHANNELS_PROCESS_LOCK.lock().await;
    librefang_api::routes::channels::__test_seed_sidecar_schema_cache(&[(
        "telegram",
        telegram_schema_with_required_secret(),
    )]);

    let h = boot_with_temp_home().await;
    let home = h.home_dir();
    std::fs::write(home.join("config.toml"), "include = [\"docs.toml\"]\n").unwrap();
    // The included file is otherwise empty — only a comment mentions
    // the array header string.
    std::fs::write(
        home.join("docs.toml"),
        "# example: [[sidecar_channels]] should look like this\n",
    )
    .unwrap();

    let body = serde_json::json!({ "values": { "TELEGRAM_BOT_TOKEN": "x" } });
    let (status, resp) = json_request(
        &h,
        Method::POST,
        "/api/channels/sidecar/telegram/configure",
        Some(body),
    )
    .await;
    // Conservative: substring match triggers 409 even on a comment.
    assert_eq!(
        status,
        StatusCode::CONFLICT,
        "documented limitation: comment-mention triggers 409: {resp}"
    );
    assert!(
        resp.to_string().contains("include"),
        "error must mention the include shadow: {resp}"
    );

    librefang_api::routes::channels::__test_seed_sidecar_schema_cache(&[]);
}
