//! #8409 — a dashboard chat turn from an authenticated `[[users]]` account is
//! attributed to that user, so their tool calls run under their own RBAC
//! policy instead of the guest gate.
//!
//! The dashboard chat rides `/api/agents/{id}/ws`. Its handler used to stamp
//! the turn's `SenderContext` with the raw client IP, and the kernel tool gate
//! resolves senders through `[[users]] channel_bindings` — a pair no dashboard
//! user ever has — so every RBAC-active deployment guest-gated the whole
//! dashboard chat: the first tool outside the guest read-only allowlist was
//! forced into the approval queue, and approving never cached anything
//! (force-approval skips the per-session cache by design).
//!
//! The assertions run over a real WebSocket turn against a wiremock Ollama
//! stub whose first response asks for a `memory_store` tool call — a tool the
//! guest gate refuses — so `GET /api/approvals/count` is a faithful readout of
//! whether the turn ran the per-user gate or the guest gate.

use axum::Router;
use librefang_api::middleware;
use librefang_api::routes::{self, AppState};
use librefang_testing::{MockKernelBuilder, TestAppState};
use librefang_types::config::UserConfig;
use std::sync::Arc;
use tower_http::cors::CorsLayer;

const ROOT_KEY: &str = "root-test-key";
const USER_KEY: &str = "chatuser-key";

/// The manifest the harness spawns: `memory_store` is offered (and not on the
/// global `[approval] require_approval` list), so whether the call prompts
/// depends entirely on the per-user gate.
const CHAT_AGENT_MANIFEST: &str = r#"
name = "test-agent"
version = "0.1.0"
description = "Integration test agent"
author = "chatuser"
module = "builtin:chat"

[model]
provider = "ollama"
model = "test-model"
system_prompt = "You are a test agent. Reply concisely."

[capabilities]
tools = ["memory_store", "file_read"]
memory_read = ["*"]
memory_write = ["self.*"]
"#;

struct Harness {
    base_url: String,
    llm: wiremock::MockServer,
    state: Arc<AppState>,
    _tmp: tempfile::TempDir,
}

impl Drop for Harness {
    fn drop(&mut self) {
        self.state.kernel.shutdown();
    }
}

/// Model catalog carrying a keyless `ollama` provider at `base_url`.
///
/// Without it the turn's provider-auth gate short-circuits before any sender
/// resolution happens, and the test would pass on a broken build for the wrong
/// reason.
fn ollama_stub_catalog(base_url: &str) -> librefang_testing::CatalogSeed {
    use librefang_types::model_catalog::{AuthStatus, ProviderInfo};

    let (mut providers, mut models) = librefang_testing::test_catalog_baseline();
    providers.push(ProviderInfo {
        id: "ollama".to_string(),
        display_name: "Ollama (wiremock stub)".to_string(),
        api_key_env: "OLLAMA_API_KEY".to_string(),
        base_url: base_url.to_string(),
        key_required: false,
        auth_status: AuthStatus::NotRequired,
        model_count: 1,
        ..ProviderInfo::default()
    });
    let mut entry = models[0].clone();
    entry.id = "test-model".to_string();
    entry.display_name = "Ollama test model".to_string();
    entry.provider = "ollama".to_string();
    models.push(entry);
    (providers, models)
}

