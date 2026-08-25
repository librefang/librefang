//! System tray setup for the LibreFang desktop app.
//!
//! Desktop-only: system tray is not available on iOS or Android.

#![cfg(not(any(target_os = "ios", target_os = "android")))]

use librefang_kernel::config::librefang_home;
use librefang_kernel::AgentSubsystemApi;
use tauri::Manager;
use tauri_plugin_autostart::ManagerExt;
use tauri_plugin_notification::NotificationExt;
use tracing::{info, warn};

/// Format seconds into a human-readable uptime string.
fn format_uptime(secs: u64) -> String {
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3600 {
        let m = secs / 60;
        let s = secs % 60;
        format!("{m}m {s}s")
    } else {
        let h = secs / 3600;
        let m = (secs % 3600) / 60;
        format!("{h}h {m}m")
    }
}

fn open_browser(app: &tauri::AppHandle) {
    if let Some(url_state) = app.try_state::<crate::ServerUrlState>() {
        let url = url_state
            .0
            .read()
            .unwrap_or_else(|p| p.into_inner())
            .clone();
        if !url.is_empty() {
            let _ = open::that(&url);
        }
    } else if let Some(port) = app.try_state::<crate::PortState>() {
        // Fallback for backward compatibility
        if let Some(p) = *port.0.read().unwrap_or_else(|p| p.into_inner()) {
            let url = format!("http://127.0.0.1:{p}");
            let _ = open::that(&url);
        }
    }
}

