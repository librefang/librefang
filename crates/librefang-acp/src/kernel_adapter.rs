//! Concrete [`AcpKernel`] impl over `Arc<LibreFangKernel>`.
//!
//! Lives behind the `kernel-adapter` feature so the protocol crate
//! doesn't pull in the heavy `librefang-kernel` dep tree by default.
//! Both the in-process CLI path (`librefang acp`) and the
//! daemon-attached UDS listener (`librefang-api`) instantiate this
//! adapter against the same kernel.

use std::sync::Arc;

use async_trait::async_trait;
use librefang_kernel::kernel_handle::KernelHandle;
use librefang_kernel::LibreFangKernel;
use librefang_llm_driver::StreamEvent;
use librefang_types::agent::{AgentId, SessionId as LfSessionId};
use librefang_types::approval::{ApprovalDecision, ApprovalEvent};
use tokio::sync::{broadcast, mpsc};
use uuid::Uuid;

use crate::{AcpError, AcpKernel, AcpResult};

/// Wraps an `Arc<LibreFangKernel>` so it can serve ACP traffic.
///
/// Construct via [`KernelAdapter::new`]; consume the `Arc<KernelAdapter>`
/// through [`crate::run`] / [`crate::run_with_transport`].
///
/// Holds the kernel both as the concrete type (for `Arc<LibreFangKernel>`
/// methods like `send_message_streaming_*` that aren't on `KernelHandle`)
/// and as a `KernelHandle` trait object (for `resolve_tool_approval`,
/// which must go through the trait so the kernel-side
/// `handle_approval_resolution` spawn fires — see the long comment on
/// `resolve_approval` below).
pub struct KernelAdapter {
    kernel: Arc<LibreFangKernel>,
    kernel_handle: Arc<dyn KernelHandle>,
}

impl KernelAdapter {
    pub fn new(kernel: Arc<LibreFangKernel>) -> Self {
        // `kernel_handle()` panics if `set_self_handle` hasn't run, but
        // `LibreFangKernel::boot` wires that up before returning, so by
        // here it's safe.
        let kernel_handle = kernel.kernel_handle();
        Self {
            kernel,
            kernel_handle,
        }
    }

    /// Borrow the underlying kernel — useful for daemon-side bookkeeping
    /// (e.g. spawning the approval-sweep task) before handing the
    /// adapter into [`crate::run`].
    pub fn kernel(&self) -> &Arc<LibreFangKernel> {
        &self.kernel
    }
}

#[async_trait]
impl AcpKernel for KernelAdapter {
    async fn resolve_agent(&self, name_or_id: &str) -> AcpResult<AgentId> {
        // Accept either a UUID (the kernel's canonical form) or a
        // human-readable agent name (resolved via the registry's name
        // index). UUIDs go through the registry too so a typo doesn't
        // surface as a vague "kernel send_prompt: …" later.
        if let Ok(uuid) = Uuid::parse_str(name_or_id) {
            let id = AgentId(uuid);
            return self
                .kernel
                .agent_registry()
                .get(id)
                .map(|_| id)
                .ok_or_else(|| AcpError::AgentNotFound(name_or_id.to_string()));
        }
        if let Some(entry) = self.kernel.agent_registry().find_by_name(name_or_id) {
            return Ok(entry.id);
        }
        Err(AcpError::AgentNotFound(name_or_id.to_string()))
    }

    async fn send_prompt(
        &self,
        agent_id: AgentId,
        message: String,
        librefang_session_id: LfSessionId,
    ) -> AcpResult<mpsc::Receiver<StreamEvent>> {
        // The kernel surfaces this method's failures as
        // `librefang_kernel::error::KernelError` rather than the
        // workspace-canonical `LibreFangError`, so we flatten through
        // `to_string` instead of `?`. Phase 2 (#3541 follow-up) will
        // narrow the kernel error type and lift this back to a typed
        // `From` conversion.
        let (rx, _join) = self
            .kernel
            .send_message_streaming_with_routing_and_session_override(
                agent_id,
                &message,
                None,
                Some(librefang_session_id),
            )
            .await
            .map_err(|e| AcpError::internal(format!("kernel send_prompt: {e}")))?;
        Ok(rx)
    }

    fn subscribe_approvals(&self) -> broadcast::Receiver<ApprovalEvent> {
        self.kernel.approvals().subscribe()
    }

    async fn resolve_approval(
        &self,
        request_id: Uuid,
        decision: ApprovalDecision,
        decided_by: Option<String>,
    ) -> AcpResult<()> {
        // Route through the trait, NOT `kernel.approvals().resolve()`
        // directly. Tools that hit the deferred-approval path (the
        // `submit_request` branch of `ApprovalManager`) are queued and
        // only execute when `kernel.handle_approval_resolution(...)` is
        // spawned alongside the resolve. `ApprovalManager::resolve` by
        // itself just clears the pending entry — it explicitly does not
        // run the deferred tool (see the doc on
        // `ApprovalManager::resolve_all_for_session` in
        // `librefang-kernel/src/approval.rs`).
        //
        // `KernelHandle::resolve_tool_approval` (impl in
        // `kernel/handles/approval_gate.rs`) wraps both: it calls
        // `ApprovalManager::resolve`, then on `Approved + Some(deferred)`
        // it spawns a background task that runs the queued tool and
        // resumes the agent loop. Without this routing, `Allow once`
        // from the editor modal would clear the approval record but the
        // agent loop would hang forever waiting for the tool result
        // that never lands.
        self.kernel_handle
            .resolve_tool_approval(request_id, decision, decided_by, false, None)
            .await
            .map_err(|e| AcpError::internal(e.to_string()))?;
        Ok(())
    }

    async fn remember_decision(
        &self,
        agent_id: &str,
        tool_name: &str,
        decision: ApprovalDecision,
    ) -> AcpResult<()> {
        self.kernel
            .approvals()
            .remember(agent_id, tool_name, decision);
        Ok(())
    }
}
