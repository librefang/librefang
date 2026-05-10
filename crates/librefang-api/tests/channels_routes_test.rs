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
use librefang_types::config::{ChannelsConfig, OneOrMany, TelegramConfig};
use std::path::Path;
use std::sync::Arc;
use tower::ServiceExt;

/// Serialises every test in this binary that touches `LIBREFANG_HOME`.
/// The handlers added in #4865 read `[channels]` from disk under the
/// `config_write_lock`, so any test that needs to drive that path must
/// own the env var for its full duration. Tests that don't read disk
/// run in parallel as before — they're insensitive to whatever
/// `LIBREFANG_HOME` happens to point at because they fail-fast at
/// validation before reaching the disk read. (#4865)
static ENV_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Drop guard that points `LIBREFANG_HOME` at a tempdir for the
/// duration of a test and restores the previous value on drop. Must be
/// constructed only while `ENV_LOCK` is held.
struct DiskHomeGuard {
    tmp: tempfile::TempDir,
    prev: Option<String>,
}

impl DiskHomeGuard {
    fn new() -> Self {
        let prev = std::env::var("LIBREFANG_HOME").ok();
        let tmp = tempfile::tempdir().expect("tempdir");
        // SAFETY: serialised via `ENV_LOCK`. Caller holds the lock.
        unsafe {
            std::env::set_var("LIBREFANG_HOME", tmp.path());
        }
        Self { tmp, prev }
    }

