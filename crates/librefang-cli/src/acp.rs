//! `librefang acp` subcommand — runs the Agent Client Protocol server
//! over stdio so editors like Zed / VS Code / JetBrains can embed
//! LibreFang as a native agent (#3313).
//!
//! Two modes:
//!
//! * **In-process** (no daemon): boot a fresh kernel in this process,
//!   serve ACP on stdio until stdin EOF.
//! * **Daemon-attached** (UDS proxy): when the daemon is running and
//!   has the ACP UDS listener up at `<home>/acp.sock` (the daemon's home, which honours `LIBREFANG_HOME` and `--config`), this
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
    // Resolve the daemon's home the same way `start` / `stop` do, so a relocated home finds the daemon's socket instead of booting a second kernel against the same database.
    let home_dir = crate::commands::common::daemon_config_context(config.as_deref()).home_dir;
    let daemon_up = crate::commands::common::find_daemon_in_home(&home_dir).is_some();

    // The daemon-side listener resolves only the default agent; attaching would silently serve a different agent than the one asked for.
    if daemon_up && !attached_mode_serves(agent.as_deref()) {
        eprintln!(
            "{}",
            crate::i18n::t_args(
                "acp-error-agent-attached",
                &[
                    ("name", agent.as_deref().unwrap_or(DEFAULT_AGENT_NAME)),
                    ("default", DEFAULT_AGENT_NAME)
                ]
            )
        );
        std::process::exit(1);
    }

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
    if let Some(sock) = daemon_up.then(|| acp_socket_in_home(&home_dir)).flatten() {
        eprintln!(
            "{}",
            crate::i18n::t_args("acp-attached-uds", &[("path", &sock.to_string_lossy())])
        );
        let exit_code = run_uds_proxy(&sock);
        if exit_code != 0 {
            std::process::exit(exit_code);
        }
        return;
    }
    #[cfg(windows)]
    if daemon_up {
        eprintln!("{}", crate::i18n::t("acp-attached-pipe"));
        let exit_code = run_pipe_proxy();
        if exit_code != 0 {
            std::process::exit(exit_code);
        }
        return;
    }
    eprintln!("{}", crate::i18n::t("acp-in-process"));

    let kernel = match LibreFangKernel::boot(config.as_deref()) {
        Ok(k) => Arc::new(k),
        Err(e) => {
            eprintln!(
                "{}",
                crate::i18n::t_args("acp-error-boot-kernel", &[("error", &e.to_string())])
            );
            std::process::exit(1);
        }
    };

    let rt = tokio::runtime::Runtime::new().expect("Failed to create Tokio runtime");
    let exit_code = rt.block_on(async {
        // `boot` leaves `self_handle` empty — it is the caller's job to fill it
        // once the kernel is in an `Arc`, the same way `server.rs` and
        // `desktop/src/server.rs` do. `KernelAdapter::new` below reads
        // `kernel_handle()` immediately, and that accessor is an `.expect`, so
        // without this the in-process ACP backend aborts before serving a
        // single request.
        kernel.clone().set_self_handle();
        kernel.clone().spawn_approval_sweep_task();

        let agent_name = agent.as_deref().unwrap_or(DEFAULT_AGENT_NAME);
        let adapter = KernelAdapter::new(Arc::clone(&kernel));
        let agent_id = match adapter.resolve_agent(agent_name).await {
            Ok(id) => id,
            Err(e) => {
                eprintln!(
                    "{}",
                    crate::i18n::t_args(
                        "acp-error-resolve-agent",
                        &[("name", agent_name), ("error", &e.to_string())]
                    )
                );
                return 1;
            }
        };

        match librefang_acp::run(Arc::new(adapter), agent_id).await {
            Ok(()) => 0,
            Err(e) => {
                eprintln!(
                    "{}",
                    crate::i18n::t_args("acp-error-server", &[("error", &e.to_string())])
                );
                1
            }
        }
    });

    if exit_code != 0 {
        std::process::exit(exit_code);
    }
}

