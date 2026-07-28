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

pub(crate) fn cmd_service_install(system: bool) {
    // The two modes have opposite privilege requirements, so the root check has to branch before anything else runs.
    // A per-user LaunchAgent / systemd user unit installed while root would be registered for the root account and never start for the invoking user, which is the mistake the original guard was written to catch.
    // A LaunchDaemon writes to /Library/LaunchDaemons, which only root can do.
    // Each arm terminates on its own — the macOS arm returns, the others exit — rather than falling
    // through to a shared `return`. A trailing `return` after the non-macOS `process::exit` would be
    // unreachable code, which `-D warnings` turns into a build failure on exactly the platforms a
    // macOS developer never compiles.
    if system {
        #[cfg(target_os = "macos")]
        {
            // SAFETY: geteuid() reads the calling process's effective uid and cannot fail.
            if unsafe { libc::geteuid() } != 0 {
                ui::error(&i18n::t("maintenance-service-system-needs-root"));
                ui::hint(&i18n::t("maintenance-service-system-sudo-hint"));
                std::process::exit(1);
            }
            service_install_macos_system(&resolve_binary_path());
            return;
        }
        #[cfg(not(target_os = "macos"))]
        {
            ui::error(&i18n::t("maintenance-service-system-macos-only"));
            #[cfg(target_os = "linux")]
            ui::hint(&i18n::t("maintenance-service-system-linux-hint"));
            std::process::exit(1);
        }
    }

    // Warn if running as root — the service would be installed for root, not
    // the actual user. This catches `sudo librefang service install` mistakes.
    #[cfg(unix)]
    {
        // SAFETY: geteuid() is always safe to call.
        if unsafe { libc::geteuid() } == 0 {
            ui::error(&i18n::t("maintenance-service-install-root-error"));
            #[cfg(target_os = "macos")]
            ui::hint(&i18n::t("maintenance-service-install-system-hint"));
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

    let binary = xml_escape(&binary.display().to_string());
    let home = xml_escape(&librefang_home.display().to_string());
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
"#
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

/// Absolute path of the boot-time LaunchDaemon, as opposed to the per-user LaunchAgent under `~/Library/LaunchAgents`.
/// launchd loads everything in this directory at boot, before any user logs in, which is the whole reason the `--system` mode exists.
#[cfg_attr(not(test), cfg(target_os = "macos"))]
pub(crate) const MACOS_SYSTEM_PLIST_PATH: &str = "/Library/LaunchDaemons/ai.librefang.daemon.plist";

/// The account a LaunchDaemon should drop to, resolved from the invoking `sudo` session.
///
/// Unlike `macos_system_plist` below this is not compiled into test builds on other platforms: the
/// tests exercise the rendered plist, and both the constructor and the consumer here are macOS-only,
/// so widening the gate would only produce dead code off macOS.
#[cfg(target_os = "macos")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SystemServiceTarget {
    pub(crate) user: String,
    pub(crate) home: std::path::PathBuf,
    pub(crate) uid: u32,
    pub(crate) gid: u32,
}

/// Look up a user's home directory, uid and gid in the passwd database.
///
/// `dirs::home_dir()` is not usable here: under `sudo` it resolves root's home, and a LaunchDaemon
/// pointed at `/var/root/.librefang` would run against a state directory the real user cannot read.
#[cfg(target_os = "macos")]
fn passwd_entry_for(user: &str) -> Option<(std::path::PathBuf, u32, u32)> {
    use std::ffi::{CStr, CString};

    let c_user = CString::new(user).ok()?;
    // SAFETY: `getpwnam` takes a NUL-terminated string and returns either NULL or a pointer to a
    // static record owned by libc. This runs on the CLI's single thread before any daemon spawn, so
    // nothing else can overwrite that static buffer between this call and the copies below.
    let pw = unsafe { libc::getpwnam(c_user.as_ptr()) };
    if pw.is_null() {
        return None;
    }
    // SAFETY: `pw` is non-null, so libc guarantees `pw_dir` points at a NUL-terminated string and
    // that `pw_uid` / `pw_gid` are initialised.
    let (dir_ptr, uid, gid) = unsafe { ((*pw).pw_dir, (*pw).pw_uid, (*pw).pw_gid) };
    if dir_ptr.is_null() {
        return None;
    }
    // SAFETY: non-null and NUL-terminated per the guarantee above.
    let dir = unsafe { CStr::from_ptr(dir_ptr) };
    Some((std::path::PathBuf::from(dir.to_str().ok()?), uid, gid))
}

/// Resolve who the LaunchDaemon should run as.
///
/// `SUDO_USER` is the only signal that identifies the human behind a root process. Its absence means
/// the command was run from a real root login rather than through `sudo`, and there is no way to guess
/// which account's `~/.librefang` the daemon should serve — so that is an error rather than a default.
#[cfg(target_os = "macos")]
fn resolve_system_service_target() -> Option<SystemServiceTarget> {
    let user = std::env::var("SUDO_USER").ok().filter(|u| !u.is_empty())?;
    if user == "root" {
        return None;
    }
    let (home, uid, gid) = passwd_entry_for(&user)?;
    Some(SystemServiceTarget {
        user,
        home,
        uid,
        gid,
    })
}

/// Escape the XML text-node metacharacters so an operator-supplied path renders as literal text.
///
/// A plist is XML, and launchd rejects an ill-formed one outright rather than ignoring the bad node.
/// Every interpolated value here is arbitrary filesystem bytes — APFS permits every byte but `/` and NUL, so a volume named `Backup & Media` or a `LIBREFANG_HOME` containing `<` is enough to produce a file that `launchctl load` refuses while the install path still reports the plist as written.
#[cfg_attr(not(test), cfg(target_os = "macos"))]
fn xml_escape(s: &str) -> String {
    // `&` first: escaping it last would re-escape the `&` introduced by `&lt;` / `&gt;`.
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Render the LaunchDaemon plist.
///
/// Kept as a pure function so the parts that are easy to get wrong — `--foreground`, `UserName`, and
/// the `HOME` / `LIBREFANG_HOME` pair — are asserted by unit tests on every platform rather than only
/// being exercised by a privileged macOS run nobody repeats.
/// `account_home` and `librefang_home` are passed separately on purpose.
/// Deriving one from the other looks safe while the state dir is the default `~/.librefang`, but a
/// `sudo -E` invocation carrying `LIBREFANG_HOME=/opt/librefang` would make the parent directory
/// `/opt` — and `HOME=/opt` sends `dirs::home_dir()` at a directory the account does not own.
#[cfg_attr(not(test), cfg(target_os = "macos"))]
pub(crate) fn macos_system_plist(
    binary: &std::path::Path,
    account_home: &std::path::Path,
    librefang_home: &std::path::Path,
    user: &str,
) -> String {
    let binary = xml_escape(&binary.display().to_string());
    let user = xml_escape(user);
    // `HOME` is the account's real home, not the state dir: `dirs::home_dir()` reads it, and the first-start `librefang init` path exits when it resolves to nothing.
    let home = xml_escape(&account_home.display().to_string());
    let librefang_home = xml_escape(&librefang_home.display().to_string());
    format!(
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
    <key>UserName</key>
    <string>{user}</string>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <true/>
    <key>EnvironmentVariables</key>
    <dict>
        <key>HOME</key>
        <string>{home}</string>
        <key>LIBREFANG_HOME</key>
        <string>{librefang_home}</string>
    </dict>
    <key>WorkingDirectory</key>
    <string>{librefang_home}</string>
    <key>StandardOutPath</key>
    <string>{librefang_home}/daemon.log</string>
    <key>StandardErrorPath</key>
    <string>{librefang_home}/daemon.log</string>
</dict>
</plist>
"#
    )
}

/// Every path an ownership handover has to cover: `root` itself plus everything beneath it.
///
/// Symlinks are returned but never descended. `DirEntry::file_type` describes the entry itself rather
/// than its target, so a link inside the state directory pointing somewhere else cannot redirect the
/// caller's `lchown` onto an unrelated tree — the same reason the skills bundle scanner uses
/// `entry.file_type().is_dir()` instead of `path.is_dir()`.
///
/// Unreadable directories are skipped rather than aborting the walk: a subtree this process cannot
/// enumerate is one it also cannot chown, and failing the whole install over it would be worse than
/// handing over everything reachable.
#[cfg_attr(not(test), cfg(target_os = "macos"))]
fn collect_ownership_handover_paths(root: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut out = vec![root.to_path_buf()];
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let descend = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
            out.push(path.clone());
            if descend {
                stack.push(path);
            }
        }
    }
    out
}

#[cfg(target_os = "macos")]
pub(crate) fn service_install_macos_system(binary: &std::path::Path) {
    let target = match resolve_system_service_target() {
        Some(t) => t,
        None => {
            ui::error(&i18n::t("maintenance-service-system-no-sudo-user"));
            ui::hint(&i18n::t("maintenance-service-system-sudo-hint"));
            std::process::exit(1);
        }
    };

    // The per-user LaunchAgent written by `service install` carries the same `ai.librefang.daemon` label and points at the same state directory, so leaving both installed is not layering, it is a respawn loop: at login the agent's `start --foreground` finds the LaunchDaemon already holding the port, exits non-zero, and `KeepAlive` relaunches it forever.
    // Refuse before touching anything rather than create it — this runs ahead of the chown and the plist write on purpose.
    let user_agent_plist = target
        .home
        .join("Library/LaunchAgents/ai.librefang.daemon.plist");
    if user_agent_plist.exists() {
        ui::error(&i18n::t_args(
            "maintenance-service-system-agent-conflict",
            &[("path", &user_agent_plist.display().to_string())],
        ));
        ui::hint(&i18n::t("maintenance-service-system-agent-conflict-fix"));
        std::process::exit(1);
    }

    // Honour an explicitly forwarded LIBREFANG_HOME (`sudo -E`), otherwise derive it from the target
    // account rather than from root's home, which is what `cli_librefang_home()` would return here.
    let librefang_home = match std::env::var("LIBREFANG_HOME") {
        Ok(h) if !h.is_empty() => std::path::PathBuf::from(h),
        _ => target.home.join(".librefang"),
    };

    // launchd opens StandardOutPath *before* dropping to UserName, so the log file and its parent are
    // created as root and the daemon would then be unable to write to them. Create both now and hand
    // them to the target account.
    if let Err(e) = std::fs::create_dir_all(&librefang_home) {
        ui::error(&i18n::t_args(
            "maintenance-failed-create-dir",
            &[
                ("path", &librefang_home.display().to_string()),
                ("error", &e.to_string()),
            ],
        ));
        return;
    }
    let log_path = librefang_home.join("daemon.log");
    if !log_path.exists() {
        if let Err(e) = std::fs::write(&log_path, b"") {
            ui::error(&i18n::t_args(
                "maintenance-failed-write-file",
                &[
                    ("path", &log_path.display().to_string()),
                    ("error", &e.to_string()),
                ],
            ));
            return;
        }
    }
    // Hand over the whole tree, not just the directory and its log.
    // `~/.librefang` almost always exists by the time anyone reaches for `--system` — the user has run
    // `librefang init` or the LaunchAgent first — so `create_dir_all` above was a no-op and chowning
    // only the directory node would leave its contents on whatever uid created them. That is harmless
    // when the invoking user created them, and a latent failure when a `sudo librefang start` did: the
    // LaunchDaemon runs as `UserName` and hits EACCES at some arbitrary depth long after
    // `service status` has reported everything registered.
    for path in collect_ownership_handover_paths(&librefang_home) {
        if let Err(e) = std::os::unix::fs::lchown(&path, Some(target.uid), Some(target.gid)) {
            ui::error(&i18n::t_args(
                "maintenance-service-system-chown-failed",
                &[
                    ("path", &path.display().to_string()),
                    ("user", &target.user),
                    ("error", &e.to_string()),
                ],
            ));
            return;
        }
    }

    let plist = macos_system_plist(binary, &target.home, &librefang_home, &target.user);
    let plist_path = std::path::Path::new(MACOS_SYSTEM_PLIST_PATH);

    if plist_path.exists() {
        let _ = std::process::Command::new("launchctl")
            .args(["unload", MACOS_SYSTEM_PLIST_PATH])
            .output();
    }

    if let Some(parent) = plist_path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            ui::error(&i18n::t_args(
                "maintenance-failed-create-dir",
                &[
                    ("path", &parent.display().to_string()),
                    ("error", &e.to_string()),
                ],
            ));
            return;
        }
    }

    if let Err(e) = std::fs::write(plist_path, &plist) {
        ui::error(&i18n::t_args(
            "maintenance-failed-write-file",
            &[
                ("path", &plist_path.display().to_string()),
                ("error", &e.to_string()),
            ],
        ));
        return;
    }

    // launchd refuses to load a LaunchDaemon whose plist is writable by anyone but its owner, and it
    // must be owned by root. The file was just written by a root process, so only the mode needs fixing.
    use std::os::unix::fs::PermissionsExt;
    if let Err(e) = std::fs::set_permissions(plist_path, std::fs::Permissions::from_mode(0o644)) {
        ui::error(&i18n::t_args(
            "maintenance-service-system-chmod-failed",
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

    match std::process::Command::new("launchctl")
        .args(["load", MACOS_SYSTEM_PLIST_PATH])
        .output()
    {
        Ok(o) if o.status.success() => {
            ui::success(&i18n::t_args(
                "maintenance-launchdaemon-loaded",
                &[("user", &target.user)],
            ));
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

#[cfg(target_os = "macos")]
pub(crate) fn service_uninstall_macos_system() {
    let plist_path = std::path::Path::new(MACOS_SYSTEM_PLIST_PATH);
    if !plist_path.exists() {
        ui::hint(&i18n::t("maintenance-launchdaemon-not-found"));
        return;
    }
    let _ = std::process::Command::new("launchctl")
        .args(["unload", MACOS_SYSTEM_PLIST_PATH])
        .output();
    match std::fs::remove_file(plist_path) {
        Ok(()) => ui::success(&i18n::t("maintenance-launchdaemon-removed")),
        Err(e) => ui::error(&i18n::t_args(
            "maintenance-launchdaemon-remove-failed",
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

pub(crate) fn cmd_service_uninstall(system: bool) {
    // Mirrors the install branch: removing /Library/LaunchDaemons/… needs root, removing a per-user
    // agent does not, and the two must not be silently confused for one another.
    if system {
        #[cfg(target_os = "macos")]
        {
            // SAFETY: geteuid() reads the calling process's effective uid and cannot fail.
            if unsafe { libc::geteuid() } != 0 {
                ui::error(&i18n::t("maintenance-service-system-needs-root"));
                ui::hint(&i18n::t("maintenance-service-system-sudo-hint"));
                std::process::exit(1);
            }
            service_uninstall_macos_system();
            return;
        }
        #[cfg(not(target_os = "macos"))]
        {
            ui::error(&i18n::t("maintenance-service-system-macos-only"));
            std::process::exit(1);
        }
    }

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

        // Report the boot-time LaunchDaemon too, unconditionally and without a flag.
        // Reading /Library/LaunchDaemons needs no privileges, and a status command that only looked at
        // the per-user agent would report "not registered" on a machine where the daemon is installed
        // and running — the exact situation an operator runs `service status` to rule out.
        let system_plist = std::path::Path::new(MACOS_SYSTEM_PLIST_PATH);
        if system_plist.exists() {
            ui::success(&i18n::t("maintenance-launchdaemon-status-registered"));
            if let Ok(output) = std::process::Command::new("launchctl")
                .args(["print", "system/ai.librefang.daemon"])
                .output()
            {
                // i18n::t() in an if-arm is a temporary dropped before the call site (E0716).
                let loaded_status = if output.status.success() {
                    i18n::t("label-yes")
                } else {
                    i18n::t("label-not-loaded")
                };
                ui::kv(
                    &i18n::t("maintenance-status-label-daemon-loaded"),
                    &loaded_status,
                );
            }
        } else {
            ui::hint(&i18n::t("maintenance-launchdaemon-status-not-registered"));
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

    #[test]
    fn macos_system_plist_runs_in_foreground_as_the_sudo_user() {
        let plist = macos_system_plist(
            std::path::Path::new("/usr/local/bin/librefang"),
            std::path::Path::new("/Users/alice"),
            std::path::Path::new("/Users/alice/.librefang"),
            "alice",
        );

        // `--foreground` is the difference between a service launchd can supervise and one it tears
        // down immediately: plain `librefang start` forks a setsid'd child and the parent returns.
        assert!(
            plist.contains("<string>--foreground</string>"),
            "plist must pass --foreground:\n{plist}"
        );
        // Without UserName the daemon would run as root against a user's state directory.
        assert!(
            plist.contains("<key>UserName</key>\n    <string>alice</string>"),
            "plist must drop to the sudo user:\n{plist}"
        );
        // HOME is the account home, so `dirs::home_dir()` resolves and the first-start init path does
        // not abort; LIBREFANG_HOME is the state dir the kernel actually uses.
        assert!(
            plist.contains("<key>HOME</key>\n        <string>/Users/alice</string>"),
            "HOME must be the account home:\n{plist}"
        );
        assert!(
            plist.contains(
                "<key>LIBREFANG_HOME</key>\n        <string>/Users/alice/.librefang</string>"
            ),
            "LIBREFANG_HOME must be the state dir:\n{plist}"
        );
        assert!(plist.contains("<key>RunAtLoad</key>"), "{plist}");
        assert!(plist.contains("<key>KeepAlive</key>"), "{plist}");
        assert!(
            plist.contains("<string>/Users/alice/.librefang/daemon.log</string>"),
            "logs belong under the state dir the install chowns to the target user:\n{plist}"
        );
    }

    #[test]
    fn macos_system_plist_never_points_at_root_home() {
        // The bug this whole mode has to avoid: resolving the state dir through `dirs::home_dir()`
        // while running under sudo yields root's home, and the daemon then serves a state directory
        // the real user cannot read.
        let plist = macos_system_plist(
            std::path::Path::new("/usr/local/bin/librefang"),
            std::path::Path::new("/Users/bob"),
            std::path::Path::new("/Users/bob/.librefang"),
            "bob",
        );
        assert!(
            !plist.contains("/var/root"),
            "plist must not reference root's home:\n{plist}"
        );
    }

    #[test]
    fn macos_system_plist_keeps_home_and_state_dir_independent() {
        // A `sudo -E` invocation can carry LIBREFANG_HOME to somewhere outside the account home.
        // HOME must still be the account home: deriving it from the state dir's parent would yield
        // /opt here, and `dirs::home_dir()` would then resolve a directory the account does not own.
        let plist = macos_system_plist(
            std::path::Path::new("/usr/local/bin/librefang"),
            std::path::Path::new("/Users/carol"),
            std::path::Path::new("/opt/librefang"),
            "carol",
        );
        assert!(
            plist.contains("<key>HOME</key>\n        <string>/Users/carol</string>"),
            "HOME must stay the account home even when the state dir lives elsewhere:\n{plist}"
        );
        assert!(
            plist.contains("<key>LIBREFANG_HOME</key>\n        <string>/opt/librefang</string>"),
            "{plist}"
        );
        assert!(
            !plist.contains("<string>/opt</string>"),
            "HOME must never be the state dir's parent:\n{plist}"
        );
        assert!(
            plist.contains("<string>/opt/librefang/daemon.log</string>"),
            "{plist}"
        );
    }

    #[test]
    fn xml_escape_escapes_ampersand_before_angle_brackets() {
        assert_eq!(xml_escape("Backup & Media"), "Backup &amp; Media");
        assert_eq!(xml_escape("a<b>c"), "a&lt;b&gt;c");
        // `&` has to be rewritten first, or the `&` inside `&lt;` gets escaped a second time.
        assert_eq!(xml_escape("<&>"), "&lt;&amp;&gt;");
        assert_eq!(
            xml_escape("/Users/dave/.librefang"),
            "/Users/dave/.librefang"
        );
    }

    #[test]
    fn macos_system_plist_escapes_xml_metacharacters_in_paths() {
        // APFS permits every byte but `/` and NUL, so a volume named `Backup & Media` reaches the renderer verbatim.
        // Left unescaped it is an XML well-formedness error and launchd rejects the whole file, while the install path still reports the plist as written.
        let plist = macos_system_plist(
            std::path::Path::new("/Volumes/Backup & Media/bin/librefang"),
            std::path::Path::new("/Users/a<b>"),
            std::path::Path::new("/Volumes/Backup & Media/state"),
            "a&b",
        );
        assert!(
            plist.contains("<string>/Volumes/Backup &amp; Media/bin/librefang</string>"),
            "{plist}"
        );
        assert!(plist.contains("<string>a&amp;b</string>"), "{plist}");
        assert!(
            plist.contains("<string>/Users/a&lt;b&gt;</string>"),
            "{plist}"
        );
        assert!(
            plist.contains("<string>/Volumes/Backup &amp; Media/state/daemon.log</string>"),
            "{plist}"
        );
        // No raw metacharacter may survive anywhere in the rendered document body.
        for line in plist
            .lines()
            .filter(|l| l.trim_start().starts_with("<string>"))
        {
            let inner = line
                .trim()
                .trim_start_matches("<string>")
                .trim_end_matches("</string>");
            assert!(
                !inner.contains('<') && !inner.contains('>'),
                "unescaped angle bracket in {line:?}"
            );
            assert!(
                inner
                    .split("&amp;")
                    .flat_map(|s| s.split("&lt;"))
                    .flat_map(|s| s.split("&gt;"))
                    .all(|s| !s.contains('&')),
                "unescaped ampersand in {line:?}"
            );
        }
    }

    #[test]
    fn ownership_handover_covers_nested_contents_without_following_symlinks() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();

        // A state dir shaped like a real one: nested directories plus files at depth.
        std::fs::create_dir_all(root.path().join("workspaces/agents/assistant")).unwrap();
        std::fs::write(root.path().join("config.toml"), b"").unwrap();
        std::fs::write(
            root.path().join("workspaces/agents/assistant/agent.toml"),
            b"",
        )
        .unwrap();

        // A file the walk must not reach by following a link out of the tree.
        std::fs::write(outside.path().join("unrelated.txt"), b"").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(outside.path(), root.path().join("escape")).unwrap();

        let paths = collect_ownership_handover_paths(root.path());

        // The root itself has to be in the set — chowning only the contents would leave the directory
        // node on the installing uid.
        assert!(paths.contains(&root.path().to_path_buf()), "{paths:?}");
        assert!(
            paths.contains(&root.path().join("config.toml")),
            "top-level files must be covered: {paths:?}"
        );
        assert!(
            paths.contains(&root.path().join("workspaces/agents/assistant/agent.toml")),
            "nested files must be covered — this is the case chowning only the root misses: {paths:?}"
        );

        // The link itself is handed over; what it points at is not.
        #[cfg(unix)]
        {
            assert!(
                paths.contains(&root.path().join("escape")),
                "the symlink entry itself should be chowned: {paths:?}"
            );
            assert!(
                !paths.contains(&outside.path().join("unrelated.txt")),
                "the walk must not descend through a symlink out of the tree: {paths:?}"
            );
        }
    }

    #[test]
    fn macos_system_plist_target_is_the_boot_time_directory() {
        // ~/Library/LaunchAgents only starts a job once its user logs in, which is exactly the
        // limitation `--system` exists to lift. /Library/LaunchDaemons is loaded at boot.
        assert!(
            MACOS_SYSTEM_PLIST_PATH.starts_with("/Library/LaunchDaemons/"),
            "unexpected plist path: {MACOS_SYSTEM_PLIST_PATH}"
        );
    }
}
