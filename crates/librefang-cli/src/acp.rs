//! `librefang acp` subcommand — runs the Agent Client Protocol server
//! over stdio so editors like Zed / VS Code / JetBrains can embed
//! LibreFang as a native agent (#3313).
//!
//! Two modes:
//!
//! * **In-process** (no daemon): boot a fresh kernel in this process,
//!   serve ACP on stdio until stdin EOF.
//! * **Daemon-attached** (UDS proxy): when the daemon is running and
//!   has the ACP UDS listener up at `~/.librefang/acp.sock`, this
//!   binary becomes a thin bidirectional pipe — stdin → socket,
//!   socket → stdout. The daemon-side ACP server uses its own
//!   long-running kernel, so multiple editors can share state, agent
//!   history, and remembered approval decisions.

use std::path::PathBuf;
use std::sync::Arc;

use librefang_acp::{AcpKernel, KernelAdapter};
use librefang_kernel::LibreFangKernel;

/// Default agent name when the CLI is invoked without `--agent`.
/// Mirrors the dashboard / TUI default so editor users land on the
/// same agent they see elsewhere.
const DEFAULT_AGENT_NAME: &str = "assistant";

/// Boot an in-process kernel and run the ACP server on stdio until
/// stdin EOF. Or, if the daemon is up and the UDS listener is
/// available, switch to proxy mode.
pub fn run_acp_server(config: Option<PathBuf>, agent: Option<String>) {
    // Fast path: daemon already hosting an ACP listener — just be a
    // transparent stdio↔socket proxy. The daemon-side kernel is shared
    // across every concurrent `librefang acp` client, so editor tabs
    // see a consistent agent state. Unix uses a UDS, Windows uses a
    // named pipe.
    //
    // We log which mode we picked to stderr so the user can tell which
    // kernel is backing this invocation. This matters because in-process
    // and daemon-attached have *different* `allow_always` caches: an
    // approval the user remembered in an earlier in-process run does
    // NOT carry over once the daemon comes up, and vice-versa. Without
    // this hint, "why did my agent forget I approved this tool?" is an
    // unfindable bug. (#3313 review, M2)
    #[cfg(unix)]
    if let Some(sock) = locate_acp_socket() {
        eprintln!("librefang acp: attached to daemon (UDS {})", sock.display());
        let exit_code = run_uds_proxy(&sock);
        if exit_code != 0 {
            std::process::exit(exit_code);
        }
        return;
    }
    #[cfg(windows)]
    if super::find_daemon().is_some() {
        eprintln!("librefang acp: attached to daemon (named pipe)");
        let exit_code = run_pipe_proxy();
        if exit_code != 0 {
            std::process::exit(exit_code);
        }
        return;
    }
    eprintln!("librefang acp: in-process kernel (no daemon detected)");

    let kernel = match LibreFangKernel::boot(config.as_deref()) {
        Ok(k) => Arc::new(k),
        Err(e) => {
            eprintln!("Failed to boot kernel: {e}");
            std::process::exit(1);
        }
    };

    let rt = tokio::runtime::Runtime::new().expect("Failed to create Tokio runtime");
    let exit_code = rt.block_on(async {
        kernel.clone().spawn_approval_sweep_task();

        let agent_name = agent.as_deref().unwrap_or(DEFAULT_AGENT_NAME);
        let adapter = KernelAdapter::new(Arc::clone(&kernel));
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

/// Look for a daemon-side ACP UDS at the canonical path. Returns the
/// path only if the daemon is reachable AND the socket file exists —
/// a stale socket from a crashed daemon falls back to in-process mode.
/// Look for a daemon-side ACP UDS at the canonical path. Returns the
/// path only if the daemon is reachable AND the socket file exists —
/// a stale socket from a crashed daemon falls back to in-process mode.
///
/// Unix-only because the daemon-side listener is itself Unix-only;
/// the sole call site is the `#[cfg(unix)]` fast-path branch in
/// `run_acp_server`. Windows / non-Unix targets skip straight to the
/// in-process path, so no stub is needed.
#[cfg(unix)]
fn locate_acp_socket() -> Option<PathBuf> {
    super::find_daemon()?;
    let path = dirs::home_dir()?.join(".librefang").join("acp.sock");
    if path.exists() {
        Some(path)
    } else {
        None
    }
}

/// Bidirectional stdin↔socket↔stdout pipe. Returns 0 on clean EOF, 1
/// otherwise.
#[cfg(unix)]
fn run_uds_proxy(sock_path: &std::path::Path) -> i32 {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::UnixStream;

    let rt = tokio::runtime::Runtime::new().expect("Failed to create Tokio runtime");
    rt.block_on(async {
        let stream = match UnixStream::connect(sock_path).await {
            Ok(s) => s,
            Err(e) => {
                eprintln!("ACP UDS connect failed at {}: {e}", sock_path.display());
                return 1;
            }
        };
        let (mut sock_read, mut sock_write) = stream.into_split();
        let mut stdin = tokio::io::stdin();
        let mut stdout = tokio::io::stdout();

        // Inbound: stdin → socket
        let inbound = async move {
            let mut buf = vec![0u8; 8192];
            loop {
                match stdin.read(&mut buf).await {
                    Ok(0) => break, // EOF on stdin
                    Ok(n) => {
                        if sock_write.write_all(&buf[..n]).await.is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
            let _ = sock_write.shutdown().await;
        };

        // Outbound: socket → stdout
        let outbound = async move {
            let mut buf = vec![0u8; 8192];
            loop {
                match sock_read.read(&mut buf).await {
                    Ok(0) => break, // socket closed
                    Ok(n) => {
                        if stdout.write_all(&buf[..n]).await.is_err() {
                            break;
                        }
                        let _ = stdout.flush().await;
                    }
                    Err(_) => break,
                }
            }
        };

        // Either direction closing ends the session. Use `tokio::join!`
        // to allow the slower side to drain before we exit.
        tokio::select! {
            _ = inbound => {}
            _ = outbound => {}
        }
        0
    })
}

/// Bidirectional stdin↔named-pipe↔stdout pipe (Windows). Returns 0 on
/// clean EOF, 1 if the pipe couldn't be reached. The daemon must
/// already be hosting `\\.\pipe\librefang-acp` for this to succeed —
/// `find_daemon()` is checked at the call site.
#[cfg(windows)]
fn run_pipe_proxy() -> i32 {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::windows::named_pipe::ClientOptions;

    const PIPE_NAME: &str = r"\\.\pipe\librefang-acp";

    let rt = tokio::runtime::Runtime::new().expect("Failed to create Tokio runtime");
    rt.block_on(async {
        let stream = match ClientOptions::new().open(PIPE_NAME) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("ACP named-pipe connect failed at {PIPE_NAME}: {e}");
                return 1;
            }
        };
        let (mut sock_read, mut sock_write) = tokio::io::split(stream);
        let mut stdin = tokio::io::stdin();
        let mut stdout = tokio::io::stdout();

        let inbound = async move {
            let mut buf = vec![0u8; 8192];
            loop {
                match stdin.read(&mut buf).await {
                    Ok(0) => break,
                    Ok(n) => {
                        if sock_write.write_all(&buf[..n]).await.is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
            let _ = sock_write.shutdown().await;
        };

        let outbound = async move {
            let mut buf = vec![0u8; 8192];
            loop {
                match sock_read.read(&mut buf).await {
                    Ok(0) => break,
                    Ok(n) => {
                        if stdout.write_all(&buf[..n]).await.is_err() {
                            break;
                        }
                        let _ = stdout.flush().await;
                    }
                    Err(_) => break,
                }
            }
        };

        tokio::select! {
            _ = inbound => {}
            _ = outbound => {}
        }
        0
    })
}
