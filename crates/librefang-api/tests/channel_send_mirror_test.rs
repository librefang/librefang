//! Integration tests for the channel_send mirror path (#4824).
//!
//! Scope:
//!   1. Route wiring: `POST /api/tools/channel_send/invoke` is reachable,
//!      passes the allowlist gate, and returns a well-formed response even
//!      when no channel adapter is configured.
//!   2. Lock-key / session-write correctness: `kernel.append_to_session` uses
//!      `agent_msg_locks` (same key space as `send_message_full`'s no-override
//!      path) and can be called from an async context without panicking.  The
//!      written message is readable back through the substrate.
//!
//! Run: cargo test -p librefang-api --test channel_send_mirror_test

use axum::body::Body;
use axum::http::{Request, StatusCode};
use librefang_api::{routes::AppState, server};
use librefang_kernel::{KernelApi, LibreFangKernel};
use librefang_types::agent::SessionId;
use librefang_types::config::{DefaultModelConfig, KernelConfig, ToolInvokeConfig};
use librefang_types::message::{Message, MessageContent, Role};
use std::sync::Arc;
use tower::ServiceExt;

// ---------------------------------------------------------------------------
// Harness
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

async fn boot_with_tool_invoke(tool_invoke: ToolInvokeConfig) -> Harness {
    let tmp = tempfile::tempdir().expect("tempdir");

    librefang_kernel::registry_sync::sync_registry(
        tmp.path(),
        librefang_kernel::registry_sync::DEFAULT_CACHE_TTL_SECS,
        "",
    );

    let config = KernelConfig {
        home_dir: tmp.path().to_path_buf(),
        data_dir: tmp.path().join("data"),
        default_model: DefaultModelConfig {
            provider: "ollama".to_string(),
            model: "test-model".to_string(),
            api_key_env: "OLLAMA_API_KEY".to_string(),
            base_url: None,
            message_timeout_secs: 300,
            extra_params: std::collections::HashMap::new(),
            cli_profile_dirs: Vec::new(),
        },
        tool_invoke,
        ..KernelConfig::default()
    };

    let kernel = LibreFangKernel::boot_with_config(config).expect("kernel boot");
    let kernel = Arc::new(kernel);
    // `set_self_handle` on the `KernelApi` trait takes `self: Arc<Self>` by
    // value (post-#3565 cleanup) — clone first so we keep `kernel` for
    // `build_router` below.
    Arc::clone(&kernel).set_self_handle();

    // `build_router` now takes `Arc<dyn KernelApi>` (post-#3565 cleanup).
    // Coerce the concrete `Arc<LibreFangKernel>` to the trait object before
    // calling.
    let kernel_dyn: Arc<dyn KernelApi> = kernel.clone();
    let (app, state) = server::build_router(kernel_dyn, "127.0.0.1:0".parse().expect("addr")).await;

    Harness {
        app,
        state,
        _tmp: tmp,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// Route wiring: `POST /api/tools/channel_send/invoke` returns 200 with
/// `is_error: true` when no channel adapter is configured.  Verifies:
///   - `channel_send` is registered as a builtin tool (existence check)
///   - the `tool_invoke` allowlist gate is applied correctly
///   - the handler returns a valid JSON body on adapter-not-found failure
#[tokio::test(flavor = "multi_thread")]
async fn test_channel_send_invoke_route_wired_and_gated() {
    let h = boot_with_tool_invoke(ToolInvokeConfig {
        enabled: true,
        allowlist: vec!["channel_send".into()],
    })
    .await;

    let body = serde_json::json!({
        "channel": "telegram",
        "recipient": "12345",
        "message": "hello from integration test",
    });

    let resp = h
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/tools/channel_send/invoke")
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .expect("oneshot");

    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "channel_send route must be reachable"
    );

    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .expect("body bytes");
    let json: serde_json::Value = serde_json::from_slice(&bytes).expect("response is JSON");

    // No adapter configured → tool execution error, not a server panic/500.
    assert_eq!(
        json["is_error"], true,
        "no adapter configured — expected is_error:true, got: {json}"
    );
}

/// Allowlist gate: `channel_send` is rejected with 403 when not allowlisted.
#[tokio::test(flavor = "multi_thread")]
async fn test_channel_send_invoke_forbidden_when_not_allowlisted() {
    let h = boot_with_tool_invoke(ToolInvokeConfig {
        enabled: true,
        allowlist: vec!["notify_owner".into()],
    })
    .await;

    let resp = h
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/tools/channel_send/invoke")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({"channel":"telegram","recipient":"1","message":"x"})
                        .to_string(),
                ))
                .unwrap(),
        )
        .await
        .expect("oneshot");

    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

/// Lock-key / async-safety: `kernel.append_to_session` must:
///   (a) not panic when called from an async context (pre-fix: `blocking_lock()`
///       panics in async; post-fix: `block_in_place` is safe),
///   (b) use `agent_msg_locks[agent_id]` so writes are serialized against the
///       inbound-routing path in `send_message_full` (same key space),
///   (c) persist the message so it is readable back via `memory_substrate`.
#[tokio::test(flavor = "multi_thread")]
async fn test_append_to_session_safe_from_async_and_persists_message() {
    let h = boot_with_tool_invoke(ToolInvokeConfig::default()).await;

    let agent_id = librefang_types::agent::AgentId(
        uuid::Uuid::parse_str("aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee").unwrap(),
    );
    let session_id = SessionId::for_sender_scope(agent_id, "telegram", Some("99999"));

    let mirror_text = "[mirror from test-agent]: hello from mirror".to_string();
    let msg = Message {
        role: Role::User,
        content: MessageContent::Text(mirror_text.clone()),
        pinned: false,
        timestamp: Some(chrono::Utc::now()),
    };

    // This call must NOT panic. Before the fix, `blocking_lock()` inside
    // `append_to_session` would panic when called from an async worker thread.
    h.state.kernel.append_to_session(session_id, agent_id, msg);

    // Verify persistence: the message must be readable back via the substrate.
    let session = h
        .state
        .kernel
        .memory_substrate()
        .get_session(session_id)
        .expect("get_session must not error")
        .expect("session must exist after append_to_session");

    assert_eq!(session.messages.len(), 1, "exactly one message in session");
    assert_eq!(session.messages[0].role, Role::User, "role must be User");
    assert_eq!(
        session.messages[0].content.text_content(),
        mirror_text,
        "mirror text must match [mirror from <agent>]: <body> format"
    );
}
