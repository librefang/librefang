//! Regression test for the TUI's throwaway-runtime session-summary loss.
//!
//! `reset_session` (`/new`) is async, but the summary write it triggers is **detached**: `save_session_summary` does `Handle::try_current()` + `handle.spawn(...)` so the aux-LLM digest happens off the reset's return path.
//! That is correct on the daemon, whose runtime outlives every request.
//!
//! The TUI drove `reset_session` with a per-call `tokio::runtime::Runtime::new()` and dropped it as soon as `block_on` returned.
//! Dropping a runtime aborts its tasks, so a summary write still in flight was killed — no panic, no warning, just a missing `session_<sid>` row.
//! The `try_current()` branch was even *taken*, which is why "no warning in the log" never surfaced it.
//!
//! **Whether the row is actually lost depends on config**, which is what makes this bug so quiet.
//! `build_session_summary` returns early with a trivial digest when `[llm.auxiliary] session_summary` is unconfigured — that path has no `.await` before the write, so the task finishes inside the scheduler window `reset_session`'s remaining async work provides, and the row lands even on a throwaway runtime.
//! Configure an aux chain and the same task now awaits an LLM round-trip, so the drop aborts it and the summary is gone.
//! The loss hits precisely the operators who paid for good summaries.
//!
//! A test that configures a real aux chain would need a live LLM, so what is pinned here is the fix's guarantee — a runtime that outlives the call lets the write complete regardless of which path it takes.
//! The abort semantics themselves are covered by `librefang-cli`'s `block_on_tui_detached_tasks_survive_the_call`, which uses an explicit await point and fails when the shared runtime is swapped for a throwaway.

use librefang_kernel::KernelApi;
use librefang_memory::session::Session;
use librefang_testing::MockKernelBuilder;
use librefang_types::agent::{AgentId, AgentManifest, ResetScope, SessionId};
use librefang_types::message::{Message, MessageContent, Role};
use std::sync::Arc;

fn spawn_test_agent(kernel: &librefang_kernel::LibreFangKernel, name: &str) -> AgentId {
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

/// A session with enough turns to clear the `messages.len() >= 2` gate that
/// guards the summary write.
fn populated_session(sid: SessionId, agent_id: AgentId, n: usize) -> Session {
    let mut messages = Vec::with_capacity(n * 2);
    for i in 0..n {
        messages.push(Message {
            role: Role::User,
            content: MessageContent::Text(format!("user turn {i}")),
            pinned: false,
            timestamp: None,
        });
        messages.push(Message {
            role: Role::Assistant,
            content: MessageContent::Text(format!("assistant turn {i}")),
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
        peer_id: None,
    }
}

/// Poll for the detached summary write to land.
/// The write is async by design, so a bounded wait is the honest check — a fixed sleep would either be flaky or slow.
fn wait_for_summary(
    kernel: &librefang_kernel::LibreFangKernel,
    agent_id: AgentId,
    sid: SessionId,
    timeout: std::time::Duration,
) -> bool {
    let key = format!("session_{}", sid.0);
    let deadline = std::time::Instant::now() + timeout;
    while std::time::Instant::now() < deadline {
        if matches!(
            kernel.memory_substrate().structured_get(agent_id, &key),
            Ok(Some(_))
        ) {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    false
}

/// Set up an agent with a populated session, exactly as `/new` would find it.
fn seed(kernel: &librefang_kernel::LibreFangKernel, name: &str) -> (AgentId, SessionId) {
    let agent_id = spawn_test_agent(kernel, name);
    let sid = kernel.agent_registry().get(agent_id).unwrap().session_id;
    kernel
        .memory_substrate()
        .save_session(&populated_session(sid, agent_id, 3))
        .expect("save_session");
    (agent_id, sid)
}

/// The fix: driving `reset_session` on a runtime that outlives the call lets the detached summary write finish.
///
/// This is the shape `tui::event::block_on_tui` produces — a process-lifetime runtime shared by every TUI operation.
#[test]
fn summary_survives_reset_on_a_long_lived_runtime() {
    let (kernel, _tmp) = MockKernelBuilder::new()
        .with_config(|c| {
            c.default_model.provider = "ollama".to_string();
            c.default_model.model = "test".to_string();
            c.default_model.api_key_env = "OLLAMA_API_KEY".to_string();
        })
        .build();
    let kernel = Arc::new(kernel);
    let (agent_id, sid) = seed(&kernel, "tui-shared-rt-agent");

    // The runtime is built once and outlives the block_on, so tasks detached
    // during the call keep running afterwards.
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("build runtime");
    rt.block_on(kernel.reset_session(agent_id, ResetScope::Agent))
        .expect("reset_session");

    assert!(
        wait_for_summary(&kernel, agent_id, sid, std::time::Duration::from_secs(10)),
        "session summary was never persisted even though the runtime outlived the reset"
    );
}

/// The reset itself must succeed on a throwaway runtime — only the *detached* write is at risk, never the caller-visible result.
///
/// Deliberately does **not** assert on the summary row.
/// In this config (`[llm.auxiliary] session_summary` unset) whether the row lands is a scheduling race: `build_session_summary` early-returns a trivial digest with no `.await` in front of the write, so the detached task usually finishes inside the window `reset_session`'s remaining async work provides — but "usually" is not a property worth asserting, and on a loaded machine it flips.
/// Asserting either direction would be a flake.
///
/// That race is precisely why the original bug was invisible in the default config, and why `block_on_tui` must not be "simplified" back to a throwaway on the strength of this suite passing.
/// The guarantee that does hold unconditionally is the one above: a runtime that outlives the call.
#[test]
fn reset_itself_succeeds_even_on_a_throwaway_runtime() {
    let (kernel, _tmp) = MockKernelBuilder::new()
        .with_config(|c| {
            c.default_model.provider = "ollama".to_string();
            c.default_model.model = "test".to_string();
            c.default_model.api_key_env = "OLLAMA_API_KEY".to_string();
        })
        .build();
    let kernel = Arc::new(kernel);
    let (agent_id, sid) = seed(&kernel, "tui-throwaway-rt-agent");

    {
        // Exactly what the TUI used to do: build, block_on, drop.
        let rt = tokio::runtime::Runtime::new().expect("build runtime");
        rt.block_on(kernel.reset_session(agent_id, ResetScope::Agent))
            .expect("reset_session must succeed regardless of runtime lifetime");
    }

    // The session row is gone either way — that part is synchronous.
    assert!(
        kernel
            .memory_substrate()
            .get_session(sid)
            .unwrap()
            .is_none()
            || kernel
                .memory_substrate()
                .get_session(sid)
                .unwrap()
                .is_some_and(|s| s.messages.is_empty()),
        "reset must clear the session transcript synchronously"
    );
}
