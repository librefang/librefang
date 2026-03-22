use crate::common::repo_root;
use clap::Parser;
use std::path::Path;
use std::process::Command;

#[derive(Parser, Debug)]
pub struct LicenseCheckArgs {
    /// Check only Rust dependencies
    #[arg(long)]
    pub rust: bool,

    /// Check only web dependencies
    #[arg(long)]
    pub web: bool,

    /// Denied licenses (comma-separated, e.g. "GPL-3.0,AGPL-3.0")
    #[arg(long, default_value = "AGPL-3.0-only,AGPL-3.0-or-later")]
    pub deny: String,
}

fn check_cargo_deny(root: &Path, denied: &[&str]) -> Result<(), Box<dyn std::error::Error>> {
    // Try cargo-deny first
    let deny_check = Command::new("cargo").args(["deny", "--version"]).output();

    if deny_check.is_ok() && deny_check.unwrap().status.success() {
        println!("Using cargo-deny...");
        let status = Command::new("cargo")
            .args(["deny", "check", "licenses"])
            .current_dir(root)
            .status()?;
        if !status.success() {
            return Err("cargo deny check failed".into());
        }
        return Ok(());
    }

    // Fallback: use cargo metadata
    println!("cargo-deny not found, using cargo metadata fallback...");
    let output = Command::new("cargo")
        .args(["metadata", "--format-version=1", "--no-deps"])
        .current_dir(root)
        .output()?;

    if !output.status.success() {
        return Err("cargo metadata failed".into());
    }

    let metadata: serde_json::Value = serde_json::from_slice(&output.stdout)?;
    let mut violations = Vec::new();
    let mut checked = 0;

    if let Some(packages) = metadata["packages"].as_array() {
        for pkg in packages {
            let name = pkg["name"].as_str().unwrap_or("unknown");
            let license = pkg["license"].as_str().unwrap_or("UNKNOWN");
            checked += 1;

            for &deny in denied {
                if license.contains(deny) {
                    violations.push(format!("  {} ({}) — {}", name, license, deny));
                }
            }
        }
    }

    println!("  Checked {} workspace crates", checked);
    if violations.is_empty() {
        println!("  No license violations found.");
    } else {
        println!("  License violations:");
        for v in &violations {
            println!("{}", v);
        }
        return Err(format!("{} license violation(s) found", violations.len()).into());
    }

    Ok(())
}

fn check_web_licenses(root: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let web_dir = root.join("web");
    if !web_dir.join("package.json").exists() {
        println!("Skipping web license check (no web/package.json)");
        return Ok(());
    }

    // Try pnpm licenses list
    let output = Command::new("pnpm")
        .args(["licenses", "list", "--json"])
        .current_dir(&web_dir)
        .output();

    match output {
        Ok(out) if out.status.success() => {
            let stdout = String::from_utf8_lossy(&out.stdout);
            println!("  Web dependency licenses:");
            // Just report — pnpm licenses list shows the breakdown
            let lines: Vec<&str> = stdout.lines().take(20).collect();
            for line in lines {
                println!("    {}", line);
            }
            if stdout.lines().count() > 20 {
                println!("    ... (truncated)");
            }
        }
        _ => {
            println!("  pnpm licenses not available, skipping web license check");
        }
    }

    Ok(())
}

pub fn run(args: LicenseCheckArgs) -> Result<(), Box<dyn std::error::Error>> {
    let root = repo_root();
    let denied: Vec<&str> = args.deny.split(',').map(|s| s.trim()).collect();
    let check_all = !args.rust && !args.web;

    println!("License check");
    println!("  Denied: {}\n", args.deny);

    if check_all || args.rust {
        println!("=== Rust Dependencies ===");
        check_cargo_deny(&root, &denied)?;
        println!();
    }

    if check_all || args.web {
        println!("=== Web Dependencies ===");
        check_web_licenses(&root)?;
        println!();
    }

    println!("License check complete.");
    Ok(())
}
