//! Tauri IPC command handlers.

use crate::{KernelState, PortState};
use librefang_kernel::config::librefang_home;
use librefang_kernel::AgentSubsystemApi;
use librefang_kernel::SkillsSubsystemApi;
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
    let agents = inner.kernel.agent_registry_ref().list().len();
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
    Ok(inner.kernel.agent_registry_ref().list().len())
}

/// Open a native file picker to import an agent TOML manifest.
///
/// Validates the TOML as a valid `AgentManifest`, spawns the agent, then copies it to `~/.librefang/workspaces/agents/{name}/agent.toml`.
/// Nothing is written to disk unless a local kernel is running and the spawn succeeds, so a failed import never leaves an orphan manifest that the next boot would pick up.
///
/// `async` moves it off the main thread: a synchronous command runs on the event-loop thread, where `blocking_pick_file` waits on a dialog that needs that same loop to make progress, and the app deadlocks.
#[tauri::command(async)]
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
    validate_import_agent_name(&agent_name)?;
    let agent_dir = librefang_home()
        .join("workspaces")
        .join("agents")
        .join(&agent_name);

    let guard = kernel_state.0.read().unwrap_or_else(|p| p.into_inner());
    let inner = guard
        .as_ref()
        .ok_or_else(|| "No local server running".to_string())?;
    inner
        .kernel
        .spawn_agent(manifest)
        .map_err(|e| format!("Failed to spawn agent: {e}"))?;
    drop(guard);

    std::fs::create_dir_all(&agent_dir).map_err(|e| {
        format!("Agent \"{agent_name}\" is running but its directory could not be created: {e}")
    })?;
    let dest = agent_dir.join("agent.toml");
    std::fs::write(&dest, &content).map_err(|e| {
        format!("Agent \"{agent_name}\" is running but its manifest could not be saved: {e}")
    })?;

    info!("Imported and spawned agent \"{agent_name}\"");
    Ok(agent_name)
}

/// Reject an imported manifest name that is unsafe to use as a directory name.
///
/// The name comes from a user-picked file and is joined under `workspaces/agents/`, so `..`, path separators, an absolute path, or an empty name would write outside that directory (or onto it).
/// The charset matches the API's template-name filter and the kernel's `safe_path_component`.
fn validate_import_agent_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("Invalid agent manifest: name must not be empty".to_string());
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(format!(
            "Invalid agent manifest: name {name:?} may only contain ASCII letters, digits, '-' and '_'"
        ));
    }
    librefang_types::agent::validate_agent_name(name)
        .map_err(|e| format!("Invalid agent manifest: {e}"))
}

/// Open a native file picker to import a skill manifest.
///
/// The registry only loads skill directories (`skills/<name>/skill.toml` or `skills/<name>/SKILL.md`), so the picked file is installed as the manifest of a new `skills/<name>/` directory, never as a loose file next to them.
/// The skill registry is then hot-reloaded, and the import fails (and is rolled back) unless the new skill actually loaded.
///
/// `async` for the same reason as [`import_agent_toml`]: `blocking_pick_file` deadlocks on the main thread.
#[tauri::command(async)]
pub fn import_skill_file(
    app: tauri::AppHandle,
    kernel_state: tauri::State<'_, KernelState>,
) -> Result<String, String> {
    let path = app
        .dialog()
        .file()
        .set_title("Import Skill")
        .add_filter("Skill manifests", &["md", "toml"])
        .blocking_pick_file();

    let file_path = match path {
        Some(p) => p,
        None => return Err("No file selected".to_string()),
    };
    let src = file_path.as_path().ok_or("Invalid file path")?;

    let guard = kernel_state.0.read().unwrap_or_else(|p| p.into_inner());
    let inner = guard
        .as_ref()
        .ok_or_else(|| "No local server running".to_string())?;
    // The kernel reloads from its own home, which can differ from the default `librefang_home()`.
    let skills_dir = inner.kernel.home_dir().join("skills");
    let count_before = inner
        .kernel
        .skill_registry_ref()
        .read()
        .unwrap_or_else(|p| p.into_inner())
        .count();

    let (skill_name, skill_dir) = stage_skill_import(&skills_dir, src)?;
    let outcome = inner.kernel.reload_skills();
    drop(guard);

    if outcome.frozen {
        return Err(format!(
            "Skill \"{skill_name}\" was installed, but the skill registry is frozen (Stable mode); restart LibreFang to load it"
        ));
    }
    if outcome.count <= count_before {
        let _ = std::fs::remove_dir_all(&skill_dir);
        return Err(format!(
            "Skill \"{skill_name}\" did not load: the manifest is invalid or was blocked by the security scan (see the logs)"
        ));
    }

    info!("Imported skill \"{skill_name}\" and reloaded registry");
    Ok(skill_name)
}

