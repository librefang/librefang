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
//!   channel with no env credentials, returns 200 with
//!   `status="error"` + a "Missing required env vars" message (the
//!   handler intentionally returns 200 so dashboards can render the
//!   diagnostic without an HTTP error path).

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use axum::Router;
use librefang_api::routes::{self, AppState};
use librefang_testing::{MockKernelBuilder, TestAppState};
use librefang_types::config::{ChannelsConfig, OneOrMany, TelegramConfig};
use std::sync::Arc;
use tower::ServiceExt;

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
    let err = body["error"].as_str().unwrap_or("");
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
        body["error"]
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
        body["error"].as_str().unwrap_or("").contains("fields"),
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
        body["error"]
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
    assert_eq!(body["status"], "error");
    assert_eq!(body["message"], "Unknown channel");
}

#[tokio::test(flavor = "multi_thread")]
async fn channels_test_known_channel_with_no_creds_reports_missing_env() {
    // The handler returns 200 even when credentials are missing — the
    // diagnostic lives in the JSON body so the dashboard can render it
    // without a separate error pipeline. Telegram requires
    // `TELEGRAM_BOT_TOKEN`, which is not set in this test process.
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
    assert_eq!(status, StatusCode::OK, "{body:?}");
    assert_eq!(
        body["status"], "error",
        "missing creds must surface as status=error: {body}"
    );
    let msg = body["message"].as_str().unwrap_or("");
    assert!(
        msg.contains("Missing required env vars"),
        "message must call out missing env vars: {body}"
    );
}
