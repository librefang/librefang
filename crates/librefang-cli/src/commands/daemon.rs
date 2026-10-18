//! `daemon` CLI command handlers, split out of `main.rs`.
//!
//! Dispatched from `main.rs`; shared helpers and imports come via
//! [`crate::commands::prelude`].

use crate::commands::prelude::*;
#[cfg(unix)]
use std::os::unix::io::RawFd;

/// Days of rotated daemon logs to keep before pruning.
const LOG_RETENTION_DAYS: u64 = 7;

/// Guard that tees all stdout/stderr to a log file in foreground mode.
/// On drop, restores original stdout/stderr and joins the tee thread.
#[cfg(unix)]
pub(crate) struct ForegroundTeeGuard {
    _pipe_fd: RawFd, // kept alive to keep pipe open until guard drops
}

#[cfg(unix)]
impl Drop for ForegroundTeeGuard {
    fn drop(&mut self) {
        // Restore original stdout/stderr
        unsafe {
            libc::dup2(self._pipe_fd, libc::STDOUT_FILENO);
            libc::dup2(self._pipe_fd, libc::STDERR_FILENO);
            libc::close(self._pipe_fd);
        }
    }
}

pub(crate) fn daemon_log_path_for_home(home_dir: &std::path::Path) -> PathBuf {
    home_dir.join("logs").join("daemon.log")
}

pub(crate) fn daemon_log_path_for_config(config: Option<&std::path::Path>) -> PathBuf {
    let daemon = daemon_config_context(config);
    if let Some(ref log_dir) = daemon.log_dir {
        log_dir.join("daemon.log")
    } else {
        daemon_log_path_for_home(&daemon.home_dir)
    }
}

pub(crate) fn detached_daemon_args(config: Option<&std::path::Path>) -> Vec<OsString> {
    let mut args = Vec::new();
    if let Some(path) = config {
        args.push(OsString::from("--config"));
        args.push(path.as_os_str().to_owned());
    }
    args.push(OsString::from("start"));
    args.push(OsString::from("--spawned"));
    args
}

pub(crate) fn spawn_detached_daemon(
    config: Option<&std::path::Path>,
    log_path: &std::path::Path,
) -> Result<std::process::Child, String> {
    let exe = std::env::current_exe()
        .map_err(|e| i18n::t_args("daemon-error-resolve-exe", &[("error", &e.to_string())]))?;
    if let Some(log_dir) = log_path.parent() {
        std::fs::create_dir_all(log_dir).map_err(|e| {
            i18n::t_args(
                "daemon-error-create-log-dir",
                &[
                    ("path", &log_dir.display().to_string()),
                    ("error", &e.to_string()),
                ],
            )
        })?;
        restrict_dir_permissions(log_dir);
    }

    let stdout = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path)
        .map_err(|e| {
            i18n::t_args(
                "daemon-error-open-log",
                &[
                    ("path", &log_path.display().to_string()),
                    ("error", &e.to_string()),
                ],
            )
        })?;
    let stderr = stdout.try_clone().map_err(|e| {
        i18n::t_args(
            "daemon-error-clone-log-handle",
            &[
                ("path", &log_path.display().to_string()),
                ("error", &e.to_string()),
            ],
        )
    })?;

    let mut command = std::process::Command::new(exe);
    command
        .args(detached_daemon_args(config))
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr))
        .envs(std::env::vars());

    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;

        unsafe {
            command.pre_exec(|| {
                if libc::setsid() == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
    }

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;

        const DETACHED_PROCESS: u32 = 0x0000_0008;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;

        command.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW);
    }

    command
        .spawn()
        .map_err(|e| i18n::t_args("daemon-error-spawn-detached", &[("error", &e.to_string())]))
}

/// Generate a daily log path for the current daemon start.
/// Returns e.g. ~/.librefang/logs/daemon-2026-04-23.log
/// Same day restarts reuse the same file.
pub(crate) fn timestamped_log_path(config: Option<&std::path::Path>) -> std::path::PathBuf {
    let daemon = daemon_config_context(config);
    let log_dir = daemon
        .log_dir
        .unwrap_or_else(|| daemon.home_dir.join("logs"));
    let date = chrono_lite_date();
    log_dir.join(format!("daemon-{date}.log"))
}

