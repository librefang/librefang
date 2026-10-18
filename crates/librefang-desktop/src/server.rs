//! Kernel lifecycle management for the desktop app.
//!
//! Boots the LibreFang kernel, binds to a random localhost port, and runs the
//! API server on a background thread with its own tokio runtime.
//!
//! Desktop-only: this module is not compiled on iOS or Android (mobile is a thin client).

#![cfg(not(any(target_os = "ios", target_os = "android")))]

use librefang_api::server::build_router;
use librefang_kernel::LibreFangKernel;
use std::net::{SocketAddr, TcpListener};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::watch;
use tracing::{error, info, warn};

/// Handle to the running embedded server. Drop or call `shutdown()` to stop.
pub struct ServerHandle {
    /// The port the server is listening on.
    pub port: u16,
    /// The kernel instance (shared with the server).
    pub kernel: Arc<LibreFangKernel>,
    /// Send `true` to trigger graceful shutdown.
    shutdown_tx: watch::Sender<bool>,
    /// Join handle for the background server thread.
    server_thread: Option<std::thread::JoinHandle<()>>,
    /// Track whether shutdown has already been initiated to prevent double shutdown.
    shutdown_initiated: Arc<AtomicBool>,
}

impl ServerHandle {
    /// Signal the server to shut down, wait up to [`SERVER_STOP_TIMEOUT`] for the background thread, then shut the kernel down.
    ///
    /// The wait is bounded because axum's graceful shutdown waits for every open connection to close, and a dashboard WebSocket or SSE stream in a still-open webview never does; an unbounded join would hang Quit and the updater's restart.
    pub fn shutdown(mut self) {
        // Only proceed if shutdown hasn't been initiated yet
        if self
            .shutdown_initiated
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::Relaxed)
            .is_ok()
        {
            let _ = self.shutdown_tx.send(true);
            if let Some(handle) = self.server_thread.take() {
                if !join_with_timeout(handle, SERVER_STOP_TIMEOUT) {
                    warn!(
                        timeout_secs = SERVER_STOP_TIMEOUT.as_secs(),
                        "Embedded server did not stop in time; shutting the kernel down anyway"
                    );
                }
            }
            self.kernel.shutdown();
            info!("LibreFang embedded server stopped");
        }
    }
}

/// How long [`ServerHandle::shutdown`] waits for the server thread before shutting the kernel down regardless.
const SERVER_STOP_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// Join `handle`, giving up after `timeout`; returns whether the thread finished in time.
fn join_with_timeout(handle: std::thread::JoinHandle<()>, timeout: std::time::Duration) -> bool {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = handle.join();
        let _ = tx.send(());
    });
    rx.recv_timeout(timeout).is_ok()
}

impl Drop for ServerHandle {
    fn drop(&mut self) {
        // Only send shutdown signal if it hasn't been initiated yet
        if self
            .shutdown_initiated
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::Relaxed)
            .is_ok()
        {
            let _ = self.shutdown_tx.send(true);
            // Best-effort: don't block in drop, the thread will exit on its own.
        }
    }
}

