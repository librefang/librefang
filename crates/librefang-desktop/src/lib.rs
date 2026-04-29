//! BossFang Desktop — Native Tauri 2.0 wrapper for the BossFang Agent OS.
//!
//! Boots the kernel + embedded API server, then opens a native window pointing
//! at the WebUI. Includes system tray, single-instance enforcement, native OS
//! notifications, global shortcuts, auto-start, and update checking.
//!
//! Supports remote server mode: connect to a running BossFang instance instead
//! of always starting a local one. Priority: CLI arg > env var > saved pref > connection screen.

mod commands;
mod connection;
#[cfg(not(any(target_os = "ios", target_os = "android")))]
mod server;
#[cfg(not(any(target_os = "ios", target_os = "android")))]
mod shortcuts;
#[cfg(not(any(target_os = "ios", target_os = "android")))]
mod tray;
#[cfg(not(any(target_os = "ios", target_os = "android")))]
mod updater;

use librefang_extensions::dotenv;
use librefang_kernel::LibreFangKernel;
use librefang_types::event::{EventPayload, LifecycleEvent, SystemEvent};
use std::net::IpAddr;
use std::sync::Arc;
use std::time::Instant;
use tauri::Manager;
#[cfg(desktop)]
use tauri::{WebviewUrl, WebviewWindowBuilder};
use tauri_plugin_notification::NotificationExt;
use tracing::{info, warn};

/// Reject http:// for non-loopback hosts (IPC-enabled webview MITM-RCE, #3673).
pub(crate) fn validate_server_url(url: &str) -> Result<(), String> {
    let lower = url.to_ascii_lowercase();
    let (scheme_is_http, rest) = if let Some(r) = lower.strip_prefix("http://") {
        (true, r)
    } else if let Some(r) = lower.strip_prefix("https://") {
        (false, r)
    } else {
        return Err(format!(
            "Server URL must start with http:// or https://, got: {url}"
        ));
    };

    if !scheme_is_http {
        return Ok(());
    }

    // strip path/query, then peel optional :port (IPv6 literal needs []).
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    // Reject userinfo: `http://[::1]@evil.com/` would otherwise pass the
    // loopback check while wry/reqwest connect to evil.com.
    if authority.contains('@') {
        return Err(format!(
            "Refusing URL with userinfo (would bypass loopback check): {url}"
        ));
    }
    let host = if let Some(stripped) = authority.strip_prefix('[') {
        match stripped.split_once(']') {
            Some((h, _)) => h,
            None => return Err(format!("Malformed IPv6 host in URL: {url}")),
        }
    } else {
        authority
            .rsplit_once(':')
            .map(|(h, _)| h)
            .unwrap_or(authority)
    };

    if host.is_empty() {
        return Err(format!("Missing host in URL: {url}"));
    }

    if host == "localhost" {
        return Ok(());
    }
    if let Ok(ip) = host.parse::<IpAddr>() {
        if ip.is_loopback() {
            return Ok(());
        }
    }

    Err(format!(
        "Refusing to load plaintext http:// from non-loopback host {host}: \
         use https:// to prevent MITM-injected IPC abuse (issue #3673). URL: {url}"
    ))
}

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
/// Desktop-only: mobile is a thin client with no embedded server.
#[cfg(not(any(target_os = "ios", target_os = "android")))]
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
                        "BossFang kernel is shutting down".to_string(),
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
    /// Boot a local server (skip connection screen) — desktop only.
    #[cfg(not(any(target_os = "ios", target_os = "android")))]
    Local,
    /// Show the connection screen and let the user decide.
    ConnectionScreen,
}

/// Mobile entry point. `tauri::mobile_entry_point` requires a 0-arg
/// function, so on iOS/Android we wrap `run()` and pass defaults — the
/// CLI flags it normally consumes don't apply when the OS launches the
/// app via the bundled binary.
#[cfg(mobile)]
#[tauri::mobile_entry_point]
fn mobile_main() {
    run(None, false);
}