/// Prune rotated daemon logs older than `max_age_days`, keeping the log dir tidy.
pub(crate) fn prune_rotated_logs(config: Option<&std::path::Path>, max_age_days: u64) {
    let daemon = daemon_config_context(config);
    let log_dir = daemon
        .log_dir
        .unwrap_or_else(|| daemon.home_dir.join("logs"));
    let cutoff = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
        .saturating_sub(max_age_days.saturating_mul(86400));

    let entries = match std::fs::read_dir(&log_dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) => n,
            None => continue,
        };
        if !name.starts_with("daemon-") || !name.ends_with(".log") {
            continue;
        }
        // Parse date from filename: daemon-YYYY-MM-DD.log
        let date_str = name
            .strip_prefix("daemon-")
            .and_then(|s| s.strip_suffix(".log"));
        let is_old = date_str
            .and_then(parse_daily_date_timestamp)
            .map(|ts| ts < cutoff)
            .unwrap_or(false);
        if is_old {
            let _ = std::fs::remove_file(&path);
        }
    }
}

/// Set up tee for --foreground mode: redirect stdout/stderr to a pipe,
/// spawn a background thread that copies to both terminal and log file.
#[cfg(unix)]
pub(crate) fn setup_foreground_tee(log_path: &std::path::Path) -> ForegroundTeeGuard {
    // Ensure the parent directory exists (e.g. `~/.librefang/logs/`) before we
    // try to open the log file. Fresh installations and test environments may
    // not have this directory yet.
    if let Some(parent) = log_path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            // Report the failure on the real stderr (not yet redirected), then
            // exit instead of limping forward into the silent-hang regime the
            // old ordering produced.
            eprintln!(
                "{}",
                i18n::t_args(
                    "daemon-error-failed-create-log-dir",
                    &[
                        ("path", &parent.display().to_string()),
                        ("error", &e.to_string())
                    ]
                )
            );
            std::process::exit(1);
        }
    }

    // Open the log file BEFORE redirecting stdout/stderr. If the open panics
    // (permissions, read-only fs, parent still missing, …) the panic message
    // reaches the real stderr the user is watching. Previously we opened the
    // file AFTER `dup2`, so a failure wrote its panic message into a pipe
    // whose reader hadn't been spawned yet — the message was trapped in the
    // pipe buffer and the process appeared to hang at "Starting daemon…".
    let log_file = std::sync::Mutex::new(
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(log_path)
            .unwrap_or_else(|e| {
                eprintln!(
                    "{}",
                    i18n::t_args(
                        "daemon-error-failed-open-log",
                        &[
                            ("path", &log_path.display().to_string()),
                            ("error", &e.to_string())
                        ]
                    )
                );
                std::process::exit(1);
            }),
    );

    // Create pipe for stdout+stderr (we'll write to it, background thread reads)
    let mut fds = [0i32, 0i32];
    unsafe {
        libc::pipe(fds.as_mut_ptr());
    }
    let pipe_write = fds[1];
    let pipe_read = fds[0];

    // Save a copy of original stdout (to restore both fd 1 and fd 2 on
    // drop). We don't keep a separate stderr copy: by the time the tee
    // thread reads from the pipe, stdout and stderr have already been
    // merged at the fd level, so we cannot route output back to the
    // correct original fd. Writing to both copies would simply duplicate
    // every line in any consumer that captures both fds (e.g. the Docker
    // log driver), which is the bug this code path used to cause.
    let stdout_copy = unsafe { libc::dup(libc::STDOUT_FILENO) };

    // Redirect stdout and stderr to the pipe. From here on any write to the
    // standard streams goes through the pipe and must be drained by the
    // read thread below — do not fail between this point and `thread::spawn`.
    unsafe {
        libc::dup2(pipe_write, libc::STDOUT_FILENO);
        libc::dup2(pipe_write, libc::STDERR_FILENO);
        libc::close(pipe_write);
    }

    // Spawn background thread that reads from pipe and writes to both terminal and log
    std::thread::spawn(move || {
        let mut buf = [0u8; 8192];
        loop {
            let n =
                unsafe { libc::read(pipe_read, buf.as_mut_ptr() as *mut libc::c_void, buf.len()) };
            if n <= 0 {
                // EOF or error — exit; guard Drop will restore stdout/stderr
                unsafe { libc::close(pipe_read) };
                break;
            }
            // Write to the saved stdout fd once. See comment at the dup site
            // for why we don't also write to a stderr copy.
            unsafe {
                libc::write(stdout_copy, buf.as_ptr() as *const libc::c_void, n as usize);
            }
            // Write to log file
            if let Ok(mut f) = log_file.lock() {
                let _ = f.write_all(&buf[..n as usize]);
                let _ = f.flush();
            }
        }
        // guard Drop closes stdout_copy; pipe_read is closed above on break
    });

    ForegroundTeeGuard {
        _pipe_fd: stdout_copy,
    }
}

