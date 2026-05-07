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
//! # Phase split
//!
//! This is **Phase 1** scope (#3313). The crate is intentionally narrow:
//! `initialize`, `session/new`, `session/prompt`, `session/cancel`, plus
//! the agent → client `session/request_permission` round trip. Phase 2
//! (separate issue) will add `fs/*`, `terminal/*`, `session/load`,
//! image/audio content blocks, and daemon-attached mode.

pub mod error;
pub mod events;
pub mod permission;
pub mod prompt;
pub mod server;
pub mod session;

pub use error::{AcpError, AcpResult};
pub use server::{run, run_with_transport};

use std::sync::Arc;

use async_trait::async_trait;
use librefang_llm_driver::StreamEvent;
use librefang_types::agent::{AgentId, SessionId as LfSessionId};
use librefang_types::approval::{ApprovalDecision, ApprovalEvent};
use tokio::sync::{broadcast, mpsc};
use uuid::Uuid;

/// Reason the agent loop ended a prompt turn. Mapped onto
/// [`agent_client_protocol::schema::StopReason`] before the response goes
/// out on the wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptStopReason {
    EndTurn,
    MaxTokens,
    MaxTurnRequests,
    Refusal,
    Cancelled,
}

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
}

/// Convenience type alias for `Arc<dyn AcpKernel>`. Most call sites pass
/// the kernel by `Arc` so handlers can clone-and-move it freely.
pub type SharedAcpKernel = Arc<dyn AcpKernel>;
