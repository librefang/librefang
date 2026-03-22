use crate::common::repo_root;
use clap::Parser;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Parser, Debug)]
pub struct CheckLinksArgs {
    /// Only check specific directory
    #[arg(long)]
    pub path: Option<String>,

    /// Exclude patterns (comma-separated)
    #[arg(long)]
    pub exclude: Option<String>,

    /// Use built-in basic checker instead of lychee
    #[arg(long)]
    pub basic: bool,
}

fn has_lychee() -> bool {
    Command::new("lychee")
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn run_lychee(root: &Path, args: &CheckLinksArgs) -> Result<(), Box<dyn std::error::Error>> {
    let target = if let Some(ref p) = args.path {
        p.clone()
    } else {
        ".".to_string()
    };

    let mut cmd_args = vec![
        target,
        "--glob-ignore-case".to_string(),
        "--include-fragments".to_string(),
    ];

    // Default excludes for common false positives
    let excludes = vec!["localhost", "127.0.0.1", "example.com", "your-domain.com"];
    for exclude in &excludes {
        cmd_args.push("--exclude".to_string());
        cmd_args.push(exclude.to_string());
    }

    // User-provided excludes
    if let Some(ref user_excludes) = args.exclude {
        for pattern in user_excludes.split(',') {
            let trimmed = pattern.trim();
            if !trimmed.is_empty() {
                cmd_args.push("--exclude".to_string());
                cmd_args.push(trimmed.to_string());
            }
        }
    }

    println!("Running lychee link checker...");
    let status = Command::new("lychee")
        .args(&cmd_args)
        .current_dir(root)
        .status()?;

    if !status.success() {
        return Err("lychee found broken links (see output above)".into());
    }

    Ok(())
}

fn basic_check(root: &Path, args: &CheckLinksArgs) -> Result<(), Box<dyn std::error::Error>> {
    use regex::Regex;

    let target_dir = if let Some(ref p) = args.path {
        root.join(p)
    } else {
        root.to_path_buf()
    };

    println!("Running basic link checker...");
    let link_re = Regex::new(r"\[([^\]]*)\]\(([^)]+)\)")?;
    let mut total = 0;
    let mut broken = 0;
    let mut checked_files = 0;

    for entry in walkdir(&target_dir, "md")? {
        checked_files += 1;
        let content = fs::read_to_string(&entry)?;
        let rel_path = entry.strip_prefix(root).unwrap_or(&entry);

        for cap in link_re.captures_iter(&content) {
            let link = cap.get(2).unwrap().as_str();
            total += 1;

            // Skip URLs, anchors, and mailto
            if link.starts_with("http://")
                || link.starts_with("https://")
                || link.starts_with('#')
                || link.starts_with("mailto:")
            {
                continue;
            }

            // Check relative file links
            let link_path = link.split('#').next().unwrap_or(link);
            if link_path.is_empty() {
                continue;
            }

            let base_dir = entry.parent().unwrap_or(root);
            let resolved = base_dir.join(link_path);
            if !resolved.exists() {
                println!("  BROKEN: {}  ->  {}", rel_path.display(), link);
                broken += 1;
            }
        }
    }

    println!();
    println!(
        "Checked {} files, {} links, {} broken",
        checked_files, total, broken
    );

    if broken > 0 {
        Err(format!("{} broken link(s) found", broken).into())
    } else {
        println!("All links OK.");
        Ok(())
    }
}

fn walkdir(dir: &Path, ext: &str) -> Result<Vec<PathBuf>, Box<dyn std::error::Error>> {
    let mut results = Vec::new();
    if !dir.exists() {
        return Ok(results);
    }
    walk_recursive(dir, ext, &mut results)?;
    Ok(results)
}

fn walk_recursive(
    dir: &Path,
    ext: &str,
    results: &mut Vec<PathBuf>,
) -> Result<(), Box<dyn std::error::Error>> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            // Skip common non-content directories
            let name = path.file_name().unwrap_or_default().to_string_lossy();
            if name.starts_with('.') || name == "node_modules" || name == "target" || name == "dist"
            {
                continue;
            }
            walk_recursive(&path, ext, results)?;
        } else if path.extension().and_then(|e| e.to_str()) == Some(ext) {
            results.push(path);
        }
    }
    Ok(())
}

pub fn run(args: CheckLinksArgs) -> Result<(), Box<dyn std::error::Error>> {
    let root = repo_root();

    if args.basic || !has_lychee() {
        if !args.basic && !has_lychee() {
            println!(
                "lychee not found — using basic checker (install lychee for full link checking)"
            );
            println!("  cargo install lychee");
            println!();
        }
        basic_check(&root, &args)
    } else {
        run_lychee(&root, &args)
    }
}
