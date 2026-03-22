use crate::common::repo_root;
use clap::Parser;
use std::process::Command;
use std::time::Instant;

#[derive(Parser, Debug)]
pub struct FmtCheckArgs {
    /// Fix formatting issues instead of just checking
    #[arg(long)]
    pub fix: bool,

    /// Skip Rust formatting
    #[arg(long)]
    pub no_rust: bool,

    /// Skip web formatting
    #[arg(long)]
    pub no_web: bool,
}

fn has_command(cmd: &str) -> bool {
    Command::new(cmd)
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

pub fn run(args: FmtCheckArgs) -> Result<(), Box<dyn std::error::Error>> {
    let root = repo_root();
    let total_start = Instant::now();
    let mut issues = 0;

    // Step 1: Rust formatting
    if !args.no_rust {
        println!("=== cargo fmt ===");
        let start = Instant::now();

        let mut cmd = Command::new("cargo");
        cmd.args(["fmt", "--all"]).current_dir(&root);
        if !args.fix {
            cmd.args(["--", "--check"]);
        }

        let status = cmd.status()?;
        let elapsed = start.elapsed();

        if status.success() {
            println!(
                "  {} ({:.1}s)",
                if args.fix { "Formatted" } else { "OK" },
                elapsed.as_secs_f64()
            );
        } else {
            if args.fix {
                println!("  cargo fmt failed ({:.1}s)", elapsed.as_secs_f64());
            } else {
                println!(
                    "  Formatting issues found ({:.1}s) — run with --fix",
                    elapsed.as_secs_f64()
                );
            }
            issues += 1;
        }
        println!();
    }

    // Step 2: Web formatting (prettier)
    if !args.no_web {
        let web_dirs = [
            (root.join("web"), "web"),
            (root.join("crates/librefang-api/dashboard"), "dashboard"),
            (root.join("docs"), "docs"),
        ];

        let has_prettier = has_command("prettier") || has_command("pnpm");

        if has_prettier {
            for (dir, label) in &web_dirs {
                if !dir.join("package.json").exists() {
                    continue;
                }

                println!("=== prettier: {} ===", label);
                let start = Instant::now();

                let (cmd_name, cmd_args) = if args.fix {
                    (
                        "pnpm",
                        vec![
                            "exec",
                            "prettier",
                            "--write",
                            "src/**/*.{ts,tsx,js,jsx,css,json}",
                        ],
                    )
                } else {
                    (
                        "pnpm",
                        vec![
                            "exec",
                            "prettier",
                            "--check",
                            "src/**/*.{ts,tsx,js,jsx,css,json}",
                        ],
                    )
                };

                let status = Command::new(cmd_name)
                    .args(&cmd_args)
                    .current_dir(dir)
                    .status();

                let elapsed = start.elapsed();

                match status {
                    Ok(s) if s.success() => {
                        println!(
                            "  {} ({:.1}s)",
                            if args.fix { "Formatted" } else { "OK" },
                            elapsed.as_secs_f64()
                        );
                    }
                    Ok(_) => {
                        println!("  Issues found ({:.1}s)", elapsed.as_secs_f64());
                        issues += 1;
                    }
                    Err(e) => {
                        println!("  Error: {} ({:.1}s)", e, elapsed.as_secs_f64());
                    }
                }
                println!();
            }
        } else {
            println!("prettier/pnpm not found — skipping web formatting");
            println!();
        }
    }

    let total = total_start.elapsed();
    if issues > 0 {
        println!(
            "{} formatting issue(s) found ({:.1}s total)",
            issues,
            total.as_secs_f64()
        );
        Err(format!("{} formatting issue(s)", issues).into())
    } else {
        println!("All formatting OK ({:.1}s total)", total.as_secs_f64());
        Ok(())
    }
}
