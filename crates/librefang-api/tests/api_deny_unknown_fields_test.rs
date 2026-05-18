//! Integration tests for `#[serde(deny_unknown_fields)]` on request DTOs (#5131).
//!
//! Before the fix, a typo'd body like `{"name": "...", "url": "...",
//! "evnts": ["foo"]}` deserialized to `CreateWebhookRequest` with the typo
//! silently dropped and `events` defaulting to `[]`. The server returned
//! 201 Created and the webhook never fired anything.
//!
//! With `deny_unknown_fields` on the DTO, serde rejects the payload up
//! front and axum's `Json` extractor surfaces 400 Bad Request. This file
//! locks in that behaviour for the representative case from the issue.
//!
//! Run: `cargo test -p librefang-api --test api_deny_unknown_fields_test`

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use librefang_api::routes::AppState;
use librefang_api::server;
use librefang_kernel::LibreFangKernel;
use librefang_types::config::{DefaultModelConfig, KernelConfig};
use std::sync::Arc;
use tower::ServiceExt;

struct Harness {
    app: axum::Router,
    state: Arc<AppState>,
    api_key: String,
    _tmp: tempfile::TempDir,
}

impl Drop for Harness {
    fn drop(&mut self) {
        self.state.kernel.shutdown();
    }
}

async fn boot(api_key: &str) -> Harness {
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

    let kernel = LibreFangKernel::boot_with_config(config).expect("kernel boot");
    let kernel = Arc::new(kernel);
    kernel.set_self_handle();

    let (app, state) = server::build_router(kernel, "127.0.0.1:0".parse().expect("addr")).await;

    Harness {
        app,
        state,
        api_key: api_key.to_string(),
        _tmp: tmp,
    }
}

async fn post_json(
    h: &Harness,
    path: &str,
    body: serde_json::Value,
) -> (StatusCode, serde_json::Value) {
    let req = Request::builder()
        .method(Method::POST)
        .uri(path)
        .header("authorization", format!("Bearer {}", h.api_key))
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap();
    let resp = h.app.clone().oneshot(req).await.expect("oneshot");
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), 1 << 20)
        .await
        .expect("body");
    let value: serde_json::Value = if bytes.is_empty() {
        serde_json::Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null)
    };
    (status, value)
}

/// Regression test for #5131: a misspelled `events` key (`evnts`) used to
/// be silently dropped, the webhook was created with the default empty
/// event list, and the caller saw a successful 201 while the feature
/// was effectively dead.  After applying `#[serde(deny_unknown_fields)]`
/// the payload is rejected at the deserialization boundary with 400.
#[tokio::test(flavor = "multi_thread")]
async fn create_webhook_rejects_typo_in_events_field() {
    let h = boot("test-secret").await;
    let body = serde_json::json!({
        "name": "typo-hook",
        "url": "https://example.com/hook",
        // typo: the field is `events`, not `evnts`
        "evnts": ["message_received"],
    });

    let (status, _v) = post_json(&h, "/api/webhooks", body).await;

    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "request body with an unknown field must be rejected with 400, \
         not silently accepted (#5131)"
    );

    // No webhook may have been persisted by the rejected request.
    let listed = h.state.webhook_store.list();
    assert!(
        listed.is_empty(),
        "rejected request must not create a webhook subscription; got: {:?}",
        listed
    );
}

/// Companion: the same handler accepts a correctly-spelled body, so the
/// previous test isn't simply demonstrating a broken endpoint.
#[tokio::test(flavor = "multi_thread")]
async fn create_webhook_accepts_correctly_spelled_body() {
    let h = boot("test-secret").await;
    let body = serde_json::json!({
        "name": "good-hook",
        "url": "https://example.com/hook",
        "events": ["message_received"],
    });

    let (status, v) = post_json(&h, "/api/webhooks", body).await;
    assert_eq!(status, StatusCode::CREATED, "body: {v}");
    assert!(
        v["id"].as_str().is_some(),
        "successful create must include an id; got: {v}"
    );
}
