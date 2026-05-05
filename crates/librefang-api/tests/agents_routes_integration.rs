//! Integration tests for the `/api/agents` route family.
//!
//! Refs #3571 — agents-domain slice. These tests exercise the production
//! router (`server::build_router`) with `tower::ServiceExt::oneshot`, so the
//! real auth middleware, route registration, and handler logic are all in
//! play. No real LLM calls (provider is `ollama` with a fake model) — every
//! test is hermetic.
//!
//! Routes covered:
//!   GET   /api/agents              (list — empty filter + populated)
//!   GET   /api/agents/{id}         (happy path + invalid id 400 + unknown 404)
//!   PATCH /api/agents/{id}         (success, invalid payload, unknown 404,
//!                                   read-after-write via GET, auth gate 401)
//!
//! Run: cargo test -p librefang-api --test agents_routes_integration

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use librefang_api::routes::AppState;
use librefang_api::server;
use librefang_kernel::LibreFangKernel;
use librefang_types::agent::{AgentId, AgentManifest};
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

async fn boot(api_key: &str) -> Harness {
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

    Harness {
        app,
        state,
        _tmp: tmp,
    }
}

fn spawn_named(state: &Arc<AppState>, name: &str) -> AgentId {
    let manifest = AgentManifest {
        name: name.to_string(),
        ..AgentManifest::default()
    };
    state.kernel.spawn_agent(manifest).expect("spawn_agent")
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

/// Bearer token used by all authenticated test requests. Every harness
/// (except the explicit auth-gate test) boots with this api_key so the
/// production middleware accepts the requests as authenticated.
const TEST_TOKEN: &str = "test-secret";

fn get(path: &str) -> Request<Body> {
    get_with(path, Some(TEST_TOKEN))
}

fn get_with(path: &str, bearer: Option<&str>) -> Request<Body> {
    let mut b = Request::builder().method(Method::GET).uri(path);
    if let Some(token) = bearer {
        b = b.header("authorization", format!("Bearer {}", token));
    }
    b.body(Body::empty()).unwrap()
}

fn patch_json(path: &str, body: serde_json::Value, bearer: Option<&str>) -> Request<Body> {
    let mut b = Request::builder()
        .method(Method::PATCH)
        .uri(path)
        .header("content-type", "application/json");
    if let Some(token) = bearer {
        b = b.header("authorization", format!("Bearer {}", token));
    }
    b.body(Body::from(body.to_string())).unwrap()
}

fn post_json(path: &str, body: serde_json::Value) -> Request<Body> {
    Request::builder()
        .method(Method::POST)
        .uri(path)
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {}", TEST_TOKEN))
        .body(Body::from(body.to_string()))
        .unwrap()
}

// ---------------------------------------------------------------------------
// GET /api/agents
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn test_list_agents_returns_default_assistant_only() {
    // The kernel auto-spawns a single default assistant on boot — so the
    // "empty user-spawn" baseline is exactly one entry. We further filter by
    // a unique q= to assert the empty case truly returns zero matches.
    let h = boot(TEST_TOKEN).await;

    let (status, body) = send(
        h.app.clone(),
        get("/api/agents?q=__definitely_no_such_agent__"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let items = body["items"].as_array().expect("items array");
    assert!(
        items.is_empty(),
        "expected empty filter result, got {:?}",
        items
    );
    assert_eq!(body["total"], 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_list_agents_returns_spawned_agents() {
    let h = boot(TEST_TOKEN).await;
    let id_a = spawn_named(&h.state, "alpha-agent");
    let id_b = spawn_named(&h.state, "beta-agent");

    let (status, body) = send(h.app.clone(), get("/api/agents")).await;
    assert_eq!(status, StatusCode::OK);

    let items = body["items"].as_array().expect("items array");
    let ids: Vec<String> = items
        .iter()
        .map(|a| a["id"].as_str().unwrap().to_string())
        .collect();
    assert!(ids.contains(&id_a.to_string()), "missing alpha: {:?}", ids);
    assert!(ids.contains(&id_b.to_string()), "missing beta: {:?}", ids);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_list_agents_rejects_invalid_sort_field() {
    let h = boot(TEST_TOKEN).await;
    let (status, body) = send(h.app.clone(), get("/api/agents?sort=not_a_field")).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body["error"].is_string());
}

// ---------------------------------------------------------------------------
// GET /api/agents/{id}
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn test_get_agent_happy_path() {
    let h = boot(TEST_TOKEN).await;
    let id = spawn_named(&h.state, "lookup-target");

    let (status, body) = send(h.app.clone(), get(&format!("/api/agents/{}", id))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["id"], id.to_string());
    assert_eq!(body["name"], "lookup-target");
    assert!(body["model"].is_object());
    assert!(body["capabilities"].is_object());
}

#[tokio::test(flavor = "multi_thread")]
async fn test_get_agent_invalid_id_returns_400() {
    let h = boot(TEST_TOKEN).await;
    let (status, body) = send(h.app.clone(), get("/api/agents/not-a-uuid")).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["code"], "invalid_agent_id");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_get_agent_unknown_returns_404() {
    let h = boot(TEST_TOKEN).await;
    let unknown = AgentId::new();
    let (status, body) = send(h.app.clone(), get(&format!("/api/agents/{}", unknown))).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["code"], "agent_not_found");
}

// ---------------------------------------------------------------------------
// PATCH /api/agents/{id}
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn test_patch_agent_updates_name_and_description() {
    let h = boot(TEST_TOKEN).await;
    let id = spawn_named(&h.state, "patch-target");

    let (status, _) = send(
        h.app.clone(),
        patch_json(
            &format!("/api/agents/{}", id),
            serde_json::json!({
                "name": "renamed-agent",
                "description": "updated via PATCH"
            }),
            Some(TEST_TOKEN),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // Read-after-write — GET should reflect the new name + description.
    let (status, body) = send(h.app.clone(), get(&format!("/api/agents/{}", id))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["name"], "renamed-agent");
    assert_eq!(body["description"], "updated via PATCH");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_patch_agent_invalid_mcp_servers_payload_returns_400() {
    let h = boot(TEST_TOKEN).await;
    let id = spawn_named(&h.state, "bad-payload");

    // mcp_servers must be an array of strings; nested objects are rejected.
    let (status, body) = send(
        h.app.clone(),
        patch_json(
            &format!("/api/agents/{}", id),
            serde_json::json!({"mcp_servers": [{"oops": true}]}),
            Some(TEST_TOKEN),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body["error"].is_string());
}

#[tokio::test(flavor = "multi_thread")]
async fn test_patch_agent_unknown_returns_404() {
    let h = boot(TEST_TOKEN).await;
    let unknown = AgentId::new();

    let (status, _) = send(
        h.app.clone(),
        patch_json(
            &format!("/api/agents/{}", unknown),
            serde_json::json!({"name": "anything"}),
            Some(TEST_TOKEN),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_patch_agent_invalid_id_returns_400() {
    let h = boot(TEST_TOKEN).await;

    let (status, _) = send(
        h.app.clone(),
        patch_json(
            "/api/agents/not-a-uuid",
            serde_json::json!({"name": "anything"}),
            Some(TEST_TOKEN),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

// ---------------------------------------------------------------------------
// Auth gate — PATCH is a mutation, NOT in PUBLIC_ROUTES_DASHBOARD_READS, so
// once an api_key is configured a non-loopback request without a Bearer
// token must be rejected with 401. (oneshot has no ConnectInfo, so the
// loopback fast-path does NOT apply — the request is treated as remote.)
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn test_patch_agent_without_token_returns_401_when_api_key_set() {
    let h = boot("test-secret").await;
    let id = spawn_named(&h.state, "auth-gated");

    let (status, _) = send(
        h.app.clone(),
        patch_json(
            &format!("/api/agents/{}", id),
            serde_json::json!({"name": "should-not-apply"}),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    // Sanity: with the correct Bearer token the same request succeeds.
    let (status_ok, _) = send(
        h.app.clone(),
        patch_json(
            &format!("/api/agents/{}", id),
            serde_json::json!({"name": "did-apply"}),
            Some("test-secret"),
        ),
    )
    .await;
    assert_eq!(status_ok, StatusCode::OK);
}

// ---------------------------------------------------------------------------
// DELETE /api/agents/{id} — idempotency (#3509)
// ---------------------------------------------------------------------------

fn delete(path: &str, bearer: Option<&str>) -> Request<Body> {
    let mut b = Request::builder().method(Method::DELETE).uri(path);
    if let Some(token) = bearer {
        b = b.header("authorization", format!("Bearer {}", token));
    }
    b.body(Body::empty()).unwrap()
}

/// Refs #4614 — DELETE /api/agents/{id} requires `?confirm=true` (or a
/// `{"confirm": true}` body) to gate the destructive canonical-UUID
/// purge. This helper appends the query param so tests don't have to
/// open-code the URL in every call site.
fn delete_confirmed(path: &str, bearer: Option<&str>) -> Request<Body> {
    let glue = if path.contains('?') { '&' } else { '?' };
    let with_confirm = format!("{path}{glue}confirm=true");
    delete(&with_confirm, bearer)
}

/// Refs #3509: DELETE is idempotent (RFC 9110 §9.2.2). Killing the same
/// agent twice MUST succeed both times — the second call returns
/// `200 OK` with `status: already-deleted` instead of `404 Not Found`,
/// so clients (dashboard double-clicks, CLI retries, network-recovery
/// loops) never see a phantom error for an outcome that already matches
/// their intent.
#[tokio::test(flavor = "multi_thread")]
async fn test_delete_agent_twice_both_succeed_idempotent() {
    let h = boot(TEST_TOKEN).await;
    let id = spawn_named(&h.state, "kill-target");

    // First call — agent exists, normal kill path. Refs #4614: confirm
    // required to gate canonical-UUID purge.
    let (status1, body1) = send(
        h.app.clone(),
        delete_confirmed(&format!("/api/agents/{}", id), Some(TEST_TOKEN)),
    )
    .await;
    assert_eq!(
        status1,
        StatusCode::OK,
        "first DELETE should be 200; body={body1:?}"
    );
    assert_eq!(body1["status"], "killed", "first DELETE body={body1:?}");

    // Second call — agent already gone. MUST still be 200, not 404.
    let (status2, body2) = send(
        h.app.clone(),
        delete_confirmed(&format!("/api/agents/{}", id), Some(TEST_TOKEN)),
    )
    .await;
    assert_eq!(
        status2,
        StatusCode::OK,
        "second DELETE on a now-absent agent must be idempotent-200 (#3509); got {status2} body={body2:?}"
    );
    assert_eq!(
        body2["status"], "already-deleted",
        "second DELETE body={body2:?}"
    );
}

/// Refs #3509: 400 stays reserved for malformed-id rejection. Only the
/// `not-found` case relaxed to 200 idempotent. Without this the relaxation
/// could mask genuine client bugs (typo'd id, wrong path).
#[tokio::test(flavor = "multi_thread")]
async fn test_delete_agent_invalid_id_still_returns_400() {
    let h = boot(TEST_TOKEN).await;
    // Bare DELETE — malformed UUID short-circuits with 400 before the
    // confirmation check fires, so the response stays the same shape
    // post-#4614.
    let (status, body) = send(
        h.app.clone(),
        delete("/api/agents/not-a-uuid", Some(TEST_TOKEN)),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "body={body:?}");
    assert_eq!(body["code"], "invalid_agent_id");
}

/// Refs #3509: deleting an unknown-but-well-formed UUID is idempotent —
/// no agent existed under that id, so the caller's intent ("agent {id}
/// should be gone") is already satisfied. 200 with `already-deleted` lets
/// idempotent clients (Terraform-style reconcilers) skip the dance.
#[tokio::test(flavor = "multi_thread")]
async fn test_delete_agent_unknown_uuid_is_idempotent_200() {
    let h = boot(TEST_TOKEN).await;
    let unknown = AgentId::new();
    // Refs #4614: confirm required even on the idempotent-already-gone
    // path so the contract is consistent across all DELETEs.
    let (status, body) = send(
        h.app.clone(),
        delete_confirmed(&format!("/api/agents/{}", unknown), Some(TEST_TOKEN)),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body={body:?}");
    assert_eq!(body["status"], "already-deleted", "body={body:?}");
}

// ---------------------------------------------------------------------------
// GET /api/agents/{id}/session — thinking blocks reach the dashboard
// ---------------------------------------------------------------------------

/// Persisted `ContentBlock::Thinking` blocks must be surfaced on the
/// agent-scoped session endpoint so the dashboard can render the
/// collapsible reasoning drawer on history reload — same UX as live
/// streaming, where `thinking_delta` events accumulate into the message.
///
/// Before this fix the endpoint flattened blocks into a string and silently
/// swallowed Thinking via the catch-all match arm, so reload showed an
/// assistant turn with no reasoning even though the session JSON had it.
#[tokio::test(flavor = "multi_thread")]
async fn test_agent_session_endpoint_surfaces_thinking_blocks() {
    use librefang_types::message::{ContentBlock, Message, MessageContent, Role};

    let h = boot(TEST_TOKEN).await;
    let id = spawn_named(&h.state, "thinking-target");

    // Seed a session with an assistant turn that has interleaved thinking
    // and text blocks. Two thinking blocks exercise the multi-block join.
    let mut session = h
        .state
        .kernel
        .memory_substrate()
        .create_session(id)
        .expect("create_session");
    session.push_message(Message {
        role: Role::User,
        content: MessageContent::Text("hi".to_string()),
        pinned: false,
        timestamp: None,
    });
    session.push_message(Message {
        role: Role::Assistant,
        content: MessageContent::Blocks(vec![
            ContentBlock::Thinking {
                thinking: "first reasoning step".to_string(),
                provider_metadata: None,
            },
            ContentBlock::Text {
                text: "visible answer".to_string(),
                provider_metadata: None,
            },
            ContentBlock::Thinking {
                thinking: "follow-up reasoning".to_string(),
                provider_metadata: None,
            },
        ]),
        pinned: false,
        timestamp: None,
    });
    let session_id = session.id.0;
    h.state
        .kernel
        .memory_substrate()
        .save_session(&session)
        .expect("save_session");

    let (status, body) = send(
        h.app.clone(),
        get(&format!(
            "/api/agents/{}/session?session_id={}",
            id, session_id
        )),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body={body:?}");
    let messages = body["messages"].as_array().expect("messages array").clone();
    let assistant = messages
        .iter()
        .find(|m| m["role"] == "Assistant")
        .expect("assistant message");
    // Visible text still flattens — same shape the dashboard already
    // rendered before this change.
    assert_eq!(assistant["content"], "visible answer");
    // Thinking now surfaces as a flat string with multi-block join. The
    // dashboard's history mapper reads this directly into
    // `ChatMessage.thinking`, mirroring the live-streaming flat-string
    // accumulation from `thinking_delta` events.
    assert_eq!(
        assistant["thinking"], "first reasoning step\n\nfollow-up reasoning",
        "thinking field missing or wrong join — body={body:?}",
    );
}

/// Sessions without thinking blocks must NOT include a `thinking` field
/// on assistant messages. Omitting (vs. emitting `""`) keeps the response
/// shape unchanged for non-thinking models and avoids triggering the
/// dashboard's empty-drawer render gate.
#[tokio::test(flavor = "multi_thread")]
async fn test_agent_session_endpoint_omits_thinking_when_none_present() {
    use librefang_types::message::{ContentBlock, Message, MessageContent, Role};

    let h = boot(TEST_TOKEN).await;
    let id = spawn_named(&h.state, "no-thinking-target");

    let mut session = h
        .state
        .kernel
        .memory_substrate()
        .create_session(id)
        .expect("create_session");
    session.push_message(Message {
        role: Role::Assistant,
        content: MessageContent::Blocks(vec![ContentBlock::Text {
            text: "plain answer".to_string(),
            provider_metadata: None,
        }]),
        pinned: false,
        timestamp: None,
    });
    let session_id = session.id.0;
    h.state
        .kernel
        .memory_substrate()
        .save_session(&session)
        .expect("save_session");

    let (status, body) = send(
        h.app.clone(),
        get(&format!(
            "/api/agents/{}/session?session_id={}",
            id, session_id
        )),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body={body:?}");
    let messages = body["messages"].as_array().expect("messages array");
    let assistant = messages
        .iter()
        .find(|m| m["role"] == "Assistant")
        .expect("assistant message");
    assert_eq!(assistant["content"], "plain answer");
    assert!(
        assistant.get("thinking").is_none(),
        "thinking field should be absent — body={body:?}",
    );
}

/// A turn whose `MessageContent::Blocks` contains ONLY `Thinking`
/// (e.g. an aborted/cancelled response, or a server filter that
/// stripped the visible text) MUST still surface to the dashboard so
/// the collapsible thinking drawer renders. Pre-fix the route's
/// `if content.is_empty() && tools.is_empty()` early-skip dropped the
/// turn before the `thinking` field was attached, contradicting the
/// dashboard's `hasThinking` render branch which is explicitly
/// designed for thinking-only turns.
#[tokio::test(flavor = "multi_thread")]
async fn test_agent_session_endpoint_surfaces_thinking_only_turns() {
    use librefang_types::message::{ContentBlock, Message, MessageContent, Role};

    let h = boot(TEST_TOKEN).await;
    let id = spawn_named(&h.state, "thinking-only-target");

    let mut session = h
        .state
        .kernel
        .memory_substrate()
        .create_session(id)
        .expect("create_session");
    // Seed a user prompt followed by an assistant turn with NO text /
    // tool_use — only Thinking. Mirrors a cancelled-mid-stream
    // response that produced reasoning before the visible answer
    // started.
    session.push_message(Message {
        role: Role::User,
        content: MessageContent::Text("hi".to_string()),
        pinned: false,
        timestamp: None,
    });
    session.push_message(Message {
        role: Role::Assistant,
        content: MessageContent::Blocks(vec![ContentBlock::Thinking {
            thinking: "reasoning that never reached an answer".to_string(),
            provider_metadata: None,
        }]),
        pinned: false,
        timestamp: None,
    });
    let session_id = session.id.0;
    h.state
        .kernel
        .memory_substrate()
        .save_session(&session)
        .expect("save_session");

    let (status, body) = send(
        h.app.clone(),
        get(&format!(
            "/api/agents/{}/session?session_id={}",
            id, session_id
        )),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body={body:?}");
    let messages = body["messages"].as_array().expect("messages array").clone();
    let assistant = messages
        .iter()
        .find(|m| m["role"] == "Assistant")
        .expect("thinking-only assistant turn must NOT be dropped — body={body:?}");
    assert_eq!(
        assistant["content"], "",
        "thinking-only turn has no visible text — body={body:?}",
    );
    assert_eq!(
        assistant["thinking"], "reasoning that never reached an answer",
        "thinking field must surface so the dashboard's hasThinking branch can render — body={body:?}",
    );
}

// ---------------------------------------------------------------------------
// Incognito mode — refs #4073
// ---------------------------------------------------------------------------

/// The `incognito` field in the POST /api/agents/{id}/message body must
/// deserialize cleanly. A request with `incognito: true` must not return a
/// 422 Unprocessable Entity; if the provider auth is missing (the test
/// harness uses a fake ollama model) the server returns 412 as usual.
/// This verifies the API surface is wired end-to-end without a real LLM.
#[tokio::test(flavor = "multi_thread")]
async fn test_incognito_field_accepted_by_message_endpoint() {
    let h = boot(TEST_TOKEN).await;
    let id = spawn_named(&h.state, "incognito-test-agent");

    // incognito: true — must NOT be 422 (unknown field / bad deserialize)
    let (status, body) = send(
        h.app.clone(),
        post_json(
            &format!("/api/agents/{id}/message"),
            serde_json::json!({"message": "hello", "incognito": true}),
        ),
    )
    .await;
    assert_ne!(
        status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "incognito field must deserialize cleanly — body={body:?}",
    );
    // Provider is unconfigured → 412 or 500, NOT 422.
    assert!(
        status == StatusCode::PRECONDITION_FAILED || status.is_server_error(),
        "expected provider-auth 412 or server error, got {status} — body={body:?}",
    );
}

/// Omitting `incognito` entirely must still work (backward compat: defaults to false).
#[tokio::test(flavor = "multi_thread")]
async fn test_incognito_defaults_to_false_when_omitted() {
    let h = boot(TEST_TOKEN).await;
    let id = spawn_named(&h.state, "incognito-omit-agent");

    let (status, _body) = send(
        h.app.clone(),
        post_json(
            &format!("/api/agents/{id}/message"),
            serde_json::json!({"message": "hello"}),
        ),
    )
    .await;
    // Must not be 422 — the field absence defaults to false via #[serde(default)].
    assert_ne!(status, StatusCode::UNPROCESSABLE_ENTITY);
}

/// Session messages must NOT be persisted when incognito: true.
///
/// We seed a session with one user message, send an incognito message via
/// the kernel directly (bypassing the LLM by using the incognito flag), and
/// then re-read the session to confirm it was not extended. The test calls
/// `kernel.send_message_with_incognito()` which will fail at the provider
/// layer (no real LLM in test), but the session save is guarded before the
/// LLM call, so the pre-existing messages must not grow regardless.
#[tokio::test(flavor = "multi_thread")]
async fn test_incognito_message_does_not_persist_session() {
    let h = boot(TEST_TOKEN).await;
    let id = spawn_named(&h.state, "incognito-persist-agent");

    // Plant one user message in the canonical session via memory substrate.
    let mem = h.state.kernel.memory_substrate();
    let mut session = mem.create_session(id).expect("create_session");
    let seed_msg = librefang_types::message::Message {
        role: librefang_types::message::Role::User,
        content: librefang_types::message::MessageContent::Text(
            "seed message before incognito".to_string(),
        ),
        pinned: false,
        timestamp: None,
    };
    session.push_message(seed_msg);
    let session_id = session.id;
    mem.save_session(&session).expect("save_session");

    // Confirm the session has exactly 1 message before the incognito call.
    let before = mem
        .get_session(session_id)
        .expect("get_session")
        .expect("session must exist");
    assert_eq!(
        before.messages.len(),
        1,
        "expected 1 seeded message before incognito call",
    );

    // Send an incognito message via the kernel. This will fail at the LLM
    // provider level (no real model in test), but the test only cares that
    // the session was not written to on the way out.
    let _ = h
        .state
        .kernel
        .send_message_with_incognito(
            id,
            "incognito message — must not persist",
            None,
            None,
            None,
            Some(session_id),
            true,
        )
        .await;

    // The session must still have exactly 1 message — the incognito turn must
    // not have appended anything to SQLite.
    let after = mem
        .get_session(session_id)
        .expect("get_session")
        .expect("session must still exist");
    assert_eq!(
        after.messages.len(),
        1,
        "incognito turn must not persist new messages to the session (got {} messages) — messages={:?}",
        after.messages.len(),
        after.messages,
    );
}
