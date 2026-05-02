//! Integration tests for the `inbox` route domain (#3571).
//!
//! Inbox is currently a single-endpoint domain: `GET /api/inbox/status`. The
//! tests below exercise the registered HTTP route end-to-end against a real
//! kernel booted in a tempdir, covering:
//!
//! 1. Default config (disabled, default poll interval, no default agent).
//! 2. Enabled config with a custom directory containing pending text files
//!    plus a `processed/` subdir, asserting `pending_count` /
//!    `processed_count` reflect on-disk state and binary files are excluded.
//! 3. Tilde expansion in `directory` round-trips through to the response.
//! 4. Wrong HTTP method on `/api/inbox/status` is rejected (405).
//!
//! Mounting only `routes::inbox::router()` (mirroring `users_test.rs`) keeps
//! the tests fast and free of LLM credentials. No global env mutation, no
//! filesystem writes outside the per-test tempdir — safe for parallel
//! execution.

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use axum::Router;
use librefang_api::routes::{self, AppState};
use librefang_testing::{MockKernelBuilder, TestAppState};
use librefang_types::config::InboxConfig;
use std::path::PathBuf;
use std::sync::Arc;
use tower::ServiceExt;

struct Harness {
    app: Router,
    tmp: PathBuf,
    _state: Arc<AppState>,
    _test: TestAppState,
}

fn make_harness(inbox: InboxConfig) -> Harness {
    let test = TestAppState::with_builder(MockKernelBuilder::new().with_config(move |cfg| {
        cfg.inbox = inbox.clone();
    }));

    let tmp = test.tmp_path().to_path_buf();
    let state = test.state.clone();
    let app = Router::new()
        .nest("/api", routes::inbox::router())
        .with_state(state.clone());

    Harness {
        app,
        tmp,
        _state: state,
        _test: test,
    }
}

async fn json_request(h: &Harness, method: Method, path: &str) -> (StatusCode, serde_json::Value) {
    let req = Request::builder()
        .method(method)
        .uri(path)
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

#[tokio::test(flavor = "multi_thread")]
async fn inbox_status_default_config_returns_disabled_with_home_dir() {
    let h = make_harness(InboxConfig::default());

    let (status, body) = json_request(&h, Method::GET, "/api/inbox/status").await;
    assert_eq!(status, StatusCode::OK, "body: {body:?}");

    assert_eq!(body["enabled"], serde_json::Value::Bool(false));
    assert_eq!(body["poll_interval_secs"], serde_json::json!(5));
    assert!(
        body["default_agent"].is_null(),
        "default_agent should be null, got {:?}",
        body["default_agent"]
    );
    assert_eq!(body["pending_count"], serde_json::json!(0));
    assert_eq!(body["processed_count"], serde_json::json!(0));

    // Directory should default to <home>/inbox.
    let expected_dir = h.tmp.join("inbox");
    assert_eq!(
        body["directory"].as_str().unwrap(),
        expected_dir.to_string_lossy()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn inbox_status_counts_pending_and_processed_text_files() {
    let inbox_dir = tempfile::tempdir().expect("tmp inbox dir");
    let inbox_path = inbox_dir.path().to_path_buf();
    std::fs::create_dir_all(inbox_path.join("processed")).unwrap();

    // Pending: two text files + one binary file (ignored).
    std::fs::write(inbox_path.join("a.txt"), "hello").unwrap();
    std::fs::write(inbox_path.join("b.md"), "world").unwrap();
    std::fs::write(inbox_path.join("ignored.png"), [0u8; 4]).unwrap();

    // Processed: three text files.
    std::fs::write(inbox_path.join("processed").join("p1.txt"), "x").unwrap();
    std::fs::write(inbox_path.join("processed").join("p2.json"), "{}").unwrap();
    std::fs::write(inbox_path.join("processed").join("p3.log"), "y").unwrap();

    let cfg = InboxConfig {
        enabled: true,
        directory: Some(inbox_path.to_string_lossy().into_owned()),
        poll_interval_secs: 11,
        default_agent: Some("triage-bot".to_string()),
    };

    let h = make_harness(cfg);
    let (status, body) = json_request(&h, Method::GET, "/api/inbox/status").await;
    assert_eq!(status, StatusCode::OK, "body: {body:?}");

    assert_eq!(body["enabled"], serde_json::Value::Bool(true));
    assert_eq!(body["poll_interval_secs"], serde_json::json!(11));
    assert_eq!(body["default_agent"], serde_json::json!("triage-bot"));
    assert_eq!(body["pending_count"], serde_json::json!(2));
    assert_eq!(body["processed_count"], serde_json::json!(3));
    assert_eq!(
        body["directory"].as_str().unwrap(),
        inbox_path.to_string_lossy()
    );

    // Keep tempdir alive past the assertions.
    drop(inbox_dir);
}

#[tokio::test(flavor = "multi_thread")]
async fn inbox_status_expands_tilde_in_directory() {
    let cfg = InboxConfig {
        enabled: true,
        directory: Some("~/.librefang-inbox-test-#3571".to_string()),
        poll_interval_secs: 5,
        default_agent: None,
    };
    let h = make_harness(cfg);

    let (status, body) = json_request(&h, Method::GET, "/api/inbox/status").await;
    assert_eq!(status, StatusCode::OK, "body: {body:?}");

    let dir = body["directory"].as_str().unwrap();
    // `~` must have been expanded — we don't know the exact home but we know
    // it must NOT start with `~`.
    assert!(!dir.starts_with('~'), "expected tilde expansion, got {dir}");
    assert!(
        dir.ends_with(".librefang-inbox-test-#3571"),
        "expected suffix preserved, got {dir}"
    );
    // Counts default to 0 because the path almost certainly doesn't exist.
    assert_eq!(body["pending_count"], serde_json::json!(0));
    assert_eq!(body["processed_count"], serde_json::json!(0));
}

#[tokio::test(flavor = "multi_thread")]
async fn inbox_status_rejects_non_get_methods() {
    let h = make_harness(InboxConfig::default());

    for method in [Method::POST, Method::PUT, Method::DELETE, Method::PATCH] {
        let (status, _body) = json_request(&h, method.clone(), "/api/inbox/status").await;
        assert_eq!(
            status,
            StatusCode::METHOD_NOT_ALLOWED,
            "method {method} should be rejected"
        );
    }
}