/// Whether the daemon-attached mode serves the requested agent: its listener always resolves [`DEFAULT_AGENT_NAME`].
fn attached_mode_serves(agent: Option<&str>) -> bool {
    agent.is_none_or(|name| name == DEFAULT_AGENT_NAME)
}

/// The daemon-side ACP UDS under `home_dir`, where the daemon binds it, if the socket file exists.
/// The caller checks the daemon is reachable first, so a stale socket from a crashed daemon falls back to in-process mode.
///
/// Unix-only because the daemon-side listener is itself Unix-only;
/// the sole call site is the `#[cfg(unix)]` fast-path branch in
/// `run_acp_server`. Windows / non-Unix targets skip straight to the
/// in-process path, so no stub is needed.
#[cfg(unix)]
fn acp_socket_in_home(home_dir: &std::path::Path) -> Option<PathBuf> {
    let path = home_dir.join("acp.sock");
    path.exists().then_some(path)
}

/// How long the proxy keeps forwarding the daemon's output after stdin closes, so replies to the editor's last requests still reach it.
const PROXY_DRAIN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// Drive both proxy directions until the session ends.
///
/// The socket closing ends it at once; stdin closing (the editor is done sending) lets the outbound side drain for up to `drain_timeout`, since the daemon may still be writing responses to the final requests.
async fn proxy_until_drained(
    inbound: impl std::future::Future<Output = ()>,
    outbound: impl std::future::Future<Output = ()>,
    drain_timeout: std::time::Duration,
) {
    let mut inbound = std::pin::pin!(inbound);
    let mut outbound = std::pin::pin!(outbound);
    tokio::select! {
        _ = &mut inbound => {
            let _ = tokio::time::timeout(drain_timeout, &mut outbound).await;
        }
        _ = &mut outbound => {}
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
                eprintln!(
                    "{}",
                    crate::i18n::t_args(
                        "acp-error-uds-connect",
                        &[
                            ("path", &sock_path.to_string_lossy()),
                            ("error", &e.to_string())
                        ]
                    )
                );
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

        proxy_until_drained(inbound, outbound, PROXY_DRAIN_TIMEOUT).await;
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
                eprintln!(
                    "{}",
                    crate::i18n::t_args(
                        "acp-error-pipe-connect",
                        &[("name", PIPE_NAME), ("error", &e.to_string())]
                    )
                );
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

        proxy_until_drained(inbound, outbound, PROXY_DRAIN_TIMEOUT).await;
        0
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Attaching to the daemon with `--agent coder` used to chat with `assistant` silently.
    #[test]
    fn attached_mode_serves_only_the_default_agent() {
        assert!(attached_mode_serves(None));
        assert!(attached_mode_serves(Some(DEFAULT_AGENT_NAME)));
        assert!(!attached_mode_serves(Some("coder")));
    }

    /// The socket was looked up under a hard-coded `~/.librefang`, missing a daemon whose home is relocated.
    #[cfg(unix)]
    #[test]
    fn acp_socket_is_found_under_the_daemon_home() {
        let home = tempfile::TempDir::new().unwrap();
        assert_eq!(acp_socket_in_home(home.path()), None);
        std::fs::write(home.path().join("acp.sock"), b"").unwrap();
        assert_eq!(
            acp_socket_in_home(home.path()),
            Some(home.path().join("acp.sock"))
        );
    }

    /// `select!` used to cancel the outbound copy the moment stdin hit EOF, dropping the daemon's final responses.
    #[tokio::test]
    async fn proxy_drains_outbound_after_stdin_eof() {
        let (tx, rx) = tokio::sync::oneshot::channel();
        let outbound = async move {
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            let _ = tx.send("final response");
        };
        proxy_until_drained(async {}, outbound, std::time::Duration::from_secs(5)).await;
        assert_eq!(rx.await.ok(), Some("final response"));
    }

    #[tokio::test]
    async fn proxy_stops_waiting_after_the_drain_timeout() {
        let started = std::time::Instant::now();
        proxy_until_drained(
            async {},
            std::future::pending::<()>(),
            std::time::Duration::from_millis(20),
        )
        .await;
        assert!(started.elapsed() < std::time::Duration::from_secs(5));
    }
}
