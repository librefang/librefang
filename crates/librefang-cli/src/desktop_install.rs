//! Desktop app discovery, download, and installation.
//!
//! When the user selects "Desktop App" but it is not installed locally, this
//! module offers to download the latest release from GitHub and install it
//! to the platform-standard location.

use std::path::{Path, PathBuf};

use crate::ui;

/// GitHub repository for release assets.
const GITHUB_REPO: &str = "librefang/librefang";

// ── Discovery ────────────────────────────────────────────────────────────────

/// Locate an existing desktop-app binary, returning its path if found.
///
/// Search order:
/// 1. Sibling of the current CLI executable
/// 2. PATH lookup
/// 3. Platform-specific standard install location
pub fn find_desktop_binary() -> Option<PathBuf> {
    let bin_name = desktop_binary_name();

    // 1. Sibling of current exe
    if let Some(sibling) = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|d| d.join(bin_name)))
    {
        if sibling.exists() {
            return Some(sibling);
        }
    }

    // 2. PATH lookup
    if let Some(found) = which_lookup(bin_name) {
        return Some(found);
    }

    // 3. Platform-specific locations
    platform_install_path()
}

/// Launch a desktop binary at `path`, detached from the current process.
pub fn launch(path: &Path) {
    #[cfg(target_os = "macos")]
    {
        // If path points inside a .app bundle, use `open -a` on the bundle
        if let Some(app_bundle) = find_parent_app_bundle(path) {
            match std::process::Command::new("open")
                .arg("-a")
                .arg(&app_bundle)
                .spawn()
            {
                Ok(_) => {
                    ui::success("Desktop app launched.");
                    return;
                }
                Err(e) => {
                    ui::error(&format!("Failed to launch {}: {e}", app_bundle.display()));
                }
            }
            return;
        }
    }

    match std::process::Command::new(path)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
    {
        Ok(_) => ui::success("Desktop app launched."),
        Err(e) => ui::error(&format!("Failed to launch desktop app: {e}")),
    }
}

/// Prompt user to download and install the desktop app.
/// Returns the installed binary path on success, `None` if cancelled or failed.
pub fn prompt_and_install() -> Option<PathBuf> {
    ui::hint("LibreFang Desktop is not installed.");

    let answer = crate::prompt_input("  Download and install it now? [Y/n] ");
    if !answer.is_empty()
        && !answer.eq_ignore_ascii_case("y")
        && !answer.eq_ignore_ascii_case("yes")
    {
        ui::hint("Skipped. You can install it later:");
        ui::hint("  brew install --cask librefang   (macOS)");
        ui::hint("  Or download from https://github.com/librefang/librefang/releases");
        return None;
    }

    download_and_install()
}

// ── Download & Install ───────────────────────────────────────────────────────

fn download_and_install() -> Option<PathBuf> {
    ui::step("Fetching latest release info...");

    let asset_suffix = match platform_asset_suffix() {
        Some(s) => s,
        None => {
            ui::error("Unsupported platform for automatic desktop install.");
            ui::hint("Download manually: https://github.com/librefang/librefang/releases");
            return None;
        }
    };

    // Query GitHub Releases API for latest release
    let api_url = format!("https://api.github.com/repos/{GITHUB_REPO}/releases/latest");
    let client = crate::http_client::new_client();
    let resp = match client
        .get(&api_url)
        .header("Accept", "application/vnd.github+json")
        .header("User-Agent", "librefang-cli")
        .send()
    {
        Ok(r) => r,
        Err(e) => {
            ui::error(&format!("Failed to reach GitHub: {e}"));
            return None;
        }
    };

    let body: serde_json::Value = match resp.json() {
        Ok(v) => v,
        Err(e) => {
            ui::error(&format!("Failed to parse release info: {e}"));
            return None;
        }
    };

    // Find the matching asset
    let assets = body["assets"].as_array()?;
    let asset = assets.iter().find(|a| {
        a["name"]
            .as_str()
            .is_some_and(|name| name.ends_with(asset_suffix))
    })?;

    let download_url = asset["browser_download_url"].as_str()?;
    let file_name = asset["name"].as_str()?;
    let size_bytes = asset["size"].as_u64().unwrap_or(0);

    let size_display = if size_bytes > 0 {
        format!(" ({:.1} MB)", size_bytes as f64 / 1_048_576.0)
    } else {
        String::new()
    };

    ui::kv("Asset", &format!("{file_name}{size_display}"));
    ui::step("Downloading...");

    let tmp_dir = std::env::temp_dir().join("librefang-desktop-install");
    let _ = std::fs::create_dir_all(&tmp_dir);
    let tmp_file = tmp_dir.join(file_name);

    if let Err(e) = download_file(download_url, &tmp_file) {
        ui::error(&format!("Download failed: {e}"));
        let _ = std::fs::remove_dir_all(&tmp_dir);
        return None;
    }

    ui::success("Download complete.");
    ui::step("Installing...");

    let result = install_platform(&tmp_file);

    // Clean up temp files
    let _ = std::fs::remove_dir_all(&tmp_dir);

    match result {
        Ok(installed_path) => {
            ui::success("LibreFang Desktop installed successfully.");
            Some(installed_path)
        }
        Err(e) => {
            ui::error(&format!("Installation failed: {e}"));
            None
        }
    }
}

