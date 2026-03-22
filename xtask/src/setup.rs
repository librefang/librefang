use clap::Parser;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Parser, Debug)]
pub struct SetupArgs {
    /// Skip pnpm install for frontend targets
    #[arg(long)]
    pub no_web: bool,

    /// Skip cargo fetch
    #[arg(long)]
    pub no_fetch: bool,
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

fn check_tool(name: &str, install_hint: &str) -> bool {
    let ok = Command::new(name)
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if ok {
        let version = Command::new(name)
            .arg("--version")
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .unwrap_or_default();
        println!("  OK  {} ({})", name, version);
    } else {
        println!("  MISSING  {} — {}", name, install_hint);
    }
    ok
}

fn install_git_hooks(root: &Path) {
    let hooks_src = root.join("scripts/hooks");
    let hooks_dst = root.join(".git/hooks");

    if !hooks_src.exists() {
        println!("  Skipping git hooks (scripts/hooks/ not found)");
        return;
    }

    if let Ok(entries) = fs::read_dir(&hooks_src) {
        for entry in entries.flatten() {
            let src = entry.path();
            if src.is_file() {
                let name = entry.file_name();
                let dst = hooks_dst.join(&name);
                match fs::copy(&src, &dst) {
                    Ok(_) => {
                        // Make executable on Unix
                        #[cfg(unix)]
                        {
                            use std::os::unix::fs::PermissionsExt;
                            let _ = fs::set_permissions(&dst, fs::Permissions::from_mode(0o755));
                        }
                        println!("  Installed hook: {}", name.to_string_lossy());
                    }
                    Err(e) => println!(
                        "  Warning: failed to install {}: {}",
                        name.to_string_lossy(),
                        e
                    ),
                }
            }
        }
    }
}

fn pnpm_install(dir: &PathBuf, label: &str) {
    if !dir.join("package.json").exists() {
        println!("  Skipping {} (no package.json)", label);
        return;
    }

    println!("  Installing {} dependencies...", label);
    let status = Command::new("pnpm")
        .args(["install"])
        .current_dir(dir)
        .status();
    match status {
        Ok(s) if s.success() => println!("  OK  {}", label),
        _ => println!("  WARN  {} install failed", label),
    }
}

pub fn run(args: SetupArgs) -> Result<(), Box<dyn std::error::Error>> {
    let root = repo_root();
    let mut all_ok = true;

    // Step 1: Check required tools
    println!("=== Checking tools ===");
    let has_cargo = check_tool("cargo", "install from https://rustup.rs");
    let has_rustup = check_tool("rustup", "install from https://rustup.rs");
    let has_pnpm = check_tool("pnpm", "install with: npm i -g pnpm");
    let has_gh = check_tool("gh", "install from https://cli.github.com");
    check_tool("docker", "install from https://docs.docker.com/get-docker/");
    check_tool("just", "install with: cargo install just");
    println!();

    if !has_cargo {
        return Err("cargo is required — install Rust from https://rustup.rs".into());
    }

    // Step 2: Check Rust edition
    println!("=== Rust toolchain ===");
    if has_rustup {
        let output = Command::new("rustup")
            .args(["show", "active-toolchain"])
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .unwrap_or_default();
        println!("  Active toolchain: {}", output);
    }
    println!();

    // Step 3: Install git hooks
    println!("=== Git hooks ===");
    install_git_hooks(&root);
    println!();

    // Step 4: Fetch Rust dependencies
    if !args.no_fetch {
        println!("=== Cargo fetch ===");
        let status = Command::new("cargo")
            .args(["fetch"])
            .current_dir(&root)
            .status()?;
        if status.success() {
            println!("  Dependencies fetched");
        } else {
            println!("  Warning: cargo fetch failed");
            all_ok = false;
        }
        println!();
    }

    // Step 5: Install frontend dependencies
    if !args.no_web && has_pnpm {
        println!("=== Frontend dependencies ===");
        pnpm_install(&root.join("web"), "web");
        pnpm_install(&root.join("crates/librefang-api/dashboard"), "dashboard");
        pnpm_install(&root.join("docs"), "docs");
        println!();
    }

    // Step 6: Create default config directory
    println!("=== Config ===");
    let config_dir = dirs_or_home();
    if let Some(dir) = config_dir {
        let librefang_dir = dir.join(".librefang");
        if !librefang_dir.exists() {
            fs::create_dir_all(&librefang_dir)?;
            println!("  Created {}", librefang_dir.display());
        } else {
            println!("  Config dir exists: {}", librefang_dir.display());
        }
        let config_file = librefang_dir.join("config.toml");
        if !config_file.exists() {
            fs::write(
                &config_file,
                "# LibreFang configuration\n# See docs for available options\n",
            )?;
            println!("  Created default config.toml");
        } else {
            println!("  config.toml exists");
        }
    }
    println!();

    // Summary
    println!("=== Setup complete ===");
    if !has_gh {
        println!("  Note: gh CLI not found — needed for release/changelog commands");
    }
    if !has_pnpm {
        println!("  Note: pnpm not found — needed for frontend builds");
    }
    if all_ok {
        println!("  Ready to build: cargo build --workspace --lib");
    }

    Ok(())
}

fn dirs_or_home() -> Option<PathBuf> {
    std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .ok()
        .map(PathBuf::from)
}
