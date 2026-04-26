use crate::common::repo_root;
use clap::Parser;
use regex::Regex;
use std::fs;
use std::path::Path;
use std::process::Command;

#[derive(Parser, Debug)]
pub struct ChangelogArgs {
    /// Version for the changelog entry (e.g. 2026.3.2114)
    pub version: String,

    /// Base tag to compare from (default: latest non-prerelease tag)
    pub base_tag: Option<String>,
}

fn find_latest_stable_tag(root: &Path) -> Option<String> {
    let output = Command::new("git")
        .args(["tag", "--sort=-creatordate"])
        .current_dir(root)
        .output()
        .ok()?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let version_re = Regex::new(r"^v[0-9]").unwrap();
    let prerelease_re = Regex::new(r"(alpha|beta|rc)").unwrap();
    for line in stdout.lines() {
        let tag = line.trim();
        if version_re.is_match(tag) && !prerelease_re.is_match(tag) {
            return Some(tag.to_string());
        }
    }
    None
}

fn extract_pr_numbers(root: &Path, git_range: &str) -> Vec<u64> {
    let args = if git_range == "HEAD" {
        vec!["log", "--oneline", "HEAD"]
    } else {
        vec!["log", "--oneline", git_range]
    };
    let output = Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .ok();
    let stdout = match output {
        Some(o) => String::from_utf8_lossy(&o.stdout).to_string(),
        None => return vec![],
    };
    let re = Regex::new(r"#(\d+)").unwrap();
    let mut nums: Vec<u64> = re
        .captures_iter(&stdout)
        .filter_map(|cap| cap.get(1)?.as_str().parse().ok())
        .collect();
    nums.sort_unstable();
    nums.dedup();
    nums
}

#[derive(Debug)]
struct PrInfo {
    number: u64,
    title: String,
    author: String,
    /// Conventional-commits breaking-change marker: `feat!:`, `fix(scope)!:`, etc.
    breaking: bool,
}

fn fetch_pr_info(num: u64) -> Option<PrInfo> {
    let output = Command::new("gh")
        .args([
            "pr",
            "view",
            &num.to_string(),
            "--json",
            "number,title,author",
        ])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).ok()?;
    let title = json["title"].as_str()?.to_string();
    let breaking_re = Regex::new(r"^\w+(?:\([^)]*\))?!:").unwrap();
    Some(PrInfo {
        number: json["number"].as_u64()?,
        breaking: breaking_re.is_match(&title),
        title,
        author: json["author"]["login"].as_str().unwrap_or("").to_string(),
    })
}

fn classify_prefix(prefix: &str) -> &'static str {
    match prefix {
        "feat" => "Added",
        "fix" => "Fixed",
        "refactor" => "Changed",
        "perf" => "Performance",
        "docs" | "doc" => "Documentation",
        "chore" | "ci" | "build" | "test" | "style" => "Maintenance",
        "revert" => "Reverted",
        _ => "Other",
    }
}

fn should_skip(title: &str) -> bool {
    let patterns = [
        Regex::new(r"(?i)Update contributors and star history").unwrap(),
        Regex::new(r"^v?\d+\.\d+\.\d+").unwrap(),
        Regex::new(r"(?i)^release:").unwrap(),
    ];
    patterns.iter().any(|re| re.is_match(title))
}

const CATEGORY_ORDER: &[&str] = &[
    "Added",
    "Fixed",
    "Changed",
    "Performance",
    "Documentation",
    "Maintenance",
    "Reverted",
    "Other",
];

/// Categories visible above the fold. Everything else (Documentation,
/// Maintenance, Other, Reverted) is folded into a `<details>` block at the
/// bottom of the section to keep the user-facing view scannable.
const PRIMARY_CATEGORIES: &[&str] = &["Added", "Fixed", "Changed", "Performance"];

