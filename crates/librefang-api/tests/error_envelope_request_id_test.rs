//! Integration tests for the #3639 error envelope + request_id correlation
//! and the server-side limit cap on the canonicalised list endpoints.
//!
//! What this exercises (end-to-end through the real `request_logging`
//! middleware):
//!
//! 1. Every JSON 4xx/5xx response carries a top-level `request_id` field
//!    that matches the `x-request-id` response header.
//! 2. Every JSON 4xx/5xx response carries a stable `code` token derived
//!    from the HTTP status when the handler didn't supply one.
//! 3. The middleware leaves 2xx response bodies alone (no spurious fields).
//! 4. The middleware honours a handler-supplied `code` and `request_id`
//!    instead of overwriting them.
//! 5. `/api/peers` and `/api/skills` paginate via `?offset=&limit=` and
//!    cap `limit` at 100 server-side (PAGINATION_MAX_LIMIT).
//!
//! These run on `tower::oneshot` against a router we hand-build to wrap
//! exactly the middleware we care about, mirroring `server.rs` for the
//! single layer under test. We deliberately do NOT spin up the full
//! production router because the test contract is exercise of the
//! middleware itself, and the smaller harness keeps the run fast.

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use axum::routing::get;
use axum::Router;
use librefang_api::middleware::{request_logging, REQUEST_ID_HEADER};
use librefang_api::routes::{self, AppState};
use librefang_api::types::{ApiErrorResponse, PAGINATION_MAX_LIMIT};
use librefang_testing::{MockKernelBuilder, TestAppState};
use std::sync::Arc;
use tower::ServiceExt;

