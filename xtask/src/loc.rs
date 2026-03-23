use crate::common::repo_root;
use clap::Parser;
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

#[derive(Parser, Debug)]
pub struct LocArgs {
    /// Show per-crate breakdown
    #[arg(long)]
    pub crates: bool,

    /// Include web/frontend code
    #[arg(long)]
    pub web: bool,

    /// Show dependency graph
    #[arg(long)]
    pub deps: bool,
}

struct LineCount {
    code: usize,
    blank: usize,
    comment: usize,
    files: usize,
}

impl LineCount {
    fn total(&self) -> usize {
        self.code + self.blank + self.comment
    }
}

fn count_file(path: &Path) -> Option<LineCount> {
    let content = fs::read_to_string(path).ok()?;
    let mut lc = LineCount {
        code: 0,
        blank: 0,
        comment: 0,
        files: 1,
    };
    let ext = path.extension()?.to_str()?;
    let comment_prefix = match ext {
        "rs" | "js" | "ts" | "tsx" | "jsx" => "//",
        "py" | "toml" | "yaml" | "yml" => "#",
        _ => "//",
    };

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            lc.blank += 1;
        } else if trimmed.starts_with(comment_prefix) {
            lc.comment += 1;
        } else {
            lc.code += 1;
        }
    }
    Some(lc)
}

fn walk_dir(dir: &Path, extensions: &[&str], counts: &mut BTreeMap<String, LineCount>) {
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        let name = path.file_name().unwrap_or_default().to_string_lossy();

        // Skip build/dependency directories
        if name == "target"
            || name == "node_modules"
            || name == ".git"
            || name == "dist"
            || name == ".next"
            || name == "out"
            || name == "vendor"
        {
            continue;
        }

        if path.is_dir() {
            walk_dir(&path, extensions, counts);
        } else if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
            if extensions.contains(&ext) {
                if let Some(lc) = count_file(&path) {
                    // Determine crate/module name
                    let key = path
                        .strip_prefix(dir)
                        .ok()
                        .and_then(|p| p.components().next())
                        .map(|c| c.as_os_str().to_string_lossy().to_string())
                        .unwrap_or_else(|| "root".to_string());

                    let entry = counts.entry(key).or_insert(LineCount {
                        code: 0,
                        blank: 0,
                        comment: 0,
                        files: 0,
                    });
                    entry.code += lc.code;
                    entry.blank += lc.blank;
                    entry.comment += lc.comment;
                    entry.files += lc.files;
                }
            }
        }
    }
}

fn print_table(label: &str, counts: &BTreeMap<String, LineCount>, show_crates: bool) {
    let total = counts.values().fold(
        LineCount {
            code: 0,
            blank: 0,
            comment: 0,
            files: 0,
        },
        |mut acc, lc| {
            acc.code += lc.code;
            acc.blank += lc.blank;
            acc.comment += lc.comment;
            acc.files += lc.files;
            acc
        },
    );

    println!("=== {} ===", label);
    if show_crates {
        println!(
            "  {:<35} {:>8} {:>8} {:>8} {:>6}",
            "Module", "Code", "Comment", "Blank", "Files"
        );
        println!("  {}", "-".repeat(73));
        for (name, lc) in counts {
            println!(
                "  {:<35} {:>8} {:>8} {:>8} {:>6}",
                name, lc.code, lc.comment, lc.blank, lc.files
            );
        }
        println!("  {}", "-".repeat(73));
    }
    println!(
        "  {:<35} {:>8} {:>8} {:>8} {:>6}",
        "TOTAL", total.code, total.comment, total.blank, total.files
    );
    println!("  Total lines: {}", total.total());
    println!();
}

pub fn run(args: LocArgs) -> Result<(), Box<dyn std::error::Error>> {
    let root = repo_root();

    if args.deps {
        // Show workspace dependency graph
        println!("=== Workspace Dependency Graph ===");
        let output = std::process::Command::new("cargo")
            .args(["metadata", "--format-version=1", "--no-deps"])
            .current_dir(&root)
            .output()?;
        let metadata: serde_json::Value = serde_json::from_slice(&output.stdout)?;
        if let Some(packages) = metadata["packages"].as_array() {
            for pkg in packages {
                let name = pkg["name"].as_str().unwrap_or("?");
                let dep_names: Vec<&str> = pkg["dependencies"]
                    .as_array()
                    .map(|deps| {
                        deps.iter()
                            .filter_map(|d| d["name"].as_str())
                            .filter(|n| n.starts_with("librefang"))
                            .collect()
                    })
                    .unwrap_or_default();
                if name.starts_with("librefang") || name == "xtask" {
                    if dep_names.is_empty() {
                        println!("  {} (leaf)", name);
                    } else {
                        println!("  {} -> {}", name, dep_names.join(", "));
                    }
                }
            }
        }
        println!();
    }

    // Count Rust code
    let mut rust_counts = BTreeMap::new();
    let crates_dir = root.join("crates");
    if crates_dir.exists() {
        for entry in fs::read_dir(&crates_dir)?.flatten() {
            if entry.path().is_dir() {
                let mut sub = BTreeMap::new();
                walk_dir(&entry.path(), &["rs"], &mut sub);
                let name = entry.file_name().to_string_lossy().to_string();
                let total = sub.values().fold(
                    LineCount {
                        code: 0,
                        blank: 0,
                        comment: 0,
                        files: 0,
                    },
                    |mut acc, lc| {
                        acc.code += lc.code;
                        acc.blank += lc.blank;
                        acc.comment += lc.comment;
                        acc.files += lc.files;
                        acc
                    },
                );
                rust_counts.insert(name, total);
            }
        }
    }
    // xtask
    let mut xtask_counts = BTreeMap::new();
    walk_dir(&root.join("xtask"), &["rs"], &mut xtask_counts);
    let xtask_total = xtask_counts.values().fold(
        LineCount {
            code: 0,
            blank: 0,
            comment: 0,
            files: 0,
        },
        |mut acc, lc| {
            acc.code += lc.code;
            acc.blank += lc.blank;
            acc.comment += lc.comment;
            acc.files += lc.files;
            acc
        },
    );
    rust_counts.insert("xtask".to_string(), xtask_total);

    print_table("Rust", &rust_counts, args.crates);

    if args.web {
        let mut web_counts = BTreeMap::new();
        let web_dir = root.join("web");
        if web_dir.exists() {
            walk_dir(
                &web_dir,
                &["ts", "tsx", "js", "jsx", "css"],
                &mut web_counts,
            );
        }
        let dashboard_dir = root.join("crates/librefang-api/dashboard");
        if dashboard_dir.exists() {
            let mut dash = BTreeMap::new();
            walk_dir(
                &dashboard_dir,
                &["ts", "tsx", "js", "jsx", "css", "html"],
                &mut dash,
            );
            let total = dash.values().fold(
                LineCount {
                    code: 0,
                    blank: 0,
                    comment: 0,
                    files: 0,
                },
                |mut acc, lc| {
                    acc.code += lc.code;
                    acc.blank += lc.blank;
                    acc.comment += lc.comment;
                    acc.files += lc.files;
                    acc
                },
            );
            web_counts.insert("dashboard".to_string(), total);
        }
        print_table("Web/Frontend", &web_counts, args.crates);
    }

    Ok(())
}
