use async_trait::async_trait;

use super::*;

// ============================================================================
// 17. AcpFsBridge — editor-backed `fs/read_text_file` / `fs/write_text_file`
//
// Used by runtime tools to route file I/O through an attached ACP editor
// instead of the agent's local filesystem (#3313). The kernel maps a
// LibreFang `SessionId` back to a registered `AcpFsClient` (an opaque
// trait object the ACP adapter installs at `initialize`-time) and
// forwards the read / write request. Sessions without an attached
// editor (the dashboard / TUI / cron / channel-bridge cases) get
// `Unavailable` — runtime tools that opt into ACP backing should
// fall back to local fs in that case rather than failing the call.
// ============================================================================

/// Object-safe client side of the `fs/*` reverse-RPC. Implemented by
/// `librefang-acp::FsClientHandle`; the kernel stores
/// `Arc<dyn AcpFsClient>` per ACP session and dispatches through it.
#[async_trait]
pub trait AcpFsClient: Send + Sync {
    /// `fs/read_text_file` — return the file content as a string.
    /// `line` is 1-based per the ACP schema.
    async fn read_text_file(
        &self,
        path: std::path::PathBuf,
        line: Option<u32>,
        limit: Option<u32>,
    ) -> KernelResult<String>;

    /// `fs/write_text_file` — overwrite the file with `content`.
    async fn write_text_file(&self, path: std::path::PathBuf, content: String) -> KernelResult<()>;

    /// `(read_text_file, write_text_file)` capability snapshot the editor
    /// declared at `initialize`. Runtime tools can use this to short-
    /// circuit before paying the round-trip when the editor doesn't
    /// support the operation.
    fn capabilities(&self) -> (bool, bool);
}

/// Runtime-facing role trait for editor-backed file I/O.
#[async_trait]
pub trait AcpFsBridge: Send + Sync {
    /// Register an `fs/*` client for `session_id`, replacing any prior
    /// registration. Called by the ACP adapter once per accepted
    /// connection. Default impl is a no-op so kernel stubs without
    /// ACP support compile.
    fn register_acp_fs_client(
        &self,
        session_id: librefang_types::agent::SessionId,
        client: std::sync::Arc<dyn AcpFsClient>,
    ) {
        let _ = (session_id, client);
    }

    /// Drop the registration for `session_id`. Called when the editor
    /// disconnects so a stale handle can't keep firing requests onto
    /// a closed connection.
    fn unregister_acp_fs_client(&self, session_id: librefang_types::agent::SessionId) {
        let _ = session_id;
    }

    /// Look up the `fs/*` client registered for `session_id`. Returns
    /// `None` when no editor is bound — runtime tools should treat
    /// that as "fall back to local fs", not as a hard error.
    fn acp_fs_client(
        &self,
        session_id: librefang_types::agent::SessionId,
    ) -> Option<std::sync::Arc<dyn AcpFsClient>> {
        let _ = session_id;
        None
    }

    /// Convenience: run `fs/read_text_file` against the editor bound to
    /// `session_id`. Returns `KernelOpError::Unavailable` when no
    /// editor is bound for the session.
    async fn acp_read_text_file(
        &self,
        session_id: librefang_types::agent::SessionId,
        path: std::path::PathBuf,
        line: Option<u32>,
        limit: Option<u32>,
    ) -> KernelResult<String> {
        match self.acp_fs_client(session_id) {
            Some(client) => client.read_text_file(path, line, limit).await,
            None => Err(KernelOpError::unavailable(
                "ACP fs/read_text_file (no editor bound to session)",
            )),
        }
    }

    /// Convenience: run `fs/write_text_file` against the editor bound to
    /// `session_id`.
    async fn acp_write_text_file(
        &self,
        session_id: librefang_types::agent::SessionId,
        path: std::path::PathBuf,
        content: String,
    ) -> KernelResult<()> {
        match self.acp_fs_client(session_id) {
            Some(client) => client.write_text_file(path, content).await,
            None => Err(KernelOpError::unavailable(
                "ACP fs/write_text_file (no editor bound to session)",
            )),
        }
    }
}
