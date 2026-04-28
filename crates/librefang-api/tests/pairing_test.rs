//! Integration tests for the mobile-pairing endpoints.
//!
//! Exercises the full request → complete → authenticated-call → revoke
//! flow against a freshly-booted kernel + mounted `system::router()`.
//! The deliberate goal is to catch the kind of failures unit tests miss:
//!
//! - The complete endpoint must mint a fresh per-device bearer and add
//!   it to `state.user_api_keys`, otherwise the device is "paired but
//!   can't authenticate".
//! - DELETE must drop the bearer from the live auth table — purging
//!   only the persisted row would leave the in-memory `Vec<ApiUserAuth>`
//!   accepting the token until the next restart.
//! - Re-using a one-time token must return 410 Gone, never 400.

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use axum::Router;
use librefang_api::routes::{self, AppState};
use librefang_kernel::LibreFangKernel;
use librefang_types::config::{DefaultModelConfig, KernelConfig, PairingConfig};
use std::sync::Arc;
use std::time::Instant;
use tower::ServiceExt;

struct Harness {
    app: Router,
    state: Arc<AppState>,
    _tmp: tempfile::TempDir,
}

async fn boot_with_pairing(enabled: bool, public_base_url: Option<String>) -> Harness {
    let tmp = tempfile::tempdir().expect("temp dir");
    let config = KernelConfig {
        home_dir: tmp.path().to_path_buf(),
        data_dir: tmp.path().join("data"),
        pairing: PairingConfig {
            enabled,
            public_base_url,
            ..PairingConfig::default()
        },
        default_model: DefaultModelConfig {
            provider: "ollama".to_string(),
            model: "test-model".to_string(),
            api_key_env: "OLLAMA_API_KEY".to_string(),
            base_url: None,
            message_timeout_secs: 300,
            extra_params: std::collections::HashMap::new(),
            cli_profile_dirs: Vec::new(),
        },
        ..KernelConfig::default()
    };
    let kernel = LibreFangKernel::boot_with_config(config).expect("boot");
    let kernel = Arc::new(kernel);
    kernel.set_self_handle();

    let state = Arc::new(AppState {
        kernel,
        started_at: Instant::now(),
        peer_registry: None,
        bridge_manager: tokio::sync::Mutex::new(None),
        channels_config: tokio::sync::RwLock::new(Default::default()),
        shutdown_notify: Arc::new(tokio::sync::Notify::new()),
        clawhub_cache: dashmap::DashMap::new(),
        skillhub_cache: dashmap::DashMap::new(),
        provider_probe_cache: librefang_runtime::provider_health::ProbeCache::new(),
        webhook_store: librefang_api::webhook_store::WebhookStore::load(std::env::temp_dir().join(
            format!("librefang-test-pairing-{}.json", uuid::Uuid::new_v4()),
        )),
        active_sessions: Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
        #[cfg(feature = "telemetry")]
        prometheus_handle: None,
        media_drivers: librefang_runtime::media::MediaDriverCache::new(),
        webhook_router: Arc::new(tokio::sync::RwLock::new(Arc::new(axum::Router::new()))),
        api_key_lock: Arc::new(tokio::sync::RwLock::new(String::new())),
        user_api_keys: Arc::new(tokio::sync::RwLock::new(Vec::new())),
        provider_test_cache: dashmap::DashMap::new(),
        config_write_lock: tokio::sync::Mutex::new(()),
        pending_a2a_agents: dashmap::DashMap::new(),
        auth_login_limiter: std::sync::Arc::new(librefang_api::rate_limiter::AuthLoginLimiter::new(0)),
    });

    let app = Router::new()
        .nest("/api", routes::system::router())
        .with_state(state.clone());

    Harness {
        app,
        state,
        _tmp: tmp,
    }
}

