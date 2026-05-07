//! `terminal/*` reverse-RPC helpers (#3313).
//!
//! ACP's `terminal/*` family is a five-method state machine that lets
//! the agent ask the editor to host a PTY: `terminal/create` → returns
//! a `TerminalId`, `terminal/output` polls captured stdio, `terminal/
//! wait_for_exit` blocks until the command finishes, `terminal/kill`
//! kills the process without releasing the terminal, `terminal/release`
//! drops the terminal entirely.
//!
//! The runtime side wraps the five-call dance into a single
//! `run_command_to_completion` helper that mirrors the synchronous
//! `shell_exec` semantics LibreFang's runtime tool already exposes —
//! create, wait for exit, snapshot output, release. The intermediate
//! `terminal/output` polling and `kill` are wired but only exposed for
//! callers that need finer-grained control.

use std::path::PathBuf;
use std::sync::Arc;

use agent_client_protocol::schema::{
    ClientCapabilities, CreateTerminalRequest, EnvVariable, KillTerminalRequest,
    ReleaseTerminalRequest, SessionId as AcpSessionId, TerminalExitStatus, TerminalId,
    TerminalOutputRequest, WaitForTerminalExitRequest,
};
use agent_client_protocol::Client;
use agent_client_protocol::ConnectionTo;
use async_trait::async_trait;
use librefang_kernel_handle::{
    AcpTerminalClient, AcpTerminalRunResult, KernelOpError, KernelResult,
};

use crate::AcpError;

/// Editor-declared terminal capabilities, captured at `initialize`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TerminalCapabilities {
    /// Editor accepts `terminal/*` requests.
    pub terminal: bool,
}

impl TerminalCapabilities {
    pub(crate) fn from_client(caps: &ClientCapabilities) -> Self {
        Self {
            terminal: caps.terminal,
        }
    }
}

/// Handle that issues `terminal/*` requests to the connected ACP client.
#[derive(Clone)]
pub struct TerminalClientHandle {
    inner: Arc<TerminalClientInner>,
}

struct TerminalClientInner {
    cx: ConnectionTo<Client>,
    caps: TerminalCapabilities,
}

impl TerminalClientHandle {
    pub(crate) fn new(cx: ConnectionTo<Client>, caps: TerminalCapabilities) -> Self {
        Self {
            inner: Arc::new(TerminalClientInner { cx, caps }),
        }
    }

    pub fn capabilities(&self) -> TerminalCapabilities {
        self.inner.caps
    }

    /// `terminal/create` — ask the editor to host a new PTY.
    pub async fn create(
        &self,
        session_id: AcpSessionId,
        command: String,
        args: Vec<String>,
        env: Vec<(String, String)>,
        cwd: Option<PathBuf>,
        output_byte_limit: Option<u64>,
    ) -> Result<TerminalId, AcpError> {
        let mut req = CreateTerminalRequest::new(session_id, command);
        req.args = args;
        req.env = env
            .into_iter()
            .map(|(name, value)| EnvVariable::new(name, value))
            .collect();
        req.cwd = cwd;
        req.output_byte_limit = output_byte_limit;
        let sent = self.inner.cx.send_request(req);
        let (tx, rx) = tokio::sync::oneshot::channel();
        sent.on_receiving_result(async move |result| {
            let _ = tx.send(result);
            Ok(())
        })
        .map_err(AcpError::Transport)?;
        match rx.await {
            Ok(Ok(resp)) => Ok(resp.terminal_id),
            Ok(Err(e)) => Err(AcpError::Transport(e)),
            Err(_) => Err(AcpError::internal(
                "terminal/create response channel dropped",
            )),
        }
    }

    /// `terminal/wait_for_exit` — block until the PTY's command exits.
    pub async fn wait_for_exit(
        &self,
        session_id: AcpSessionId,
        terminal_id: TerminalId,
    ) -> Result<TerminalExitStatus, AcpError> {
        let req = WaitForTerminalExitRequest::new(session_id, terminal_id);
        let sent = self.inner.cx.send_request(req);
        let (tx, rx) = tokio::sync::oneshot::channel();
        sent.on_receiving_result(async move |result| {
            let _ = tx.send(result);
            Ok(())
        })
        .map_err(AcpError::Transport)?;
        match rx.await {
            Ok(Ok(resp)) => Ok(resp.exit_status),
            Ok(Err(e)) => Err(AcpError::Transport(e)),
            Err(_) => Err(AcpError::internal(
                "terminal/wait_for_exit response channel dropped",
            )),
        }
    }

