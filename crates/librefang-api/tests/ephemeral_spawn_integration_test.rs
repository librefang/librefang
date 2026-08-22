//! Integration tests for `POST /api/agents/spawn-ephemeral` (#6930).
//!
//! These tests exercise the production router (`server::build_router`) with `tower::ServiceExt::oneshot`, so the real auth middleware, route registration, and handler logic are all in play — the pattern established by `agents_routes_integration.rs`.
//! No real LLM calls are made: every case here is decided before `resolve_driver` is ever reached (auth, JSON validation, `parent_agent_id` validation, or the pre-flight cost-quota gate), so the tests stay hermetic without needing a stub LLM driver.
//! A happy-path assertion on a completed run's response body is deliberately out of scope here: injecting a stub `LlmDriver` requires mutating `LibreFangKernel::llm.default_driver`, which is `pub(crate)` to `librefang-kernel` and not reachable from this crate's tests — that path is covered instead by the kernel-crate-local unit tests in `crates/librefang-kernel/src/kernel/messaging.rs` (`ephemeral_spawn_is_denied_when_parent_is_over_its_cost_quota`'s uncapped-parent control case).
//!
//! Routes covered:
//!   POST /api/agents/spawn-ephemeral (auth gate, malformed body, parent_agent_id validation, cost-quota gate)
//!
//! Run: cargo test -p librefang-api --test ephemeral_spawn_integration_test

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use librefang_api::routes::AppState;
use librefang_api::server;
use librefang_kernel::LibreFangKernel;
use librefang_types::agent::{AgentId, AgentManifest, ResourceQuota};
use librefang_types::config::{DefaultModelConfig, KernelConfig};
use std::sync::Arc;
use tower::ServiceExt;

// ---------------------------------------------------------------------------
// Harness — boots the production router with a configurable api_key.
// ---------------------------------------------------------------------------

struct Harness {
    app: axum::Router,
    state: Arc<AppState>,
    _tmp: tempfile::TempDir,
}

impl Drop for Harness {
    fn drop(&mut self) {
        self.state.kernel.shutdown();
    }
}

const TEST_TOKEN: &str = "test-secret";

async fn boot() -> Harness {
    let tmp = tempfile::tempdir().expect("tempdir");

    librefang_kernel::registry_sync::seed_registry_fixture_for_tests(tmp.path());

    let config = KernelConfig {
        home_dir: tmp.path().to_path_buf(),
        data_dir: tmp.path().join("data"),
        api_key: TEST_TOKEN.to_string(),
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

    let (app, state) = server::build_router(kernel, "127.0.0.1:0".parse().expect("addr")).await;

    Harness {
        app,
        state,
        _tmp: tmp,
    }
}

/// Registers an agent with the given resource quota through the same `KernelApi::spawn_agent_typed` entry point the production router uses, so tests never reach into kernel internals directly.
fn register_agent_with_quota(
    state: &Arc<AppState>,
    name: &str,
    resources: ResourceQuota,
) -> AgentId {
    let manifest = AgentManifest {
        name: name.to_string(),
        resources,
        ..AgentManifest::default()
    };
    state
        .kernel
        .spawn_agent_typed(manifest)
        .expect("spawn_agent_typed")
}

/// Records prior spend against `agent_id` through the same metering surface `spawn_ephemeral_worker`'s own post-run accounting uses, so a test can put a parent "already over its cap" before ever calling the route.
fn record_prior_spend(state: &Arc<AppState>, agent_id: AgentId, cost_usd: f64) {
    state
        .kernel
        .metering_ref()
        .record(&librefang_memory::usage::UsageRecord {
            agent_id,
            provider: "openai".to_string(),
            model: "gpt-4o".to_string(),
            input_tokens: 1_000,
            output_tokens: 1_000,
            cost_usd,
            tool_calls: 0,
            latency_ms: 1,
            user_id: None,
            channel: None,
            session_id: None,
        })
        .expect("record prior spend");
}

async fn send(app: axum::Router, req: Request<Body>) -> (StatusCode, serde_json::Value) {
    let resp = app.oneshot(req).await.expect("oneshot");
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .expect("body");
    let json = if bytes.is_empty() {
        serde_json::Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null)
    };
    (status, json)
}

fn spawn_ephemeral_request(body: serde_json::Value, bearer: Option<&str>) -> Request<Body> {
    let mut b = Request::builder()
        .method(Method::POST)
        .uri("/api/agents/spawn-ephemeral")
        .header("content-type", "application/json");
    if let Some(token) = bearer {
        b = b.header("authorization", format!("Bearer {}", token));
    }
    b.body(Body::from(body.to_string())).unwrap()
}

fn raw_body_request(body: &str, bearer: Option<&str>) -> Request<Body> {
    let mut b = Request::builder()
        .method(Method::POST)
        .uri("/api/agents/spawn-ephemeral")
        .header("content-type", "application/json");
    if let Some(token) = bearer {
        b = b.header("authorization", format!("Bearer {}", token));
    }
    b.body(Body::from(body.to_string())).unwrap()
}