/// Boot the kernel and start the embedded API server on a background thread.
///
/// Binds to `127.0.0.1:0` on the calling thread so the port is known before
/// any Tauri window is created. The actual axum server runs on a dedicated
/// thread with its own tokio runtime.
pub fn start_server() -> Result<ServerHandle, Box<dyn std::error::Error>> {
    // Boot kernel (sync — no tokio needed)
    let kernel = LibreFangKernel::boot(None)?;
    let kernel = Arc::new(kernel);
    kernel.set_self_handle();

    // Bind to a random free port on localhost (main thread — guarantees port)
    let std_listener = TcpListener::bind("127.0.0.1:0")?;
    let port = std_listener.local_addr()?.port();
    let listen_addr: SocketAddr = std_listener.local_addr()?;

    info!("LibreFang embedded server bound to http://127.0.0.1:{port}");

    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let kernel_clone = kernel.clone();
    let shutdown_initiated = Arc::new(AtomicBool::new(false));

    let server_thread = std::thread::Builder::new()
        .name("librefang-server".into())
        // This thread runs `block_on`, so the outermost turn is polled on *its* stack — which `Builder::thread_stack_size` below does not cover, that setting applies to the runtime's worker/blocking threads.
        // Size both (refs #6659): the desktop app embeds the same kernel as the daemon, so it stacks the same agent-turn chain and aborts the same way.
        .stack_size(8 * 1024 * 1024)
        .spawn(move || {
            let rt = tokio::runtime::Builder::new_multi_thread()
                // Same 2 MiB -> 8 MiB worker stack rationale as the daemon (`librefang-cli/src/commands/daemon.rs`): an `agent_send` or workflow step runs the callee's whole turn inline on the worker, restacking a chain of very large futures.
                .thread_stack_size(8 * 1024 * 1024)
                .enable_all()
                .build()
                .expect("Failed to create tokio runtime for embedded server");

            rt.block_on(async move {
                // start_background_agents() uses tokio::spawn/bootstrap async work, so it must
                // run inside a tokio runtime context.
                kernel_clone.start_background_agents().await;
                // Spawn approval expiry sweep task
                kernel_clone.clone().spawn_approval_sweep_task();
                run_embedded_server(kernel_clone, std_listener, listen_addr, shutdown_rx).await;
            });
        })?;

    Ok(ServerHandle {
        port,
        kernel,
        shutdown_tx,
        server_thread: Some(server_thread),
        shutdown_initiated,
    })
}

/// Run the axum server inside a tokio runtime, shut down when the watch
/// channel fires.
async fn run_embedded_server(
    kernel: Arc<LibreFangKernel>,
    std_listener: TcpListener,
    listen_addr: SocketAddr,
    mut shutdown_rx: watch::Receiver<bool>,
) {
    let (app, state) = build_router(kernel.clone(), listen_addr).await;

    // The standalone API server starts provider validation after building the router.
    // Desktop embeds the router directly, so it must start the same task here or live provider catalogs (notably OpenRouter) never refresh.
    kernel.clone().spawn_key_validation();

    // Sync dashboard assets in background (downloads from release if outdated).
    // Keep the handle so shutdown does not destroy the embedded runtime while
    // the one-shot install is still updating the dashboard directory.
    let dashboard_sync = {
        let home = kernel.home_dir().to_path_buf();
        tokio::spawn(async move {
            librefang_api::webchat::sync_dashboard(&home).await;
        })
    };

    // Convert std TcpListener → tokio TcpListener
    std_listener
        .set_nonblocking(true)
        .expect("Failed to set listener to non-blocking");
    let listener = tokio::net::TcpListener::from_std(std_listener)
        .expect("Failed to convert std TcpListener to tokio");

    info!("LibreFang embedded server listening on http://{listen_addr}");

    let server = axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(async move {
        let _ = shutdown_rx.wait_for(|v| *v).await;
        info!("Embedded server received shutdown signal");
    });

    if let Err(e) = server.await {
        error!("Embedded server error: {e}");
    }

    if let Err(e) = dashboard_sync.await {
        error!("Dashboard sync task failed: {e}");
    }

    // Clean up channel bridges. Swap out atomically then stop with owned access.
    {
        let old = state.bridge_manager.swap(std::sync::Arc::new(None));
        if let Ok(Some(ref mut b)) = std::sync::Arc::try_unwrap(old) {
            b.stop().await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::join_with_timeout;
    use std::time::Duration;

    #[test]
    fn join_with_timeout_reports_a_finished_thread() {
        let handle = std::thread::spawn(|| {});
        assert!(join_with_timeout(handle, Duration::from_secs(5)));
    }

    /// A server thread stuck in graceful shutdown (an open dashboard WebSocket) must not block Quit forever.
    #[test]
    fn join_with_timeout_gives_up_on_a_stuck_thread() {
        let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
        let handle = std::thread::spawn(move || {
            let _ = release_rx.recv();
        });
        assert!(!join_with_timeout(handle, Duration::from_millis(50)));
        let _ = release_tx.send(());
    }
}
