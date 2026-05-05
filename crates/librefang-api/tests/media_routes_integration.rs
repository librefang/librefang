//! Integration tests for the `/media/*` HTTP surface.
//!
//! These exercise the real `media` router against a freshly-booted kernel
//! with no media-provider API keys configured. We focus on:
//!
//! 1. Validation paths in each `MediaError`-producing handler — the cheap,
//!    deterministic 4xx slice. No live network, no real provider keys.
//! 2. The `media_error_response` mapping (status code + `code` string).
//! 3. `GET /media/providers` — which must list every known provider with
//!    `configured: false` even when nothing is wired.
//! 4. `POST /media/transcribe` — content-type / body-size gates that fire
//!    before any driver call.
//! 5. `GET /media/video/{task_id}` — the missing-`provider` query gate
//!    and the unknown-provider path that `media_error_response` maps to 400.
//!
//! Mutating endpoints that need real binary fixtures or live driver calls
//! are intentionally NOT happy-pathed — they would either depend on
//! external API keys or pad the suite without exercising real logic.
//!
//! Refs #3571 (media slice).

use axum::body::Body;
use axum::http::{HeaderValue, Method, Request, StatusCode};
use axum::Router;
use librefang_api::routes::{self, AppState};
use librefang_testing::{MockKernelBuilder, TestAppState};
use std::sync::Arc;
use tower::ServiceExt;

struct Harness {
    app: Router,
    _state: Arc<AppState>,
    _test: TestAppState,
}

