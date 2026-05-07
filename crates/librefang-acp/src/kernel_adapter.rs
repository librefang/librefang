//! Concrete [`AcpKernel`] impl over `Arc<LibreFangKernel>`.
//!
//! Lives behind the `kernel-adapter` feature so the protocol crate
//! doesn't pull in the heavy `librefang-kernel` dep tree by default.
//! Both the in-process CLI path (`librefang acp`) and the
//! daemon-attached UDS listener (`librefang-api`) instantiate this
//! adapter against the same kernel.

use std::sync::Arc;
use std::sync::RwLock;

use async_trait::async_trait;
use librefang_kernel::kernel_handle::{AcpFsBridge, KernelHandle};
use librefang_kernel::LibreFangKernel;
use librefang_llm_driver::StreamEvent;
use librefang_types::agent::{AgentId, SessionId as LfSessionId};
use librefang_types::approval::{ApprovalDecision, ApprovalEvent};
use tokio::sync::{broadcast, mpsc};
use uuid::Uuid;

use crate::fs::FsClientHandle;
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
    /// Editor-bound `fs/*` reverse-RPC handle, populated at
    /// `initialize` time by the ACP server. `None` for transports that
    /// haven't completed the handshake yet (test harnesses,
    /// pre-initialize tool calls — both Phase-2 niches).
    fs_client: Arc<RwLock<Option<FsClientHandle>>>,
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
            fs_client: Arc::new(RwLock::new(None)),
        }
    }

    /// Borrow the underlying kernel — useful for daemon-side bookkeeping
    /// (e.g. spawning the approval-sweep task) before handing the
    /// adapter into [`crate::run`].
    pub fn kernel(&self) -> &Arc<LibreFangKernel> {
        &self.kernel
    }

    /// Snapshot of the `fs/*` client handle, if any. Cloned out so the
    /// caller doesn't hold the lock across `await`. Returns `None`
    /// before `initialize` has handed us a connection.
    pub fn fs_client(&self) -> Option<FsClientHandle> {
        self.fs_client.read().ok().and_then(|guard| guard.clone())
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

    fn set_fs_client(&self, handle: FsClientHandle) {
        // The lock can only be poisoned by a panic in another writer,
        // and we only ever overwrite the slot. Recover the inner
        // value so a poisoned lock from a prior connection (defensive)
        // doesn't strand the new handshake.
        let mut guard = self
            .fs_client
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *guard = Some(handle);
    }

    fn register_session_fs(&self, lf_session_id: LfSessionId) {
        // No client handle yet (initialize hasn't run, or this server
        // is a stub-mock harness) → nothing to register; runtime tools
        // will see `Unavailable` and fall back to local fs.
        let Some(handle) = self.fs_client() else {
            return;
        };
        // `FsClientHandle` impls `AcpFsClient`, so we coerce into the
        // trait object the kernel registry expects.
        let client: Arc<dyn librefang_kernel::kernel_handle::AcpFsClient> = Arc::new(handle);
        self.kernel.register_acp_fs_client(lf_session_id, client);
    }

    fn unregister_session_fs(&self, lf_session_id: LfSessionId) {
        self.kernel.unregister_acp_fs_client(lf_session_id);
    }
}
