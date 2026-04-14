use crate::common::repo_root;
use clap::Parser;
use std::path::PathBuf;
use std::process::Command;

#[derive(Parser, Debug)]
pub struct DevArgs {
    /// Skip starting the dashboard dev server
    #[arg(long)]
    pub no_dashboard: bool,

    /// Custom port for the daemon
    #[arg(long, default_value = "4545")]
    pub port: u16,

    /// Build in release mode
    #[arg(long)]
    pub release: bool,
}

pub fn run(args: DevArgs) -> Result<(), Box<dyn std::error::Error>> {
    let root = repo_root();

    // Kill stale processes on relevant ports
    kill_stale_processes();

    // Build the daemon binary
    println!("Building librefang-cli...");
    let mut build_cmd = Command::new("cargo");
    build_cmd
        .args(["build", "-p", "librefang-cli"])
        .current_dir(&root);
    if args.release {
        build_cmd.arg("--release");
    }
    let status = build_cmd.status()?;
    if !status.success() {
        return Err("Failed to build librefang-cli".into());
    }

    let profile = if args.release { "release" } else { "debug" };
    let binary = root.join("target").join(profile).join("librefang");

    if !binary.exists() {
        return Err(format!("Binary not found: {}", binary.display()).into());
    }

    // Auto-init if config.toml does not exist
    let config_dir = librefang_home();
    let config_path = config_dir.join("config.toml");
    if !config_path.exists() {
        println!("No config.toml found — running `librefang init --quick`...");
        let init_status = Command::new(&binary).args(["init", "--quick"]).status()?;
        if !init_status.success() {
            eprintln!("Warning: init --quick failed, continuing with defaults");
        }
    }

    // Copy config.example.toml to the config directory if it doesn't exist
    let example_dest = config_dir.join("config.example.toml");
    if !example_dest.exists() {
        let example_src = root.join("crates/librefang-cli/templates/init_default_config.toml");
        if example_src.exists() {
            if let Err(e) = std::fs::copy(&example_src, &example_dest) {
                eprintln!("Warning: could not copy config.example.toml: {e}");
            } else {
                println!("Copied config.example.toml to {}", example_dest.display());
            }
        }
    }

    // Start dashboard dev server in background (if dashboard exists)
    let dashboard_dir = root.join("crates/librefang-api/dashboard");
    let mut _dashboard_child = None;
    if !args.no_dashboard && dashboard_dir.join("package.json").exists() {
        println!("Installing dashboard dependencies...");
        let _ = Command::new("sh")
            .args(["-c", "pnpm install"])
            .current_dir(&dashboard_dir)
            .status();

        println!("Starting dashboard dev server...");
        let child = Command::new("sh")
            .args(["-c", "pnpm dev"])
            .current_dir(&dashboard_dir)
            .spawn();
        match child {
            Ok(c) => _dashboard_child = Some(c),
            Err(e) => eprintln!("Warning: could not start dashboard dev server: {}", e),
        }

        // Open browser once dashboard is ready
        std::thread::spawn(|| {
            let dashboard_url = detect_dashboard_url();
            for _ in 0..60 {
                std::thread::sleep(std::time::Duration::from_secs(2));
                if reqwest_probe("http://127.0.0.1:5173/dashboard/") {
                    let _ = Command::new("open").arg(&dashboard_url).status();
                    return;
                }
            }
            eprintln!("Warning: dashboard did not become ready in time");
        });
    }

    run_watch(&args, &root, &binary, _dashboard_child)
}

