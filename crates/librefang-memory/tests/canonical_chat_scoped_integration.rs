//! Integration regression test for chat-scoped canonical context filtering.
//!
//! Guards the fix in `session.rs` that tags each `CanonicalEntry` with the
//! originating `SessionId` and filters at read time. Before this fix, every
//! WhatsApp DM and group sharing the same agent saw each other's history
//! injected into the LLM prompt — a private chat could leak group messages
//! and vice versa.
//!
//! The test exercises the full append → load → context roundtrip via the
//! crate's public API, which is what the kernel actually calls.

use librefang_memory::migration::run_migrations;
use librefang_memory::session::SessionStore;
use librefang_types::agent::{AgentId, SessionId};
use librefang_types::message::MessageContent;
use librefang_types::message::{Message, Role};
use r2d2::Pool;
use r2d2_sqlite::SqliteConnectionManager;

fn setup() -> SessionStore {
    let pool = Pool::builder()
        .max_size(1)
        .build(SqliteConnectionManager::memory())
        .expect("build pool");
    run_migrations(&pool.get().expect("get conn")).expect("run migrations");
    SessionStore::new(pool)
}

fn user_msg(text: &str) -> Message {
    Message {
        role: Role::User,
        content: MessageContent::Text(text.to_string()),
        pinned: false,
        timestamp: None,
    }
}

#[test]
fn canonical_context_isolates_two_whatsapp_chats_for_same_agent() {
    let store = setup();
    let agent = AgentId::new();

    // Two chats — Jessica's DM and a group containing Jessica — produce
    // two distinct SessionIds via the channel-derivation function the
    // kernel uses on every inbound message.
    let session_dm = SessionId::for_channel(agent, "whatsapp:393331111111@s.whatsapp.net");
    let session_group = SessionId::for_channel(agent, "whatsapp:120363111111111111@g.us");

    assert_ne!(
        session_dm, session_group,
        "different chats must derive different sessions"
    );

    store
        .append_canonical(agent, &[user_msg("dm-1")], None, Some(session_dm))
        .expect("append dm-1");
    store
        .append_canonical(agent, &[user_msg("group-1")], None, Some(session_group))
        .expect("append group-1");
    store
        .append_canonical(agent, &[user_msg("dm-2")], None, Some(session_dm))
        .expect("append dm-2");

    // Filtering by the DM session must surface only the DM messages, never
    // the group message that arrived between them.
    let (_summary, dm_recent) = store
        .canonical_context(agent, Some(session_dm), None)
        .expect("canonical_context dm");
    let dm_texts: Vec<String> = dm_recent
        .iter()
        .filter_map(|m| match &m.content {
            MessageContent::Text(s) => Some(s.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(
        dm_texts,
        vec!["dm-1", "dm-2"],
        "DM context must NOT include group-1"
    );

    // Filtering by the group session must surface only the group messages.
    let (_summary, group_recent) = store
        .canonical_context(agent, Some(session_group), None)
        .expect("canonical_context group");
    let group_texts: Vec<String> = group_recent
        .iter()
        .filter_map(|m| match &m.content {
            MessageContent::Text(s) => Some(s.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(
        group_texts,
        vec!["group-1"],
        "group context must NOT include dm-1 or dm-2"
    );
}

#[test]
fn canonical_context_unfiltered_returns_all_for_backward_compat() {
    let store = setup();
    let agent = AgentId::new();
    let session_a = SessionId::for_channel(agent, "whatsapp:393331111111@s.whatsapp.net");
    let session_b = SessionId::for_channel(agent, "telegram:42");

    store
        .append_canonical(agent, &[user_msg("a-1")], None, Some(session_a))
        .unwrap();
    store
        .append_canonical(agent, &[user_msg("b-1")], None, Some(session_b))
        .unwrap();

    // Calling with session_id = None returns everything — preserves the
    // original cross-channel canonical-memory semantics for callers that
    // haven't adopted the per-session tag yet.
    let (_summary, all) = store.canonical_context(agent, None, None).unwrap();
    let texts: Vec<String> = all
        .iter()
        .filter_map(|m| match &m.content {
            MessageContent::Text(s) => Some(s.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(texts, vec!["a-1", "b-1"]);
}
