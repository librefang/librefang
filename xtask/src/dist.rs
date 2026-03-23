use crate::common::repo_root;
use clap::Parser;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

#[derive(Parser, Debug)]
pub struct DistArgs {
    /// Build for specific target (e.g. x86_64-unknown-linux-gnu)
    #[arg(long)]
    pub target: Option<String>,

    /// Use cross instead of cargo for cross-compilation
    #[arg(long)]
    pub cross: bool,

    /// Output directory for archives (default: dist/)
    #[arg(long, default_value = "dist")]
    pub output: String,
}

const DEFAULT_TARGETS: &[&str] = &[
    "x86_64-unknown-linux-gnu",
    "aarch64-unknown-linux-gnu",
    "x86_64-apple-darwin",
    "aarch64-apple-darwin",
    "x86_64-pc-windows-msvc",
];

fn read_workspace_version(root: &Path) -> String {
    let content = fs::read_to_string(root.join("Cargo.toml")).unwrap_or_default();
    let doc = content.parse::<toml_edit::DocumentMut>().ok();
    doc.and_then(|d| {
        d["workspace"]["package"]["version"]
            .as_str()
            .map(|s| s.to_string())
    })
    .unwrap_or_else(|| "unknown".to_string())
}

fn binary_name(target: &str) -> &str {
    if target.contains("windows") {
        "librefang.exe"
    } else {
        "librefang"
    }
}

fn archive_ext(target: &str) -> &str {
    if target.contains("windows") {
        "zip"
    } else {
        "tar.gz"
    }
}

fn create_archive(
    root: &Path,
    output_dir: &Path,
    target: &str,
    version: &str,
) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let bin = binary_name(target);
    let binary_path = root.join("target").join(target).join("release").join(bin);

    if !binary_path.exists() {
        return Err(format!("binary not found: {}", binary_path.display()).into());
    }

    let archive_name = format!("librefang-v{}-{}.{}", version, target, archive_ext(target));
    let archive_path = output_dir.join(&archive_name);

    if target.contains("windows") {
        // Create zip
        let status = Command::new("zip")
            .args([
                "-j",
                &archive_path.to_string_lossy(),
                &binary_path.to_string_lossy(),
            ])
            .current_dir(root)
            .status()?;
        if !status.success() {
            return Err(format!("zip failed for {}", target).into());
        }
    } else {
        // Create tar.gz
        let status = Command::new("tar")
            .args([
                "czf",
                &archive_path.to_string_lossy(),
                "-C",
                &binary_path.parent().unwrap().to_string_lossy(),
                bin,
            ])
            .current_dir(root)
            .status()?;
        if !status.success() {
            return Err(format!("tar failed for {}", target).into());
        }
    }

    Ok(archive_path)
}

pub fn run(args: DistArgs) -> Result<(), Box<dyn std::error::Error>> {
    let root = repo_root();
    let version = read_workspace_version(&root);
    let output_dir = root.join(&args.output);
    fs::create_dir_all(&output_dir)?;

    let targets: Vec<&str> = if let Some(ref t) = args.target {
        vec![t.as_str()]
    } else {
        DEFAULT_TARGETS.to_vec()
    };

    let build_cmd = if args.cross { "cross" } else { "cargo" };

    if args.cross && Command::new("cross").arg("--version").output().is_err() {
        return Err("cross not found — install with: cargo install cross".into());
    }

    println!("Building v{} for {} target(s)...", version, targets.len());
    println!();

    let mut built: Vec<String> = Vec::new();
    let mut failed: Vec<String> = Vec::new();

    for target in &targets {
        println!("=== {} ===", target);
        let start = Instant::now();

        let status = Command::new(build_cmd)
            .args([
                "build",
                "--release",
                "--target",
                target,
                "-p",
                "librefang-cli",
            ])
            .current_dir(&root)
            .status();

        match status {
            Ok(s) if s.success() => {
                let elapsed = start.elapsed();
                println!("  Compiled ({:.1}s)", elapsed.as_secs_f64());

                match create_archive(&root, &output_dir, target, &version) {
                    Ok(path) => {
                        println!("  Archived: {}", path.display());
                        built.push(target.to_string());
                    }
                    Err(e) => {
                        println!("  Archive failed: {}", e);
                        failed.push(target.to_string());
                    }
                }
            }
            Ok(_) => {
                println!("  Build failed");
                failed.push(target.to_string());
            }
            Err(e) => {
                println!("  Error: {}", e);
                failed.push(target.to_string());
            }
        }
        println!();
    }

    println!("=== Summary ===");
    println!("  Built:  {} / {}", built.len(), targets.len());
    if !failed.is_empty() {
        println!("  Failed: {}", failed.join(", "));
    }
    println!("  Output: {}", output_dir.display());

    if !failed.is_empty() {
        Err(format!("{} target(s) failed", failed.len()).into())
    } else {
        Ok(())
    }
}