/// Kill stale processes on API and dashboard ports.
fn kill_stale_processes() {
    // Remove launchctl service if registered
    let _ = Command::new("launchctl")
        .args(["remove", "ai.librefang.daemon"])
        .output();

    // Kill any lingering daemon processes by name (handles the case where Ctrl+C
    // kills xtask before the cleanup code can run, leaving the daemon orphaned)
    let _ = Command::new("pkill")
        .args(["-9", "-f", "librefang.*start"])
        .output();

    // Kill listeners on API port and dashboard dev server ports
    for port in [4545, 5173, 5174, 5175, 5176, 5177, 5178] {
        let output = Command::new("lsof")
            .args(["-ti", &format!(":{port}"), "-sTCP:LISTEN"])
            .output();
        if let Ok(out) = output {
            let pids = String::from_utf8_lossy(&out.stdout);
            for pid in pids.split_whitespace() {
                let _ = Command::new("kill").args(["-9", pid]).output();
            }
        }
    }

    // Remove stale daemon info file so the new daemon doesn't think
    // the old one is still alive (race between kill and PID check).
    let daemon_json = librefang_home().join("daemon.json");
    if daemon_json.exists() {
        let _ = std::fs::remove_file(&daemon_json);
    }

    std::thread::sleep(std::time::Duration::from_secs(1));
}

/// Detect the LAN IP and build the dashboard URL.
fn detect_dashboard_url() -> String {
    // macOS: ipconfig getifaddr en0
    if let Ok(out) = Command::new("ipconfig").args(["getifaddr", "en0"]).output() {
        let ip = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if !ip.is_empty() {
            return format!("http://{ip}:5173/dashboard/");
        }
    }
    // Linux: hostname -I
    if let Ok(out) = Command::new("hostname").arg("-I").output() {
        if let Some(ip) = String::from_utf8_lossy(&out.stdout)
            .split_whitespace()
            .next()
        {
            return format!("http://{ip}:5173/dashboard/");
        }
    }
    "http://127.0.0.1:5173/dashboard/".to_string()
}

/// Probe a URL to check if it's reachable (simple TCP-level check via curl).
fn reqwest_probe(url: &str) -> bool {
    Command::new("curl")
        .args(["-s", "-o", "/dev/null", "-w", "%{http_code}", url])
        .output()
        .map(|o| !o.stdout.is_empty() && o.stdout != b"000")
        .unwrap_or(false)
}

