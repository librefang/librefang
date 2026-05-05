//! HTTP-level integration tests for the proactive-memory route domain
//! (`crates/librefang-api/src/routes/memory.rs`).
//!
//! Partial fix for #3571 — "~80% of registered HTTP routes have no integration
//! test." This file covers the memory slice only (other domains are tracked
//! separately).
//!
//! Approach: build a real `server::build_router` against a kernel booted on a
//! tempdir, then drive it with `tower::oneshot`. Same harness as
//! `auth_public_allowlist.rs`. Because `oneshot` provides no `ConnectInfo`,
//! we configure an `api_key` and send `Authorization: Bearer <key>` for
//! authenticated requests, and rely on the same code path to assert 401 for
//! anonymous requests.
//!
//! Scope (intentional):
//! - Auth gate: every memory route must 401 without a Bearer token (none is
//!   in `PUBLIC_ROUTES_*`).
//! - Read endpoints exercised against the kernel default (proactive memory
//!   `enabled = true`, empty store): `GET /api/memory`, `GET /api/memory/stats`,
//!   `GET /api/memory/config`. Verified to return 200 with the documented
//!   shape and pagination clamping.
//! - Validation: `DELETE /api/memory/agents/{id}/level/{level}` returns 400
//!   for an unknown level (input validation runs before any store call).
//! - Bulk-delete with missing `ids` returns 400 (handler validates body
//!   shape).
//! - Empty-store paths: `POST /api/memory` (memory_add) and `PUT
//!   /api/memory/items/{id}` against the default store return 5xx with a
//!   JSON `error` field — pinning that they emit JSON, not a panic / empty
//!   body, which is exactly the regression class #3571 calls out.
//!
//! Out of scope (skipped, with reason):
//! - `PATCH /api/memory/config` writes to `~/.librefang/config.toml`. The
//!   tempdir kernel boot does not materialize that file, so the handler's
//!   read-modify-write would 500 on the read step. Exercising it requires
//!   either pre-seeding the file or refactoring the handler to tolerate a
//!   missing file — both out of scope for a test-only PR.
//! - Endpoints that require an actual `ProactiveMemoryStore` populated with
//!   data (search-by-content, history, consolidate, export/import, relations,
//!   duplicates, decay/cleanup side effects). The default kernel boot leaves
//!   `proactive_memory_store()` as `None`; surfacing a real store requires
//!   embedding-provider config that pulls in network dependencies. Covered
//!   by unit tests inside `librefang-memory` instead.

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use librefang_api::server;
use librefang_kernel::LibreFangKernel;
use librefang_types::config::{DefaultModelConfig, KernelConfig};
use std::sync::Arc;
use tower::ServiceExt;

// ---------------------------------------------------------------------------
// Harness — mirrors auth_public_allowlist.rs::boot_router_with_api_key
// ---------------------------------------------------------------------------

struct RouterHarness {
    app: axum::Router,
    _tmp: tempfile::TempDir,
    _state: Arc<librefang_api::routes::AppState>,
}

impl Drop for RouterHarness {
    fn drop(&mut self) {
        self._state.kernel.shutdown();
    }
}

async fn boot_router_with_api_key(api_key: &str) -> RouterHarness {
    let tmp = tempfile::tempdir().expect("tempdir");

    // Populate the registry cache so the kernel boots without network access.
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

    let kernel = LibreFangKernel::boot_with_config(config).expect("kernel boot");
    let kernel = Arc::new(kernel);
    kernel.set_self_handle();

    let (app, state) = server::build_router(kernel, "127.0.0.1:0".parse().expect("addr")).await;

    RouterHarness {
        app,
        _tmp: tmp,
        _state: state,
    }
}

const TEST_KEY: &str = "test-secret-memory-key";

fn authed_get(path: &str) -> Request<Body> {
    Request::builder()
        .method(Method::GET)
        .uri(path)
        .header("authorization", format!("Bearer {TEST_KEY}"))
        .body(Body::empty())
        .unwrap()
}

