//! Integration test: the auto-reply turn runs under the sender's identity (refs #8423).
//!
//! `KernelBridgeAdapter::check_auto_reply` launches a full agent turn — tools included — and it used to do so by reaching into `self.kernel` instead of going through this adapter's own channel-turn method.
//! That bypass dropped everything the method carries, not one argument: the sender identity the tool authorization gate reads, and the conversation's `/think` preference.
//! A turn that reaches the gate with no channel and no sender is answered by `guest_gate`: the read-only tools are allowed and everything else is queued as `NeedsApproval`, so an agent answered through auto-reply could execute nothing.
//!
//! The turn's identity is observable where the production instance was caught: `usage_events.channel`, stamped from the `SenderContext` (see `attribution_channel` in `kernel/agent_execution.rs`).
//! With no context the column stays NULL — the same value an unidentified turn hands the tool gate.
//!
//! The kernel points at a local OpenAI-compatible stub so the turn actually runs and records: the driverless default short-circuits at `!driver.is_configured()` before any usage row exists, and a real provider would make this a network test.
//!
//! Run: cargo test -p librefang-api --test auto_reply_sender_identity_test

use librefang_api::channel_bridge::KernelBridgeAdapter;
use librefang_channels::bridge::{AutoReplyOutcome, ChannelBridgeHandle};
use librefang_channels::types::{ConversationScope, SenderContext};
use librefang_kernel::KernelApi;
use librefang_kernel::LibreFangKernel;
use librefang_types::agent::{AgentId, AgentManifest};
use librefang_types::config::{AutoReplyConfig, DefaultModelConfig, KernelConfig, ThinkingConfig};
use std::sync::Arc;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const API_KEY_ENV: &str = "AUTO_REPLY_IDENTITY_TEST_KEY";

/// Mount an OpenAI-compatible chat completion that answers `content`.
async fn mount_completion(server: &MockServer, content: &str) {
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": "chatcmpl-test",
            "object": "chat.completion",
            "choices": [{
                "index": 0,
                "message": { "role": "assistant", "content": content },
                "finish_reason": "stop"
            }],
            "usage": { "prompt_tokens": 5, "completion_tokens": 3, "total_tokens": 8 }
        })))
        .mount(server)
        .await;
}

/// Every request body the provider stub has received so far.
///
/// A turn can reach the provider more than once — the loop's own call plus any
/// post-turn round-trip — so a test that wants to speak about "the turn's
/// request" has to delimit by index rather than take the last one.
async fn request_bodies(server: &MockServer) -> Vec<serde_json::Value> {
    server
        .received_requests()
        .await
        .expect("recorded requests")
        .iter()
        .map(|req| serde_json::from_slice(&req.body).expect("request body is JSON"))
        .collect()
}

/// Boot a kernel whose provider is a local OpenAI-compatible stub, with the
/// auto-reply engine on.
fn boot(server: &MockServer) -> (Arc<LibreFangKernel>, tempfile::TempDir) {
    // Read by the kernel when it builds the driver for the `openai` provider.
    std::env::set_var(API_KEY_ENV, "sk-test-auto-reply-identity");

    let tmp = tempfile::tempdir().expect("tempdir");
    let home = tmp.path().to_path_buf();
    std::fs::create_dir_all(home.join("data")).expect("data dir");

    let config = KernelConfig {
        home_dir: home.clone(),
        data_dir: home.join("data"),
        default_model: DefaultModelConfig {
            provider: "openai".to_string(),
            model: "gpt-4o-mini".to_string(),
            api_key_env: API_KEY_ENV.to_string(),
            base_url: Some(server.uri()),
            ..DefaultModelConfig::default()
        },
        auto_reply: AutoReplyConfig {
            enabled: true,
            ..AutoReplyConfig::default()
        },
        ..KernelConfig::default()
    };

    let kernel = Arc::new(LibreFangKernel::boot_with_config(config).expect("kernel boot"));
    kernel.clone().set_self_handle();
    (kernel, tmp)
}

