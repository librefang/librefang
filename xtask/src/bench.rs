use crate::common::repo_root;
use crate::local_check_mode;
use clap::Parser;
use std::process::Command;

#[derive(Parser, Debug)]
pub struct BenchArgs {
    /// Run a specific benchmark by name
    #[arg(long)]
    pub name: Option<String>,

    /// Save baseline for comparison
    #[arg(long)]
    pub save_baseline: Option<String>,

    /// Compare against a saved baseline
    #[arg(long)]
    pub baseline: Option<String>,

    /// Open HTML report in browser
    #[arg(long)]
    pub open: bool,
}

pub fn run(args: BenchArgs) -> Result<(), Box<dyn std::error::Error>> {
    local_check_mode::apply_for_subcommand("bench");

    let root = repo_root();

    let mut cmd = Command::new("cargo");
    cmd.arg("bench").current_dir(&root);

    // If a specific benchmark name is given, filter to it
    if let Some(ref name) = args.name {
        cmd.args(["--bench", name]);
    }

    // Pass criterion arguments after --
    let mut criterion_args: Vec<String> = Vec::new();

    if let Some(ref baseline) = args.save_baseline {
        criterion_args.push("--save-baseline".to_string());
        criterion_args.push(baseline.clone());
    }

    if let Some(ref baseline) = args.baseline {
        criterion_args.push("--baseline".to_string());
        criterion_args.push(baseline.clone());
    }

    if !criterion_args.is_empty() {
        cmd.arg("--");
        cmd.args(&criterion_args);
    }

    println!("Running benchmarks...");
    if let Some(ref name) = args.name {
        println!("  Filter: {}", name);
    }
    if let Some(ref b) = args.save_baseline {
        println!("  Saving baseline: {}", b);
    }
    if let Some(ref b) = args.baseline {
        println!("  Comparing against: {}", b);
    }
    println!();

    let status = cmd.status()?;

    if !status.success() {
        return Err("cargo bench failed".into());
    }

    let report = root.join("target/criterion/report/index.html");
    if report.exists() {
        println!();
        println!("Report: {}", report.display());
        if args.open {
            let _ = Command::new("open").arg(&report).status();
        }
    }

    Ok(())
}
