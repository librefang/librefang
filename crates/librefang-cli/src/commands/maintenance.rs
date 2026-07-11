//! `maintenance` CLI command handlers, split out of `main.rs`.
//!
//! Dispatched from `main.rs`; shared helpers and imports come via
//! [`crate::commands::prelude`].

use crate::commands::prelude::*;

const RELEASE_REPO: &str = "librefang/librefang";
const RELEASES_LATEST_API: &str =
    "https://api.github.com/repos/librefang/librefang/releases/latest";
const RELEASES_API: &str = "https://api.github.com/repos/librefang/librefang/releases";
const SHELL_INSTALLER_URL: &str = "https://librefang.ai/install.sh";
const POWERSHELL_INSTALLER_URL: &str = "https://librefang.ai/install.ps1";

pub(crate) enum UpdateLaunch {
    #[cfg(not(windows))]
    Completed,
    #[cfg(windows)]
    Detached,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ReleaseComparison {
    Newer,
    SameCore,
    Older,
    Unknown,
}

// ---------------------------------------------------------------------------
// Service management (boot auto-start)
// ---------------------------------------------------------------------------

/// Resolve the absolute path to the current librefang binary.
pub(crate) fn resolve_binary_path() -> std::path::PathBuf {
    std::env::current_exe()
        .unwrap_or_else(|_| std::path::PathBuf::from("librefang"))
        .canonicalize()
        .unwrap_or_else(|_| std::env::current_exe().unwrap_or_else(|_| "librefang".into()))
}

pub(crate) fn cmd_service_install() {
    // Warn if running as root — the service would be installed for root, not
    // the actual user. This catches `sudo librefang service install` mistakes.
    #[cfg(unix)]
    {
        // SAFETY: geteuid() is always safe to call.
        if unsafe { libc::geteuid() } == 0 {
            ui::error(&i18n::t("maintenance-service-install-root-error"));
            std::process::exit(1);
        }
    }

    let binary = resolve_binary_path();

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    let librefang_home = cli_librefang_home();

    #[cfg(target_os = "linux")]
    {
        service_install_linux(&binary, &librefang_home);
    }
    #[cfg(target_os = "macos")]
    {
        service_install_macos(&binary, &librefang_home);
    }
    #[cfg(windows)]
    {
        service_install_windows(&binary);
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
    {
        let _ = &binary;
        ui::error(&i18n::t("maintenance-service-unsupported"));
    }
}

#[cfg(target_os = "linux")]
pub(crate) fn service_install_linux(binary: &std::path::Path, librefang_home: &std::path::Path) {
    let home = match dirs::home_dir() {
        Some(h) => h,
        None => {
            ui::error(&i18n::t("migrate-error-home-dir"));
            return;
        }
    };
    let service_dir = home.join(".config/systemd/user");
    if let Err(e) = std::fs::create_dir_all(&service_dir) {
        ui::error(&i18n::t_args(
            "maintenance-failed-create-dir",
            &[
                ("path", &service_dir.display().to_string()),
                ("error", &e.to_string()),
            ],
        ));
        return;
    }

    let unit = format!(
        "[Unit]\n\
         Description=LibreFang Agent OS Daemon\n\
         Documentation=https://librefang.ai\n\
         After=network-online.target\n\
         Wants=network-online.target\n\
         \n\
         [Service]\n\
         Type=simple\n\
         ExecStart={binary} start --foreground\n\
         Restart=on-failure\n\
         RestartSec=5\n\
         WorkingDirectory={home}\n\
         EnvironmentFile=-{home}/env\n\
         EnvironmentFile=-{home}/secrets.env\n\
         \n\
         [Install]\n\
         WantedBy=default.target\n",
        binary = binary.display(),
        home = librefang_home.display(),
    );

    let service_path = service_dir.join("librefang.service");
    if let Err(e) = std::fs::write(&service_path, &unit) {
        ui::error(&i18n::t_args(
            "maintenance-failed-write-file",
            &[
                ("path", &service_path.display().to_string()),
                ("error", &e.to_string()),
            ],
        ));
        return;
    }
    ui::success(&i18n::t_args(
        "maintenance-wrote-file",
        &[("path", &service_path.display().to_string())],
    ));

    // Reload and enable
    let reload = std::process::Command::new("systemctl")
        .args(["--user", "daemon-reload"])
        .output();
    if let Ok(o) = &reload {
        if !o.status.success() {
            ui::error(&i18n::t("maintenance-systemctl-reload-failed"));
            return;
        }
    }
    let enable = std::process::Command::new("systemctl")
        .args(["--user", "enable", "librefang.service"])
        .output();
    match enable {
        Ok(o) if o.status.success() => {
            ui::success(&i18n::t("maintenance-service-enabled"));
            ui::hint(&i18n::t("maintenance-service-start-hint"));
            // Enable lingering so the user service runs without an active login session
            ui::hint(&i18n::t("maintenance-service-linger-hint"));
        }
        _ => ui::error(&i18n::t("maintenance-systemctl-enable-failed")),
    }
}

#[cfg(target_os = "macos")]
pub(crate) fn service_install_macos(binary: &std::path::Path, librefang_home: &std::path::Path) {
    let home = match dirs::home_dir() {
        Some(h) => h,
        None => {
            ui::error(&i18n::t("migrate-error-home-dir"));
            return;
        }
    };
    let agents_dir = home.join("Library/LaunchAgents");
    if let Err(e) = std::fs::create_dir_all(&agents_dir) {
        ui::error(&i18n::t_args(
            "maintenance-failed-create-dir",
            &[
                ("path", &agents_dir.display().to_string()),
                ("error", &e.to_string()),
            ],
        ));
        return;
    }

    let plist = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN"
  "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>ai.librefang.daemon</string>
    <key>ProgramArguments</key>
    <array>
        <string>{binary}</string>
        <string>start</string>
        <string>--foreground</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <true/>
    <key>WorkingDirectory</key>
    <string>{home}</string>
    <key>StandardOutPath</key>
    <string>{home}/daemon.log</string>
    <key>StandardErrorPath</key>
    <string>{home}/daemon.log</string>
</dict>
</plist>
"#,
        binary = binary.display(),
        home = librefang_home.display(),
    );

    let plist_path = agents_dir.join("ai.librefang.daemon.plist");

    // Unload existing service first (if any) to avoid launchctl errors
    if plist_path.exists() {
        let _ = std::process::Command::new("launchctl")
            .args(["unload", &plist_path.to_string_lossy()])
            .output();
    }

    if let Err(e) = std::fs::write(&plist_path, &plist) {
        ui::error(&i18n::t_args(
            "maintenance-failed-write-file",
            &[
                ("path", &plist_path.display().to_string()),
                ("error", &e.to_string()),
            ],
        ));
        return;
    }
    ui::success(&i18n::t_args(
        "maintenance-wrote-file",
        &[("path", &plist_path.display().to_string())],
    ));

    let load = std::process::Command::new("launchctl")
        .args(["load", &plist_path.to_string_lossy()])
        .output();
    match load {
        Ok(o) if o.status.success() => {
            ui::success(&i18n::t("maintenance-launchagent-loaded"));
        }
        Ok(o) => {
            let stderr = String::from_utf8_lossy(&o.stderr);
            ui::error(&i18n::t_args(
                "maintenance-launchctl-load-failed",
                &[("error", stderr.as_ref())],
            ));
        }
        Err(e) => ui::error(&i18n::t_args(
            "maintenance-launchctl-run-failed",
            &[("error", &e.to_string())],
        )),
    }
}

#[cfg(windows)]
pub(crate) fn service_install_windows(binary: &std::path::Path) {
    let value = format!("\"{}\" start", binary.display());
    let output = std::process::Command::new("reg")
        .args([
            "add",
            r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run",
            "/v",
            "LibreFang",
            "/t",
            "REG_SZ",
            "/d",
            &value,
            "/f",
        ])
        .output();
    match output {
        Ok(o) if o.status.success() => {
            ui::success(&i18n::t("maintenance-windows-startup-added"));
        }
        Ok(o) => {
            let stderr = String::from_utf8_lossy(&o.stderr);
            ui::error(&i18n::t_args(
                "maintenance-windows-registry-write-failed",
                &[("error", &stderr.to_string())],
            ));
        }
        Err(e) => ui::error(&i18n::t_args(
            "maintenance-windows-reg-run-failed",
            &[("error", &e.to_string())],
        )),
    }
}

pub(crate) fn cmd_service_uninstall() {
    #[cfg(target_os = "linux")]
    {
        let home = dirs::home_dir().unwrap_or_default();
        let service_path = home.join(".config/systemd/user/librefang.service");
        if service_path.exists() {
            let _ = std::process::Command::new("systemctl")
                .args(["--user", "disable", "--now", "librefang.service"])
                .output();
            match std::fs::remove_file(&service_path) {
                Ok(()) => {
                    let _ = std::process::Command::new("systemctl")
                        .args(["--user", "daemon-reload"])
                        .output();
                    ui::success(&i18n::t("maintenance-systemd-removed"));
                }
                Err(e) => ui::error(&i18n::t_args(
                    "maintenance-systemd-remove-failed",
                    &[("error", &e.to_string())],
                )),
            }
        } else {
            ui::hint(&i18n::t("maintenance-systemd-not-found"));
        }
    }
    #[cfg(target_os = "macos")]
    {
        let home = dirs::home_dir().unwrap_or_default();
        let plist_path = home.join("Library/LaunchAgents/ai.librefang.daemon.plist");
        if plist_path.exists() {
            let _ = std::process::Command::new("launchctl")
                .args(["unload", &plist_path.to_string_lossy()])
                .output();
            match std::fs::remove_file(&plist_path) {
                Ok(()) => ui::success(&i18n::t("maintenance-launchagent-removed")),
                Err(e) => ui::error(&i18n::t_args(
                    "maintenance-launchagent-remove-failed",
                    &[("error", &e.to_string())],
                )),
            }
        } else {
            ui::hint(&i18n::t("maintenance-launchagent-not-found"));
        }
    }
    #[cfg(windows)]
    {
        let output = std::process::Command::new("reg")
            .args([
                "delete",
                r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run",
                "/v",
                "LibreFang",
                "/f",
            ])
            .output();
        match output {
            Ok(o) if o.status.success() => {
                ui::success(&i18n::t("maintenance-windows-startup-removed"));
            }
            _ => ui::hint(&i18n::t("maintenance-windows-startup-not-found")),
        }
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
    {
        ui::error(&i18n::t("maintenance-service-unsupported"));
    }
}

pub(crate) fn cmd_service_status() {
    #[cfg(target_os = "linux")]
    {
        let home = dirs::home_dir().unwrap_or_default();
        let service_path = home.join(".config/systemd/user/librefang.service");
        if service_path.exists() {
            ui::success(&i18n::t("maintenance-systemd-status-registered"));
            // Show enabled/active status
            if let Ok(output) = std::process::Command::new("systemctl")
                .args(["--user", "is-enabled", "librefang.service"])
                .output()
            {
                let status = String::from_utf8_lossy(&output.stdout).trim().to_string();
                ui::kv(&i18n::t("maintenance-status-label-enabled"), &status);
            }
            if let Ok(output) = std::process::Command::new("systemctl")
                .args(["--user", "is-active", "librefang.service"])
                .output()
            {
                let status = String::from_utf8_lossy(&output.stdout).trim().to_string();
                ui::kv(&i18n::t("maintenance-status-label-active"), &status);
            }
        } else {
            ui::hint(&i18n::t("maintenance-systemd-status-not-registered"));
            ui::hint(&i18n::t("maintenance-service-install-hint"));
        }
    }
    #[cfg(target_os = "macos")]
    {
        let home = dirs::home_dir().unwrap_or_default();
        let plist_path = home.join("Library/LaunchAgents/ai.librefang.daemon.plist");
        if plist_path.exists() {
            ui::success(&i18n::t("maintenance-launchagent-status-registered"));
            if let Ok(output) = std::process::Command::new("launchctl")
                .args(["list"])
                .output()
            {
                let stdout = String::from_utf8_lossy(&output.stdout);
                let running = stdout.lines().any(|l| l.contains("ai.librefang.daemon"));
                // i18n::t() in an if-arm is a temporary dropped before the call site (E0716).
                let loaded_status = if running {
                    i18n::t("label-yes")
                } else {
                    i18n::t("label-not-loaded")
                };
                ui::kv(&i18n::t("maintenance-status-label-loaded"), &loaded_status);
            }
        } else {
            ui::hint(&i18n::t("maintenance-launchagent-status-not-registered"));
            ui::hint(&i18n::t("maintenance-service-install-hint"));
        }
    }
    #[cfg(windows)]
    {
        let output = std::process::Command::new("reg")
            .args([
                "query",
                r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run",
                "/v",
                "LibreFang",
            ])
            .output();
        match output {
            Ok(o) if o.status.success() => {
                ui::success(&i18n::t("maintenance-windows-status-registered"));
            }
            _ => {
                ui::hint(&i18n::t("maintenance-windows-status-not-registered"));
                ui::hint(&i18n::t("maintenance-service-install-hint"));
            }
        }
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
    {
        ui::error(&i18n::t("maintenance-service-unsupported"));
    }
}

pub(crate) fn cmd_reset(confirm: bool) {
    let librefang_dir = cli_librefang_home();

    if !librefang_dir.exists() {
        println!(
            "{}",
            i18n::t_args(
                "reset-not-needed",
                &[("path", &librefang_dir.display().to_string())]
            )
        );
        return;
    }

    if !confirm {
        println!(
            "{}",
            i18n::t_args(
                "reset-confirm-message",
                &[("path", &librefang_dir.display().to_string())]
            )
        );
        let answer = prompt_input(&i18n::t("reset-confirm-prompt"));
        if answer.trim() != "yes" {
            println!("{}", i18n::t("uninstall-cancelled"));
            return;
        }
    }

    match std::fs::remove_dir_all(&librefang_dir) {
        Ok(()) => ui::success(&i18n::t_args(
            "reset-success",
            &[("path", &librefang_dir.display().to_string())],
        )),
        Err(e) => {
            ui::error(&i18n::t_args(
                "reset-fail",
                &[
                    ("path", &librefang_dir.display().to_string()),
                    ("error", &e.to_string()),
                ],
            ));
            std::process::exit(1);
        }
    }
}

pub(crate) fn cmd_update(check: bool, version: Option<String>, channel_override: Option<String>) {
    use librefang_types::config::UpdateChannel;

    let current_exe = std::env::current_exe().unwrap_or_else(|e| {
        ui::error(&i18n::t_args(
            "maintenance-update-error-exe-path",
            &[("error", &e.to_string())],
        ));
        std::process::exit(1);
    });

    let current_version = env!("CARGO_PKG_VERSION");
    let current_exe_display = current_exe.display().to_string();
    let requested_version = version.as_deref();

    // Resolve update channel: CLI arg > config.toml > default (stable)
    let channel = if let Some(ref ch) = channel_override {
        match ch.parse::<UpdateChannel>() {
            Ok(c) => c,
            Err(e) => {
                ui::error(&e);
                std::process::exit(1);
            }
        }
    } else {
        load_update_channel_from_config().unwrap_or_default()
    };

    ui::section(&i18n::t("maintenance-update-section"));
    ui::kv(&i18n::t("label-current"), current_version);
    ui::kv(&i18n::t("label-channel"), &channel.to_string());
    ui::kv(&i18n::t("label-binary"), &current_exe_display);

    let latest_tag = if requested_version.is_none() {
        match fetch_latest_release_tag(channel) {
            Ok(tag) => {
                ui::kv(&i18n::t("label-latest"), &tag);
                Some(tag)
            }
            Err(err) => {
                if check {
                    ui::error(&i18n::t_args(
                        "maintenance-update-error-check-release",
                        &[("error", &err.to_string())],
                    ));
                    std::process::exit(1);
                }
                ui::warn_with_fix(
                    &i18n::t_args(
                        "maintenance-update-warn-resolve-release",
                        &[("error", &err.to_string())],
                    ),
                    &i18n::t("maintenance-update-warn-resolve-release-fix"),
                );
                None
            }
        }
    } else {
        if let Some(target) = requested_version {
            ui::kv(&i18n::t("label-target"), target);
        }
        None
    };
    let target_tag = requested_version
        .map(str::to_owned)
        .or_else(|| latest_tag.clone());
    let target_comparison = target_tag
        .as_deref()
        .map(|tag| compare_release_tag(tag, current_version));

    if check {
        match (target_tag.as_deref(), target_comparison) {
            (Some(tag), Some(ReleaseComparison::Newer)) => {
                ui::warn_with_fix(
                    &i18n::t_args("maintenance-update-available", &[("tag", tag)]),
                    &i18n::t("maintenance-update-run-hint"),
                );
            }
            (Some(tag), Some(ReleaseComparison::SameCore)) => {
                ui::warn_with_fix(
                    &i18n::t_args(
                        "maintenance-update-same-core",
                        &[("tag", tag), ("current", current_version)],
                    ),
                    &i18n::t("maintenance-update-same-core-hint"),
                );
            }
            (Some(tag), Some(ReleaseComparison::Older)) => {
                ui::success(&i18n::t_args(
                    "maintenance-update-ahead",
                    &[("current", current_version), ("tag", tag)],
                ));
            }
            (Some(tag), Some(ReleaseComparison::Unknown)) => {
                ui::warn_with_fix(
                    &i18n::t_args("maintenance-update-compare-unknown", &[("tag", tag)]),
                    &i18n::t("maintenance-update-compare-unknown-hint"),
                );
            }
            _ => {
                ui::warn_with_fix(
                    &i18n::t("maintenance-update-unable-to-determine"),
                    &i18n::t("maintenance-update-unable-to-determine-hint"),
                );
            }
        }
        return;
    }

    if requested_version.is_none() {
        match (latest_tag.as_deref(), target_comparison) {
            (Some(tag), Some(ReleaseComparison::Older)) => {
                ui::success(&i18n::t_args(
                    "maintenance-update-ahead",
                    &[("current", current_version), ("tag", tag)],
                ));
                return;
            }
            (Some(tag), Some(ReleaseComparison::Unknown)) => {
                ui::warn_with_fix(
                    &i18n::t_args("maintenance-update-cannot-compare-safely", &[("tag", tag)]),
                    &i18n::t_args(
                        "maintenance-update-cannot-compare-safely-hint",
                        &[("tag", tag)],
                    ),
                );
                return;
            }
            _ => {}
        }
    }

    let default_install = default_install_executable();
    let cargo_install = cargo_install_executable();
    let target_version = target_tag.as_deref();
    // Explicit --version is a hard pin; auto-resolved latest is a soft preference — see installer_version_env.
    let target_pinned = requested_version.is_some();

    #[cfg(windows)]
    if same_path(&current_exe, &default_install) && find_daemon().is_some() {
        ui::error_with_fix(
            &i18n::t("maintenance-update-windows-daemon-running-error"),
            &i18n::t("maintenance-update-windows-daemon-running-error-fix"),
        );
        std::process::exit(1);
    }

    if same_path(&current_exe, &default_install) {
        match run_official_update(target_version, target_pinned) {
            #[cfg(not(windows))]
            Ok(UpdateLaunch::Completed) => {
                ui::success(&i18n::t("maintenance-update-cli-success"));
                if let Some(installed) = installed_binary_version(&default_install) {
                    ui::kv(&i18n::t("label-installed"), &installed);
                }
                // Merge any new config defaults added in the updated binary.
                // Spawn the new binary rather than calling cmd_init_upgrade() here,
                // because the current process still holds the old binary's template.
                ui::blank();
                ui::hint(&i18n::t("maintenance-update-merging-config-defaults"));
                let _ = std::process::Command::new(&default_install)
                    .args(["init", "--upgrade"])
                    .status();
                ui::hint(&i18n::t("maintenance-update-restart-daemon-hint"));
            }
            #[cfg(windows)]
            Ok(UpdateLaunch::Detached) => {
                ui::success(&i18n::t("maintenance-update-background-launched"));
                ui::hint(&i18n::t("maintenance-update-background-hint-terminal"));
                ui::hint(&i18n::t("maintenance-update-background-hint-restart"));
            }
            Err(err) => {
                ui::error(&i18n::t_args(
                    "maintenance-update-failed-error",
                    &[("error", &err.to_string())],
                ));
                std::process::exit(1);
            }
        }
        return;
    }

    if same_path(&current_exe, &cargo_install) {
        let cargo_cmd = cargo_update_command(target_version);
        ui::warn_with_fix(&i18n::t("maintenance-update-cargo-blocked"), &cargo_cmd);
        return;
    }

    let official_path = default_install.display().to_string();
    ui::warn_with_fix(
        &i18n::t_args(
            "maintenance-update-unofficial-path",
            &[("path", &official_path)],
        ),
        &manual_installer_command(target_version),
    );
    ui::hint(&i18n::t("maintenance-update-package-manager-hint"));
}

pub(crate) fn fetch_latest_release_tag(
    channel: librefang_types::config::UpdateChannel,
) -> Result<String, String> {
    use librefang_types::config::UpdateChannel;

    let client = update_http_client()?;

    match channel {
        UpdateChannel::Stable => {
            // /releases/latest returns the latest non-draft, non-prerelease
            let response = client.get(RELEASES_LATEST_API).send().map_err(|e| {
                i18n::t_args(
                    "maintenance-error-github-request",
                    &[("error", &e.to_string())],
                )
            })?;
            let status = response.status();
            if !status.is_success() {
                return Err(i18n::t_args(
                    "maintenance-error-github-status",
                    &[("status", &status.to_string())],
                ));
            }
            let body = response.json::<serde_json::Value>().map_err(|e| {
                i18n::t_args(
                    "maintenance-error-decode-release",
                    &[("error", &e.to_string())],
                )
            })?;
            body["tag_name"]
                .as_str()
                .filter(|tag| !tag.is_empty())
                .map(str::to_string)
                .ok_or_else(|| i18n::t("maintenance-error-missing-tag"))
        }
        UpdateChannel::Beta | UpdateChannel::Rc => {
            // /releases lists all releases, newest first — filter by channel
            let response = client.get(RELEASES_API).send().map_err(|e| {
                i18n::t_args(
                    "maintenance-error-github-request",
                    &[("error", &e.to_string())],
                )
            })?;
            let status = response.status();
            if !status.is_success() {
                return Err(i18n::t_args(
                    "maintenance-error-github-status",
                    &[("status", &status.to_string())],
                ));
            }
            let releases = response.json::<Vec<serde_json::Value>>().map_err(|e| {
                i18n::t_args(
                    "maintenance-error-decode-list",
                    &[("error", &e.to_string())],
                )
            })?;

            for release in &releases {
                let draft = release["draft"].as_bool().unwrap_or(false);
                if draft {
                    continue;
                }
                let Some(tag) = release["tag_name"].as_str().filter(|t| !t.is_empty()) else {
                    continue;
                };
                match channel {
                    UpdateChannel::Rc => return Ok(tag.to_string()),
                    UpdateChannel::Beta => {
                        if !tag.contains("-rc") {
                            return Ok(tag.to_string());
                        }
                    }
                    _ => unreachable!(),
                }
            }
            Err(i18n::t_args(
                "maintenance-error-no-release",
                &[("channel", &channel.to_string())],
            ))
        }
    }
}

pub(crate) fn update_http_client() -> Result<reqwest::blocking::Client, String> {
    crate::http_client::client_builder()
        .user_agent(format!("librefang-cli/{}", env!("CARGO_PKG_VERSION")))
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .map_err(|e| {
            i18n::t_args(
                "maintenance-error-http-client",
                &[("error", &e.to_string())],
            )
        })
}

pub(crate) fn compare_release_tag(tag: &str, current_version: &str) -> ReleaseComparison {
    let Some(release_core) = parse_version_core(normalize_release_tag(tag)) else {
        return ReleaseComparison::Unknown;
    };
    let Some(current_core) = parse_version_core(current_version) else {
        return ReleaseComparison::Unknown;
    };

    match release_core.cmp(&current_core) {
        std::cmp::Ordering::Greater => ReleaseComparison::Newer,
        std::cmp::Ordering::Equal => ReleaseComparison::SameCore,
        std::cmp::Ordering::Less => ReleaseComparison::Older,
    }
}

pub(crate) fn parse_version_core(version: &str) -> Option<Vec<u64>> {
    let core = version.split('-').next()?;
    if core.is_empty() {
        return None;
    }
    core.split('.')
        .map(|part| part.parse::<u64>().ok())
        .collect()
}

/// Maps version + pin intent to `LIBREFANG_VERSION` (hard pin) or `LIBREFANG_PREFERRED_VERSION` (soft hint that falls back on stuck releases).
fn installer_version_env(version: Option<&str>, pinned: bool) -> Option<(&'static str, String)> {
    let tag = version?;
    let key = if pinned {
        "LIBREFANG_VERSION"
    } else {
        "LIBREFANG_PREFERRED_VERSION"
    };
    Some((key, tag.to_string()))
}

pub(crate) fn run_official_update(
    version: Option<&str>,
    pinned: bool,
) -> Result<UpdateLaunch, String> {
    let script_url = if cfg!(windows) {
        POWERSHELL_INSTALLER_URL
    } else {
        SHELL_INSTALLER_URL
    };
    let script = download_text(script_url)?;

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;

        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        const DETACHED_PROCESS: u32 = 0x0000_0008;

        let wrapped = format!(
            "Start-Sleep -Seconds 1\r\n{script}\r\nRemove-Item $MyInvocation.MyCommand.Path -ErrorAction SilentlyContinue\r\n"
        );
        let script_path = write_update_script(&wrapped, "ps1")?;
        let script_arg = script_path.to_string_lossy().to_string();

        let mut command = std::process::Command::new("powershell");
        command
            .args([
                "-NoProfile",
                "-ExecutionPolicy",
                "Bypass",
                "-File",
                &script_arg,
            ])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .creation_flags(CREATE_NEW_PROCESS_GROUP | DETACHED_PROCESS);
        if let Some((key, value)) = installer_version_env(version, pinned) {
            command.env(key, value);
        }

        command.spawn().map_err(|e| {
            i18n::t_args(
                "maintenance-error-powershell-updater",
                &[("error", &e.to_string())],
            )
        })?;
        Ok(UpdateLaunch::Detached)
    }

    #[cfg(not(windows))]
    {
        let script_path = write_update_script(&script, "sh")?;
        let mut command = std::process::Command::new("sh");
        command.arg(&script_path);
        if let Some((key, value)) = installer_version_env(version, pinned) {
            command.env(key, value);
        }

        let status = command.status().map_err(|e| {
            i18n::t_args(
                "maintenance-error-run-installer",
                &[("error", &e.to_string())],
            )
        })?;
        let _ = std::fs::remove_file(&script_path);
        if !status.success() {
            return Err(i18n::t_args(
                "maintenance-error-installer-status",
                &[("status", &status.to_string())],
            ));
        }
        Ok(UpdateLaunch::Completed)
    }
}

pub(crate) fn download_text(url: &str) -> Result<String, String> {
    let client = update_http_client()?;
    let response = client.get(url).send().map_err(|e| {
        i18n::t_args(
            "maintenance-error-download-fail",
            &[("error", &e.to_string())],
        )
    })?;
    let status = response.status();
    if !status.is_success() {
        return Err(i18n::t_args(
            "maintenance-error-download-status",
            &[("status", &status.to_string())],
        ));
    }
    response.text().map_err(|e| {
        i18n::t_args(
            "maintenance-error-read-response",
            &[("error", &e.to_string())],
        )
    })
}

#[cfg(not(windows))]
pub(crate) fn installed_binary_version(path: &std::path::Path) -> Option<String> {
    let output = std::process::Command::new(path)
        .arg("--version")
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }

    let version = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if version.is_empty() {
        None
    } else {
        Some(version)
    }
}

pub(crate) fn write_update_script(contents: &str, extension: &str) -> Result<PathBuf, String> {
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    // SECURITY: this script is `sh`-exec'd right after. Stage it in a per-user
    // 0700 directory instead of the world-writable shared temp dir, and create
    // the file atomically with `create_new` + mode 0600. The previous
    // `fs::write` + later `restrict_file_permissions` (a) followed a pre-planted
    // symlink at the predictable `librefang-update-<pid>-<millis>` path and
    // (b) left a default-umask window a local attacker on a shared host could
    // race to swap the contents before they ran. `create_new` refuses an
    // existing path / dangling symlink and never follows one.
    let dir = cli_librefang_home().join("updates");
    std::fs::create_dir_all(&dir)
        .map_err(|e| i18n::t_args("maintenance-error-create-dir", &[("error", &e.to_string())]))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700));
    }
    let path = dir.join(format!(
        "librefang-update-{}-{unique}.{extension}",
        std::process::id()
    ));
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut f = opts.open(&path).map_err(|e| {
        i18n::t_args(
            "maintenance-error-create-script",
            &[("error", &e.to_string())],
        )
    })?;
    use std::io::Write as _;
    f.write_all(contents.as_bytes()).map_err(|e| {
        i18n::t_args(
            "maintenance-error-write-script",
            &[("error", &e.to_string())],
        )
    })?;
    Ok(path)
}

