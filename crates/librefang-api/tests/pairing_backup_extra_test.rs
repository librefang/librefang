//! Integration tests for pairing/notify, pairing/devices listing, and the
//! full backup / restore family of routes in `routes::system`. Refs #3571
//! ("~80% of registered HTTP routes have no integration test").
//!
//! These tests intentionally avoid pretending to exercise real archive
//! roundtrips end-to-end — restore in particular only validates the 4xx
//! paths because a meaningful happy-path requires a fully-populated
//! kernel home with cron / hand_state / data dirs that the mock kernel
//! does not own. The validation paths are still where the actual
//! security-relevant logic lives (path traversal, extension check,
//! manifest presence), so coverage is concentrated there.
//!
//! Mounting strategy mirrors `pairing_test.rs`: `routes::system::router()`
//! nested under `/api`, driven by `tower::oneshot`. No auth middleware —
//! the system router itself enforces the `pairing.enabled` gate, which is
//! the behaviour these tests are checking.

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use axum::Router;
use librefang_api::routes::{self, AppState};
use librefang_testing::{MockKernelBuilder, TestAppState};
use std::sync::Arc;
use tower::ServiceExt;

struct Harness {
    app: Router,
    state: Arc<AppState>,
    _test: TestAppState,
}

fn default_model_cfg() -> librefang_types::config::DefaultModelConfig {
    librefang_types::config::DefaultModelConfig {
        provider: "ollama".to_string(),
        model: "test-model".to_string(),
        api_key_env: "OLLAMA_API_KEY".to_string(),
        base_url: None,
        message_timeout_secs: 300,
        extra_params: std::collections::HashMap::new(),
        cli_profile_dirs: Vec::new(),
    }
}

async fn boot(pairing_enabled: bool) -> Harness {
    let test = TestAppState::with_builder(MockKernelBuilder::new().with_config(move |cfg| {
        cfg.pairing = librefang_types::config::PairingConfig {
            enabled: pairing_enabled,
            public_base_url: Some("https://daemon.example.com".into()),
            ..librefang_types::config::PairingConfig::default()
        };
        cfg.default_model = default_model_cfg();
    }));
    let state = test.state.clone();
    let app = Router::new()
        .nest("/api", routes::system::router())
        .with_state(state.clone());
    Harness {
        app,
        state,
        _test: test,
    }
}

