//! LibreFang Desktop — Native Tauri 2.0 wrapper for the LibreFang Agent OS.
//!
//! Boots the kernel + embedded API server, then opens a native window pointing
//! at the WebUI. Includes system tray, single-instance enforcement, native OS
//! notifications, global shortcuts, auto-start, and update checking.
//!
//! Supports remote server mode: connect to a running LibreFang instance instead
//! of always starting a local one. Priority: CLI arg > env var > saved pref > connection screen.

mod commands;
mod connection;
mod dotenv;
mod server;
mod shortcuts;
mod tray;
mod updater;

use librefang_kernel::LibreFangKernel;
use librefang_types::event::{EventPayload, LifecycleEvent, SystemEvent};
use std::sync::Arc;
use std::time::Instant;
use tauri::{Manager, WebviewUrl, WebviewWindowBuilder};
use tauri_plugin_notification::NotificationExt;
use tracing::{info, warn};

/// Managed state: the port the embedded server listens on.
/// Wrapped in `RwLock<Option<_>>` — `None` when running in remote mode or before local boot.
pub struct PortState(pub std::sync::RwLock<Option<u16>>);

/// Inner data for `KernelState`.
pub struct KernelInner {
    pub kernel: Arc<LibreFangKernel>,
    pub started_at: Instant,
}

/// Managed state: the kernel instance and startup time.
/// Wrapped in `RwLock<Option<_>>` — `None` when running in remote mode or before local boot.
pub struct KernelState(pub std::sync::RwLock<Option<KernelInner>>);

/// Managed state: the server URL (local or remote) the WebView points at.
/// Uses interior mutability so it can be updated when the user changes servers.
pub struct ServerUrlState(pub std::sync::RwLock<String>);

/// Managed state: whether the app is connected to a remote server.
/// Uses interior mutability so it can be updated when the user changes servers.
pub struct RemoteMode(pub std::sync::RwLock<bool>);

/// Managed state: holds the `ServerHandle` for shutdown when running in local mode.
/// Wrapped in a `Mutex<Option<_>>` so it can be filled after app setup (from the
/// `start_local` command) and taken on app exit.
pub struct ServerHandleHolder(pub std::sync::Mutex<Option<server::ServerHandle>>);

/// Forward critical kernel events as native OS notifications.
///
/// Only truly critical events — crashes, hard quota limits, and kernel shutdown.
pub async fn forward_kernel_events(
    app_handle: tauri::AppHandle,
    event_rx: &mut tokio::sync::broadcast::Receiver<librefang_types::event::Event>,
) {
    loop {
        match event_rx.recv().await {
            Ok(event) => {
                let (title, body) = match &event.payload {
                    EventPayload::Lifecycle(LifecycleEvent::Crashed { agent_id, error }) => (
                        "Agent Crashed".to_string(),
                        format!("Agent {agent_id} crashed: {error}"),
                    ),
                    EventPayload::System(SystemEvent::KernelStopping) => (
                        "Kernel Stopping".to_string(),
                        "LibreFang kernel is shutting down".to_string(),
                    ),
                    EventPayload::System(SystemEvent::QuotaEnforced {
                        agent_id,
                        spent,
                        limit,
                    }) => (
                        "Quota Enforced".to_string(),
                        format!("Agent {agent_id} quota hit: ${spent:.4} / ${limit:.4}"),
                    ),
                    _ => continue,
                };

                if let Err(e) = app_handle
                    .notification()
                    .builder()
                    .title(&title)
                    .body(&body)
                    .show()
                {
                    warn!("Failed to send desktop notification: {e}");
                }
            }
            Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                warn!("Notification listener lagged, skipped {n} events");
            }
            Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                info!("Event bus closed, stopping notification listener");
                break;
            }
        }
    }
}

/// Resolved startup mode.
enum StartupMode {
    /// Connect directly to a remote server URL (skip connection screen).
    Remote(String),
    /// Boot a local server (skip connection screen).
    Local,
    /// Show the connection screen and let the user decide.
    ConnectionScreen,
}