pub(crate) fn default_install_executable() -> PathBuf {
    cli_librefang_home().join("bin").join(binary_name())
}

pub(crate) fn cargo_install_executable() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join(".cargo")
        .join("bin")
        .join(binary_name())
}

pub(crate) fn binary_name() -> &'static str {
    if cfg!(windows) {
        "librefang.exe"
    } else {
        "librefang"
    }
}

pub(crate) fn same_path(left: &std::path::Path, right: &std::path::Path) -> bool {
    let left = std::fs::canonicalize(left).unwrap_or_else(|_| left.to_path_buf());
    let right = std::fs::canonicalize(right).unwrap_or_else(|_| right.to_path_buf());
    left == right
}

pub(crate) fn normalize_release_tag(tag: &str) -> &str {
    tag.strip_prefix('v').unwrap_or(tag)
}

pub(crate) fn cargo_update_command(version: Option<&str>) -> String {
    match version {
        Some(tag) => format!(
            "cargo install --git https://github.com/{RELEASE_REPO} --tag {tag} librefang-cli --force"
        ),
        None => format!(
            "cargo install --git https://github.com/{RELEASE_REPO} librefang-cli --force"
        ),
    }
}

pub(crate) fn manual_installer_command(version: Option<&str>) -> String {
    #[cfg(windows)]
    {
        match version {
            Some(tag) => {
                format!("$env:LIBREFANG_VERSION='{tag}'; irm {POWERSHELL_INSTALLER_URL} | iex")
            }
            None => format!("irm {POWERSHELL_INSTALLER_URL} | iex"),
        }
    }

    #[cfg(not(windows))]
    {
        match version {
            Some(tag) => format!("curl -fsSL {SHELL_INSTALLER_URL} | LIBREFANG_VERSION={tag} sh"),
            None => format!("curl -fsSL {SHELL_INSTALLER_URL} | sh"),
        }
    }
}