fn generate_classified_output(prs: &[PrInfo]) -> String {
    let conv_re = Regex::new(r"^(\w+)(?:\([^)]*\))?[!]?:\s*(.*)").unwrap();
    let mut categories: std::collections::HashMap<&str, Vec<String>> =
        std::collections::HashMap::new();

    for pr in prs {
        let title = pr.title.trim();
        if should_skip(title) {
            continue;
        }

        let credit = if pr.author.is_empty() {
            String::new()
        } else {
            format!(" (@{})", pr.author)
        };

        let (category, desc) = if let Some(caps) = conv_re.captures(title) {
            let prefix = caps.get(1).unwrap().as_str().to_lowercase();
            let desc_part = caps.get(2).unwrap().as_str().trim().to_string();
            let cat = classify_prefix(&prefix);
            (cat, desc_part)
        } else {
            ("Other", title.to_string())
        };

        // Capitalize first letter
        let desc = if desc.is_empty() {
            title.to_string()
        } else {
            let mut chars = desc.chars();
            match chars.next() {
                None => desc,
                Some(c) => c.to_uppercase().to_string() + chars.as_str(),
            }
        };

        categories
            .entry(category)
            .or_default()
            .push(format!("{} (#{}){}", desc, pr.number, credit));
    }

    let mut output = String::new();
    let mut secondary = String::new();
    for &cat in CATEGORY_ORDER {
        let Some(items) = categories.get(cat) else {
            continue;
        };
        if items.is_empty() {
            continue;
        }
        let target = if PRIMARY_CATEGORIES.contains(&cat) {
            &mut output
        } else {
            &mut secondary
        };
        target.push_str(&format!("### {}\n\n", cat));
        for item in items {
            target.push_str(&format!("- {}\n", item));
        }
        target.push('\n');
    }

    if !secondary.is_empty() {
        output.push_str("<details>\n<summary>Documentation, maintenance, and other internal changes</summary>\n\n");
        output.push_str(&secondary);
        output.push_str("</details>\n\n");
    }

    output
}

/// Build a `### Breaking Changes` block from PRs whose conventional-commit
/// title carries the `!` marker (`feat!:`, `fix(scope)!:`, etc.). Returns
/// `None` when there are none — the section is omitted entirely.
fn generate_breaking_changes(prs: &[PrInfo]) -> Option<String> {
    let conv_re = Regex::new(r"^(\w+)(?:\([^)]*\))?!:\s*(.*)").unwrap();
    let mut bullets = Vec::new();
    for pr in prs {
        if !pr.breaking || should_skip(pr.title.trim()) {
            continue;
        }
        let credit = if pr.author.is_empty() {
            String::new()
        } else {
            format!(" (@{})", pr.author)
        };
        let desc = conv_re
            .captures(pr.title.trim())
            .and_then(|c| c.get(2).map(|m| m.as_str().trim().to_string()))
            .unwrap_or_else(|| pr.title.trim().to_string());
        let desc = {
            let mut chars = desc.chars();
            match chars.next() {
                None => desc,
                Some(c) => c.to_uppercase().to_string() + chars.as_str(),
            }
        };
        bullets.push(format!("- {} (#{}){}", desc, pr.number, credit));
    }
    if bullets.is_empty() {
        return None;
    }
    let mut block = String::from("### Breaking Changes\n\n");
    for b in bullets {
        block.push_str(&b);
        block.push('\n');
    }
    block.push('\n');
    Some(block)
}

/// One-line stats prefix: `_N PRs from M contributors since vBASE._`
/// `base_tag` is the version we're comparing from; `None` when unknown.
fn generate_stats_line(prs: &[PrInfo], base_tag: Option<&str>) -> Option<String> {
    let included: Vec<&PrInfo> = prs
        .iter()
        .filter(|p| !should_skip(p.title.trim()))
        .collect();
    if included.is_empty() {
        return None;
    }
    let pr_count = included.len();
    let mut authors: Vec<&str> = included
        .iter()
        .map(|p| p.author.as_str())
        .filter(|a| !a.is_empty())
        .collect();
    authors.sort_unstable();
    authors.dedup();
    let author_count = authors.len();
    let pr_word = if pr_count == 1 { "PR" } else { "PRs" };
    let contrib_word = if author_count == 1 {
        "contributor"
    } else {
        "contributors"
    };
    let suffix = match base_tag {
        Some(t) => format!(" since {}", t),
        None => String::new(),
    };
    Some(format!(
        "_{} {} from {} {}{}._\n\n",
        pr_count, pr_word, author_count, contrib_word, suffix
    ))
}

/// Summarize the classified changelog into a `### Highlights` block via local
/// `claude` CLI. Returns `None` if claude isn't installed, the call fails, or
/// the response is empty — never propagates errors to gate the release.
fn generate_highlights(classified: &str) -> Option<String> {
    if classified.trim().is_empty() {
        return None;
    }

    if Command::new("claude").arg("--version").output().is_err() {
        println!("  claude CLI not available, skipping Highlights generation");
        return None;
    }

    let prompt = format!(
        "Summarize this LibreFang release changelog into 3-5 user-facing highlights as a markdown bullet list under a `### Highlights` heading. \
        Lead each bullet with the headline feature name in **bold**, followed by an em dash and a short clause. \
        Pick the most impactful user-visible changes; group related items into one bullet. \
        Skip internal milestone names (M2, M3, etc.), test/CI/typecheck fixes, refactors, and pure maintenance. \
        Output ONLY the `### Highlights` section and its bullets — no preamble, no trailing prose.\n\n\
        Changelog:\n{}",
        classified
    );

    let output = Command::new("claude")
        .args([
            "-p",
            "--model",
            "claude-sonnet-4-6",
            "--output-format",
            "text",
            &prompt,
        ])
        .env_remove("CLAUDECODE")
        .output()
        .ok()?;

    if !output.status.success() {
        println!("  claude call failed, skipping Highlights generation");
        return None;
    }

    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if text.is_empty() {
        return None;
    }

    let block = if text.starts_with("### Highlights") {
        format!("{}\n\n", text)
    } else {
        format!("### Highlights\n\n{}\n\n", text)
    };
    println!("  Generated Highlights via claude");
    Some(block)
}

