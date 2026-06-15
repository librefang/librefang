//! Integration tests for `POST /api/registry/content/{content_type}`.
//!
//! The endpoint previously returned an absolute filesystem path in its
//! response body, which leaks the operator's OS-username structure
//! (`/Users/<user>` on macOS, `/home/<user>` on Linux). This file pins
//! the fix: the response's `path` field must be **relative to the
//! kernel `home_dir`**, never the absolute path.
//!
//! Run: `cargo test -p librefang-api --test registry_content_path_test`

use axum::body::Body;
use axum::http::{header, Method, Request, StatusCode};
use axum::Router;
use librefang_api::routes::AppState;
use librefang_api::server;
use librefang_kernel::LibreFangKernel;
use librefang_types::config::{DefaultModelConfig, KernelConfig};
use std::path::PathBuf;
use std::sync::Arc;
use tower::ServiceExt;

const TEST_API_KEY: &str = "test-secret-key";

struct Harness {
    app: Router,
    home_dir: PathBuf,
    _tmp: tempfile::TempDir,
    _state: Arc<AppState>,
}

impl Drop for Harness {
    fn drop(&mut self) {
        self._state.kernel.shutdown();
    }
}

async fn boot() -> Harness {
    let tmp = tempfile::tempdir().expect("tempdir");

    // Populate the registry cache so the kernel boots without network.
    librefang_kernel::registry_sync::sync_registry(
        tmp.path(),
        librefang_kernel::registry_sync::DEFAULT_CACHE_TTL_SECS,
        "",
        None,
    );

    let config = KernelConfig {
        home_dir: tmp.path().to_path_buf(),
        data_dir: tmp.path().join("data"),
        api_key: TEST_API_KEY.to_string(),
        default_model: DefaultModelConfig {
            provider: "ollama".to_string(),
            model: "test-model".to_string(),
            api_key_env: "OLLAMA_API_KEY".to_string(),
            base_url: None,
            message_timeout_secs: 300,
            extra_params: std::collections::BTreeMap::new(),
            cli_profile_dirs: Vec::new(),
        },
        ..KernelConfig::default()
    };

    let kernel = LibreFangKernel::boot_with_config(config).expect("kernel boot");
    let kernel = Arc::new(kernel);
    kernel.set_self_handle();
    let home_dir = kernel.home_dir().to_path_buf();

    let (app, state) = server::build_router(kernel, "127.0.0.1:0".parse().expect("addr")).await;

    Harness {
        app,
        home_dir,
        _tmp: tmp,
        _state: state,
    }
}

async fn post_json(
    app: &Router,
    path: &str,
    body: serde_json::Value,
) -> (StatusCode, serde_json::Value) {
    let req = Request::builder()
        .method(Method::POST)
        .uri(path)
        .header(header::AUTHORIZATION, format!("Bearer {TEST_API_KEY}"))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
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

/// The response `path` must be **relative** — must not start with the
/// host's home_dir, and must not contain the OS-username-bearing
/// `/Users/` or `/home/` prefixes from the canonical Unix layouts.
///
/// This is the regression test for the registry-content abs-path leak
/// noted in `docs/issues/registry-content-abs-path-leak.md`.
#[tokio::test(flavor = "multi_thread")]
async fn registry_content_path_is_relative_not_absolute() {
    let h = boot().await;
    let body = serde_json::json!({
        "id": "test-provider-abs-path-leak",
        "display_name": "Test Provider",
        "api_key_env": "TEST_PROVIDER_ABS_PATH_LEAK_API_KEY",
        "base_url": "https://example.invalid/v1",
        "key_required": false
    });
    let (status, resp) = post_json(&h.app, "/api/registry/content/provider", body).await;
    assert_eq!(status, StatusCode::OK, "unexpected status: {resp}");
    assert_eq!(resp.get("ok").and_then(|v| v.as_bool()), Some(true));

    let path = resp
        .get("path")
        .and_then(|v| v.as_str())
        .expect("response missing `path` field");

    // Must not be the absolute path — i.e. must not contain the
    // tempdir's home_dir prefix.
    let home_str = h.home_dir.display().to_string();
    assert!(
        !path.contains(&home_str),
        "response `path` leaks absolute home_dir: path={path:?}, home_dir={home_str:?}"
    );

    // Must not look like an absolute Unix path at all.
    assert!(
        !path.starts_with('/'),
        "response `path` is absolute (starts with `/`): {path:?}"
    );

    // Sanity: the expected relative form is `providers/<id>.toml`.
    assert_eq!(
        path, "providers/test-provider-abs-path-leak.toml",
        "expected relative `providers/<id>.toml`, got {path:?}"
    );
}

/// API-surface-hygiene roundup (#2: registry id validation). The identifier is
/// joined into a filesystem path, so it is validated against
/// `^[a-zA-Z0-9._-]+$` with a 128-char cap. Anything outside the allowlist —
/// path separators, `..`, whitespace, shell metacharacters, or an over-long
/// value — must be rejected with 400.
#[tokio::test(flavor = "multi_thread")]
async fn registry_content_rejects_malformed_identifiers() {
    let h = boot().await;

    let long = "a".repeat(129);
    let bad_ids: &[&str] = &[
        "../etc",
        "..",
        ".",
        "a/b",
        "a\\b",
        "a b",
        "a;rm -rf",
        "a$(whoami)",
        "naïve",
        long.as_str(),
        "", // empty after extraction → "Missing" or "Invalid", both 400
    ];

    for id in bad_ids {
        let body = serde_json::json!({ "id": id });
        let (status, resp) = post_json(&h.app, "/api/registry/content/provider", body).await;
        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "identifier {id:?} must be rejected with 400; got {status} / {resp}"
        );
    }
}

