//! Concrete [`AcpKernel`] impl over `Arc<LibreFangKernel>`.
//!
//! Lives behind the `kernel-adapter` feature so the protocol crate
//! doesn't pull in the heavy `librefang-kernel` dep tree by default.
//! Both the in-process CLI path (`librefang acp`) and the
//! daemon-attached UDS listener (`librefang-api`) instantiate this
//! adapter against the same kernel.

use std::sync::Arc;

use async_trait::async_trait;
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
pub struct KernelAdapter {
    kernel: Arc<LibreFangKernel>,
}

impl KernelAdapter {
    pub fn new(kernel: Arc<LibreFangKernel>) -> Self {
        Self { kernel }
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
        if let Ok(uuid) = Uuid::parse_str(name_or_id) {
            return Ok(AgentId(uuid));
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
        self.kernel
            .approvals()
            .resolve(request_id, decision, decided_by, false, None)
            .map_err(AcpError::internal)?;
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