fn write_changelog(
    changelog_path: &Path,
    version: &str,
    classified: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let date = chrono::Local::now().format("%Y-%m-%d").to_string();

    let section = if classified.is_empty() {
        format!("## [{}] - {}\n\n_No notable changes._\n", version, date)
    } else {
        format!("## [{}] - {}\n\n{}", version, date, classified)
    };

    if !changelog_path.exists() {
        let content = format!("# Changelog\n\n{}\n", section);
        fs::write(changelog_path, content)?;
    } else {
        let content = fs::read_to_string(changelog_path)?;
        let heading_re = Regex::new(r"(?m)^## \[")?;
        let version_re = Regex::new(&format!(r"(?m)^## \[{}\]", regex::escape(version)))?;

        if version_re.is_match(&content) {
            // Replace existing section
            println!("Replacing existing changelog entry for {}", version);
            let lines: Vec<&str> = content.lines().collect();
            let mut start = None;
            let mut end = None;
            let version_heading = format!("## [{}]", version);
            for (i, line) in lines.iter().enumerate() {
                if line.starts_with(&version_heading) {
                    start = Some(i);
                } else if start.is_some() && end.is_none() && line.starts_with("## [") {
                    end = Some(i);
                }
            }
            if let Some(s) = start {
                let mut result = String::new();
                for line in &lines[..s] {
                    result.push_str(line);
                    result.push('\n');
                }
                result.push_str(&section);
                result.push('\n');
                if let Some(e) = end {
                    for line in &lines[e..] {
                        result.push_str(line);
                        result.push('\n');
                    }
                }
                fs::write(changelog_path, result)?;
            }
        } else if let Some(m) = heading_re.find(&content) {
            // Insert before first ## [
            let pos = m.start();
            let mut result = String::new();
            result.push_str(&content[..pos]);
            result.push_str(&section);
            result.push('\n');
            result.push_str(&content[pos..]);
            fs::write(changelog_path, result)?;
        } else {
            // Append
            let mut result = content;
            result.push('\n');
            result.push_str(&section);
            result.push('\n');
            fs::write(changelog_path, result)?;
        }
    }

    Ok(())
}

pub fn run(args: ChangelogArgs) -> Result<(), Box<dyn std::error::Error>> {
    let root = repo_root();
    let changelog_path = root.join("CHANGELOG.md");

    let base_tag = args.base_tag.or_else(|| find_latest_stable_tag(&root));

    println!(
        "Generating changelog: {} (since {})",
        args.version,
        base_tag.as_deref().unwrap_or("beginning")
    );

    // Check for gh CLI
    if Command::new("gh").arg("--version").output().is_err() {
        return Err("gh CLI required".into());
    }

    let git_range = match &base_tag {
        Some(tag) => format!("{}..HEAD", tag),
        None => "HEAD".to_string(),
    };

    let pr_numbers = extract_pr_numbers(&root, &git_range);

    if pr_numbers.is_empty() {
        println!("No PRs found in range {}", git_range);
    }

    // Fetch PR info
    let prs: Vec<PrInfo> = pr_numbers
        .iter()
        .filter_map(|&num| fetch_pr_info(num))
        .collect();

    let classified = generate_classified_output(&prs);
    let breaking = generate_breaking_changes(&prs).unwrap_or_default();
    let stats = generate_stats_line(&prs, base_tag.as_deref()).unwrap_or_default();

    // Feed breaking + classified to claude so highlights can flag breaking items.
    let highlights_input = format!("{}{}", breaking, classified);
    let highlights = generate_highlights(&highlights_input).unwrap_or_default();

    let final_output = format!("{}{}{}{}", stats, breaking, highlights, classified);

    write_changelog(&changelog_path, &args.version, &final_output)?;

    println!("Updated {}", changelog_path.display());

    // Print summary
    let pr_count = prs.len();
    let skip_count = prs.iter().filter(|pr| should_skip(pr.title.trim())).count();
    println!(
        "Summary: {} PRs found, {} skipped, {} included",
        pr_count,
        skip_count,
        pr_count - skip_count
    );

    Ok(())
}
