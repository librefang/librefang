use crate::common::repo_root;
use crate::local_check_mode;
use clap::Parser;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

#[derive(Parser, Debug)]
pub struct CoverageArgs {
    /// Output lcov format (for CI upload)
    #[arg(long)]
    pub lcov: bool,

    /// Open HTML report in browser
    #[arg(long)]
    pub open: bool,

    /// Output directory (default: target/llvm-cov)
    #[arg(long)]
    pub output: Option<String>,
}

fn has_cargo_llvm_cov() -> bool {
    Command::new("cargo")
        .args(["llvm-cov", "--version"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

pub fn run(args: CoverageArgs) -> Result<(), Box<dyn std::error::Error>> {
    local_check_mode::apply_for_subcommand("coverage");

    let root = repo_root();

    if !has_cargo_llvm_cov() {
        println!("cargo-llvm-cov not found. Installing...");
        let status = Command::new("cargo")
            .args(["install", "cargo-llvm-cov"])
            .status()?;
        if !status.success() {
            return Err("failed to install cargo-llvm-cov".into());
        }
    }

    if args.lcov {
        let output_path = args
            .output
            .unwrap_or_else(|| "target/llvm-cov/lcov.info".to_string());

        // Ensure parent dir exists
        if let Some(parent) = PathBuf::from(&output_path).parent() {
            fs::create_dir_all(parent)?;
        }

        println!("Generating lcov coverage report...");
        let status = Command::new("cargo")
            .args([
                "llvm-cov",
                "--workspace",
                "--lcov",
                "--output-path",
                &output_path,
            ])
            .current_dir(&root)
            .status()?;

        if !status.success() {
            return Err("cargo llvm-cov failed".into());
        }

        println!("Coverage report: {}", output_path);
    } else {
        println!("Generating HTML coverage report...");
        let mut cmd_args = vec!["llvm-cov", "--workspace", "--html"];

        let output_dir = args
            .output
            .unwrap_or_else(|| "target/llvm-cov/html".to_string());
        cmd_args.push("--output-dir");
        cmd_args.push(&output_dir);

        if args.open {
            cmd_args.push("--open");
        }

        let status = Command::new("cargo")
            .args(&cmd_args)
            .current_dir(&root)
            .status()?;

        if !status.success() {
            return Err("cargo llvm-cov failed".into());
        }

        println!("Coverage report: {}/index.html", output_dir);
        if !args.open {
            println!("Run with --open to view in browser");
        }
    }

    Ok(())
}