/// Well-formed identifiers that exercise the full allowlist (alphanumerics plus
/// `.`, `_`, `-`) are accepted.
#[tokio::test(flavor = "multi_thread")]
async fn registry_content_accepts_dotted_and_dashed_identifiers() {
    let h = boot().await;
    let body = serde_json::json!({
        "id": "my.provider-name_1",
        "display_name": "OK",
        "api_key_env": "MY_PROVIDER_NAME_1_API_KEY",
        "base_url": "https://example.invalid/v1",
        "key_required": false
    });
    let (status, resp) = post_json(&h.app, "/api/registry/content/provider", body).await;
    assert_eq!(status, StatusCode::OK, "unexpected status: {resp}");
    assert_eq!(
        resp.get("path").and_then(|v| v.as_str()),
        Some("providers/my.provider-name_1.toml")
    );
}

async fn get_json(app: &Router, path: &str) -> (StatusCode, serde_json::Value) {
    let req = Request::builder()
        .method(Method::GET)
        .uri(path)
        .header(header::AUTHORIZATION, format!("Bearer {TEST_API_KEY}"))
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), 1 << 20)
        .await
        .unwrap();
    let value: serde_json::Value =
        serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
    (status, value)
}

/// #5822: creating a custom provider whose model carries the out-of-vocabulary
/// tier `"reasoning"` (which the dashboard used to offer) must succeed AND the
/// model must actually load into the catalog. Previously the catalog TOML
/// parser rejected the unknown tier, the merge was swallowed as a `warn!`, and
/// the provider silently never appeared — the endpoint still returned 200, so
/// asserting the status alone is not enough; we read the catalog back.
#[tokio::test(flavor = "multi_thread")]
async fn provider_with_reasoning_tier_model_loads_into_catalog() {
    let h = boot().await;
    let body = serde_json::json!({
        "id": "myai-ds4",
        "display_name": "My AI",
        "api_key_env": "MYAI_DS4_API_KEY", // pragma: allowlist secret
        "base_url": "https://example.invalid/v1",
        "key_required": false,
        "models": [{
            "id": "myai-reasoner-x",
            "display_name": "MyAI Reasoner X",
            "tier": "reasoning",
            "context_window": 128000,
            "max_output_tokens": 8192,
            "input_cost_per_m": 1.0,
            "output_cost_per_m": 2.0
        }]
    });
    let (status, resp) = post_json(&h.app, "/api/registry/content/provider", body).await;
    assert_eq!(status, StatusCode::OK, "create must succeed: {resp}");

    let (status, models) = get_json(&h.app, "/api/models").await;
    assert_eq!(status, StatusCode::OK);
    let raw = models.to_string();
    assert!(
        raw.contains("myai-reasoner-x"),
        "the reasoning-tier model must be present in the catalog; got: {raw}"
    );
}

/// A provider definition that fails to load into the catalog (here: a model
/// whose `context_window` is the wrong TOML type) must be reported to the
/// caller as a 400 — not swallowed as a success — and the rejected file must
/// not be left behind on disk to break catalog parsing on every boot.
#[tokio::test(flavor = "multi_thread")]
async fn provider_rejected_by_catalog_returns_400_and_leaves_no_file() {
    let h = boot().await;
    let body = serde_json::json!({
        "id": "broken-provider",
        "display_name": "Broken",
        "api_key_env": "BROKEN_PROVIDER_API_KEY", // pragma: allowlist secret
        "base_url": "https://example.invalid/v1",
        "key_required": false,
        "models": [{
            "id": "broken-model",
            "display_name": "Broken Model",
            "tier": "fast",
            "context_window": "not-a-number",
            "max_output_tokens": 8192,
            "input_cost_per_m": 1.0,
            "output_cost_per_m": 2.0
        }]
    });
    let (status, resp) = post_json(&h.app, "/api/registry/content/provider", body).await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "a provider the catalog rejects must surface a 400, got {status} / {resp}"
    );

    let file = h.home_dir.join("providers").join("broken-provider.toml");
    assert!(
        !file.exists(),
        "the rejected provider file must be rolled back, not left at {file:?}"
    );
}
