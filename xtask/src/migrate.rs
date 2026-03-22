use clap::Parser;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

#[derive(Parser, Debug)]
pub struct MigrateArgs {
    /// Source framework: openclaw, openfang
    #[arg(long)]
    pub source: String,

    /// Source directory to import from
    #[arg(long)]
    pub source_dir: String,

    /// Target directory (default: ~/.librefang)
    #[arg(long)]
    pub target_dir: Option<String>,

    /// Dry run — show what would be imported
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

pub fn run(args: MigrateArgs) -> Result<(), Box<dyn std::error::Error>> {
    let root = repo_root();

    // Validate source
    let valid_sources = ["openclaw", "openfang"];
    if !valid_sources.contains(&args.source.as_str()) {
        return Err(format!(
            "unknown source '{}' — supported: {}",
            args.source,
            valid_sources.join(", ")
        )
        .into());
    }

    let source_dir = PathBuf::from(&args.source_dir);
    if !source_dir.exists() {
        return Err(format!("source directory not found: {}", source_dir.display()).into());
    }

    let target_dir = args.target_dir.unwrap_or_else(|| {
        dirs_or_home()
            .map(|h| h.join(".librefang").to_string_lossy().to_string())
            .unwrap_or_else(|| ".librefang".to_string())
    });

    println!("Migration:");
    println!("  Source:    {} ({})", args.source, source_dir.display());
    println!("  Target:   {}", target_dir);
    if args.dry_run {
        println!("  Mode:     dry-run");
    }
    println!();

    // Run via cargo, passing args to the migrate binary/test
    let mut cmd = Command::new("cargo");
    cmd.args(["run", "-p", "librefang-migrate", "--"])
        .arg("--source")
        .arg(&args.source)
        .arg("--source-dir")
        .arg(&args.source_dir)
        .arg("--target-dir")
        .arg(&target_dir)
        .current_dir(&root);

    if args.dry_run {
        cmd.arg("--dry-run");
    }

    let status = cmd.status()?;

    if !status.success() {
        return Err("migration failed".into());
    }

    println!();
    println!("Migration complete.");
    Ok(())
}

fn dirs_or_home() -> Option<PathBuf> {
    std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .ok()
        .map(PathBuf::from)
}
