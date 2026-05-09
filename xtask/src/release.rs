use crate::changelog;
use crate::common::repo_root;
use crate::sync_versions;
use clap::{Parser, ValueEnum};
use regex::Regex;
use std::fs;
use std::io::{self, Write as _};
use std::path::Path;
use std::process::Command;

/// Release channel for non-interactive version pick. Mirrors the 1/2/3/4/5
/// prompt entries in the interactive flow so `just release` and
/// `gh workflow run Release --input channel=…` pick versions identically.
///
/// `Current` is special — it does not bump the version or open a PR.
/// Instead it dispatches the existing release run for the latest tag
/// with `channel=current`, which causes `release.yml` to force-sync
/// the tag to `main` HEAD and re-publish every artifact from `main`'s
/// current code. Use it when a release tag failed on a code bug, the
/// fix has landed on `main`, and the same version number should be
/// re-published with the fix included.
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum Channel {
    Stable,
    Beta,
    Rc,
    Lts,
    Current,
}

#[derive(Parser, Debug)]
pub struct ReleaseArgs {
    /// Explicit version (e.g. 2026.5.4 or 2026.5.4-beta.1).
    /// Pre-release tags follow SemVer (`-beta.N` / `-rc.N`, with the dot)
    /// per #3310; the legacy `-betaN` form is still parsed for backward
    /// compatibility but new tags should use the canonical form.
    #[arg(long)]
    pub version: Option<String>,