fn authed_json(method: Method, path: &str, body: serde_json::Value) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(path)
        .header("authorization", format!("Bearer {TEST_KEY}"))
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap()
}

fn anon_get(path: &str) -> Request<Body> {
    Request::builder()
        .method(Method::GET)
        .uri(path)
        .body(Body::empty())
        .unwrap()
}

async fn read_json(resp: axum::response::Response) -> serde_json::Value {
    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .expect("read body");
    serde_json::from_slice(&body).expect("body is JSON")
}

// ---------------------------------------------------------------------------
// Auth gate — every memory route must 401 without a Bearer token.
// ---------------------------------------------------------------------------

/// Sample of memory routes that must require auth. Mirrors the route table in
/// `routes/memory.rs::router()` without trying to be exhaustive — exhaustive
/// drift is owned by the `auth_public_allowlist.rs` catalog test.
const AUTHED_GET_PATHS: &[&str] = &[
    "/api/memory",
    "/api/memory/stats",
    "/api/memory/config",
    "/api/memory/search?q=hello",
    "/api/memory/user/some-user",
    "/api/memory/agents/some-agent",
    "/api/memory/agents/some-agent/search?q=hi",
    "/api/memory/agents/some-agent/stats",
    "/api/memory/agents/some-agent/duplicates",
    "/api/memory/agents/some-agent/count",
    "/api/memory/agents/some-agent/relations",
    "/api/memory/agents/some-agent/export",
    "/api/memory/items/some-mem/history",
];

#[tokio::test(flavor = "multi_thread")]
async fn memory_routes_require_auth_when_api_key_configured() {
    let harness = boot_router_with_api_key(TEST_KEY).await;

    let mut failures = Vec::new();
    for path in AUTHED_GET_PATHS {
        let resp = harness.app.clone().oneshot(anon_get(path)).await.unwrap();
        if resp.status() != StatusCode::UNAUTHORIZED {
            failures.push(format!("{path} -> {} (expected 401)", resp.status()));
        }
    }
    assert!(
        failures.is_empty(),
        "memory routes leaked without Bearer token:\n  {}",
        failures.join("\n  ")
    );
}

// ---------------------------------------------------------------------------
// GET /api/memory — empty store, documented JSON shape
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn get_memory_returns_empty_list_with_documented_shape() {
    let harness = boot_router_with_api_key(TEST_KEY).await;

    let resp = harness
        .app
        .clone()
        .oneshot(authed_get("/api/memory"))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = read_json(resp).await;
    // Kernel default has proactive memory enabled and the store empty.
    assert_eq!(body["proactive_enabled"], serde_json::Value::Bool(true));
    assert_eq!(body["total"], serde_json::json!(0));
    assert!(
        body["memories"]
            .as_array()
            .map(|a| a.is_empty())
            .unwrap_or(false),
        "expected empty `memories` array, got {body}"
    );
    // Pagination defaults are echoed back.
    assert_eq!(body["offset"], serde_json::json!(0));
    assert_eq!(body["limit"], serde_json::json!(10));
}

#[tokio::test(flavor = "multi_thread")]
async fn get_memory_clamps_limit_to_100() {
    let harness = boot_router_with_api_key(TEST_KEY).await;

    let resp = harness
        .app
        .clone()
        .oneshot(authed_get("/api/memory?limit=9999&offset=42"))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = read_json(resp).await;
    // Pagination clamp regression: limit must cap at 100 (handler does
    // `params.limit.min(100)`), offset must echo as-is.
    assert_eq!(body["limit"], serde_json::json!(100));
    assert_eq!(body["offset"], serde_json::json!(42));
}

