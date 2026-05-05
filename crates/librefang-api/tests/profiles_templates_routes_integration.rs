//! Integration tests for `/api/profiles` and `/api/templates` sub-routes
//! inside `crates/librefang-api/src/routes/system.rs` (refs #3571 — "~80%
//! of registered HTTP routes have no integration test").
//!
//! These exercise the real `system::router()` via `tower::oneshot`, with a
//! `TestAppState` + `MockKernelBuilder` boot. The auth middleware is not
//! mounted in this slice — same approach as `users_test.rs` — because the
//! profile/template handlers are pure (profiles) or filesystem-bound
//! (templates) and the goal is to catch the "compiles but routes are dead /
//! return wrong shape" class of bug called out in the issue.
//!
//! ### Templates and `LIBREFANG_HOME`
//!
//! `list_agent_templates` / `get_agent_template` / `get_agent_template_toml`
//! all read from `librefang_home()/workspaces/agents/`, where `librefang_home`
//! honours the `LIBREFANG_HOME` env var. We pin a single tempdir for the
//! whole test binary via `OnceLock` and serialise the template tests behind
//! a `Mutex` so unique-name fixtures can coexist without env-var races.

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use axum::Router;
use librefang_api::routes::{self, AppState};
use librefang_testing::{MockKernelBuilder, TestAppState};
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};
use tempfile::TempDir;
use tokio::sync::Mutex;
use tower::ServiceExt;

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

struct Harness {
    app: Router,
    _state: Arc<AppState>,
    _test: TestAppState,
}