/// Boot the agents + approvals routers behind the real auth middleware, with
/// one `owner`-role user wired into both `KernelConfig.users` (so the
/// `AuthManager` snapshot knows them) and `AuthState.user_api_keys` (so the
/// WebSocket upgrade admits their bearer token and populates
/// `AuthenticatedApiUser`).
async fn start_harness() -> Harness {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let llm = MockServer::start().await;

    // The Ollama driver's streaming parser splits the body on '\n' and never
    // parses a final unterminated line, so every stub body MUST end with a
    // newline — a `set_body_json` body (no trailing newline) parses as an
    // empty response and spins the agent loop to max iterations.
    let ndjson = |v: serde_json::Value| {
        ResponseTemplate::new(200)
            .set_body_string(format!("{}\n", serde_json::to_string(&v).unwrap()))
    };

    // Turn 1 of the stub: the model asks to store a memory. `done_reason:
    // "tool_calls"` is what the Ollama driver maps to `StopReason::ToolUse`.
    Mock::given(method("POST"))
        .and(path("/api/chat"))
        .respond_with(ndjson(serde_json::json!({
            "model": "test-model",
            "message": {
                "role": "assistant",
                "content": "",
                "tool_calls": [{
                    "function": {
                        "name": "memory_store",
                        "arguments": {"key": "note", "value": "remembered from the dashboard"}
                    }
                }]
            },
            "done": true,
            "done_reason": "tool_calls",
            "prompt_eval_count": 7,
            "eval_count": 2,
        })))
        .up_to_n_times(1)
        .with_priority(1)
        .mount(&llm)
        .await;

    // Turn 2: after the tool result, the model answers and the loop ends.
    Mock::given(method("POST"))
        .and(path("/api/chat"))
        .respond_with(ndjson(serde_json::json!({
            "model": "test-model",
            "message": { "role": "assistant", "content": "stored." },
            "done": true,
            "done_reason": "stop",
            "prompt_eval_count": 7,
            "eval_count": 2,
        })))
        .with_priority(2)
        .mount(&llm)
        .await;

    let hash = librefang_api::password_hash::hash_password(USER_KEY)
        .expect("password hash should succeed");
    let user_configs = vec![UserConfig {
        name: "chatuser".to_string(),
        role: "owner".to_string(),
        channel_bindings: std::collections::HashMap::new(),
        api_key_hash: Some(hash.clone()),
        ..Default::default()
    }];
    let api_user_records = vec![middleware::ApiUserAuth {
        name: "chatuser".to_string(),
        role: librefang_kernel::auth::UserRole::from_str_role("owner"),
        api_key_hash: hash,
        user_id: librefang_types::agent::UserId::from_name("chatuser"),
    }];

    let uri = llm.uri();
    let config_uri = uri.clone();
    let test = TestAppState::with_builder(
        MockKernelBuilder::new()
            .with_config(move |cfg| {
                cfg.api_key = ROOT_KEY.to_string();
                cfg.users = user_configs;
                cfg.default_model.provider = "ollama".to_string();
                cfg.default_model.model = "test-model".to_string();
                cfg.default_model.api_key_env = "OLLAMA_API_KEY".to_string();
                cfg.default_model.base_url = Some(config_uri);
                // Keep the turn to the two scripted provider round trips —
                // proactive memory would add retrieval and extraction calls
                // that muddy both the stub sequence and the pending-approval
                // assertion.
                cfg.proactive_memory.enabled = false;
            })
            .with_catalog_seed(ollama_stub_catalog(&uri)),
    )
    .with_api_key(ROOT_KEY)
    .with_user_api_keys(api_user_records);

    let config_path = test.tmp_path().join("config.toml");
    let test = test.with_config_path(config_path);
    let (state, tmp, _) = test.into_parts();

    let auth_state = middleware::AuthState {
        api_key_lock: state.api_key_lock.clone(),
        master_key: state.master_key.clone(),
        active_sessions: state.active_sessions.clone(),
        dashboard_auth_enabled: state.dashboard_auth_enabled.clone(),
        user_api_keys: state.user_api_keys.clone(),
        require_auth_for_reads: false,
        allow_no_auth: false,
        audit_log: Some(state.kernel.audit().clone()),
    };

    let app = Router::new()
        .nest("/api", routes::agents::router())
        .layer(axum::middleware::from_fn_with_state(
            auth_state,
            middleware::auth,
        ))
        .layer(CorsLayer::permissive())
        .with_state(state.clone());

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test server");
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .await
        .unwrap();
    });

    Harness {
        base_url: format!("http://{}", addr),
        llm,
        state,
        _tmp: tmp,
    }
}

/// Spawn the chat agent as its author via the root (Owner) key.
async fn spawn_agent(h: &Harness) -> String {
    let resp = reqwest::Client::new()
        .post(format!("{}/api/agents", h.base_url))
        .bearer_auth(ROOT_KEY)
        .json(&serde_json::json!({ "manifest_toml": CHAT_AGENT_MANIFEST }))
        .send()
        .await
        .expect("spawn request");
    assert_eq!(resp.status().as_u16(), 201, "spawn must succeed");
    let body: serde_json::Value = resp.json().await.expect("spawn body");
    body["agent_id"]
        .as_str()
        .expect("agent_id in spawn body")
        .to_string()
}

/// The pending-approval count, read straight off the kernel the turn ran on.
fn pending_approvals(h: &Harness) -> usize {
    h.state.kernel.approvals().pending_count()
}