// ---------------------------------------------------------------------------
// Uninstall
// ---------------------------------------------------------------------------

pub(crate) fn cmd_uninstall(confirm: bool, keep_config: bool) {
    let librefang_dir = cli_librefang_home();
    let exe_path = std::env::current_exe().ok();

    // Step 1: Show what will be removed
    println!();
    println!("  {}", i18n::t("uninstall-warning").bold().red());
    println!();
    if librefang_dir.exists() {
        if keep_config {
            println!(
                "{}",
                i18n::t_args(
                    "uninstall-remove-data-kept",
                    &[("path", &librefang_dir.display().to_string())]
                )
            );
        } else {
            println!(
                "{}",
                i18n::t_args(
                    "uninstall-remove-all",
                    &[("path", &librefang_dir.display().to_string())]
                )
            );
        }
    }
    if let Some(ref exe) = exe_path {
        println!(
            "{}",
            i18n::t_args(
                "uninstall-remove-binary",
                &[("path", &exe.display().to_string())]
            )
        );
    }
    // Check cargo bin path
    let cargo_bin = dirs::home_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join(".cargo")
        .join("bin")
        .join(if cfg!(windows) {
            "librefang.exe"
        } else {
            "librefang"
        });
    if cargo_bin.exists() && exe_path.as_ref().is_none_or(|e| *e != cargo_bin) {
        println!(
            "{}",
            i18n::t_args(
                "uninstall-remove-cargo-binary",
                &[("path", &cargo_bin.display().to_string())]
            )
        );
    }
    println!("{}", i18n::t("uninstall-remove-autostart"));
    println!("{}", i18n::t("uninstall-clean-path"));
    println!();

    // Step 2: Confirm
    if !confirm {
        let answer = prompt_input(&i18n::t("uninstall-confirm-prompt"));
        if answer.trim() != "uninstall" {
            println!("{}", i18n::t("uninstall-cancelled"));
            return;
        }
        println!();
    }

    // Step 3: Stop running daemon
    if find_daemon().is_some() {
        println!("  {}", i18n::t("uninstall-stopping-daemon"));
        cmd_stop(None);
        // Give it a moment
        std::thread::sleep(std::time::Duration::from_secs(1));
        // Force kill if still alive
        if find_daemon().is_some() {
            if let Some(info) = read_daemon_info(&librefang_dir) {
                force_kill_pid(info.pid);
                let _ = std::fs::remove_file(librefang_dir.join("daemon.json"));
            }
        }
    }

    // Step 4: Remove auto-start entries
    let user_home = dirs::home_dir().unwrap_or_else(std::env::temp_dir);
    remove_autostart_entries(&user_home);

    // Step 5: Clean PATH from shell configs
    if let Some(ref exe) = exe_path {
        if let Some(bin_dir) = exe.parent() {
            clean_path_entries(&user_home, &bin_dir.to_string_lossy());
        }
    }

    // Step 6: Remove ~/.librefang/ data
    if librefang_dir.exists() {
        if keep_config {
            remove_dir_except_config(&librefang_dir);
            ui::success(&i18n::t("uninstall-removed-data-kept"));
        } else {
            match std::fs::remove_dir_all(&librefang_dir) {
                Ok(()) => ui::success(&i18n::t_args(
                    "uninstall-removed",
                    &[("path", &librefang_dir.display().to_string())],
                )),
                Err(e) => ui::error(&i18n::t_args(
                    "uninstall-remove-failed",
                    &[
                        ("path", &librefang_dir.display().to_string()),
                        ("error", &e.to_string()),
                    ],
                )),
            }
        }
    }

    // Step 7: Remove cargo bin copy if it exists and is separate from current exe
    if cargo_bin.exists() && exe_path.as_ref().is_none_or(|e| *e != cargo_bin) {
        match std::fs::remove_file(&cargo_bin) {
            Ok(()) => ui::success(&i18n::t_args(
                "uninstall-removed",
                &[("path", &cargo_bin.display().to_string())],
            )),
            Err(e) => ui::error(&i18n::t_args(
                "uninstall-remove-failed",
                &[
                    ("path", &cargo_bin.display().to_string()),
                    ("error", &e.to_string()),
                ],
            )),
        }
    }

    // Step 8: Remove the binary itself (skip if already removed with ~/.librefang/)
    if let Some(exe) = exe_path {
        if exe.exists() {
            remove_self_binary(&exe);
        }
    }

    println!();
    ui::success(&i18n::t("uninstall-goodbye"));
}

