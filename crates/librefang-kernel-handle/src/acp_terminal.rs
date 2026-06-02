use async_trait::async_trait;

use super::*;

// ============================================================================
// 18. AcpTerminalBridge — editor-backed `terminal/*` reverse-RPC
//
// Used by `shell_exec` and similar runtime tools to host the command's
// PTY in the editor (so output appears in the editor's terminal panel
// and the user can kill / interact with it) instead of spawning a
// detached process the agent never sees (#3313).
// ============================================================================

/// Result of a single full `terminal/*` create→wait→output→release run.
/// Mirrors the values the runtime needs to assemble a `shell_exec`
/// `ToolResult` without taking a `agent-client-protocol` dep.
#[derive(Debug, Clone)]
pub struct AcpTerminalRunResult {
    /// Captured stdout/stderr (interleaved as the PTY received them).
    pub output: String,
    /// `true` if the editor truncated the output to fit the
    /// `output_byte_limit`. Runtime tools should surface this in the
    /// tool result so the LLM knows it didn't see the whole transcript.
    pub truncated: bool,
    /// Process exit code, when the command exited normally. `None`
    /// when the command was killed by signal — see `signal`.
    pub exit_code: Option<i32>,
    /// Signal name (e.g. `"SIGTERM"`) when the command was killed by
    /// signal rather than a clean exit.
    pub signal: Option<String>,
}

/// Object-safe client side of the `terminal/*` reverse-RPC. Implemented
/// by `librefang-acp::TerminalClientHandle`; the kernel stores
/// `Arc<dyn AcpTerminalClient>` per session and dispatches through it.
#[async_trait]
pub trait AcpTerminalClient: Send + Sync {
    /// Run a single command to completion through the editor's PTY:
    /// `terminal/create` → `terminal/wait_for_exit` →
    /// `terminal/output` → `terminal/release`. The default impl on
    /// `TerminalClientHandle` always releases at the end, even on
    /// intermediate failure.
    async fn run_command(
        &self,
        command: String,
        args: Vec<String>,
        env: Vec<(String, String)>,
        cwd: Option<std::path::PathBuf>,
        output_byte_limit: Option<u64>,
    ) -> KernelResult<AcpTerminalRunResult>;

    /// Whether the editor declared `terminal` capability at
    /// `initialize` time. Runtime tools can use this to short-circuit
    /// before paying a round-trip when the editor doesn't support
    /// terminals.
    fn capabilities(&self) -> bool;
}

/// Runtime-facing role trait for editor-backed terminal commands.
#[async_trait]
pub trait AcpTerminalBridge: Send + Sync {
    /// Register a `terminal/*` client for `session_id`. Default impl
    /// is a no-op.
    fn register_acp_terminal_client(
        &self,
        session_id: librefang_types::agent::SessionId,
        client: std::sync::Arc<dyn AcpTerminalClient>,
    ) {
        let _ = (session_id, client);
    }

    /// Drop the registration for `session_id`.
    fn unregister_acp_terminal_client(&self, session_id: librefang_types::agent::SessionId) {
        let _ = session_id;
    }

    /// Look up the `terminal/*` client registered for `session_id`.
    /// Returns `None` when no editor is bound — runtime tools should
    /// fall back to local process spawning, not error out.
    fn acp_terminal_client(
        &self,
        session_id: librefang_types::agent::SessionId,
    ) -> Option<std::sync::Arc<dyn AcpTerminalClient>> {
        let _ = session_id;
        None
    }

    /// Convenience: run `command` through the editor bound to
    /// `session_id`. Returns `KernelOpError::Unavailable` when no
    /// editor is bound for the session.
    async fn acp_run_terminal_command(
        &self,
        session_id: librefang_types::agent::SessionId,
        command: String,
        args: Vec<String>,
        env: Vec<(String, String)>,
        cwd: Option<std::path::PathBuf>,
        output_byte_limit: Option<u64>,
    ) -> KernelResult<AcpTerminalRunResult> {
        match self.acp_terminal_client(session_id) {
            Some(client) => {
                client
                    .run_command(command, args, env, cwd, output_byte_limit)
                    .await
            }
            None => Err(KernelOpError::unavailable(
                "ACP terminal/* (no editor bound to session)",
            )),
        }
    }
}
