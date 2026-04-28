//! Tauri IPC command handlers.

use crate::{KernelState, PortState};
use librefang_kernel::config::librefang_home;
use tauri_plugin_dialog::DialogExt;
use tracing::info;

#[cfg(not(any(target_os = "ios", target_os = "android")))]
use tauri_plugin_autostart::ManagerExt;

/// Get the port the embedded server is listening on.
#[tauri::command]
pub fn get_port(port: tauri::State<'_, PortState>) -> Result<u16, String> {
    port.0
        .read()
        .unwrap_or_else(|p| p.into_inner())
        .ok_or_else(|| "No local server running".to_string())
}

/// Get a status summary of the running kernel.
#[tauri::command]
pub fn get_status(
    port: tauri::State<'_, PortState>,
    kernel_state: tauri::State<'_, KernelState>,
) -> Result<serde_json::Value, String> {
    let p = port
        .0
        .read()
        .unwrap_or_else(|p| p.into_inner())
        .ok_or_else(|| "No local server running".to_string())?;
    let guard = kernel_state.0.read().unwrap_or_else(|p| p.into_inner());
    let inner = guard
        .as_ref()
        .ok_or_else(|| "No local server running".to_string())?;
    let agents = inner.kernel.agent_registry().list().len();
    let uptime_secs = inner.started_at.elapsed().as_secs();

    Ok(serde_json::json!({
        "status": "running",
        "port": p,
        "agents": agents,
        "uptime_secs": uptime_secs,
    }))
}

/// Get the number of registered agents.
#[tauri::command]
pub fn get_agent_count(kernel_state: tauri::State<'_, KernelState>) -> Result<usize, String> {
    let guard = kernel_state.0.read().unwrap_or_else(|p| p.into_inner());
    let inner = guard
        .as_ref()
        .ok_or_else(|| "No local server running".to_string())?;
    Ok(inner.kernel.agent_registry().list().len())
}

/// Open a native file picker to import an agent TOML manifest.
///
/// Validates the TOML as a valid `AgentManifest`, copies it to
/// `~/.librefang/workspaces/agents/{name}/agent.toml`, then spawns the agent.
#[tauri::command]
pub fn import_agent_toml(
    app: tauri::AppHandle,
    kernel_state: tauri::State<'_, KernelState>,
) -> Result<String, String> {
    let path = app
        .dialog()
        .file()
        .set_title("Import Agent Manifest")
        .add_filter("TOML files", &["toml"])
        .blocking_pick_file();

    let file_path = match path {
        Some(p) => p,
        None => return Err("No file selected".to_string()),
    };

    let content = std::fs::read_to_string(file_path.as_path().ok_or("Invalid file path")?)
        .map_err(|e| format!("Failed to read file: {e}"))?;

    let manifest: librefang_types::agent::AgentManifest =
        toml::from_str(&content).map_err(|e| format!("Invalid agent manifest: {e}"))?;

    let agent_name = manifest.name.clone();
    let agent_dir = librefang_home()
        .join("workspaces")
        .join("agents")
        .join(&agent_name);
    std::fs::create_dir_all(&agent_dir)
        .map_err(|e| format!("Failed to create agent directory: {e}"))?;

    let dest = agent_dir.join("agent.toml");
    std::fs::write(&dest, &content).map_err(|e| format!("Failed to write manifest: {e}"))?;

    let guard = kernel_state.0.read().unwrap_or_else(|p| p.into_inner());
    let inner = guard
        .as_ref()
        .ok_or_else(|| "No local server running".to_string())?;
    inner
        .kernel
        .spawn_agent(manifest)
        .map_err(|e| format!("Failed to spawn agent: {e}"))?;

    info!("Imported and spawned agent \"{agent_name}\"");
    Ok(agent_name)
}

/// Open a native file picker to import a skill file.
///
/// Copies the selected file to `~/.librefang/skills/` and triggers a
/// hot-reload of the skill registry.
#[tauri::command]
pub fn import_skill_file(
    app: tauri::AppHandle,
    kernel_state: tauri::State<'_, KernelState>,
) -> Result<String, String> {
    let path = app
        .dialog()
        .file()
        .set_title("Import Skill File")
        .add_filter("Skill files", &["md", "toml", "py", "js", "wasm"])
        .blocking_pick_file();

    let file_path = match path {
        Some(p) => p,
        None => return Err("No file selected".to_string()),
    };

    let src = file_path.as_path().ok_or("Invalid file path")?;
    let file_name = src
        .file_name()
        .ok_or("No filename")?
        .to_string_lossy()
        .to_string();

    let skills_dir = librefang_home().join("skills");
    std::fs::create_dir_all(&skills_dir)
        .map_err(|e| format!("Failed to create skills directory: {e}"))?;

    let dest = skills_dir.join(&file_name);
    std::fs::copy(src, &dest).map_err(|e| format!("Failed to copy skill file: {e}"))?;

    let guard = kernel_state.0.read().unwrap_or_else(|p| p.into_inner());
    let inner = guard
        .as_ref()
        .ok_or_else(|| "No local server running".to_string())?;
    inner.kernel.reload_skills();

    info!("Imported skill file \"{file_name}\" and reloaded registry");
    Ok(file_name)
}

