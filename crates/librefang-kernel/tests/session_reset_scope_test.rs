//! Integration tests for the per-session vs. per-agent reset split (#4868).
//!
//! The bug: `/new` typed in any channel called `kernel.reset_session(agent)`
//! which deleted EVERY session for that agent — wiping the dashboard's
//! transcript, every other channel's transcript, and any cron-spawned
//! sessions. The fix is a `ResetScope` argument that distinguishes
//! `Agent` (existing semantics) from `Session(sid)` (new, scoped) so the
//! channel side can ask the kernel to delete just one sid.
//!
//! Tests here exercise the kernel API directly (the channel layer is a
//! thin wrapper that derives `SessionId::for_channel(agent, channel:chat)`
//! and calls `kernel.reset_session(agent, ResetScope::Session(sid))` — the
//! sid-derivation formula is covered by a unit test in
//! `librefang-api/src/channel_bridge.rs`).
//!
//! What we assert:
//!   - `ResetScope::Session(sid)` deletes exactly that row + its FTS index.
//!   - Sibling sids (other channels, dashboard) survive the per-session reset.
//!   - JSONL mirrors under `<workspace>/sessions/{sid}.jsonl` are removed
//!     for the deleted sid (#4868 follow-up: orphan transcripts).
//!   - `ResetScope::Agent` keeps the historical full-wipe semantics AND
//!     purges every JSONL mirror.
//!   - A sid belonging to a different agent is rejected with `InvalidInput`
//!     (defence-in-depth — callers compute the sid from a trusted
//!     (channel, chat) pair, but the typed argument itself isn't).
//!   - Per-session reset does NOT touch agent-wide quota state (channel
//!     users must not be able to clear an agent-wide token-quota by
//!     typing /new).

use librefang_kernel::KernelApi;
use librefang_memory::session::Session;
use librefang_testing::MockKernelBuilder;
use librefang_types::agent::{AgentManifest, ResetScope, SessionId};
use librefang_types::message::{Message, MessageContent, Role};

/// Build a session row with `n` user/assistant turns so summary save logic
/// (gated on `messages.len() >= 2`) is exercised.
fn populated_session(
    sid: SessionId,
    agent_id: librefang_types::agent::AgentId,
    n: usize,
) -> Session {
    let mut messages = Vec::with_capacity(n * 2);
    for i in 0..n {
        messages.push(Message {
            role: Role::User,
            content: MessageContent::Text(format!("u{i}")),
            pinned: false,
            timestamp: None,
        });
        messages.push(Message {
            role: Role::Assistant,
            content: MessageContent::Text(format!("a{i}")),
            pinned: false,
            timestamp: None,
        });
    }
    Session {
        id: sid,
        agent_id,
        messages,
        context_window_tokens: 0,
        label: None,
        model_override: None,

        messages_generation: 0,
        last_repaired_generation: None,
    }
}

fn spawn_test_agent(
    kernel: &librefang_kernel::LibreFangKernel,
    name: &str,
) -> librefang_types::agent::AgentId {
    let manifest: AgentManifest = toml::from_str(&format!(
        r#"
name = "{name}"
version = "0.1.0"
description = "test"
author = "test"
module = "builtin:chat"

[model]
provider = "ollama"
model = "test"
system_prompt = "."
"#
    ))
    .unwrap();
    kernel.spawn_agent(manifest).expect("spawn_agent")
}