/// Remove auto-start / launch-agent / systemd entries.
#[allow(unused_variables)]
pub(crate) fn remove_autostart_entries(home: &std::path::Path) {
    #[cfg(windows)]
    {
        // Windows: remove from HKCU\Software\Microsoft\Windows\CurrentVersion\Run
        let output = std::process::Command::new("reg")
            .args([
                "delete",
                r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run",
                "/v",
                "LibreFang",
                "/f",
            ])
            .output();
        match output {
            Ok(o) if o.status.success() => {
                ui::success(&i18n::t("uninstall-removed-autostart-win"));
            }
            _ => {} // Entry didn't exist — that's fine
        }
    }

    #[cfg(target_os = "macos")]
    {
        let plist = home.join("Library/LaunchAgents/ai.librefang.desktop.plist");
        if plist.exists() {
            // Unload first
            let _ = std::process::Command::new("launchctl")
                .args(["unload", &plist.to_string_lossy()])
                .output();
            match std::fs::remove_file(&plist) {
                Ok(()) => ui::success(&i18n::t("uninstall-removed-launch-agent")),
                Err(e) => ui::error(&i18n::t_args(
                    "uninstall-remove-launch-fail",
                    &[("error", &e.to_string())],
                )),
            }
        }
    }

    #[cfg(target_os = "linux")]
    {
        let desktop_file = home.join(".config/autostart/LibreFang.desktop");
        if desktop_file.exists() {
            match std::fs::remove_file(&desktop_file) {
                Ok(()) => ui::success(&i18n::t("uninstall-removed-autostart-linux")),
                Err(e) => ui::error(&i18n::t_args(
                    "uninstall-remove-autostart-fail",
                    &[("error", &e.to_string())],
                )),
            }
        }

        // Also check for systemd user service
        let service_file = home.join(".config/systemd/user/librefang.service");
        if service_file.exists() {
            let _ = std::process::Command::new("systemctl")
                .args(["--user", "disable", "--now", "librefang.service"])
                .output();
            match std::fs::remove_file(&service_file) {
                Ok(()) => {
                    let _ = std::process::Command::new("systemctl")
                        .args(["--user", "daemon-reload"])
                        .output();
                    ui::success(&i18n::t("uninstall-removed-systemd"));
                }
                Err(e) => ui::error(&i18n::t_args(
                    "uninstall-remove-systemd-fail",
                    &[("error", &e.to_string())],
                )),
            }
        }
    }
}

