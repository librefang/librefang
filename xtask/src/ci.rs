use crate::common::repo_root;
use crate::local_check_mode;
use clap::Parser;
use std::process::Command;
use std::time::Instant;

#[derive(Parser, Debug)]
pub struct CiArgs {
    /// Skip web lint step
    #[arg(long)]
    pub no_web: bool,

    /// Skip test step
    #[arg(long)]
    pub no_test: bool,

    /// Use release profile for build
    #[arg(long)]
    pub release: bool,
}

fn run_step(name: &str, cmd: &mut Command) -> Result<(), Box<dyn std::error::Error>> {
    println!("=== {} ===", name);
    let start = Instant::now();
    let status = cmd.status()?;
    let elapsed = start.elapsed();
    if !status.success() {
        return Err(format!(
            "{} failed (exit code: {:?}) [{:.1}s]",
            name,
            status.code(),
            elapsed.as_secs_f64()
        )
        .into());
    }
    println!("  Passed ({:.1}s)", elapsed.as_secs_f64());
    println!();
    Ok(())
}

pub fn run(args: CiArgs) -> Result<(), Box<dyn std::error::Error>> {
    // Apply LIBREFANG_LOCAL_CHECK_MODE before any cargo invocation (#3301).
    // Auto-throttles cargo concurrency on low-spec hosts; CI=true preserves
    // full parallelism. See `local_check_mode` for the behaviour matrix.
    local_check_mode::apply_for_subcommand("ci");

    let root = repo_root();
    let total_start = Instant::now();

    // Step 1: cargo build
    {
        let mut cmd = Command::new("cargo");
        cmd.args(["build", "--workspace", "--lib"])
            .current_dir(&root);
        if args.release {
            cmd.arg("--release");
        }
        run_step("cargo build", &mut cmd)?;
    }

    // Step 2: cargo test (unless --no-test)
    if !args.no_test {
        let mut cmd = Command::new("cargo");
        cmd.args(["test", "--workspace"]).current_dir(&root);
        if args.release {
            cmd.arg("--release");
        }
        run_step("cargo test", &mut cmd)?;
    }

    // Step 3: cargo clippy
    {
        let mut cmd = Command::new("cargo");
        cmd.args([
            "clippy",
            "--workspace",
            "--all-targets",
            "--",
            "-D",
            "warnings",
        ])
        .current_dir(&root);
        run_step("cargo clippy", &mut cmd)?;
    }

    // Step 4: web lint (if web/package.json exists and not --no-web)
    if !args.no_web {
        let web_dir = root.join("web");
        let web_pkg = web_dir.join("package.json");
        if web_pkg.exists() {
            let mut cmd = Command::new("pnpm");
            cmd.args(["run", "lint"]).current_dir(&web_dir);
            run_step("web lint", &mut cmd)?;
        } else {
            println!("Skipping web lint (no web/package.json)");
        }
    }

    let total = total_start.elapsed();
    println!("All CI checks passed ({:.1}s total)", total.as_secs_f64());
    Ok(())
}