async fn json_post(
    h: &Harness,
    path: &str,
    body: serde_json::Value,
    headers: &[(&str, &str)],
) -> (StatusCode, serde_json::Value) {
    let mut builder = Request::builder()
        .method(Method::POST)
        .uri(path)
        .header("content-type", "application/json")
        .header("host", "test.local");
    for (k, v) in headers {
        builder = builder.header(*k, *v);
    }
    let req = builder
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap();
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

async fn delete_request(h: &Harness, path: &str) -> StatusCode {
    let req = Request::builder()
        .method(Method::DELETE)
        .uri(path)
        .header("host", "test.local")
        .body(Body::empty())
        .unwrap();
    h.app.clone().oneshot(req).await.unwrap().status()
}

#[tokio::test(flavor = "multi_thread")]
async fn request_returns_404_when_disabled() {
    let h = boot_with_pairing(false, None).await;
    let (status, _) = json_post(&h, "/api/pairing/request", serde_json::json!({}), &[]).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test(flavor = "multi_thread")]
async fn request_emits_qr_uri_with_base64_payload() {
    let h = boot_with_pairing(true, Some("https://daemon.example.com".into())).await;
    let (status, body) = json_post(&h, "/api/pairing/request", serde_json::json!({}), &[]).await;
    assert_eq!(status, StatusCode::OK, "got body: {body:?}");
    let qr = body["qr_uri"].as_str().expect("qr_uri string");
    assert!(qr.starts_with("librefang://pair?payload="), "qr_uri = {qr}");
    assert!(body["token"].as_str().is_some());
    assert!(body["expires_at"].as_str().is_some());
}

#[tokio::test(flavor = "multi_thread")]
async fn complete_mints_bearer_and_registers_in_auth_table() {
    let h = boot_with_pairing(true, Some("https://daemon.example.com".into())).await;
    let (status, req) = json_post(&h, "/api/pairing/request", serde_json::json!({}), &[]).await;
    assert_eq!(status, StatusCode::OK);
    let token = req["token"].as_str().unwrap();

    let (status, complete) = json_post(
        &h,
        "/api/pairing/complete",
        serde_json::json!({
            "token": token,
            "display_name": "iPhone 15",
            "platform": "ios",
        }),
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "got: {complete:?}");
    let api_key = complete["api_key"].as_str().expect("plaintext api_key");
    let device_id = complete["device_id"].as_str().expect("device_id");

    // Plaintext key must be 64 hex chars (32 bytes from rand::random).
    assert_eq!(api_key.len(), 64);
    assert!(api_key.chars().all(|c| c.is_ascii_hexdigit()));

    // The auth table must now carry exactly one `device:{id}` entry.
    let auth_table = h.state.user_api_keys.read().await;
    let entry = auth_table
        .iter()
        .find(|u| u.name == format!("device:{device_id}"))
        .expect("device bearer must be registered");
    // The persisted hash must verify the freshly-issued plaintext.
    assert!(librefang_api::password_hash::verify_password(
        api_key,
        &entry.api_key_hash
    ));
    // And the hash format is the SHA-256 dispatch path, not Argon2 — ensures
    // the optimisation actually shipped to the device write path.
    assert!(
        entry.api_key_hash.starts_with("$sha256$"),
        "expected SHA-256 hash, got: {}",
        entry.api_key_hash
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn complete_rejects_reused_token_with_410() {
    let h = boot_with_pairing(true, Some("https://daemon.example.com".into())).await;
    let (_, req) = json_post(&h, "/api/pairing/request", serde_json::json!({}), &[]).await;
    let token = req["token"].as_str().unwrap().to_string();

    let payload = serde_json::json!({
        "token": token,
        "display_name": "first",
        "platform": "ios",
    });
    let (status1, _) = json_post(&h, "/api/pairing/complete", payload.clone(), &[]).await;
    assert_eq!(status1, StatusCode::OK);

    // Same token, second time → must be 410 Gone, not 400. The mobile
    // wizard branches on this status to surface "QR expired or already
    // used" specifically.
    let (status2, _) = json_post(&h, "/api/pairing/complete", payload, &[]).await;
    assert_eq!(status2, StatusCode::GONE);
}

#[tokio::test(flavor = "multi_thread")]
async fn complete_rejects_empty_token() {
    let h = boot_with_pairing(true, Some("https://daemon.example.com".into())).await;
    let (status, _) = json_post(
        &h,
        "/api/pairing/complete",
        serde_json::json!({"token": ""}),
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test(flavor = "multi_thread")]
async fn remove_device_drops_bearer_from_live_auth_table() {
    let h = boot_with_pairing(true, Some("https://daemon.example.com".into())).await;
    let (_, req) = json_post(&h, "/api/pairing/request", serde_json::json!({}), &[]).await;
    let token = req["token"].as_str().unwrap();
    let (_, complete) = json_post(
        &h,
        "/api/pairing/complete",
        serde_json::json!({"token": token, "display_name": "d", "platform": "ios"}),
        &[],
    )
    .await;
    let device_id = complete["device_id"].as_str().unwrap().to_string();
    assert_eq!(h.state.user_api_keys.read().await.len(), 1);

    let status = delete_request(&h, &format!("/api/pairing/devices/{device_id}")).await;
    assert_eq!(status, StatusCode::OK);

    // The bearer must be gone — otherwise a "revoked" device's stored
    // key would keep authenticating until the next process restart.
    assert_eq!(
        h.state.user_api_keys.read().await.len(),
        0,
        "device entry must be evicted from the live auth table"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn request_falls_back_to_host_header_when_no_public_base_url() {
    let h = boot_with_pairing(true, None).await;
    let (status, body) = json_post(&h, "/api/pairing/request", serde_json::json!({}), &[]).await;
    assert_eq!(status, StatusCode::OK, "got: {body:?}");
    let qr = body["qr_uri"].as_str().unwrap();
    // Decode the base64url payload and confirm base_url honoured the
    // Host header (falls back to plain http when no X-Forwarded-Proto).
    let payload_b64 = qr.strip_prefix("librefang://pair?payload=").unwrap();
    // Decoder must match the encoder used in pairing_request — URL_SAFE_NO_PAD.
    let bytes = base64::Engine::decode(
        &base64::engine::general_purpose::URL_SAFE_NO_PAD,
        payload_b64.as_bytes(),
    )
    .expect("decode payload");
    let parsed: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(parsed["base_url"].as_str().unwrap(), "http://test.local");
}

#[tokio::test(flavor = "multi_thread")]
async fn request_honors_x_forwarded_proto_when_no_public_base_url() {
    let h = boot_with_pairing(true, None).await;
    let (status, body) = json_post(
        &h,
        "/api/pairing/request",
        serde_json::json!({}),
        &[("x-forwarded-proto", "https")],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "got: {body:?}");
    let qr = body["qr_uri"].as_str().unwrap();
    let payload_b64 = qr.strip_prefix("librefang://pair?payload=").unwrap();
    // Decoder must match the encoder used in pairing_request — URL_SAFE_NO_PAD.
    let bytes = base64::Engine::decode(
        &base64::engine::general_purpose::URL_SAFE_NO_PAD,
        payload_b64.as_bytes(),
    )
    .expect("decode payload");
    let parsed: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(parsed["base_url"].as_str().unwrap(), "https://test.local");
}