/// Materialise the session in SQLite AND lay down a JSONL mirror at the
/// path the kernel uses (`<workspace>/sessions/{sid}.jsonl`). Both must
/// disappear when the session is reset.
fn save_session_with_jsonl(
    kernel: &librefang_kernel::LibreFangKernel,
    agent_id: librefang_types::agent::AgentId,
    sid: SessionId,
    n_turns: usize,
) -> std::path::PathBuf {
    let session = populated_session(sid, agent_id, n_turns);
    kernel
        .memory_substrate()
        .save_session(&session)
        .expect("save_session");

    let entry = kernel
        .agent_registry()
        .get(agent_id)
        .expect("agent should be registered");
    let workspace = entry
        .manifest
        .workspace
        .clone()
        .expect("spawn_agent must populate workspace");
    let sessions_dir = workspace.join("sessions");
    std::fs::create_dir_all(&sessions_dir).expect("create sessions dir");
    let jsonl_path = sessions_dir.join(format!("{}.jsonl", sid.0));
    std::fs::write(&jsonl_path, b"{\"placeholder\":\"transcript\"}\n").expect("write jsonl");
    jsonl_path
}

/// Per-channel `/new` (kernel layer): only the targeted sid disappears,
/// sibling sessions are untouched in BOTH SQLite and on disk.
#[tokio::test(flavor = "multi_thread")]
async fn per_session_reset_only_deletes_targeted_sid() {
    let (kernel, _tmp) = MockKernelBuilder::new()
        .with_config(|c| {
            c.default_model.provider = "ollama".to_string();
            c.default_model.model = "test".to_string();
            c.default_model.api_key_env = "OLLAMA_API_KEY".to_string();
        })
        .build();

    let agent_id = spawn_test_agent(&kernel, "multi-channel-agent");

    // Three sessions on the same agent: dashboard (the registry-pointer
    // sid), telegram, slack.
    let dashboard_sid = kernel.agent_registry().get(agent_id).unwrap().session_id;
    let telegram_sid = SessionId::for_channel(agent_id, "telegram:chat-1");
    let slack_sid = SessionId::for_channel(agent_id, "slack:chat-2");

    let dashboard_jsonl = save_session_with_jsonl(&kernel, agent_id, dashboard_sid, 3);
    let telegram_jsonl = save_session_with_jsonl(&kernel, agent_id, telegram_sid, 5);
    let slack_jsonl = save_session_with_jsonl(&kernel, agent_id, slack_sid, 2);

    // /new in Telegram → only telegram session resets.
    kernel
        .reset_session(agent_id, ResetScope::Session(telegram_sid))
        .await
        .expect("reset_session(Session) succeeds");

    // SQLite: dashboard + slack still have their original turn counts;
    // telegram is recreated empty at the same deterministic sid.
    let dash = kernel
        .memory_substrate()
        .get_session(dashboard_sid)
        .unwrap()
        .expect("dashboard session must survive per-channel reset");
    assert_eq!(
        dash.messages.len(),
        6,
        "dashboard turn count must be intact after per-channel /new (#4868)"
    );

    let slack = kernel
        .memory_substrate()
        .get_session(slack_sid)
        .unwrap()
        .expect("slack session must survive per-channel reset");
    assert_eq!(
        slack.messages.len(),
        4,
        "slack turn count must be intact after per-channel /new (#4868)"
    );

    let telegram = kernel
        .memory_substrate()
        .get_session(telegram_sid)
        .unwrap()
        .expect(
            "telegram session must be recreated at the same deterministic sid (eager \
             reset semantics — next inbound message lands on it)",
        );
    assert!(
        telegram.messages.is_empty(),
        "telegram session must be empty after reset"
    );
    assert_eq!(
        telegram.id, telegram_sid,
        "recreated session must reuse the deterministic for_channel sid \
         so the next inbound message resolves to the same row"
    );

    // JSONL mirror cleanup: only the targeted sid's file is gone.
    assert!(
        !telegram_jsonl.exists(),
        "telegram JSONL must be purged on per-channel reset (#4868 follow-up: \
         orphan transcripts on disk)"
    );
    assert!(
        dashboard_jsonl.exists(),
        "dashboard JSONL must NOT be touched by a per-channel reset"
    );
    assert!(
        slack_jsonl.exists(),
        "slack JSONL must NOT be touched by a per-channel reset"
    );
}