/// Stream-download a file from `url` to `dest`.
fn download_file(url: &str, dest: &Path) -> Result<(), String> {
    let client = crate::http_client::new_client();
    let mut resp = client
        .get(url)
        .header("User-Agent", "librefang-cli")
        .send()
        .map_err(|e| format!("HTTP request failed: {e}"))?;

    let mut file = std::fs::File::create(dest)
        .map_err(|e| format!("Cannot create {}: {e}", dest.display()))?;

    resp.copy_to(&mut file)
        .map_err(|e| format!("Write error: {e}"))?;
    Ok(())
}

// ── Platform helpers ─────────────────────────────────────────────────────────

fn desktop_binary_name() -> &'static str {
    if cfg!(windows) {
        "librefang-desktop.exe"
    } else {
        "librefang-desktop"
    }
}

/// Return the asset filename suffix for the current platform/arch.
fn platform_asset_suffix() -> Option<&'static str> {
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        return Some("_aarch64.dmg");
    }
    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    {
        return Some("_x64.dmg");
    }

    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    {
        return Some("_x64-setup.exe");
    }
    #[cfg(all(target_os = "windows", target_arch = "aarch64"))]
    {
        return Some("_aarch64-setup.exe");
    }

    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    {
        return Some("_amd64.AppImage");
    }

    #[allow(unreachable_code)]
    None
}

/// Return the platform-specific binary path if already installed.
fn platform_install_path() -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        let app_binary = PathBuf::from("/Applications/LibreFang.app/Contents/MacOS/LibreFang");
        if app_binary.exists() {
            return Some(app_binary);
        }
    }

    #[cfg(target_os = "windows")]
    {
        if let Ok(local) = std::env::var("LOCALAPPDATA") {
            let p = PathBuf::from(local).join("LibreFang").join("LibreFang.exe");
            if p.exists() {
                return Some(p);
            }
        }
    }

    #[cfg(target_os = "linux")]
    {
        if let Some(home) = dirs::home_dir() {
            let p = home.join(".local/bin/librefang-desktop");
            if p.exists() {
                return Some(p);
            }
        }
        // Also check common AppImage locations
        if let Some(home) = dirs::home_dir() {
            let p = home.join("Applications/LibreFang.AppImage");
            if p.exists() {
                return Some(p);
            }
        }
    }

    None
}

/// Platform-specific installation. Returns the path to the installed binary.
fn install_platform(downloaded: &Path) -> Result<PathBuf, String> {
    #[cfg(target_os = "macos")]
    return install_macos_dmg(downloaded);

    #[cfg(target_os = "windows")]
    return install_windows(downloaded);

    #[cfg(target_os = "linux")]
    return install_linux_appimage(downloaded);

    #[allow(unreachable_code)]
    Err("Unsupported platform".into())
}

