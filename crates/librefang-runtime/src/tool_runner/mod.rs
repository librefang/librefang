//! Built-in tool execution.
//!
//! Provides filesystem, web, shell, and inter-agent tools. Agent tools
//! (agent_send, agent_spawn, etc.) require a KernelHandle to be passed in.

use crate::kernel_handle::prelude::*;
use std::sync::Arc;

mod a2a;
mod agent;
mod artifact;
mod canvas;
mod channel;
mod cron;
mod definitions;
mod dispatch;
mod event;
mod fs;
mod goal;
mod hand;
mod image;
mod knowledge;
#[cfg(feature = "media")]
mod media;
mod memory;
mod meta;
mod notify;
mod process;
#[cfg(feature = "docker-sandbox")]
mod sandbox;
mod schedule;
mod shell;
mod shell_safety;
mod skill;
mod spill;
mod system;
mod taint;
mod task;
mod web_legacy;
mod wiki;
mod workflow;

use self::a2a::{tool_a2a_discover, tool_a2a_send};
use self::agent::{
    tool_agent_find, tool_agent_kill, tool_agent_list, tool_agent_send, tool_agent_spawn,
};
use self::artifact::tool_read_artifact;
pub use self::canvas::sanitize_canvas_html;
use self::canvas::tool_canvas_present;
use self::channel::tool_channel_send;
use self::cron::{tool_cron_cancel, tool_cron_create, tool_cron_list};
pub use self::definitions::{builtin_tool_definitions, select_native_tools, ALWAYS_NATIVE_TOOLS};
pub use self::dispatch::{current_agent_depth, execute_tool, execute_tool_raw, ToolExecContext};
use self::event::tool_event_publish;
use self::fs::{
    check_absolute_path_inside_workspace, maybe_dedup_file_read, maybe_snapshot, named_ws_prefixes,
    named_ws_prefixes_readonly, named_ws_prefixes_writable, resolve_file_path_ext,
    tool_apply_patch, tool_file_list, tool_file_read, tool_file_write,
};
use self::goal::tool_goal_update;
use self::hand::{tool_hand_activate, tool_hand_deactivate, tool_hand_list, tool_hand_status};
use self::image::tool_image_analyze;
use self::knowledge::{
    tool_knowledge_add_entity, tool_knowledge_add_relation, tool_knowledge_query,
};
#[cfg(feature = "media")]
use self::media::{
    tool_image_generate, tool_media_describe, tool_media_transcribe, tool_music_generate,
    tool_speech_to_text, tool_text_to_speech, tool_video_generate, tool_video_status,
};
use self::memory::{tool_memory_list, tool_memory_recall, tool_memory_store};
use self::meta::{tool_meta_load, tool_meta_search};
use self::notify::tool_notify_owner;
use self::process::{
    tool_process_kill, tool_process_list, tool_process_poll, tool_process_start, tool_process_write,
};
#[cfg(feature = "docker-sandbox")]
use self::sandbox::tool_docker_exec;
use self::schedule::{tool_schedule_create, tool_schedule_delete, tool_schedule_list};
use self::shell::tool_shell_exec;
use self::shell_safety::{classify_shell_exec_ro_safety, RoSafety};
use self::skill::{
    tool_skill_evolve_create, tool_skill_evolve_delete, tool_skill_evolve_patch,
    tool_skill_evolve_remove_file, tool_skill_evolve_rollback, tool_skill_evolve_update,
    tool_skill_evolve_write_file, tool_skill_read_file,
};
use self::spill::{resolve_spill_config, spill_or_passthrough};
use self::system::{tool_location_get, tool_system_time};
use self::taint::{
    check_taint_net_fetch, check_taint_outbound_header, check_taint_outbound_text,
    check_taint_shell_exec,
};
use self::task::{
    tool_task_claim, tool_task_complete, tool_task_list, tool_task_post, tool_task_status,
};
use self::web_legacy::{tool_web_fetch_legacy, tool_web_search_legacy};
use self::wiki::{tool_wiki_get, tool_wiki_search, tool_wiki_write};
#[cfg(test)]
use self::workflow::{
    build_workflow_run_result, prepare_workflow_input, resolve_workflow_input_artifacts,
};
use self::workflow::{
    tool_workflow_cancel, tool_workflow_describe, tool_workflow_list, tool_workflow_run,
    tool_workflow_start, tool_workflow_status,
};

/// Maximum inter-agent call depth to prevent infinite recursion (A->B->C->...).
#[allow(dead_code)]
const MAX_AGENT_CALL_DEPTH: u32 = 5;

