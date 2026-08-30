//! Integration tests for pairing/notify, pairing/devices listing, and the
//! full backup / restore family of routes in `routes::system`. Refs #3571
//! ("~80% of registered HTTP routes have no integration test").
//!
//! Two kinds of coverage sit here. The 4xx paths (path traversal, extension
//! check, manifest presence, component-selection validation) are cheap and are
//! where the security-relevant decisions are made. The happy paths seed the
//! mock kernel's home with one file per backup component and drive a real
//! create-then-restore round trip, asserting on the files that end up on disk —
//! that is the only way to catch a restore which answers `200` after writing
//! nothing, or which writes to a different path than the one it archived from.
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
        extra_params: std::collections::BTreeMap::new(),
        cli_profile_dirs: Vec::new(),
    }
}

/// Surface the restore handler's own diagnostics in the test output.
///
/// A partial restore answers `500` with nothing but `restored_files` and `error_count` — the per-entry reasons go to `tracing::error!`, and with no subscriber installed a failure here says only that two entries failed, never which two or why.
/// That is exactly the shape of the Windows-only failure this harness could not diagnose from CI logs.
fn capture_restore_diagnostics() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let _ = tracing_subscriber::fmt()
            .with_max_level(tracing::Level::ERROR)
            .with_test_writer()
            .try_init();
    });
}