/// Install `src` as the manifest of a new skill directory under `skills_dir`, returning the directory's name and path.
///
/// A `.md` file becomes `SKILL.md` and a `.toml` file `skill.toml`; anything else is refused, since a lone script or module is not a loadable skill.
/// The directory is named after the file stem, or after the file's parent directory when the file is itself called `SKILL.md` / `skill.toml`.
/// An existing directory is never overwritten.
fn stage_skill_import(
    skills_dir: &std::path::Path,
    src: &std::path::Path,
) -> Result<(String, std::path::PathBuf), String> {
    let ext = src
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase);
    let manifest_name = match ext.as_deref() {
        Some("md") => "SKILL.md",
        Some("toml") => "skill.toml",
        _ => {
            return Err("A skill is imported from its SKILL.md or skill.toml manifest".to_string())
        }
    };
    let stem = src
        .file_stem()
        .and_then(|s| s.to_str())
        .ok_or("Invalid file name")?;
    let name = if stem.eq_ignore_ascii_case("skill") {
        src.parent()
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str())
            .ok_or("Cannot name the skill: rename the file after the skill")?
    } else {
        stem
    };
    if name.is_empty()
        || !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(format!(
            "Skill name {name:?} may only contain ASCII letters, digits, '-' and '_'; rename the file"
        ));
    }

    let skill_dir = skills_dir.join(name);
    if skill_dir.exists() {
        return Err(format!("A skill named \"{name}\" is already installed"));
    }
    std::fs::create_dir_all(&skill_dir)
        .map_err(|e| format!("Failed to create skill directory: {e}"))?;
    if let Err(e) = std::fs::copy(src, skill_dir.join(manifest_name)) {
        let _ = std::fs::remove_dir_all(&skill_dir);
        return Err(format!("Failed to copy skill manifest: {e}"));
    }
    Ok((name.to_string(), skill_dir))
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

#[cfg(any(target_os = "windows", test))]
fn parse_registry_uninstall_string(output: &str) -> Option<&str> {
    output.lines().find_map(|line| {
        let remainder = line.trim_start().strip_prefix("UninstallString")?;
        let value = remainder.trim_start().strip_prefix("REG_SZ")?.trim();
        (!value.is_empty()).then_some(value)
    })
}

#[cfg(any(target_os = "windows", test))]
fn uninstall_process_from_argv(
    argv: Vec<std::ffi::OsString>,
) -> Result<(std::ffi::OsString, Vec<std::ffi::OsString>), String> {
    let mut argv = argv.into_iter();
    let program = argv
        .next()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "UninstallString contains no executable".to_string())?;
    Ok((program, argv.collect()))
}

