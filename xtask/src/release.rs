use crate::build_web;
use crate::changelog;
use crate::common::repo_root;
use crate::sync_versions;
use clap::Parser;
use regex::Regex;
use std::fs;
use std::io::{self, Write as _};
use std::path::Path;
use std::process::Command;

#[derive(Parser, Debug)]
pub struct ReleaseArgs {
    /// Explicit version (e.g. 2026.3.2114 or 2026.3.2114-beta1)
    #[arg(long)]
    pub version: Option<String>,

    /// Skip confirmation prompts
    #[arg(long)]
    pub no_confirm: bool,

    /// Skip Dev.to article generation
    #[arg(long)]
    pub no_article: bool,

    /// Local only — don't push or create PR
    #[arg(long)]
    pub no_push: bool,

    /// Create an LTS patch release on the current release/ branch.
    /// Auto-detects the LTS series from branch name and increments patch.
    #[arg(long)]
    pub lts_patch: bool,

    /// Dry run — print what would happen without making changes
    #[arg(long)]
    pub dry_run: bool,
}

fn git(root: &Path, args: &[&str]) -> Result<String, Box<dyn std::error::Error>> {
    let output = Command::new("git").args(args).current_dir(root).output()?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("git {} failed: {}", args.join(" "), stderr).into());
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn current_branch(root: &Path) -> Result<String, Box<dyn std::error::Error>> {
    git(root, &["rev-parse", "--abbrev-ref", "HEAD"])
}

fn is_worktree_clean(root: &Path) -> bool {
    let diff_ok = Command::new("git")
        .args(["diff", "--quiet"])
        .current_dir(root)
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    let cached_ok = Command::new("git")
        .args(["diff", "--cached", "--quiet"])
        .current_dir(root)
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    diff_ok && cached_ok
}

fn read_workspace_version(root: &Path) -> Result<String, Box<dyn std::error::Error>> {
    let content = fs::read_to_string(root.join("Cargo.toml"))?;
    let doc = content.parse::<toml_edit::DocumentMut>()?;
    let version = doc["workspace"]["package"]["version"]
        .as_str()
        .ok_or("could not read workspace.package.version from Cargo.toml")?
        .to_string();
    Ok(version)
}

/// Find the latest tag, optionally including pre-releases (rc, beta).
fn find_latest_tag(root: &Path, include_prerelease: bool) -> Option<String> {
    let output = Command::new("git")
        .args(["tag", "--sort=-creatordate"])
        .current_dir(root)
        .output()
        .ok()?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        let tag = line.trim();
        if tag.starts_with('v') && tag.len() > 1 && tag.as_bytes()[1].is_ascii_digit() {
            if include_prerelease {
                // Skip alpha but include rc and beta
                if !tag.contains("alpha") {
                    return Some(tag.to_string());
                }
            } else if !tag.contains("alpha") && !tag.contains("beta") && !tag.contains("rc") {
                return Some(tag.to_string());
            }
        }
    }
    None
}

fn prompt(message: &str) -> String {
    print!("{}", message);
    io::stdout().flush().unwrap();
    let mut input = String::new();
    io::stdin().read_line(&mut input).unwrap();
    input.trim().to_string()
}

fn compute_calver() -> String {
    let now = chrono::Local::now();
    format!(
        "{}.{}.{}{}",
        now.format("%Y"),
        now.format("%-m"),
        now.format("%d"),
        now.format("%H"),
    )
}

fn extract_changelog_section(content: &str, heading: &str) -> String {
    let lines: Vec<&str> = content.lines().collect();
    let mut start = None;
    let mut end = None;
    for (i, line) in lines.iter().enumerate() {
        if line.starts_with(heading) {
            start = Some(i + 1);
        } else if start.is_some() && end.is_none() && line.starts_with("## [") {
            end = Some(i);
        }
    }
    match start {
        Some(s) => {
            let e = end.unwrap_or(lines.len());
            lines[s..e].join("\n").trim().to_string()
        }
        None => String::new(),
    }
}