fn change_server(app: &tauri::AppHandle) {
    // Shut down existing local server if running.
    if let Some(holder) = app.try_state::<crate::ServerHandleHolder>() {
        let mut guard = crate::lock_server_handle(&holder.0);
        if let Some(handle) = guard.take() {
            std::thread::spawn(move || handle.shutdown());
        }
    }
    // Clear local-mode state so commands report "not running".
    if let Some(state) = app.try_state::<crate::PortState>() {
        *state.0.write().unwrap_or_else(|p| p.into_inner()) = None;
    }
    if let Some(state) = app.try_state::<crate::KernelState>() {
        *state.0.write().unwrap_or_else(|p| p.into_inner()) = None;
    }

    // Navigate back to the connection screen
    if let Some(w) = app.get_webview_window("main") {
        let html = crate::connection::connection_html();
        let escaped = serde_json::to_string(&html).unwrap_or_default();
        let js = format!("document.open(); document.write({escaped}); document.close();");
        if let Err(e) = w.eval(&js) {
            warn!("Failed to show connection screen: {e}");
        }
        let _ = w.set_title("LibreFang — Connect");
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

fn toggle_launch_at_login(app: &tauri::AppHandle) {
    let manager = app.autolaunch();
    let currently_enabled = manager.is_enabled().unwrap_or(false);
    if currently_enabled {
        if let Err(e) = manager.disable() {
            warn!("Failed to disable autostart: {e}");
        }
    } else if let Err(e) = manager.enable() {
        warn!("Failed to enable autostart: {e}");
    }
    info!(
        "Autostart toggled: {}",
        manager.is_enabled().unwrap_or(false)
    );
}

fn check_updates(app: &tauri::AppHandle) {
    let app_handle = app.clone();
    tauri::async_runtime::spawn(async move {
        // First check what's available
        match crate::updater::check_for_update(&app_handle).await {
            Ok(info) if info.available => {
                let version = info.version.as_deref().unwrap_or("unknown");
                // Notify user we're starting install
                let _ = app_handle
                    .notification()
                    .builder()
                    .title("Installing Update...")
                    .body(format!(
                        "Downloading LibreFang v{version}. App will restart shortly."
                    ))
                    .show();
                // Perform install
                if let Err(e) = crate::updater::download_and_install_update(&app_handle).await {
                    warn!("Manual update install failed: {e}");
                    let _ = app_handle
                        .notification()
                        .builder()
                        .title("Update Failed")
                        .body(format!("Could not install update: {e}"))
                        .show();
                }
                // If we reach here, install failed (success causes restart)
            }
            Ok(_) => {
                let _ = app_handle
                    .notification()
                    .builder()
                    .title("Up to Date")
                    .body("You're running the latest version of LibreFang.")
                    .show();
            }
            Err(e) => {
                warn!("Tray update check failed: {e}");
                let _ = app_handle
                    .notification()
                    .builder()
                    .title("Update Check Failed")
                    .body("Could not check for updates. Try again later.")
                    .show();
            }
        }
    });
}

fn open_config_dir() {
    let dir = librefang_home();
    let _ = std::fs::create_dir_all(&dir);
    if let Err(e) = open::that(&dir) {
        warn!("Failed to open config dir: {e}");
    }
}

fn quit_app(app: &tauri::AppHandle) {
    info!("Quit requested from system tray");
    app.exit(0);
}

fn get_status_text(app: &tauri::AppHandle) -> String {
    let is_remote = app
        .try_state::<crate::RemoteMode>()
        .map(|r| *r.0.read().unwrap_or_else(|p| p.into_inner()))
        .unwrap_or(false);

    if is_remote {
        let url = app
            .try_state::<crate::ServerUrlState>()
            .map(|s| s.0.read().unwrap_or_else(|p| p.into_inner()).clone())
            .unwrap_or_else(|| "unknown".to_string());
        format!("Status: Remote ({url})")
    } else if let Some(ks) = app.try_state::<crate::KernelState>() {
        let guard = ks.0.read().unwrap_or_else(|p| p.into_inner());
        if let Some(ref inner) = *guard {
            let uptime = format_uptime(inner.started_at.elapsed().as_secs());
            format!("Status: Running ({uptime})")
        } else {
            "Status: Not connected".to_string()
        }
    } else {
        "Status: Not connected".to_string()
    }
}

fn get_agent_count(app: &tauri::AppHandle) -> usize {
    if let Some(ks) = app.try_state::<crate::KernelState>() {
        let guard = ks.0.read().unwrap_or_else(|p| p.into_inner());
        if let Some(ref inner) = *guard {
            inner.kernel.agent_registry_ref().list().len()
        } else {
            0
        }
    } else {
        0
    }
}

// ==========================================
// Windows & macOS implementation (Tauri Tray)
// ==========================================
#[cfg(not(target_os = "linux"))]
mod platform_tray {
    use tauri::{
        menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem},
        tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
        Manager,
    };
    use tauri_plugin_autostart::ManagerExt;

    pub fn setup_tray(app: &tauri::App) -> Result<(), Box<dyn std::error::Error>> {
        // Action items
        let show = MenuItem::with_id(app, "show", "Show Window", true, None::<&str>)?;
        let browser = MenuItem::with_id(app, "browser", "Open in Browser", true, None::<&str>)?;
        let change_server =
            MenuItem::with_id(app, "change_server", "Change Server...", true, None::<&str>)?;
        let sep1 = PredefinedMenuItem::separator(app)?;

        // Informational items (disabled — display only)
        let status_text = super::get_status_text(app.handle());
        let agent_count = super::get_agent_count(app.handle());

        let agents_info = MenuItem::with_id(
            app,
            "agents_info",
            format!("Agents: {agent_count} running"),
            false,
            None::<&str>,
        )?;
        let status_info = MenuItem::with_id(app, "status_info", &status_text, false, None::<&str>)?;
        let sep2 = PredefinedMenuItem::separator(app)?;

        // Settings items
        let autostart_enabled = app.autolaunch().is_enabled().unwrap_or(false);
        let launch_at_login = CheckMenuItem::with_id(
            app,
            "launch_at_login",
            "Launch at Login",
            true,
            autostart_enabled,
            None::<&str>,
        )?;
        let check_updates = MenuItem::with_id(
            app,
            "check_updates",
            "Check for Updates...",
            true,
            None::<&str>,
        )?;
        let open_config = MenuItem::with_id(
            app,
            "open_config",
            "Open Config Directory",
            true,
            None::<&str>,
        )?;
        let sep3 = PredefinedMenuItem::separator(app)?;

        let quit = MenuItem::with_id(app, "quit", "Quit LibreFang", true, None::<&str>)?;

        let menu = Menu::with_items(
            app,
            &[
                &show,
                &browser,
                &change_server,
                &sep1,
                &agents_info,
                &status_info,
                &sep2,
                &launch_at_login,
                &check_updates,
                &open_config,
                &sep3,
                &quit,
            ],
        )?;

        // Load the tray icon from embedded PNG bytes
        let tray_icon = tauri::image::Image::from_bytes(include_bytes!("../icons/32x32.png"))
            .expect("Failed to decode tray icon PNG");

        let _tray = TrayIconBuilder::new()
            .icon(tray_icon)
            .menu(&menu)
            .tooltip("LibreFang Agent OS")
            .on_menu_event(move |app, event| match event.id().as_ref() {
                "show" => {
                    if let Some(w) = app.get_webview_window("main") {
                        let _ = w.show();
                        let _ = w.unminimize();
                        let _ = w.set_focus();
                    }
                }
                "browser" => {
                    super::open_browser(app);
                }
                "change_server" => {
                    super::change_server(app);
                }
                "launch_at_login" => {
                    super::toggle_launch_at_login(app);
                }
                "check_updates" => {
                    super::check_updates(app);
                }
                "open_config" => {
                    super::open_config_dir();
                }
                "quit" => {
                    super::quit_app(app);
                }
                _ => {}
            })
            .on_tray_icon_event(|tray, event| {
                if let TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                } = event
                {
                    let app = tray.app_handle();
                    if let Some(w) = app.get_webview_window("main") {
                        let _ = w.show();
                        let _ = w.unminimize();
                        let _ = w.set_focus();
                    }
                }
            })
            .build(app)?;

        Ok(())
    }
}