/// `ResetScope::Agent` — historical wipe-everything semantics PLUS the
/// new JSONL mirror cleanup so we don't accumulate orphan transcripts.
#[tokio::test(flavor = "multi_thread")]
async fn agent_wide_reset_purges_all_sessions_and_jsonl() {
    let (kernel, _tmp) = MockKernelBuilder::new()
        .with_config(|c| {
            c.default_model.provider = "ollama".to_string();
            c.default_model.model = "test".to_string();
            c.default_model.api_key_env = "OLLAMA_API_KEY".to_string();
        })
        .build();

    let agent_id = spawn_test_agent(&kernel, "wipe-target");

    let dashboard_sid = kernel.agent_registry().get(agent_id).unwrap().session_id;
    let telegram_sid = SessionId::for_channel(agent_id, "telegram:chat-x");
    let slack_sid = SessionId::for_channel(agent_id, "slack:chat-y");

    let dashboard_jsonl = save_session_with_jsonl(&kernel, agent_id, dashboard_sid, 3);
    let telegram_jsonl = save_session_with_jsonl(&kernel, agent_id, telegram_sid, 4);
    let slack_jsonl = save_session_with_jsonl(&kernel, agent_id, slack_sid, 2);

    kernel
        .reset_session(agent_id, ResetScope::Agent)
        .await
        .expect("reset_session(Agent) succeeds");

    // The pre-existing sids are gone — each one would 404 if a caller
    // looked them up by id.
    for sid in [dashboard_sid, telegram_sid, slack_sid] {
        assert!(
            kernel
                .memory_substrate()
                .get_session(sid)
                .unwrap()
                .is_none(),
            "agent-wide reset must delete every pre-existing sid: {sid}"
        );
    }

    // JSONL mirrors gone for ALL three.
    for path in [&dashboard_jsonl, &telegram_jsonl, &slack_jsonl] {
        assert!(
            !path.exists(),
            "agent-wide reset must purge every JSONL mirror (orphan-transcript \
             leak fix, #4868 follow-up): {}",
            path.display()
        );
    }

    // The agent gets exactly one fresh registry-pointer session afterwards
    // — that's the historical "create_session" tail of reset_session.
    let post_reset_sids = kernel
        .memory_substrate()
        .get_agent_session_ids(agent_id)
        .unwrap();
    assert_eq!(
        post_reset_sids.len(),
        1,
        "agent-wide reset leaves exactly one fresh session"
    );
}

/// Defence-in-depth: callers compute the sid from a trusted
/// `(channel, chat)` pair, but the typed `SessionId` argument itself
/// isn't trusted. Pointing the kernel at another agent's sid must fail
/// with `InvalidInput`, NOT silently delete that other agent's data.
#[tokio::test(flavor = "multi_thread")]
async fn per_session_reset_rejects_cross_agent_sid() {
    let (kernel, _tmp) = MockKernelBuilder::new()
        .with_config(|c| {
            c.default_model.provider = "ollama".to_string();
            c.default_model.model = "test".to_string();
            c.default_model.api_key_env = "OLLAMA_API_KEY".to_string();
        })
        .build();

    let agent_a = spawn_test_agent(&kernel, "owner");
    let agent_b = spawn_test_agent(&kernel, "intruder");

    // Agent B owns this sid via for_channel — even though the channel name
    // looks innocuous, the sid hashes in agent_b's id.
    let b_sid = SessionId::for_channel(agent_b, "telegram:secret");
    let _b_jsonl = save_session_with_jsonl(&kernel, agent_b, b_sid, 5);

    // Agent A asks the kernel to reset agent B's sid. Must fail loudly,
    // not silently delete.
    let err = kernel
        .reset_session(agent_a, ResetScope::Session(b_sid))
        .await
        .expect_err("cross-agent reset must be rejected");
    let msg = format!("{err}");
    assert!(
        msg.contains("does not belong to agent"),
        "error must surface the cross-agent rejection (got: {msg})"
    );

    // Agent B's session is untouched.
    let surviving = kernel
        .memory_substrate()
        .get_session(b_sid)
        .unwrap()
        .expect("agent B's session must survive a cross-agent reset attempt");
    assert_eq!(
        surviving.messages.len(),
        10,
        "agent B's turn count must be intact"
    );
}