/// Ensure LibreFang is initialized (config.toml exists). Auto-runs quick init on first run.
pub(crate) fn ensure_initialized(config: &Option<PathBuf>) {
    match config {
        None => {
            if !cli_config_path().exists() {
                ui::hint(&i18n::t("daemon-first-run-setup"));
                cmd_init(true);
            }
        }
        Some(path) => {
            if !path.exists() {
                ui::error_with_fix(
                    &i18n::t_args(
                        "daemon-config-not-found",
                        &[("path", &path.display().to_string())],
                    ),
                    &i18n::t("daemon-config-not-found-fix"),
                );
                std::process::exit(1);
            }
        }
    }
}

pub(crate) fn cmd_start(config: Option<PathBuf>, tail: bool, spawned: bool, foreground: bool) {
    ensure_initialized(&config);

    // Issue #5186 follow-up: `cmd_start` boots a real daemon, so a bad
    // `config.toml` must abort here BEFORE `daemon_config_context` swallows
    // the load error and substitutes `KernelConfig::default()`. The
    // tolerant default would silently change `home_dir` (used a few lines
    // below to detect an already-running daemon) and the spawned child
    // would only fail-closed during its own boot — losing the field-name
    // diagnostic from the parent's stderr in the process.
    //
    // `load_config` already prints the underlying error to stderr (so
    // operators see the field name even before the tracing subscriber is
    // wired up); we only need to short-circuit with a non-zero exit.
    if load_config(config.as_deref()).is_err() {
        std::process::exit(1);
    }

    let daemon = daemon_config_context(config.as_deref());
    if let Some(base) = find_daemon_in_home(&daemon.home_dir) {
        ui::error_with_fix(
            &i18n::t_args("daemon-already-running", &[("url", &base)]),
            &i18n::t("daemon-already-running-fix"),
        );
        std::process::exit(1);
    }

    if !spawned && !foreground {
        let log_path = daemon_log_path_for_config(config.as_deref());
        let mut child = spawn_detached_daemon(config.as_deref(), &log_path).unwrap_or_else(|e| {
            ui::error_with_fix(&i18n::t("daemon-launch-fail"), &e);
            std::process::exit(1);
        });

        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Some(base) = find_daemon_in_home(&daemon.home_dir) {
                let pid = child.id();
                std::mem::forget(child);
                ui::success(&i18n::t("daemon-started-bg"));
                ui::kv(&i18n::t("label-pid"), &pid.to_string());
                ui::kv(&i18n::t("label-api"), &base);
                ui::kv(&i18n::t("label-dashboard"), &format!("{base}/"));
                ui::kv(&i18n::t("label-log"), &log_path.display().to_string());
                if tail {
                    ui::hint(&i18n::t("hint-tail-stop"));
                    ui::blank();
                    show_log_file(&log_path, 50, true);
                } else {
                    ui::hint(&i18n::t("hint-stop-daemon"));
                }
                return;
            }

            match child.try_wait() {
                Ok(Some(status)) => {
                    ui::error_with_fix(
                        &i18n::t_args("daemon-bg-exited", &[("status", &status.to_string())]),
                        &i18n::t_args(
                            "daemon-bg-exited-fix",
                            &[("path", &log_path.display().to_string())],
                        ),
                    );
                    std::process::exit(1);
                }
                Ok(None) => {}
                Err(e) => {
                    ui::error_with_fix(
                        &i18n::t("daemon-bg-wait-fail"),
                        &i18n::t_args(
                            "daemon-bg-wait-fail-fix",
                            &[
                                ("error", &e.to_string()),
                                ("path", &log_path.display().to_string()),
                            ],
                        ),
                    );
                    std::process::exit(1);
                }
            }

            if Instant::now() >= deadline {
                let pid = child.id();
                std::mem::forget(child);
                ui::success(&i18n::t("daemon-still-starting"));
                ui::kv(&i18n::t("label-pid"), &pid.to_string());
                ui::kv(&i18n::t("label-log"), &log_path.display().to_string());
                if tail {
                    ui::hint(&i18n::t("hint-tail-stop"));
                    ui::blank();
                    show_log_file(&log_path, 50, true);
                } else {
                    ui::hint(&i18n::t("hint-check-status"));
                }
                return;
            }

            std::thread::sleep(Duration::from_millis(250));
        }
    }

    // Load `<home>/secrets.env` into the current process environment BEFORE
    // building the tokio runtime or booting the kernel. Without this, a
    // dashboard-saved provider key (`POST /api/providers/{p}/key` writes to
    // `secrets.env`) is dropped on every daemon restart because the systemd
    // unit's `EnvironmentFile=` references a different file and nothing in
    // the boot path re-reads `secrets.env`. Synchronous on the main thread —
    // `std::env::set_var` is unsound under concurrent reads, but no tokio
    // runtime exists yet so this is the safe window. See #4701.
    match librefang_api::secrets_env::load_into_process_blocking(&daemon.home_dir) {
        Ok(0) => {}
        Ok(n) => tracing::debug!("Loaded {n} entries from secrets.env"),
        Err(e) => tracing::warn!(
            "Failed to read secrets.env from {}: {e}",
            daemon.home_dir.display()
        ),
    }

    ui::banner();
    ui::blank();
    println!("  {}", i18n::t("daemon-starting"));
    ui::blank();

    // For --foreground mode, tee stdout/stderr to both the terminal and a time-stamped
    // log file. Detached mode keeps appending to the stable daemon.log.
    let log_path = if foreground {
        // Prune rotated logs older than LOG_RETENTION_DAYS, then start a fresh daily file.
        prune_rotated_logs(config.as_deref(), LOG_RETENTION_DAYS);
        timestamped_log_path(config.as_deref())
    } else {
        daemon_log_path_for_config(config.as_deref())
    };
    #[cfg(unix)]
    let _foreground_guard = if foreground {
        Some(setup_foreground_tee(&log_path))
    } else {
        None
    };
    ui::kv(&i18n::t("label-log"), &log_path.display().to_string());

    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        // Raise the worker/blocking thread stack from tokio's 2 MiB default (refs #6659).
        // An agent turn is a deep chain of very large futures — `send_message_full_inner` -> `execute_llm_agent` -> `run_agent_loop` — and a tool call that dispatches to another agent (`agent_send`, a workflow step) runs the callee's entire turn inline on the same worker, stacking that chain again.
        // Overflowing it SIGABRTs the whole process: with only 2 workers a single deep turn takes the HTTP API and every cron job down with it.
        // The depth cap on the workflow path is the real bound; this is headroom so a legal-depth chain of fat frames does not abort before the cap can refuse it.
        .thread_stack_size(8 * 1024 * 1024)
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(async {
        let kernel = match LibreFangKernel::boot(config.as_deref()) {
            Ok(k) => k,
            Err(e) => {
                boot_kernel_error(&e);
                std::process::exit(1);
            }
        };

        // Wire the live tracing filter into the kernel's hot-reload path so
        // dashboard edits to `log_level` take effect immediately instead of
        // requiring a daemon restart. Only the daemon path needs this — TUI
        // / one-shot CLI commands route through `init_tracing_file` (no
        // dashboard) so the slot stays unwired there.
        kernel.set_log_reloader(std::sync::Arc::new(log_filter::CliLogLevelReloader));

        let cfg = kernel.config_ref();
        let listen_addr = cfg.api_listen.clone();
        let daemon_info_path = kernel.home_dir().join("daemon.json");
        let provider = cfg.default_model.provider.clone();
        let model = cfg.default_model.model.clone();
        let agent_count = kernel.agent_registry_ref().count();
        let model_count = kernel.model_catalog_swap().load().list_models().len();

        ui::success(&i18n::t_args(
            "kernel-booted",
            &[("provider", &provider), ("model", &model)],
        ));
        if model_count > 0 {
            ui::success(&i18n::t_args(
                "models-available",
                &[("count", &model_count.to_string())],
            ));
        }
        if agent_count > 0 {
            ui::success(&i18n::t_args(
                "agents-loaded",
                &[("count", &agent_count.to_string())],
            ));
        }
        ui::blank();
        ui::kv(&i18n::t("label-api"), &format!("http://{listen_addr}"));
        ui::kv(
            &i18n::t("label-dashboard"),
            &format!("http://{listen_addr}/"),
        );
        ui::kv(&i18n::t("label-provider"), &provider);
        ui::kv(&i18n::t("label-model"), &model);
        ui::blank();
        ui::hint(&i18n::t("hint-open-dashboard"));
        ui::hint(&i18n::t("hint-stop-daemon"));
        ui::blank();

        if let Err(e) =
            librefang_api::server::run_daemon(kernel, &listen_addr, Some(&daemon_info_path)).await
        {
            ui::error(&i18n::t_args("daemon-error", &[("error", &e.to_string())]));
            std::process::exit(1);
        }

        ui::blank();
        println!("  {}", i18n::t("daemon-stopped"));
    });
}

