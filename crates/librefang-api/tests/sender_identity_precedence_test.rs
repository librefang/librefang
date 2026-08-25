//! #7744 — an authenticated caller may not assert someone else's sender identity.
//!
//! `POST /api/agents/{id}/message` accepts `sender_id` / `sender_name` / `channel_type` as
//! `#[serde(default)]` body fields, and `request_sender_context` used to copy them into
//! `SenderContext` verbatim.
//! That struct is not decoration: `SenderContext.{channel, user_id}` is the
//! `(channel_type, platform_id)` tuple `AuthManager::identify` and `AuthManager::resolve_user` key
//! on, and it is stamped into `manifest.metadata["sender_user_id"]` for per-sender tool
//! authorization and `peer:{user_id}:KEY` memory scoping.
//! `user_role_allows_request` admits any `User`-role bearer to this route, so the body could name
//! any user the operator had bound in `[[users]] channel_bindings` — a privilege escalation, not a
//! feature.
//!
//! These tests assert over the **system prompt the provider actually receives**, captured from a
//! wiremock stub speaking the native Ollama protocol.
//! `prompt_builder::build_channel_section` renders `You are responding via {channel}` and
//! `The current message is from user "{display_name}" (platform ID: {user_id})`, so the captured
//! request body is a faithful readout of the `SenderContext` the kernel resolved — a runtime
//! assertion that compiles identically before and after the fix rather than a tautology over Rust
//! types.

use axum::Router;
use librefang_api::middleware;
use librefang_api::routes::{self, AppState};
use librefang_testing::{MockKernelBuilder, TestAppState};
use librefang_types::config::UserConfig;
use std::sync::Arc;
use tower_http::cors::CorsLayer;

const ROOT_KEY: &str = "root-test-key";

/// The identity the attacking body asserts: an `owner`-role user the operator bound to
/// `telegram:9999`, i.e. exactly the binding `authorize_channel_user` exists to establish for
/// inbound channel traffic.
const VICTIM_PLATFORM_ID: &str = "victim-9999";

/// The attacker's *own* declared `api` binding — the documented REST-operator recipe from
/// `docs/src/app/security/approvals/page.mdx`. Asserting this one is stating a true fact about
/// themselves and must keep working.
const ATTACKER_OWN_REST_ID: &str = "attacker-rest-id";

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
/// Without it the message handler's provider-auth gate short-circuits with 412 before any sender
/// resolution happens, and the test would pass on a broken build for the wrong reason.
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