/// Remove lines from shell config files that add librefang to PATH.
#[allow(unused_variables)]
pub(crate) fn clean_path_entries(home: &std::path::Path, librefang_dir: &str) {
    #[cfg(not(windows))]
    {
        let shell_files = [
            home.join(".bashrc"),
            home.join(".bash_profile"),
            home.join(".profile"),
            home.join(".zshrc"),
            home.join(".config/fish/config.fish"),
        ];

        for path in &shell_files {
            if !path.exists() {
                continue;
            }
            let Ok(content) = std::fs::read_to_string(path) else {
                continue;
            };
            let filtered: Vec<&str> = content
                .lines()
                .filter(|line| !is_librefang_path_line(line, librefang_dir))
                .collect();
            if filtered.len() < content.lines().count() {
                let new_content = filtered.join("\n");
                // Preserve trailing newline if original had one
                let new_content = if content.ends_with('\n') {
                    format!("{new_content}\n")
                } else {
                    new_content
                };
                if std::fs::write(path, &new_content).is_ok() {
                    ui::success(&i18n::t_args(
                        "uninstall-cleaned-path",
                        &[("path", &path.display().to_string())],
                    ));
                }
            }
        }
    }

    #[cfg(windows)]
    {
        // Read User PATH via PowerShell, filter out librefang entries, write back
        let output = std::process::Command::new("powershell")
            .args([
                "-NoProfile",
                "-Command",
                "[Environment]::GetEnvironmentVariable('PATH', 'User')",
            ])
            .output();
        if let Ok(out) = output {
            if out.status.success() {
                let current = String::from_utf8_lossy(&out.stdout);
                let current = current.trim();
                if !current.is_empty() {
                    let dir_lower = librefang_dir.to_lowercase();
                    let filtered: Vec<&str> = current
                        .split(';')
                        .filter(|entry| {
                            let e = entry.trim().to_lowercase();
                            !e.is_empty() && !e.contains("librefang") && !e.contains(&dir_lower)
                        })
                        .collect();
                    if filtered.len() < current.split(';').count() {
                        let new_path = filtered.join(";");
                        let ps_cmd = format!(
                            "[Environment]::SetEnvironmentVariable('PATH', '{}', 'User')",
                            new_path.replace('\'', "''")
                        );
                        let result = std::process::Command::new("powershell")
                            .args(["-NoProfile", "-Command", &ps_cmd])
                            .output();
                        if result.is_ok_and(|o| o.status.success()) {
                            ui::success(&i18n::t("uninstall-cleaned-path-win"));
                        }
                    }
                }
            }
        }
    }
}