/// Build a minimal router that mounts `/api/peers` and `/api/skills` (the
/// two list endpoints under #3639 review) plus a synthetic 404 handler the
/// envelope tests can hit without depending on any real route's i18n keys.
fn boot_full_harness() -> (Router, TestAppState) {
    let test = TestAppState::with_builder(MockKernelBuilder::new().with_config(|cfg| {
        // Non-LLM provider keeps boot fast.
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
    let state: Arc<AppState> = test.state.clone();

    // Synthetic handlers exercising the middleware's body-rewriting path.
    async fn synthetic_404() -> ApiErrorResponse {
        ApiErrorResponse::not_found("synthetic missing resource")
    }
    async fn synthetic_500_with_handler_set_fields() -> ApiErrorResponse {
        ApiErrorResponse::internal("synthetic blow-up")
            .with_code("custom_handler_code")
            .with_request_id("handler-supplied-id")
    }
    async fn synthetic_200() -> axum::Json<serde_json::Value> {
        axum::Json(serde_json::json!({"ok": true}))
    }

    let app = Router::new()
        .nest("/api", routes::network::router())
        .nest("/api", routes::skills::router())
        .route("/test/synthetic-404", get(synthetic_404))
        .route(
            "/test/synthetic-500-handler-fields",
            get(synthetic_500_with_handler_set_fields),
        )
        .route("/test/synthetic-200", get(synthetic_200))
        .with_state(state)
        .layer(axum::middleware::from_fn(request_logging));

    (app, test)
}

/// Send a GET request through the harness and return `(status, headers, body)`.
async fn send(app: &Router, uri: &str) -> (StatusCode, axum::http::HeaderMap, serde_json::Value) {
    let req = Request::builder()
        .method(Method::GET)
        .uri(uri)
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let headers = resp.headers().clone();
    let bytes = axum::body::to_bytes(resp.into_body(), 1_048_576)
        .await
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
    (status, headers, body)
}

// ---------------------------------------------------------------------------
// Error envelope + request_id correlation
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn json_404_carries_request_id_matching_header() {
    let (app, _t) = boot_full_harness();
    let (status, headers, body) = send(&app, "/test/synthetic-404").await;

    assert_eq!(status, StatusCode::NOT_FOUND);
    let header_id = headers
        .get(REQUEST_ID_HEADER)
        .expect("middleware must set x-request-id header on every response")
        .to_str()
        .unwrap()
        .to_string();

    let body_id = body
        .get("request_id")
        .and_then(|v| v.as_str())
        .expect("4xx JSON body must carry a request_id field (#3639)");
    assert_eq!(
        body_id, header_id,
        "body request_id must equal the x-request-id header"
    );
    // UUID v4 sanity: 36 chars, two hyphens at fixed positions.
    assert_eq!(
        body_id.len(),
        36,
        "request_id must be a UUID, got {body_id}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn json_404_carries_default_stable_code() {
    let (app, _t) = boot_full_harness();
    let (_status, _headers, body) = send(&app, "/test/synthetic-404").await;

    let code = body
        .get("code")
        .and_then(|v| v.as_str())
        .expect("4xx JSON body must carry a stable code field (#3639)");
    assert_eq!(code, "not_found", "404 default code must be `not_found`");

    // The legacy `type` alias is mirrored for old clients.
    let typ = body
        .get("type")
        .and_then(|v| v.as_str())
        .expect("legacy `type` alias must be mirrored alongside `code`");
    assert_eq!(typ, "not_found");
}

#[tokio::test(flavor = "multi_thread")]
async fn middleware_does_not_overwrite_handler_supplied_code_or_request_id() {
    let (app, _t) = boot_full_harness();
    let (status, headers, body) = send(&app, "/test/synthetic-500-handler-fields").await;

    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);

    // Header still carries the middleware-generated id (handlers cannot
    // shape the response header from inside their own body).
    assert!(headers.contains_key(REQUEST_ID_HEADER));

    // But the body's `code` and `request_id` are whatever the handler set —
    // the middleware must not overwrite them.
    assert_eq!(
        body.get("code").and_then(|v| v.as_str()),
        Some("custom_handler_code")
    );
    assert_eq!(
        body.get("request_id").and_then(|v| v.as_str()),
        Some("handler-supplied-id")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn json_2xx_responses_are_left_untouched() {
    let (app, _t) = boot_full_harness();
    let (status, headers, body) = send(&app, "/test/synthetic-200").await;

    assert_eq!(status, StatusCode::OK);
    // Header IS still set (response-side stamp is unconditional).
    assert!(headers.contains_key(REQUEST_ID_HEADER));
    // But the body must NOT have been rewritten — no `request_id` / `code`
    // injected on success.
    assert!(
        body.get("request_id").is_none(),
        "2xx body must not be rewritten by the error-envelope middleware"
    );
    assert!(body.get("code").is_none());
    assert_eq!(body.get("ok").and_then(|v| v.as_bool()), Some(true));
}

// ---------------------------------------------------------------------------
// /api/peers + /api/skills pagination cap (#3639)
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn peers_list_uses_paginated_envelope() {
    let (app, _t) = boot_full_harness();
    let (status, _headers, body) = send(&app, "/api/peers").await;

    assert_eq!(status, StatusCode::OK);
    // Canonical PaginatedResponse fields, even on an empty registry.
    assert!(
        body.get("items").is_some(),
        "peers response must use the canonical `items` field"
    );
    assert!(
        body.get("total").is_some(),
        "peers response must include `total`"
    );
    assert!(
        body.get("offset").is_some(),
        "peers response must include `offset`"
    );
    // `limit` is only set when the caller passed pagination params; with
    // neither offset nor limit set, the field stays absent (preserves the
    // legacy unbounded shape, see PaginationQuery::paginate).
}

#[tokio::test(flavor = "multi_thread")]
async fn peers_list_clamps_limit_at_max() {
    let (app, _t) = boot_full_harness();
    // Ask for 9999; server must clamp at PAGINATION_MAX_LIMIT (100).
    let (status, _headers, body) = send(&app, "/api/peers?limit=9999").await;

    assert_eq!(status, StatusCode::OK);
    let limit = body
        .get("limit")
        .and_then(|v| v.as_u64())
        .expect("limit field must be present once pagination is engaged");
    assert_eq!(
        limit as usize, PAGINATION_MAX_LIMIT,
        "/api/peers must cap limit server-side at PAGINATION_MAX_LIMIT (#3639)"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn skills_list_clamps_limit_at_max() {
    let (app, _t) = boot_full_harness();
    let (status, _headers, body) = send(&app, "/api/skills?limit=9999").await;

    assert_eq!(status, StatusCode::OK);
    let limit = body
        .get("limit")
        .and_then(|v| v.as_u64())
        .expect("/api/skills `limit` field must be present once pagination is engaged");
    assert_eq!(
        limit as usize, PAGINATION_MAX_LIMIT,
        "/api/skills must cap limit server-side at PAGINATION_MAX_LIMIT (#3639)"
    );
    // `items` is the canonical PaginatedResponse field; the skills handler
    // also includes a `categories` sidecar which is allowed.
    assert!(body.get("items").is_some());
    assert!(body.get("total").is_some());
}
