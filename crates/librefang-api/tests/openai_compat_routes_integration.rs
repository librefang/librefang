//! Integration tests for the OpenAI-compatible `/v1/*` surface.
//!
//! Scope (intentional, partial slice of #3571):
//! - GET  /v1/models
//! - POST /v1/chat/completions   — validation paths only
//!
//! What this file covers:
//! - Auth gating: with `api_key` configured, both routes return 401 without a
//!   Bearer token. /v1/* is NOT in any PUBLIC_ROUTES_* allowlist.
//! - /v1/models response shape (object="list", data=[]) when no agents exist.
//! - /v1/chat/completions validation:
//!     * unknown model               -> 404 + OpenAI-style error envelope
//!     * empty messages array        -> 400 (no user message found)
//!     * messages with no user role  -> 400
//!     * malformed JSON              -> 400 (axum Json extractor)
//!     * missing required `model`    -> 422 (axum Json extractor)
//!
//! What this file deliberately does NOT cover:
//! - Happy-path completion (would require a real LLM provider key + network).
//! - Streaming SSE happy-path (same reason).
//!
//! These are tracked as follow-ups under #3571.
//!
//! Run: cargo test -p librefang-api --test openai_compat_routes_integration

use axum::body::Body;
use axum::http::{header, Method, Request, StatusCode};
use http_body_util::BodyExt;
use librefang_api::server;
use librefang_kernel::LibreFangKernel;
use librefang_types::config::{DefaultModelConfig, KernelConfig};
use std::sync::Arc;
use tower::ServiceExt;

struct RouterHarness {
    app: axum::Router,
    _tmp: tempfile::TempDir,
    state: Arc<librefang_api::routes::AppState>,
}

impl Drop for RouterHarness {
    fn drop(&mut self) {
        self.state.kernel.shutdown();
    }
}

async fn boot(api_key: &str) -> RouterHarness {
    let tmp = tempfile::tempdir().expect("tempdir");
    librefang_runtime::registry_sync::sync_registry(
        tmp.path(),
        librefang_runtime::registry_sync::DEFAULT_CACHE_TTL_SECS,
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
        state,
    }
}

async fn read_json(body: axum::body::Body) -> serde_json::Value {
    let bytes = body.collect().await.expect("body").to_bytes();
    serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null)
}

fn get(path: &str, bearer: Option<&str>) -> Request<Body> {
    let mut b = Request::builder().method(Method::GET).uri(path);
    if let Some(t) = bearer {
        b = b.header(header::AUTHORIZATION, format!("Bearer {t}"));
    }
    b.body(Body::empty()).unwrap()
}

fn post_json(path: &str, body: &str, bearer: Option<&str>) -> Request<Body> {
    let mut b = Request::builder()
        .method(Method::POST)
        .uri(path)
        .header(header::CONTENT_TYPE, "application/json");
    if let Some(t) = bearer {
        b = b.header(header::AUTHORIZATION, format!("Bearer {t}"));
    }
    b.body(Body::from(body.to_string())).unwrap()
}

// ── Auth gate ───────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread")]
async fn v1_models_requires_bearer_when_api_key_configured() {
    let h = boot("test-secret-key").await;
    let resp = h
        .app
        .clone()
        .oneshot(get("/v1/models", None))
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::UNAUTHORIZED,
        "/v1/models must 401 without Bearer when api_key is set"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn v1_chat_completions_requires_bearer_when_api_key_configured() {
    let h = boot("test-secret-key").await;
    let body = r#"{"model":"librefang:any","messages":[{"role":"user","content":"hi"}]}"#;
    let resp = h
        .app
        .clone()
        .oneshot(post_json("/v1/chat/completions", body, None))
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::UNAUTHORIZED,
        "/v1/chat/completions must 401 without Bearer when api_key is set"
    );
}

// ── /v1/models shape ────────────────────────────────────────────────────────

const KEY: &str = "test-secret-key";