/// Returns true if a shell config line is an librefang PATH export.
/// Must match BOTH an librefang reference AND a PATH-setting pattern.
#[cfg(any(not(windows), test))]
pub(crate) fn is_librefang_path_line(line: &str, librefang_dir: &str) -> bool {
    let lower = line.to_lowercase();
    let has_librefang =
        lower.contains("librefang") || lower.contains(&librefang_dir.to_lowercase());
    if !has_librefang {
        return false;
    }
    // Match common PATH-setting patterns
    lower.contains("export path=")
        || lower.contains("export path =")
        || lower.starts_with("path=")
        || lower.contains("set -gx path")
        || lower.contains("fish_add_path")
}

/// Remove everything in ~/.librefang/ except config files.
pub(crate) fn remove_dir_except_config(librefang_dir: &std::path::Path) {
    let keep = ["config.toml", ".env", "secrets.env"];
    let Ok(entries) = std::fs::read_dir(librefang_dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if keep.contains(&name_str.as_ref()) {
            continue;
        }
        let path = entry.path();
        if path.is_dir() {
            let _ = std::fs::remove_dir_all(&path);
        } else {
            let _ = std::fs::remove_file(&path);
        }
    }
}

/// Remove the currently-running binary.
pub(crate) fn remove_self_binary(exe_path: &std::path::Path) {
    #[cfg(unix)]
    {
        // On Unix, running binaries can be unlinked — the OS keeps the inode
        // alive until the process exits.
        match std::fs::remove_file(exe_path) {
            Ok(()) => ui::success(&i18n::t_args(
                "uninstall-removed",
                &[("path", &exe_path.display().to_string())],
            )),
            Err(e) => ui::error(&i18n::t_args(
                "uninstall-remove-failed",
                &[
                    ("path", &exe_path.display().to_string()),
                    ("error", &e.to_string()),
                ],
            )),
        }
    }

    #[cfg(windows)]
    {
        // Windows locks running executables. Rename first, then spawn a
        // detached process that waits briefly and deletes the renamed file.
        let old_path = exe_path.with_extension("exe.old");
        if std::fs::rename(exe_path, &old_path).is_err() {
            ui::error(&format!(
                "Could not rename binary for deferred deletion: {}",
                exe_path.display()
            ));
            return;
        }

        use std::os::windows::process::CommandExt;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        const DETACHED_PROCESS: u32 = 0x0000_0008;

        let del_cmd = format!(
            "ping -n 3 127.0.0.1 >nul & del /f /q \"{}\"",
            old_path.display()
        );
        let _ = std::process::Command::new("cmd.exe")
            .args(["/C", &del_cmd])
            .creation_flags(CREATE_NEW_PROCESS_GROUP | DETACHED_PROCESS)
            .spawn();

        ui::success(&i18n::t_args(
            "uninstall-removed",
            &[("path", &exe_path.display().to_string())],
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn installer_version_env_hard_pins_explicit_version() {
        // --version maps to LIBREFANG_VERSION (hard pin: must install exactly this tag or fail).
        assert_eq!(
            installer_version_env(Some("v2026.6.22-beta.21"), true),
            Some(("LIBREFANG_VERSION", "v2026.6.22-beta.21".to_string()))
        );
    }

    #[test]
    fn installer_version_env_soft_prefers_resolved_latest() {
        // Auto-resolved latest maps to LIBREFANG_PREFERRED_VERSION (soft hint: falls back on stuck releases).
        assert_eq!(
            installer_version_env(Some("v2026.6.22-beta.21"), false),
            Some((
                "LIBREFANG_PREFERRED_VERSION",
                "v2026.6.22-beta.21".to_string()
            ))
        );
    }

    #[test]
    fn installer_version_env_none_sets_nothing() {
        // No version → no env var; installer resolves the newest installable release.
        assert_eq!(installer_version_env(None, true), None);
        assert_eq!(installer_version_env(None, false), None);
    }
}