#[cfg(target_os = "windows")]
fn parse_windows_command_line(command_line: &str) -> Result<Vec<std::ffi::OsString>, String> {
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::UI::Shell::CommandLineToArgvW;

    let mut wide: Vec<u16> = std::ffi::OsStr::new(command_line)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let mut argc = 0_i32;
    // SAFETY: `wide` is writable, NUL-terminated storage that remains live for the call, and `argc` is a valid out pointer.
    let argv = unsafe { CommandLineToArgvW(wide.as_mut_ptr(), &mut argc) };
    if argv.is_null() {
        return Err(format!(
            "Failed to parse UninstallString: {}",
            std::io::Error::last_os_error()
        ));
    }

    struct LocalArgv(*mut windows_sys::core::PWSTR);
    impl Drop for LocalArgv {
        fn drop(&mut self) {
            // SAFETY: CommandLineToArgvW allocated this pointer with LocalAlloc, and ownership is released exactly once here.
            unsafe {
                LocalFree(self.0.cast());
            }
        }
    }
    let argv = LocalArgv(argv);
    if argc <= 0 {
        return Err("UninstallString contains no executable".to_string());
    }

    // SAFETY: CommandLineToArgvW returned an array containing `argc` pointers.
    let pointers = unsafe { std::slice::from_raw_parts(argv.0, argc as usize) };
    let mut parsed = Vec::with_capacity(pointers.len());
    for &pointer in pointers {
        let mut len = 0;
        // SAFETY: every returned argv element is NUL-terminated and remains live until the LocalArgv guard is dropped.
        while unsafe { *pointer.add(len) } != 0 {
            len += 1;
        }
        // SAFETY: `len` was established by scanning this live argv element.
        let value = unsafe { std::slice::from_raw_parts(pointer, len) };
        parsed.push(std::ffi::OsString::from_wide(value));
    }
    Ok(parsed)
}

/// Launch the platform uninstaller and exit the app.
///
/// - **Windows**: reads `UninstallString` from the NSIS registry key and runs it.
/// - **macOS**: moves the `.app` bundle to Trash via `osascript` + Finder.
/// - **Linux/AppImage**: deletes the AppImage binary directly.
/// - **Linux/system package**: returns a hint to run the distro uninstall command.
#[tauri::command]
#[cfg_attr(
    not(any(target_os = "windows", target_os = "macos", target_os = "linux")),
    allow(unused_variables)
)]
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

        if !output.status.success() {
            return Err("Uninstaller not found in registry. The app may have been installed without the NSIS installer.".to_string());
        }
        let stdout = String::from_utf8_lossy(&output.stdout);
        let command_line = parse_registry_uninstall_string(&stdout).ok_or_else(|| {
            "Uninstaller not found in registry. The app may have been installed without the NSIS installer.".to_string()
        })?;
        let argv = parse_windows_command_line(command_line)?;
        let (program, args) = uninstall_process_from_argv(argv)?;

        // Execute the parsed program directly.
        // Shell metacharacters from a malformed registry value remain ordinary argv entries.
        std::process::Command::new(program)
            .args(args)
            .spawn()
            .map_err(|e| format!("Failed to launch uninstaller: {e}"))?;
        app.exit(0);
        Ok(())
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

#[cfg(test)]
mod tests {
    use super::{
        parse_registry_uninstall_string, stage_skill_import, uninstall_process_from_argv,
        validate_import_agent_name,
    };
    use std::ffi::OsString;

