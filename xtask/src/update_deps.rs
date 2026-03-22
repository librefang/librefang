use crate::common::repo_root;
use clap::Parser;
use std::path::Path;
use std::process::Command;

#[derive(Parser, Debug)]
pub struct UpdateDepsArgs {
    /// Update only Rust dependencies
    #[arg(long)]
    pub rust: bool,

    /// Update only web dependencies
    #[arg(long)]
    pub web: bool,

    /// Dry run — show what would be updated
    #[arg(long)]
    pub dry_run: bool,

    /// Run tests after updating
    #[arg(long)]
    pub test: bool,
}

fn update_rust(root: &Path, dry_run: bool) -> Result<(), Box<dyn std::error::Error>> {
    println!("=== Rust Dependencies ===");

    if dry_run {
        println!("  Checking for outdated packages...");
        let status = Command::new("cargo")
            .args(["outdated", "--workspace"])
            .current_dir(root)
            .status();
        match status {
            Ok(s) if s.success() => {}
            _ => {
                // Fallback: just show what cargo update would do
                println!("  (cargo-outdated not available, showing cargo update --dry-run)");
                let _ = Command::new("cargo")
                    .args(["update", "--dry-run"])
                    .current_dir(root)
                    .status()?;
            }
        }
    } else {
        println!("  Running cargo update...");
        let status = Command::new("cargo")
            .args(["update"])
            .current_dir(root)
            .status()?;
        if !status.success() {
            return Err("cargo update failed".into());
        }
        println!("  Cargo.lock updated.");
    }
    println!();
    Ok(())
}

fn update_web(root: &Path, dry_run: bool) -> Result<(), Box<dyn std::error::Error>> {
    let web_dirs = [
        ("web", root.join("web")),
        ("dashboard", root.join("crates/librefang-api/dashboard")),
        ("docs", root.join("docs")),
    ];

    println!("=== Web Dependencies ===");
    for (name, dir) in &web_dirs {
        if !dir.join("package.json").exists() {
            continue;
        }
        println!("  [{name}]");
        if dry_run {
            let _ = Command::new("pnpm")
                .args(["outdated"])
                .current_dir(dir)
                .status();
        } else {
            let status = Command::new("pnpm")
                .args(["update"])
                .current_dir(dir)
                .status()?;
            if !status.success() {
                eprintln!("  Warning: pnpm update failed for {name}");
            } else {
                println!("  Updated.");
            }
        }
    }
    println!();
    Ok(())
}

pub fn run(args: UpdateDepsArgs) -> Result<(), Box<dyn std::error::Error>> {
    let root = repo_root();
    let update_all = !args.rust && !args.web;

    if args.dry_run {
        println!("Dry run — showing outdated dependencies\n");
    }

    if update_all || args.rust {
        update_rust(&root, args.dry_run)?;
    }

    if update_all || args.web {
        update_web(&root, args.dry_run)?;
    }

    if args.test && !args.dry_run {
        println!("=== Running Tests ===");
        let status = Command::new("cargo")
            .args(["test", "--workspace"])
            .current_dir(&root)
            .status()?;
        if !status.success() {
            return Err("Tests failed after update — consider reverting Cargo.lock".into());
        }
        println!("  All tests passed.\n");
    }

    if args.dry_run {
        println!("Dry run complete. Run without --dry-run to apply updates.");
    } else {
        println!("Dependency update complete.");
    }
    Ok(())
}