pub fn run(args: ReleaseArgs) -> Result<(), Box<dyn std::error::Error>> {
    let root = repo_root();

    // --- LTS patch shortcut ---
    if args.lts_patch {
        return run_lts_patch(&root, &args);
    }

    // --- Dry run with explicit version: skip all preflight ---
    if args.dry_run {
        if let Some(ref v) = args.version {
            let current = read_workspace_version(&root).unwrap_or_default();
            let tag = format!("v{}", v);
            let is_lts = v.contains("-lts");
            let is_pre = v.contains("-beta") || v.contains("-rc");
            println!();
            println!("=== Dry Run ===");
            println!("  Version: {} -> {}", current, v);
            println!("  Tag:     {}", tag);
            if is_lts {
                let lts_ver = v.split("-lts").next().unwrap_or(v);
                let parts: Vec<&str> = lts_ver.split('.').collect();
                let branch = if parts.len() >= 2 {
                    format!("release/{}.{}", parts[0], parts[1])
                } else {
                    format!("release/{}", lts_ver)
                };
                println!("  Type:    LTS");
                println!("  Branch:  {} (auto-created by CI)", branch);
            } else if is_pre {
                println!("  Type:    pre-release");
            } else {
                println!("  Type:    stable");
            }
            println!();
            println!("No changes made.");
            return Ok(());
        }
    }

    // --- Preflight checks ---
    println!("Preflight checks...");

    let branch = current_branch(&root)?;
    if branch != "main" {
        return Err(format!("must be on 'main' branch (currently on '{}')", branch).into());
    }

    if !is_worktree_clean(&root) {
        let status = Command::new("git")
            .args(["status", "--short"])
            .current_dir(&root)
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
            .unwrap_or_default();
        return Err(format!(
            "working tree is dirty. Commit or stash changes first.\n{}",
            status
        )
        .into());
    }

    println!("Pulling latest main...");
    git(&root, &["pull", "--rebase", "origin", "main"])?;

    let current = read_workspace_version(&root)?;
    // Include prerelease tags so rc/beta compare against previous rc/beta
    let prev_tag = find_latest_tag(&root, true);

    // --- Determine version ---
    let version = if let Some(v) = args.version {
        v
    } else {
        let base_version = compute_calver();

        if args.no_confirm {
            // Default to stable
            base_version
        } else {
            // Count existing tags to auto-increment
            let beta_count = Command::new("git")
                .args(["tag", "-l", &format!("v{}-beta*", base_version)])
                .current_dir(&root)
                .output()
                .map(|o| {
                    String::from_utf8_lossy(&o.stdout)
                        .lines()
                        .filter(|l| !l.trim().is_empty())
                        .count()
                })
                .unwrap_or(0);
            let rc_count = Command::new("git")
                .args(["tag", "-l", &format!("v{}-rc*", base_version)])
                .current_dir(&root)
                .output()
                .map(|o| {
                    String::from_utf8_lossy(&o.stdout)
                        .lines()
                        .filter(|l| !l.trim().is_empty())
                        .count()
                })
                .unwrap_or(0);
            let next_beta = beta_count + 1;
            let next_rc = rc_count + 1;

            // Compute LTS: YYYY.M.PATCH-lts
            let lts_base = {
                let now = chrono::Local::now();
                format!("{}.{}", now.format("%Y"), now.format("%-m"))
            };
            // Count existing LTS tags to auto-increment patch
            let lts_count = Command::new("git")
                .args(["tag", "-l", &format!("v{}.*-lts", lts_base)])
                .current_dir(&root)
                .output()
                .map(|o| {
                    String::from_utf8_lossy(&o.stdout)
                        .lines()
                        .filter(|l| !l.trim().is_empty())
                        .count()
                })
                .unwrap_or(0);
            let next_lts_patch = lts_count;

            println!();
            println!(
                "Current version: {} (tag: {})",
                current,
                prev_tag.as_deref().unwrap_or("none")
            );
            println!();
            println!("  1) stable  -> {}", base_version);
            println!("  2) beta    -> {}-beta{}", base_version, next_beta);
            println!("  3) rc      -> {}-rc{}", base_version, next_rc);
            println!("  4) lts     -> {}.{}-lts", lts_base, next_lts_patch);
            println!();

            let choice = prompt("Choose [1/2/3/4]: ");
            match choice.as_str() {
                "1" => base_version,
                "2" => format!("{}-beta{}", base_version, next_beta),
                "3" => format!("{}-rc{}", base_version, next_rc),
                "4" => format!("{}.{}-lts", lts_base, next_lts_patch),
                _ => return Err("Invalid choice".into()),
            }
        }
    };

    // Validate CalVer early, before using version in git tags/branches
    let calver_re =
        Regex::new(r"^[0-9]{4}\.[0-9]{1,2}(\.[0-9]{2,4})?(-(beta|rc)[0-9]+|-lts(\.[0-9]+)?)?$")
            .unwrap();
    if !calver_re.is_match(&version) {
        return Err(format!(
            "'{}' is not a valid CalVer (expected: YYYY.M.DDHH, YYYY.M-lts, etc.)",
            version
        )
        .into());
    }

    let tag = format!("v{}", version);
    let is_prerelease = version.contains("-beta") || version.contains("-rc");
    let is_lts = version.contains("-lts");

    // --- Check if tag already exists ---
    let tag_exists = Command::new("git")
        .args(["rev-parse", &tag])
        .current_dir(&root)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);

    // --- Confirmation ---
    if !args.no_confirm {
        println!();
        println!("=== Release Summary ===");
        println!("  Version: {} -> {}", current, version);
        println!("  Tag:     {}", tag);
        if is_lts {
            println!("  Type:    LTS (long-term support)");
            // v2026.3.0-lts -> release/2026.3, v2026.3.1-lts -> release/2026.3
            let lts_ver = version.split("-lts").next().unwrap_or(&version);
            let parts: Vec<&str> = lts_ver.split('.').collect();
            let lts_branch = if parts.len() >= 2 {
                format!("release/{}.{}", parts[0], parts[1])
            } else {
                format!("release/{}", lts_ver)
            };
            println!("  Branch:  {} (auto-created on push)", lts_branch);
        } else if is_prerelease {
            println!("  Type:    pre-release");
        }
        if tag_exists {
            println!("  Warning: tag {} already exists, will be overwritten", tag);
        }
        if let Some(ref pt) = prev_tag {
            println!(
                "  Review:  https://github.com/librefang/librefang/compare/{}...main",
                pt
            );
        }
        println!();

        let confirm = prompt("Release? [Y/n]: ");
        if confirm.starts_with('n') || confirm.starts_with('N') {
            println!("Aborted.");
            return Ok(());
        }
    }

    // --- Clean up existing tag if re-releasing ---
    let mut prev_tag = prev_tag;
    if tag_exists {
        println!();
        println!("Cleaning up existing tag '{}'...", tag);
        let _ = git(&root, &["tag", "-d", &tag]);
        let _ = Command::new("git")
            .args(["push", "origin", "--delete", &tag])
            .current_dir(&root)
            .status();

        let release_branch_check = format!("chore/bump-version-{}", version);
        let branch_exists = Command::new("git")
            .args([
                "rev-parse",
                "--verify",
                &format!("refs/heads/{}", release_branch_check),
            ])
            .current_dir(&root)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        if branch_exists {
            let _ = git(&root, &["branch", "-D", &release_branch_check]);
        }
        let _ = Command::new("git")
            .args(["push", "origin", "--delete", &release_branch_check])
            .current_dir(&root)
            .status();

        // Delete existing GitHub Release
        let _ = Command::new("gh")
            .args([
                "release",
                "delete",
                &tag,
                "--repo",
                "librefang/librefang",
                "--yes",
            ])
            .current_dir(&root)
            .status();

        // Re-compute prev_tag since we deleted the old one
        prev_tag = find_latest_tag(&root, true);
    }

    // --- Generate changelog ---
    println!();
    println!("Generating changelog...");
    let changelog_version = {
        let base = version.split('-').next().unwrap_or(&version);
        let parts: Vec<&str> = base.split('.').collect();
        if parts.len() == 3 && parts[2].len() == 4 {
            // Strip hour from DDHH -> DD
            format!("{}.{}.{}", parts[0], parts[1], &parts[2][..2])
        } else {
            base.to_string()
        }
    };
    changelog::run(changelog::ChangelogArgs {
        version: changelog_version.clone(),
        base_tag: prev_tag.clone(),
    })?;

    // --- Sync versions ---
    println!();
    println!("Syncing versions...");
    sync_versions::run(sync_versions::SyncVersionsArgs {
        version: Some(version.clone()),
    })?;

    // --- Update Cargo.lock ---
    println!();
    println!("Updating Cargo.lock...");
    let lock_status = Command::new("cargo")
        .args(["update", "--workspace"])
        .current_dir(&root)
        .status();
    match lock_status {
        Ok(s) if s.success() => println!("  Cargo.lock updated"),
        _ => println!("  Warning: cargo update failed, continuing"),
    }

    // --- Generate Dev.to article (skip for pre-releases or --no-article) ---
    let article_path = if !args.no_article && !is_prerelease && !is_lts {
        let article = root.join(format!("articles/release-{}.md", changelog_version));
        if !article.exists() {
            let changelog_path = root.join("CHANGELOG.md");
            if changelog_path.exists() {
                let cl_content = fs::read_to_string(&changelog_path).unwrap_or_default();
                let heading = format!("## [{}]", changelog_version);
                let changes = extract_changelog_section(&cl_content, &heading);
                if !changes.is_empty() {
                    println!();
                    println!("Generating Dev.to article...");
                    // Ensure articles/ directory exists
                    let _ = fs::create_dir_all(root.join("articles"));
                    let article_content = format!(
                        r#"---
title: "LibreFang {} Released"
published: true
description: "LibreFang v{} release notes — open-source Agent OS built in Rust"
tags: rust, ai, opensource, release
canonical_url: https://github.com/librefang/librefang/releases/tag/{}
cover_image: https://raw.githubusercontent.com/librefang/librefang/main/public/assets/logo.png
---

# LibreFang {} Released

We're excited to announce **LibreFang v{}**! Here's what's new:

{}

## Install / Upgrade

```bash
# Binary
curl -fsSL https://get.librefang.ai | sh

# Rust SDK
cargo add librefang

# JavaScript SDK
npm install @librefang/sdk

# Python SDK
pip install librefang-sdk
```

## Links

- [Full Changelog](https://github.com/librefang/librefang/blob/main/CHANGELOG.md)
- [GitHub Release](https://github.com/librefang/librefang/releases/tag/{})
- [GitHub](https://github.com/librefang/librefang)
- [Discord](https://discord.gg/DzTYqAZZmc)
- [Contributing Guide](https://github.com/librefang/librefang/blob/main/docs/CONTRIBUTING.md)
"#,
                        changelog_version,
                        changelog_version,
                        tag,
                        changelog_version,
                        changelog_version,
                        changes,
                        tag,
                    );
                    fs::write(&article, article_content)?;

                    // Polish with Claude CLI if available
                    if let Ok(output) = Command::new("claude")
                        .args([
                            "-p",
                            "--model", "claude-haiku-4-5-20251001",
                            "--output-format", "text",
                            &format!(
                                "You are writing a Dev.to release announcement for LibreFang, an open-source Agent OS built in Rust.\n\
                                Rewrite the article body to be more engaging and developer-friendly.\n\
                                Group related changes, highlight the most impactful ones, and add a brief intro.\n\
                                Keep the same front matter (--- block), Install/Upgrade section, and Links section exactly as-is.\n\
                                Only rewrite the content between the front matter and the Install section.\n\
                                Output the COMPLETE article (front matter + body + install + links), ready to save as-is.\n\n\
                                Current article:\n{}",
                                fs::read_to_string(&article).unwrap_or_default()
                            ),
                        ])
                        .env_remove("CLAUDECODE")
                        .output()
                    {
                        if output.status.success() {
                            let polished = String::from_utf8_lossy(&output.stdout).to_string();
                            if !polished.trim().is_empty() {
                                fs::write(&article, polished)?;
                                println!("  AI polished");
                            }
                        } else {
                            println!("  AI polish failed, using raw changelog");
                        }
                    }

                    println!("  Generated {}", article.display());
                }
            }
            Some(article)
        } else {
            Some(article)
        }
    } else {
        if is_prerelease || is_lts {
            println!();
            println!(
                "Skipping Dev.to article for {}",
                if is_lts { "LTS release" } else { "pre-release" }
            );
        }
        None
    };

    // --- Build dashboard ---
    println!();
    println!("Building React dashboard...");
    let build_result = build_web::run(build_web::BuildWebArgs {
        dashboard: true,
        web: false,
        docs: false,
    });
    if let Err(e) = build_result {
        println!("  Warning: dashboard build failed: {}", e);
    }

    // --- Git add + commit + tag ---
    println!();
    println!("Committing version bump...");

    let files_to_add = [
        "Cargo.toml",
        "Cargo.lock",
        "CHANGELOG.md",
        "sdk/javascript/package.json",
        "sdk/python/setup.py",
        "sdk/rust/Cargo.toml",
        "sdk/rust/README.md",
        "packages/whatsapp-gateway/package.json",
        "crates/librefang-desktop/tauri.conf.json",
        "crates/librefang-api/static/react/",
    ];

    for file in &files_to_add {
        let path = root.join(file);
        if path.exists() {
            let _ = Command::new("git")
                .args(["add", file])
                .current_dir(&root)
                .status();
        }
    }

    // Add article if generated
    if let Some(ref article) = article_path {
        if article.exists() {
            let _ = Command::new("git")
                .args(["add", &article.display().to_string()])
                .current_dir(&root)
                .status();
        }
    }

    // Check if there are staged changes
    let has_changes = !Command::new("git")
        .args(["diff", "--cached", "--quiet"])
        .current_dir(&root)
        .status()
        .map(|s| s.success())
        .unwrap_or(true);

    // --- Create release branch BEFORE committing ---
    // This avoids committing on main (which has branch protection).
    let release_branch = format!("chore/bump-version-{}", version);
    if !args.no_push {
        println!();
        println!("Creating release branch '{}'...", release_branch);
        git(&root, &["checkout", "-b", &release_branch])?;
    }

    if has_changes {
        let commit_msg = format!("chore: bump version to {}", tag);
        // First attempt — pre-commit hooks (e.g. cargo fmt) may reformat files
        if git(&root, &["commit", "-m", &commit_msg]).is_err() {
            println!("  Commit failed (likely formatter hook). Re-staging and retrying...");
            git(&root, &["add", "-A"])?;
            git(&root, &["commit", "-m", &commit_msg])?;
        }
    } else {
        println!("  No file changes. Tagging current HEAD.");
    }

    // Tag is created by GitHub Action after PR merges (not here).
    println!("Tag {} will be created when PR merges.", tag);

    // --- Push ---
    if !args.no_push {
        git(&root, &["push", "-u", "origin", &release_branch])?;

        // Create PR via gh
        if Command::new("gh").arg("--version").output().is_ok() {
            println!();
            println!("Creating Pull Request...");

            // Build PR body with changelog content
            // <!-- release-tag:vX.Y.Z --> marker is used by the auto-tag workflow
            let mut pr_body = format!("<!-- release-tag:{} -->\n## Release {}", tag, tag);
            let changelog_path = root.join("CHANGELOG.md");
            if changelog_path.exists() {
                let cl_content = fs::read_to_string(&changelog_path).unwrap_or_default();
                let heading = format!("## [{}]", changelog_version);
                let section = extract_changelog_section(&cl_content, &heading);
                if !section.is_empty() {
                    pr_body.push_str("\n\n");
                    pr_body.push_str(&section);
                }
            }
            if let Some(ref pt) = prev_tag {
                pr_body.push_str(&format!(
                    "\n\n---\n**Full diff:** https://github.com/librefang/librefang/compare/{}...{}",
                    pt, tag
                ));
            }

            let pr_output = Command::new("gh")
                .args([
                    "pr",
                    "create",
                    "--repo",
                    "librefang/librefang",
                    "--title",
                    &format!("release: {}", tag),
                    "--body",
                    &pr_body,
                    "--base",
                    "main",
                    "--head",
                    &release_branch,
                ])
                .current_dir(&root)
                .output()?;

            if pr_output.status.success() {
                let pr_url = String::from_utf8_lossy(&pr_output.stdout)
                    .trim()
                    .to_string();
                println!("-> {}", pr_url);

                // Auto-merge
                let _ = Command::new("gh")
                    .args([
                        "pr",
                        "merge",
                        &pr_url,
                        "--auto",
                        "--squash",
                        "--repo",
                        "librefang/librefang",
                    ])
                    .current_dir(&root)
                    .status();
            } else {
                let stderr = String::from_utf8_lossy(&pr_output.stderr);
                println!("  Warning: PR creation failed: {}", stderr);
            }
        } else {
            println!(
                "gh CLI not found. Create a PR manually for branch '{}'.",
                release_branch
            );
        }
    }

    println!();
    println!(
        "Tag {} {} — release.yml workflow will auto-create the GitHub Release.",
        tag,
        if args.no_push {
            "created locally"
        } else {
            "pushed"
        }
    );
    if !args.no_push {
        println!("Merge the PR to land the version bump on main.");
    }

    Ok(())
}

/// LTS patch release: must be on a release/ branch, auto-increments patch number.
fn run_lts_patch(root: &Path, args: &ReleaseArgs) -> Result<(), Box<dyn std::error::Error>> {
    let branch = current_branch(root)?;
    if !branch.starts_with("release/") {
        return Err(format!(
            "must be on a 'release/*' branch for --lts-patch (currently on '{}')",
            branch
        )
        .into());
    }

    if !is_worktree_clean(root) {
        return Err("working tree is dirty. Commit cherry-picked fixes first.".into());
    }

    // release/2026.3 -> 2026.3
    let series = branch.strip_prefix("release/").unwrap();

    // Find next patch number
    let pattern = format!("v{}.*-lts", series);
    let existing = Command::new("git")
        .args(["tag", "-l", &pattern])
        .current_dir(root)
        .output()
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .filter(|l| !l.trim().is_empty())
                .count()
        })
        .unwrap_or(0);

    let patch = existing; // 0-lts exists → next is 1
    let version = format!("{}.{}-lts", series, patch);
    let tag = format!("v{}", version);

    println!();
    println!("=== LTS Patch Release ===");
    println!("  Branch:  {}", branch);
    println!("  Series:  {}-lts", series);
    println!("  Version: {}", version);
    println!("  Tag:     {}", tag);
    println!();

    if args.dry_run {
        println!("No changes made.");
        return Ok(());
    }

    if !args.no_confirm {
        let confirm = prompt("Release? [Y/n]: ");
        if confirm.starts_with('n') || confirm.starts_with('N') {
            println!("Aborted.");
            return Ok(());
        }
    }

    // Sync version in Cargo.toml
    sync_versions::run(sync_versions::SyncVersionsArgs {
        version: Some(version.clone()),
    })?;

    // Update Cargo.lock
    let _ = Command::new("cargo")
        .args(["update", "--workspace"])
        .current_dir(root)
        .status();

    // Commit version bump if there are changes
    let has_changes = !Command::new("git")
        .args(["diff", "--quiet"])
        .current_dir(root)
        .status()
        .map(|s| s.success())
        .unwrap_or(true);

    if has_changes {
        let _ = Command::new("git")
            .args(["add", "Cargo.toml", "Cargo.lock"])
            .current_dir(root)
            .status();
        let lts_msg = format!("chore: bump to {}", tag);
        if git(root, &["commit", "-m", &lts_msg]).is_err() {
            let _ = Command::new("git")
                .args(["add", "-A"])
                .current_dir(root)
                .status();
            git(root, &["commit", "-m", &lts_msg])?;
        }
    }

    git(root, &["tag", &tag])?;
    println!("Created tag {}", tag);

    if !args.no_push {
        git(root, &["push", "origin", &branch])?;
        git(root, &["push", "origin", &tag])?;
        println!("Pushed {} and {}", branch, tag);
    }

    println!();
    println!("LTS patch {} released.", tag);

    Ok(())
}
