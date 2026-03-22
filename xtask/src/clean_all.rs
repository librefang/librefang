use clap::Parser;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Parser, Debug)]
pub struct CleanAllArgs {
    /// Only clean Rust build artifacts (target/)
    #[arg(long)]
    pub rust: bool,

    /// Only clean frontend artifacts (node_modules/, dist/, .next/)
    #[arg(long)]
    pub web: bool,

    /// Show what would be deleted without deleting
    #[arg(long)]
    pub dry_run: bool,
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

fn dir_size(path: &Path) -> u64 {
    if !path.exists() {
        return 0;
    }
    fs::read_dir(path)
        .map(|entries| {
            entries
                .filter_map(|e| e.ok())
                .map(|e| {
                    let path = e.path();
                    if path.is_dir() {
                        dir_size(&path)
                    } else {
                        e.metadata().map(|m| m.len()).unwrap_or(0)
                    }
                })
                .sum()
        })
        .unwrap_or(0)
}

fn format_size(bytes: u64) -> String {
    if bytes >= 1_073_741_824 {
        format!("{:.1} GB", bytes as f64 / 1_073_741_824.0)
    } else if bytes >= 1_048_576 {
        format!("{:.1} MB", bytes as f64 / 1_048_576.0)
    } else if bytes >= 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{} B", bytes)
    }
}

fn remove_dir(path: &Path, dry_run: bool) -> u64 {
    if !path.exists() {
        return 0;
    }
    let size = dir_size(path);
    if dry_run {
        println!(
            "  [dry-run] would remove {} ({})",
            path.display(),
            format_size(size)
        );
    } else {
        match fs::remove_dir_all(path) {
            Ok(_) => println!("  Removed {} ({})", path.display(), format_size(size)),
            Err(e) => println!("  Failed to remove {}: {}", path.display(), e),
        }
    }
    size
}

pub fn run(args: CleanAllArgs) -> Result<(), Box<dyn std::error::Error>> {
    let root = repo_root();
    let clean_all = !args.rust && !args.web;
    let mut total_freed: u64 = 0;

    if args.dry_run {
        println!("=== Dry run ===");
        println!();
    }

    // Rust artifacts
    if clean_all || args.rust {
        println!("=== Rust build artifacts ===");
        total_freed += remove_dir(&root.join("target"), args.dry_run);
        total_freed += remove_dir(&root.join("dist"), args.dry_run);
        println!();
    }

    // Frontend artifacts
    if clean_all || args.web {
        println!("=== Frontend artifacts ===");

        let web_dirs = ["web", "docs", "crates/librefang-api/dashboard"];

        for dir in &web_dirs {
            let base = root.join(dir);
            if !base.exists() {
                continue;
            }
            total_freed += remove_dir(&base.join("node_modules"), args.dry_run);
            total_freed += remove_dir(&base.join("dist"), args.dry_run);
            total_freed += remove_dir(&base.join(".next"), args.dry_run);
            total_freed += remove_dir(&base.join("out"), args.dry_run);
        }
        println!();
    }

    println!(
        "Total {}: {}",
        if args.dry_run { "reclaimable" } else { "freed" },
        format_size(total_freed)
    );
    Ok(())
}
