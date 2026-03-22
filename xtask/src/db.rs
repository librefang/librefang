use clap::Parser;
use std::fs;
use std::path::PathBuf;

#[derive(Parser, Debug)]
pub struct DbArgs {
    /// Reset database (delete and recreate)
    #[arg(long)]
    pub reset: bool,

    /// Show database file info
    #[arg(long)]
    pub info: bool,

    /// Custom data directory (default: ~/.librefang)
    #[arg(long)]
    pub data_dir: Option<String>,

    /// Backup database to a file
    #[arg(long)]
    pub backup: Option<String>,
}

fn default_data_dir() -> PathBuf {
    std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .ok()
        .map(|h| PathBuf::from(h).join(".librefang"))
        .unwrap_or_else(|| PathBuf::from(".librefang"))
}

fn find_db_files(data_dir: &PathBuf) -> Vec<PathBuf> {
    let mut dbs = Vec::new();
    if data_dir.exists() {
        if let Ok(entries) = fs::read_dir(data_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if let Some(ext) = path.extension() {
                    if ext == "db" || ext == "sqlite" || ext == "sqlite3" {
                        dbs.push(path);
                    }
                }
            }
        }
    }
    dbs
}

fn file_size_human(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{} B", bytes)
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    }
}

pub fn run(args: DbArgs) -> Result<(), Box<dyn std::error::Error>> {
    let data_dir = args
        .data_dir
        .map(PathBuf::from)
        .unwrap_or_else(default_data_dir);

    println!("Data directory: {}", data_dir.display());

    if !data_dir.exists() {
        println!("  Directory does not exist yet.");
        return Ok(());
    }

    let db_files = find_db_files(&data_dir);

    if args.info || (!args.reset && args.backup.is_none()) {
        // Default action: show info
        if db_files.is_empty() {
            println!("  No database files found.");
        } else {
            println!("  Database files:");
            for db in &db_files {
                let meta = fs::metadata(db)?;
                let size = file_size_human(meta.len());
                println!(
                    "    {} ({})",
                    db.file_name().unwrap_or_default().to_string_lossy(),
                    size
                );
            }
        }
        return Ok(());
    }

    if let Some(backup_path) = &args.backup {
        let backup_dir = PathBuf::from(backup_path);
        fs::create_dir_all(&backup_dir)?;
        for db in &db_files {
            let dest = backup_dir.join(db.file_name().unwrap());
            fs::copy(db, &dest)?;
            println!(
                "  Backed up: {} -> {}",
                db.file_name().unwrap().to_string_lossy(),
                dest.display()
            );
        }
        println!("Backup complete.");
        return Ok(());
    }

    if args.reset {
        if db_files.is_empty() {
            println!("  No database files to reset.");
            return Ok(());
        }
        println!("  Resetting {} database file(s)...", db_files.len());
        for db in &db_files {
            fs::remove_file(db)?;
            println!(
                "    Removed: {}",
                db.file_name().unwrap_or_default().to_string_lossy()
            );
        }
        // Also remove WAL/SHM files
        for suffix in &["-wal", "-shm"] {
            for db in &db_files {
                let wal = db.with_extension(format!(
                    "{}{}",
                    db.extension().unwrap_or_default().to_string_lossy(),
                    suffix
                ));
                if wal.exists() {
                    fs::remove_file(&wal)?;
                }
            }
        }
        println!("Database reset complete. Restart the daemon to recreate.");
    }

    Ok(())
}