/// Boot a driverless kernel with the auto-reply engine on.
///
/// A driverless turn answers `silent` with an empty response — the shape the
/// auto-reply arm has to turn into "no reply" rather than into an empty bubble,
/// and the only one that reaches the arm as an empty string: a provider that
/// returns empty content is recovered into a diagnostic notice instead.
fn boot_driverless() -> (Arc<LibreFangKernel>, tempfile::TempDir) {
    let tmp = tempfile::tempdir().expect("tempdir");
    let home = tmp.path().to_path_buf();
    std::fs::create_dir_all(home.join("data")).expect("data dir");

    let config = KernelConfig {
        home_dir: home.clone(),
        data_dir: home.join("data"),
        default_model: DefaultModelConfig::driverless(),
        auto_reply: AutoReplyConfig {
            enabled: true,
            ..AutoReplyConfig::default()
        },
        ..KernelConfig::default()
    };

    let kernel = Arc::new(LibreFangKernel::boot_with_config(config).expect("kernel boot"));
    kernel.clone().set_self_handle();
    (kernel, tmp)
}

fn spawn_agent(
    kernel: &Arc<LibreFangKernel>,
    name: &str,
    thinking: Option<ThinkingConfig>,
) -> AgentId {
    let manifest = AgentManifest {
        name: name.to_string(),
        source_template: None,
        thinking,
        ..AgentManifest::default()
    };
    kernel
        .spawn_agent_typed(manifest)
        .expect("spawn_agent_typed")
}

/// The `channel` column of the most recent usage row for `agent_id`.
fn latest_usage_channel(kernel: &Arc<LibreFangKernel>, agent_id: AgentId) -> Option<String> {
    let pool = kernel.memory_substrate().pool();
    let conn = pool.get().expect("pool connection");
    conn.query_row(
        "SELECT channel FROM usage_events WHERE agent_id = ?1 ORDER BY timestamp DESC LIMIT 1",
        rusqlite::params![agent_id.0.to_string()],
        |row| row.get::<_, Option<String>>(0),
    )
    .expect("the auto-reply turn must have recorded a usage row")
}

/// A Telegram DM as the channel bridge builds it: the platform id is both the
/// sender and the chat, and it is the pair the RBAC gate resolves bindings on.
fn telegram_dm() -> SenderContext {
    SenderContext {
        channel: "telegram".to_string(),
        user_id: "34387719".to_string(),
        chat_id: Some("34387719".to_string()),
        display_name: "user".to_string(),
        is_group: false,
        ..Default::default()
    }
}

/// The auto-reply turn must record the channel it arrived on.
///
/// Without it the tool authorization gate has no channel and no sender, resolves
/// the call as `guest_gate`, and turns every non-read-only tool into an approval
/// request.
#[tokio::test(flavor = "multi_thread")]
async fn auto_reply_turn_records_the_sender_channel() {
    let server = MockServer::start().await;
    mount_completion(&server, "hello").await;

    let (kernel, _tmp) = boot(&server);
    let agent_id = spawn_agent(&kernel, "auto-reply-agent", None);

    let adapter = KernelBridgeAdapter::new(kernel.clone() as Arc<dyn KernelApi>);

    adapter
        .check_auto_reply(agent_id, "hello", &telegram_dm())
        .await;

    assert_eq!(
        latest_usage_channel(&kernel, agent_id).as_deref(),
        Some("telegram"),
        "the auto-reply turn recorded no channel, so it ran without a SenderContext — the \
         tool authorization gate reads that as an unrecognised sender and forces every tool \
         outside the read-only allowlist into approval"
    );
}

