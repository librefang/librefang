use clap::Parser;
use std::fs;
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

fn repo_root() -> PathBuf {
    let mut dir = std::env::current_dir().expect("cannot get cwd");
    loop {
        let cargo_toml = dir.join("Cargo.toml");
        if cargo_toml.exists() {
            let content = fs::read_to_string(&cargo_toml).unwrap_or_default();
            if content.contains("[workspace]") {
                return dir;
            }
        }
        if !dir.pop() {
            panic!("could not find workspace root");
        }
    }
}

pub fn run(args: DevArgs) -> Result<(), Box<dyn std::error::Error>> {
    let root = repo_root();

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

    // Start dashboard dev server in background (if dashboard exists)
    let dashboard_dir = root.join("crates/librefang-api/dashboard");
    let mut _dashboard_child = None;
    if !args.no_dashboard && dashboard_dir.join("package.json").exists() {
        println!("Starting dashboard dev server...");
        let child = Command::new("pnpm")
            .arg("dev")
            .current_dir(&dashboard_dir)
            .spawn();
        match child {
            Ok(c) => _dashboard_child = Some(c),
            Err(e) => eprintln!("Warning: could not start dashboard dev server: {}", e),
        }
    }

    // Start daemon
    println!("Starting daemon on port {}...", args.port);
    println!("  Binary: {}", binary.display());
    println!("  Press Ctrl+C to stop\n");

    let status = Command::new(&binary)
        .args(["start", "--foreground"])
        .env("LIBREFANG_PORT", args.port.to_string())
        .current_dir(&root)
        .status()?;

    // Cleanup dashboard if it was started
    if let Some(mut child) = _dashboard_child {
        let _ = child.kill();
    }

    if !status.success() {
        return Err("Daemon exited with error".into());
    }

    Ok(())
}
