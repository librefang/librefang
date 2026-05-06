//! Integration tests for the config-domain HTTP routes registered via
//! `routes::config::router()` (see `crates/librefang-api/src/routes/config.rs`).
//!
//! Coverage per #3571 — config slice only:
//!   - GET  /api/config            (happy path + auth gate)
//!   - GET  /api/config/schema     (happy path; public, no auth gate)
//!   - GET  /api/config/export     (happy path with on-disk file + fallback to in-memory)
//!   - POST /api/config/set        (allowlisted round-trip; rejects empty path,
//!     traversal, non-allowlisted key, missing fields)
//!   - POST /api/config/reload     (no-op reload returns 200 with status field)
//!
//! Out of scope (intentionally skipped):
//!   - POST /api/migrate, /api/migrate/scan, GET /api/migrate/detect — touches
//!     real on-disk migration state outside the tempdir.
//!   - POST /api/shutdown / /api/init — would tear down the harness kernel.
//!   - GET  /api/metrics, /api/health, /api/version, /api/status — covered
//!     elsewhere or trivial.
//!
//! All tests use a tempdir-backed kernel (config.home_dir = tempdir) so any
//! write-through to `config.toml` lands in the test sandbox, never the real
//! `~/.librefang/config.toml`.

use axum::body::{to_bytes, Body};
use axum::http::{header, Method, Request, StatusCode};
use librefang_api::server;
use librefang_kernel::LibreFangKernel;
use librefang_types::config::{DefaultModelConfig, KernelConfig};
use std::sync::Arc;
use tower::ServiceExt;

const API_KEY: &str = "test-secret-key";

struct RouterHarness {
    app: axum::Router,
    home: std::path::PathBuf,
    _tmp: tempfile::TempDir,
    state: Arc<librefang_api::routes::AppState>,
}

impl Drop for RouterHarness {
    fn drop(&mut self) {
        self.state.kernel.shutdown();
    }
}

