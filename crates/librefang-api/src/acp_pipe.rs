//! Daemon-attached Agent Client Protocol (ACP) listener — Windows
//! named-pipe variant of [`crate::acp_uds`] (#3313).
//!
//! Wire-compatible with the Unix UDS path: same JSON-RPC framing,
//! same per-connection `KernelAdapter`, same shared kernel. Only the
//! transport layer differs — Unix uses
//! `tokio::net::UnixListener`/`UnixStream`, Windows uses
//! `tokio::net::windows::named_pipe::NamedPipeServer`.
//!
//! Pipe naming: `\\.\pipe\librefang-acp` (host-local). The CLI side
//! (`librefang-cli::acp::run_pipe_proxy`) connects via
//! `NamedPipeClient` and pipes stdin↔pipe↔stdout.

#![cfg(windows)]

use std::sync::Arc;

use librefang_acp::{AcpKernel, KernelAdapter};
use librefang_kernel::LibreFangKernel;
use tokio::net::windows::named_pipe::{NamedPipeServer, ServerOptions};
use tokio_util::compat::{TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};
use tracing::{info, warn};

const DEFAULT_AGENT: &str = "assistant";

/// Canonical pipe name. Local-only namespace (`\\.\pipe\…`). The CLI
/// side hard-codes the same name when reaching for daemon-attached
/// mode — keep them in sync.
pub const PIPE_NAME: &str = r"\\.\pipe\librefang-acp";

/// Accept connections on the librefang ACP named pipe and serve ACP
/// over each. Returns only when the bg-task harness aborts the
/// listener loop.
pub async fn run_listener(kernel: Arc<LibreFangKernel>) -> std::io::Result<()> {
    info!(pipe = %PIPE_NAME, "ACP named-pipe listener bound");
    // Named pipes work differently from UDS — there's no single
    // listener accepting multiple `connect()` calls. Instead, the
    // server creates one pipe instance, awaits a connect, then
    // creates the next instance for the next connection. The
    // `first_pipe_instance(true)` on the very first server prevents
    // a second daemon from racing in on the same name.
    let mut server = ServerOptions::new()
        .first_pipe_instance(true)
        .create(PIPE_NAME)?;

    loop {
        if let Err(e) = server.connect().await {
            warn!(error = %e, "ACP named-pipe connect failed");
            // Drop the busted instance and rebind for the next
            // attempt. Returning here would tear the listener down
            // for the rest of the daemon's lifetime.
            server = ServerOptions::new().create(PIPE_NAME)?;
            continue;
        }
        // Hand the connected pipe off to a background task and
        // immediately create the next instance so the next CLI
        // invocation isn't blocked behind this connection's
        // lifetime.
        let connected = server;
        server = ServerOptions::new().create(PIPE_NAME)?;
        let kernel = Arc::clone(&kernel);
        tokio::spawn(async move {
            if let Err(e) = handle_connection(kernel, connected).await {
                warn!(error = %e, "ACP named-pipe connection ended with error");
            }
        });
    }
}

async fn handle_connection(
    kernel: Arc<LibreFangKernel>,
    stream: NamedPipeServer,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let adapter = Arc::new(KernelAdapter::new(kernel));
    let agent_id = adapter.resolve_agent(DEFAULT_AGENT).await?;

    // `tokio::io::split` for `NamedPipeServer` returns `ReadHalf` /
    // `WriteHalf` that implement `AsyncRead` / `AsyncWrite`. The
    // ACP transport wants `futures::AsyncRead` / `AsyncWrite`, so we
    // adapt through `tokio_util::compat`.
    let (read_half, write_half) = tokio::io::split(stream);
    let transport =
        agent_client_protocol::ByteStreams::new(write_half.compat_write(), read_half.compat());
    librefang_acp::run_with_transport(adapter, agent_id, transport).await?;
    Ok(())
}
