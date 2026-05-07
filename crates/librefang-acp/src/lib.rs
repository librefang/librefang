//! Agent Client Protocol (ACP) adapter for LibreFang (#3313).
//!
//! This crate bridges LibreFang's runtime to the [Agent Client Protocol](
//! https://agentclientprotocol.com/), letting editors like Zed, VS Code, and
//! JetBrains embed a LibreFang agent natively — with the editor providing
//! approval modals, file references, image attachments, and prompt streaming
//! through its own UI rather than the LibreFang dashboard.
//!
//! # Architecture
//!
//! ACP is a JSON-RPC 2.0 protocol over a duplex byte stream (typically
//! stdio). The wire format is handled by the [`agent-client-protocol`] crate
//! published by Zed; this crate only does the LibreFang-specific glue:
//!
//! * [`events`] — translate [`librefang_llm_driver::StreamEvent`] from the
//!   agent loop into ACP `session/update` notifications.
//! * [`session`] — maintain a map of ACP session ids to LibreFang session
//!   ids and per-session cancel tokens.
//! * [`permission`] — bridge LibreFang's `ApprovalRequest` /
//!   `ApprovalDecision` to ACP `session/request_permission` round trips.
//! * [`prompt`] — drive a single prompt turn: pump events, dispatch
//!   permission requests, return a `PromptResponse`.
//! * [`server`] — assemble the handler chain on top of
//!   [`agent_client_protocol::Agent`]'s `.builder()` and run the stdio loop.
//!
//! # Scope
//!
//! Implements `initialize`, `session/{new,load,list,resume,close,prompt,
//! cancel}`, and the agent → client `session/request_permission` round
//! trip with persisted `allow_always` / `reject_always`. Both
//! in-process (over stdio) and daemon-attached (over a Unix domain
//! socket) execution modes are wired up. `fs/*`, `terminal/*`, and
//! image / audio / embedded resource content blocks are tracked as
//! follow-up issues.

pub mod error;
pub mod events;
pub mod fs;
#[cfg(feature = "kernel-adapter")]
pub mod kernel_adapter;
pub mod permission;
pub mod prompt;
pub mod server;
pub mod session;
pub mod terminal;

#[cfg(feature = "kernel-adapter")]
pub use kernel_adapter::KernelAdapter;

pub use error::{AcpError, AcpResult};
pub use fs::{FsCapabilities, FsClientHandle};
pub use server::{run, run_with_transport};
pub use terminal::{TerminalCapabilities, TerminalClientHandle};

use std::sync::Arc;

use async_trait::async_trait;
use librefang_llm_driver::StreamEvent;
use librefang_types::agent::{AgentId, SessionId as LfSessionId};
use librefang_types::approval::{ApprovalDecision, ApprovalEvent};
use tokio::sync::{broadcast, mpsc};
use uuid::Uuid;

