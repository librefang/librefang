//! The authenticated owner of a turn must reach the tool dispatcher (#7744).
//!
//! Two identities travel with every agent turn and, before this test existed, only one of them survived as far as `tool_runner::dispatch`:
//!
//! 1. `owner: Option<UserId>` — extracted from the bearer credential by the auth middleware, typed, unforgeable by the caller.
//! 2. `sender_id: Option<&str>` — a free-form platform handle that, over HTTP, is read straight out of `MessageRequest.sender_id` in the request **body**.
//!
//! `user_role_allows_request` lets any bearer holding the `User` role POST to `/api/agents/{id}/message`, so `sender_id` is attacker-chosen, yet it alone drove per-sender tool authorization, approval routing, and the `peer:{user_id}:KEY` memory namespace.
//!
//! This test pins the fix: dispatch must carry the bearer-authenticated identity, and a body-supplied `sender_id` that asserts somebody else must not survive to reach it.
//! `request_sender_context` is what enforces the second half — it pins a non-Admin caller to their own identity — so in this scenario `owner` and `sender_id` are *both* the authenticated caller.
//! That is the point the two assertions below make together, and it is worth stating plainly because the reverse would be easy to assume: this is not a test that the two fields can be told apart from each other, it is a test that a forged `sender_id` cannot displace either.
//!
//! The assertions read the structured `librefang::tool_identity` record that `execute_tool_with_sender_account` emits for every tool call.
//! Driving the check through log capture rather than through a Rust type means this file compiles against a tree that lacks the instrumentation and fails at *runtime* there with no records captured — which is what makes it a guard for that instrumentation rather than a compile-time tautology.
//!
//! A wiremock server speaking the native Ollama protocol (`POST /api/chat`) stands in for the provider so a complete turn — LLM call, tool dispatch, second LLM call with the tool result — runs without credentials or network access.

use librefang_api::middleware;
use librefang_api::routes::{self, AppState};
use librefang_kernel::auth::UserRole as KernelUserRole;
use librefang_testing::{MockKernelBuilder, TestAppState};
use librefang_types::agent::UserId;
use librefang_types::config::UserConfig;
use librefang_types::principal::Principal;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, OnceLock};
use tracing::field::{Field, Visit};
use tracing_subscriber::layer::{Context, SubscriberExt};
use tracing_subscriber::Layer;

/// Target of the structured identity record emitted at tool dispatch.
/// Kept in sync with `librefang_runtime::tool_runner::dispatch`.
const TOOL_IDENTITY_TARGET: &str = "librefang::tool_identity";

/// Alice holds the `User` role — the weakest role that may POST to `/api/agents/{id}/message`, and therefore the role an attacker would hold.
const ALICE_KEY: &str = "alice-dispatch-owner-key";
/// Agent creation needs `Admin`; Alice deliberately does not have it.
/// The admin exists only to spawn the agent — the turn under test is Alice's.
const ADMIN_KEY: &str = "admin-dispatch-owner-key";
/// The platform handle the *attacker* supplies in the request body.
/// Deliberately unrelated to any configured user.
const SPOOFED_SENDER_ID: &str = "B";

const MASTER_KEY: &str = "dispatch-owner-master-key";

/// Agent manifest granting exactly one tool, so the stub LLM has something dispatchable to ask for.
/// `file_read` needs no network and no fixture — the call is expected to fail on a nonexistent path, which is irrelevant here: the assertion is about the identity carried *into* dispatch, not the result.
///
/// `author = "Alice"` because `can_access_agent` scopes a non-Admin principal to the agents they authored — without it Alice's message would 404 before the turn ever starts.
const TEST_MANIFEST: &str = r#"
name = "dispatch-owner-agent"
version = "0.1.0"
description = "Owner-at-dispatch integration test agent"
author = "Alice"
module = "builtin:chat"

[model]
provider = "ollama"
model = "test-model"
system_prompt = "You are a test agent."

[capabilities]
tools = ["file_read"]
memory_read = ["*"]
memory_write = ["self.*"]
"#;

/// The group that owns the unattended agent in the group-principal case.
const ONCALL_GROUP: &str = "oncall";

/// Same single-tool agent, but owned by a group through the manifest `owner` field, which is how an operator attributes turns that have no human caller (cron fires, triggers, autonomous ticks).
const GROUP_OWNED_MANIFEST: &str = r#"
name = "dispatch-group-owner-agent"
version = "0.1.0"
description = "Group-owned dispatch identity test agent"
author = "Admin"
owner = "group:oncall"
module = "builtin:chat"

