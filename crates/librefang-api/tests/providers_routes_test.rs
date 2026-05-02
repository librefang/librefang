//! Integration tests for the model-catalog & provider-management endpoints.
//!
//! Refs #3571 — "~80% of registered HTTP routes have no integration test."
//! This file targets the providers/models slice (`crates/librefang-api/src/
//! routes/providers.rs`). It mounts the real `providers::router()` against a
//! `MockKernel`-backed `AppState` and exercises happy + error paths through
//! `tower::ServiceExt::oneshot` — same harness pattern as `users_test.rs`.
//!
//! Out of scope (not exercised here, by design):
//!   - `POST /api/providers/{name}/key`             — mutates global `std::env`
//!   - `DELETE /api/providers/{name}/key`           — mutates global `std::env`
//!   - `POST /api/providers/github-copilot/oauth/*` — outbound device-flow HTTP
//!   - `GET  /api/providers/ollama/detect`          — outbound HTTP probe
//!   - `POST /api/catalog/update`                   — outbound network sync
//!   - `POST /api/providers/{name}/test` (success)  — outbound HTTP / CLI probe
//!     (only the unknown-provider 404 branch is verified — pure catalog lookup)
//!
//! These would either flake on CI (real network) or contaminate other test
//! binaries running in parallel via `std::env::set_var`. Per CLAUDE.md
//! "no global env mutation, no fs writes outside tempfile."

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
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