/// An authenticated dashboard chat turn runs the caller's own tool policy.
///
/// `chatuser` is a registered `owner` with no per-user policy of their own, so
/// the Layer B walk allows every tool: the `memory_store` call the model asked
/// for must EXECUTE, leaving no pending approval behind. Before #8409 the same
/// turn guest-gated the sender (the `channel_index` has no `webui` binding to
/// hand), `force_approval` flipped on, and this exact call sat in the queue
/// with no way for the dashboard to see it resolve.
#[tokio::test(flavor = "multi_thread")]
async fn authenticated_dashboard_turn_executes_a_tool_without_queuing_an_approval() {
    use futures::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::{client::IntoClientRequest, Message};

    let h = start_harness().await;
    let agent_id = spawn_agent(&h).await;

    let mut request = format!(
        "{}/api/agents/{agent_id}/ws",
        h.base_url.replacen("http://", "ws://", 1)
    )
    .into_client_request()
    .unwrap();
    request.headers_mut().insert(
        "authorization",
        format!("Bearer {USER_KEY}").parse().unwrap(),
    );
    let (mut socket, _) = tokio_tungstenite::connect_async(request).await.unwrap();
    let connected = socket.next().await.unwrap().unwrap();
    assert!(connected.to_text().unwrap().contains("connected"));

    socket
        .send(Message::Text(
            serde_json::json!({
                "type": "message",
                "content": "remember that the dashboard works",
                "message_id": "rbac-turn",
            })
            .to_string()
            .into(),
        ))
        .await
        .unwrap();

    // Drain frames until the terminal `response` frame OR the approval queue
    // gains a pending request, whichever comes first. Both outcomes are
    // informative: the attributed path completes the turn and must leave the
    // queue empty; the guest-gated path stalls the turn inside the approval
    // flow — which is precisely what the final assertion below turns into a
    // failure instead of a timeout.
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(30);
    let mut frames: Vec<String> = Vec::new();
    let mut saw_terminal_frame = false;
    loop {
        if tokio::time::Instant::now() >= deadline || pending_approvals(&h) > 0 {
            break;
        }
        let frame = match tokio::time::timeout(std::time::Duration::from_millis(250), socket.next())
            .await
        {
            Ok(Some(Ok(frame))) => frame,
            Ok(_) => break,     // stream closed
            Err(_) => continue, // poll slice elapsed — re-check the queue
        };
        let text = frame.to_text().expect("text frame").to_string();
        saw_terminal_frame = serde_json::from_str::<serde_json::Value>(&text)
            .ok()
            .and_then(|json| json["type"].as_str().map(str::to_string))
            .is_some_and(|t| t == "response");
        frames.push(text);
        if saw_terminal_frame {
            break;
        }
    }
    let _ = socket.close(None).await;

    // The stub must have been consulted for the tool call — otherwise the turn
    // never reached the tool gate and a passing "no pending approval" below
    // would be vacuous.
    let requests = h.llm.received_requests().await.expect("wiremock records");
    assert!(
        !requests.is_empty(),
        "the stub provider received no request: the turn never reached the LLM; \
         frames: {frames:?}"
    );
    if saw_terminal_frame {
        assert_eq!(
            requests.len(),
            2,
            "the turn must run the model twice (tool call, then answer); frames: {frames:?}"
        );
    }

    assert_eq!(
        pending_approvals(&h),
        0,
        "an authenticated owner's dashboard turn must execute the tool, not queue an \
         approval; frames: {frames:?}"
    );

    // The tool call itself is visible to the dashboard as executed, not as a
    // deferred approval card. Only the completed turn can assert this — the
    // guest-gated turn never executes the tool at all.
    let executed = frames.iter().any(|f| {
        serde_json::from_str::<serde_json::Value>(f)
            .ok()
            .and_then(|json| json["type"].as_str().map(str::to_string))
            == Some("tool_result".to_string())
    });
    assert!(
        executed,
        "the turn must surface an executed tool_result frame; frames: {frames:?}"
    );
}

/// A sender the gate cannot recognise still escalates — the guard that keeps
/// the fix from becoming a bypass, asserted against the *same booted kernel*
/// the authenticated turn above ran on.
///
/// The unauthenticated dashboard path stamps `user_id = client_ip`, so this is
/// the exact gate decision that turn's sender would hit: the
/// `resolve_user_tool_decision` trait method the runtime dispatch calls, with
/// the guest-gated pair. Asserting it here too means the guard cannot drift
/// from the kernel configuration the integration harness boots.
#[tokio::test(flavor = "multi_thread")]
async fn unrecognised_webui_sender_still_gates_as_guest_on_the_booted_kernel() {
    use librefang_types::user_policy::UserToolGate;

    let h = start_harness().await;
    let gate = h.state.kernel.resolve_user_tool_decision(
        "memory_store",
        Some("127.0.0.1"),
        Some("webui"),
        false,
    );
    assert!(
        matches!(gate, UserToolGate::NeedsApproval { .. }),
        "the unauthenticated webui sender (client IP) must keep the guest gate, got {gate:?}"
    );
    assert_eq!(pending_approvals(&h), 0, "no turn ran on this harness");
}