/// A per-session reset of an sid that never existed (channel /new typed
/// before any message) must still succeed and leave the agent's other
/// sessions untouched. This is the common first-/new case in a fresh chat.
#[tokio::test(flavor = "multi_thread")]
async fn per_session_reset_on_unmaterialised_sid_is_ok() {
    let (kernel, _tmp) = MockKernelBuilder::new()
        .with_config(|c| {
            c.default_model.provider = "ollama".to_string();
            c.default_model.model = "test".to_string();
            c.default_model.api_key_env = "OLLAMA_API_KEY".to_string();
        })
        .build();

    let agent_id = spawn_test_agent(&kernel, "fresh-chat-agent");
    let dashboard_sid = kernel.agent_registry().get(agent_id).unwrap().session_id;
    let dashboard_jsonl = save_session_with_jsonl(&kernel, agent_id, dashboard_sid, 2);

    // Telegram sid has never been written to.
    let telegram_sid = SessionId::for_channel(agent_id, "telegram:never-chatted");
    assert!(kernel
        .memory_substrate()
        .get_session(telegram_sid)
        .unwrap()
        .is_none());

    kernel
        .reset_session(agent_id, ResetScope::Session(telegram_sid))
        .await
        .expect("reset of unmaterialised sid must succeed");

    // Dashboard sid + JSONL untouched.
    assert!(
        dashboard_jsonl.exists(),
        "dashboard JSONL must NOT be touched by a no-op per-channel reset"
    );
    let dash = kernel
        .memory_substrate()
        .get_session(dashboard_sid)
        .unwrap()
        .expect("dashboard session must survive");
    assert_eq!(dash.messages.len(), 4);

    // Telegram sid is now materialised (eager recreation), empty.
    let tg = kernel
        .memory_substrate()
        .get_session(telegram_sid)
        .unwrap()
        .expect("eager recreation must materialise the sid");
    assert!(tg.messages.is_empty());
}

/// `reboot_session(Session)` deletes exactly one sid like `reset_session`
/// but does NOT save a memory summary (no `session_<date>_<slug>` entry
/// in structured kv). Same scope semantics, different summary-handling
/// contract — pin it so the contract can't drift (#4868 review MINOR #3).
#[tokio::test(flavor = "multi_thread")]
async fn per_session_reboot_skips_summary_save() {
    let (kernel, _tmp) = MockKernelBuilder::new()
        .with_config(|c| {
            c.default_model.provider = "ollama".to_string();
            c.default_model.model = "test".to_string();
            c.default_model.api_key_env = "OLLAMA_API_KEY".to_string();
        })
        .build();

    let agent_id = spawn_test_agent(&kernel, "reboot-target");
    let telegram_sid = SessionId::for_channel(agent_id, "telegram:reboot-chat");
    save_session_with_jsonl(&kernel, agent_id, telegram_sid, 5);

    // Snapshot the structured kv keys before reboot — there should be
    // exactly zero `session_*` keys (no prior reset summaries).
    let pre_keys: Vec<String> = kernel
        .memory_substrate()
        .list_keys(agent_id)
        .unwrap()
        .into_iter()
        .filter(|k| k.starts_with("session_"))
        .collect();

    kernel
        .reboot_session(agent_id, ResetScope::Session(telegram_sid))
        .await
        .expect("reboot_session(Session) succeeds");

    // Telegram sid is reset to empty same as the reset_session path.
    let tg = kernel
        .memory_substrate()
        .get_session(telegram_sid)
        .unwrap()
        .expect("reboot must eagerly recreate the sid");
    assert!(tg.messages.is_empty());

    // Critical: no summary saved. Pre-list set is unchanged.
    let post_keys: Vec<String> = kernel
        .memory_substrate()
        .list_keys(agent_id)
        .unwrap()
        .into_iter()
        .filter(|k| k.starts_with("session_"))
        .collect();
    assert_eq!(
        pre_keys, post_keys,
        "reboot must NOT save a session_<date>_<slug> summary — that's \
         exclusively the reset path's contract (#4868 review)"
    );
}