async fn boot_router_with_api_key(api_key: &str) -> RouterHarness {
    let tmp = tempfile::tempdir().expect("tempdir");

    librefang_kernel::registry_sync::sync_registry(
        tmp.path(),
        librefang_kernel::registry_sync::DEFAULT_CACHE_TTL_SECS,
        "",
    );

    let config = KernelConfig {
        home_dir: tmp.path().to_path_buf(),
        data_dir: tmp.path().join("data"),
        api_key: api_key.to_string(),
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

    let home = config.home_dir.clone();
    let kernel = LibreFangKernel::boot_with_config(config).expect("kernel boot");
    let kernel = Arc::new(kernel);
    kernel.set_self_handle();

    let (app, state) = server::build_router(kernel, "127.0.0.1:0".parse().expect("addr")).await;

    RouterHarness {
        app,
        home,
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

fn auth_get(path: &str) -> Request<Body> {
    Request::builder()
        .method(Method::GET)
        .uri(path)
        .header(header::AUTHORIZATION, format!("Bearer {API_KEY}"))
        .body(Body::empty())
        .unwrap()
}

fn anon_get(path: &str) -> Request<Body> {
    Request::builder()
        .method(Method::GET)
        .uri(path)
        .body(Body::empty())
        .unwrap()
}

fn auth_post_json(path: &str, body: serde_json::Value) -> Request<Body> {
    Request::builder()
        .method(Method::POST)
        .uri(path)
        .header(header::AUTHORIZATION, format!("Bearer {API_KEY}"))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_string()))
        .unwrap()
}

// ---------------------------------------------------------------------------
// GET /api/config
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn get_config_returns_redacted_view() {
    let h = boot_router_with_api_key(API_KEY).await;
    let (status, body) = send(h.app.clone(), auth_get("/api/config")).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "body: {}",
        String::from_utf8_lossy(&body)
    );

    let json: serde_json::Value = serde_json::from_slice(&body).expect("response is JSON");
    // Spot-check some fields the redacted view always includes.
    assert!(json.is_object(), "expected object, got {json}");
    for key in ["channels", "mcp_servers", "fallback_providers"] {
        assert!(
            json.get(key).is_some(),
            "missing redacted field '{key}' in /api/config response: {json}"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn get_config_is_dashboard_read_when_no_api_key() {
    // With api_key empty, dashboard reads must work without a token.
    let h = boot_router_with_api_key("").await;
    let (status, _) = send(h.app.clone(), anon_get("/api/config")).await;
    assert_ne!(
        status,
        StatusCode::UNAUTHORIZED,
        "/api/config must be reachable without auth in no-key dev mode"
    );
}

// ---------------------------------------------------------------------------
// GET /api/config/schema
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn get_config_schema_is_public_and_returns_json_schema() {
    // Schema is in PUBLIC_ROUTES_ALWAYS, so anonymous GET must succeed even
    // when an api_key is configured.
    let h = boot_router_with_api_key(API_KEY).await;
    let (status, body) = send(h.app.clone(), anon_get("/api/config/schema")).await;
    assert_eq!(status, StatusCode::OK);

    let json: serde_json::Value = serde_json::from_slice(&body).expect("response is JSON");
    // Schemars-generated draft-07 output, plus our two extension keys.
    assert!(
        json.get("x-sections").is_some(),
        "schema missing x-sections overlay"
    );
    assert!(
        json.get("x-ui-options").is_some(),
        "schema missing x-ui-options overlay"
    );
}

// ---------------------------------------------------------------------------
// GET /api/config/export
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn get_config_export_falls_back_to_in_memory_when_no_file() {
    // Tempdir has no config.toml — handler must serialize the in-memory config.
    let h = boot_router_with_api_key(API_KEY).await;
    assert!(!h.home.join("config.toml").exists());

    let (status, body) = send(h.app.clone(), auth_get("/api/config/export")).await;
    assert_eq!(status, StatusCode::OK);
    let toml_text = String::from_utf8(body).expect("toml is utf-8");
    // Must parse as TOML and include at least a top-level table marker.
    let _: toml::Value = toml::from_str(&toml_text).expect("export body is valid TOML");
    assert!(!toml_text.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn get_config_export_reads_disk_file_when_present() {
    let h = boot_router_with_api_key(API_KEY).await;
    let on_disk = "# sentinel-marker-3571\nlog_level = \"debug\"\n";
    std::fs::write(h.home.join("config.toml"), on_disk).expect("write config.toml");

    let (status, body) = send(h.app.clone(), auth_get("/api/config/export")).await;
    assert_eq!(status, StatusCode::OK);
    let text = String::from_utf8(body).unwrap();
    assert!(
        text.contains("sentinel-marker-3571"),
        "export should pass through the on-disk file verbatim, got: {text}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn config_export_requires_auth_when_key_set() {
    let h = boot_router_with_api_key(API_KEY).await;
    let (status, _) = send(h.app.clone(), anon_get("/api/config/export")).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

// ---------------------------------------------------------------------------
// POST /api/config/set
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn config_set_writes_allowlisted_path_to_tempdir_toml() {
    let h = boot_router_with_api_key(API_KEY).await;
    // `log_level` is a real top-level KernelConfig field on the allowlist;
    // it round-trips through the schema validator AND survives the post-write
    // kernel reload (which re-serializes the in-memory config), unlike
    // dashboard-only paths such as `ui.theme` that the kernel doesn't model.
    let (status, body) = send(
        h.app.clone(),
        auth_post_json(
            "/api/config/set",
            serde_json::json!({"path": "log_level", "value": "debug"}),
        ),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "expected 200 for allowlisted log_level write, got {status}: {}",
        String::from_utf8_lossy(&body)
    );

    // Verify the write landed in the tempdir's config.toml — NOT the user's
    // real ~/.librefang/config.toml. (kernel.home_dir is the tempdir.)
    let written = std::fs::read_to_string(h.home.join("config.toml")).expect("toml exists");
    let parsed: toml::Value = toml::from_str(&written).expect("valid toml");
    let log_level = parsed.get("log_level").and_then(|v| v.as_str());
    assert_eq!(log_level, Some("debug"), "wrote: {written}");

    // And the in-memory kernel config reflects it (post-reload).
    assert_eq!(h.state.kernel.config_ref().log_level, "debug");
}

#[tokio::test(flavor = "multi_thread")]
async fn config_set_rejects_non_allowlisted_path() {
    let h = boot_router_with_api_key(API_KEY).await;
    // `api_key` is excluded from the allowlist for security.
    let (status, body) = send(
        h.app.clone(),
        auth_post_json(
            "/api/config/set",
            serde_json::json!({"path": "api_key", "value": "stolen"}),
        ),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "api_key write must be 403, got {status}: {}",
        String::from_utf8_lossy(&body)
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn config_set_rejects_path_traversal() {
    let h = boot_router_with_api_key(API_KEY).await;
    let (status, _) = send(
        h.app.clone(),
        auth_post_json(
            "/api/config/set",
            serde_json::json!({"path": "../etc/passwd", "value": "x"}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test(flavor = "multi_thread")]
async fn config_set_rejects_empty_path() {
    let h = boot_router_with_api_key(API_KEY).await;
    let (status, _) = send(
        h.app.clone(),
        auth_post_json(
            "/api/config/set",
            serde_json::json!({"path": "", "value": "x"}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test(flavor = "multi_thread")]
async fn config_set_rejects_missing_path_field() {
    let h = boot_router_with_api_key(API_KEY).await;
    let (status, _) = send(
        h.app.clone(),
        auth_post_json("/api/config/set", serde_json::json!({"value": "x"})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test(flavor = "multi_thread")]
async fn config_set_rejects_missing_value_field() {
    let h = boot_router_with_api_key(API_KEY).await;
    let (status, _) = send(
        h.app.clone(),
        auth_post_json("/api/config/set", serde_json::json!({"path": "ui.theme"})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

// ---------------------------------------------------------------------------
// POST /api/config/set — collection-typed sections (#4678)
//
// Round-trips for the BTreeMap<String, String|u64> sections that the
// dashboard's StringMapEditor / NumberMapEditor save as a whole-blob
// payload at the section's bare path. Vec<Struct> sections (sidecar_channels,
// fallback_providers, taint_rules) and tightened-out section prefixes
// (external_auth, oauth, audit, telemetry, proxy) must be rejected.
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn config_set_writes_provider_urls_collection_to_toml() {
    let h = boot_router_with_api_key(API_KEY).await;
    let payload = serde_json::json!({
        "openai": "https://api.openai.com/v1",
        "ollama": "http://127.0.0.1:11434/v1",
    });
    let (status, body) = send(
        h.app.clone(),
        auth_post_json(
            "/api/config/set",
            serde_json::json!({"path": "provider_urls", "value": payload.clone()}),
        ),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "expected 200 for whole-collection provider_urls write, got {status}: {}",
        String::from_utf8_lossy(&body)
    );

    let written = std::fs::read_to_string(h.home.join("config.toml")).expect("toml exists");
    let parsed: toml::Value = toml::from_str(&written).expect("valid toml");
    let urls = parsed
        .get("provider_urls")
        .and_then(|v| v.as_table())
        .expect("provider_urls table present");
    assert_eq!(
        urls.get("openai").and_then(|v| v.as_str()),
        Some("https://api.openai.com/v1"),
        "wrote: {written}"
    );
    assert_eq!(
        urls.get("ollama").and_then(|v| v.as_str()),
        Some("http://127.0.0.1:11434/v1"),
        "wrote: {written}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn config_set_writes_tool_timeouts_number_map_to_toml() {
    let h = boot_router_with_api_key(API_KEY).await;
    let (status, body) = send(
        h.app.clone(),
        auth_post_json(
            "/api/config/set",
            serde_json::json!({"path": "tool_timeouts", "value": {"shell": 60, "fetch": 30}}),
        ),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "tool_timeouts whole-collection write should round-trip; got {status}: {}",
        String::from_utf8_lossy(&body)
    );

    let written = std::fs::read_to_string(h.home.join("config.toml")).expect("toml exists");
    let parsed: toml::Value = toml::from_str(&written).expect("valid toml");
    let timeouts = parsed
        .get("tool_timeouts")
        .and_then(|v| v.as_table())
        .expect("tool_timeouts table");
    assert_eq!(timeouts.get("shell").and_then(|v| v.as_integer()), Some(60));
    assert_eq!(timeouts.get("fetch").and_then(|v| v.as_integer()), Some(30));
}

#[tokio::test(flavor = "multi_thread")]
async fn config_set_rejects_sidecar_channels_whole_blob_write() {
    // Vec<Struct> sections cannot be whole-blob-written — their items
    // contain nested env maps that the path-string SCRUB cannot police
    // when they arrive as a JSON payload at the section's bare path.
    let h = boot_router_with_api_key(API_KEY).await;
    let evil_payload = serde_json::json!([
        {
            "name": "evil",
            "command": "/bin/cat",
            "channel_type": "telegram",
            "env": {"AWS_SECRET_ACCESS_KEY": "stolen"}
        }
    ]);
    let (status, body) = send(
        h.app.clone(),
        auth_post_json(
            "/api/config/set",
            serde_json::json!({"path": "sidecar_channels", "value": evil_payload}),
        ),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "sidecar_channels whole-blob write must 403; got {status}: {}",
        String::from_utf8_lossy(&body)
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn config_set_rejects_fallback_providers_whole_blob_write() {
    let h = boot_router_with_api_key(API_KEY).await;
    let (status, _) = send(
        h.app.clone(),
        auth_post_json(
            "/api/config/set",
            serde_json::json!({
                "path": "fallback_providers",
                "value": [{"provider": "openai", "model": "gpt-4o", "api_key_env": "STOLEN"}]
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test(flavor = "multi_thread")]
async fn config_set_rejects_external_auth_issuer_url() {
    // external_auth.* is intentionally NOT in SECTION_PREFIXES — flipping
    // issuer_url post-auth would let an Owner-role attacker redirect login
    // to an attacker IDP (regression vector for #3703).
    let h = boot_router_with_api_key(API_KEY).await;
    let (status, _) = send(
        h.app.clone(),
        auth_post_json(
            "/api/config/set",
            serde_json::json!({
                "path": "external_auth.issuer_url",
                "value": "https://attacker.example/"
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test(flavor = "multi_thread")]
async fn config_set_rejects_audit_anchor_path() {
    let h = boot_router_with_api_key(API_KEY).await;
    let (status, _) = send(
        h.app.clone(),
        auth_post_json(
            "/api/config/set",
            serde_json::json!({"path": "audit.anchor_path", "value": "/tmp/evil-anchor"}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test(flavor = "multi_thread")]
async fn config_set_rejects_telemetry_otlp_endpoint() {
    let h = boot_router_with_api_key(API_KEY).await;
    let (status, _) = send(
        h.app.clone(),
        auth_post_json(
            "/api/config/set",
            serde_json::json!({
                "path": "telemetry.otlp_endpoint",
                "value": "https://attacker.example:4317"
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test(flavor = "multi_thread")]
async fn config_set_rejects_proxy_http_proxy() {
    let h = boot_router_with_api_key(API_KEY).await;
    let (status, _) = send(
        h.app.clone(),
        auth_post_json(
            "/api/config/set",
            serde_json::json!({"path": "proxy.http_proxy", "value": "http://attacker:8080"}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test(flavor = "multi_thread")]
async fn config_set_rejects_env_suffix_redirect_inside_writable_section() {
    // SCRUB_SUFFIXES extension catches `<anything>_env` so an attacker
    // can't repoint `default_model.api_key_env` (or any *.token_env /
    // *.client_secret_env / *.password_env) at an arbitrary daemon env var.
    let h = boot_router_with_api_key(API_KEY).await;
    let (status, _) = send(
        h.app.clone(),
        auth_post_json(
            "/api/config/set",
            serde_json::json!({"path": "default_model.api_key_env", "value": "HOME"}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test(flavor = "multi_thread")]
async fn config_set_requires_auth_when_key_set() {
    let h = boot_router_with_api_key(API_KEY).await;
    let req = Request::builder()
        .method(Method::POST)
        .uri("/api/config/set")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            serde_json::json!({"path": "ui.theme", "value": "dark"}).to_string(),
        ))
        .unwrap();
    let (status, _) = send(h.app.clone(), req).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

// ---------------------------------------------------------------------------
// POST /api/config/reload
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn config_reload_returns_no_changes_when_disk_matches_memory() {
    let h = boot_router_with_api_key(API_KEY).await;
    let req = Request::builder()
        .method(Method::POST)
        .uri("/api/config/reload")
        .header(header::AUTHORIZATION, format!("Bearer {API_KEY}"))
        .body(Body::empty())
        .unwrap();
    let (status, body) = send(h.app.clone(), req).await;
    // Reload may return 200 (no changes / applied) or 400 (no on-disk file
    // depending on kernel impl). Either way the body must be JSON with a
    // `status` field — the route must be wired and not 404 / 500-stack-trace.
    assert!(
        status == StatusCode::OK || status == StatusCode::BAD_REQUEST,
        "unexpected status {status}: {}",
        String::from_utf8_lossy(&body)
    );
    let json: serde_json::Value = serde_json::from_slice(&body).expect("reload body is JSON");
    assert!(
        json.get("status").is_some(),
        "missing 'status' field: {json}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn config_reload_requires_auth_when_key_set() {
    let h = boot_router_with_api_key(API_KEY).await;
    let req = Request::builder()
        .method(Method::POST)
        .uri("/api/config/reload")
        .body(Body::empty())
        .unwrap();
    let (status, _) = send(h.app.clone(), req).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

/// Regression for #4664: a syntactically-broken `config.toml` (the bug report
/// hit a duplicate `[web.searxng]` key) used to silently reset the live config
/// to defaults on the next hot-reload tick because `crate::config::load_config`
/// is tolerant. From the operator's POV the dashboard "stopped loading" because
/// `default_model`, `provider_api_keys`, channels, etc. all reverted to
/// defaults. The fix makes `reload_config` strict-parse the file first and
/// surface the error so the live config stays intact.
#[tokio::test(flavor = "multi_thread")]
async fn config_reload_with_invalid_toml_returns_error_and_preserves_live_config() {
    let h = boot_router_with_api_key(API_KEY).await;

    // Capture the live `default_model` BEFORE the bad reload so we can prove
    // it survived. The harness boots with `model = "test-model"`.
    let (status, body) = send(h.app.clone(), auth_get("/api/config")).await;
    assert_eq!(status, StatusCode::OK);
    let before: serde_json::Value = serde_json::from_slice(&body).expect("response is JSON");
    let before_model = before
        .get("default_model")
        .and_then(|m| m.get("model"))
        .and_then(|m| m.as_str())
        .unwrap_or("")
        .to_string();
    assert_eq!(
        before_model, "test-model",
        "harness must seed default_model.model = test-model"
    );

    // Write a config.toml with a TOML duplicate-key error. This mirrors the
    // exact failure shape from the user's report: two `[web.searxng]` sections.
    let bad_toml =
        "[web.searxng]\nurl = \"http://first\"\n\n[web.searxng]\nurl = \"http://second\"\n";
    std::fs::write(h.home.join("config.toml"), bad_toml).expect("write bad config.toml");

    // Reload must report a parse error (400) — NOT silently apply defaults (200).
    let req = Request::builder()
        .method(Method::POST)
        .uri("/api/config/reload")
        .header(header::AUTHORIZATION, format!("Bearer {API_KEY}"))
        .body(Body::empty())
        .unwrap();
    let (status, body) = send(h.app.clone(), req).await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "bad TOML must produce a 400 with an explicit error, not a 200 + silent defaults; body={}",
        String::from_utf8_lossy(&body)
    );
    let json: serde_json::Value = serde_json::from_slice(&body).expect("response is JSON");
    let err_str = json
        .get("error")
        .and_then(|e| e.as_str())
        .unwrap_or("")
        .to_string();
    assert!(
        err_str.contains("invalid TOML") && err_str.contains("live config unchanged"),
        "error must be operator-actionable; got: {err_str}"
    );

    // Live config must be unchanged — the failed reload did not blow away
    // `default_model.model` (which is the symptom that broke the dashboard).
    let (status, body) = send(h.app.clone(), auth_get("/api/config")).await;
    assert_eq!(status, StatusCode::OK);
    let after: serde_json::Value = serde_json::from_slice(&body).expect("response is JSON");
    let after_model = after
        .get("default_model")
        .and_then(|m| m.get("model"))
        .and_then(|m| m.as_str())
        .unwrap_or("");
    assert_eq!(
        after_model, before_model,
        "live default_model.model must be preserved after a failed reload"
    );
}

/// Internal helper: drop a `config.toml` into the harness's home dir,
/// POST `/api/config/reload`, and assert that it returns 400 *and* that
/// `GET /api/config` still reports the seeded `default_model.model`.
///
/// Used by the next two regressions to cover the two non-syntax failure
/// modes that `try_load_config` (introduced in #4664) refuses: a
/// deserialize-shape mismatch and a broken `include = [...]` chain. The
/// duplicate-key TOML-syntax case has its own dedicated test above
/// (preserved with its own assertion text so a regression on the syntax
/// path stays distinguishable in test output).
async fn assert_reload_rejects_and_preserves_default_model(
    h: &RouterHarness,
    bad_toml_filename: &str,
    bad_toml_contents: &str,
    extra_files: &[(&str, &str)],
    failure_label: &str,
) {
    // Capture pre-reload `default_model.model`.
    let (status, body) = send(h.app.clone(), auth_get("/api/config")).await;
    assert_eq!(status, StatusCode::OK);
    let before: serde_json::Value = serde_json::from_slice(&body).expect("response is JSON");
    let before_model = before
        .get("default_model")
        .and_then(|m| m.get("model"))
        .and_then(|m| m.as_str())
        .unwrap_or("")
        .to_string();
    assert_eq!(before_model, "test-model");

    for (name, contents) in extra_files {
        std::fs::write(h.home.join(name), contents)
            .unwrap_or_else(|e| panic!("write helper file {name}: {e}"));
    }
    std::fs::write(h.home.join(bad_toml_filename), bad_toml_contents)
        .expect("write bad config.toml");

    let req = Request::builder()
        .method(Method::POST)
        .uri("/api/config/reload")
        .header(header::AUTHORIZATION, format!("Bearer {API_KEY}"))
        .body(Body::empty())
        .unwrap();
    let (status, body) = send(h.app.clone(), req).await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "{failure_label} must produce a 400, not silent defaults; body={}",
        String::from_utf8_lossy(&body)
    );
    let json: serde_json::Value = serde_json::from_slice(&body).expect("response is JSON");
    let err_str = json
        .get("error")
        .and_then(|e| e.as_str())
        .unwrap_or("")
        .to_string();
    // Every reload-time rejection MUST go through the strict loader's
    // `try_load_config` and be wrapped with the "live config unchanged"
    // pledge, so future failure modes can be asserted with the same
    // substring without needing to know which inner branch tripped.
    assert!(
        err_str.contains("live config unchanged"),
        "{failure_label} error must carry the reload-boundary pledge; got: {err_str}"
    );

    let (status, body) = send(h.app.clone(), auth_get("/api/config")).await;
    assert_eq!(status, StatusCode::OK);
    let after: serde_json::Value = serde_json::from_slice(&body).expect("response is JSON");
    let after_model = after
        .get("default_model")
        .and_then(|m| m.get("model"))
        .and_then(|m| m.as_str())
        .unwrap_or("");
    assert_eq!(
        after_model, before_model,
        "{failure_label}: live default_model.model must be preserved after a failed reload"
    );
}

/// End-to-end regression for the second silent-defaults path that
/// `try_load_config` (#4664) closes: TOML parses cleanly but a field
/// has the wrong shape (`default_model = "string"` where a table is
/// expected). Pre-fix, `load_config` would warn and return defaults
/// and the reload would silently overwrite the live config; post-fix,
/// `POST /api/config/reload` must return 400.
#[tokio::test(flavor = "multi_thread")]
async fn config_reload_with_deserialize_shape_mismatch_returns_error_and_preserves_live_config() {
    let h = boot_router_with_api_key(API_KEY).await;
    assert_reload_rejects_and_preserves_default_model(
        &h,
        "config.toml",
        // TOML parses fine; deserialize fails because `default_model` is a struct.
        "default_model = \"not-a-table\"\n",
        &[],
        "deserialize-shape mismatch",
    )
    .await;
}

/// End-to-end regression for the third silent-defaults path: root
/// config is well-formed but `include = ["bad.toml"]` points at a
/// file that fails TOML parsing. Pre-fix, `resolve_config_includes`'s
/// error was swallowed by `load_config` and the reload proceeded with
/// the root only; post-fix, the reload must refuse.
#[tokio::test(flavor = "multi_thread")]
async fn config_reload_with_broken_include_returns_error_and_preserves_live_config() {
    let h = boot_router_with_api_key(API_KEY).await;
    assert_reload_rejects_and_preserves_default_model(
        &h,
        "config.toml",
        "include = [\"bad.toml\"]\nlog_level = \"debug\"\n",
        &[(
            "bad.toml",
            // Same duplicate-key shape as #4664, just inside the include.
            "[memory]\ndecay_rate = 0.1\n[memory]\ndecay_rate = 0.2\n",
        )],
        "broken include chain",
    )
    .await;
}

/// End-to-end regression locking in the `live config unchanged` reload-
/// boundary contract for the *post-loader* validation path
/// (`config_reload::validate_config_for_reload`). The strict loader
/// accepts the file (parses cleanly, deserialises into a valid
/// `KernelConfig`), but the validator rejects the result — e.g.
/// `network_enabled = true` with an empty `network.shared_secret`.
///
/// Without the contract being uniform, a future regression on this
/// branch would surface as a confusing assertion-helper diff rather
/// than the clear "wrapper missing" message. Asserting the substring
/// here means the helper covers every reload-rejection branch
/// regardless of which one trips.
#[tokio::test(flavor = "multi_thread")]
async fn config_reload_with_validation_failure_returns_error_and_preserves_live_config() {
    let h = boot_router_with_api_key(API_KEY).await;
    assert_reload_rejects_and_preserves_default_model(
        &h,
        "config.toml",
        // Parses + deserialises fine; validator refuses because
        // network_enabled requires a non-empty shared_secret.
        "network_enabled = true\n[network]\nshared_secret = \"\"\n",
        &[],
        "post-loader validation failure",
    )
    .await;
}

// ---------------------------------------------------------------------------
// GET /api/health/detail (#3776)
//
// Validates that the new operational metric sections (`budget`, `llm`) are
// wired into the response and serialize with the documented shape so that
// monitoring systems (Prometheus blackbox exporter, alerting rules) can rely
// on the field names.
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn health_detail_includes_budget_and_llm_sections() {
    let h = boot_router_with_api_key(API_KEY).await;
    let (status, body) = send(h.app.clone(), auth_get("/api/health/detail")).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "body: {}",
        String::from_utf8_lossy(&body)
    );

    let json: serde_json::Value = serde_json::from_slice(&body).expect("response is JSON");

    // Pre-existing fields must remain (regression guard).
    for key in [
        "status",
        "version",
        "uptime_seconds",
        "panic_count",
        "restart_count",
        "agent_count",
        "database",
        "memory",
        "config_warnings",
        "event_bus",
    ] {
        assert!(
            json.get(key).is_some(),
            "missing pre-existing field '{key}' in /api/health/detail: {json}"
        );
    }

    // New `budget` block — exposes already-collected MeteringEngine spend.
    let budget = json
        .get("budget")
        .expect("missing 'budget' section in /api/health/detail");
    for key in [
        "hourly_spend_usd",
        "hourly_limit_usd",
        "hourly_spend_percent",
        "daily_spend_usd",
        "daily_limit_usd",
        "daily_spend_percent",
        "monthly_spend_usd",
        "monthly_limit_usd",
        "monthly_spend_percent",
        "alert_threshold",
    ] {
        assert!(
            budget.get(key).is_some(),
            "missing budget.{key} in /api/health/detail: {budget}"
        );
    }
    // With no budget cap configured in the test kernel, the *_percent fields
    // must serialize as JSON null (operators distinguish "no cap" from "0%").
    for key in [
        "daily_spend_percent",
        "hourly_spend_percent",
        "monthly_spend_percent",
    ] {
        assert!(
            budget.get(key).expect("present").is_null(),
            "{key} must be null when no cap is configured: {budget}"
        );
    }

    // New `llm` block — sourced from query_model_performance() snapshot.
    let llm = json
        .get("llm")
        .expect("missing 'llm' section in /api/health/detail");
    for key in [
        "total_calls",
        "avg_latency_ms",
        "max_latency_ms",
        "model_count",
    ] {
        assert!(
            llm.get(key).is_some(),
            "missing llm.{key} in /api/health/detail: {llm}"
        );
    }
    // No LLM calls have been recorded in this fresh kernel.
    assert_eq!(llm["total_calls"].as_u64(), Some(0));
    assert_eq!(llm["max_latency_ms"].as_u64(), Some(0));
}

#[tokio::test(flavor = "multi_thread")]
async fn health_detail_daily_spend_percent_reflects_configured_cap() {
    use librefang_types::config::BudgetConfig;

    let h = boot_router_with_api_key(API_KEY).await;

    // Set a non-zero daily cap so the *_percent fields become defined (0.0
    // for an empty kernel rather than null).
    h.state.kernel.update_budget_config(|b: &mut BudgetConfig| {
        b.max_daily_usd = 25.0;
        b.max_hourly_usd = 5.0;
    });

    let (status, body) = send(h.app.clone(), auth_get("/api/health/detail")).await;
    assert_eq!(status, StatusCode::OK);
    let json: serde_json::Value = serde_json::from_slice(&body).expect("response is JSON");
    let budget = &json["budget"];

    assert_eq!(budget["daily_limit_usd"].as_f64(), Some(25.0));
    assert_eq!(budget["hourly_limit_usd"].as_f64(), Some(5.0));
    assert_eq!(
        budget["daily_spend_percent"].as_f64(),
        Some(0.0),
        "daily_spend_percent must be 0.0 (not null) once a cap is set: {budget}"
    );
    assert_eq!(
        budget["hourly_spend_percent"].as_f64(),
        Some(0.0),
        "hourly_spend_percent must be 0.0 (not null) once a cap is set: {budget}"
    );
    // No monthly cap was set — must remain null.
    assert!(
        budget["monthly_spend_percent"].is_null(),
        "monthly_spend_percent must stay null when no monthly cap is set: {budget}"
    );
}