/// Boots a kernel with a sane default-model provider so handlers that fall
/// back to `config.default_model.provider` (notably `add_custom_model`)
/// don't end up tagging entries with the placeholder `"auto"` provider.
fn boot() -> Harness {
    let test = TestAppState::with_builder(MockKernelBuilder::new().with_config(|cfg| {
        cfg.default_model = librefang_types::config::DefaultModelConfig {
            provider: "openai".to_string(),
            model: "gpt-4o-mini".to_string(),
            api_key_env: "OPENAI_API_KEY".to_string(),
            base_url: None,
            message_timeout_secs: 300,
            extra_params: std::collections::HashMap::new(),
            cli_profile_dirs: Vec::new(),
        };
    }));

    let state = test.state.clone();
    let app = Router::new()
        .nest("/api", routes::providers::router())
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
// GET /api/models
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn list_models_returns_well_formed_envelope() {
    let h = boot();
    let (status, body) = json_request(&h, Method::GET, "/api/models", None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.get("models").and_then(|v| v.as_array()).is_some());
    assert!(body.get("total").and_then(|v| v.as_u64()).is_some());
    assert!(body.get("available").and_then(|v| v.as_u64()).is_some());
    // Built-in catalog has at least one entry from the registry.
    assert!(body["total"].as_u64().unwrap() > 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn list_models_filters_by_unknown_provider_yields_empty() {
    let h = boot();
    let (status, body) = json_request(
        &h,
        Method::GET,
        "/api/models?provider=__no_such_provider__",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["models"].as_array().unwrap().len(), 0);
}

// ---------------------------------------------------------------------------
// GET /api/models/{id}
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn get_model_unknown_id_returns_404() {
    let h = boot();
    let (status, body) = json_request(&h, Method::GET, "/api/models/__no_such_model__", None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(body.get("error").is_some() || body.get("message").is_some());
}

// ---------------------------------------------------------------------------
// Aliases — list / create / delete round-trip
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn aliases_list_starts_with_envelope() {
    let h = boot();
    let (status, body) = json_request(&h, Method::GET, "/api/models/aliases", None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.get("aliases").and_then(|v| v.as_array()).is_some());
    assert!(body.get("total").and_then(|v| v.as_u64()).is_some());
}

#[tokio::test(flavor = "multi_thread")]
async fn create_alias_rejects_missing_alias_field() {
    let h = boot();
    let (status, _body) = json_request(
        &h,
        Method::POST,
        "/api/models/aliases",
        Some(serde_json::json!({ "model_id": "gpt-4o" })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test(flavor = "multi_thread")]
async fn create_alias_rejects_missing_model_id_field() {
    let h = boot();
    let (status, _body) = json_request(
        &h,
        Method::POST,
        "/api/models/aliases",
        Some(serde_json::json!({ "alias": "fast" })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test(flavor = "multi_thread")]
async fn create_alias_then_list_then_delete_round_trips() {
    let h = boot();

    // Create
    let (status, body) = json_request(
        &h,
        Method::POST,
        "/api/models/aliases",
        Some(serde_json::json!({
            "alias": "Test-Alias-3571",
            "model_id": "gpt-4o-mini",
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    // Handler lowercases the alias name on return.
    assert_eq!(body["alias"].as_str().unwrap(), "test-alias-3571");
    assert_eq!(body["model_id"].as_str().unwrap(), "gpt-4o-mini");

    // List should include it.
    let (status, body) = json_request(&h, Method::GET, "/api/models/aliases", None).await;
    assert_eq!(status, StatusCode::OK);
    let entries = body["aliases"].as_array().unwrap();
    let found = entries.iter().any(|e| {
        e["alias"].as_str() == Some("test-alias-3571")
            && e["model_id"].as_str() == Some("gpt-4o-mini")
    });
    assert!(found, "newly created alias must appear in /models/aliases");

    // Duplicate should return 409.
    let (status, _body) = json_request(
        &h,
        Method::POST,
        "/api/models/aliases",
        Some(serde_json::json!({
            "alias": "Test-Alias-3571",
            "model_id": "gpt-4o-mini",
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);

    // Delete
    let (status, _body) = json_request(
        &h,
        Method::DELETE,
        "/api/models/aliases/test-alias-3571",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    // Second delete -> 404.
    let (status, _body) = json_request(
        &h,
        Method::DELETE,
        "/api/models/aliases/test-alias-3571",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

// ---------------------------------------------------------------------------
// Custom models — POST /api/models/custom + DELETE /api/models/custom/{id}
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn add_custom_model_rejects_missing_id() {
    let h = boot();
    let (status, _body) = json_request(
        &h,
        Method::POST,
        "/api/models/custom",
        Some(serde_json::json!({ "display_name": "no id" })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test(flavor = "multi_thread")]
async fn add_custom_model_then_get_then_delete_round_trips() {
    let h = boot();

    // Create
    let (status, body) = json_request(
        &h,
        Method::POST,
        "/api/models/custom",
        Some(serde_json::json!({
            "id": "test-custom-3571",
            "provider": "openai",
            "display_name": "Test Custom 3571",
            "context_window": 64_000,
            "max_output_tokens": 4_096,
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(body["id"].as_str().unwrap(), "test-custom-3571");
    assert_eq!(body["status"].as_str().unwrap(), "added");

    // Duplicate -> 409.
    let (status, _body) = json_request(
        &h,
        Method::POST,
        "/api/models/custom",
        Some(serde_json::json!({
            "id": "test-custom-3571",
            "provider": "openai",
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);

    // GET via /api/models/{id}
    let (status, body) = json_request(&h, Method::GET, "/api/models/test-custom-3571", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["id"].as_str().unwrap(), "test-custom-3571");
    assert_eq!(body["provider"].as_str().unwrap(), "openai");

    // Delete
    let (status, _body) = json_request(
        &h,
        Method::DELETE,
        "/api/models/custom/test-custom-3571",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    // Second delete -> 404.
    let (status, _body) = json_request(
        &h,
        Method::DELETE,
        "/api/models/custom/test-custom-3571",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

// ---------------------------------------------------------------------------
// Per-model overrides — GET / PUT / DELETE /api/models/overrides/{id}
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn model_overrides_unset_returns_empty_object() {
    let h = boot();
    let (status, body) = json_request(
        &h,
        Method::GET,
        "/api/models/overrides/openai:gpt-4o-mini",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    // Handler returns `{}` when no overrides exist for the key.
    assert!(body.is_object());
    assert!(body.as_object().unwrap().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn model_overrides_set_then_get_then_delete_round_trips() {
    let h = boot();

    // PUT
    let (status, body) = json_request(
        &h,
        Method::PUT,
        "/api/models/overrides/openai:gpt-4o-mini",
        Some(serde_json::json!({ "temperature": 0.42 })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["status"].as_str().unwrap(), "ok");

    // GET — overrides now present.
    let (status, body) = json_request(
        &h,
        Method::GET,
        "/api/models/overrides/openai:gpt-4o-mini",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        body.is_object() && !body.as_object().unwrap().is_empty(),
        "overrides body should be a non-empty object after PUT, got {body}"
    );

    // DELETE
    let (status, _body) = json_request(
        &h,
        Method::DELETE,
        "/api/models/overrides/openai:gpt-4o-mini",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    // GET again -> empty object.
    let (status, body) = json_request(
        &h,
        Method::GET,
        "/api/models/overrides/openai:gpt-4o-mini",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.is_object() && body.as_object().unwrap().is_empty());
}

// ---------------------------------------------------------------------------
// GET /api/providers + GET /api/providers/{name}
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn list_providers_returns_well_formed_envelope() {
    let h = boot();
    let (status, body) = json_request(&h, Method::GET, "/api/providers", None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.get("providers").and_then(|v| v.as_array()).is_some());
    assert!(body.get("total").and_then(|v| v.as_u64()).is_some());
    let providers = body["providers"].as_array().unwrap();
    // Every entry must have the required identity fields.
    for p in providers {
        assert!(p["id"].is_string(), "provider entry missing 'id': {p}");
        assert!(
            p["display_name"].is_string(),
            "provider entry missing 'display_name': {p}"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn get_provider_unknown_returns_404() {
    let h = boot();
    let (status, _body) =
        json_request(&h, Method::GET, "/api/providers/__no_such_provider__", None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

// ---------------------------------------------------------------------------
// POST /api/providers/{name}/test — only verify unknown-provider 404
// (the success branch performs outbound HTTP/CLI probes — see file header).
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn test_provider_unknown_returns_404() {
    let h = boot();
    let (status, _body) = json_request(
        &h,
        Method::POST,
        "/api/providers/__no_such_provider__/test",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

// ---------------------------------------------------------------------------
// PUT /api/providers/{name}/url — input validation
// (value-side path persists into config.toml under the temp-dir home,
// so it stays inside the harness sandbox.)
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn set_provider_url_rejects_missing_base_url() {
    let h = boot();
    let (status, _body) = json_request(
        &h,
        Method::PUT,
        "/api/providers/openai/url",
        Some(serde_json::json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test(flavor = "multi_thread")]
async fn set_provider_url_rejects_invalid_scheme() {
    let h = boot();
    let (status, _body) = json_request(
        &h,
        Method::PUT,
        "/api/providers/openai/url",
        Some(serde_json::json!({ "base_url": "ftp://example.com" })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test(flavor = "multi_thread")]
async fn set_provider_url_rejects_invalid_proxy_scheme() {
    let h = boot();
    let (status, _body) = json_request(
        &h,
        Method::PUT,
        "/api/providers/openai/url",
        Some(serde_json::json!({
            "base_url": "https://api.openai.com/v1",
            "proxy_url": "gopher://nope",
        })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

// ---------------------------------------------------------------------------
// POST /api/providers/{name}/default
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn set_default_provider_unknown_returns_404() {
    let h = boot();
    let (status, _body) = json_request(
        &h,
        Method::POST,
        "/api/providers/__no_such_provider__/default",
        Some(serde_json::json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

// ---------------------------------------------------------------------------
// GET /api/catalog/status — purely reads filesystem state (none in tempdir).
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn catalog_status_returns_last_sync_field() {
    let h = boot();
    let (status, body) = json_request(&h, Method::GET, "/api/catalog/status", None).await;
    assert_eq!(status, StatusCode::OK);
    // Field is always present; value may be null when no sync has run.
    assert!(
        body.get("last_sync").is_some(),
        "catalog status should always include 'last_sync' key, got {body}"
    );
}

// ---------------------------------------------------------------------------
// GET /api/providers/github-copilot/oauth/poll/{poll_id} — unknown id branch
// (the start endpoint hits GitHub; we only verify the lookup-failure path.)
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn copilot_oauth_poll_unknown_id_returns_404() {
    let h = boot();
    let (status, body) = json_request(
        &h,
        Method::GET,
        "/api/providers/github-copilot/oauth/poll/this-poll-id-does-not-exist",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["status"].as_str().unwrap(), "not_found");
}