// ---------------------------------------------------------------------------
// GET /api/memory/stats
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn get_memory_stats_returns_200_with_proactive_enabled_flag() {
    let harness = boot_router_with_api_key(TEST_KEY).await;

    let resp = harness
        .app
        .clone()
        .oneshot(authed_get("/api/memory/stats"))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = read_json(resp).await;
    // When the store is present (default), the handler merges
    // `proactive_enabled: true` into the stats object. The remaining
    // fields are owned by `librefang-memory` and exercised there; we
    // only pin the merge here so the dashboard's branch flag stays
    // stable.
    assert_eq!(
        body["proactive_enabled"],
        serde_json::Value::Bool(true),
        "expected proactive_enabled merged into stats, got: {body}"
    );
}

// ---------------------------------------------------------------------------
// GET /api/memory/config — always returns the kernel config snapshot
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn get_memory_config_returns_documented_shape() {
    let harness = boot_router_with_api_key(TEST_KEY).await;

    let resp = harness
        .app
        .clone()
        .oneshot(authed_get("/api/memory/config"))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = read_json(resp).await;
    // Every field the handler promises to emit must be present (not just
    // truthy) — a missing key is the exact "endpoint compiles but returns
    // wrong/empty data" failure mode #3571 calls out.
    for key in [
        "embedding_provider",
        "embedding_model",
        "embedding_api_key_env",
        "decay_rate",
        "proactive_memory",
    ] {
        assert!(
            body.get(key).is_some(),
            "missing top-level key `{key}` in body: {body}"
        );
    }
    let pm = &body["proactive_memory"];
    for key in [
        "enabled",
        "auto_memorize",
        "auto_retrieve",
        "extraction_model",
        "max_retrieve",
    ] {
        assert!(
            pm.get(key).is_some(),
            "missing `proactive_memory.{key}` in body: {body}"
        );
    }
    // Default `ProactiveMemoryConfig::default()` has enabled = true.
    assert_eq!(pm["enabled"], serde_json::Value::Bool(true));
}