/// Entry point for the Tauri application.
///
/// `server_url` — CLI `--server-url` override (remote mode).
/// `force_local` — CLI `--local` flag (skip connection screen, start local).
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run(server_url: Option<String>, force_local: bool) {
    // Init tracing
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "librefang=info,tauri=info".into()),
        )
        .init();

    info!("Starting LibreFang Desktop...");

    // Load ~/.librefang/.env into process environment (system env takes priority).
    dotenv::load_dotenv();

    // Resolve connection mode (priority: CLI > env var > saved preference > connection screen)
    let mode = if let Some(ref url) = server_url {
        StartupMode::Remote(url.trim_end_matches('/').to_string())
    } else if force_local {
        StartupMode::Local
    } else if let Some(url) = std::env::var("LIBREFANG_SERVER_URL")
        .ok()
        .filter(|s| !s.is_empty())
    {
        StartupMode::Remote(url.trim_end_matches('/').to_string())
    } else if let Some(pref) = connection::load_saved_preference() {
        match pref.mode.as_str() {
            "remote" if pref.server_url.is_some() => {
                StartupMode::Remote(pref.server_url.unwrap().trim_end_matches('/').to_string())
            }
            "local" => StartupMode::Local,
            _ => StartupMode::ConnectionScreen,
        }
    } else {
        StartupMode::ConnectionScreen
    };

    // For direct modes (remote or forced local), resolve URL + optional server handle now.
    let (initial_url, server_handle, is_remote) = match &mode {
        StartupMode::Remote(url) => {
            if !url.starts_with("http://") && !url.starts_with("https://") {
                eprintln!("Server URL must use http:// or https://, got: {url}");
                std::process::exit(1);
            }
            info!("Remote mode: connecting to {url}");
            (url.clone(), None, true)
        }
        StartupMode::Local => {
            let handle = server::start_server().expect("Failed to start LibreFang server");
            let port = handle.port;
            info!("LibreFang server running on port {port}");
            (format!("http://127.0.0.1:{port}"), Some(handle), false)
        }
        StartupMode::ConnectionScreen => {
            // Will be resolved later via the connection screen IPC commands.
            (String::new(), None, false)
        }
    };

    let show_connection_screen = matches!(mode, StartupMode::ConnectionScreen);

    let mut builder = tauri::Builder::default()
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_dialog::init());

    // Desktop-only plugins
    #[cfg(desktop)]
    {
        builder = builder.plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            // Another instance tried to launch — focus the existing window
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.show();
                let _ = w.unminimize();
                let _ = w.set_focus();
            }
        }));

        builder = builder.plugin(
            tauri_plugin_autostart::Builder::new()
                .args(["--minimized"])
                .build(),
        );

        builder = builder.plugin(tauri_plugin_updater::Builder::new().build());

        // Global shortcuts — non-fatal on registration failure
        match shortcuts::build_shortcut_plugin() {
            Ok(plugin) => {
                builder = builder.plugin(plugin);
            }
            Err(e) => {
                warn!("Failed to register global shortcuts: {e}");
            }
        }
    }

    // Always register the ServerHandleHolder so start_local can fill it later.
    let holder = ServerHandleHolder(std::sync::Mutex::new(server_handle));

    // Pre-compute initial values for interior-mutable state.
    let (init_port, init_kernel_inner, init_url, init_remote) = match &mode {
        StartupMode::Remote(_) => (None, None, initial_url.clone(), true),
        StartupMode::Local => {
            let guard = holder.0.lock().expect("ServerHandleHolder lock poisoned");
            let (p, k) = if let Some(ref handle) = *guard {
                (
                    Some(handle.port),
                    Some(KernelInner {
                        kernel: handle.kernel.clone(),
                        started_at: Instant::now(),
                    }),
                )
            } else {
                (None, None)
            };
            drop(guard);
            (p, k, initial_url.clone(), false)
        }
        StartupMode::ConnectionScreen => (None, None, String::new(), false),
    };

    // Register ALL state types ONCE with initial values. Updates go through
    // interior-mutable RwLocks — Tauri `manage()` is a no-op for duplicates.
    builder = builder
        .manage(PortState(std::sync::RwLock::new(init_port)))
        .manage(KernelState(std::sync::RwLock::new(init_kernel_inner)))
        .manage(ServerUrlState(std::sync::RwLock::new(init_url)))
        .manage(RemoteMode(std::sync::RwLock::new(init_remote)));

    builder
        .manage(holder)
        .invoke_handler(tauri::generate_handler![
            commands::get_port,
            commands::get_status,
            commands::get_agent_count,
            commands::import_agent_toml,
            commands::import_skill_file,
            commands::get_autostart,
            commands::set_autostart,
            commands::check_for_updates,
            commands::install_update,
            commands::open_config_dir,
            commands::open_logs_dir,
            connection::test_connection,
            connection::connect_remote,
            connection::start_local,
        ])
        .setup(move |app| {
            if show_connection_screen {
                // Show the connection screen via about:blank + eval
                let window = WebviewWindowBuilder::new(
                    app,
                    "main",
                    WebviewUrl::External("about:blank".parse().expect("Invalid about:blank URL")),
                )
                .title("LibreFang — Connect")
                .inner_size(1280.0, 800.0)
                .min_inner_size(800.0, 600.0)
                .center()
                .visible(true)
                .build()?;

                // Inject the connection screen HTML into the blank page
                let html = connection::connection_html();
                let escaped = serde_json::to_string(&html).unwrap_or_default();
                window.eval(format!(
                    "document.open(); document.write({escaped}); document.close();"
                ))?;
            } else {
                // Direct mode — navigate to the resolved URL
                let _window = WebviewWindowBuilder::new(
                    app,
                    "main",
                    WebviewUrl::External(initial_url.parse().expect("Invalid server URL")),
                )
                .title("LibreFang")
                .inner_size(1280.0, 800.0)
                .min_inner_size(800.0, 600.0)
                .center()
                .visible(true)
                .build()?;
            }

            // Set up system tray (desktop only)
            #[cfg(desktop)]
            tray::setup_tray(app)?;

            // For local direct-boot mode, start event forwarding for notifications
            if !is_remote && !show_connection_screen {
                if let Some(ks) = app.try_state::<KernelState>() {
                    let guard = ks.0.read().unwrap();
                    if let Some(ref inner) = *guard {
                        let app_handle = app.handle().clone();
                        let mut event_rx = inner.kernel.event_bus_ref().subscribe_all();
                        drop(guard);
                        tauri::async_runtime::spawn(async move {
                            forward_kernel_events(app_handle, &mut event_rx).await;
                        });
                    }
                }
            }

            // Spawn startup update check (desktop only, after event forwarding is set up)
            #[cfg(desktop)]
            updater::spawn_startup_check(app.handle().clone());

            info!("LibreFang Desktop window created");
            Ok(())
        })
        .on_window_event(|window, event| {
            // Hide to tray on close instead of quitting (desktop)
            #[cfg(desktop)]
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                let _ = window.hide();
                api.prevent_close();
            }
        })
        .build(tauri::generate_context!())
        .expect("Failed to build Tauri application")
        .run(|_app, event| {
            if let tauri::RunEvent::ExitRequested { .. } = event {
                info!("Tauri app exit requested");
            }
        });

    // App event loop has ended — shut down the embedded server + kernel if local
    info!("Tauri app closed, shutting down...");
    // The ServerHandle's Drop impl will signal shutdown automatically when
    // Tauri's managed state is dropped.
}
