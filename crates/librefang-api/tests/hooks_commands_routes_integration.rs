//! Integration tests for the `/api/hooks/*` (webhook trigger) and
//! `/api/commands{,/:name}` (chat-command catalog) endpoints in
//! `routes::system`.
//!
//! Issue #3571 — refs: ~80% of registered HTTP routes have no integration
//! test. This file owns the **hooks/commands slice** of `system.rs` only
//! and intentionally does **not** touch the wider system.rs surface.
//!
//! Strategy:
//! - Mount `routes::system::router()` under `/api` against a kernel built
//!   by `MockKernelBuilder` + `TestAppState`. We bypass the full server
//!   stack so the auth middleware is not in scope — the handler-level
//!   bearer-token check on `/api/hooks/*` (env-var sourced) is what gets
//!   exercised here.
//! - For `/api/hooks/agent` we cover validation + auth-gate + 404 paths
//!   only. The happy path requires a live LLM dispatch (`send_message`
//!   round-trip), which is out of scope for unit-suite-friendly tests.
//! - Each test that touches the bearer-token env var uses a unique env
//!   name so parallel test execution stays deterministic (env is process
//!   global).

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use axum::Router;
use librefang_api::routes::{self, AppState};
use librefang_testing::{MockKernelBuilder, TestAppState};
use librefang_types::config::WebhookTriggerConfig;
use std::sync::Arc;
use tower::ServiceExt;

struct Harness {
    app: Router,
    _state: Arc<AppState>,
    _test: TestAppState,
}