    /// Non-interactive channel pick. When set, the 1/2/3/4 prompt is
    /// replaced with the corresponding auto-computed version. Mutually
    /// exclusive with `--version`.
    #[arg(long, value_enum, conflicts_with = "version")]
    pub channel: Option<Channel>,

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

/// Best-effort GitHub `<owner>/<repo>` lookup from the `origin` remote
/// URL. Returns `None` when there is no `origin`, the URL doesn't point
/// at github.com, or it doesn't parse into exactly one `<owner>/<repo>`
/// pair.
///
/// Why this exists: `gh workflow run` requires either a configured
/// default repo (`gh repo set-default …`) or an explicit `--repo`
/// argument. On a fresh clone the default isn't set, so the dispatch
/// fails with "No default remote repository has been set". Inferring
/// from `origin` lets `cargo xtask release --channel current` work
/// without that prerequisite.
fn infer_gh_repo(root: &Path) -> Option<String> {
    let out = Command::new("git")
        .args(["remote", "get-url", "origin"])
        .current_dir(root)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let url = String::from_utf8_lossy(&out.stdout).trim().to_string();
    parse_github_owner_repo(&url)
}

fn parse_github_owner_repo(url: &str) -> Option<String> {
    let url = url.strip_suffix(".git").unwrap_or(url);
    let after_host = if let Some(rest) = url.strip_prefix("git@github.com:") {
        rest
    } else if let Some(rest) = url.strip_prefix("ssh://git@github.com/") {
        rest
    } else if let Some(rest) = url.strip_prefix("https://github.com/") {
        rest
    } else if let Some(rest) = url.strip_prefix("http://github.com/") {
        rest
    } else {
        return None;
    };
    let mut parts = after_host.split('/');
    let owner = parts.next()?;
    let repo = parts.next()?;
    if owner.is_empty() || repo.is_empty() {
        return None;
    }
    if parts.next().is_some() {
        return None;
    }
    Some(format!("{}/{}", owner, repo))
}

/// Best-effort `git rev-parse --short <revspec>`. Returns `None` when
/// git fails or the revspec does not resolve in the local repo.
fn git_short_sha(root: &Path, revspec: &str) -> Option<String> {
    let out = Command::new("git")
        .args(["rev-parse", "--short", revspec])
        .current_dir(root)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

fn compute_calver() -> String {
    let now = chrono::Local::now();
    format!(
        "{}.{}.{}",
        now.format("%Y"),
        now.format("%-m"),
        now.format("%-d"),
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

/// Dispatch the existing release run for the latest tag with
/// `channel=current`. `release.yml` then force-syncs the tag to
/// `main` HEAD and re-publishes every artifact from main's code.
///
/// We do not change Cargo.toml, do not open a PR, and do not push a
/// new tag locally — every mutating step happens server-side via the
/// dispatched workflow.
fn run_current(
    root: &Path,
    no_confirm: bool,
    dry_run: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let tag = find_latest_tag(root, true)
        .ok_or("no existing release tag found; nothing to re-publish")?;

    // Resolve the tag's current commit and main HEAD locally so the
    // operator sees, before confirming, exactly which commit pointer
    // is about to move. Both lookups are best-effort: if the tag is
    // not present locally (fresh clone, shallow fetch) we fall back
    // to "(unknown)" rather than blocking — the workflow itself
    // re-resolves both in the runner.
    let tag_sha = git_short_sha(root, &format!("refs/tags/{}^{{commit}}", tag))
        .unwrap_or_else(|| "(unknown)".to_string());
    let main_sha = git_short_sha(root, "refs/heads/main")
        .or_else(|| git_short_sha(root, "origin/main"))
        .unwrap_or_else(|| "(unknown)".to_string());
    let already_synced = tag_sha != "(unknown)" && main_sha != "(unknown)" && tag_sha == main_sha;

    println!("=== channel=current: re-publish latest tag ===");
    println!("  Tag:        {}", tag);
    println!("  Tag → SHA:  {}", tag_sha);
    println!("  main HEAD:  {}", main_sha);
    if already_synced {
        println!("  Status:     tag already at main HEAD — sync_tag_to_main will be a no-op");
    } else {
        println!(
            "  Status:     tag will be force-pushed: {} → {}",
            tag_sha, main_sha
        );
    }
    println!();
    println!("Effect: release.yml's `sync_tag_to_main` job will force-update");
    println!(
        "`{}` to main HEAD, then publish every artifact from main's",
        tag
    );
    println!(
        "current code. The previous `{}` GitHub Release artifacts will be",
        tag
    );
    println!("clobbered; npm / PyPI / crates.io entries will be skipped where the");
    println!("version already exists (idempotent publish steps).");
    println!();

    // Infer `<owner>/<repo>` from the `origin` remote so the dispatch
    // works on fresh clones where `gh repo set-default` hasn't been
    // run. Falls back to gh's own default-resolution when origin is
    // not a GitHub URL we recognize.
    let repo = infer_gh_repo(root);

    if dry_run {
        println!("Dry run — would dispatch:");
        match &repo {
            Some(r) => println!(
                "  gh workflow run Release --repo {} --ref {} -f channel=current",
                r, tag
            ),
            None => println!("  gh workflow run Release --ref {} -f channel=current", tag),
        }
        return Ok(());
    }

    if !no_confirm {
        let answer = prompt(&format!("Dispatch Release workflow for {}? [y/N]: ", tag));
        if answer.to_lowercase() != "y" && answer.to_lowercase() != "yes" {
            return Err("Aborted".into());
        }
    }

    let mut cmd = Command::new("gh");
    cmd.arg("workflow").arg("run").arg("Release");
    if let Some(r) = &repo {
        cmd.arg("--repo").arg(r);
    }
    cmd.args(["--ref", &tag, "-f", "channel=current"]);
    let status = cmd.current_dir(root).status()?;
    if !status.success() {
        let hint = if repo.is_none() {
            " — could not infer GitHub `<owner>/<repo>` from `origin` either; \
                pin one with `gh repo set-default <owner>/<repo>` and retry"
        } else {
            ""
        };
        return Err(format!(
            "gh workflow run failed (exit {}); is `gh` authenticated and the \
             repo accessible?{}",
            status.code().unwrap_or(-1),
            hint
        )
        .into());
    }

    println!();
    println!("✓ Dispatched. Watch with:");
    println!("  gh run watch (or open the Actions tab)");
    Ok(())
}

pub fn run(args: ReleaseArgs) -> Result<(), Box<dyn std::error::Error>> {
    let root = repo_root();

    // --- channel=current shortcut: dispatch only, no version bump / PR ---
    if args.channel == Some(Channel::Current) {
        return run_current(&root, args.no_confirm, args.dry_run);
    }

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
    let mut prev_tag = find_latest_tag(&root, true);

    // --- Determine version ---
    let version = if let Some(v) = args.version {
        v
    } else {
        let base_version = compute_calver();

        // Pre-compute every candidate up front so both the interactive
        // prompt and the `--channel` non-interactive path pick from the
        // same numbers. Previously this was only done inside the prompt
        // branch, which meant `--no-confirm` silently defaulted to
        // stable and skipped rc/beta/lts entirely.
        // Optional dot in `-beta.?N` accepts both the canonical `-beta.N`
        // (#3310) and the legacy `-betaN` so an in-flight Cargo.toml from
        // either era still resolves correctly.
        let current_beta_num = Regex::new(r"-beta\.?(\d+)$")
            .unwrap()
            .captures(&current)
            .and_then(|c| c.get(1)?.as_str().parse::<u64>().ok())
            .unwrap_or(0);
        let current_rc_num = Regex::new(r"-rc\.?(\d+)$")
            .unwrap()
            .captures(&current)
            .and_then(|c| c.get(1)?.as_str().parse::<u64>().ok())
            .unwrap_or(0);

        let beta_re = Regex::new(&format!(
            r"^v{}-beta\.?(\d+)$",
            regex::escape(&base_version)
        ))
        .unwrap();
        let max_beta_tag = Command::new("git")
            .args(["tag", "-l", &format!("v{}-beta*", base_version)])
            .current_dir(&root)
            .output()
            .map(|o| {
                String::from_utf8_lossy(&o.stdout)
                    .lines()
                    .filter_map(|l| beta_re.captures(l.trim()))
                    .filter_map(|cap| cap.get(1)?.as_str().parse::<u64>().ok())
                    .max()
                    .unwrap_or(0)
            })
            .unwrap_or(0);
        let next_beta = max_beta_tag.max(current_beta_num) + 1;

        let rc_re =
            Regex::new(&format!(r"^v{}-rc\.?(\d+)$", regex::escape(&base_version))).unwrap();
        let max_rc_tag = Command::new("git")
            .args(["tag", "-l", &format!("v{}-rc*", base_version)])
            .current_dir(&root)
            .output()
            .map(|o| {
                String::from_utf8_lossy(&o.stdout)
                    .lines()
                    .filter_map(|l| rc_re.captures(l.trim()))
                    .filter_map(|cap| cap.get(1)?.as_str().parse::<u64>().ok())
                    .max()
                    .unwrap_or(0)
            })
            .unwrap_or(0);
        let next_rc = max_rc_tag.max(current_rc_num) + 1;

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

        // Canonical pre-release form is `-beta.N` / `-rc.N` (SemVer
        // pre-release identifier), unified in #3310. The dot is required
        // for npm dist-tag automation and node-semver / cargo parsing —
        // historical `-betaN` (no dot) tags remain readable but no new
        // ones are minted from this generator.
        let version_for = |ch: Channel| -> String {
            match ch {
                Channel::Stable => base_version.clone(),
                Channel::Beta => format!("{}-beta.{}", base_version, next_beta),
                Channel::Rc => format!("{}-rc.{}", base_version, next_rc),
                Channel::Lts => format!("{}.{}-lts", lts_base, next_lts_patch),
                // Current bypasses the version pick entirely — it's
                // handled by the early `run_current` branch above and
                // by the `"5"` arm of the interactive prompt below.
                Channel::Current => unreachable!(
                    "Channel::Current must be intercepted before version_for() — \
                     see the early-return in run() and the \"5\" arm of the prompt"
                ),
            }
        };

        if let Some(ch) = args.channel {
            version_for(ch)
        } else if args.no_confirm {
            // Default to stable when the caller asked to skip prompts
            // without committing to a channel.
            base_version
        } else {
            println!();
            println!(
                "Current version: {} (tag: {})",
                current,
                prev_tag.as_deref().unwrap_or("none")
            );
            println!();
            println!("  1) stable  -> {}", version_for(Channel::Stable));
            println!("  2) beta    -> {}", version_for(Channel::Beta));
            println!("  3) rc      -> {}", version_for(Channel::Rc));
            println!("  4) lts     -> {}", version_for(Channel::Lts));
            println!(
                "  5) current -> re-publish latest tag ({}) with main HEAD code",
                prev_tag.as_deref().unwrap_or("none")
            );
            println!();

            let choice = prompt("Choose [1/2/3/4/5]: ");
            match choice.as_str() {
                "1" => version_for(Channel::Stable),
                "2" => version_for(Channel::Beta),
                "3" => version_for(Channel::Rc),
                "4" => version_for(Channel::Lts),
                "5" => return run_current(&root, args.no_confirm, args.dry_run),
                _ => return Err("Invalid choice".into()),
            }
        }
    };

    // Validate CalVer early, before using version in git tags/branches.
    // `(beta|rc)\.?[0-9]+` accepts both the canonical `-beta.N` (#3310) and
    // the legacy `-betaN`; the generator only emits the canonical form, but
    // operators may still pass `--version 2026.5.4-beta1` against an old
    // workflow snippet and we don't want that to abort the release.
    let calver_re =
        Regex::new(r"^[0-9]{4}\.[0-9]{1,2}(\.[0-9]{1,4})?(-(beta|rc)\.?[0-9]+|-lts(\.[0-9]+)?)?$")
            .unwrap();
    if !calver_re.is_match(&version) {
        return Err(format!(
            "'{}' is not a valid CalVer (expected: YYYY.M.DD, YYYY.M-lts, etc.)",
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
                "  Review:  https://github.com/librefang/librefang/compare/{}...{}",
                pt, tag
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
    // Save prev_tag BEFORE deletion so changelog range stays correct.
    // If we're overwriting the current tag, find the tag before it for changelog.
    if tag_exists {
        // Find the tag that precedes the one we're about to delete
        let output = Command::new("git")
            .args(["tag", "--sort=-creatordate"])
            .current_dir(&root)
            .output()
            .ok();
        if let Some(output) = output {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let mut found_current = false;
            for line in stdout.lines() {
                let t = line.trim();
                if t == tag {
                    found_current = true;
                    continue;
                }
                if found_current
                    && t.starts_with('v')
                    && t.len() > 1
                    && t.as_bytes()[1].is_ascii_digit()
                    && !t.contains("alpha")
                {
                    prev_tag = Some(t.to_string());
                    break;
                }
            }
        }

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
        // Only delete remote branch if it exists
        let remote_branch_exists = Command::new("git")
            .args(["ls-remote", "--heads", "origin", &release_branch_check])
            .current_dir(&root)
            .output()
            .map(|o| !String::from_utf8_lossy(&o.stdout).trim().is_empty())
            .unwrap_or(false);
        if remote_branch_exists {
            let _ = Command::new("git")
                .args(["push", "origin", "--delete", &release_branch_check])
                .current_dir(&root)
                .status();
        }

        // Delete existing GitHub Release so CI can recreate it
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
        println!("✓ Cleaned up {}", tag);
    }

    // --- Generate changelog ---
    println!();
    println!("Generating changelog...");
    let changelog_version = version.split('-').next().unwrap_or(&version).to_string();
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

    // --- Regenerate OpenAPI spec + SDK clients ---
    // openapi.json and the generated SDK source files (sdk/go/librefang.go,
    // sdk/rust/src/lib.rs, sdk/javascript/index.js, sdk/python/librefang/
    // librefang_client.py) embed the workspace version string. If we skip
    // this, CI's "OpenAPI Drift" check regenerates them in the runner and
    // fails because the version embedded in the checked-in artifacts no
    // longer matches the just-bumped Cargo.toml.
    println!();
    println!("Regenerating OpenAPI spec and SDK clients...");
    let openapi_status = Command::new("cargo")
        .args(["xtask", "codegen", "--openapi"])
        .current_dir(&root)
        .status()?;
    if !openapi_status.success() {
        return Err("cargo xtask codegen --openapi failed".into());
    }
    let sdk_status = Command::new("python3")
        .args(["scripts/codegen-sdks.py"])
        .current_dir(&root)
        .status()?;
    if !sdk_status.success() {
        return Err("python3 scripts/codegen-sdks.py failed".into());
    }
    // Refresh schema sha256 baselines (xtask/baselines/openapi.sha256 etc.)
    // so the openapi-drift CI gate stays green. Skipping this is what made
    // every prior version bump land a follow-up baseline-only commit (#4690).
    let baseline_status = Command::new("cargo")
        .args(["xtask", "schema-check", "gen"])
        .current_dir(&root)
        .status()?;
    if !baseline_status.success() {
        return Err("cargo xtask schema-check gen failed".into());
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

    // Dashboard is built by CI (dashboard-build.yml), not embedded in release commits.

    // --- Git add + commit + tag ---
    println!();
    println!("Committing version bump...");

    let files_to_add = [
        "Cargo.toml",
        "Cargo.lock",
        "CHANGELOG.md",
        "openapi.json",
        "sdk/javascript/package.json",
        "sdk/javascript/index.js",
        "sdk/python/setup.py",
        "sdk/python/librefang/librefang_client.py",
        "sdk/rust/Cargo.toml",
        "sdk/rust/README.md",
        "sdk/rust/src/lib.rs",
        "sdk/go/librefang.go",
        "packages/whatsapp-gateway/package.json",
        "crates/librefang-desktop/tauri.conf.json",
    ];

    for file in &files_to_add {
        let path = root.join(file);
        if path.exists() {
            let _ = Command::new("git")
                .args(["add", "-f", file])
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

    // Regenerate OpenAPI spec + SDK clients so the embedded version matches
    // (same reason as the main release path — the OpenAPI Drift CI check
    // would otherwise fail on this LTS bump commit).
    let openapi_status = Command::new("cargo")
        .args(["xtask", "codegen", "--openapi"])
        .current_dir(root)
        .status()?;
    if !openapi_status.success() {
        return Err("cargo xtask codegen --openapi failed".into());
    }
    let sdk_status = Command::new("python3")
        .args(["scripts/codegen-sdks.py"])
        .current_dir(root)
        .status()?;
    if !sdk_status.success() {
        return Err("python3 scripts/codegen-sdks.py failed".into());
    }
    // Refresh schema sha256 baselines (xtask/baselines/openapi.sha256 etc.)
    // so the openapi-drift CI gate stays green. Skipping this is what made
    // every prior LTS bump land a follow-up baseline-only commit (#4690).
    let baseline_status = Command::new("cargo")
        .args(["xtask", "schema-check", "gen"])
        .current_dir(root)
        .status()?;
    if !baseline_status.success() {
        return Err("cargo xtask schema-check gen failed".into());
    }

    // Commit version bump if there are changes
    let has_changes = !Command::new("git")
        .args(["diff", "--quiet"])
        .current_dir(root)
        .status()
        .map(|s| s.success())
        .unwrap_or(true);

    if has_changes {
        let _ = Command::new("git")
            .args([
                "add",
                "Cargo.toml",
                "Cargo.lock",
                "openapi.json",
                "sdk/javascript/index.js",
                "sdk/python/librefang/librefang_client.py",
                "sdk/rust/src/lib.rs",
                "sdk/go/librefang.go",
            ])
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Reproduces the closure in `run()` so the generator's output format
    /// is locked down by a unit test. Issue #3310 unified pre-release tags
    /// to SemVer `-beta.N` / `-rc.N`; this test guards against accidental
    /// regression to the old `-betaN` form.
    fn version_for(
        base_version: &str,
        lts_base: &str,
        next_beta: u64,
        next_rc: u64,
        next_lts_patch: u64,
        ch: Channel,
    ) -> String {
        match ch {
            Channel::Stable => base_version.to_string(),
            Channel::Beta => format!("{}-beta.{}", base_version, next_beta),
            Channel::Rc => format!("{}-rc.{}", base_version, next_rc),
            Channel::Lts => format!("{}.{}-lts", lts_base, next_lts_patch),
            Channel::Current => {
                unreachable!("test fixture should never call with Channel::Current")
            }
        }
    }

    #[test]
    fn generator_emits_canonical_beta_with_dot() {
        let v = version_for("2026.5.4", "2026.5", 1, 0, 0, Channel::Beta);
        assert_eq!(v, "2026.5.4-beta.1");
    }

    #[test]
    fn parse_github_owner_repo_handles_https() {
        assert_eq!(
            parse_github_owner_repo("https://github.com/librefang/librefang.git"),
            Some("librefang/librefang".to_string())
        );
        assert_eq!(
            parse_github_owner_repo("https://github.com/librefang/librefang"),
            Some("librefang/librefang".to_string())
        );
    }

    #[test]
    fn parse_github_owner_repo_handles_ssh() {
        assert_eq!(
            parse_github_owner_repo("git@github.com:librefang/librefang.git"),
            Some("librefang/librefang".to_string())
        );
        assert_eq!(
            parse_github_owner_repo("ssh://git@github.com/librefang/librefang.git"),
            Some("librefang/librefang".to_string())
        );
    }

    #[test]
    fn parse_github_owner_repo_rejects_non_github() {
        assert_eq!(
            parse_github_owner_repo("https://gitlab.com/librefang/librefang.git"),
            None
        );
        assert_eq!(parse_github_owner_repo("not a url"), None);
        assert_eq!(parse_github_owner_repo(""), None);
    }

    #[test]
    fn parse_github_owner_repo_rejects_subpath() {
        // Defensive: don't accept a URL with extra path components,
        // because feeding an extra segment to `gh --repo` would error
        // confusingly. Better to fall back to gh's own default.
        assert_eq!(
            parse_github_owner_repo("https://github.com/librefang/librefang/tree/main"),
            None
        );
    }

    #[test]
    fn generator_emits_canonical_rc_with_dot() {
        let v = version_for("2026.5.4", "2026.5", 0, 3, 0, Channel::Rc);
        assert_eq!(v, "2026.5.4-rc.3");
    }

    #[test]
    fn generator_does_not_zero_pad() {
        // Single-digit month/day stay unpadded — npm-friendly SemVer form.
        let v = version_for("2026.5.4", "2026.5", 7, 0, 0, Channel::Beta);
        assert_eq!(v, "2026.5.4-beta.7");
        assert!(!v.contains(".05."));
        assert!(!v.contains(".04-"));
    }

    #[test]
    fn parser_accepts_canonical_beta_dot_form() {
        // current_beta_num regex from the run() body.
        let re = Regex::new(r"-beta\.?(\d+)$").unwrap();
        assert_eq!(
            re.captures("2026.5.4-beta.1")
                .and_then(|c| c.get(1).map(|m| m.as_str().to_string())),
            Some("1".to_string())
        );
    }

    #[test]
    fn parser_accepts_legacy_beta_no_dot() {
        // Historical tags like v2026.5.2-beta8 must still parse so
        // `--channel beta` next-number computation lands on beta.9
        // (or higher) rather than starting over at beta.1.
        let re = Regex::new(r"-beta\.?(\d+)$").unwrap();
        assert_eq!(
            re.captures("2026.5.2-beta8")
                .and_then(|c| c.get(1).map(|m| m.as_str().to_string())),
            Some("8".to_string())
        );
    }

    #[test]
    fn parser_accepts_legacy_zero_padded_day() {
        // Very old `vYYYY.M.DD-betaN` zero-pad form. The day still has to
        // round-trip through the calver_re; we only assert the suffix
        // parser here since day digits are not zero-checked.
        let re = Regex::new(r"-beta\.?(\d+)$").unwrap();
        assert_eq!(
            re.captures("2026.03.21-beta1")
                .and_then(|c| c.get(1).map(|m| m.as_str().to_string())),
            Some("1".to_string())
        );
    }

    #[test]
    fn migration_path_parse_then_regenerate_uses_dot_form() {
        // Read a legacy tag, extract the beta number, then regenerate
        // using the new canonical format. Confirms the migration path:
        // historical `-beta8` → next is `-beta.9`.
        let legacy = "2026.5.2-beta8";
        let re = Regex::new(r"-beta\.?(\d+)$").unwrap();
        let n: u64 = re
            .captures(legacy)
            .and_then(|c| c.get(1)?.as_str().parse::<u64>().ok())
            .unwrap();
        assert_eq!(n, 8);
        let next = version_for("2026.5.2", "2026.5", n + 1, 0, 0, Channel::Beta);
        assert_eq!(next, "2026.5.2-beta.9");
    }

    #[test]
    fn calver_re_accepts_both_forms() {
        let re = Regex::new(
            r"^[0-9]{4}\.[0-9]{1,2}(\.[0-9]{1,4})?(-(beta|rc)\.?[0-9]+|-lts(\.[0-9]+)?)?$",
        )
        .unwrap();
        assert!(re.is_match("2026.5.4-beta.1"));
        assert!(re.is_match("2026.5.4-rc.3"));
        assert!(re.is_match("2026.5.4-beta1"));
        assert!(re.is_match("2026.5.4-rc1"));
        assert!(re.is_match("2026.5.4"));
        assert!(re.is_match("2026.5.0-lts.1"));
        assert!(!re.is_match("2026.5.4-beta"));
        assert!(!re.is_match("v2026.5.4-beta.1"));
    }
}
