use crate::common::repo_root;
use clap::Parser;
use std::path::Path;
use std::process::Command;
use std::time::Instant;

#[derive(Parser, Debug)]
pub struct BuildWebArgs {
    /// Build React dashboard only
    #[arg(long)]
    pub dashboard: bool,

    /// Build web/ frontend only
    #[arg(long)]
    pub web: bool,

    /// Build docs/ site only
    #[arg(long)]
    pub docs: bool,
}

fn run_pnpm_build(dir: &Path, label: &str) -> Result<(), Box<dyn std::error::Error>> {
    let package_json = dir.join("package.json");
    if !package_json.exists() {
        println!("  Skipping {} (no package.json)", label);
        return Ok(());
    }

    println!("Building {}...", label);
    let start = Instant::now();

    // pnpm install
    println!("  Running pnpm install --frozen-lockfile...");
    let status = Command::new("pnpm")
        .args(["install", "--frozen-lockfile"])
        .current_dir(dir)
        .status()?;
    if !status.success() {
        return Err(format!("pnpm install failed for {}", label).into());
    }

    // pnpm run build
    println!("  Running pnpm run build...");
    let status = Command::new("pnpm")
        .args(["run", "build"])
        .current_dir(dir)
        .status()?;
    if !status.success() {
        return Err(format!("pnpm run build failed for {}", label).into());
    }

    let elapsed = start.elapsed();
    println!(
        "  {} built successfully ({:.1}s)",
        label,
        elapsed.as_secs_f64()
    );
    Ok(())
}

pub fn run(args: BuildWebArgs) -> Result<(), Box<dyn std::error::Error>> {
    let root = repo_root();

    // If no specific flag is set, build all
    let build_all = !args.dashboard && !args.web && !args.docs;

    if build_all || args.dashboard {
        let dashboard_dir = root.join("crates/librefang-api/dashboard");
        run_pnpm_build(&dashboard_dir, "dashboard")?;
    }

    if build_all || args.web {
        let web_dir = root.join("web");
        run_pnpm_build(&web_dir, "web")?;
    }

    if build_all || args.docs {
        let docs_dir = root.join("docs");
        run_pnpm_build(&docs_dir, "docs")?;
    }

    println!("All web builds complete.");
    Ok(())
}