[model]
provider = "ollama"
model = "test-model"
system_prompt = "You are a test agent."

[capabilities]
tools = ["file_read"]
memory_read = ["*"]
memory_write = ["self.*"]
"#;

// ---------------------------------------------------------------------------
// Tracing capture
// ---------------------------------------------------------------------------

/// Collects the fields of every `librefang::tool_identity` event.
///
/// A field-level visitor rather than a text scrape of the `fmt` layer: the assertions below compare exact field values, so they must not be sensitive to formatter configuration or field ordering.
/// `owner` is recorded through `tracing::field::display`, whose `Debug` rendering is the principal's `Display` form, and `sender_id` arrives through `record_str`, so neither value carries `Option`'s `Debug` wrapping; a field that was `None` at the call site is simply absent.
#[derive(Clone, Default)]
struct IdentityCapture(Arc<Mutex<Vec<BTreeMap<String, String>>>>);

struct FieldVisitor<'a>(&'a mut BTreeMap<String, String>);

impl Visit for FieldVisitor<'_> {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        self.0
            .insert(field.name().to_string(), format!("{value:?}"));
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        self.0.insert(field.name().to_string(), value.to_string());
    }
}

impl<S: tracing::Subscriber> Layer<S> for IdentityCapture {
    fn on_event(&self, event: &tracing::Event<'_>, _ctx: Context<'_, S>) {
        if event.metadata().target() != TOOL_IDENTITY_TARGET {
            return;
        }
        let mut fields = BTreeMap::new();
        event.record(&mut FieldVisitor(&mut fields));
        self.0.lock().expect("capture lock").push(fields);
    }
}

/// Installs the capture layer as the process-wide subscriber exactly once.
///
/// The turn runs on the multi-threaded tokio runtime, so a thread-local (`set_default`) subscriber would miss events emitted on worker threads.
fn install_capture() -> IdentityCapture {
    static CAPTURE: OnceLock<IdentityCapture> = OnceLock::new();
    CAPTURE
        .get_or_init(|| {
            let capture = IdentityCapture::default();
            let subscriber = tracing_subscriber::registry().with(capture.clone());
            tracing::subscriber::set_global_default(subscriber)
                .expect("no other global subscriber may be installed in this test binary");
            capture
        })
        .clone()
}

/// Serializes the tests in this binary: they share the one global capture, and both drive `file_read`, so records from a concurrently running test would otherwise be indistinguishable from this test's own.
static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

impl IdentityCapture {
    /// Number of records captured so far, used as the starting offset for the current test.
    fn mark(&self) -> usize {
        self.0.lock().expect("capture lock").len()
    }

    /// Records for `tool` captured since `mark`.
    fn since(&self, mark: usize, tool: &str) -> Vec<BTreeMap<String, String>> {
        self.0.lock().expect("capture lock")[mark..]
            .iter()
            .filter(|r| r.get("tool").map(String::as_str) == Some(tool))
            .cloned()
            .collect()
    }
}

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

struct Harness {
    base_url: String,
    state: Arc<AppState>,
    llm: wiremock::MockServer,
    _tmp: tempfile::TempDir,
}

impl Drop for Harness {
    fn drop(&mut self) {
        self.state.kernel.shutdown();
    }
}

/// Model catalog carrying an `ollama` provider that needs no key and lives at `base_url`.
/// Without it the message handler's provider-auth gate short-circuits with 412 before the agent loop — and hence dispatch — is reached.
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