/// Boot a kernel + system router with optional `webhook_triggers` config.
async fn boot_with_webhook(webhook: Option<WebhookTriggerConfig>) -> Harness {
    let test = TestAppState::with_builder(MockKernelBuilder::new().with_config(move |cfg| {
        cfg.webhook_triggers = webhook.clone();
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

async fn boot_disabled() -> Harness {
    boot_with_webhook(None).await
}

async fn boot_enabled(token_env: &str) -> Harness {
    boot_with_webhook(Some(WebhookTriggerConfig {
        enabled: true,
        token_env: token_env.to_string(),
        max_payload_bytes: 65536,
        rate_limit_per_minute: 30,
    }))
    .await
}

async fn send(
    h: &Harness,
    method: Method,
    path: &str,
    body: Option<serde_json::Value>,
    bearer: Option<&str>,
) -> (StatusCode, serde_json::Value) {
    let mut builder = Request::builder().method(method).uri(path);
    if let Some(tok) = bearer {
        builder = builder.header("authorization", format!("Bearer {tok}"));
    }
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
    let value = if bytes.is_empty() {
        serde_json::Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null)
    };
    (status, value)
}

// ---------------------------------------------------------------------------
// /api/hooks/wake
// ---------------------------------------------------------------------------

/// When `webhook_triggers` is unset the handler must short-circuit with a
/// 404 *before* validating the bearer token. Confirms we don't accidentally
/// expose the validation error surface as a probe oracle.
#[tokio::test(flavor = "multi_thread")]
async fn hooks_wake_returns_404_when_webhook_triggers_not_enabled() {
    let h = boot_disabled().await;
    let (status, body) = send(
        &h,
        Method::POST,
        "/api/hooks/wake",
        Some(serde_json::json!({"text": "hi"})),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body:?}");
    assert!(body["error"].is_string(), "{body:?}");
}

/// `webhook_triggers.enabled = false` is the default-flagged case — also a 404.
#[tokio::test(flavor = "multi_thread")]
async fn hooks_wake_returns_404_when_webhook_triggers_disabled_explicitly() {
    let h = boot_with_webhook(Some(WebhookTriggerConfig {
        enabled: false,
        token_env: "LIBREFANG_WEBHOOK_TOKEN_DISABLED_3571".into(),
        max_payload_bytes: 65536,
        rate_limit_per_minute: 30,
    }))
    .await;
    let (status, _) = send(
        &h,
        Method::POST,
        "/api/hooks/wake",
        Some(serde_json::json!({"text": "hi"})),
        Some("anything"),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

/// Missing Authorization header against an enabled hook = 400.
#[tokio::test(flavor = "multi_thread")]
async fn hooks_wake_rejects_missing_bearer() {
    let env_name = "LIBREFANG_TEST_WEBHOOK_TOKEN_WAKE_MISSING_3571";
    // SAFETY: tests run with `--test-threads` but each test uses a unique
    // env-var name so concurrent reads/writes do not collide.
    unsafe {
        std::env::set_var(env_name, "x".repeat(40));
    }
    let h = boot_enabled(env_name).await;
    let (status, body) = send(
        &h,
        Method::POST,
        "/api/hooks/wake",
        Some(serde_json::json!({"text": "hi"})),
        None,
    )
    .await;
    unsafe {
        std::env::remove_var(env_name);
    }
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body:?}");
}

/// Wrong bearer token against an enabled hook = 400 (constant-time mismatch).
#[tokio::test(flavor = "multi_thread")]
async fn hooks_wake_rejects_wrong_bearer() {
    let env_name = "LIBREFANG_TEST_WEBHOOK_TOKEN_WAKE_WRONG_3571";
    let real = "a".repeat(40);
    unsafe {
        std::env::set_var(env_name, &real);
    }
    let h = boot_enabled(env_name).await;
    let (status, _) = send(
        &h,
        Method::POST,
        "/api/hooks/wake",
        Some(serde_json::json!({"text": "hi"})),
        Some(&"b".repeat(40)),
    )
    .await;
    unsafe {
        std::env::remove_var(env_name);
    }
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

/// A token shorter than 32 bytes is treated as "no token configured", so the
/// auth gate fails closed even when the caller sends a matching string.
#[tokio::test(flavor = "multi_thread")]
async fn hooks_wake_rejects_short_configured_token() {
    let env_name = "LIBREFANG_TEST_WEBHOOK_TOKEN_WAKE_SHORT_3571";
    unsafe {
        std::env::set_var(env_name, "tooshort"); // < 32 chars
    }
    let h = boot_enabled(env_name).await;
    let (status, _) = send(
        &h,
        Method::POST,
        "/api/hooks/wake",
        Some(serde_json::json!({"text": "hi"})),
        Some("tooshort"),
    )
    .await;
    unsafe {
        std::env::remove_var(env_name);
    }
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

/// Auth passes, payload validation fails — empty `text` is rejected with
/// 400 carrying the validator's message.
#[tokio::test(flavor = "multi_thread")]
async fn hooks_wake_rejects_empty_text_payload() {
    let env_name = "LIBREFANG_TEST_WEBHOOK_TOKEN_WAKE_EMPTY_3571";
    let token = "z".repeat(40);
    unsafe {
        std::env::set_var(env_name, &token);
    }
    let h = boot_enabled(env_name).await;
    let (status, body) = send(
        &h,
        Method::POST,
        "/api/hooks/wake",
        Some(serde_json::json!({"text": ""})),
        Some(&token),
    )
    .await;
    unsafe {
        std::env::remove_var(env_name);
    }
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body:?}");
    assert!(
        body["error"]
            .as_str()
            .unwrap_or("")
            .to_lowercase()
            .contains("empty"),
        "expected validator message about empty text: {body:?}"
    );
}

/// Auth passes, payload validation passes — handler publishes the wake event
/// through the kernel and returns 200 with `{status: "accepted", mode: ..}`.
/// This is the happy-path smoke for `/hooks/wake`.
#[tokio::test(flavor = "multi_thread")]
async fn hooks_wake_accepts_valid_payload() {
    let env_name = "LIBREFANG_TEST_WEBHOOK_TOKEN_WAKE_OK_3571";
    let token = "k".repeat(40);
    unsafe {
        std::env::set_var(env_name, &token);
    }
    let h = boot_enabled(env_name).await;
    let (status, body) = send(
        &h,
        Method::POST,
        "/api/hooks/wake",
        Some(serde_json::json!({"text": "hello world"})),
        Some(&token),
    )
    .await;
    unsafe {
        std::env::remove_var(env_name);
    }
    assert_eq!(status, StatusCode::OK, "{body:?}");
    assert_eq!(body["status"], "accepted");
    // Mode defaults to "now" via WakeMode::default + serde rename_all=snake_case.
    assert_eq!(body["mode"], "now");
}

// ---------------------------------------------------------------------------
// /api/hooks/agent
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn hooks_agent_returns_404_when_webhook_triggers_not_enabled() {
    let h = boot_disabled().await;
    let (status, _) = send(
        &h,
        Method::POST,
        "/api/hooks/agent",
        Some(serde_json::json!({"message": "hi"})),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test(flavor = "multi_thread")]
async fn hooks_agent_rejects_missing_bearer() {
    let env_name = "LIBREFANG_TEST_WEBHOOK_TOKEN_AGENT_MISSING_3571";
    unsafe {
        std::env::set_var(env_name, "y".repeat(40));
    }
    let h = boot_enabled(env_name).await;
    let (status, _) = send(
        &h,
        Method::POST,
        "/api/hooks/agent",
        Some(serde_json::json!({"message": "hi"})),
        None,
    )
    .await;
    unsafe {
        std::env::remove_var(env_name);
    }
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

/// Auth passes, payload validation fails — empty `message`.
#[tokio::test(flavor = "multi_thread")]
async fn hooks_agent_rejects_empty_message() {
    let env_name = "LIBREFANG_TEST_WEBHOOK_TOKEN_AGENT_EMPTY_3571";
    let token = "m".repeat(40);
    unsafe {
        std::env::set_var(env_name, &token);
    }
    let h = boot_enabled(env_name).await;
    let (status, body) = send(
        &h,
        Method::POST,
        "/api/hooks/agent",
        Some(serde_json::json!({"message": ""})),
        Some(&token),
    )
    .await;
    unsafe {
        std::env::remove_var(env_name);
    }
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body:?}");
    assert!(body["error"]
        .as_str()
        .unwrap_or("")
        .to_lowercase()
        .contains("empty"));
}

/// Auth passes, payload validation fails — `timeout_secs` out of bounds
/// (max 600). Pins the validator's range check at the API boundary.
#[tokio::test(flavor = "multi_thread")]
async fn hooks_agent_rejects_oversize_timeout() {
    let env_name = "LIBREFANG_TEST_WEBHOOK_TOKEN_AGENT_TIMEOUT_3571";
    let token = "t".repeat(40);
    unsafe {
        std::env::set_var(env_name, &token);
    }
    let h = boot_enabled(env_name).await;
    let (status, body) = send(
        &h,
        Method::POST,
        "/api/hooks/agent",
        Some(serde_json::json!({"message": "hi", "timeout_secs": 9999})),
        Some(&token),
    )
    .await;
    unsafe {
        std::env::remove_var(env_name);
    }
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body:?}");
    assert!(
        body["error"]
            .as_str()
            .unwrap_or("")
            .contains("timeout_secs"),
        "{body:?}"
    );
}

/// Auth + payload pass and the default mock agent dispatches the message.
/// Proves the agent-resolve branch picks the only available agent when the
/// caller doesn't name one, and that the handler returns the dispatch result
/// envelope (`status`, `agent_id`, `response`).
#[tokio::test(flavor = "multi_thread")]
async fn hooks_agent_dispatches_to_default_agent() {
    let env_name = "LIBREFANG_TEST_WEBHOOK_TOKEN_AGENT_DEFAULT_3571";
    let token = "n".repeat(40);
    unsafe {
        std::env::set_var(env_name, &token);
    }
    let h = boot_enabled(env_name).await;
    let (status, body) = send(
        &h,
        Method::POST,
        "/api/hooks/agent",
        Some(serde_json::json!({"message": "hello"})),
        Some(&token),
    )
    .await;
    unsafe {
        std::env::remove_var(env_name);
    }
    assert_eq!(status, StatusCode::OK, "{body:?}");
    assert_eq!(body["status"], "completed", "{body:?}");
    assert!(body["agent_id"].is_string(), "{body:?}");
    assert!(body["response"].is_string(), "{body:?}");
}

/// When the caller names an agent that does not exist (and isn't a UUID),
/// handler returns 404 with a translated "not found" message.
#[tokio::test(flavor = "multi_thread")]
async fn hooks_agent_404_when_named_agent_missing() {
    let env_name = "LIBREFANG_TEST_WEBHOOK_TOKEN_AGENT_NAMED_3571";
    let token = "u".repeat(40);
    unsafe {
        std::env::set_var(env_name, &token);
    }
    let h = boot_enabled(env_name).await;
    let (status, body) = send(
        &h,
        Method::POST,
        "/api/hooks/agent",
        Some(serde_json::json!({"message": "hello", "agent": "ghost-agent"})),
        Some(&token),
    )
    .await;
    unsafe {
        std::env::remove_var(env_name);
    }
    assert_eq!(status, StatusCode::NOT_FOUND, "{body:?}");
}

// ---------------------------------------------------------------------------
// /api/commands and /api/commands/:name
// ---------------------------------------------------------------------------

/// `/api/commands` lists the built-in chat commands. The set is hard-coded
/// in the handler; we pin a few stable entries so refactors that drop a
/// command surface here.
#[tokio::test(flavor = "multi_thread")]
async fn commands_lists_builtins() {
    let h = boot_disabled().await;
    let (status, body) = send(&h, Method::GET, "/api/commands", None, None).await;
    assert_eq!(status, StatusCode::OK, "{body:?}");
    let arr = body["commands"].as_array().expect("commands array");
    let names: Vec<&str> = arr.iter().filter_map(|v| v["cmd"].as_str()).collect();
    for must_have in ["/help", "/new", "/reset", "/model", "/status"] {
        assert!(
            names.contains(&must_have),
            "expected `{must_have}` in commands list: {names:?}"
        );
    }
    // Each entry has both `cmd` and `desc` fields.
    for v in arr {
        assert!(v["cmd"].is_string(), "missing cmd: {v:?}");
        assert!(v["desc"].is_string(), "missing desc: {v:?}");
    }
}

/// `/api/commands/{name}` accepts the slash-prefixed and bare forms — both
/// must round-trip to the same record. Pins the leading-slash normalisation.
#[tokio::test(flavor = "multi_thread")]
async fn commands_lookup_normalises_leading_slash() {
    let h = boot_disabled().await;
    let (s_with, b_with) = send(&h, Method::GET, "/api/commands/%2Fhelp", None, None).await;
    let (s_without, b_without) = send(&h, Method::GET, "/api/commands/help", None, None).await;
    assert_eq!(s_with, StatusCode::OK, "{b_with:?}");
    assert_eq!(s_without, StatusCode::OK, "{b_without:?}");
    // Both responses describe the same command — desc string equality is the
    // tightest invariant available without coupling to the literal copy.
    assert_eq!(b_with["desc"], b_without["desc"]);
    assert!(b_without["cmd"].as_str().unwrap_or("").contains("help"));
}

/// Looking up an unknown command returns 404 with a translated error.
#[tokio::test(flavor = "multi_thread")]
async fn commands_lookup_unknown_returns_404() {
    let h = boot_disabled().await;
    let (status, body) = send(
        &h,
        Method::GET,
        "/api/commands/this-command-does-not-exist",
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body:?}");
    assert!(body["error"].is_string(), "{body:?}");
}