/// Boot the agents router behind the real auth middleware, with RBAC users wired into both
/// `KernelConfig.users` (so `AuthManager` resolves them) and `AuthState.user_api_keys` (so the
/// middleware admits their bearer tokens and populates `AuthenticatedApiUser`).
///
/// Each tuple is `(name, role, api_key)`.
async fn start_harness(users: Vec<(&str, &str, &str)>) -> Harness {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let llm = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/chat"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "model": "test-model",
            "message": { "role": "assistant", "content": "ack" },
            "done": true,
            "done_reason": "stop",
            "prompt_eval_count": 7,
            "eval_count": 2,
        })))
        .mount(&llm)
        .await;

    let mut user_configs: Vec<UserConfig> = Vec::with_capacity(users.len());
    let mut api_user_records: Vec<middleware::ApiUserAuth> = Vec::with_capacity(users.len());
    for (name, role_str, key) in &users {
        let hash =
            librefang_api::password_hash::hash_password(key).expect("password hash should succeed");
        // The victim owns a `telegram` binding. Before #7744 an HTTP body naming
        // `channel_type = "telegram"` + `sender_id = VICTIM_PLATFORM_ID` resolved straight through
        // `AuthManager::identify` to this user.
        let mut channel_bindings = std::collections::HashMap::new();
        if *name == "victim" {
            channel_bindings.insert("telegram".to_string(), VICTIM_PLATFORM_ID.to_string());
        }
        if *name == "attacker" {
            channel_bindings.insert("api".to_string(), ATTACKER_OWN_REST_ID.to_string());
        }
        user_configs.push(UserConfig {
            name: (*name).to_string(),
            role: (*role_str).to_string(),
            channel_bindings,
            api_key_hash: Some(hash.clone()),
            ..Default::default()
        });
        api_user_records.push(middleware::ApiUserAuth {
            name: (*name).to_string(),
            role: librefang_kernel::auth::UserRole::from_str_role(role_str),
            api_key_hash: hash,
            user_id: librefang_types::agent::UserId::from_name(name),
        });
    }

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
                // Keep the turn to a single provider round trip — proactive memory would add
                // retrieval and extraction calls that muddy `received_requests()`.
                cfg.proactive_memory.enabled = false;
            })
            .with_catalog_seed(ollama_stub_catalog(&uri)),
    )
    .with_api_key(ROOT_KEY)
    .with_user_api_keys(api_user_records);

    let (state, tmp, _) = test.into_parts();

    let auth_state = middleware::AuthState {
        api_key_lock: state.api_key_lock.clone(),
        master_key: state.master_key.clone(),
        active_sessions: state.active_sessions.clone(),
        dashboard_auth_enabled: false,
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

/// Spawn an agent authored by `author` using the root (Owner) key.
///
/// `can_access_agent` scopes sub-Admin callers to agents whose `manifest.author` matches their
/// name, so the attacker must own the agent they are messaging — otherwise the handler 404s before
/// any sender resolution happens and the test would prove nothing.
async fn spawn_agent_for(h: &Harness, author: &str) -> String {
    let manifest = format!(
        r#"
name = "test-agent"
version = "0.1.0"
description = "Integration test agent"
author = "{author}"
module = "builtin:chat"

[model]
provider = "ollama"
model = "test-model"
system_prompt = "You are a test agent. Reply concisely."

[capabilities]
tools = ["file_read"]
memory_read = ["*"]
memory_write = ["self.*"]
"#
    );
    let resp = reqwest::Client::new()
        .post(format!("{}/api/agents", h.base_url))
        .bearer_auth(ROOT_KEY)
        .json(&serde_json::json!({ "manifest_toml": manifest }))
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

/// POST a message that asserts the victim's channel identity in the body.
///
/// The response body is drained before returning. On `message/stream` the handler answers 200 as
/// soon as the SSE stream opens and the provider call happens as the stream is consumed, so
/// checking `received_requests()` against an unread response would race the turn and read an empty
/// list — which is exactly the shape of a passing "the forged id never arrived" assertion.
async fn post_impersonating_message(h: &Harness, agent_id: &str, bearer: &str, route: &str) -> u16 {
    let resp = reqwest::Client::new()
        .post(format!("{}/api/agents/{}/{}", h.base_url, agent_id, route))
        .bearer_auth(bearer)
        .timeout(std::time::Duration::from_secs(30))
        .json(&serde_json::json!({
            "message": "who am I?",
            "sender_id": VICTIM_PLATFORM_ID,
            "sender_name": "Victim",
            "channel_type": "telegram",
        }))
        .send()
        .await
        .expect("message request");
    let status = resp.status().as_u16();
    resp.text().await.expect("drain response body");
    status
}

/// Concatenate every system prompt the stub provider received.
///
/// Panics when nothing arrived — that means the turn never reached the provider and any
/// "the forged id is absent" assertion below would be vacuously true.
async fn captured_prompts(h: &Harness) -> String {
    let requests = h
        .llm
        .received_requests()
        .await
        .expect("wiremock records requests");
    assert!(
        !requests.is_empty(),
        "the stub provider received no request: the turn never reached the LLM, so an \
         absent forged sender id proves nothing"
    );
    requests
        .iter()
        .map(|r| String::from_utf8_lossy(&r.body).into_owned())
        .collect::<Vec<_>>()
        .join("\n")
}

/// A `User`-role caller asserting an `Owner`'s bound channel identity gets their own instead.
///
/// This is the escalation #7744 names: `victim` is bound to `telegram:VICTIM_PLATFORM_ID`, so
/// before the fix `AuthManager::identify("telegram", VICTIM_PLATFORM_ID)` resolved the turn's
/// sender to an Owner-role principal chosen entirely by the request body.
#[tokio::test(flavor = "multi_thread")]
async fn sub_admin_caller_cannot_assert_another_users_sender_identity() {
    let h = start_harness(vec![
        ("attacker", "user", "attacker-key"),
        ("victim", "owner", "victim-key"),
    ])
    .await;
    let agent_id = spawn_agent_for(&h, "attacker").await;

    let status = post_impersonating_message(&h, &agent_id, "attacker-key", "message").await;
    assert_eq!(
        status, 200,
        "the request must succeed silently — a body asserting its own true identity is not an \
         attack, and failing it would leak the role check back to the caller"
    );

    let prompts = captured_prompts(&h).await;
    assert!(
        !prompts.contains(VICTIM_PLATFORM_ID),
        "the forged platform id reached the turn: {prompts}"
    );
    assert!(
        prompts.contains("platform ID: attacker"),
        "the authenticated identity must win: {prompts}"
    );
    assert!(
        prompts.contains("responding via api"),
        "the identity namespace must be pinned alongside the id — `identify` keys on the \
         (channel, user_id) pair: {prompts}"
    );
    assert!(
        !prompts.contains("responding via telegram"),
        "a sub-Admin caller must not be able to claim the turn arrived over a channel: {prompts}"
    );
}

/// The streaming sibling applies the same precedence.
///
/// `send_message_stream` reads the authenticated identity and builds the asserted one in two
/// places that were never joined, and it is the handler with no `owner` consumer downstream at
/// all — so it needs its own coverage, not an inference from the non-streaming test.
#[tokio::test(flavor = "multi_thread")]
async fn sub_admin_caller_cannot_assert_another_identity_on_the_streaming_route() {
    let h = start_harness(vec![
        ("attacker", "user", "attacker-key"),
        ("victim", "owner", "victim-key"),
    ])
    .await;
    let agent_id = spawn_agent_for(&h, "attacker").await;

    let status = post_impersonating_message(&h, &agent_id, "attacker-key", "message/stream").await;
    assert_eq!(status, 200, "the SSE route must open normally");

    let prompts = captured_prompts(&h).await;
    assert!(
        !prompts.contains(VICTIM_PLATFORM_ID),
        "the forged platform id reached the streaming turn: {prompts}"
    );
    assert!(
        prompts.contains("platform ID: attacker"),
        "the authenticated identity must win on the streaming route too: {prompts}"
    );
}

/// An `Admin` may assert a sender identity — an operator impersonating a user for support, or a
/// channel gateway relaying real platform users, is legitimate and must keep working.
#[tokio::test(flavor = "multi_thread")]
async fn admin_caller_may_assert_a_sender_identity() {
    let h = start_harness(vec![
        ("ops", "admin", "ops-key"),
        ("victim", "owner", "victim-key"),
    ])
    .await;
    let agent_id = spawn_agent_for(&h, "ops").await;

    let status = post_impersonating_message(&h, &agent_id, "ops-key", "message").await;
    assert_eq!(status, 200, "an admin-authored message must succeed");

    let prompts = captured_prompts(&h).await;
    assert!(
        prompts.contains(&format!("platform ID: {VICTIM_PLATFORM_ID}")),
        "an Admin must retain the ability to assert a sender identity: {prompts}"
    );
    assert!(
        prompts.contains("responding via telegram"),
        "an Admin must retain the ability to name the channel: {prompts}"
    );
}

/// A sub-Admin caller asserting an id that `identify` resolves back to *themselves* keeps it.
///
/// Without this carve-out the fix would silently demote every REST operator following the
/// `[[users]] channel_bindings.api = "..."` recipe in `docs/src/app/security/approvals/page.mdx` to
/// the built-in guest gate — turning a privilege-escalation fix into a per-user tool-policy outage.
#[tokio::test(flavor = "multi_thread")]
async fn sub_admin_caller_may_assert_their_own_declared_binding() {
    let h = start_harness(vec![
        ("attacker", "user", "attacker-key"),
        ("victim", "owner", "victim-key"),
    ])
    .await;
    let agent_id = spawn_agent_for(&h, "attacker").await;

    let resp = reqwest::Client::new()
        .post(format!("{}/api/agents/{}/message", h.base_url, agent_id))
        .bearer_auth("attacker-key")
        .json(&serde_json::json!({
            "message": "who am I?",
            "sender_id": ATTACKER_OWN_REST_ID,
            "sender_name": "Ops Bot",
            "channel_type": "api",
        }))
        .send()
        .await
        .expect("message request");
    let status = resp.status().as_u16();
    resp.text().await.expect("drain response body");
    assert_eq!(status, 200);

    let prompts = captured_prompts(&h).await;
    assert!(
        prompts.contains(&format!("platform ID: {ATTACKER_OWN_REST_ID}")),
        "a caller asserting their own declared binding must keep it: {prompts}"
    );
}
