//! Regression test for the 2026-05-20 cross-chat attachment leak.
//!
//! **Incident.** WhatsApp DM (chat 121043) at 2026-05-20T08:43:13Z: the
//! owner sent a private Amazon-order screenshot ("metro laser, 27,98€")
//! plus the caption "segna spesa in Shopping". One hour later the agent
//! replied in a *public* WhatsApp group ("Non perdiamoci 💻", chat
//! 120957) with a message containing those same private order details
//! verbatim. A msgpack dump of the group session's `sessions.messages`
//! showed an `Image (image/jpeg) previously processed` placeholder at
//! the same second as the DM inbound — i.e. the DM image had been
//! persisted into the group session's history.
//!
//! **Root cause.** `SessionWriter::inject_attachment_blocks` (kernel
//! impl in `kernel/handles/session_writer.rs`) took only
//! `(agent_id, blocks)` and wrote into `entry.session_id` — the
//! agent's persistent registry session. For a chat agent whose
//! `session_mode = "persistent"` keeps a *group* chat hot,
//! `entry.session_id` was that warm group session. Any DM-inbound image
//! therefore landed in the group session instead of the DM session that
//! the text part of the same request would derive via
//! `SessionId::for_sender_scope(agent, "whatsapp", "121043@…")`.
//!
//! **Fix.** The trait now requires an explicit `session_id` parameter
//! and the API call site (`inject_attachments_into_session` in
//! `routes/agents.rs`) derives the same session id the kernel's
//! `send_message_*` would, using the per-request `SenderContext` and
//! `session_id_override`.
//!
//! This test pins the kernel-side behaviour: passing
//! `(agent_id, X, blocks)` writes into session `X`, not into
//! `entry.session_id`. If a future refactor reintroduces the
//! "fall back to entry.session_id" behaviour the assertion will fail
//! and the leak will not regress silently.

use librefang_kernel::kernel_handle::SessionWriter as _;
use librefang_kernel::KernelApi;
use librefang_testing::MockKernelBuilder;
use librefang_types::agent::{AgentManifest, SessionId};
use librefang_types::message::ContentBlock;
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

fn boot_kernel() -> (Arc<librefang_kernel::LibreFangKernel>, tempfile::TempDir) {
    MockKernelBuilder::new()
        .with_config(|c| {
            c.default_model.provider = "ollama".to_string();
            c.default_model.model = "test".to_string();
            c.default_model.api_key_env = "OLLAMA_API_KEY".to_string();
        })
        .build()
}

fn dummy_image_block() -> ContentBlock {
    ContentBlock::Image {
        media_type: "image/png".to_string(),
        // 1x1 transparent PNG, plenty for an assertion that the block
        // shows up in the right session's history.
        data: "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNkYAAAAAYAAjCB0C8AAAAASUVORK5CYII=".to_string(),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn inject_attachment_blocks_writes_into_explicit_session_not_registry_default() {
    let (kernel, _tmp) = boot_kernel();
    let agent_id = spawn_test_agent(&kernel, "iso-agent-attach-1");

    // The agent's persistent registry session — what the buggy
    // pre-fix implementation wrote to. Snapshot it before the call.
    let entry_session_id = kernel
        .agent_registry()
        .get(agent_id)
        .expect("agent must exist post-spawn")
        .session_id;

    // The session we ACTUALLY want the image to land in — derived the
    // same way the API call site derives it from the per-chat
    // `SenderContext` (WhatsApp DM chat 121043).
    let dm_session_id =
        SessionId::for_sender_scope(agent_id, "whatsapp", Some("121043@s.whatsapp.net"));

    // Sanity: these must differ, otherwise the test cannot distinguish
    // "wrote into correct session" from "wrote into entry.session_id by
    // accident".
    assert_ne!(
        dm_session_id, entry_session_id,
        "test precondition: DM-derived session id must differ from entry.session_id"
    );

    let blocks = vec![dummy_image_block()];
    kernel.inject_attachment_blocks(agent_id, dm_session_id, blocks);

    // The DM session must now exist and contain the image.
    let dm_session = kernel
        .memory_substrate()
        .get_session(dm_session_id)
        .expect("substrate get_session")
        .expect("DM session must be created by the attachment write");
    assert_eq!(
        dm_session.messages.len(),
        1,
        "DM session should contain exactly the one injected attachment message"
    );

    // The registry session must NOT have been touched.
    let entry_session = kernel
        .memory_substrate()
        .get_session(entry_session_id)
        .expect("substrate get_session");
    let leaked = entry_session
        .as_ref()
        .map(|s| s.messages.len())
        .unwrap_or(0);
    assert_eq!(
        leaked, 0,
        "CROSS-CHAT LEAK GUARD: entry.session_id must have ZERO messages after a DM-scoped \
         attachment write — the 2026-05-20 incident wrote here instead. Found {leaked} message(s)."
    );
}

/// Second scenario: two different chats in sequence. Asserts that two
/// `inject_attachment_blocks` calls with different explicit session ids
/// produce two independent sessions, each containing exactly its own
/// image. This is the multi-chat shape of the production leak (a DM
/// inbound followed by a group inbound on the same warm agent).
#[tokio::test(flavor = "multi_thread")]
async fn inject_attachment_blocks_isolates_two_chat_scopes() {
    let (kernel, _tmp) = boot_kernel();
    let agent_id = spawn_test_agent(&kernel, "iso-agent-attach-2");

    let dm_session_id =
        SessionId::for_sender_scope(agent_id, "whatsapp", Some("111111111@s.whatsapp.net"));
    let group_session_id =
        SessionId::for_sender_scope(agent_id, "whatsapp", Some("222222222@g.us"));
    assert_ne!(dm_session_id, group_session_id);

    kernel.inject_attachment_blocks(agent_id, dm_session_id, vec![dummy_image_block()]);
    kernel.inject_attachment_blocks(agent_id, group_session_id, vec![dummy_image_block()]);

    let dm = kernel
        .memory_substrate()
        .get_session(dm_session_id)
        .expect("substrate get_session")
        .expect("DM session must exist");
    let group = kernel
        .memory_substrate()
        .get_session(group_session_id)
        .expect("substrate get_session")
        .expect("group session must exist");

    assert_eq!(
        dm.messages.len(),
        1,
        "DM session must hold exactly its image"
    );
    assert_eq!(
        group.messages.len(),
        1,
        "group session must hold exactly its image"
    );
    // The two sessions must be physically distinct in the substrate.
    assert_ne!(dm.id, group.id);
}