// ==========================================
// Linux implementation (ksni D-Bus Tray)
// ==========================================
#[cfg(target_os = "linux")]
mod platform_tray {
    use ksni::{menu::*, MenuItem, Tray, TrayMethods};
    use tauri::Manager;
    use tauri_plugin_autostart::ManagerExt;
    use tracing::{info, warn};

    #[derive(Clone)]
    struct LibreFangLinuxTray {
        app_handle: tauri::AppHandle,
    }

    pub(super) fn rgba_to_argb(mut rgba: Vec<u8>) -> Vec<u8> {
        for pixel in rgba.as_chunks_mut::<4>().0 {
            pixel.rotate_right(1); // convert RGBA to ARGB
        }
        rgba
    }

    impl Tray for LibreFangLinuxTray {
        fn id(&self) -> String {
            "librefang".into()
        }

        fn title(&self) -> String {
            "LibreFang".into()
        }

        fn tool_tip(&self) -> ksni::ToolTip {
            ksni::ToolTip {
                title: "LibreFang Agent OS".into(),
                ..Default::default()
            }
        }

        fn activate(&mut self, _x: i32, _y: i32) {
            if let Some(w) = self.app_handle.get_webview_window("main") {
                let visible = w.is_visible().unwrap_or(false);
                let minimized = w.is_minimized().unwrap_or(false);
                if visible && !minimized {
                    let _ = w.hide();
                } else {
                    let _ = w.show();
                    let _ = w.unminimize();
                    let _ = w.set_focus();
                }
            }
        }

        fn icon_pixmap(&self) -> Vec<ksni::Icon> {
            use std::sync::LazyLock;
            static ICON_CACHE: LazyLock<Vec<ksni::Icon>> = LazyLock::new(|| {
                let tauri_image =
                    tauri::image::Image::from_bytes(include_bytes!("../icons/32x32.png"))
                        .expect("Failed to decode tray icon PNG");
                let width = tauri_image.width();
                let height = tauri_image.height();
                let rgba_data = rgba_to_argb(tauri_image.rgba().to_vec());
                vec![ksni::Icon {
                    width: width as i32,
                    height: height as i32,
                    data: rgba_data,
                }]
            });
            ICON_CACHE.clone()
        }