async fn boot() -> Harness {
    let test = TestAppState::with_builder(MockKernelBuilder::new().with_config(|cfg| {
        cfg.default_model = librefang_types::config::DefaultModelConfig {
            provider: "ollama".to_string(),
            model: "test-model".to_string(),
            api_key_env: "OLLAMA_API_KEY".to_string(),
            base_url: None,
            message_timeout_secs: 300,
            extra_params: std::collections::HashMap::new(),
            cli_profile_dirs: Vec::new(),
        };
    }));
    let state = test.state.clone();
    let app = Router::new()
        .nest("/api", routes::media::router())
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

async fn raw_request(
    h: &Harness,
    method: Method,
    path: &str,
    content_type: Option<&str>,
    body: Vec<u8>,
) -> (StatusCode, serde_json::Value) {
    let mut builder = Request::builder().method(method).uri(path);
    if let Some(ct) = content_type {
        builder = builder.header("content-type", HeaderValue::from_str(ct).unwrap());
    }
    let req = builder.body(Body::from(body)).unwrap();
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

// ── POST /media/image ───────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread")]
async fn media_image_rejects_empty_prompt() {
    let h = boot().await;
    let (status, body) = json_request(
        &h,
        Method::POST,
        "/api/media/image",
        Some(serde_json::json!({"prompt": ""})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "got: {body:?}");
    assert!(
        body["error"]["message"]
            .as_str()
            .unwrap_or("")
            .contains("empty"),
        "{body:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn media_image_rejects_invalid_count() {
    let h = boot().await;
    let (status, body) = json_request(
        &h,
        Method::POST,
        "/api/media/image",
        Some(serde_json::json!({"prompt": "a cat", "count": 99})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "got: {body:?}");
    assert!(body["error"]["message"]
        .as_str()
        .unwrap_or("")
        .contains("count"));
}

#[tokio::test(flavor = "multi_thread")]
async fn media_image_no_configured_provider_returns_missing_key() {
    // No API keys are exported in the test env — auto-detect must surface
    // a 422 `missing_key` rather than panic or 500.
    let h = boot().await;
    let (status, body) = json_request(
        &h,
        Method::POST,
        "/api/media/image",
        Some(serde_json::json!({"prompt": "a cat"})),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "got: {body:?}");
    assert_eq!(body["code"], "missing_key");
}

#[tokio::test(flavor = "multi_thread")]
async fn media_image_unknown_provider_returns_invalid_request() {
    let h = boot().await;
    let (status, body) = json_request(
        &h,
        Method::POST,
        "/api/media/image",
        Some(serde_json::json!({"prompt": "a cat", "provider": "definitely_not_a_provider"})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "got: {body:?}");
    assert_eq!(body["code"], "invalid_request");
}

// ── POST /media/speech ──────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread")]
async fn media_speech_rejects_empty_text() {
    let h = boot().await;
    let (status, body) = json_request(
        &h,
        Method::POST,
        "/api/media/speech",
        Some(serde_json::json!({"text": ""})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "got: {body:?}");
    assert!(body["error"]["message"]
        .as_str()
        .unwrap_or("")
        .contains("empty"));
}

#[tokio::test(flavor = "multi_thread")]
async fn media_speech_rejects_out_of_range_speed() {
    let h = boot().await;
    let (status, body) = json_request(
        &h,
        Method::POST,
        "/api/media/speech",
        Some(serde_json::json!({"text": "hello", "speed": 99.0})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "got: {body:?}");
    assert!(body["error"]["message"]
        .as_str()
        .unwrap_or("")
        .contains("speed"));
}

#[tokio::test(flavor = "multi_thread")]
async fn media_speech_no_configured_provider_returns_missing_key() {
    let h = boot().await;
    let (status, body) = json_request(
        &h,
        Method::POST,
        "/api/media/speech",
        Some(serde_json::json!({"text": "hello"})),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "got: {body:?}");
    assert_eq!(body["code"], "missing_key");
}

// ── POST /media/video ───────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread")]
async fn media_video_rejects_empty_prompt_and_no_image() {
    let h = boot().await;
    let (status, body) = json_request(
        &h,
        Method::POST,
        "/api/media/video",
        Some(serde_json::json!({"prompt": ""})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "got: {body:?}");
    assert!(body["error"]["message"]
        .as_str()
        .unwrap_or("")
        .to_lowercase()
        .contains("prompt"));
}

#[tokio::test(flavor = "multi_thread")]
async fn media_video_rejects_invalid_duration() {
    let h = boot().await;
    let (status, body) = json_request(
        &h,
        Method::POST,
        "/api/media/video",
        Some(serde_json::json!({"prompt": "a cat", "duration_secs": 9999})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "got: {body:?}");
    assert!(body["error"]["message"]
        .as_str()
        .unwrap_or("")
        .contains("duration"));
}

// ── GET /media/video/{task_id} ──────────────────────────────────────────

#[tokio::test(flavor = "multi_thread")]
async fn media_video_poll_requires_provider_query() {
    let h = boot().await;
    let (status, body) = json_request(&h, Method::GET, "/api/media/video/abc123", None).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "got: {body:?}");
    assert!(body["error"]["message"]
        .as_str()
        .unwrap_or("")
        .contains("provider"));
}

#[tokio::test(flavor = "multi_thread")]
async fn media_video_poll_unknown_provider_returns_400() {
    let h = boot().await;
    let (status, body) = json_request(
        &h,
        Method::GET,
        "/api/media/video/abc123?provider=definitely_not_a_provider",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "got: {body:?}");
    assert_eq!(body["code"], "invalid_request");
}

// ── POST /media/music ───────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread")]
async fn media_music_rejects_when_neither_prompt_nor_lyrics() {
    let h = boot().await;
    let (status, body) = json_request(
        &h,
        Method::POST,
        "/api/media/music",
        Some(serde_json::json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "got: {body:?}");
    assert!(body["error"]["message"]
        .as_str()
        .unwrap_or("")
        .to_lowercase()
        .contains("prompt"));
}

#[tokio::test(flavor = "multi_thread")]
async fn media_music_rejects_overlong_lyrics() {
    let h = boot().await;
    let huge = "la ".repeat(2000); // > 3500 chars
    let (status, body) = json_request(
        &h,
        Method::POST,
        "/api/media/music",
        Some(serde_json::json!({"lyrics": huge})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "got: {body:?}");
    assert!(body["error"]["message"]
        .as_str()
        .unwrap_or("")
        .contains("Lyrics"));
}

// ── GET /media/providers ────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread")]
async fn media_providers_lists_all_known_with_unconfigured_status() {
    // No API keys are set in this test env, so every known provider must
    // appear with `configured: false`. The list shape is what the dashboard
    // depends on to render the "Media providers" badge grid.
    let h = boot().await;
    let (status, body) = json_request(&h, Method::GET, "/api/media/providers", None).await;
    assert_eq!(status, StatusCode::OK, "got: {body:?}");
    let providers = body["providers"].as_array().expect("providers array");
    let names: Vec<&str> = providers
        .iter()
        .filter_map(|p| p["name"].as_str())
        .collect();
    for required in ["openai", "gemini", "elevenlabs", "minimax", "google_tts"] {
        assert!(
            names.contains(&required),
            "missing known provider '{required}' in: {names:?}"
        );
    }
    // None should be `configured` when the test env has no keys.
    for p in providers {
        assert_eq!(
            p["configured"], false,
            "unexpected configured=true with no API keys: {p}"
        );
    }
}

// ── POST /media/transcribe ──────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread")]
async fn media_transcribe_rejects_non_audio_content_type() {
    let h = boot().await;
    let (status, body) = raw_request(
        &h,
        Method::POST,
        "/api/media/transcribe",
        Some("text/plain"),
        b"not audio".to_vec(),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "got: {body:?}");
    assert!(body["error"]["message"]
        .as_str()
        .unwrap_or("")
        .contains("audio"));
}

#[tokio::test(flavor = "multi_thread")]
async fn media_transcribe_rejects_empty_body() {
    let h = boot().await;
    let (status, body) = raw_request(
        &h,
        Method::POST,
        "/api/media/transcribe",
        Some("audio/webm"),
        Vec::new(),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "got: {body:?}");
    assert!(body["error"]["message"]
        .as_str()
        .unwrap_or("")
        .to_lowercase()
        .contains("empty"));
}

#[tokio::test(flavor = "multi_thread")]
async fn media_transcribe_rejects_oversized_body() {
    // The handler explicitly caps at 10 MB. Axum's default `Bytes` extractor
    // cap may also fire before that — in either case we MUST get a 413 and
    // never let an outsized body reach the kernel transcription pipeline.
    let h = boot().await;
    let (status, _body) = raw_request(
        &h,
        Method::POST,
        "/api/media/transcribe",
        Some("audio/webm"),
        vec![0u8; 11 * 1024 * 1024],
    )
    .await;
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
}

#[tokio::test(flavor = "multi_thread")]
async fn media_transcribe_strips_content_type_parameters() {
    // `audio/webm;codecs=opus` must pass the `starts_with("audio/")` gate.
    // We don't have a working transcription driver in tests, so the call
    // will hit the kernel's MediaEngine and bubble back a 5xx — but the
    // status MUST NOT be 400 (the content-type gate has already passed).
    let h = boot().await;
    let (status, _body) = raw_request(
        &h,
        Method::POST,
        "/api/media/transcribe",
        Some("audio/webm;codecs=opus"),
        b"\x00\x01\x02fake-audio-bytes".to_vec(),
    )
    .await;
    assert_ne!(
        status,
        StatusCode::BAD_REQUEST,
        "audio/webm;codecs=opus must pass the content-type gate"
    );
}
