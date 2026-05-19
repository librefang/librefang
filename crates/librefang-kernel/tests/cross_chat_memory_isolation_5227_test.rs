//! Regression test for #5227 — cross-chat memory bleed via
//! `auto_memorize` + `auto_retrieve`.
//!
//! Reproduces the bug scenario at the kernel boundary:
//!   1. The same physical user sends a message in a WhatsApp group chat
//!      (high-level: agent extracts a memory tagged with the group's
//!      `(channel, chat)` scope).
//!   2. Within minutes, the same user sends a message in a 1:1 DM with
//!      the agent. Pre-#5227, the agent's `auto_retrieve` step pulled
//!      back the memory from the group chat (filter was
//!      `(agent_id, peer_id)`-only) and surfaced it in the DM prompt,
//!      contaminating the reply with content the user never typed in
//!      that channel.
//!
//! What the test asserts at the kernel-public API boundary:
//!   - `SessionId::for_sender_scope` produces **distinct** session IDs
//!     for the DM vs the group of the same peer — confirming the
//!     kernel-side session isolation is intact and the bleed had to
//!     come from a layer above sessions.
//!   - `ProactiveMemoryHooks::auto_memorize` invoked through the
//!     `proactive_memory_store()` accessor stamps each memory it writes
//!     with the originating `chat_scope` (the new
//!     `CHAT_SCOPE_METADATA_KEY` entry must equal the channel string
//!     the caller passed in).
//!   - When `chat_scope = None` (dashboard / direct API), the recall
//!     side is a no-op and pre-fix data remains visible — backward
//!     compatibility guarantee.
//!
//! The full cross-chat read filter (a session-level memory tagged for
//! chat A must be blocked from a recall in chat B) is exercised in
//! `librefang-memory::proactive::tests::test_auto_retrieve_cross_chat_isolation_5227`
//! against a freshly built substrate. That test sits closer to the
//! storage layer and can write session-level memories synthetically
//! (the rule-based `DefaultMemoryExtractor` available here lifts every
//! preference to `MemoryLevel::User`, which is exempt from the filter
//! by design, so the rule extractor cannot drive the session-level
//! branch from this side of the crate boundary).
//!
//! Together the two test files pin the fix end-to-end without needing
//! a real LLM at the integration boundary.

use librefang_kernel::KernelApi;
use librefang_memory::ProactiveMemoryStore;
use librefang_testing::MockKernelBuilder;
use librefang_types::agent::{AgentManifest, SessionId};
use librefang_types::memory::{ProactiveMemoryHooks, CHAT_SCOPE_METADATA_KEY};
use std::sync::Arc;

