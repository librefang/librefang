use clap::Parser;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

#[derive(Parser, Debug)]
pub struct DepsArgs {
    /// Run cargo audit for security vulnerabilities
    #[arg(long)]
    pub audit: bool,

    /// Run cargo outdated to check for updates
    #[arg(long)]
    pub outdated: bool,

    /// Include frontend (pnpm audit)
    #[arg(long)]
    pub web: bool,
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
            panic!("could not find workspace root (no Cargo.toml with [workspace])");
        }
    }
}

fn has_command(cmd: &str) -> bool {
    Command::new(cmd)
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn has_cargo_subcommand(sub: &str) -> bool {
    Command::new("cargo")
        .args([sub, "--version"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn run_cargo_audit(root: &PathBuf) -> Result<bool, Box<dyn std::error::Error>> {
    if !has_cargo_subcommand("audit") {
        println!("  Installing cargo-audit...");
        let status = Command::new("cargo")
            .args(["install", "cargo-audit"])
            .status()?;
        if !status.success() {
            return Err("failed to install cargo-audit".into());
        }
    }

    println!("=== cargo audit ===");
    let status = Command::new("cargo")
        .args(["audit"])
        .current_dir(root)
        .status()?;
    println!();

    Ok(status.success())
}

fn run_cargo_outdated(root: &PathBuf) -> Result<bool, Box<dyn std::error::Error>> {
    if !has_cargo_subcommand("outdated") {
        println!("  Installing cargo-outdated...");
        let status = Command::new("cargo")
            .args(["install", "cargo-outdated"])
            .status()?;
        if !status.success() {
            return Err("failed to install cargo-outdated".into());
        }
    }

    println!("=== cargo outdated ===");
    let status = Command::new("cargo")
        .args(["outdated", "--workspace", "--root-deps-only"])
        .current_dir(root)
        .status()?;
    println!();

    Ok(status.success())
}

fn run_pnpm_audit(dir: &PathBuf, label: &str) -> bool {
    if !dir.join("package.json").exists() {
        return true;
    }

    println!("--- pnpm audit: {} ---", label);
    let status = Command::new("pnpm")
        .args(["audit"])
        .current_dir(dir)
        .status();
    println!();

    status.map(|s| s.success()).unwrap_or(false)
}

pub fn run(args: DepsArgs) -> Result<(), Box<dyn std::error::Error>> {
    let root = repo_root();

    // If no flags set, run all
    let run_all = !args.audit && !args.outdated && !args.web;
    let mut issues = 0;

    if run_all || args.audit {
        match run_cargo_audit(&root) {
            Ok(true) => println!("Cargo audit: no vulnerabilities found"),
            Ok(false) => {
                println!("Cargo audit: vulnerabilities found!");
                issues += 1;
            }
            Err(e) => {
                println!("Cargo audit error: {}", e);
                issues += 1;
            }
        }
        println!();
    }

    if run_all || args.outdated {
        match run_cargo_outdated(&root) {
            Ok(_) => {}
            Err(e) => {
                println!("Cargo outdated error: {}", e);
                issues += 1;
            }
        }
    }

    if run_all || args.web {
        if has_command("pnpm") {
            println!("=== pnpm audit ===");
            let web_dirs = [
                (root.join("web"), "web"),
                (root.join("crates/librefang-api/dashboard"), "dashboard"),
                (root.join("docs"), "docs"),
            ];
            for (dir, label) in &web_dirs {
                if !run_pnpm_audit(dir, label) {
                    issues += 1;
                }
            }
        } else {
            println!("pnpm not found — skipping frontend audit");
        }
    }

    println!("=== Summary ===");
    if issues > 0 {
        println!("{} issue(s) found — review output above", issues);
        Err(format!("{} dependency issue(s) found", issues).into())
    } else {
        println!("All clean.");
        Ok(())
    }
}
