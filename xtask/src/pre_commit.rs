use clap::Parser;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::time::Instant;

#[derive(Parser, Debug)]
pub struct PreCommitArgs {
    /// Skip formatting check
    #[arg(long)]
    pub no_fmt: bool,

    /// Skip clippy
    #[arg(long)]
    pub no_clippy: bool,

    /// Skip tests
    #[arg(long)]
    pub no_test: bool,

    /// Auto-fix formatting issues
    #[arg(long)]
    pub fix: bool,
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

fn run_step(name: &str, cmd: &mut Command) -> Result<(), Box<dyn std::error::Error>> {
    let start = Instant::now();
    let status = cmd.status()?;
    let elapsed = start.elapsed();
    if !status.success() {
        return Err(format!("{} failed [{:.1}s]", name, elapsed.as_secs_f64()).into());
    }
    println!("  {} passed ({:.1}s)", name, elapsed.as_secs_f64());
    Ok(())
}

pub fn run(args: PreCommitArgs) -> Result<(), Box<dyn std::error::Error>> {
    let root = repo_root();
    let total_start = Instant::now();

    println!("Pre-commit checks:\n");

    // 1. Format check
    if !args.no_fmt {
        if args.fix {
            run_step(
                "cargo fmt",
                Command::new("cargo")
                    .args(["fmt", "--all"])
                    .current_dir(&root),
            )?;
        } else {
            run_step(
                "cargo fmt --check",
                Command::new("cargo")
                    .args(["fmt", "--all", "--", "--check"])
                    .current_dir(&root),
            )?;
        }
    }

    // 2. Clippy
    if !args.no_clippy {
        run_step(
            "clippy",
            Command::new("cargo")
                .args([
                    "clippy",
                    "--workspace",
                    "--all-targets",
                    "--",
                    "-D",
                    "warnings",
                ])
                .current_dir(&root),
        )?;
    }

    // 3. Tests (only lib tests for speed)
    if !args.no_test {
        run_step(
            "cargo test (lib)",
            Command::new("cargo")
                .args(["test", "--workspace", "--lib"])
                .current_dir(&root),
        )?;
    }

    let total = total_start.elapsed();
    println!(
        "\nAll pre-commit checks passed ({:.1}s)",
        total.as_secs_f64()
    );
    Ok(())
}
