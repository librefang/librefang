//! `librefang acp` subcommand — runs the Agent Client Protocol server
//! over stdio so editors like Zed / VS Code / JetBrains can embed
//! LibreFang as a native agent (#3313).
//!
//! Phase 1 is **in-process only**: each invocation boots its own
//! [`LibreFangKernel`] in the same process, runs the ACP server, and
//! tears the kernel down on stdin EOF. This matches the
//! "spawn-per-workspace" model Zed expects from ACP child processes.
//! Daemon-attached mode (multiple editors talking to one long-running
//! daemon) is Phase 2.
//!
//! The actual ACP wire protocol + handler chain lives in the
//! `librefang-acp` crate; this module is the kernel-binding glue.

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use librefang_acp::{AcpError, AcpKernel, AcpResult};
use librefang_kernel::LibreFangKernel;
use librefang_llm_driver::StreamEvent;
use librefang_types::agent::{AgentId, SessionId as LfSessionId};
use librefang_types::approval::{ApprovalDecision, ApprovalEvent};
use tokio::sync::{broadcast, mpsc};
use uuid::Uuid;

/// Wraps an `Arc<LibreFangKernel>` to implement [`AcpKernel`].
///
/// Splitting this glue out of `librefang-acp` keeps the protocol crate
/// independent of the heavy `librefang-kernel` dependency tree —
/// integration tests in `librefang-acp` use a mock impl, and the real
/// binding only links here in the CLI.
struct KernelAdapter {
    kernel: Arc<LibreFangKernel>,
}

#[async_trait]
impl AcpKernel for KernelAdapter {
    async fn resolve_agent(&self, name_or_id: &str) -> AcpResult<AgentId> {
        // Accept either a UUID (the kernel's canonical form) or a
        // human-readable agent name (resolved via the registry's name
        // index).
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
        // workspace-canonical `LibreFangError`, so we have to flatten
        // through `to_string` instead of `?`. Phase 2 (#3541 follow-up)
        // will narrow the kernel error type and lift this back to a
        // typed `From` conversion.
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
        // We deliberately drop the `JoinHandle`. The agent loop's final
        // `StopReason` is delivered via `StreamEvent::ContentComplete`
        // on the receiver, and the receiver closing is the unambiguous
        // "turn over" signal. The kernel manages its own task lifecycle
        // — there's no orphan-task risk because the spawned future
        // shuts down when the receiver is dropped.
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
        // `ApprovalManager::resolve` returns `Result<_, String>` for
        // legacy reasons; map to `AcpError::Internal` so transport
        // errors and kernel errors share one variant.
        self.kernel
            .approvals()
            .resolve(request_id, decision, decided_by, false, None)
            .map_err(AcpError::internal)?;
        Ok(())
    }
}

/// Phase-1 default agent name when the CLI is invoked without `--agent`.
///
/// Mirrors the dashboard / TUI default so editor users land on the same
/// agent they see in the rest of LibreFang's surfaces.
const DEFAULT_AGENT_NAME: &str = "assistant";

/// Boot an in-process kernel and run the ACP server on stdio until
/// stdin EOF.
///
/// Exits the process with a non-zero status on hard failures (kernel
/// boot, agent resolution, transport setup). Successful completion
/// (editor disconnects cleanly) returns 0.
pub fn run_acp_server(config: Option<PathBuf>, agent: Option<String>) {
    // Phase 1 runs in-process even when a daemon is up — the daemon
    // would need an ACP-over-RPC gateway to host editor sessions on
    // behalf of multiple `librefang acp` clients, which is Phase 2
    // scope. Warn so users notice if they expected daemon-attached
    // behaviour.
    if super::find_daemon().is_some() {
        tracing::warn!(
            "librefang acp Phase 1 runs in-process even with a daemon present; \
             daemon-attached mode will land in a follow-up to #3313"
        );
    }

    let kernel = match LibreFangKernel::boot(config.as_deref()) {
        Ok(k) => Arc::new(k),
        Err(e) => {
            eprintln!("Failed to boot kernel: {e}");
            std::process::exit(1);
        }
    };

    let rt = tokio::runtime::Runtime::new().expect("Failed to create Tokio runtime");
    let exit_code = rt.block_on(async {
        // Background task: periodically expire timed-out approvals, the
        // same way the daemon does. Without this, deferred approvals
        // would never time out in an `acp` session.
        kernel.clone().spawn_approval_sweep_task();

        let agent_name = agent.as_deref().unwrap_or(DEFAULT_AGENT_NAME);
        let adapter = KernelAdapter {
            kernel: Arc::clone(&kernel),
        };
        let agent_id = match adapter.resolve_agent(agent_name).await {
            Ok(id) => id,
            Err(e) => {
                eprintln!("Failed to resolve agent '{agent_name}': {e}");
                return 1;
            }
        };

        match librefang_acp::run(Arc::new(adapter), agent_id).await {
            Ok(()) => 0,
            Err(e) => {
                eprintln!("ACP server error: {e}");
                1
            }
        }
    });

    if exit_code != 0 {
        std::process::exit(exit_code);
    }
}