    /// `terminal/output` — snapshot stdout/stderr captured so far.
    pub async fn output(
        &self,
        session_id: AcpSessionId,
        terminal_id: TerminalId,
    ) -> Result<(String, bool, Option<TerminalExitStatus>), AcpError> {
        let req = TerminalOutputRequest::new(session_id, terminal_id);
        let sent = self.inner.cx.send_request(req);
        let (tx, rx) = tokio::sync::oneshot::channel();
        sent.on_receiving_result(async move |result| {
            let _ = tx.send(result);
            Ok(())
        })
        .map_err(AcpError::Transport)?;
        match rx.await {
            Ok(Ok(resp)) => Ok((resp.output, resp.truncated, resp.exit_status)),
            Ok(Err(e)) => Err(AcpError::Transport(e)),
            Err(_) => Err(AcpError::internal(
                "terminal/output response channel dropped",
            )),
        }
    }

    /// `terminal/kill` — kill the PTY's process without releasing the
    /// terminal (so subsequent `output` calls still work).
    pub async fn kill(
        &self,
        session_id: AcpSessionId,
        terminal_id: TerminalId,
    ) -> Result<(), AcpError> {
        let req = KillTerminalRequest::new(session_id, terminal_id);
        let sent = self.inner.cx.send_request(req);
        let (tx, rx) = tokio::sync::oneshot::channel();
        sent.on_receiving_result(async move |result| {
            let _ = tx.send(result);
            Ok(())
        })
        .map_err(AcpError::Transport)?;
        match rx.await {
            Ok(Ok(_)) => Ok(()),
            Ok(Err(e)) => Err(AcpError::Transport(e)),
            Err(_) => Err(AcpError::internal("terminal/kill response channel dropped")),
        }
    }

    /// `terminal/release` — drop the terminal entirely. Required after
    /// every `terminal/create` to free editor-side resources.
    pub async fn release(
        &self,
        session_id: AcpSessionId,
        terminal_id: TerminalId,
    ) -> Result<(), AcpError> {
        let req = ReleaseTerminalRequest::new(session_id, terminal_id);
        let sent = self.inner.cx.send_request(req);
        let (tx, rx) = tokio::sync::oneshot::channel();
        sent.on_receiving_result(async move |result| {
            let _ = tx.send(result);
            Ok(())
        })
        .map_err(AcpError::Transport)?;
        match rx.await {
            Ok(Ok(_)) => Ok(()),
            Ok(Err(e)) => Err(AcpError::Transport(e)),
            Err(_) => Err(AcpError::internal(
                "terminal/release response channel dropped",
            )),
        }
    }

    fn dummy_acp_session_id() -> AcpSessionId {
        AcpSessionId::new(String::new())
    }
}

/// Bridge into [`librefang_kernel_handle::AcpTerminalClient`] so the
/// kernel can route runtime tool calls through the editor without
/// depending on the ACP schema crate.
#[async_trait]
impl AcpTerminalClient for TerminalClientHandle {
    async fn run_command(
        &self,
        command: String,
        args: Vec<String>,
        env: Vec<(String, String)>,
        cwd: Option<PathBuf>,
        output_byte_limit: Option<u64>,
    ) -> KernelResult<AcpTerminalRunResult> {
        // Five-call dance: create → wait_for_exit → output → release.
        // We need the terminal id alive across all four, and we always
        // release at the end (even on intermediate failure) so the
        // editor doesn't leak terminals.
        let sid = TerminalClientHandle::dummy_acp_session_id();
        let terminal_id = self
            .create(sid.clone(), command, args, env, cwd, output_byte_limit)
            .await
            .map_err(map_err)?;
        let result = self
            .wait_then_collect(sid.clone(), terminal_id.clone())
            .await;
        // Release regardless of success — log a warning on failure but
        // don't surface it as the call's error since the run already
        // produced a result we want to return.
        if let Err(e) = self.release(sid, terminal_id).await {
            tracing::warn!(error = %e, "ACP terminal/release failed (terminal may be leaked)");
        }
        result
    }

    fn capabilities(&self) -> bool {
        TerminalClientHandle::capabilities(self).terminal
    }
}

impl TerminalClientHandle {
    async fn wait_then_collect(
        &self,
        sid: AcpSessionId,
        terminal_id: TerminalId,
    ) -> KernelResult<AcpTerminalRunResult> {
        let exit = self
            .wait_for_exit(sid.clone(), terminal_id.clone())
            .await
            .map_err(map_err)?;
        let (output, truncated, _post_exit) =
            self.output(sid, terminal_id).await.map_err(map_err)?;
        Ok(AcpTerminalRunResult {
            output,
            truncated,
            exit_code: exit.exit_code.map(|c| c as i32),
            signal: exit.signal,
        })
    }
}

fn map_err(e: AcpError) -> KernelOpError {
    KernelOpError::Internal(e.to_string())
}
