//! Daemon-attached Agent Client Protocol (ACP) listener (#3313).
//!
//! When the daemon is running, this module exposes a Unix domain
//! socket at `~/.librefang/acp.sock` so multiple `librefang acp`
//! invocations can share one long-running kernel: one approval
//! decision is visible to all editor tabs, agent state persists
//! across the editor restarting the child process, and remembered
//! `allow_always` decisions outlive the per-invocation child.
//!
//! Each accepted UDS connection runs an isolated
//! [`librefang_acp::run_with_transport`] over the connection's
//! framed JSON-RPC stream against a shared [`KernelAdapter`] backed
//! by the daemon's existing kernel. The CLI side (
//! `librefang-cli::acp::run_uds_proxy`) becomes a transparent
//! stdin↔socket pipe, so ACP wire frames flow directly between the
//! editor and the daemon.

#![cfg(unix)]

use std::path::PathBuf;
use std::sync::Arc;

use librefang_acp::{AcpKernel, KernelAdapter};
use librefang_kernel::LibreFangKernel;
use tokio::net::{UnixListener, UnixStream};
use tokio_util::compat::{TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};
use tracing::{info, warn};

/// Default agent name when the editor doesn't specify one. Mirrors
/// the in-process CLI default (`assistant`) so behaviour is consistent
/// across modes. A future protocol extension may let the editor pick
/// per-connection — see #3313 phase-2 follow-up.
const DEFAULT_AGENT: &str = "assistant";

/// Accept connections on `sock_path` and serve ACP over each. Returns
/// only when the listener fails to bind or the loop is interrupted —
/// the daemon's bg-task harness logs and forgets in either case.
pub async fn run_listener(kernel: Arc<LibreFangKernel>, sock_path: PathBuf) -> std::io::Result<()> {
    // Clean up a stale socket from a crashed daemon. `bind` would
    // otherwise fail with `EADDRINUSE`. We ignore the remove error
    // because a missing file is the expected cold-start path.
    if sock_path.exists() {
        let _ = tokio::fs::remove_file(&sock_path).await;
    }
    if let Some(parent) = sock_path.parent() {
        let _ = tokio::fs::create_dir_all(parent).await;
    }

    let listener = UnixListener::bind(&sock_path)?;
    info!(path = %sock_path.display(), "ACP UDS listener bound");

    loop {
        let stream = match listener.accept().await {
            Ok((s, _)) => s,
            Err(e) => {
                warn!(error = %e, "ACP UDS accept failed");
                continue;
            }
        };
        let kernel = Arc::clone(&kernel);
        tokio::spawn(async move {
            if let Err(e) = handle_connection(kernel, stream).await {
                warn!(error = %e, "ACP UDS connection ended with error");
            }
        });
    }
}

async fn handle_connection(
    kernel: Arc<LibreFangKernel>,
    stream: UnixStream,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let adapter = Arc::new(KernelAdapter::new(kernel));
    let agent_id = adapter.resolve_agent(DEFAULT_AGENT).await?;

    let (read_half, write_half) = stream.into_split();
    let transport =
        agent_client_protocol::ByteStreams::new(write_half.compat_write(), read_half.compat());
    librefang_acp::run_with_transport(adapter, agent_id, transport).await?;
    Ok(())
}