async fn json_post(
    h: &Harness,
    path: &str,
    body: serde_json::Value,
) -> (StatusCode, serde_json::Value) {
    let req = Request::builder()
        .method(Method::POST)
        .uri(path)
        .header("content-type", "application/json")
        .header("host", "test.local")
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

async fn get(h: &Harness, path: &str) -> (StatusCode, serde_json::Value) {
    let req = Request::builder()
        .method(Method::GET)
        .uri(path)
        .header("host", "test.local")
        .body(Body::empty())
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

async fn delete(h: &Harness, path: &str) -> (StatusCode, serde_json::Value) {
    let req = Request::builder()
        .method(Method::DELETE)
        .uri(path)
        .header("host", "test.local")
        .body(Body::empty())
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

// ---------------------------------------------------------------------------
// /api/pairing/devices (GET)
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn pairing_devices_returns_404_when_disabled() {
    let h = boot(false).await;
    let (status, _) = get(&h, "/api/pairing/devices").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test(flavor = "multi_thread")]
async fn pairing_devices_returns_empty_list_when_no_pairings() {
    let h = boot(true).await;
    let (status, body) = get(&h, "/api/pairing/devices").await;
    assert_eq!(status, StatusCode::OK, "got: {body:?}");
    let devices = body["devices"].as_array().expect("devices array");
    assert!(
        devices.is_empty(),
        "expected empty devices, got: {devices:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn pairing_devices_lists_paired_device_after_completion() {
    let h = boot(true).await;
    // Drive a real pairing flow so list_devices() has something to return.
    let (_, req) = json_post(&h, "/api/pairing/request", serde_json::json!({})).await;
    let token = req["token"].as_str().expect("token from request");
    let (status, _) = json_post(
        &h,
        "/api/pairing/complete",
        serde_json::json!({
            "token": token,
            "display_name": "iPad Pro",
            "platform": "ios",
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, body) = get(&h, "/api/pairing/devices").await;
    assert_eq!(status, StatusCode::OK);
    let devices = body["devices"].as_array().expect("devices array");
    assert_eq!(devices.len(), 1, "expected one paired device");
    assert_eq!(devices[0]["display_name"].as_str(), Some("iPad Pro"));
    assert_eq!(devices[0]["platform"].as_str(), Some("ios"));
    assert!(devices[0]["device_id"].as_str().is_some());
    assert!(devices[0]["paired_at"].as_str().is_some());
    assert!(devices[0]["last_seen"].as_str().is_some());
}

// ---------------------------------------------------------------------------
// /api/pairing/notify (POST)
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn pairing_notify_returns_404_when_disabled() {
    let h = boot(false).await;
    let (status, _) = json_post(
        &h,
        "/api/pairing/notify",
        serde_json::json!({"title": "x", "message": "y"}),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test(flavor = "multi_thread")]
async fn pairing_notify_rejects_missing_message() {
    let h = boot(true).await;
    let (status, body) = json_post(
        &h,
        "/api/pairing/notify",
        serde_json::json!({"title": "alert"}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "got: {body:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn pairing_notify_rejects_empty_message() {
    let h = boot(true).await;
    let (status, _) = json_post(
        &h,
        "/api/pairing/notify",
        serde_json::json!({"title": "alert", "message": ""}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test(flavor = "multi_thread")]
async fn pairing_notify_returns_zero_notified_with_no_devices() {
    let h = boot(true).await;
    let (status, body) = json_post(
        &h,
        "/api/pairing/notify",
        serde_json::json!({"title": "alert", "message": "hello"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "got: {body:?}");
    assert_eq!(body["ok"].as_bool(), Some(true));
    assert_eq!(body["notified"].as_u64(), Some(0));
}

// ---------------------------------------------------------------------------
// /api/backup, /api/backups, DELETE /api/backups/{filename}
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn list_backups_returns_empty_when_dir_missing() {
    let h = boot(true).await;
    let (status, body) = get(&h, "/api/backups").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["total"].as_u64(), Some(0));
    assert!(body["backups"].as_array().unwrap().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn create_backup_writes_archive_and_list_returns_it() {
    let h = boot(true).await;
    let (status, body) = json_post(&h, "/api/backup", serde_json::json!({})).await;
    assert_eq!(status, StatusCode::OK, "got: {body:?}");
    let filename = body["filename"]
        .as_str()
        .expect("filename present in create_backup response")
        .to_string();
    assert!(
        filename.starts_with("librefang_backup_") && filename.ends_with(".zip"),
        "unexpected filename: {filename}"
    );
    assert!(body["size_bytes"].as_u64().unwrap_or(0) > 0);

    // The created file must actually be on disk under the kernel's home_dir/backups.
    let backups_dir = h.state.kernel.home_dir().join("backups");
    let on_disk = backups_dir.join(&filename);
    assert!(on_disk.exists(), "backup file missing on disk: {on_disk:?}");

    // GET /api/backups must surface the new archive with a populated manifest.
    let (status, body) = get(&h, "/api/backups").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["total"].as_u64(), Some(1));
    let entry = &body["backups"][0];
    assert_eq!(entry["filename"].as_str(), Some(filename.as_str()));
    assert!(entry["librefang_version"].as_str().is_some());
}

#[tokio::test(flavor = "multi_thread")]
async fn delete_backup_rejects_path_traversal() {
    let h = boot(true).await;
    let (status, _) = delete(&h, "/api/backups/..%2Fetc%2Fpasswd").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test(flavor = "multi_thread")]
async fn delete_backup_rejects_non_zip_extension() {
    let h = boot(true).await;
    let (status, _) = delete(&h, "/api/backups/notes.txt").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test(flavor = "multi_thread")]
async fn delete_backup_returns_404_for_missing_file() {
    let h = boot(true).await;
    let (status, _) = delete(&h, "/api/backups/librefang_backup_19700101_000000.zip").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test(flavor = "multi_thread")]
async fn delete_backup_removes_existing_archive() {
    let h = boot(true).await;
    // Create a backup so we have a real file to delete.
    let (status, body) = json_post(&h, "/api/backup", serde_json::json!({})).await;
    assert_eq!(status, StatusCode::OK);
    let filename = body["filename"].as_str().unwrap().to_string();

    let (status, _) = delete(&h, &format!("/api/backups/{filename}")).await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    // Subsequent listing must no longer include it.
    let (_, body) = get(&h, "/api/backups").await;
    assert_eq!(body["total"].as_u64(), Some(0));
    let on_disk = h.state.kernel.home_dir().join("backups").join(&filename);
    assert!(!on_disk.exists(), "file should be gone: {on_disk:?}");
}

// ---------------------------------------------------------------------------
// /api/restore (POST) — validation paths only.
// A meaningful happy-path roundtrip needs a populated home_dir + restart
// semantics that the mock kernel cannot replicate, so we cover the four
// 4xx branches that are the actual security-relevant logic.
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn restore_rejects_missing_filename_field() {
    let h = boot(true).await;
    let (status, _) = json_post(&h, "/api/restore", serde_json::json!({})).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test(flavor = "multi_thread")]
async fn restore_rejects_path_traversal_filename() {
    let h = boot(true).await;
    let (status, _) = json_post(
        &h,
        "/api/restore",
        serde_json::json!({"filename": "../etc/passwd.zip"}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test(flavor = "multi_thread")]
async fn restore_rejects_non_zip_extension() {
    let h = boot(true).await;
    let (status, _) = json_post(
        &h,
        "/api/restore",
        serde_json::json!({"filename": "leak.tar"}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test(flavor = "multi_thread")]
async fn restore_returns_404_when_archive_missing() {
    let h = boot(true).await;
    let (status, _) = json_post(
        &h,
        "/api/restore",
        serde_json::json!({"filename": "librefang_backup_19700101_000000.zip"}),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}