/// Boots the API with a single authenticated `User`-role principal (Alice) and a stub LLM whose first reply asks for `file_read` and whose second ends the turn — exactly one tool dispatch per request.
///
/// The router is `routes::agents::router()` behind the real auth middleware, not the full `server.rs` composition: the agent and message routes plus authentication are all this test exercises.
async fn boot() -> Harness {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let llm = MockServer::start().await;

    // First round trip: the model requests a tool. `up_to_n_times(1)` plus the higher priority makes this the response to the opening call only.
    Mock::given(method("POST"))
        .and(path("/api/chat"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "model": "test-model",
            "message": {
                "role": "assistant",
                "content": "",
                "tool_calls": [{
                    "function": {
                        "name": "file_read",
                        "arguments": { "path": "/nonexistent/dispatch-owner-probe.txt" }
                    }
                }],
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

    // Second round trip (and any beyond): plain text, so the loop terminates.
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
        .with_priority(2)
        .mount(&llm)
        .await;

    let uri = llm.uri();
    let config_uri = uri.clone();

    let mut user_configs = Vec::new();
    let mut auth_users = Vec::new();
    for (name, role, key) in [("Alice", "user", ALICE_KEY), ("Admin", "admin", ADMIN_KEY)] {
        let hash = librefang_api::password_hash::hash_password(key).expect("hash test key");
        user_configs.push(UserConfig {
            name: name.to_string(),
            role: role.to_string(),
            api_key_hash: Some(hash.clone()),
            ..Default::default()
        });
        auth_users.push(middleware::ApiUserAuth {
            name: name.to_string(),
            role: KernelUserRole::from_str_role(role),
            api_key_hash: hash,
            user_id: UserId::from_name(name),
        });
    }

    let test = TestAppState::with_builder(
        MockKernelBuilder::new()
            .with_config(move |cfg| {
                cfg.api_key = MASTER_KEY.to_string();
                cfg.users = user_configs;
                cfg.default_model.provider = "ollama".to_string();
                cfg.default_model.model = "test-model".to_string();
                cfg.default_model.api_key_env = "OLLAMA_API_KEY".to_string();
                cfg.default_model.base_url = Some(config_uri);
                // Keep the turn to the two provider round trips above.
                // Proactive memory would add retrieval / extraction calls that are orthogonal to identity threading.
                cfg.proactive_memory.enabled = false;
            })
            .with_catalog_seed(ollama_stub_catalog(&uri)),
    )
    .with_api_key(MASTER_KEY)
    .with_user_api_keys(auth_users);

    let (state, tmp, _) = test.into_parts();
    state.kernel.clone().set_self_handle();

    let auth_state = middleware::AuthState {
        api_key_lock: state.api_key_lock.clone(),
        master_key: state.master_key.clone(),
        active_sessions: state.active_sessions.clone(),
        dashboard_auth_enabled: state.dashboard_auth_enabled.clone(),
        user_api_keys: state.user_api_keys.clone(),
        require_auth_for_reads: true,
        allow_no_auth: false,
        audit_log: Some(state.kernel.audit().clone()),
        a2a_agents_path: None,
    };

    let app = axum::Router::new()
        .nest("/api", routes::agents::router())
        .layer(axum::middleware::from_fn_with_state(
            auth_state,
            middleware::auth,
        ))
        .with_state(state.clone());

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind ephemeral port");
    let addr = listener.local_addr().expect("local addr");
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });

    Harness {
        base_url: format!("http://{addr}"),
        state,
        llm,
        _tmp: tmp,
    }
}

// ---------------------------------------------------------------------------
// Acceptance test (#7744)
// ---------------------------------------------------------------------------

/// Spawns `manifest_toml` as the admin and returns the new agent's id.
async fn spawn_agent(h: &Harness, client: &reqwest::Client, manifest_toml: &str) -> String {
    let spawn = client
        .post(format!("{}/api/agents", h.base_url))
        .header("authorization", format!("Bearer {ADMIN_KEY}"))
        .json(&serde_json::json!({ "manifest_toml": manifest_toml }))
        .send()
        .await
        .expect("spawn request");
    assert_eq!(
        spawn.status().as_u16(),
        201,
        "agent spawn must succeed before the turn can be driven"
    );
    let spawn_body: serde_json::Value = spawn.json().await.expect("spawn body");
    spawn_body["agent_id"]
        .as_str()
        .expect("agent_id in spawn body")
        .to_string()
}

/// Positive control, independent of the identity plumbing: two provider round trips prove the tool actually ran (the opening call returned `tool_calls`; the second carried the tool result).
/// Without it, a missing identity record could just mean "dispatch was never reached" rather than "the owner was dropped on the way".
async fn assert_tool_round_trip(h: &Harness) {
    let llm_calls = h
        .llm
        .received_requests()
        .await
        .expect("wiremock records requests")
        .len();
    assert!(
        llm_calls >= 2,
        "expected >= 2 provider round trips (tool request + tool result), got {llm_calls} — the turn never reached tool dispatch, so this test cannot say anything about identity"
    );
}

/// `POST /api/agents/{id}/message` with user A's bearer and `"sender_id": "B"` in the body must reach tool dispatch carrying A, not B.
///
/// Both fields end up as A: `owner` from the bearer, and `sender_id` because `request_sender_context` pins a non-Admin caller to their own identity when the body asserts one that does not resolve back to them.
/// The body's "B" is what must not survive — see the module header for why that is the claim, and why it is not a test that the two fields can be told apart.
#[tokio::test(flavor = "multi_thread")]
async fn authenticated_owner_reaches_tool_dispatch_and_body_sender_id_cannot_forge_it() {
    let _serial = SERIAL.lock().await;
    let capture = install_capture();
    let h = boot().await;
    let client = reqwest::Client::new();

    // Agent creation is an Admin action; Alice cannot do it. Irrelevant to what is under test — the turn below is Alice's.
    let agent_id = spawn_agent(&h, &client, TEST_MANIFEST).await;
    let mark = capture.mark();

    // The attack: Alice's bearer, but a `sender_id` naming somebody else.
    let resp = client
        .post(format!("{}/api/agents/{}/message", h.base_url, agent_id))
        .header("authorization", format!("Bearer {ALICE_KEY}"))
        .json(&serde_json::json!({
            "message": "read a file for me",
            "sender_id": SPOOFED_SENDER_ID,
        }))
        .send()
        .await
        .expect("message request");
    let status = resp.status().as_u16();
    let body: serde_json::Value = resp.json().await.unwrap_or(serde_json::Value::Null);
    assert_eq!(
        status, 200,
        "the turn must complete against the stub provider — body={body:?}"
    );

    assert_tool_round_trip(&h).await;

    let file_read = capture.since(mark, "file_read");
    assert!(
        !file_read.is_empty(),
        "tool dispatch emitted no `{TOOL_IDENTITY_TARGET}` record for `file_read`; the authenticated owner never reached the dispatcher"
    );

    let expected_owner = Principal::user(UserId::from_name("Alice")).to_string();
    for record in &file_read {
        assert_eq!(
            record.get("owner").map(String::as_str),
            Some(expected_owner.as_str()),
            "dispatch must carry the bearer-authenticated owner (Alice) as a `user:` principal, not the body-supplied sender_id — record={record:?}"
        );
        // sender_id at dispatch is the SenderContext.user_id, which `request_sender_context` pins to the authenticated caller when a non-Admin tries to assert a foreign identity.
        // The body's "B" is correctly overridden to "Alice".
        assert_eq!(
            record.get("sender_id").map(String::as_str),
            Some("Alice"),
            "sender_id must reflect the pinned authenticated identity — record={record:?}"
        );
    }
}

/// An unattended turn — no bearer caller, as for a cron fire or an autonomous tick — on an agent whose manifest declares `owner = "group:oncall"` must reach dispatch carrying that group principal.
///
/// This pins the record for the path where an operator most needs it: narrowing the principal to a user id would log a group owner as no owner at all, indistinguishable from a turn with no principal.
#[tokio::test(flavor = "multi_thread")]
async fn manifest_group_owner_reaches_tool_dispatch_on_an_unattended_turn() {
    let _serial = SERIAL.lock().await;
    let capture = install_capture();
    let h = boot().await;
    let client = reqwest::Client::new();

    let agent_id = spawn_agent(&h, &client, GROUP_OWNED_MANIFEST).await;
    let agent_id: librefang_types::agent::AgentId = agent_id.parse().expect("agent id parses");
    let mark = capture.mark();

    // `send_message` is the kernel entry point cron, triggers and autonomous ticks use: it carries no authenticated owner, so the manifest `owner` is what resolves.
    h.state
        .kernel
        .send_message(agent_id, "read a file for me")
        .await
        .expect("unattended turn completes against the stub provider");

    assert_tool_round_trip(&h).await;

    let file_read = capture.since(mark, "file_read");
    assert!(
        !file_read.is_empty(),
        "tool dispatch emitted no `{TOOL_IDENTITY_TARGET}` record for `file_read` on the unattended turn"
    );

    let expected_owner = Principal::group_named(ONCALL_GROUP).to_string();
    for record in &file_read {
        assert_eq!(
            record.get("owner").map(String::as_str),
            Some(expected_owner.as_str()),
            "dispatch must carry the manifest's group principal, not drop it — record={record:?}"
        );
        assert_eq!(
            record.get("sender_id"),
            None,
            "an unattended turn has no platform sender — record={record:?}"
        );
    }
}