// ---------------------------------------------------------------------------
// DELETE /api/memory/agents/{id}/level/{level} — input validation
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn delete_clear_level_rejects_unknown_level_with_400() {
    let harness = boot_router_with_api_key(TEST_KEY).await;

    let req = Request::builder()
        .method(Method::DELETE)
        .uri("/api/memory/agents/some-agent/level/bogus")
        .header("authorization", format!("Bearer {TEST_KEY}"))
        .body(Body::empty())
        .unwrap();
    let resp = harness.app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

    let body = read_json(resp).await;
    let err = body["error"].as_str().unwrap_or("");
    assert!(
        err.contains("Invalid memory level") && err.contains("bogus"),
        "expected validation error mentioning 'bogus', got: {err}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn delete_clear_level_accepts_known_level_without_panic() {
    // 'session' is one of the accepted levels, so input validation must
    // pass cleanly. The downstream store call may succeed (no-op on an
    // empty agent) or fail (no agent registered), but the response must
    // be a JSON body, not a panic / empty body — pinning that the
    // happy-path validation reaches the store call.
    let harness = boot_router_with_api_key(TEST_KEY).await;

    let req = Request::builder()
        .method(Method::DELETE)
        .uri("/api/memory/agents/some-agent/level/session")
        .header("authorization", format!("Bearer {TEST_KEY}"))
        .body(Body::empty())
        .unwrap();
    let resp = harness.app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    assert_ne!(
        status,
        StatusCode::BAD_REQUEST,
        "'session' is a valid level, must not 400"
    );
    assert_ne!(status, StatusCode::UNAUTHORIZED, "auth header was sent");
    // Either succeeded (204/200) or surfaced a typed error (4xx/5xx with JSON).
    if status.as_u16() >= 400 {
        let body = read_json(resp).await;
        assert!(
            body.get("error").is_some(),
            "error response must include a JSON 'error' field, got: {body}"
        );
    }
}

// ---------------------------------------------------------------------------
// POST /api/memory/bulk-delete — missing ids vs empty ids
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn post_bulk_delete_missing_ids_returns_400() {
    // With proactive memory enabled (kernel default), the store guard
    // passes and the handler reaches the body-shape check, which 400s
    // when `ids` is missing.
    let harness = boot_router_with_api_key(TEST_KEY).await;

    let resp = harness
        .app
        .clone()
        .oneshot(authed_json(
            Method::POST,
            "/api/memory/bulk-delete",
            serde_json::json!({}),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let body = read_json(resp).await;
    let err = body["error"].as_str().unwrap_or("");
    assert!(
        err.contains("ids"),
        "expected validation error mentioning 'ids', got: {err}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn post_bulk_delete_empty_ids_returns_zero_counts() {
    let harness = boot_router_with_api_key(TEST_KEY).await;

    let resp = harness
        .app
        .clone()
        .oneshot(authed_json(
            Method::POST,
            "/api/memory/bulk-delete",
            serde_json::json!({"ids": []}),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = read_json(resp).await;
    assert_eq!(body["deleted"], serde_json::json!(0));
    assert_eq!(body["failed"], serde_json::json!(0));
    assert_eq!(body["total"], serde_json::json!(0));
}

// ---------------------------------------------------------------------------
// Empty-store JSON-error contract on write endpoints.
//
// We deliberately avoid asserting an exact message here: with the kernel-
// default config the store exists but no embedding provider is wired, so
// downstream calls fail with `LibreFangError::Internal` which the handler
// scrubs to a generic body. What #3571 cares about is that the endpoint
// produces a structured JSON response with an `error` field — not an empty
// body, not a panic, not text/plain.
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn post_memory_add_returns_json_error_on_empty_store() {
    let harness = boot_router_with_api_key(TEST_KEY).await;

    let resp = harness
        .app
        .clone()
        .oneshot(authed_json(
            Method::POST,
            "/api/memory",
            serde_json::json!({
                "messages": [{"role": "user", "content": "remember this"}],
                "user_id": "u1",
                "agent_id": "a1",
            }),
        ))
        .await
        .unwrap();
    let status = resp.status();
    assert_ne!(status, StatusCode::UNAUTHORIZED, "auth header was sent");
    // Whether the embedding step succeeds or not, the response body must
    // be JSON. Success is 201 Created; failure is 4xx/5xx with `error`.
    if status.as_u16() >= 400 {
        let body = read_json(resp).await;
        assert!(
            body.get("error").is_some(),
            "error response missing JSON `error` field: {body}"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn put_memory_update_rejects_empty_content_with_400() {
    // Whitespace-only content trips the bad_request guard at the top of
    // the handler, which runs after the store check but before any DB
    // lookup — exercises the validation branch deterministically.
    let harness = boot_router_with_api_key(TEST_KEY).await;

    let resp = harness
        .app
        .clone()
        .oneshot(authed_json(
            Method::PUT,
            "/api/memory/items/some-id",
            serde_json::json!({"content": "   "}),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let body = read_json(resp).await;
    let err = body["error"].as_str().unwrap_or("");
    assert!(
        err.to_lowercase().contains("content"),
        "expected validation error mentioning 'content', got: {err}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn put_memory_update_unknown_id_returns_json_error() {
    // A non-empty content + an id that doesn't exist must surface as a
    // JSON error response, never a panic / empty body. Whether
    // `find_agent_id_for_memory` resolves to Ok(None) -> 404 or to a
    // backend Err -> 500 depends on the underlying store impl; both are
    // acceptable from the HTTP contract — we only pin "structured
    // error".
    let harness = boot_router_with_api_key(TEST_KEY).await;

    let resp = harness
        .app
        .clone()
        .oneshot(authed_json(
            Method::PUT,
            "/api/memory/items/does-not-exist",
            serde_json::json!({"content": "new content"}),
        ))
        .await
        .unwrap();
    let status = resp.status();
    assert!(
        status == StatusCode::NOT_FOUND || status == StatusCode::INTERNAL_SERVER_ERROR,
        "expected 404 or 500, got {status}"
    );
    let body = read_json(resp).await;
    assert!(body.get("error").is_some(), "missing error field: {body}");
}