pub(crate) fn cmd_stop(config: Option<PathBuf>) {
    let daemon = daemon_config_context(config.as_deref());
    match find_daemon_in_home(&daemon.home_dir) {
        Some(base) => {
            // Record the PID before asking for shutdown: the daemon removes `daemon.json` on its way out, and only the process exiting means it has released `daemon.lock`.
            let pid = read_daemon_info(&daemon.home_dir).map(|info| info.pid);
            let client = daemon_client_with_api_key(daemon.api_key.as_deref());
            match client.post(format!("{base}/api/shutdown")).send() {
                Ok(r) if r.status().is_success() => {
                    let Some(pid) = pid else {
                        // No PID to watch: fall back to waiting for the API to go away.
                        let stopped = wait_until(STOP_EXIT_BUDGET, STOP_POLL_INTERVAL, || {
                            find_daemon_in_home(&daemon.home_dir).is_none()
                        });
                        if stopped {
                            ui::success(&i18n::t("daemon-stopped-ok"));
                        } else {
                            ui::error(&i18n::t_args(
                                "daemon-stop-timeout",
                                &[("secs", &STOP_EXIT_BUDGET.as_secs().to_string())],
                            ));
                            std::process::exit(1);
                        }
                        return;
                    };
                    // The HTTP listener closes at the start of graceful shutdown, while cleanup (background tasks, bridges, MCP children) still runs under `daemon.lock` for up to the daemon's 30s hard deadline.
                    // Wait for the process itself, so `restart` does not start a second daemon that dies on the lock, and so a slow cleanup is not cut short by SIGKILL.
                    ui::hint(&i18n::t_args(
                        "daemon-stop-waiting",
                        &[("pid", &pid.to_string())],
                    ));
                    if wait_until(STOP_EXIT_BUDGET, STOP_POLL_INTERVAL, || !process_alive(pid)) {
                        ui::success(&i18n::t("daemon-stopped-ok"));
                        return;
                    }
                    // Still alive past the daemon's own deadline — force kill, then confirm it is gone.
                    force_kill_pid(pid);
                    if wait_until(
                        std::time::Duration::from_secs(5),
                        STOP_POLL_INTERVAL,
                        || !process_alive(pid),
                    ) {
                        let _ = std::fs::remove_file(daemon.home_dir.join("daemon.json"));
                        ui::success(&i18n::t("daemon-stopped-forced"));
                    } else {
                        ui::error_with_fix(
                            &i18n::t_args("daemon-stop-kill-failed", &[("pid", &pid.to_string())]),
                            &i18n::t_args(
                                "daemon-stop-kill-failed-fix",
                                &[("pid", &pid.to_string())],
                            ),
                        );
                        std::process::exit(1);
                    }
                }
                Ok(r) if r.status().as_u16() == 401 => {
                    // Issue #4693 — the new CLI cannot authenticate against
                    // the running daemon. Typical trigger: `curl install.sh
                    // | sh` upgraded the binary without restarting the
                    // daemon, so the running daemon was started with an
                    // api_key the new CLI no longer reads (locked vault,
                    // rotated key, freshly-enabled dashboard credentials).
                    // Surface the cause and fall back to PID-based stop so
                    // the user is not stuck on a half-restarted machine.
                    ui::error(&i18n::t("shutdown-401-detected"));
                    ui::hint(&i18n::t("shutdown-401-explainer"));
                    if let Some(info) = read_daemon_info(&daemon.home_dir) {
                        let pid = info.pid;
                        ui::hint(&i18n::t_args(
                            "shutdown-401-fallback-attempt",
                            &[("pid", &pid.to_string())],
                        ));
                        force_kill_pid(pid);
                        for _ in 0..10 {
                            std::thread::sleep(std::time::Duration::from_millis(500));
                            if find_daemon_in_home(&daemon.home_dir).is_none() {
                                let _ = std::fs::remove_file(daemon.home_dir.join("daemon.json"));
                                ui::success(&i18n::t_args(
                                    "shutdown-401-fallback-success",
                                    &[("pid", &pid.to_string())],
                                ));
                                return;
                            }
                        }
                        ui::error(&i18n::t("shutdown-401-fallback-fail"));
                        ui::hint(&i18n::t_args(
                            "shutdown-401-fallback-fix",
                            &[("pid", &pid.to_string())],
                        ));
                    } else {
                        let info_path = daemon.home_dir.join("daemon.json");
                        ui::hint(&i18n::t_args(
                            "shutdown-401-no-pid-fix",
                            &[("path", &info_path.display().to_string())],
                        ));
                    }
                }
                Ok(r) => {
                    ui::error(&i18n::t_args(
                        "shutdown-request-fail",
                        &[("status", &r.status().to_string())],
                    ));
                }
                Err(e) => {
                    ui::error(&i18n::t_args(
                        "could-not-reach-daemon",
                        &[("error", &e.to_string())],
                    ));
                }
            }
        }
        None => {
            ui::warn_with_fix(
                &i18n::t("daemon-no-running-found"),
                &i18n::t("daemon-no-running-found-fix"),
            );
        }
    }
}