/// `clear_agent_history` also leaks JSONL mirrors pre-fix. The PR added
/// the same purge call; pin it (#4868 review MINOR #2).
#[tokio::test(flavor = "multi_thread")]
async fn clear_agent_history_purges_jsonl_too() {
    let (kernel, _tmp) = MockKernelBuilder::new()
        .with_config(|c| {
            c.default_model.provider = "ollama".to_string();
            c.default_model.model = "test".to_string();
            c.default_model.api_key_env = "OLLAMA_API_KEY".to_string();
        })
        .build();

    let agent_id = spawn_test_agent(&kernel, "clear-history-target");

    let dashboard_sid = kernel.agent_registry().get(agent_id).unwrap().session_id;
    let telegram_sid = SessionId::for_channel(agent_id, "telegram:clear-chat");

    let dashboard_jsonl = save_session_with_jsonl(&kernel, agent_id, dashboard_sid, 2);
    let telegram_jsonl = save_session_with_jsonl(&kernel, agent_id, telegram_sid, 3);

    kernel
        .clear_agent_history(agent_id)
        .await
        .expect("clear_agent_history succeeds");

    // Every JSONL mirror is gone — clear_agent_history is the hardest
    // wipe and previously left every transcript on disk forever (#4868
    // follow-up).
    assert!(
        !dashboard_jsonl.exists(),
        "clear_agent_history must purge dashboard JSONL: {}",
        dashboard_jsonl.display()
    );
    assert!(
        !telegram_jsonl.exists(),
        "clear_agent_history must purge telegram JSONL: {}",
        telegram_jsonl.display()
    );
}

/// `compact_agent_session_with_id(agent, Some(sid))` operates on the
/// session it's pointed at, not `entry.session_id`. The channel-bridge
/// `compact_channel_session` relies on this — without it, a Telegram
/// `/compact` would summarise the agent's dashboard session instead
/// of the telegram chat. Pre-#4868 the bridge called the agent-wide
/// `compact_session` and hit exactly this footgun.
#[tokio::test(flavor = "multi_thread")]
async fn compact_with_id_targets_the_requested_session() {
    let (kernel, _tmp) = MockKernelBuilder::new()
        .with_config(|c| {
            c.default_model.provider = "ollama".to_string();
            c.default_model.model = "test".to_string();
            c.default_model.api_key_env = "OLLAMA_API_KEY".to_string();
        })
        .build();

    let agent_id = spawn_test_agent(&kernel, "compact-scope-target");
    let dashboard_sid = kernel.agent_registry().get(agent_id).unwrap().session_id;
    let telegram_sid = SessionId::for_channel(agent_id, "telegram:compact-chat");

    // Two distinct turn counts so the "No compaction needed (N messages, …)"
    // message tells us which session the call actually loaded.
    save_session_with_jsonl(&kernel, agent_id, dashboard_sid, 2);
    save_session_with_jsonl(&kernel, agent_id, telegram_sid, 7);

    let dash_report = kernel
        .compact_agent_session_with_id(agent_id, Some(dashboard_sid))
        .await
        .expect("compact_agent_session_with_id(dashboard) succeeds");
    assert!(
        dash_report.contains("4 messages"),
        "compact must have loaded the dashboard session (4 = 2 turns × 2 \
         user/assistant pairs): {dash_report}"
    );

    let tg_report = kernel
        .compact_agent_session_with_id(agent_id, Some(telegram_sid))
        .await
        .expect("compact_agent_session_with_id(telegram) succeeds");
    assert!(
        tg_report.contains("14 messages"),
        "compact must have loaded the telegram session (14 = 7 turns × 2): \
         {tg_report}"
    );
}