/// Watch crates/ for changes, rebuild librefang-cli, then kill + restart the daemon.
fn run_watch(
    args: &DevArgs,
    root: &std::path::Path,
    binary: &std::path::Path,
    dashboard_child: Option<std::process::Child>,
) -> Result<(), Box<dyn std::error::Error>> {
    // Auto-install cargo-watch if missing
    let has_watch = Command::new("cargo")
        .args(["watch", "--version"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    if !has_watch {
        println!("cargo-watch not found, installing...");
        let status = Command::new("cargo")
            .args(["install", "cargo-watch"])
            .status()?;
        if !status.success() {
            return Err("Failed to install cargo-watch".into());
        }
    }

    let binary_str = binary.display().to_string();
    let port = args.port;

    println!("Starting daemon on port {port} (watch mode)...");
    println!("  Binary: {binary_str}");
    println!("  Watching: crates/");
    println!("  Hotkeys: r=pull  o=open  l=logs  s=status  c=clear  ?=help\n");

    // Stop any running daemon via the CLI (reads daemon.json, sends SIGTERM,
    // waits for exit) — far more reliable than lsof + kill -9.
    let _ = Command::new(binary).arg("stop").status();
    // Belt-and-suspenders: also kill by port in case `stop` missed something.
    let home_dir = librefang_home().display().to_string();
    let stop_script = format!(
        "{binary} stop 2>/dev/null; \
         for pid in $(lsof -ti :{port} -sTCP:LISTEN 2>/dev/null); do kill -9 $pid 2>/dev/null; done; \
         rm -f {home}/daemon.json; \
         for _i in 1 2 3 4 5 6 7 8 9 10; do \
           lsof -ti :{port} -sTCP:LISTEN >/dev/null 2>&1 || break; \
           sleep 0.3; \
         done",
        binary = binary_str,
        port = port,
        home = home_dir,
    );

    // Start daemon immediately (no build needed — already built above).
    let _ = Command::new("sh")
        .args([
            "-c",
            &format!(
                "({stop} && LIBREFANG_PORT={port} {binary} start --foreground &)",
                stop = stop_script,
                port = port,
                binary = binary_str,
            ),
        ])
        .current_dir(root)
        .status();

    // On every crate change: rebuild, then stop+restart.
    let rebuild_and_restart = format!(
        "(cargo build -p librefang-cli && {stop} && LIBREFANG_PORT={port} {binary} start --foreground &)",
        stop = stop_script,
        port = port,
        binary = binary_str,
    );

    // Background thread: auto-pull origin/main every 30 seconds.
    {
        let root_auto = root.to_path_buf();
        std::thread::spawn(move || loop {
            std::thread::sleep(std::time::Duration::from_secs(30));
            let fetch = Command::new("git")
                .args(["fetch", "origin", "main"])
                .current_dir(&root_auto)
                .stderr(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .status();
            if !matches!(fetch, Ok(s) if s.success()) {
                continue;
            }
            // Only rebase if there are new commits
            let behind = Command::new("git")
                .args(["rev-list", "--count", "HEAD..origin/main"])
                .current_dir(&root_auto)
                .output();
            let count: u64 = behind
                .ok()
                .and_then(|o| String::from_utf8_lossy(&o.stdout).trim().parse().ok())
                .unwrap_or(0);
            if count > 0 {
                println!("\n\x1b[36m↻ auto-pull: {count} new commit(s), rebasing...\x1b[0m");
                let status = Command::new("git")
                    .args(["rebase", "origin/main"])
                    .current_dir(&root_auto)
                    .status();
                match status {
                    Ok(s) if s.success() => {
                        println!("\x1b[32m✓ auto-pull done — cargo-watch will rebuild\x1b[0m")
                    }
                    _ => eprintln!("\x1b[31m✗ auto-pull rebase failed\x1b[0m"),
                }
            }
        });
    }

    // Background thread: hotkey listener for dev workflow shortcuts.
    let root_clone = root.to_path_buf();
    let hotkey_port = port;
    let hotkey_binary = binary_str.clone();
    std::thread::spawn(move || {
        use std::io::Read;
        // Set terminal to raw mode so we get keypresses without Enter
        let _ = Command::new("stty").args(["-icanon", "min", "1"]).status();
        let stdin = std::io::stdin();
        let mut buf = [0u8; 1];
        loop {
            if stdin.lock().read_exact(&mut buf).is_err() {
                break;
            }
            match buf[0] {
                b'r' => {
                    println!("\n\x1b[36m↻ git fetch + rebase...\x1b[0m");
                    let _ = Command::new("git")
                        .args(["fetch", "origin", "main"])
                        .current_dir(&root_clone)
                        .status();
                    let status = Command::new("git")
                        .args(["rebase", "origin/main"])
                        .current_dir(&root_clone)
                        .status();
                    match status {
                        Ok(s) if s.success() => {
                            println!("\x1b[32m✓ rebase done — cargo-watch will rebuild\x1b[0m")
                        }
                        Ok(s) => eprintln!(
                            "\x1b[31m✗ rebase failed (exit {})\x1b[0m",
                            s.code().unwrap_or(-1)
                        ),
                        Err(e) => eprintln!("\x1b[31m✗ rebase error: {e}\x1b[0m"),
                    }
                }
                b'o' => {
                    println!("\n\x1b[36m↻ opening dashboard...\x1b[0m");
                    let url = format!("http://127.0.0.1:{hotkey_port}");
                    let _ = Command::new("open").arg(&url).status();
                }
                b'l' => {
                    println!("\n\x1b[36m── recent logs ──\x1b[0m");
                    let log_dir = librefang_home().join("logs");
                    let latest = Command::new("ls")
                        .args(["-t"])
                        .current_dir(&log_dir)
                        .output()
                        .ok()
                        .and_then(|o| {
                            String::from_utf8_lossy(&o.stdout)
                                .lines()
                                .next()
                                .map(String::from)
                        });
                    if let Some(file) = latest {
                        let _ = Command::new("tail")
                            .args(["-30", &file])
                            .current_dir(&log_dir)
                            .status();
                    } else {
                        // Fallback: try daemon stdout via the binary
                        let _ = Command::new(&hotkey_binary)
                            .args(["logs", "--lines", "30"])
                            .status();
                    }
                    println!("\x1b[36m── end logs ──\x1b[0m");
                }
                b's' => {
                    println!("\n\x1b[36m── status ──\x1b[0m");
                    // Git branch
                    if let Ok(out) = Command::new("git")
                        .args(["branch", "--show-current"])
                        .current_dir(&root_clone)
                        .output()
                    {
                        let branch = String::from_utf8_lossy(&out.stdout);
                        println!("  branch: {}", branch.trim());
                    }
                    // Git short status
                    if let Ok(out) = Command::new("git")
                        .args(["status", "--short"])
                        .current_dir(&root_clone)
                        .output()
                    {
                        let changes = String::from_utf8_lossy(&out.stdout);
                        let count = changes.lines().count();
                        if count > 0 {
                            println!("  changes: {count} file(s)");
                        } else {
                            println!("  changes: clean");
                        }
                    }
                    // Port / process check
                    if let Ok(out) = Command::new("lsof")
                        .args(["-ti", &format!(":{hotkey_port}"), "-sTCP:LISTEN"])
                        .output()
                    {
                        let pids = String::from_utf8_lossy(&out.stdout);
                        let pid_list: Vec<&str> = pids.split_whitespace().collect();
                        if pid_list.is_empty() {
                            println!("  daemon: \x1b[31mnot running\x1b[0m");
                        } else {
                            println!(
                                "  daemon: \x1b[32mrunning\x1b[0m (pid {})",
                                pid_list.join(", ")
                            );
                        }
                    }
                    println!("\x1b[36m── end ──\x1b[0m");
                }
                b'c' => {
                    // Clear screen (ANSI escape)
                    print!("\x1b[2J\x1b[H");
                    let _ = std::io::Write::flush(&mut std::io::stdout());
                }
                b'?' | b'h' => {
                    println!("\n\x1b[36m  Hotkeys:\x1b[0m");
                    println!("    r  git fetch + rebase origin/main");
                    println!("    o  open dashboard in browser");
                    println!("    l  show recent daemon logs");
                    println!("    s  show status (branch, changes, daemon)");
                    println!("    c  clear screen");
                    println!("    ?  show this help");
                }
                _ => {}
            }
        }
        // Restore terminal on exit
        let _ = Command::new("stty").arg("sane").status();
    });

    // Watch the Rust workspace only. The dashboard lives under
    // `crates/librefang-api/dashboard/` but has its own vite HMR via
    // `pnpm dev`, so changes there must NOT trigger a Rust rebuild +
    // daemon restart. Ignore the dashboard directory and any editor
    // scratch files that could otherwise bounce cargo-watch in a loop.
    let cargo_watch_status = Command::new("cargo")
        .args([
            "watch",
            "--watch",
            "crates",
            "--ignore",
            "crates/librefang-api/dashboard/**",
            "--ignore",
            "**/node_modules/**",
            "--ignore",
            "**/target/**",
            "--ignore",
            "**/*.md",
            "-s",
            &rebuild_and_restart,
        ])
        .current_dir(root)
        .status()?;

    // Restore terminal mode
    let _ = Command::new("stty").arg("sane").status();

    // Cleanup dashboard on exit
    if let Some(mut child) = dashboard_child {
        let _ = child.kill();
    }
    // Kill daemon on exit
    for pid in get_pids_on_port(port) {
        let _ = Command::new("kill").args(["-9", &pid]).output();
    }

    if !cargo_watch_status.success() {
        return Err("cargo-watch exited with error".into());
    }
    Ok(())
}

/// Return PIDs listening on the given port.
fn get_pids_on_port(port: u16) -> Vec<String> {
    Command::new("lsof")
        .args(["-ti", &format!(":{port}"), "-sTCP:LISTEN"])
        .output()
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .split_whitespace()
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

/// Resolve the LibreFang home directory (mirrors kernel logic).
fn librefang_home() -> PathBuf {
    if let Ok(home) = std::env::var("LIBREFANG_HOME") {
        return PathBuf::from(home);
    }
    let home = std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir());
    home.join(".librefang")
}
