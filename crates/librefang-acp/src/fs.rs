//! `fs/*` reverse-RPC helpers (#3313).
//!
//! The Agent Client Protocol exposes `fs/read_text_file` and
//! `fs/write_text_file` as **agent → client** requests: the editor is
//! the file source, not the agent's local filesystem. From inside a
//! LibreFang tool the runtime needs to be able to "read this file"
//! and have the request flow over the same JSON-RPC connection that
//! the editor opened, so the editor reads the file with whatever
//! authority it has (current buffer, in-memory edits, virtual
//! filesystems, …) instead of the agent process's view of the disk.
//!
//! This module hides the protocol crate's transport types behind a
//! pair of small async APIs:
//!
//! * [`FsClientHandle`] — a thin wrapper around
//!   [`agent_client_protocol::ConnectionTo<Client>`] that exposes
//!   `read_text_file` / `write_text_file` and the editor's declared
//!   capabilities. Cloneable so each agent turn can grab a snapshot
//!   without re-touching the connection.
//! * [`FsCapabilities`] — flat bools mirroring the ACP
//!   `FileSystemCapabilities` schema, so consumers don't have to
//!   reach into the protocol crate.
//!
//! The `kernel-adapter` feature wires these into [`crate::KernelAdapter`]
//! so the LibreFang kernel can route a runtime tool call back through
//! the editor.

use std::path::PathBuf;
use std::sync::Arc;

use agent_client_protocol::schema::{
    ClientCapabilities, ReadTextFileRequest, WriteTextFileRequest,
};
use agent_client_protocol::Client;
use agent_client_protocol::ConnectionTo;

use crate::AcpError;

/// Editor-declared filesystem capabilities, captured at `initialize` time.
///
/// Mirrors [`agent_client_protocol::schema::FileSystemCapabilities`]
/// but as a flat plain-old-data struct so callers in `librefang-kernel`
/// and `librefang-runtime` don't pull in the ACP schema crate.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FsCapabilities {
    /// Editor accepts `fs/read_text_file` requests.
    pub read_text_file: bool,
    /// Editor accepts `fs/write_text_file` requests.
    pub write_text_file: bool,
}

impl FsCapabilities {
    pub(crate) fn from_client(caps: &ClientCapabilities) -> Self {
        Self {
            read_text_file: caps.fs.read_text_file,
            write_text_file: caps.fs.write_text_file,
        }
    }
}

/// Handle that issues `fs/*` requests to the connected ACP client.
///
/// Internally holds an [`Arc`] over the protocol connection so it can
/// be cloned freely into per-session task spawns. The [`FsCapabilities`]
/// snapshot lets the runtime decide up front whether the editor will
/// even accept the request — calling `read_text_file` on an editor
/// that didn't advertise the capability still works (the protocol
/// permits it) but most clients will reply with
/// `method_not_found`, so the runtime can short-circuit when it
/// already knows the capability is off.
#[derive(Clone)]
pub struct FsClientHandle {
    inner: Arc<FsClientInner>,
}

struct FsClientInner {
    cx: ConnectionTo<Client>,
    caps: FsCapabilities,
}

impl FsClientHandle {
    pub(crate) fn new(cx: ConnectionTo<Client>, caps: FsCapabilities) -> Self {
        Self {
            inner: Arc::new(FsClientInner { cx, caps }),
        }
    }

    /// Editor-declared capabilities. Returned by value — callers that
    /// hold the handle long-term should re-read on each turn since a
    /// future Phase-2 reinitialize might update them.
    pub fn capabilities(&self) -> FsCapabilities {
        self.inner.caps
    }

    /// Issue an `fs/read_text_file` request and await the response.
    ///
    /// `line` is 1-based per the ACP schema; both `line` and `limit`
    /// are optional. Errors propagate the protocol-level error
    /// verbatim — an editor that doesn't support the capability will
    /// typically return JSON-RPC `method_not_found`.
    pub async fn read_text_file(
        &self,
        session_id: agent_client_protocol::schema::SessionId,
        path: PathBuf,
        line: Option<u32>,
        limit: Option<u32>,
    ) -> Result<String, AcpError> {
        let mut req = ReadTextFileRequest::new(session_id, path);
        req.line = line;
        req.limit = limit;
        let sent = self.inner.cx.send_request(req);
        let (tx, rx) = tokio::sync::oneshot::channel();
        sent.on_receiving_result(async move |result| {
            let _ = tx.send(result);
            Ok(())
        })
        .map_err(AcpError::Transport)?;
        match rx.await {
            Ok(Ok(resp)) => Ok(resp.content),
            Ok(Err(e)) => Err(AcpError::Transport(e)),
            Err(_) => Err(AcpError::internal(
                "fs/read_text_file response channel dropped",
            )),
        }
    }

    /// Issue an `fs/write_text_file` request and await the response.
    pub async fn write_text_file(
        &self,
        session_id: agent_client_protocol::schema::SessionId,
        path: PathBuf,
        content: String,
    ) -> Result<(), AcpError> {
        let req = WriteTextFileRequest::new(session_id, path, content);
        let sent = self.inner.cx.send_request(req);
        let (tx, rx) = tokio::sync::oneshot::channel();
        sent.on_receiving_result(async move |result| {
            let _ = tx.send(result);
            Ok(())
        })
        .map_err(AcpError::Transport)?;
        match rx.await {
            Ok(Ok(_resp)) => Ok(()),
            Ok(Err(e)) => Err(AcpError::Transport(e)),
            Err(_) => Err(AcpError::internal(
                "fs/write_text_file response channel dropped",
            )),
        }
    }
}