async fn boot(pairing_enabled: bool) -> Harness {
    capture_restore_diagnostics();
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
async fn list_backups_reports_unreadable_backup_path() {
    let h = boot(true).await;
    let backups_path = h.state.kernel.home_dir().join("backups");
    std::fs::write(&backups_path, b"not a directory").expect("create invalid backup path");

    let (status, body) = get(&h, "/api/backups").await;

    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(body["message"].as_str(), Some("Internal server error"));
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

/// Refs `docs/issues/blocking-fs-on-executor.md` — `create_backup`
/// must dispatch its `walkdir` / `std::fs::read` work onto
/// `tokio::task::spawn_blocking` so a large backup walk doesn't
/// stall the axum worker. We can't directly probe for
/// "did spawn_blocking get called" without poking internals, but we
/// can assert the behavioural invariant: while a backup is in
/// flight, another request submitted to the same router must make
/// progress and complete. Pre-fix, the in-flight handler held the
/// worker, so on a 2-worker runtime two concurrent backups would
/// serialise (and on a 1-worker runtime the test would deadlock).
/// With `spawn_blocking` the second request hops off onto a fresh
/// worker thread immediately.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn create_backup_does_not_block_other_handlers() {
    let h = boot(true).await;
    let app1 = h.app.clone();
    let app2 = h.app.clone();

    // Kick off a backup. Don't await it yet — we want a second
    // request to overlap.
    let backup_task = tokio::spawn(async move {
        let req = Request::builder()
            .method(Method::POST)
            .uri("/api/backup")
            .header("content-type", "application/json")
            .header("host", "test.local")
            .body(Body::from("{}"))
            .unwrap();
        app1.oneshot(req).await.unwrap()
    });

    // Concurrent listing must complete, with a generous-but-still-
    // bounded timeout. If the backup ever migrates back onto the
    // executor synchronously, this race tightens against the worker
    // budget and starts flaking under load.
    let list_req = Request::builder()
        .method(Method::GET)
        .uri("/api/backups")
        .header("host", "test.local")
        .body(Body::empty())
        .unwrap();
    let list_resp = tokio::time::timeout(std::time::Duration::from_secs(5), app2.oneshot(list_req))
        .await
        .expect("GET /api/backups must complete while a backup is in flight")
        .unwrap();
    assert_eq!(list_resp.status(), StatusCode::OK);

    // Backup itself eventually completes.
    let backup_resp = backup_task.await.unwrap();
    assert_eq!(backup_resp.status(), StatusCode::OK);
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

    let audit_entry = h
        .state
        .kernel
        .audit()
        .recent(50)
        .into_iter()
        .find(|entry| entry.detail == format!("Backup deleted: {filename}"))
        .expect("successful backup deletion must be audited");
    assert!(matches!(
        audit_entry.action,
        librefang_kernel::audit::AuditAction::ConfigChange
    ));
    assert_eq!(audit_entry.outcome, "completed");
}

// ---------------------------------------------------------------------------
// /api/restore (POST) — filename validation, before the archive is opened.
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

// ---------------------------------------------------------------------------
// /api/restore (POST) — selective restore onto an existing system.
//
// `keep_config` and `components` are what make a backup restorable onto a
// machine that is already running: clone mode keeps the target's own key,
// port and paths, and the component list keeps a restore from dragging in
// state the operator did not ask for. Both are decided inside the extraction
// loop, so the only honest assertion is on the files that end up on disk.
// ---------------------------------------------------------------------------

/// Where `create_backup` reads the `agents` component from, and therefore the
/// only place a restore of that component may land: the archive stores the tree
/// under the `agents/` prefix, but the prefix is a component name, not a path.
fn agent_workspace_file(home: &std::path::Path) -> std::path::PathBuf {
    home.join("workspaces")
        .join("agents")
        .join("scout")
        .join("agent.toml")
}

/// Seed the kernel home with one file per component the classifier
/// recognises, so a round-trip can distinguish "restored", "skipped by
/// keep_config" and "skipped by the component filter".
fn seed_home(home: &std::path::Path) {
    std::fs::write(home.join("config.toml"), b"origin = \"backup\"\n").expect("write config.toml");
    std::fs::create_dir_all(home.join("data")).expect("mkdir data");
    std::fs::write(
        home.join("data").join("cron_jobs.json"),
        b"[\"from-backup\"]",
    )
    .expect("write cron_jobs.json");
    std::fs::write(home.join("data").join("hand_state.json"), b"{\"h\":1}")
        .expect("write hand_state.json");
    std::fs::write(home.join("data").join("custom_models.json"), b"{\"m\":1}")
        .expect("write custom_models.json");
    // A `data/` entry no named component owns, so the `data` selection can be
    // told apart from the three components that live inside `data/`.
    std::fs::write(home.join("data").join("memory.sqlite"), b"sqlite-bytes")
        .expect("write data/memory.sqlite");
    std::fs::create_dir_all(home.join("skills")).expect("mkdir skills");
    std::fs::write(home.join("skills").join("from-backup.md"), b"# skill")
        .expect("write skills entry");
    let agent_file = agent_workspace_file(home);
    std::fs::create_dir_all(agent_file.parent().expect("agent dir"))
        .expect("mkdir agent workspace");
    std::fs::write(&agent_file, b"name = \"scout\"\n").expect("write agent.toml");
}

async fn create_backup_of_seeded_home(h: &Harness) -> String {
    seed_home(h.state.kernel.home_dir());
    let (status, body) = json_post(h, "/api/backup", serde_json::json!({})).await;
    assert_eq!(status, StatusCode::OK, "backup failed: {body:?}");
    body["filename"]
        .as_str()
        .expect("filename present in create_backup response")
        .to_string()
}

#[tokio::test(flavor = "multi_thread")]
async fn restore_without_options_writes_every_component_back() {
    let h = boot(true).await;
    let filename = create_backup_of_seeded_home(&h).await;
    let home = h.state.kernel.home_dir().to_path_buf();

    // Diverge the live system from the archive.
    std::fs::write(home.join("config.toml"), b"origin = \"local\"\n").expect("overwrite config");
    std::fs::remove_file(home.join("data").join("cron_jobs.json")).expect("remove cron_jobs");
    std::fs::remove_file(home.join("skills").join("from-backup.md")).expect("remove skill");

    let (status, body) = json_post(
        &h,
        "/api/restore",
        serde_json::json!({"filename": filename}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "restore failed: {body:?}");

    assert_eq!(
        std::fs::read_to_string(home.join("config.toml")).expect("config.toml back"),
        "origin = \"backup\"\n",
        "a restore with no options must overwrite config.toml"
    );
    assert!(home.join("data").join("cron_jobs.json").exists());
    assert!(home.join("skills").join("from-backup.md").exists());
}

#[tokio::test(flavor = "multi_thread")]
async fn restore_honours_keep_config_and_the_component_selection() {
    let h = boot(true).await;
    let filename = create_backup_of_seeded_home(&h).await;
    let home = h.state.kernel.home_dir().to_path_buf();

    std::fs::write(home.join("config.toml"), b"origin = \"local\"\n").expect("overwrite config");
    std::fs::remove_file(home.join("data").join("cron_jobs.json")).expect("remove cron_jobs");
    std::fs::remove_file(home.join("skills").join("from-backup.md")).expect("remove skill");

    // `config` is selected *and* `keep_config` is set, so the skip is
    // attributable to clone mode rather than to the component filter.
    let (status, body) = json_post(
        &h,
        "/api/restore",
        serde_json::json!({
            "filename": filename,
            "keep_config": true,
            "components": ["config", "cron_jobs"],
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "restore failed: {body:?}");

    assert_eq!(
        std::fs::read_to_string(home.join("config.toml")).expect("config.toml still there"),
        "origin = \"local\"\n",
        "keep_config must leave the target's own config.toml untouched"
    );
    assert!(
        home.join("data").join("cron_jobs.json").exists(),
        "a selected component must be restored"
    );
    assert!(
        !home.join("skills").join("from-backup.md").exists(),
        "a classified component that was not selected must be skipped"
    );
}

// ---------------------------------------------------------------------------
// /api/restore (POST) — component-selection validation and classification.
//
// A restore that writes nothing and answers 200 is the worst failure this
// endpoint has: the operator believes their data is back. Every way of
// mis-stating the selection therefore has to be a 4xx, and every accepted
// selection has to mean the same set of files on the way in as on the way out.
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn restore_rejects_an_empty_component_list() {
    let h = boot(true).await;
    let filename = create_backup_of_seeded_home(&h).await;
    let home = h.state.kernel.home_dir().to_path_buf();
    std::fs::write(home.join("config.toml"), b"origin = \"local\"\n").expect("overwrite config");

    let (status, body) = json_post(
        &h,
        "/api/restore",
        serde_json::json!({"filename": filename, "components": []}),
    )
    .await;

    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "`components: []` must be rejected, not read as a selection of nothing: {body:?}"
    );
    // And nothing may have been written on the way to that rejection.
    assert_eq!(
        std::fs::read_to_string(home.join("config.toml")).expect("config.toml untouched"),
        "origin = \"local\"\n"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn restore_rejects_an_unknown_component_name() {
    let h = boot(true).await;
    let filename = create_backup_of_seeded_home(&h).await;

    // "agent" for "agents" — a typo that used to restore nothing and report 200.
    let (status, body) = json_post(
        &h,
        "/api/restore",
        serde_json::json!({"filename": filename, "components": ["agent"]}),
    )
    .await;

    assert_eq!(status, StatusCode::BAD_REQUEST, "got: {body:?}");
    let message = body["message"].as_str().expect("message in error body");
    assert!(
        message.contains("agent"),
        "the error must name the unrecognised value: {message}"
    );
    assert!(
        message.contains("agents") && message.contains("custom_models"),
        "the error must list the valid components: {message}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn restore_rejects_a_components_field_that_is_not_a_string_array() {
    let h = boot(true).await;
    let filename = create_backup_of_seeded_home(&h).await;

    // A bare string used to fall through `as_array()` to `None`, i.e. "no
    // filter" — a full overwrite from a request that asked for one component.
    let (status, body) = json_post(
        &h,
        "/api/restore",
        serde_json::json!({"filename": filename.clone(), "components": "data"}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "got: {body:?}");

    // Non-string elements used to be filtered out, collapsing to an empty
    // selection.
    let (status, body) = json_post(
        &h,
        "/api/restore",
        serde_json::json!({"filename": filename, "components": [7]}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "got: {body:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn restore_rejects_a_keep_config_that_is_not_a_boolean() {
    let h = boot(true).await;
    let filename = create_backup_of_seeded_home(&h).await;
    let home = h.state.kernel.home_dir().to_path_buf();
    std::fs::write(home.join("config.toml"), b"origin = \"local\"\n").expect("overwrite config");

    // `"true"` used to coerce to `false`, overwriting the very config.toml the
    // caller was asking to keep.
    let (status, body) = json_post(
        &h,
        "/api/restore",
        serde_json::json!({"filename": filename, "keep_config": "true"}),
    )
    .await;

    assert_eq!(status, StatusCode::BAD_REQUEST, "got: {body:?}");
    assert_eq!(
        std::fs::read_to_string(home.join("config.toml")).expect("config.toml untouched"),
        "origin = \"local\"\n",
        "a rejected keep_config must not have overwritten the config"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn restore_of_the_data_component_covers_every_data_entry() {
    let h = boot(true).await;
    let filename = create_backup_of_seeded_home(&h).await;
    let home = h.state.kernel.home_dir().to_path_buf();
    let data = home.join("data");

    for name in [
        "cron_jobs.json",
        "hand_state.json",
        "custom_models.json",
        "memory.sqlite",
    ] {
        std::fs::remove_file(data.join(name)).unwrap_or_else(|e| panic!("remove {name}: {e}"));
    }

    let (status, body) = json_post(
        &h,
        "/api/restore",
        serde_json::json!({"filename": filename, "components": ["data"]}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "restore failed: {body:?}");

    // The three files with a component name of their own live inside `data/`,
    // so `data` has to cover them. A first-match-wins classifier left exactly
    // these three behind.
    for name in [
        "cron_jobs.json",
        "hand_state.json",
        "custom_models.json",
        "memory.sqlite",
    ] {
        assert!(
            data.join(name).exists(),
            "components: [\"data\"] must restore data/{name}; response: {body:?}"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn restore_round_trips_an_agent_workspace_file_to_its_original_path() {
    let h = boot(true).await;
    let filename = create_backup_of_seeded_home(&h).await;
    let home = h.state.kernel.home_dir().to_path_buf();
    let agent_file = agent_workspace_file(&home);

    std::fs::remove_file(&agent_file).expect("remove agent.toml");

    let (status, body) = json_post(
        &h,
        "/api/restore",
        serde_json::json!({"filename": filename}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "restore failed: {body:?}");

    // `create_backup` reads this tree from `<home>/workspaces/agents/` and
    // stores it under the archive prefix `agents/`; restore wrote it to
    // `<home>/agents/` instead, so the component never came back.
    assert_eq!(
        std::fs::read_to_string(&agent_file).unwrap_or_default(),
        "name = \"scout\"\n",
        "the agents component must restore to the path it was archived from"
    );
    assert!(
        !home.join("agents").exists(),
        "nothing may be written to the pre-unification legacy <home>/agents/ layout"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn restore_of_the_agents_component_alone_restores_the_agent_workspace() {
    let h = boot(true).await;
    let filename = create_backup_of_seeded_home(&h).await;
    let home = h.state.kernel.home_dir().to_path_buf();
    let agent_file = agent_workspace_file(&home);

    std::fs::remove_file(&agent_file).expect("remove agent.toml");
    std::fs::remove_file(home.join("skills").join("from-backup.md")).expect("remove skill");

    let (status, body) = json_post(
        &h,
        "/api/restore",
        serde_json::json!({"filename": filename, "components": ["agents"]}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "restore failed: {body:?}");

    assert!(
        agent_file.exists(),
        "components: [\"agents\"] must restore the agent workspace"
    );
    assert!(
        !home.join("skills").join("from-backup.md").exists(),
        "a component that was not selected must stay unrestored"
    );
}

/// The overlapping scopes are what made this fail silently: `zip` refuses a
/// duplicate entry name, so re-archiving `data/cron_jobs.json` as part of the
/// `data/` walk aborted that walk and dropped the rest of the tree — the
/// database included — while `POST /api/backup` still answered `200`.
#[tokio::test(flavor = "multi_thread")]
async fn create_backup_archives_the_data_tree_and_the_named_json_files() {
    let h = boot(true).await;
    seed_home(h.state.kernel.home_dir());

    let (status, body) = json_post(&h, "/api/backup", serde_json::json!({})).await;
    assert_eq!(status, StatusCode::OK, "backup failed: {body:?}");

    let components: Vec<&str> = body["components"]
        .as_array()
        .expect("components array")
        .iter()
        .filter_map(|c| c.as_str())
        .collect();
    for expected in [
        "config",
        "cron_jobs",
        "hand_state",
        "custom_models",
        "agents",
        "skills",
        "data",
    ] {
        assert!(
            components.contains(&expected),
            "{expected} missing from the archive's components: {components:?}"
        );
    }
}