#[cfg(target_os = "macos")]
fn install_macos_dmg(dmg_path: &Path) -> Result<PathBuf, String> {
    use std::process::Command;

    // Mount the DMG
    let output = Command::new("hdiutil")
        .args(["attach", "-nobrowse", "-readonly", "-mountpoint"])
        .arg("/tmp/librefang-dmg-mount")
        .arg(dmg_path)
        .output()
        .map_err(|e| format!("hdiutil attach failed: {e}"))?;

    if !output.status.success() {
        return Err(format!(
            "hdiutil attach failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }

    let mount_point = Path::new("/tmp/librefang-dmg-mount");
    let app_src = mount_point.join("LibreFang.app");

    if !app_src.exists() {
        let _ = Command::new("hdiutil")
            .args(["detach", "/tmp/librefang-dmg-mount", "-quiet"])
            .status();
        return Err("LibreFang.app not found in DMG".into());
    }

    // Remove old installation if present
    let dest = Path::new("/Applications/LibreFang.app");
    if dest.exists() {
        std::fs::remove_dir_all(dest)
            .map_err(|e| format!("Failed to remove old installation: {e}"))?;
    }

    // Copy .app bundle to /Applications
    let cp = Command::new("cp")
        .args(["-R"])
        .arg(&app_src)
        .arg("/Applications/")
        .output()
        .map_err(|e| format!("cp failed: {e}"))?;

    // Always detach
    let _ = Command::new("hdiutil")
        .args(["detach", "/tmp/librefang-dmg-mount", "-quiet"])
        .status();

    if !cp.status.success() {
        return Err(format!(
            "Copy to /Applications failed: {}",
            String::from_utf8_lossy(&cp.stderr)
        ));
    }

    // Clear quarantine attribute so the app launches without Gatekeeper dialog
    let _ = Command::new("xattr")
        .args(["-rd", "com.apple.quarantine", "/Applications/LibreFang.app"])
        .status();

    Ok(PathBuf::from(
        "/Applications/LibreFang.app/Contents/MacOS/LibreFang",
    ))
}

#[cfg(target_os = "windows")]
fn install_windows(installer_path: &Path) -> Result<PathBuf, String> {
    use std::process::Command;

    ui::hint("Running installer...");

    // NSIS installer: run with /S for silent install
    let status = Command::new(installer_path)
        .arg("/S")
        .status()
        .map_err(|e| format!("Failed to run installer: {e}"))?;

    if !status.success() {
        return Err(format!("Installer exited with: {status}"));
    }

    // NSIS installs to %LOCALAPPDATA%\LibreFang\
    let local =
        std::env::var("LOCALAPPDATA").map_err(|_| "Cannot determine %LOCALAPPDATA%".to_string())?;
    let bin = PathBuf::from(local).join("LibreFang").join("LibreFang.exe");

    if bin.exists() {
        Ok(bin)
    } else {
        // Fallback: check the standard desktop binary name next to CLI
        Err("Installer completed but binary not found at expected location".into())
    }
}

#[cfg(target_os = "linux")]
fn install_linux_appimage(appimage_path: &Path) -> Result<PathBuf, String> {
    let dest_dir = dirs::home_dir()
        .ok_or_else(|| "Cannot determine home directory".to_string())?
        .join(".local/bin");

    std::fs::create_dir_all(&dest_dir)
        .map_err(|e| format!("Failed to create {}: {e}", dest_dir.display()))?;

    let dest = dest_dir.join("librefang-desktop");
    std::fs::copy(appimage_path, &dest).map_err(|e| format!("Failed to copy AppImage: {e}"))?;

    // Make executable
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&dest, std::fs::Permissions::from_mode(0o755));
    }

    Ok(dest)
}

/// Simple PATH lookup for a binary name.
fn which_lookup(name: &str) -> Option<PathBuf> {
    let path_var = std::env::var("PATH").ok()?;
    let separator = if cfg!(windows) { ';' } else { ':' };
    for dir in path_var.split(separator) {
        let candidate = PathBuf::from(dir).join(name);
        if candidate.exists() {
            return Some(candidate);
        }
    }
    None
}

/// Walk up from a binary path to find the enclosing `.app` bundle (macOS).
#[cfg(target_os = "macos")]
fn find_parent_app_bundle(path: &Path) -> Option<PathBuf> {
    let mut current = path.to_path_buf();
    while let Some(parent) = current.parent() {
        if parent.extension().is_some_and(|ext| ext == "app") && parent.is_dir() {
            return Some(parent.to_path_buf());
        }
        current = parent.to_path_buf();
    }
    None
}