pub(crate) fn cmd_restart(config: Option<PathBuf>, tail: bool, foreground: bool) {
    // Same fail-closed rule as `cmd_start` (#5186 follow-up): a bad config
    // must abort before we read `home_dir` to look up a running daemon, or
    // we'd `find_daemon_in_home` on `~/.librefang` (the default) and either
    // miss a real daemon at a user-configured `home_dir` or "stop" the
    // wrong one. `load_config` already eprintln!s the underlying error.
    if load_config(config.as_deref()).is_err() {
        std::process::exit(1);
    }

    let daemon = daemon_config_context(config.as_deref());
    if find_daemon_in_home(&daemon.home_dir).is_some() {
        ui::hint(&i18n::t("daemon-restarting"));
        cmd_stop(config.clone());
    } else {
        ui::hint(&i18n::t("daemon-no-running-starting"));
    }

    cmd_start(config, tail, false, foreground);
}

/// How long `stop` waits for the daemon process to exit after a successful `/api/shutdown`: a little above the daemon's own 30s `SHUTDOWN_HARD_DEADLINE`, after which it aborts itself.
const STOP_EXIT_BUDGET: std::time::Duration = std::time::Duration::from_secs(35);
const STOP_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_millis(250);

/// Poll `done` every `interval` until it returns true or `budget` elapses; returns whether it did.
fn wait_until(
    budget: std::time::Duration,
    interval: std::time::Duration,
    mut done: impl FnMut() -> bool,
) -> bool {
    let deadline = std::time::Instant::now() + budget;
    loop {
        if done() {
            return true;
        }
        if std::time::Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(interval);
    }
}

