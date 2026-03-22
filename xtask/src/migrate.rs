use clap::Parser;
use std::path::PathBuf;

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

fn dirs_or_home() -> Option<PathBuf> {
    std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .ok()
        .map(PathBuf::from)
}

pub fn run(args: MigrateArgs) -> Result<(), Box<dyn std::error::Error>> {
    let source = match args.source.as_str() {
        "openclaw" => librefang_migrate::MigrateSource::OpenClaw,
        "openfang" => librefang_migrate::MigrateSource::OpenFang,
        other => {
            return Err(
                format!("unknown source '{}' — supported: openclaw, openfang", other).into(),
            )
        }
    };

    let source_dir = PathBuf::from(&args.source_dir);
    if !source_dir.exists() {
        return Err(format!("source directory not found: {}", source_dir.display()).into());
    }

    let target_dir = PathBuf::from(args.target_dir.unwrap_or_else(|| {
        dirs_or_home()
            .map(|h| h.join(".librefang").to_string_lossy().to_string())
            .unwrap_or_else(|| ".librefang".to_string())
    }));

    let options = librefang_migrate::MigrateOptions {
        source,
        source_dir: source_dir.clone(),
        target_dir: target_dir.clone(),
        dry_run: args.dry_run,
    };

    println!("Migration:");
    println!("  Source:    {} ({})", source, source_dir.display());
    println!("  Target:   {}", target_dir.display());
    if args.dry_run {
        println!("  Mode:     dry-run");
    }
    println!();

    let report = librefang_migrate::run_migration(&options)?;
    report.print_summary();

    Ok(())
}