/// Check whether auto-start on login is enabled.
#[cfg(not(any(target_os = "ios", target_os = "android")))]
#[tauri::command]
pub fn get_autostart(app: tauri::AppHandle) -> Result<bool, String> {
    app.autolaunch().is_enabled().map_err(|e| e.to_string())
}

/// Enable or disable auto-start on login.
#[cfg(not(any(target_os = "ios", target_os = "android")))]
#[tauri::command]
pub fn set_autostart(app: tauri::AppHandle, enabled: bool) -> Result<bool, String> {
    let manager = app.autolaunch();
    if enabled {
        manager.enable().map_err(|e| e.to_string())?;
    } else {
        manager.disable().map_err(|e| e.to_string())?;
    }
    manager.is_enabled().map_err(|e| e.to_string())
}

/// Perform an on-demand update check.
#[cfg(not(any(target_os = "ios", target_os = "android")))]
#[tauri::command]
pub async fn check_for_updates(
    app: tauri::AppHandle,
) -> Result<crate::updater::UpdateInfo, String> {
    crate::updater::check_for_update(&app).await
}

/// Download and install the latest update, then restart the app.
/// Returns Ok(()) which triggers an app restart — the command will not return
/// if the update succeeds (the app restarts). On error, returns Err(message).
#[cfg(not(any(target_os = "ios", target_os = "android")))]
#[tauri::command]
pub async fn install_update(app: tauri::AppHandle) -> Result<(), String> {
    crate::updater::download_and_install_update(&app).await
}

// ── Credential storage (mobile only — keyring) ───────────────────────────

#[cfg(any(target_os = "ios", target_os = "android"))]
const KEYRING_SERVICE: &str = "librefang-mobile";
#[cfg(any(target_os = "ios", target_os = "android"))]
const KEYRING_ACCOUNT: &str = "daemon-credentials";

/// Store daemon credentials in the OS keyring.
///
/// JSON-encodes `{"base_url": ..., "api_key": ...}` as the secret.
/// Falls back gracefully on platforms where keyring is unavailable.
#[cfg(any(target_os = "ios", target_os = "android"))]
#[tauri::command]
pub fn store_credentials(base_url: String, api_key: String) -> Result<(), String> {
    let creds = serde_json::json!({ "base_url": base_url, "api_key": api_key }).to_string();
    keyring::Entry::new(KEYRING_SERVICE, KEYRING_ACCOUNT)
        .map_err(|e| format!("Keyring init failed: {e}"))?
        .set_password(&creds)
        .map_err(|e| format!("Failed to store credentials: {e}"))
}

/// Retrieve daemon credentials from the OS keyring.
///
/// Returns `null` if no credentials are stored.
#[cfg(any(target_os = "ios", target_os = "android"))]
#[tauri::command]
pub fn get_credentials() -> Result<Option<serde_json::Value>, String> {
    let entry = keyring::Entry::new(KEYRING_SERVICE, KEYRING_ACCOUNT)
        .map_err(|e| format!("Keyring init failed: {e}"))?;
    match entry.get_password() {
        Ok(secret) => {
            let val: serde_json::Value =
                serde_json::from_str(&secret).map_err(|e| format!("Invalid stored creds: {e}"))?;
            Ok(Some(val))
        }
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(format!("Failed to read credentials: {e}")),
    }
}

/// Remove daemon credentials from the OS keyring.
#[cfg(any(target_os = "ios", target_os = "android"))]
#[tauri::command]
pub fn clear_credentials() -> Result<(), String> {
    let entry = keyring::Entry::new(KEYRING_SERVICE, KEYRING_ACCOUNT)
        .map_err(|e| format!("Keyring init failed: {e}"))?;
    match entry.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(format!("Failed to clear credentials: {e}")),
    }
}