/// Whether a process with this PID exists.
fn process_alive(pid: u32) -> bool {
    #[cfg(unix)]
    {
        let Ok(pid) = libc::pid_t::try_from(pid) else {
            return false;
        };
        // SAFETY: signal 0 performs only the existence and permission check; no signal is delivered.
        if unsafe { libc::kill(pid, 0) } == 0 {
            return true;
        }
        // EPERM means the process exists but belongs to someone else.
        std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
    }
    #[cfg(windows)]
    {
        // tasklist filter expression `PID eq <pid>`; it is a command argument, not user-facing text.
        let filter = ["PID", "eq", &pid.to_string()].join(" ");
        std::process::Command::new("tasklist")
            .args(["/FI", &filter, "/NH"])
            .output()
            .map(|o| {
                o.status.success() && {
                    let out = String::from_utf8_lossy(&o.stdout);
                    !out.contains("INFO:") && out.contains(&pid.to_string())
                }
            })
            .unwrap_or(false)
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = pid;
        false
    }
}

pub(crate) fn force_kill_pid(pid: u32) {
    #[cfg(unix)]
    {
        let _ = std::process::Command::new("kill")
            .args(["-9", &pid.to_string()])
            .output();
    }
    #[cfg(windows)]
    {
        let _ = std::process::Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/F"])
            .output();
    }
}