/// The auto-reply turn must honour the conversation's `/think`.
///
/// The adapter stores that preference per `(agent, conversation)` and
/// `send_message_with_sender` is the only place that reads it. The agent is given
/// its own reasoning config so that "inherit" and "off" produce different
/// requests — with no config on either side the two are indistinguishable and
/// this test would pass by construction.
#[tokio::test(flavor = "multi_thread")]
async fn auto_reply_turn_honours_the_conversation_think_setting() {
    let server = MockServer::start().await;
    mount_completion(&server, "hello").await;

    let (kernel, _tmp) = boot(&server);
    let agent_id = spawn_agent(
        &kernel,
        "thinking-agent",
        Some(ThinkingConfig {
            budget_tokens: 4242,
            ..ThinkingConfig::default()
        }),
    );

    let adapter = KernelBridgeAdapter::new(kernel.clone() as Arc<dyn KernelApi>);

    // No `/think` has been issued in this conversation, so the agent's own
    // reasoning configuration stands.
    adapter
        .check_auto_reply(agent_id, "hello", &telegram_dm())
        .await;
    let after_first = request_bodies(&server).await;
    assert!(
        !after_first.is_empty(),
        "the turn never reached the provider, so nothing below is measured"
    );
    assert!(
        after_first
            .iter()
            .any(|b| b.get("reasoning_effort").is_some()),
        "control case: with no /think issued the manifest's reasoning config must reach the \
         provider, else this test cannot discriminate; bodies were {after_first:?}"
    );

    // The same conversation switches reasoning off.
    adapter
        .set_thinking(
            agent_id,
            false,
            &ConversationScope::from_sender(&telegram_dm()),
        )
        .await
        .expect("set_thinking");

    let before_second = after_first.len();
    adapter
        .check_auto_reply(agent_id, "hello again", &telegram_dm())
        .await;
    let all = request_bodies(&server).await;
    let second_turn = &all[before_second..];
    assert!(
        !second_turn.is_empty(),
        "the second turn never reached the provider, so the /think assertion below is vacuous"
    );
    assert!(
        second_turn
            .iter()
            .all(|b| b.get("reasoning_effort").is_none()),
        "the auto-reply turn ignored the conversation's /think: it still asked the provider to \
         reason; bodies were {second_turn:?}"
    );
}

/// A turn with nothing to say must produce no reply — and must still count as
/// a claimed message.
///
/// The auto-reply arm prefixes and sends whatever it is handed, and
/// `maybe_prefix_response` prefixes the empty string, so an empty reply reaches
/// the user as a prefix-only bubble whenever `prefix_agent_name` is configured.
///
/// The outcome must be `Fired(None)`, not `NotFired`: the turn *ran* and
/// consumed the message, so the bridge must not fall through and dispatch the
/// identical turn a second time in the same channel session.
#[tokio::test(flavor = "multi_thread")]
async fn auto_reply_silent_turn_returns_no_reply() {
    let (kernel, _tmp) = boot_driverless();
    let agent_id = spawn_agent(&kernel, "silent-agent", None);

    let adapter = KernelBridgeAdapter::new(kernel.clone() as Arc<dyn KernelApi>);

    let outcome = adapter
        .check_auto_reply(agent_id, "hello", &telegram_dm())
        .await;

    assert_eq!(
        outcome,
        AutoReplyOutcome::Fired(None),
        "a silent turn must be reported as a claimed message with no reply, not as \
         `NotFired`; `NotFired` makes the bridge re-dispatch the same turn in the same \
         channel session"
    );
}

/// A turn that fails is still a claimed message.
///
/// `Err` used to collapse into `None` alongside the silent case, so the bridge
/// read it as "auto-reply did not fire" and re-ran the identical turn — the
/// user message landed in the same channel session twice.
///
/// The failure travels with the error text so the bridge can surface it as an
/// error bubble and record a failed delivery, exactly as the ordinary path
/// does when the kernel rejects a turn.
#[tokio::test(flavor = "multi_thread")]
async fn failed_auto_reply_turn_reports_failed() {
    let (kernel, _tmp) = boot_driverless();

    // No agent is registered under this id, so the turn cannot start.
    let missing_agent = AgentId::new();
    let adapter = KernelBridgeAdapter::new(kernel.clone() as Arc<dyn KernelApi>);

    let outcome = adapter
        .check_auto_reply(missing_agent, "hello", &telegram_dm())
        .await;

    match outcome {
        AutoReplyOutcome::Failed(error) => assert!(
            !error.is_empty(),
            "the failure text must ride along so the bridge can deliver an error \
             bubble and record the failed delivery"
        ),
        other => panic!(
            "a failed auto-reply turn must be reported as claimed, not as `{other:?}`; \
             the bridge would otherwise re-run the identical turn and duplicate the \
             user message in the same channel session"
        ),
    }
}