tokio::task_local! {
    /// Tracks the current inter-agent call depth within a task.
    pub(super) static AGENT_CALL_DEPTH: std::cell::Cell<u32>;
    /// Canvas max HTML size in bytes (set from kernel config at loop start).
    pub static CANVAS_MAX_BYTES: usize;
}

/// Shared `Option<&Arc<dyn KernelHandle>>` -> `&Arc<dyn KernelHandle>` unwrap
/// used by every kernel-backed tool module. Kept at the parent module so each
/// child can `use super::require_kernel` without redeclaring it.
pub(super) fn require_kernel(
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<&Arc<dyn KernelHandle>, String> {
    kernel.ok_or_else(|| {
        "Kernel handle not available. Inter-agent tools require a running kernel.".to_string()
    })
}

/// The memory-namespace operation a memory / wiki tool is about to perform.
/// Maps onto [`librefang_memory::namespace_acl::MemoryNamespaceGuard`]'s
/// `check_read` / `check_write`.
#[derive(Clone, Copy, Debug)]
pub(super) enum MemoryAclOp {
    Read,
    Write,
}

/// Enforce the per-user `UserMemoryAccess` ACL at the tool dispatch boundary
/// (#5139).
///
/// The shared-KV (`memory_*`) and wiki (`wiki_*`) tools previously reached the
/// substrate without ever consulting the per-user RBAC ACL — only the
/// proactive-retrieval path in `agent_loop::prompt` did. A user whose
/// `UserMemoryAccess.writable_namespaces` is restricted (or empty, e.g. the
/// `viewer` role default) could still drive `tool_memory_store` /
/// `tool_wiki_write` and reach cross-user shared state.
///
/// This resolves the ACL from the attributed sender + channel via
/// [`librefang_kernel_handle::MemoryAccess::memory_acl_for_sender`] — the SAME
/// resolver the proactive path uses — builds a `MemoryNamespaceGuard`, and
/// returns `Err` (which the dispatcher surfaces to the model as a tool error,
/// so the underlying substrate call never runs) when the requested op is
/// denied for the requested namespace.
///
/// `Ok(())` when the kernel reports `None` (RBAC disabled or sender not
/// attributed to a registered user) — that preserves the existing
/// single-user / RBAC-off behaviour, exactly as the proactive path does.
pub(super) fn enforce_memory_acl(
    kernel: Option<&Arc<dyn KernelHandle>>,
    sender_id: Option<&str>,
    channel: Option<&str>,
    op: MemoryAclOp,
    namespace: &str,
) -> Result<(), String> {
    let kh = match kernel {
        Some(kh) => kh,
        // No kernel handle (legacy / test call sites): nothing to enforce
        // against. The substrate-boundary peer-key guards (#5119/#5120)
        // still apply downstream.
        None => return Ok(()),
    };
    let Some(acl) = kh.memory_acl_for_sender(sender_id, channel) else {
        // RBAC disabled or sender unattributed — no per-user restriction.
        return Ok(());
    };
    let guard = librefang_memory::namespace_acl::MemoryNamespaceGuard::new(acl);
    let gate = match op {
        MemoryAclOp::Read => guard.check_read(namespace),
        MemoryAclOp::Write => guard.check_write(namespace),
    };
    match gate {
        librefang_memory::namespace_acl::NamespaceGate::Allow => Ok(()),
        librefang_memory::namespace_acl::NamespaceGate::Deny(reason) => Err(format!(
            "Access denied: your user policy does not permit this memory operation \
             on namespace '{namespace}' ({reason})."
        )),
    }
}

/// Map a shared-KV peer scope to the ACL namespace string.
///
/// The shared-KV tools store under `peer:{peer_id}:{key}` (see
/// `kernel::peer_scoped_key`). The per-user ACL namespace mirrors that scope as
/// `kv:{peer_id}` so the role-default patterns (`kv:*` for `user`, none for
/// `viewer`) gate it the way the audit (#5139) and the existing
/// `default_memory_acl` intend. An unscoped call (`peer_id = None`) maps to the
/// bare `kv` global bucket — but in practice the ACL is only consulted when the
/// sender was attributed, in which case `peer_id` is always `Some`.
pub(super) fn kv_acl_namespace(peer_id: Option<&str>) -> String {
    match peer_id {
        Some(pid) => format!("kv:{pid}"),
        None => "kv".to_string(),
    }
}

#[cfg(test)]
mod tests;
