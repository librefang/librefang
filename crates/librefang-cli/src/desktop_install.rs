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
            return linux_install_path_in(&home);
        }
    }

    None
}

/// Linux variant of [`platform_install_path`] parameterised on the home
/// directory so it can be exercised under a tempdir in tests.
#[cfg_attr(not(test), cfg(target_os = "linux"))]
#[allow(dead_code)]
fn linux_install_path_in(home: &Path) -> Option<PathBuf> {
    let local_bin = home.join(".local/bin/librefang-desktop");
    if local_bin.exists() {
        return Some(local_bin);
    }
    let appimage = home.join("Applications/LibreFang.AppImage");
    if appimage.exists() {
        return Some(appimage);
    }
    None
}

/// Platform-specific installation. Returns the path to the installed binary.
// On non-desktop targets (e.g. Android in the CLI release matrix) every cfg
// branch below is excluded, so `downloaded` is unused — silence the lint there.
#[cfg_attr(
    not(any(target_os = "macos", target_os = "windows", target_os = "linux")),
    allow(unused_variables)
)]
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
    install_linux_appimage_to(appimage_path, &dest_dir)
}

/// Inner, dependency-injected variant of [`install_linux_appimage`] that
/// takes an explicit destination directory so tests can route writes into a
/// tempdir instead of the user's real `~/.local/bin`.
#[cfg_attr(not(test), cfg(target_os = "linux"))]
#[allow(dead_code)]
fn install_linux_appimage_to(appimage_path: &Path, dest_dir: &Path) -> Result<PathBuf, String> {
    std::fs::create_dir_all(dest_dir)
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
#[cfg_attr(not(test), cfg(target_os = "macos"))]
#[allow(dead_code)]
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

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    //! Scoped tests for #3582 — `desktop_install.rs` previously had 0 tests
    //! and writes to the user filesystem. All filesystem mutations here are
    //! routed through `tempfile::TempDir` so nothing escapes the tempdir.

    use super::*;
    use tempfile::TempDir;

    #[test]
    fn desktop_binary_name_matches_platform() {
        let name = desktop_binary_name();
        if cfg!(windows) {
            assert_eq!(name, "librefang-desktop.exe");
        } else {
            assert_eq!(name, "librefang-desktop");
        }
    }

    #[test]
    #[allow(clippy::nonminimal_bool)]
    fn platform_asset_suffix_is_consistent_with_target() {
        let suffix = platform_asset_suffix();
        // Every supported (os, arch) triple known to the function returns
        // Some; on any other platform it must return None rather than
        // panicking. The expression mirrors the matrix in
        // `platform_asset_suffix` one-for-one for auditability — clippy's
        // suggested simplification merges unrelated platforms and obscures
        // intent, so we keep the explicit form.
        let supported = (cfg!(target_os = "macos")
            && (cfg!(target_arch = "aarch64") || cfg!(target_arch = "x86_64")))
            || (cfg!(target_os = "windows")
                && (cfg!(target_arch = "x86_64") || cfg!(target_arch = "aarch64")))
            || (cfg!(target_os = "linux") && cfg!(target_arch = "x86_64"));
        assert_eq!(suffix.is_some(), supported);

        if let Some(s) = suffix {
            // Must be a recognised installer extension.
            assert!(
                s.ends_with(".dmg") || s.ends_with(".exe") || s.ends_with(".AppImage"),
                "unexpected asset suffix: {s}"
            );
        }
    }

    #[test]
    fn which_lookup_finds_existing_binary_in_path() {
        let tmp = TempDir::new().expect("tempdir");
        let bin_dir = tmp.path().join("bin");
        std::fs::create_dir_all(&bin_dir).unwrap();
        let bin_name = "librefang-desktop-test-marker";
        let bin_path = bin_dir.join(bin_name);
        std::fs::write(&bin_path, b"#!/bin/sh\n").unwrap();

        // Scoped PATH override; restored before drop.
        let prev = std::env::var_os("PATH");
        // SAFETY: tests are single-threaded for this module under cargo's
        // default test runner per-binary; we restore in this same scope.
        unsafe {
            std::env::set_var("PATH", bin_dir.as_os_str());
        }
        let found = which_lookup(bin_name);
        unsafe {
            match prev {
                Some(v) => std::env::set_var("PATH", v),
                None => std::env::remove_var("PATH"),
            }
        }

        assert_eq!(found.as_deref(), Some(bin_path.as_path()));
    }

    #[test]
    fn which_lookup_returns_none_when_missing() {
        let tmp = TempDir::new().expect("tempdir");
        let prev = std::env::var_os("PATH");
        unsafe {
            std::env::set_var("PATH", tmp.path().as_os_str());
        }
        let found = which_lookup("definitely-not-a-real-binary-xyzzy-3582");
        unsafe {
            match prev {
                Some(v) => std::env::set_var("PATH", v),
                None => std::env::remove_var("PATH"),
            }
        }
        assert!(found.is_none());
    }

    #[test]
    fn linux_install_path_in_returns_none_on_empty_home() {
        let tmp = TempDir::new().expect("tempdir");
        assert!(linux_install_path_in(tmp.path()).is_none());
    }

    #[test]
    fn linux_install_path_in_finds_local_bin_first() {
        let tmp = TempDir::new().expect("tempdir");
        let local_bin = tmp.path().join(".local/bin");
        std::fs::create_dir_all(&local_bin).unwrap();
        let bin = local_bin.join("librefang-desktop");
        std::fs::write(&bin, b"x").unwrap();

        let found = linux_install_path_in(tmp.path()).expect("should find binary");
        assert_eq!(found, bin);
        // Must stay inside the tempdir — no escape to the real home.
        assert!(found.starts_with(tmp.path()));
    }

    #[test]
    fn linux_install_path_in_falls_back_to_appimage() {
        let tmp = TempDir::new().expect("tempdir");
        let app_dir = tmp.path().join("Applications");
        std::fs::create_dir_all(&app_dir).unwrap();
        let appimage = app_dir.join("LibreFang.AppImage");
        std::fs::write(&appimage, b"AI\x02").unwrap();

        let found = linux_install_path_in(tmp.path()).expect("should find AppImage");
        assert_eq!(found, appimage);
        assert!(found.starts_with(tmp.path()));
    }

    #[cfg(unix)]
    #[test]
    fn install_linux_appimage_to_copies_into_dest_and_marks_executable() {
        use std::os::unix::fs::PermissionsExt;

        let tmp = TempDir::new().expect("tempdir");
        let src = tmp.path().join("LibreFang_amd64.AppImage");
        std::fs::write(&src, b"FAKE-APPIMAGE-PAYLOAD").unwrap();

        let dest_dir = tmp.path().join("home/.local/bin");
        // Note: dest_dir does NOT exist yet — install must create it.
        let installed = install_linux_appimage_to(&src, &dest_dir).expect("install ok");

        assert_eq!(installed, dest_dir.join("librefang-desktop"));
        assert!(installed.starts_with(tmp.path()), "must not escape tempdir");
        assert!(installed.exists());

        let copied = std::fs::read(&installed).unwrap();
        assert_eq!(copied, b"FAKE-APPIMAGE-PAYLOAD");

        let mode = std::fs::metadata(&installed).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o755, "AppImage must be marked executable");
    }

    #[test]
    fn install_linux_appimage_to_errors_on_missing_source() {
        let tmp = TempDir::new().expect("tempdir");
        let missing = tmp.path().join("nope.AppImage");
        let dest_dir = tmp.path().join("dest");

        let err =
            install_linux_appimage_to(&missing, &dest_dir).expect_err("missing source must fail");
        assert!(
            err.contains("Failed to copy AppImage"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn find_parent_app_bundle_walks_up_to_dot_app() {
        let tmp = TempDir::new().expect("tempdir");
        let bundle = tmp.path().join("LibreFang.app");
        let macos = bundle.join("Contents/MacOS");
        std::fs::create_dir_all(&macos).unwrap();
        let bin = macos.join("LibreFang");
        std::fs::write(&bin, b"x").unwrap();

        let found = find_parent_app_bundle(&bin).expect("should locate .app");
        // Compare canonicalised paths to tolerate /private/var vs /var on macOS.
        assert_eq!(
            std::fs::canonicalize(found).unwrap(),
            std::fs::canonicalize(&bundle).unwrap()
        );
    }

    #[test]
    fn find_parent_app_bundle_returns_none_when_no_bundle() {
        let tmp = TempDir::new().expect("tempdir");
        let bin_dir = tmp.path().join("usr/local/bin");
        std::fs::create_dir_all(&bin_dir).unwrap();
        let bin = bin_dir.join("librefang-desktop");
        std::fs::write(&bin, b"x").unwrap();

        assert!(find_parent_app_bundle(&bin).is_none());
    }
}