/// Open the LibreFang config directory (`~/.librefang/`) in the OS file manager.
#[tauri::command]
pub fn open_config_dir() -> Result<(), String> {
    let dir = librefang_home();
    std::fs::create_dir_all(&dir).map_err(|e| format!("Failed to create config dir: {e}"))?;
    open::that(&dir).map_err(|e| format!("Failed to open directory: {e}"))
}

/// Open the LibreFang logs directory (`~/.librefang/logs/`) in the OS file manager.
#[tauri::command]
pub fn open_logs_dir() -> Result<(), String> {
    let dir = librefang_home().join("logs");
    std::fs::create_dir_all(&dir).map_err(|e| format!("Failed to create logs dir: {e}"))?;
    open::that(&dir).map_err(|e| format!("Failed to open directory: {e}"))
}

/// Launch the platform uninstaller and exit the app.
///
/// - **Windows**: reads `UninstallString` from the NSIS registry key and runs it.
/// - **macOS**: moves the `.app` bundle to Trash via `osascript` + Finder.
/// - **Linux/AppImage**: deletes the AppImage binary directly.
/// - **Linux/system package**: returns a hint to run the distro uninstall command.
#[tauri::command]
pub async fn uninstall_app(app: tauri::AppHandle) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        // `reg query` is a built-in Windows tool — no extra deps required.
        let output = std::process::Command::new("reg")
            .args([
                "query",
                "HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall",
                "/f",
                "LibreFang",
                "/s",
            ])
            .output()
            .map_err(|e| format!("Failed to query registry: {e}"))?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        for line in stdout.lines() {
            let trimmed = line.trim_start();
            if trimmed.starts_with("UninstallString") {
                // reg output columns are space-separated: Name    REG_SZ    Value
                if let Some(value) = trimmed.splitn(3, "    ").nth(2) {
                    let cmd = value.trim().to_string();
                    std::process::Command::new("cmd")
                        .args(["/C", &cmd])
                        .spawn()
                        .map_err(|e| format!("Failed to launch uninstaller: {e}"))?;
                    app.exit(0);
                    return Ok(());
                }
            }
        }
        Err("Uninstaller not found in registry. The app may have been installed without the NSIS installer.".to_string())
    }

    #[cfg(target_os = "macos")]
    {
        // Walk up from the executable to find the enclosing .app bundle.
        let exe = std::env::current_exe().map_err(|e| format!("Cannot locate executable: {e}"))?;
        let bundle = exe
            .ancestors()
            .find(|p| p.extension().and_then(|e| e.to_str()) == Some("app"))
            .map(|p| p.to_path_buf())
            .ok_or_else(|| {
                "App bundle (.app) not found — was LibreFang installed from a .dmg?".to_string()
            })?;

        let path = bundle.to_string_lossy().replace('"', "\\\"");
        std::process::Command::new("osascript")
            .args([
                "-e",
                &format!(r#"tell application "Finder" to move POSIX file "{path}" to trash"#),
            ])
            .spawn()
            .map_err(|e| format!("Failed to move app to Trash: {e}"))?;
        app.exit(0);
        Ok(())
    }

    #[cfg(target_os = "linux")]
    {
        let exe = std::env::current_exe().map_err(|e| format!("Cannot locate executable: {e}"))?;
        let exe_str = exe.to_string_lossy();

        // AppImage: the executable IS the package — just remove it.
        if exe_str.ends_with(".AppImage") || std::env::var("APPIMAGE").is_ok() {
            let target = std::env::var("APPIMAGE")
                .map(std::path::PathBuf::from)
                .unwrap_or(exe.clone());
            std::fs::remove_file(&target).map_err(|e| format!("Failed to remove AppImage: {e}"))?;
            app.exit(0);
            return Ok(());
        }

        // System package: we can't elevate from inside the app, so surface the command.
        let hint = if std::path::Path::new("/usr/bin/apt").exists() {
            "sudo apt remove librefang"
        } else if std::path::Path::new("/usr/bin/dnf").exists() {
            "sudo dnf remove librefang"
        } else if std::path::Path::new("/usr/bin/pacman").exists() {
            "sudo pacman -R librefang"
        } else {
            "use your distro's package manager to remove librefang"
        };
        Err(format!("To uninstall, run in a terminal: {hint}"))
    }

    #[cfg(any(target_os = "ios", target_os = "android"))]
    {
        // Mobile: uninstall through the platform app store / system settings.
        Err("Uninstall via the platform app store or system settings.".to_string())
    }
}