/// The minimal kernel surface the ACP adapter needs.
///
/// Pulled out as a trait so:
///
/// 1. The ACP server is testable in isolation — integration tests in
///    `tests/acp_integration.rs` use a stub impl that returns canned
///    [`StreamEvent`] sequences without booting a real kernel.
/// 2. The crate doesn't depend directly on `librefang-kernel` (which
///    transitively pulls in every driver and storage layer). The thin
///    glue lives in `librefang-cli`'s `acp` module where we can spend
///    the dependency budget.
#[async_trait]
pub trait AcpKernel: Send + Sync + 'static {
    /// Resolve a name or UUID string to an [`AgentId`]. Called once at
    /// startup to anchor the adapter to a single agent.
    async fn resolve_agent(&self, name_or_id: &str) -> AcpResult<AgentId>;

    /// Begin a streaming prompt turn against `agent_id` on
    /// `librefang_session_id`. Returns the event channel; the channel
    /// closes when the agent loop ends. The final
    /// [`StreamEvent::ContentComplete`] (if any) carries the
    /// [`librefang_types::message::StopReason`] that becomes the
    /// `PromptResponse.stop_reason`.
    async fn send_prompt(
        &self,
        agent_id: AgentId,
        message: String,
        librefang_session_id: LfSessionId,
    ) -> AcpResult<mpsc::Receiver<StreamEvent>>;

    /// Subscribe to [`ApprovalEvent`]s emitted by the kernel's approval
    /// manager. The ACP server filters by `session_id` to match its own
    /// sessions and forwards via `session/request_permission`.
    fn subscribe_approvals(&self) -> broadcast::Receiver<ApprovalEvent>;

    /// Resolve a pending approval. Called after the editor user picks
    /// a permission option (or the 60s timeout fires with `Denied`).
    async fn resolve_approval(
        &self,
        request_id: Uuid,
        decision: ApprovalDecision,
        decided_by: Option<String>,
    ) -> AcpResult<()>;

    /// Persist an "always" decision so future approval queries for the
    /// same `(agent_id, tool_name)` short-circuit (#3313). Called by
    /// the permission bridge when the editor user picks
    /// `allow_always` / `reject_always`. Default impl is a no-op so
    /// test mocks don't have to override.
    async fn remember_decision(
        &self,
        _agent_id: &str,
        _tool_name: &str,
        _decision: ApprovalDecision,
    ) -> AcpResult<()> {
        Ok(())
    }

    /// Hand the kernel a [`FsClientHandle`] so the runtime can route
    /// `fs/read_text_file` / `fs/write_text_file` back through the
    /// editor (#3313). Called once per connection right after
    /// `initialize` lands, before any `session/prompt` arrives.
    /// Default impl drops the handle on the floor — pure-protocol
    /// consumers (integration tests) ignore it.
    fn set_fs_client(&self, _handle: fs::FsClientHandle) {}

    /// Bind the active `fs/*` client (set via [`Self::set_fs_client`])
    /// to a LibreFang session so the runtime can dispatch through it
    /// when handling that session's tool calls (#3313). Called once
    /// per ACP `session/{new,load,resume}` after the LibreFang
    /// session id has been minted. Default impl is a no-op.
    fn register_session_fs(&self, _lf_session_id: LfSessionId) {}

    /// Drop the runtime-side `fs/*` registration for the given
    /// LibreFang session — called from `session/close` so a stale
    /// handle can't keep firing requests onto a closed connection.
    fn unregister_session_fs(&self, _lf_session_id: LfSessionId) {}

    /// Hand the kernel a [`terminal::TerminalClientHandle`] for the
    /// `terminal/*` reverse-RPC channel (#3313). Mirrors
    /// [`Self::set_fs_client`].
    fn set_terminal_client(&self, _handle: terminal::TerminalClientHandle) {}

    /// Bind the active `terminal/*` client (set via
    /// [`Self::set_terminal_client`]) to a LibreFang session.
    fn register_session_terminal(&self, _lf_session_id: LfSessionId) {}

    /// Drop the runtime-side `terminal/*` registration on close.
    fn unregister_session_terminal(&self, _lf_session_id: LfSessionId) {}

    /// Pull the persisted message history for a session so the editor's
    /// chat panel can rehydrate immediately on reconnect (#3313).
    /// Returned tuples are `(role, text)` — system messages are filtered
    /// out at the boundary since the editor is interested in the
    /// human-visible turns. Default impl returns empty for
    /// pure-protocol consumers (integration tests).
    async fn fetch_session_history(
        &self,
        _lf_session_id: LfSessionId,
    ) -> Vec<(librefang_types::message::Role, String)> {
        Vec::new()
    }
}

/// Convenience type alias for `Arc<dyn AcpKernel>`. Most call sites pass
/// the kernel by `Arc` so handlers can clone-and-move it freely.
pub type SharedAcpKernel = Arc<dyn AcpKernel>;