pub(crate) fn show_log_file(log_path: &std::path::Path, lines: usize, follow: bool) {
    if !log_path.exists() {
        ui::error_with_fix(
            &i18n::t("daemon-log-file-not-found"),
            &i18n::t_args(
                "daemon-log-file-not-found-fix",
                &[("path", &log_path.display().to_string())],
            ),
        );
        std::process::exit(1);
    }

    if follow {
        // Use tail -f equivalent
        #[cfg(unix)]
        {
            let _ = std::process::Command::new("tail")
                .args(["-f", "-n", &lines.to_string()])
                .arg(log_path)
                .status();
        }
        #[cfg(windows)]
        {
            // On Windows, read in a loop
            let content = std::fs::read_to_string(log_path).unwrap_or_default();
            let all_lines: Vec<&str> = content.lines().collect();
            let start = all_lines.len().saturating_sub(lines);
            for line in &all_lines[start..] {
                println!("{line}");
            }
            println!(
                "{}",
                i18n::t_args(
                    "log-following",
                    &[("path", &log_path.display().to_string())]
                )
            );
            let mut last_len = content.len();
            loop {
                std::thread::sleep(std::time::Duration::from_millis(500));
                if let Ok(new_content) = std::fs::read_to_string(log_path) {
                    if new_content.len() > last_len {
                        print!("{}", &new_content[last_len..]);
                        last_len = new_content.len();
                    }
                }
            }
        }
    } else {
        let content = std::fs::read_to_string(log_path).unwrap_or_default();
        let all_lines: Vec<&str> = content.lines().collect();
        let start = all_lines.len().saturating_sub(lines);
        for line in &all_lines[start..] {
            println!("{line}");
        }
    }
}

/// The daemon log most recently written next to `daemon_log`: `daemon_log` itself (background mode) or a dated `daemon-YYYY-MM-DD.log` (written by `start --foreground`, the usual mode under Docker and systemd).
///
/// Picks by modification time, so whichever mode the daemon last ran in wins; `None` when neither exists.
pub(crate) fn most_recent_daemon_log(daemon_log: &std::path::Path) -> Option<PathBuf> {
    let modified = |p: &std::path::Path| std::fs::metadata(p).and_then(|m| m.modified()).ok();
    let mut candidates: Vec<PathBuf> = Vec::new();
    if daemon_log.is_file() {
        candidates.push(daemon_log.to_path_buf());
    }
    if let Some(Ok(entries)) = daemon_log.parent().map(std::fs::read_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            let is_dated = path
                .file_name()
                .and_then(|n| n.to_str())
                .and_then(|n| n.strip_prefix("daemon-"))
                .and_then(|n| n.strip_suffix(".log"))
                .and_then(parse_daily_date_timestamp)
                .is_some();
            if is_dated && path.is_file() {
                candidates.push(path);
            }
        }
    }
    // Ties (or unreadable mtimes) fall back to the name, where the newest date sorts last.
    candidates
        .into_iter()
        .max_by(|a, b| modified(a).cmp(&modified(b)).then_with(|| a.cmp(b)))
}