    fn scratch_dir(tag: &str) -> std::path::PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or_default();
        let dir =
            std::env::temp_dir().join(format!("lf-desktop-{tag}-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("create scratch dir");
        dir
    }

    /// The registry skips loose files in `skills/`; the old import copied the picked file there and reported success for a skill that never loaded.
    #[test]
    fn skill_import_installs_manifest_into_its_own_directory() {
        let root = scratch_dir("skill-import");
        let skills = root.join("skills");
        let src = root.join("my-skill.md");
        std::fs::write(&src, "---\nname: my-skill\n---\nbody").unwrap();

        let (name, dir) = stage_skill_import(&skills, &src).expect("import");
        assert_eq!(name, "my-skill");
        assert_eq!(dir, skills.join("my-skill"));
        assert!(dir.join("SKILL.md").is_file());
        assert!(
            !skills.join("my-skill.md").exists(),
            "no loose file at the skills root"
        );

        // Re-importing never overwrites the installed skill.
        assert!(stage_skill_import(&skills, &src).is_err());

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn skill_import_names_a_skill_md_after_its_parent_and_maps_toml() {
        let root = scratch_dir("skill-import-parent");
        let skills = root.join("skills");
        let pkg = root.join("web_search");
        std::fs::create_dir_all(&pkg).unwrap();
        std::fs::write(pkg.join("SKILL.md"), "x").unwrap();
        let (name, dir) = stage_skill_import(&skills, &pkg.join("SKILL.md")).expect("import");
        assert_eq!(name, "web_search");
        assert!(dir.join("SKILL.md").is_file());

        let toml = root.join("calc.toml");
        std::fs::write(&toml, "x").unwrap();
        let (_, dir) = stage_skill_import(&skills, &toml).expect("import");
        assert!(dir.join("skill.toml").is_file());

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn skill_import_rejects_non_manifests_and_unsafe_names() {
        let root = scratch_dir("skill-import-reject");
        let skills = root.join("skills");
        for bad in ["tool.py", "mod.wasm", "bad name.md"] {
            let p = root.join(bad);
            std::fs::write(&p, "x").unwrap();
            assert!(stage_skill_import(&skills, &p).is_err(), "{bad}");
        }
        assert!(!skills.exists() || std::fs::read_dir(&skills).unwrap().next().is_none());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn import_agent_name_accepts_safe_names() {
        for name in ["assistant", "chat-agent", "my_agent_2"] {
            assert!(validate_import_agent_name(name).is_ok(), "{name}");
        }
    }

    #[test]
    fn import_agent_name_rejects_path_traversal_and_empty() {
        for name in [
            "",
            "..",
            "../../../Library/LaunchAgents/x",
            "/etc/passwd",
            "a/b",
            "a\\b",
            "C:evil",
            "has space",
            ".hidden",
        ] {
            assert!(
                validate_import_agent_name(name).is_err(),
                "{name:?} must be rejected"
            );
        }
    }

    #[cfg(target_os = "windows")]
    use super::parse_windows_command_line;

    #[test]
    fn parses_uninstall_string_without_fixed_column_spacing() {
        let output = concat!(
            "HKEY_CURRENT_USER\\Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\LibreFang\r\n",
            "    UninstallString\tREG_SZ\t\"C:\\Program Files\\LibreFang\\uninstall.exe\" /S\r\n",
        );

        assert_eq!(
            parse_registry_uninstall_string(output),
            Some(r#""C:\Program Files\LibreFang\uninstall.exe" /S"#)
        );
    }

    #[test]
    fn uninstall_process_keeps_shell_metacharacters_as_plain_arguments() {
        let argv = vec![
            OsString::from(r"C:\Program Files\LibreFang\uninstall.exe"),
            OsString::from("/S"),
            OsString::from("&"),
            OsString::from("calc.exe"),
        ];

        let (program, args) = uninstall_process_from_argv(argv).unwrap();
        assert_eq!(program, r"C:\Program Files\LibreFang\uninstall.exe");
        assert_eq!(
            args,
            [
                OsString::from("/S"),
                OsString::from("&"),
                OsString::from("calc.exe")
            ]
        );
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn windows_parser_does_not_interpret_shell_metacharacters() {
        let argv = parse_windows_command_line(
            r#""C:\Program Files\LibreFang\uninstall.exe" /S & calc.exe"#,
        )
        .unwrap();

        assert_eq!(
            argv,
            [
                OsString::from(r"C:\Program Files\LibreFang\uninstall.exe"),
                OsString::from("/S"),
                OsString::from("&"),
                OsString::from("calc.exe"),
            ]
        );
    }
}