        fn menu(&self) -> Vec<MenuItem<Self>> {
            let show_win = StandardItem {
                label: "Show Window".into(),
                activate: Box::new(|this: &mut LibreFangLinuxTray| {
                    if let Some(w) = this.app_handle.get_webview_window("main") {
                        let _ = w.show();
                        let _ = w.unminimize();
                        let _ = w.set_focus();
                    }
                }),
                ..Default::default()
            };

            let open_browser = StandardItem {
                label: "Open in Browser".into(),
                activate: Box::new(|this: &mut LibreFangLinuxTray| {
                    super::open_browser(&this.app_handle);
                }),
                ..Default::default()
            };

            let change_server = StandardItem {
                label: "Change Server...".into(),
                activate: Box::new(|this: &mut LibreFangLinuxTray| {
                    super::change_server(&this.app_handle);
                }),
                ..Default::default()
            };

            // Informational items (disabled)
            let status_text = super::get_status_text(&self.app_handle);
            let agent_count = super::get_agent_count(&self.app_handle);

            let agents_info = StandardItem {
                label: format!("Agents: {agent_count} running"),
                enabled: false,
                ..Default::default()
            };

            let status_info = StandardItem {
                label: status_text,
                enabled: false,
                ..Default::default()
            };

            // Settings items
            let autostart_enabled = self.app_handle.autolaunch().is_enabled().unwrap_or(false);
            let launch_at_login = CheckmarkItem {
                label: "Launch at Login".into(),
                checked: autostart_enabled,
                activate: Box::new(|this: &mut LibreFangLinuxTray| {
                    super::toggle_launch_at_login(&this.app_handle);
                }),
                ..Default::default()
            };

            let check_updates = StandardItem {
                label: "Check for Updates...".into(),
                activate: Box::new(|this: &mut LibreFangLinuxTray| {
                    super::check_updates(&this.app_handle);
                }),
                ..Default::default()
            };

            let open_config = StandardItem {
                label: "Open Config Directory".into(),
                activate: Box::new(|_this: &mut LibreFangLinuxTray| {
                    super::open_config_dir();
                }),
                ..Default::default()
            };

            let quit = StandardItem {
                label: "Quit LibreFang".into(),
                activate: Box::new(|this: &mut LibreFangLinuxTray| {
                    super::quit_app(&this.app_handle);
                }),
                ..Default::default()
            };

            vec![
                show_win.into(),
                open_browser.into(),
                change_server.into(),
                MenuItem::Separator,
                agents_info.into(),
                status_info.into(),
                MenuItem::Separator,
                launch_at_login.into(),
                check_updates.into(),
                open_config.into(),
                MenuItem::Separator,
                quit.into(),
            ]
        }

        fn watcher_online(&self) {
            info!("Linux StatusNotifierWatcher is online.");
        }

        fn watcher_offline(&self, reason: ksni::OfflineReason) -> bool {
            warn!("Linux StatusNotifierWatcher is offline: {:?}", reason);
            true
        }
    }

    pub fn setup_tray(app: &tauri::App) -> Result<(), Box<dyn std::error::Error>> {
        let tray = LibreFangLinuxTray {
            app_handle: app.handle().clone(),
        };
        tauri::async_runtime::spawn(async move {
            use librefang_types::backoff::{BackoffStrategy, ExponentialBackoff};

            let backoff = ExponentialBackoff::new(
                std::time::Duration::from_secs(10),
                2.0,
                std::time::Duration::from_secs(300),
            );
            let mut consecutive_failures = 0;

            loop {
                // We use `assume_sni_available(true)` so that `ksni` registers and publishes the tray service immediately even if no StatusNotifierWatcher is running on the bus yet (e.g. at boot).
                // This avoids startup errors and allows the tray to automatically become visible as soon as the desktop panel starts.
                match tray.clone().assume_sni_available(true).spawn().await {
                    Ok(handle) => {
                        info!("Linux system tray successfully spawned.");
                        consecutive_failures = 0;
                        // Periodically call handle.update() to diff properties and emit D-Bus update signals (e.g. NewTitle / NewTooltip).
                        // Without this heartbeat loop, properties like Uptime and Agents count (which are rendered inside menu()) will never update dynamically, remaining stale until a manual user activation query.
                        loop {
                            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                            if handle.update(|_| {}).await.is_none() {
                                warn!("Linux system tray service disconnected; will attempt to reconnect.");
                                let _ = handle.shutdown().await;
                                break;
                            }
                        }
                    }
                    Err(e) => {
                        consecutive_failures += 1;
                        let delay = backoff.next_delay(consecutive_failures);
                        // Only log warning for the first 3 consecutive attempts to avoid spamming system logs.
                        if consecutive_failures <= 3 {
                            warn!(
                                "Failed to spawn Linux system tray: {e}. Retrying in {delay:?}..."
                            );
                        }
                        tokio::time::sleep(delay).await;
                    }
                }
            }
        });
        Ok(())
    }
}

pub use platform_tray::setup_tray;

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::platform_tray::rgba_to_argb;

    #[test]
    fn test_rgba_to_argb() {
        let rgba = vec![0xFF, 0x00, 0x00, 0xFF]; // Opaque Red (RGBA)
        let argb = rgba_to_argb(rgba);
        assert_eq!(argb, vec![0xFF, 0xFF, 0x00, 0x00]); // Opaque Red (ARGB)
    }
}