#[tokio::test(flavor = "multi_thread")]
async fn v1_models_returns_openai_list_shape() {
    // Bear in mind: tower::oneshot has no ConnectInfo extension, so the auth
    // middleware does NOT treat the call as loopback. We must supply the
    // configured api_key as a Bearer token for any non-public route.
    let h = boot(KEY).await;
    let resp = h
        .app
        .clone()
        .oneshot(get("/v1/models", Some(KEY)))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = read_json(resp.into_body()).await;
    assert_eq!(json["object"], "list", "object field must be 'list'");
    assert!(
        json["data"].is_array(),
        "data field must be an array, got: {json}"
    );
}

// ── /v1/chat/completions validation ─────────────────────────────────────────

#[tokio::test(flavor = "multi_thread")]
async fn v1_chat_completions_unknown_model_returns_404_envelope() {
    let h = boot(KEY).await;
    let body =
        r#"{"model":"librefang:does-not-exist","messages":[{"role":"user","content":"hi"}]}"#;
    let resp = h
        .app
        .clone()
        .oneshot(post_json("/v1/chat/completions", body, Some(KEY)))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    let json = read_json(resp.into_body()).await;
    assert_eq!(json["error"]["type"], "invalid_request_error");
    assert_eq!(json["error"]["code"], "model_not_found");
    assert!(
        json["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("does-not-exist"),
        "error message should reference the unknown model id, got: {json}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn v1_chat_completions_empty_messages_returns_400() {
    let h = boot(KEY).await;
    // model resolution falls through to default agent when registry has any
    // entry; with an empty registry resolve_agent returns None -> 404 instead
    // of 400. Pre-create an agent so we exercise the "no user message" branch.
    let agent_name = create_agent(&h).await;
    let body = format!(r#"{{"model":"librefang:{agent_name}","messages":[]}}"#);
    let resp = h
        .app
        .clone()
        .oneshot(post_json("/v1/chat/completions", &body, Some(KEY)))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let json = read_json(resp.into_body()).await;
    assert_eq!(json["error"]["code"], "missing_message");
}

#[tokio::test(flavor = "multi_thread")]
async fn v1_chat_completions_no_user_role_returns_400() {
    let h = boot(KEY).await;
    let agent_name = create_agent(&h).await;
    // Only an assistant message — handler scans for last user msg, finds none.
    let body = format!(
        r#"{{"model":"librefang:{agent_name}","messages":[{{"role":"assistant","content":"hello"}}]}}"#
    );
    let resp = h
        .app
        .clone()
        .oneshot(post_json("/v1/chat/completions", &body, Some(KEY)))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let json = read_json(resp.into_body()).await;
    assert_eq!(json["error"]["code"], "missing_message");
}

#[tokio::test(flavor = "multi_thread")]
async fn v1_chat_completions_malformed_json_returns_400() {
    let h = boot(KEY).await;
    let resp = h
        .app
        .clone()
        .oneshot(post_json("/v1/chat/completions", "{not json", Some(KEY)))
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::BAD_REQUEST,
        "malformed JSON should be rejected by the Json extractor"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn v1_chat_completions_missing_model_field_is_rejected() {
    let h = boot(KEY).await;
    // `model` is a required field on ChatCompletionRequest with no default.
    // axum's Json extractor returns 422 Unprocessable Entity for serde-failed
    // payloads; we accept any 4xx to avoid coupling to extractor internals.
    let body = r#"{"messages":[{"role":"user","content":"hi"}]}"#;
    let resp = h
        .app
        .clone()
        .oneshot(post_json("/v1/chat/completions", body, Some(KEY)))
        .await
        .unwrap();
    let s = resp.status();
    assert!(
        s.is_client_error(),
        "missing required 'model' field should produce a 4xx, got {s}"
    );
}

// ── Helpers ─────────────────────────────────────────────────────────────────

/// Spawn a minimal agent so model-resolution can succeed in handler tests
/// where the unknown-model 404 branch would otherwise mask the 400 we want
/// to assert. Returns the agent's `name` (used as `librefang:<name>` in the
/// OpenAI `model` field).
async fn create_agent(h: &RouterHarness) -> String {
    use librefang_types::agent::{AgentEntry, AgentId};

    let name = format!("test-agent-{}", uuid::Uuid::new_v4().simple());
    let entry = AgentEntry {
        id: AgentId::new(),
        name: name.clone(),
        ..Default::default()
    };
    h.state
        .kernel
        .agent_registry()
        .register(entry)
        .expect("register agent");
    name
}