async fn boot() -> Harness {
    let test = TestAppState::with_builder(MockKernelBuilder::new().with_config(|cfg| {
        // Minimal default model so kernel boot is happy. Same shape as
        // `users_test.rs::boot`.
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
        .nest("/api", routes::system::router())
        .with_state(state.clone());
    Harness {
        app,
        _state: state,
        _test: test,
    }
}

async fn get(h: &Harness, path: &str) -> (StatusCode, axum::http::HeaderMap, Vec<u8>) {
    let req = Request::builder()
        .method(Method::GET)
        .uri(path)
        .body(Body::empty())
        .unwrap();
    let resp = h.app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let headers = resp.headers().clone();
    let bytes = axum::body::to_bytes(resp.into_body(), 1 << 20)
        .await
        .unwrap()
        .to_vec();
    (status, headers, bytes)
}

async fn get_json(h: &Harness, path: &str) -> (StatusCode, serde_json::Value) {
    let (status, _hdr, bytes) = get(h, path).await;
    let value: serde_json::Value = if bytes.is_empty() {
        serde_json::Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null)
    };
    (status, value)
}

// ---------------------------------------------------------------------------
// /api/profiles — pure handler, no filesystem.
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn profiles_list_returns_six_known_profiles() {
    let h = boot().await;
    let (status, body) = get_json(&h, "/api/profiles").await;
    assert_eq!(status, StatusCode::OK);
    let arr = body.as_array().expect("array");
    let names: Vec<&str> = arr.iter().map(|v| v["name"].as_str().unwrap()).collect();
    // Pin the registered set so a refactor that drops a profile is loud.
    assert_eq!(
        names,
        vec![
            "minimal",
            "coding",
            "research",
            "messaging",
            "automation",
            "full",
        ],
        "profile registration drift: {body}"
    );
    // Each entry must carry a non-empty tools list — the dashboard renders
    // these directly. An empty list would silently break the UI.
    for entry in arr {
        let tools = entry["tools"].as_array().expect("tools array");
        assert!(
            !tools.is_empty(),
            "profile {:?} has no tools",
            entry["name"]
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn profiles_get_known_profile_returns_tools() {
    let h = boot().await;
    let (status, body) = get_json(&h, "/api/profiles/coding").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["name"], "coding");
    assert!(
        body["tools"]
            .as_array()
            .map(|a| !a.is_empty())
            .unwrap_or(false),
        "coding profile must expose tools: {body}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn profiles_get_unknown_profile_returns_404() {
    let h = boot().await;
    let (status, body) = get_json(&h, "/api/profiles/no-such-profile").await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert!(
        body["error"].is_string() || body["error"]["message"].is_string(),
        "404 must carry a structured error payload: {body}"
    );
}

// ---------------------------------------------------------------------------
// /api/templates — filesystem-bound, scoped to a per-binary LIBREFANG_HOME.
// ---------------------------------------------------------------------------

/// One tempdir for the whole test binary. We never unset `LIBREFANG_HOME`
/// once it's set — flipping it mid-run would race with any other test that
/// happens to call `librefang_home()`.
fn templates_root() -> PathBuf {
    static HOME: OnceLock<TempDir> = OnceLock::new();
    let dir = HOME.get_or_init(|| {
        let tmp = tempfile::tempdir().expect("tempdir");
        // Safety: env mutation. Setting it once, before any concurrent
        // test reads, is the standard pattern in this workspace's
        // env-var-driven tests; see `crates/librefang-llm-drivers` etc.
        // The unsafe block is only required on Rust 2024+.
        std::env::set_var("LIBREFANG_HOME", tmp.path());
        tmp
    });
    dir.path().join("workspaces").join("agents")
}

/// Serialise template-mutating tests so unique-name fixtures don't read each
/// other's listings as "extra entries". `list_agent_templates` walks the
/// whole `agents/` dir, so a parallel test seeding `bravo` while another is
/// asserting "exactly one entry" would flake. Each test takes the lock,
/// writes its fixtures into a unique subdir, runs, then drops the lock.
fn templates_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

fn write_template(name: &str, body: &str) {
    let root = templates_root();
    let dir = root.join(name);
    std::fs::create_dir_all(&dir).expect("create template dir");
    std::fs::write(dir.join("agent.toml"), body).expect("write agent.toml");
}

fn remove_template(name: &str) {
    let dir = templates_root().join(name);
    let _ = std::fs::remove_dir_all(dir);
}

fn minimal_manifest_toml(name: &str, description: &str) -> String {
    format!(
        r#"name = "{name}"
version = "0.1.0"
description = "{description}"
module = "builtin:chat"
tags = ["test"]

[model]
provider = "default"
model = "default"

[capabilities]
tools = ["web_fetch"]
"#
    )
}

#[tokio::test(flavor = "multi_thread")]
async fn templates_list_includes_seeded_template() {
    let _g = templates_lock().lock().await;
    // Force the home init before the harness boots so the kernel's own
    // setup doesn't hit `~/.librefang`.
    let _ = templates_root();

    let unique = "tmpl_list_alpha";
    write_template(
        unique,
        &minimal_manifest_toml("alpha", "Alpha test template"),
    );

    let h = boot().await;
    let (status, body) = get_json(&h, "/api/templates").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let templates = body["templates"].as_array().expect("templates array");
    let total = body["total"].as_u64().expect("total u64");
    assert_eq!(
        total as usize,
        templates.len(),
        "total must match array len: {body}"
    );
    let row = templates
        .iter()
        .find(|r| r["name"] == unique)
        .unwrap_or_else(|| panic!("seeded template missing from list: {body}"));
    assert_eq!(row["description"], "Alpha test template", "{body}");

    remove_template(unique);
}

#[tokio::test(flavor = "multi_thread")]
async fn templates_get_known_template_returns_manifest() {
    let _g = templates_lock().lock().await;
    let _ = templates_root();

    let unique = "tmpl_get_bravo";
    let toml_body = minimal_manifest_toml("bravo", "Bravo description");
    write_template(unique, &toml_body);

    let h = boot().await;
    let (status, body) = get_json(&h, &format!("/api/templates/{unique}")).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["name"], unique);
    assert_eq!(body["manifest"]["name"], "bravo");
    assert_eq!(body["manifest"]["description"], "Bravo description");
    assert_eq!(body["manifest"]["module"], "builtin:chat");
    assert!(
        body["manifest_toml"]
            .as_str()
            .map(|s| s.contains("name = \"bravo\""))
            .unwrap_or(false),
        "manifest_toml must round-trip the raw file: {body}"
    );

    remove_template(unique);
}

#[tokio::test(flavor = "multi_thread")]
async fn templates_get_unknown_returns_404() {
    let _g = templates_lock().lock().await;
    let _ = templates_root();
    let h = boot().await;
    let (status, body) = get_json(&h, "/api/templates/does_not_exist_xyz").await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert!(
        body["error"].is_string() || body["error"]["message"].is_string(),
        "{body}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn templates_get_rejects_path_traversal_as_404() {
    // The handler runs `validate_template_name` and turns a malformed name
    // into a 404 (NOT 400) so we don't leak the existence of the validator
    // to scanners. Pin that contract.
    let _g = templates_lock().lock().await;
    let _ = templates_root();
    let h = boot().await;
    // axum normalises `..` in paths, so target a name that survives URL
    // routing but still trips the validator: a dot-bearing string.
    let (status, body) = get_json(&h, "/api/templates/foo.bar").await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
}

#[tokio::test(flavor = "multi_thread")]
async fn templates_toml_returns_plaintext_for_known_template() {
    let _g = templates_lock().lock().await;
    let _ = templates_root();

    let unique = "tmpl_toml_charlie";
    let toml_body = minimal_manifest_toml("charlie", "Charlie raw");
    write_template(unique, &toml_body);

    let h = boot().await;
    let (status, headers, bytes) = get(&h, &format!("/api/templates/{unique}/toml")).await;
    assert_eq!(status, StatusCode::OK);
    let ct = headers
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert!(
        ct.starts_with("text/plain"),
        "expected text/plain content-type, got: {ct:?}"
    );
    let body_str = String::from_utf8(bytes).expect("utf8");
    assert!(
        body_str.contains("name = \"charlie\""),
        "raw TOML must round-trip verbatim: {body_str:?}"
    );
    assert!(
        body_str.contains("Charlie raw"),
        "raw TOML must include description: {body_str:?}"
    );

    remove_template(unique);
}

#[tokio::test(flavor = "multi_thread")]
async fn templates_toml_unknown_returns_plaintext_404() {
    let _g = templates_lock().lock().await;
    let _ = templates_root();
    let h = boot().await;
    let (status, headers, bytes) = get(&h, "/api/templates/no_such_tmpl/toml").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let ct = headers
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert!(
        ct.starts_with("text/plain"),
        "404 path must also serve text/plain to match success shape: {ct:?}"
    );
    assert!(!bytes.is_empty(), "404 plaintext body must be non-empty");
}