fn spawn_test_agent(
    kernel: &Arc<librefang_kernel::LibreFangKernel>,
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

/// Boot a kernel with the proactive memory subsystem at its defaults
/// (enabled, auto_memorize/auto_retrieve both on — matching the
/// production defaults under which #5227 was reported).
fn boot_kernel_with_proactive_memory() -> (Arc<librefang_kernel::LibreFangKernel>, tempfile::TempDir)
{
    MockKernelBuilder::new()
        .with_config(|c| {
            c.default_model.provider = "ollama".to_string();
            c.default_model.model = "test".to_string();
            c.default_model.api_key_env = "OLLAMA_API_KEY".to_string();
        })
        .build()
}

/// Build a standalone `ProactiveMemoryStore` sitting on top of the
/// kernel's substrate — boot defaults leave the kernel-owned slot at
/// `None` when no embedding driver is wired, but the storage layer
/// itself works fine. This mirrors what the runtime wraps when the
/// embedding driver IS present, just without the embedding step.
fn standalone_proactive_store(
    kernel: &Arc<librefang_kernel::LibreFangKernel>,
) -> Arc<ProactiveMemoryStore> {
    Arc::new(ProactiveMemoryStore::with_default_config(Arc::clone(
        kernel.memory_substrate(),
    )))
}

/// Reified channel labels matching what the WhatsApp gateway forwards
/// via `POST /api/agents/<id>/message`: the chat JID is baked into the
/// `channel_type` string (see `packages/whatsapp-gateway/lib/session-key.js`
/// `channelTypeForChat`).
const DM_CHANNEL: &str = "whatsapp:+15551234567@s.whatsapp.net";
const GROUP_CHANNEL: &str = "whatsapp:9999@g.us";
const SHARED_PEER: &str = "+15551234567";

#[tokio::test(flavor = "multi_thread")]
async fn kernel_resolves_distinct_sessions_for_dm_and_group_5227() {
    let (kernel, _tmp) = boot_kernel_with_proactive_memory();
    let agent_id = spawn_test_agent(&kernel, "iso-agent");

    // Session resolution mirrors `kernel::messaging::send_message_full`'s
    // channel branch — `for_sender_scope(agent, channel, None)` collapses
    // to `for_channel(agent, channel)` since the WhatsApp gateway packs
    // the chatJid into `channel_type` itself.
    let dm_sid = SessionId::for_sender_scope(agent_id, DM_CHANNEL, None);
    let group_sid = SessionId::for_sender_scope(agent_id, GROUP_CHANNEL, None);
    assert_ne!(
        dm_sid, group_sid,
        "kernel must derive distinct SessionIds for DM vs group with the same peer (#5227)"
    );

    // Cross-check: the same scope is deterministic across calls (UUID v5).
    assert_eq!(
        dm_sid,
        SessionId::for_sender_scope(agent_id, DM_CHANNEL, None),
        "for_sender_scope must be deterministic (UUID v5)"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn auto_memorize_stamps_chat_scope_metadata_5227() {
    let (kernel, _tmp) = boot_kernel_with_proactive_memory();
    let agent_id = spawn_test_agent(&kernel, "iso-agent-2");
    let store = standalone_proactive_store(&kernel);

    // Run auto_memorize with a chat scope — the new code path must
    // stamp `CHAT_SCOPE_METADATA_KEY` onto every stored memory.
    let group_extract = store
        .auto_memorize(
            &agent_id.0.to_string(),
            &[serde_json::json!({
                "role": "user",
                "content": "I prefer to ship Atlas by Friday"
            })],
            Some(SHARED_PEER),
            Some(GROUP_CHANNEL),
        )
        .await
        .expect("auto_memorize must succeed");
    assert!(
        group_extract.has_content,
        "DefaultMemoryExtractor must produce a memory from the preference pattern"
    );

    // The stamp must equal exactly the scope we passed in.
    let stored_scope = group_extract
        .memories
        .iter()
        .find_map(|m| {
            m.metadata
                .get(CHAT_SCOPE_METADATA_KEY)
                .and_then(|v| v.as_str())
                .map(String::from)
        })
        .expect("auto_memorize must stamp chat_scope onto every stored memory (#5227)");
    assert_eq!(
        stored_scope, GROUP_CHANNEL,
        "chat_scope stamp must match the originating channel"
    );

    // Negative — same call WITHOUT a chat scope must NOT leave a
    // `chat_scope` key on the stored memory (legacy / direct API path).
    let unscoped = store
        .auto_memorize(
            &agent_id.0.to_string(),
            &[serde_json::json!({
                "role": "user",
                "content": "I prefer dark mode"
            })],
            Some(SHARED_PEER),
            None,
        )
        .await
        .expect("auto_memorize must succeed");
    assert!(unscoped.has_content, "extractor must produce a memory");
    for m in &unscoped.memories {
        assert!(
            !m.metadata.contains_key(CHAT_SCOPE_METADATA_KEY),
            "no-scope writes must not stamp a chat_scope tag; got {:?}",
            m.metadata
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn unscoped_caller_preserves_legacy_recall_5227() {
    // Callers without channel context (dashboard, direct API, internal
    // tooling) pass `chat_scope = None` and must keep the pre-fix
    // recall behaviour: no filtering at all, every (agent, peer)
    // memory remains visible.
    let (kernel, _tmp) = boot_kernel_with_proactive_memory();
    let agent_id = spawn_test_agent(&kernel, "iso-agent-3");
    let store = standalone_proactive_store(&kernel);

    // Write a memory scoped to the DM channel.
    store
        .auto_memorize(
            &agent_id.0.to_string(),
            &[serde_json::json!({
                "role": "user",
                "content": "I prefer dark mode"
            })],
            Some(SHARED_PEER),
            Some(DM_CHANNEL),
        )
        .await
        .expect("auto_memorize must succeed");

    let unscoped = store
        .auto_retrieve(
            &agent_id.0.to_string(),
            "dark mode",
            Some(SHARED_PEER),
            None, // no chat scope — filter is a no-op
        )
        .await
        .expect("auto_retrieve must succeed");
    assert!(
        unscoped
            .iter()
            .any(|m| m.content.to_lowercase().contains("dark mode")),
        "no-scope recall must still find chat-scoped memories \
         (filter must be a no-op when caller has no channel context); got {:?}",
        unscoped.iter().map(|m| &m.content).collect::<Vec<_>>()
    );
}

/// #5227 follow-up — verify the chat-scope composition formula is
/// shared between `SessionId::for_sender_scope` (kernel-side session
/// isolation) and `compose_sender_scope` (memory-side `chat_scope`
/// stamp). If the two ever drift, a memory written under one chat
/// would leak into the SessionId-isolated history of the OTHER chat —
/// which is exactly the regression #5227 set out to close, but for
/// adapters that split `channel` from `chat_id` (Telegram / Slack /
/// Discord) instead of pre-qualifying like the WhatsApp gateway.
#[test]
fn compose_sender_scope_matches_for_sender_scope_formula_5227() {
    use librefang_types::agent::{compose_sender_scope, AgentId, SessionId};

    let agent = AgentId::new();

    // 1) WhatsApp-gateway shape — channel pre-qualified, chat_id None.
    //    Bug-for-bug compatible with the original PR test.
    {
        let dm_channel = "whatsapp:+15551234567@s.whatsapp.net";
        let scope = compose_sender_scope(dm_channel, None).expect("non-empty channel");
        assert_eq!(scope, dm_channel);
        let sid = SessionId::for_sender_scope(agent, dm_channel, None);
        assert_eq!(sid, SessionId::for_channel(agent, &scope));
    }

    // 2) Telegram / Slack / Discord shape — bare channel + chat_id.
    //    The formula MUST compose `"<channel>:<chat_id>"` so DM and
    //    group of the same user (which share `channel = "telegram"`
    //    but differ on chat_id) get distinct scopes.
    {
        let dm_scope = compose_sender_scope("telegram", Some("user-123")).unwrap();
        let group_scope = compose_sender_scope("telegram", Some("group-456")).unwrap();
        assert_ne!(
            dm_scope, group_scope,
            "Telegram DM and group of same channel must compose to distinct scopes"
        );
        assert_eq!(dm_scope, "telegram:user-123");
        assert_eq!(group_scope, "telegram:group-456");

        // SessionId derived from the same inputs must collapse to
        // for_channel(agent, scope) — proving the two consumers of
        // the formula agree.
        assert_eq!(
            SessionId::for_sender_scope(agent, "telegram", Some("user-123")),
            SessionId::for_channel(agent, &dm_scope),
        );
        assert_eq!(
            SessionId::for_sender_scope(agent, "telegram", Some("group-456")),
            SessionId::for_channel(agent, &group_scope),
        );
        assert_ne!(
            SessionId::for_sender_scope(agent, "telegram", Some("user-123")),
            SessionId::for_sender_scope(agent, "telegram", Some("group-456")),
            "kernel session isolation must distinguish telegram DM vs group of same user"
        );
    }

    // 3) Empty chat_id collapses to bare channel (matches the
    //    historical for_channel-only behaviour for adapters that
    //    don't populate chat_id at all).
    {
        let scope = compose_sender_scope("slack", Some("")).unwrap();
        assert_eq!(scope, "slack");
        assert_eq!(
            SessionId::for_sender_scope(agent, "slack", Some("")),
            SessionId::for_channel(agent, "slack"),
        );
    }

    // 4) Empty channel is `None` — the caller has no scope to compose
    //    and the kernel inject sites must not stamp `sender_chat_scope`.
    assert!(compose_sender_scope("", None).is_none());
    assert!(compose_sender_scope("", Some("anything")).is_none());
}

// The Session-level cross-chat-isolation behaviour for the
// Telegram-shape (bare channel + chat_id) lives in
// `librefang-memory::proactive::tests::
// test_auto_retrieve_cross_chat_isolation_telegram_shape_5227`
// — see that test for the end-to-end recall-side filter assertion.
// We can't run it from this crate without exposing the substrate's
// per-peer write API publicly (the proactive store's `semantic` is
// `pub(crate)`), so the unit-test above plus the in-memory crate-
// local regression keep the formula and the filter pinned together.