/// Entry point for the Tauri application.
///
/// `server_url` — CLI `--server-url` override (remote mode).
/// `force_local` — CLI `--local` flag (skip connection screen, start local).
pub fn run(server_url: Option<String>, force_local: bool) {
    // Init tracing
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "librefang=info,tauri=info".into()),
        )
        .init();

    info!("Starting BossFang Desktop...");

    // Load ~/.librefang/.env into process environment (system env takes priority).
    dotenv::load_dotenv();

    // Resolve connection mode (priority: CLI > env var > saved preference > connection screen)
    let mode = if let Some(ref url) = server_url {
        StartupMode::Remote(url.trim_end_matches('/').to_string())
    } else if force_local {
        // force_local is only meaningful on desktop — on mobile always use connection screen
        #[cfg(not(any(target_os = "ios", target_os = "android")))]
        {
            StartupMode::Local
        }
        #[cfg(any(target_os = "ios", target_os = "android"))]
        {
            StartupMode::ConnectionScreen
        }
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
            #[cfg(not(any(target_os = "ios", target_os = "android")))]
            "local" => StartupMode::Local,
            _ => StartupMode::ConnectionScreen,
        }
    } else {
        StartupMode::ConnectionScreen
    };

    // For direct modes (remote or forced local), resolve URL + optional server handle now.
    #[cfg(not(any(target_os = "ios", target_os = "android")))]
    let (initial_url, server_handle, is_remote) = match &mode {
        StartupMode::Remote(url) => {
            if let Err(e) = validate_server_url(url) {
                eprintln!("{e}");
                std::process::exit(1);
            }
            info!("Remote mode: connecting to {url}");
            (url.clone(), None, true)
        }
        StartupMode::Local => {
            let handle = server::start_server().expect("Failed to start BossFang server");
            let port = handle.port;
            info!("BossFang server running on port {port}");
            (format!("http://127.0.0.1:{port}"), Some(handle), false)
        }
        StartupMode::ConnectionScreen => {
            // Will be resolved later via the connection screen IPC commands.
            (String::new(), None, false)
        }
    };

    // On mobile, we are always in remote or connection-screen mode — no local server.
    #[cfg(any(target_os = "ios", target_os = "android"))]
    let (initial_url, is_remote) = match &mode {
        StartupMode::Remote(url) => {
            if let Err(e) = validate_server_url(url) {
                eprintln!("{e}");
                std::process::exit(1);
            }
            info!("Remote mode: connecting to {url}");
            (url.clone(), true)
        }
        StartupMode::ConnectionScreen => (String::new(), false),
    };

    let show_connection_screen = matches!(mode, StartupMode::ConnectionScreen);

    // Serve the connection screen HTML through a custom URI scheme instead of
    // about:blank + document.write. The old approach no-ops on WebKitGTK 2.50
    // (stock NixOS, current AppImage), leaving a blank window — see #3052.
    let mut builder = tauri::Builder::default()
        .register_uri_scheme_protocol("lfconnect", |_ctx, _req| {
            tauri::http::Response::builder()
                .status(200)
                .header("Content-Type", "text/html; charset=utf-8")
                .body(connection::connection_html().into_bytes())
                .expect("connection response must build")
        })
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_dialog::init());

    // Shell plugin spawns CLI processes — desktop only
    #[cfg(not(any(target_os = "ios", target_os = "android")))]
    {
        builder = builder.plugin(tauri_plugin_shell::init());
    }

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

    // Always register the ServerHandleHolder on desktop so start_local can fill it later.
    // On mobile, there is no embedded server so this type does not exist.
    #[cfg(not(any(target_os = "ios", target_os = "android")))]
    let holder = ServerHandleHolder(std::sync::Mutex::new(server_handle));

    // Pre-compute initial values for interior-mutable state.
    #[cfg(not(any(target_os = "ios", target_os = "android")))]
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

    // Mobile: no kernel state, port state, or server handle — always thin client.
    #[cfg(any(target_os = "ios", target_os = "android"))]
    let (init_port, init_kernel_inner, init_url, init_remote) = match &mode {
        StartupMode::Remote(_) => (None::<u16>, None::<KernelInner>, initial_url.clone(), true),
        StartupMode::ConnectionScreen => (None, None, String::new(), false),
    };

    // Register ALL state types ONCE with initial values. Updates go through
    // interior-mutable RwLocks — Tauri `manage()` is a no-op for duplicates.
    builder = builder
        .manage(PortState(std::sync::RwLock::new(init_port)))
        .manage(KernelState(std::sync::RwLock::new(init_kernel_inner)))
        .manage(ServerUrlState(std::sync::RwLock::new(init_url)))
        .manage(RemoteMode(std::sync::RwLock::new(init_remote)));

    #[cfg(not(any(target_os = "ios", target_os = "android")))]
    {
        builder = builder.manage(holder);
    }

    // `generate_handler!` does not support cfg attributes inside the macro, so we
    // build two separate handler closures and attach the correct one at compile
    // time. The macro produces a closure whose runtime type parameter is inferred
    // from `builder`, so we call `.invoke_handler(...)` directly inside each cfg
    // branch — binding the result to a `let` first leaves rustc unable to infer
    // the runtime type and triggers E0282.
    #[cfg(not(any(target_os = "ios", target_os = "android")))]
    let builder = builder.invoke_handler(tauri::generate_handler![
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
        commands::uninstall_app,
        connection::test_connection,
        connection::connect_remote,
        connection::start_local,
    ]);
    #[cfg(any(target_os = "ios", target_os = "android"))]
    let builder = builder.invoke_handler(tauri::generate_handler![
        commands::get_port,
        commands::get_status,
        commands::get_agent_count,
        commands::import_agent_toml,
        commands::import_skill_file,
        commands::open_config_dir,
        commands::open_logs_dir,
        commands::uninstall_app,
        commands::store_credentials,
        commands::get_credentials,
        commands::clear_credentials,
        connection::test_connection,
        connection::connect_remote,
    ]);

    builder
        .setup(move |app| {
            // Desktop window. `.title()` / `.inner_size()` / `.center()` /
            // `.min_inner_size()` are not exposed on mobile, so the mobile
            // branch below has its own minimal builder.
            #[cfg(desktop)]
            {
                if show_connection_screen {
                    let _window = WebviewWindowBuilder::new(
                        app,
                        "main",
                        WebviewUrl::CustomProtocol(
                            "lfconnect://localhost/"
                                .parse()
                                .expect("lfconnect URL must parse"),
                        ),
                    )
                    .title("LibreFang — Connect")
                    .inner_size(1280.0, 800.0)
                    .min_inner_size(800.0, 600.0)
                    .center()
                    .visible(true)
                    .build()?;
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
            }

            // Mobile window. Without this, iOS/Android launches into a
            // black WebView because tauri.conf.json's `app.windows` is
            // empty and Tauri 2 does not auto-create a mobile window.
            // The OS manages size/orientation, so we only set the URL
            // and visibility.
            #[cfg(mobile)]
            {
                let url = if show_connection_screen {
                    WebviewUrl::CustomProtocol(
                        "lfconnect://localhost/"
                            .parse()
                            .expect("lfconnect URL must parse"),
                    )
                } else {
                    WebviewUrl::External(initial_url.parse().expect("Invalid server URL"))
                };
                let _window = WebviewWindowBuilder::new(app, "main", url)
                    .visible(true)
                    .build()?;
            }

            // Set up system tray (desktop only)
            #[cfg(desktop)]
            tray::setup_tray(app)?;

            // For local direct-boot mode, start event forwarding for notifications
            if !is_remote && !show_connection_screen {
                if let Some(ks) = app.try_state::<KernelState>() {
                    let guard = ks.0.read().unwrap_or_else(|p| p.into_inner());
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

            info!("BossFang Desktop window created");
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

#[cfg(test)]
mod tests {
    use super::validate_server_url;

    #[test]
    fn https_remote_host_accepted() {
        assert!(validate_server_url("https://example.com").is_ok());
        assert!(validate_server_url("https://example.com:8443/path").is_ok());
        assert!(validate_server_url("https://192.0.2.10").is_ok());
    }

    #[test]
    fn http_loopback_accepted() {
        assert!(validate_server_url("http://127.0.0.1:4545").is_ok());
        assert!(validate_server_url("http://localhost").is_ok());
        assert!(validate_server_url("http://localhost:4545/dashboard").is_ok());
        assert!(validate_server_url("http://[::1]:4545").is_ok());
        assert!(validate_server_url("http://127.0.0.1").is_ok());
        // Anywhere in 127.0.0.0/8 is loopback per IpAddr::is_loopback.
        assert!(validate_server_url("http://127.5.6.7:4545").is_ok());
    }

    #[test]
    fn http_remote_host_rejected() {
        assert!(validate_server_url("http://example.com").is_err());
        assert!(validate_server_url("http://192.168.1.10:4545").is_err());
        assert!(validate_server_url("http://10.0.0.1:4545/dashboard").is_err());
        assert!(validate_server_url("http://[2001:db8::1]:4545").is_err());
    }

    #[test]
    fn case_insensitive_scheme() {
        assert!(validate_server_url("HTTP://example.com").is_err());
        assert!(validate_server_url("Http://127.0.0.1:4545").is_ok());
        assert!(validate_server_url("HTTPS://example.com").is_ok());
    }

    #[test]
    fn unknown_scheme_rejected() {
        assert!(validate_server_url("ftp://example.com").is_err());
        assert!(validate_server_url("javascript:alert(1)").is_err());
        assert!(validate_server_url("file:///etc/passwd").is_err());
        assert!(validate_server_url("").is_err());
        assert!(validate_server_url("example.com").is_err());
    }

    #[test]
    fn malformed_url_rejected() {
        assert!(validate_server_url("http://").is_err());
        assert!(validate_server_url("http://[::1").is_err());
    }

    #[test]
    fn userinfo_loopback_bypass_rejected() {
        // wry/reqwest connect to the @-suffix host, not the userinfo;
        // the IPv6/IPv4 prefix must NOT vouch for the real target.
        assert!(validate_server_url("http://[::1]@evil.com/").is_err());
        assert!(validate_server_url("http://[::1]:80@evil.com/").is_err());
        assert!(validate_server_url("http://localhost@evil.com/").is_err());
        assert!(validate_server_url("http://127.0.0.1@evil.com/").is_err());
        assert!(validate_server_url("http://user:pass@evil.com/").is_err());
    }
}