pub(crate) fn cmd_logs(config: Option<PathBuf>, lines: usize, follow: bool) {
    let daemon = daemon_config_context(config.as_deref());
    let daemon_log = daemon_log_path_for_config(config.as_deref());
    if let Some(log) = most_recent_daemon_log(&daemon_log) {
        show_log_file(&log, lines, follow);
        return;
    }

    let tui_log = match daemon.log_dir.as_deref() {
        Some(dir) => dir.join("tui.log"),
        None => daemon.home_dir.join("logs").join("tui.log"),
    };
    if tui_log.exists() {
        ui::hint(&i18n::t_args(
            "daemon-log-not-found-showing-tui",
            &[("path", &tui_log.display().to_string())],
        ));
        show_log_file(&tui_log, lines, follow);
        return;
    }

    show_log_file(&daemon_log, lines, follow);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `stop` used to report success as soon as the health probe failed, while the daemon was still cleaning up under `daemon.lock`; it now waits on the process itself.
    #[test]
    fn process_alive_tracks_a_real_process() {
        assert!(process_alive(std::process::id()));
        let mut child = std::process::Command::new(if cfg!(windows) { "cmd" } else { "true" })
            .args(if cfg!(windows) {
                &["/C", "exit"][..]
            } else {
                &[][..]
            })
            .spawn()
            .expect("spawn child");
        let pid = child.id();
        child.wait().expect("reap child");
        assert!(!process_alive(pid), "a reaped child must not read as alive");
    }

    #[test]
    fn wait_until_polls_until_done_or_budget() {
        let mut calls = 0;
        assert!(wait_until(
            std::time::Duration::from_secs(5),
            std::time::Duration::from_millis(1),
            || {
                calls += 1;
                calls == 3
            }
        ));
        assert_eq!(calls, 3);
        assert!(!wait_until(
            std::time::Duration::from_millis(20),
            std::time::Duration::from_millis(5),
            || false
        ));
    }

    /// The budget must outlast the daemon's own 30s hard shutdown deadline, or `stop` SIGKILLs a daemon that is still cleaning up.
    #[test]
    fn stop_budget_outlasts_daemon_hard_deadline() {
        assert!(STOP_EXIT_BUDGET > std::time::Duration::from_secs(30));
    }

    fn write_with_mtime(path: &std::path::Path, secs_ago: u64) {
        std::fs::write(path, "x").unwrap();
        let t = SystemTime::now() - std::time::Duration::from_secs(secs_ago);
        std::fs::File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_modified(t)
            .unwrap();
    }

    /// `start --foreground` writes only `daemon-YYYY-MM-DD.log`, which `librefang logs` used to never look at.
    #[test]
    fn most_recent_daemon_log_finds_foreground_dated_log() {
        let dir = tempfile::TempDir::new().unwrap();
        let daemon_log = dir.path().join("daemon.log");
        assert_eq!(most_recent_daemon_log(&daemon_log), None);

        write_with_mtime(&dir.path().join("daemon-2026-09-25.log"), 200);
        write_with_mtime(&dir.path().join("daemon-2026-09-26.log"), 100);
        write_with_mtime(&dir.path().join("daemon-notadate.log"), 0);
        write_with_mtime(&dir.path().join("tui.log"), 0);
        assert_eq!(
            most_recent_daemon_log(&daemon_log),
            Some(dir.path().join("daemon-2026-09-26.log"))
        );

        // A background daemon writing `daemon.log` more recently wins over an older foreground run.
        write_with_mtime(&daemon_log, 10);
        assert_eq!(
            most_recent_daemon_log(&daemon_log),
            Some(daemon_log.clone())
        );

        // And a newer foreground run wins over a stale `daemon.log`.
        write_with_mtime(&daemon_log, 1000);
        assert_eq!(
            most_recent_daemon_log(&daemon_log),
            Some(dir.path().join("daemon-2026-09-26.log"))
        );
    }
}