// ---------------------------------------------------------------------------
// Auth gate + basic wiring
// ---------------------------------------------------------------------------

/// Proves the route is actually registered in the production router (`server.rs`'s `api_v1_routes()`), not just present in the `routes/agents` module: without registration this would 404, not 401.
#[tokio::test(flavor = "multi_thread")]
async fn spawn_ephemeral_requires_auth() {
    let h = boot().await;

    let (status, _) = send(
        h.app.clone(),
        spawn_ephemeral_request(serde_json::json!({ "message": "hi" }), None),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "missing bearer must be rejected before reaching the handler"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn spawn_ephemeral_rejects_malformed_json() {
    let h = boot().await;

    let (status, _) = send(
        h.app.clone(),
        raw_body_request("{not valid json", Some(TEST_TOKEN)),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test(flavor = "multi_thread")]
async fn spawn_ephemeral_rejects_missing_message_field() {
    let h = boot().await;

    // `message` has no `#[serde(default)]` — an absent field must fail JSON
    // extraction rather than silently defaulting to an empty task. axum's
    // `Json` extractor answers a missing-required-field payload with 422
    // (distinct from the malformed-syntax case above, which is 400) — accept
    // any 4xx rather than coupling to that extractor-internal distinction.
    let (status, _) = send(
        h.app.clone(),
        spawn_ephemeral_request(
            serde_json::json!({ "system_prompt": "be brief" }),
            Some(TEST_TOKEN),
        ),
    )
    .await;
    assert!(
        status.is_client_error(),
        "missing required 'message' field should produce a 4xx, got {status}"
    );
}

// ---------------------------------------------------------------------------
// parent_agent_id validation (#6930 review: budget-attribution was
// previously bypassed outright by hardcoding None for every HTTP caller)
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn spawn_ephemeral_rejects_malformed_parent_agent_id() {
    let h = boot().await;

    let (status, body) = send(
        h.app.clone(),
        spawn_ephemeral_request(
            serde_json::json!({ "message": "hi", "parent_agent_id": "not-a-uuid" }),
            Some(TEST_TOKEN),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["code"], "invalid_parent_agent_id");
}

#[tokio::test(flavor = "multi_thread")]
async fn spawn_ephemeral_rejects_unregistered_parent_agent_id() {
    let h = boot().await;
    let unregistered = AgentId::new();

    let (status, body) = send(
        h.app.clone(),
        spawn_ephemeral_request(
            serde_json::json!({ "message": "hi", "parent_agent_id": unregistered.to_string() }),
            Some(TEST_TOKEN),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["code"], "parent_agent_not_found");
}

// ---------------------------------------------------------------------------
// Cost-quota gate, end-to-end through the real HTTP route (#6930)
// ---------------------------------------------------------------------------

/// The single highest-value assertion in this file: a parent already over its own configured cost cap must be denied with 429 before any LLM call is attempted — proving the fix in `spawn_ephemeral_worker` (billed agent's own quota gates the run) is actually wired end-to-end through the real router, auth middleware and `parent_agent_id` plumbing, not just true at the kernel-unit level.
#[tokio::test(flavor = "multi_thread")]
async fn spawn_ephemeral_denies_when_parent_is_over_cost_quota() {
    let h = boot().await;

    let capped = register_agent_with_quota(
        &h.state,
        "capped-parent",
        ResourceQuota {
            max_cost_per_day_usd: 5.0,
            ..Default::default()
        },
    );
    record_prior_spend(&h.state, capped, 6.0);

    let (status, body) = send(
        h.app.clone(),
        spawn_ephemeral_request(
            serde_json::json!({ "message": "hi", "parent_agent_id": capped.to_string() }),
            Some(TEST_TOKEN),
        ),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::TOO_MANY_REQUESTS,
        "an over-quota parent must be denied with 429, got body: {body}"
    );
    assert_eq!(body["code"], "quota_exceeded");
}

/// Control for the test above: an uncapped parent with the *same* prior spend must not be blocked by the quota gate — the 429 above is a real decision, not a blanket refusal once any spend exists.
/// The request still cannot complete in this hermetic test (no LLM driver is configured), so this asserts the negative — it is specifically NOT the quota-exceeded response — rather than a 200.
#[tokio::test(flavor = "multi_thread")]
async fn spawn_ephemeral_does_not_quota_block_an_uncapped_parent() {
    let h = boot().await;

    let uncapped = register_agent_with_quota(&h.state, "uncapped-parent", ResourceQuota::default());
    record_prior_spend(&h.state, uncapped, 6.0);

    let (status, body) = send(
        h.app.clone(),
        spawn_ephemeral_request(
            serde_json::json!({ "message": "hi", "parent_agent_id": uncapped.to_string() }),
            Some(TEST_TOKEN),
        ),
    )
    .await;
    assert_ne!(
        status,
        StatusCode::TOO_MANY_REQUESTS,
        "an uncapped parent must not be quota-blocked, got body: {body}"
    );
    assert_ne!(body["code"], "quota_exceeded");
}