    fn home(&self) -> &Path {
        self.tmp.path()
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

/// Write a `config.toml` containing one `[[channels.telegram]]` per pair.
/// Used by the disk-roundtrip tests below.
fn write_telegram_instances(home: &Path, instances: &[&str]) {
    let mut content = String::new();
    for env_name in instances {
        content.push_str("[[channels.telegram]]\n");
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

    // Telegram MUST be present — it's the canonical adapter.
    let telegram = arr
        .iter()
        .find(|r| r["name"] == "telegram")
        .expect("telegram must appear in registry");
    assert_eq!(telegram["configured"], false);
}

#[tokio::test(flavor = "multi_thread")]
async fn channels_list_flips_configured_flag_when_seeded() {
    let channels = ChannelsConfig {
        telegram: OneOrMany(vec![TelegramConfig::default()]),
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
    let telegram = arr
        .iter()
        .find(|r| r["name"] == "telegram")
        .expect("telegram row");
    assert_eq!(
        telegram["configured"], true,
        "seeded telegram must report configured=true: {telegram}"
    );

    // Other rows must NOT be flipped just because telegram is configured.
    let discord = arr
        .iter()
        .find(|r| r["name"] == "discord")
        .expect("discord row");
    assert_eq!(discord["configured"], false);
}

// ---------------------------------------------------------------------------
// GET /api/channels/{name}
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn channels_get_returns_metadata_for_known_channel() {
    let h = boot().await;
    let (status, body) = json_request(&h, Method::GET, "/api/channels/telegram", None).await;
    assert_eq!(status, StatusCode::OK, "{body:?}");
    assert_eq!(body["name"], "telegram");
    assert_eq!(body["display_name"], "Telegram");
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
        "/api/channels/telegram/configure",
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
// Telegram bots, three Slack workspaces, etc. on the same channel type.
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
        json_request(&h, Method::GET, "/api/channels/telegram/instances", None).await;
    assert_eq!(status, StatusCode::OK, "{body:?}");
    assert_eq!(body["channel"], "telegram");
    assert_eq!(body["total"], 0);
    let arr = body["items"].as_array().expect("items must be array");
    assert!(arr.is_empty(), "no instances seeded → items must be empty");
}

#[tokio::test(flavor = "multi_thread")]
async fn channels_instances_list_returns_seeded_instances() {
    // Seed two telegram instances and assert both come through with
    // their configured fields and `index` values.
    let channels = ChannelsConfig {
        telegram: OneOrMany(vec![
            TelegramConfig {
                bot_token_env: "TG_SUPPORT".into(),
                ..TelegramConfig::default()
            },
            TelegramConfig {
                bot_token_env: "TG_OPS".into(),
                ..TelegramConfig::default()
            },
        ]),
        ..ChannelsConfig::default()
    };
    let h = boot_with_channels(channels).await;

    let (status, body) =
        json_request(&h, Method::GET, "/api/channels/telegram/instances", None).await;
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
        "/api/channels/telegram/instances",
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
        telegram: OneOrMany(vec![TelegramConfig::default()]),
        ..ChannelsConfig::default()
    };
    let h = boot_with_channels(channels).await;
    let (status, body) = json_request(
        &h,
        Method::PUT,
        "/api/channels/telegram/instances/7",
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
        telegram: OneOrMany(vec![TelegramConfig::default()]),
        ..ChannelsConfig::default()
    };
    let h = boot_with_channels(channels).await;
    let (status, body) = json_request(
        &h,
        Method::PUT,
        "/api/channels/telegram/instances/0",
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
        telegram: OneOrMany(vec![TelegramConfig::default()]),
        ..ChannelsConfig::default()
    };
    let h = boot_with_channels(channels).await;
    let (status, body) = json_request(
        &h,
        Method::PUT,
        "/api/channels/telegram/instances/0",
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
        telegram: OneOrMany(vec![TelegramConfig::default()]),
        ..ChannelsConfig::default()
    };
    let h = boot_with_channels(channels).await;
    // Post-#4865 DELETE requires `?signature=` for CAS — a stale value
    // here is fine, the range check fires first under the write lock.
    let (status, body) = json_request(
        &h,
        Method::DELETE,
        "/api/channels/telegram/instances/3?signature=stale",
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
        telegram: OneOrMany(vec![TelegramConfig::default()]),
        ..ChannelsConfig::default()
    };
    let h = boot_with_channels(channels).await;
    let (status, body) = json_request(
        &h,
        Method::DELETE,
        "/api/channels/telegram/instances/0",
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
// These tests own `LIBREFANG_HOME` (via `ENV_LOCK` + `DiskHomeGuard`) so
// they can seed an actual `config.toml` and drive the post-#4865 handler
// flow that re-reads disk under the `config_write_lock`. Cheaper unit-
// level coverage of the same primitives lives next to the helpers in
// `routes::channels::instance_helper_tests` and `routes::skills::tests`;
// these guard the HTTP-layer wiring.

#[tokio::test(flavor = "multi_thread")]
async fn channels_update_instance_signature_mismatch_returns_409() {
    let _lock = ENV_LOCK.lock().await;
    let guard = DiskHomeGuard::new();
    write_telegram_instances(guard.home(), &["TG_DISK_A"]);

    let h = boot_with_channels(ChannelsConfig {
        telegram: OneOrMany(vec![TelegramConfig {
            bot_token_env: "TG_DISK_A".into(),
            ..TelegramConfig::default()
        }]),
        ..ChannelsConfig::default()
    })
    .await;

    // PUT idx=0 with a deliberately stale signature. After #4865 the
    // handler re-reads disk, recomputes the signature for the current
    // disk-side instance, and rejects on mismatch with 409 Conflict.
    let (status, body) = json_request(
        &h,
        Method::PUT,
        "/api/channels/telegram/instances/0",
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
    let _lock = ENV_LOCK.lock().await;
    let guard = DiskHomeGuard::new();
    write_telegram_instances(guard.home(), &["TG_DISK_B", "TG_DISK_C"]);

    let h = boot_with_channels(ChannelsConfig {
        telegram: OneOrMany(vec![
            TelegramConfig {
                bot_token_env: "TG_DISK_B".into(),
                ..TelegramConfig::default()
            },
            TelegramConfig {
                bot_token_env: "TG_DISK_C".into(),
                ..TelegramConfig::default()
            },
        ]),
        ..ChannelsConfig::default()
    })
    .await;

    let (status, body) = json_request(
        &h,
        Method::DELETE,
        "/api/channels/telegram/instances/0?signature=ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
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
    let raw = std::fs::read_to_string(guard.home().join("config.toml")).expect("read config.toml");
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
    let _lock = ENV_LOCK.lock().await;
    let guard = DiskHomeGuard::new();
    write_telegram_instances(guard.home(), &["TG_DISK_D"]);

    let h = boot_with_channels(ChannelsConfig {
        telegram: OneOrMany(vec![TelegramConfig {
            bot_token_env: "TG_DISK_D".into(),
            ..TelegramConfig::default()
        }]),
        ..ChannelsConfig::default()
    })
    .await;

    // GET the list to obtain the server-computed signature for the row
    // we're about to update.
    let (list_status, list_body) =
        json_request(&h, Method::GET, "/api/channels/telegram/instances", None).await;
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
        "/api/channels/telegram/instances/0",
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
    let raw = std::fs::read_to_string(guard.home().join("config.toml")).expect("read config.toml");
    assert!(
        raw.contains("default_agent = \"rotated\""),
        "PUT must have written the new field to disk: {raw}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn channels_update_instance_clear_secrets_drops_orphan_env_var() {
    let _lock = ENV_LOCK.lock().await;
    let guard = DiskHomeGuard::new();
    write_telegram_instances(guard.home(), &["TG_LONELY"]);
    // Prime `secrets.env` with the env var the instance is pointing at,
    // so we can assert the cleanup loop actually removed it.
    std::fs::write(guard.home().join("secrets.env"), "TG_LONELY=fake-token\n")
        .expect("seed secrets.env");

    let h = boot_with_channels(ChannelsConfig {
        telegram: OneOrMany(vec![TelegramConfig {
            bot_token_env: "TG_LONELY".into(),
            ..TelegramConfig::default()
        }]),
        ..ChannelsConfig::default()
    })
    .await;

    let (_, list_body) =
        json_request(&h, Method::GET, "/api/channels/telegram/instances", None).await;
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
        "/api/channels/telegram/instances/0",
        Some(serde_json::json!({
            "fields": { "default_agent": "no-auth" },
            "signature": signature,
            "clear_secrets": ["bot_token_env"],
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body:?}");

    let cfg = std::fs::read_to_string(guard.home().join("config.toml")).expect("read config.toml");
    assert!(
        !cfg.contains("bot_token_env"),
        "cleared secret ref must be dropped from the rebuilt instance: {cfg}"
    );
    let secrets =
        std::fs::read_to_string(guard.home().join("secrets.env")).expect("read secrets.env");
    assert!(
        !secrets.contains("TG_LONELY"),
        "orphan env-var line must be scrubbed when no sibling references it: {secrets}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn channels_update_instance_clear_secrets_preserves_shared_env_var() {
    let _lock = ENV_LOCK.lock().await;
    let guard = DiskHomeGuard::new();
    // Two instances, BOTH pointing at the same env var (a possible
    // user setup if they hand-edited secrets.env). Clearing one
    // instance's ref must NOT remove the env var, since the sibling
    // is still using it.
    write_telegram_instances(guard.home(), &["TG_SHARED", "TG_SHARED"]);
    std::fs::write(guard.home().join("secrets.env"), "TG_SHARED=fake\n").expect("seed secrets.env");

    let h = boot_with_channels(ChannelsConfig {
        telegram: OneOrMany(vec![
            TelegramConfig {
                bot_token_env: "TG_SHARED".into(),
                ..TelegramConfig::default()
            },
            TelegramConfig {
                bot_token_env: "TG_SHARED".into(),
                ..TelegramConfig::default()
            },
        ]),
        ..ChannelsConfig::default()
    })
    .await;

    let (_, list_body) =
        json_request(&h, Method::GET, "/api/channels/telegram/instances", None).await;
    let signature = list_body["items"][0]["signature"]
        .as_str()
        .expect("signature")
        .to_string();

    let (status, body) = json_request(
        &h,
        Method::PUT,
        "/api/channels/telegram/instances/0",
        Some(serde_json::json!({
            "fields": { "default_agent": "no-auth" },
            "signature": signature,
            "clear_secrets": ["bot_token_env"],
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body:?}");

    let secrets =
        std::fs::read_to_string(guard.home().join("secrets.env")).expect("read secrets.env");
    assert!(
        secrets.contains("TG_SHARED"),
        "shared env var must survive — sibling instance still references it: {secrets}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn channels_list_includes_instance_count() {
    // The dashboard's card subtitle ("Telegram · 2 bots") depends on
    // `instance_count` riding alongside the existing `configured` flag.
    let channels = ChannelsConfig {
        telegram: OneOrMany(vec![
            TelegramConfig::default(),
            TelegramConfig::default(),
            TelegramConfig::default(),
        ]),
        ..ChannelsConfig::default()
    };
    let h = boot_with_channels(channels).await;
    let (status, body) = json_request(&h, Method::GET, "/api/channels", None).await;
    assert_eq!(status, StatusCode::OK);
    let arr = body["items"].as_array().expect("items");
    let telegram = arr
        .iter()
        .find(|r| r["name"] == "telegram")
        .expect("telegram row");
    assert_eq!(
        telegram["instance_count"], 3,
        "telegram seeded with 3 instances must report instance_count=3: {telegram}"
    );
    let discord = arr
        .iter()
        .find(|r| r["name"] == "discord")
        .expect("discord row");
    assert_eq!(
        discord["instance_count"], 0,
        "discord untouched must report instance_count=0: {discord}"
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
    // not exported `TELEGRAM_BOT_TOKEN`. The test harness never sets it,
    // and other tests in this file never call `set_var`, so the
    // pre-condition holds.
    if std::env::var("TELEGRAM_BOT_TOKEN").is_ok() {
        eprintln!(
            "skipping channels_test_known_channel_with_no_creds_reports_missing_env: \
             TELEGRAM_BOT_TOKEN is set in the environment"
        );
        return;
    }

    let h = boot().await;
    let (status, body) = json_request(&h, Method::POST, "/api/channels/telegram/test", None).await;
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
