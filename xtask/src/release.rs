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

fn is_worktree_clean(root: &Path) -> Result<bool, Box<dyn std::error::Error>> {
    Ok(git(root, &["status", "--porcelain", "--untracked-files=normal"])?.is_empty())
}

fn git_diff_has_changes(root: &Path, cached: bool) -> Result<bool, Box<dyn std::error::Error>> {
    let mut command = Command::new("git");
    command.arg("diff");
    if cached {
        command.arg("--cached");
    }
    let output = command.arg("--quiet").current_dir(root).output()?;
    match output.status.code() {
        Some(0) => Ok(false),
        Some(1) => Ok(true),
        _ => Err(format!(
            "git diff{} --quiet failed: {}",
            if cached { " --cached" } else { "" },
            String::from_utf8_lossy(&output.stderr).trim()
        )
        .into()),
    }
}

/// Paths the release commit stages, on top of a generated Dev.to article.
///
/// The last entry is a directory, and deliberately so: the fold step
/// (`changelog::collect_fragments`) **deletes** every fragment it consumed, and
/// a deletion has to be staged like any other change. Naming the fragments
/// individually could not do it — `stage_release_files` skips a path that no
/// longer exists, which is precisely the state a consumed fragment is in. The
/// directory itself survives the fold because the `.gitkeep` files do, so
/// `git add -f changelog.d` stages the deletions beneath it (`git add <dir>` has
/// implied `--all` semantics since git 2.0). Without it the release commit would
/// carry the folded bullets while leaving the fragment files on `main`, and the
/// next release would fold the same entries in a second time.
const RELEASE_STAGED_PATHS: &[&str] = &[
    "Cargo.toml",
    "Cargo.lock",
    "CHANGELOG.md",
    "openapi.json",
    "sdk/javascript/package.json",
    "sdk/javascript/index.js",
    "sdk/python/pyproject.toml",
    "sdk/python/setup.py",
    "sdk/python/librefang/librefang_client.py",
    "sdk/rust/Cargo.toml",
    "sdk/rust/README.md",
    "sdk/rust/src/lib.rs",
    "sdk/go/librefang.go",
    "packages/whatsapp-gateway/package.json",
    "crates/librefang-desktop/tauri.conf.json",
    // The release run calls `cargo xtask schema-check gen` just before committing, so any schema surface the version bump touched leaves a rewritten baseline behind.
    // Staging `openapi.json` without its baseline is what left v2026.7.31's `xtask/baselines/openapi.sha256` uncommitted in the working tree — the exact drift the regen step exists to prevent.
    "xtask/baselines",
    changelog::FRAGMENT_DIR,
];

/// Stage the release commit's file set, skipping paths this repo does not have.
///
/// Any staging failure aborts the release before it can create an incomplete
/// commit or PR.
fn stage_release_files(root: &Path) -> Result<(), Box<dyn std::error::Error>> {
    for file in RELEASE_STAGED_PATHS {
        let path = root.join(file);
        if path.exists() {
            git(root, &["add", "-f", file])?;
        }
    }
    Ok(())
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

fn eligible_release_tag(tag: &str, include_prerelease: bool) -> bool {
    let Some(version) = tag.strip_prefix('v') else {
        return false;
    };
    if version.is_empty() || !version.as_bytes()[0].is_ascii_digit() || version.contains("alpha") {
        return false;
    }
    include_prerelease || (!version.contains("-beta") && !version.contains("-rc"))
}

/// Find the latest release tag using Git's version-aware ordering.
fn find_latest_tag(
    root: &Path,
    include_prerelease: bool,
) -> Result<Option<String>, Box<dyn std::error::Error>> {
    let tags = git(root, &["tag", "--sort=-v:refname"])?;
    Ok(tags
        .lines()
        .map(str::trim)
        .find(|tag| eligible_release_tag(tag, include_prerelease))
        .map(str::to_string))
}

fn find_previous_tag(
    root: &Path,
    current: &str,
    include_prerelease: bool,
) -> Result<Option<String>, Box<dyn std::error::Error>> {
    let tags = git(root, &["tag", "--sort=-v:refname"])?;
    let mut found_current = false;
    for tag in tags.lines().map(str::trim) {
        if tag == current {
            found_current = true;
        } else if found_current && eligible_release_tag(tag, include_prerelease) {
            return Ok(Some(tag.to_string()));
        }
    }
    Ok(None)
}

fn tag_exists(root: &Path, tag: &str) -> Result<bool, Box<dyn std::error::Error>> {
    Ok(!git(root, &["tag", "--list", tag])?.is_empty())
}

fn delete_github_release_if_present(
    root: &Path,
    tag: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let output = Command::new("gh")
        .args([
            "release",
            "delete",
            tag,
            "--repo",
            "librefang/librefang",
            "--yes",
        ])
        .current_dir(root)
        .output()?;
    if output.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    let normalized = stderr.to_ascii_lowercase();
    if normalized.contains("release not found") || normalized.contains("http 404") {
        return Ok(());
    }
    Err(format!("gh release delete {} failed: {}", tag, stderr.trim()).into())
}

fn next_lts_patch(tags: &str, series: &str) -> u64 {
    let pattern = Regex::new(&format!(
        r"^v{}\.(\d+)-lts(?:\.\d+)?$",
        regex::escape(series)
    ))
    .expect("escaped LTS series must compile");
    tags.lines()
        .filter_map(|tag| pattern.captures(tag.trim()))
        .filter_map(|captures| captures.get(1)?.as_str().parse::<u64>().ok())
        .max()
        .map_or(0, |patch| patch + 1)
}

fn validate_calver(version: &str) -> Result<(), Box<dyn std::error::Error>> {
    let calver_re =
        Regex::new(r"^[0-9]{4}\.[0-9]{1,2}(\.[0-9]{1,4})?(-(beta|rc)\.?[0-9]+|-lts(\.[0-9]+)?)?$")
            .expect("static CalVer regex must compile");
    if calver_re.is_match(version) {
        Ok(())
    } else {
        Err(format!(
            "'{}' is not a valid CalVer (expected: YYYY.M.DD, YYYY.M-lts, etc.)",
            version
        )
        .into())
    }
}

fn print_dry_run(current: &str, version: &str) {
    let tag = format!("v{}", version);
    let is_lts = version.contains("-lts");
    let is_prerelease = version.contains("-beta") || version.contains("-rc");
    println!();
    println!("=== Dry Run ===");
    println!("  Version: {} -> {}", current, version);
    println!("  Tag:     {}", tag);
    if is_lts {
        let lts_version = version.split("-lts").next().unwrap_or(version);
        let parts: Vec<&str> = lts_version.split('.').collect();
        let branch = if parts.len() >= 2 {
            format!("release/{}.{}", parts[0], parts[1])
        } else {
            format!("release/{}", lts_version)
        };
        println!("  Type:    LTS");
        println!("  Branch:  {} (auto-created by CI)", branch);
    } else if is_prerelease {
        println!("  Type:    pre-release");
    } else {
        println!("  Type:    stable");
    }
    println!();
    println!("No changes made.");
}

fn prompt(message: &str) -> Result<String, Box<dyn std::error::Error>> {
    print!("{}", message);
    io::stdout().flush()?;
    let mut input = String::new();
    if io::stdin().read_line(&mut input)? == 0 {
        return Err("input closed".into());
    }
    Ok(input.trim().to_string())
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
    let after_host = url
        .strip_prefix("git@github.com:")
        .or_else(|| url.strip_prefix("ssh://git@github.com/"))
        .or_else(|| url.strip_prefix("https://github.com/"))
        .or_else(|| url.strip_prefix("http://github.com/"))?;
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

/// GitHub rejects a pull-request body longer than 65,536 characters with `GraphQL: Body is too long`.
/// We cut well under it so the truncation notice, the trailing full-diff link, and any future prefix all fit without recomputing the margin.
const MAX_PR_BODY_CHARS: usize = 60_000;

/// Clamp a PR body to what GitHub will accept, truncating the *tail*.
///
/// Everything load-bearing lives in the prefix: `tag_on_merge` in `release.yml` reads the `<!-- release-tag:vX.Y.Z -->` marker, which `build_release_pr_body` writes at position 0, and the changelog's Highlights section leads the body.
/// So a prefix-preserving cut keeps the release machinery working and drops only the least-important bullets.
///
/// The cut lands on a line boundary, and a fence that was opened but not closed by the kept prefix gets one appended — otherwise the truncation notice and the full-diff link would render inside a code block.
fn truncate_pr_body(body: &str, max_chars: usize) -> String {
    if body.chars().count() <= max_chars {
        return body.to_string();
    }

    const NOTICE: &str = "\n\n_Changelog truncated — GitHub caps a PR body at 65,536 characters. \
                          The full section is in [CHANGELOG.md](https://github.com/librefang/librefang/blob/main/CHANGELOG.md)._";

    // Reserve room for the notice, then cut on a char boundary (byte slicing would panic mid-codepoint on the non-ASCII em dashes these changelog entries are full of).
    let budget = max_chars.saturating_sub(NOTICE.chars().count());
    let mut kept: String = body.chars().take(budget).collect();

    // Back up to the last complete line so we never publish half a bullet.
    if let Some(nl) = kept.rfind('\n') {
        kept.truncate(nl);
    }

    // An odd number of fence lines means the kept prefix opened a block it never closed.
    let fences = kept
        .lines()
        .filter(|l| l.trim_start().starts_with("```"))
        .count();
    if fences % 2 == 1 {
        kept.push_str("\n```");
    }

    kept.push_str(NOTICE);
    kept
}

/// Assemble the release PR body: the `release-tag` marker the auto-tag workflow greps for, this version's changelog section, and a compare link against the previous tag.
fn build_release_pr_body(
    tag: &str,
    changelog_section: &str,
    prev_tag: Option<&str>,
    max_chars: usize,
) -> String {
    let mut body = format!("<!-- release-tag:{} -->\n## Release {}", tag, tag);
    if !changelog_section.is_empty() {
        body.push_str("\n\n");
        body.push_str(changelog_section);
    }
    let suffix = prev_tag
        .map(|pt| {
            format!(
                "\n\n---\n**Full diff:** https://github.com/librefang/librefang/compare/{}...{}",
                pt, tag
            )
        })
        .unwrap_or_default();
    let mut body = truncate_pr_body(&body, max_chars.saturating_sub(suffix.chars().count()));
    body.push_str(&suffix);
    body
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
    let tag = find_latest_tag(root, true)?
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

    // Dispatch ref MUST be `main`, not the tag — GitHub Actions reads
    // the workflow YAML from the dispatched ref. An old tag's commit
    // contains a stale release.yml that does not know about
    // `channel=current`. We pass the tag name via the `tag` input
    // instead, and release.yml's `RELEASE_TAG` env routes it through
    // every `github.ref_name` site.
    let tag_input = format!("tag={}", tag);

    if dry_run {
        println!("Dry run — would dispatch:");
        match &repo {
            Some(r) => println!(
                "  gh workflow run Release --repo {} --ref main -f channel=current -f {}",
                r, tag_input
            ),
            None => println!(
                "  gh workflow run Release --ref main -f channel=current -f {}",
                tag_input
            ),
        }
        return Ok(());
    }

    if !no_confirm {
        let answer = prompt(&format!("Dispatch Release workflow for {}? [y/N]: ", tag))?;
        if answer.to_lowercase() != "y" && answer.to_lowercase() != "yes" {
            return Err("Aborted".into());
        }
    }

    let mut cmd = Command::new("gh");
    cmd.arg("workflow").arg("run").arg("Release");
    if let Some(r) = &repo {
        cmd.arg("--repo").arg(r);
    }
    cmd.args(["--ref", "main", "-f", "channel=current", "-f", &tag_input]);
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
            validate_calver(v)?;
            let current = read_workspace_version(&root).unwrap_or_default();
            print_dry_run(&current, v);
            return Ok(());
        }
    }

    // --- Preflight checks ---
    println!("Preflight checks...");

    let branch = current_branch(&root)?;
    if branch != "main" {
        return Err(format!("must be on 'main' branch (currently on '{}')", branch).into());
    }

    if !is_worktree_clean(&root)? {
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

    if !args.dry_run {
        println!("Pulling latest main...");
        git(&root, &["pull", "--rebase", "origin", "main"])?;
    }

    let current = read_workspace_version(&root)?;
    // Include prerelease tags so rc/beta compare against previous rc/beta
    let mut prev_tag = find_latest_tag(&root, true)?;

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
        let lts_tags = git(&root, &["tag", "-l", &format!("v{}.*-lts*", lts_base)])?;
        let next_lts_patch = next_lts_patch(&lts_tags, &lts_base);

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

            let choice = prompt("Choose [1/2/3/4/5]: ")?;
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
    validate_calver(&version)?;

    let tag = format!("v{}", version);
    let is_prerelease = version.contains("-beta") || version.contains("-rc");
    let is_lts = version.contains("-lts");

    // --- Check if tag already exists ---
    let tag_exists = tag_exists(&root, &tag)?;
    if tag_exists {
        // The latest tag is the one being replaced. Resolve its predecessor
        // before confirmation so both the preview and changelog use the same
        // version-aware, channel-appropriate base.
        prev_tag = find_previous_tag(&root, &tag, is_prerelease)?;
    }

    if args.dry_run {
        print_dry_run(&current, &version);
        return Ok(());
    }

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

        let confirm = prompt("Release? [Y/n]: ")?;
        if confirm.starts_with('n') || confirm.starts_with('N') {
            println!("Aborted.");
            return Ok(());
        }
    }

    // --- Clean up existing tag if re-releasing ---
    // Save prev_tag BEFORE deletion so changelog range stays correct.
    // If we're overwriting the current tag, find the tag before it for changelog.
    if tag_exists {
        println!();
        println!("Cleaning up existing tag '{}'...", tag);
        git(&root, &["tag", "-d", &tag])?;
        let remote_tag_ref = format!("refs/tags/{}", tag);
        if !git(&root, &["ls-remote", "--tags", "origin", &remote_tag_ref])?.is_empty() {
            git(&root, &["push", "origin", "--delete", &tag])?;
        }

        let release_branch_check = format!("chore/bump-version-{}", version);
        if !git(&root, &["branch", "--list", &release_branch_check])?.is_empty() {
            git(&root, &["branch", "-D", &release_branch_check])?;
        }
        // Only delete remote branch if it exists
        if !git(
            &root,
            &["ls-remote", "--heads", "origin", &release_branch_check],
        )?
        .is_empty()
        {
            git(
                &root,
                &["push", "origin", "--delete", &release_branch_check],
            )?;
        }

        // Delete existing GitHub Release so CI can recreate it
        delete_github_release_if_present(&root, &tag)?;
        println!("✓ Cleaned up {}", tag);
    }

    // --- Fold changelog.d fragments into [Unreleased] ---
    // Deliberately ahead of `changelog::run`, which cuts the dated
    // `## [VERSION]` section immediately below `## [Unreleased]`. Folding first
    // means the fragments are already ordinary `[Unreleased]` bullets by the
    // time a release heading exists, so nothing downstream has to learn about
    // them: the `awk` extractors in `.github/workflows/release.yml` and
    // `release-notify.yml` keep slicing exactly the file shape they always did.
    println!();
    println!("Folding changelog.d fragments into [Unreleased]...");
    changelog::collect_fragments(changelog::CollectFragmentsArgs {})?;

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
                    match Command::new("claude")
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
                        Ok(output) if output.status.success() => {
                            let polished = String::from_utf8_lossy(&output.stdout).to_string();
                            if !polished.trim().is_empty() {
                                fs::write(&article, polished)?;
                                println!("  AI polished");
                            } else {
                                println!("  AI polish returned no content, using raw changelog");
                            }
                        }
                        Ok(output) => println!(
                            "  AI polish failed (exit {}), using raw changelog",
                            output.status.code().unwrap_or(-1)
                        ),
                        Err(error) => {
                            println!("  AI polish unavailable ({error}), using raw changelog")
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

    stage_release_files(&root)?;

    // Add article if generated
    if let Some(ref article) = article_path {
        if article.exists() {
            git(&root, &["add", &article.display().to_string()])?;
        }
    }

    // Check if there are staged changes
    let has_changes = git_diff_has_changes(&root, true)?;

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
            let changelog_path = root.join("CHANGELOG.md");
            let section = if changelog_path.exists() {
                let cl_content = fs::read_to_string(&changelog_path).unwrap_or_default();
                let heading = format!("## [{}]", changelog_version);
                extract_changelog_section(&cl_content, &heading)
            } else {
                String::new()
            };
            let pr_body =
                build_release_pr_body(&tag, &section, prev_tag.as_deref(), MAX_PR_BODY_CHARS);

            // Pass the body through a file rather than an argv entry: a release body runs to tens of kilobytes, which is a large fraction of the platform `ARG_MAX`, and markdown in argv has to survive whatever quoting the shell layer applies.
            let body_file = std::env::temp_dir().join(format!("librefang-release-pr-{}.md", tag));
            fs::write(&body_file, &pr_body)?;

            let pr_output = Command::new("gh")
                .args([
                    "pr",
                    "create",
                    "--repo",
                    "librefang/librefang",
                    "--title",
                    &format!("release: {}", tag),
                    "--body-file",
                    &body_file.display().to_string(),
                    "--base",
                    "main",
                    "--head",
                    &release_branch,
                ])
                .current_dir(&root)
                .output()?;
            let _ = fs::remove_file(&body_file);

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

    if !is_worktree_clean(root)? {
        return Err("working tree is dirty. Commit cherry-picked fixes first.".into());
    }

    // release/2026.3 -> 2026.3
    let series = branch.strip_prefix("release/").unwrap();

    // Find the highest existing patch. Counting tags would reuse a version
    // whenever an earlier tag was deleted or the sequence had a gap.
    let pattern = format!("v{}.*-lts*", series);
    let existing = git(root, &["tag", "-l", &pattern])?;
    let patch = next_lts_patch(&existing, series);
    let version = format!("{}.{}-lts", series, patch);
    validate_calver(&version)?;
    let tag = format!("v{}", version);
    if tag_exists(root, &tag)? {
        return Err(format!("refusing to overwrite existing LTS tag {}", tag).into());
    }

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
        let confirm = prompt("Release? [Y/n]: ")?;
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
    let has_changes = git_diff_has_changes(root, false)?;

    if has_changes {
        git(
            root,
            &[
                "add",
                "Cargo.toml",
                "Cargo.lock",
                "openapi.json",
                "sdk/javascript/index.js",
                "sdk/python/librefang/librefang_client.py",
                "sdk/rust/src/lib.rs",
                "sdk/go/librefang.go",
            ],
        )?;
        let lts_msg = format!("chore: bump to {}", tag);
        if git(root, &["commit", "-m", &lts_msg]).is_err() {
            git(root, &["add", "-A"])?;
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

    /// A filesystem-safe stand-in for the current thread's name, for tests that need a scratch directory of their own.
    ///
    /// Under the test harness the thread name is the test's full path — `release::tests::git_diff_change_detection_distinguishes_changes_from_errors`.
    /// `:` is an ordinary character in a POSIX filename but is reserved on Windows, where it separates a drive letter or an NTFS alternate data stream, so joining the raw name into a temp path succeeds on Linux and macOS and fails on Windows with `InvalidFilename` (OS error 123).
    /// That asymmetry is why the two tests using it were red on the Windows lane alone while every other lane stayed green.
    ///
    /// Process id still distinguishes concurrent runs; this only has to keep sibling tests in one process apart.
    fn thread_scratch_slug() -> String {
        scratch_slug(std::thread::current().name().unwrap_or("unnamed"))
    }

    /// The pure half of [`thread_scratch_slug`], split out so the rule can be asserted against inputs the running test does not happen to have.
    fn scratch_slug(raw: &str) -> String {
        raw.chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                    c
                } else {
                    '_'
                }
            })
            .collect()
    }

    /// Windows rejects these outright in a path component; a directory name containing one fails to create with `InvalidFilename` (OS error 123).
    /// POSIX accepts all nine, which is why a scratch path built from an unsanitised thread name is green on Linux and macOS and red on Windows.
    const WINDOWS_RESERVED: [char; 9] = ['<', '>', ':', '"', '/', '\\', '|', '?', '*'];

    /// Guards the fix for the Windows-only `InvalidFilename` failures in
    /// `tag_selection_uses_version_order_and_channel_filtering` and
    /// `git_diff_change_detection_distinguishes_changes_from_errors`.
    ///
    /// Those two build a scratch directory out of the thread name, which under the test harness is the test's full path and therefore contains `::`.
    /// The Windows lane is the only place the bug reproduces, and `changes`-gated PRs that touch `xtask` do not run that lane — so this asserts the rule directly instead, on every platform.
    #[test]
    fn scratch_slug_strips_every_character_windows_reserves() {
        // The exact name that produced OS error 123 on the Windows lane.
        let offender =
            "release::tests::git_diff_change_detection_distinguishes_changes_from_errors";
        assert!(
            offender.contains(':'),
            "the regression premise is that harness thread names carry `::`"
        );
        let slug = scratch_slug(offender);
        for reserved in WINDOWS_RESERVED {
            assert!(
                !slug.contains(reserved),
                "slug {slug:?} still contains {reserved:?}, which Windows rejects in a path component"
            );
        }

        // Every reserved character, not just the `:` we happened to hit.
        let all_reserved: String = WINDOWS_RESERVED.iter().collect();
        let slug = scratch_slug(&all_reserved);
        assert_eq!(
            slug,
            "_".repeat(WINDOWS_RESERVED.len()),
            "each reserved character must map to a single safe placeholder"
        );

        // The slug still has to separate sibling tests sharing one process,
        // or the directories it names would collide instead of failing.
        assert_ne!(
            scratch_slug("release::tests::alpha"),
            scratch_slug("release::tests::beta"),
            "distinct test names must keep distinct scratch directories"
        );

        // And the live thread name — whatever the harness calls this test — has to satisfy the same rule.
        let live = thread_scratch_slug();
        for reserved in WINDOWS_RESERVED {
            assert!(
                !live.contains(reserved),
                "live thread slug {live:?} contains {reserved:?}"
            );
        }
    }

    /// The seam between the fold and the release commit: `collect_fragments_in`
    /// deletes the fragments it consumed, and only `stage_release_files` can get
    /// those deletions into the commit.
    ///
    /// Both halves are unit-tested in isolation and neither one catches this —
    /// the fold's own tests run against a scratch tree with no git in it at all,
    /// so a staging set that omits `changelog.d` would leave the deletions
    /// unstaged, ship a release whose fragments survive on `main`, and fold the
    /// same bullets in again on the next release. Hence a real repository here:
    /// the assertion is about `git add` semantics as much as about the path list.
    #[test]
    fn release_staging_carries_the_fragment_deletions() {
        let root = std::env::temp_dir().join(format!("lf-release-stage-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let hooks = root.join("no-hooks");
        fs::create_dir_all(&hooks).unwrap();

        git(&root, &["init", "-q", "."]).unwrap();
        git(&root, &["config", "user.email", "release@example.invalid"]).unwrap();
        git(&root, &["config", "user.name", "release test"]).unwrap();
        // Neutralise whatever the developer's own git config does to a commit:
        // this repo points `core.hooksPath` at `scripts/hooks`, and a global
        // signing key would make `git commit` prompt or fail outright.
        git(
            &root,
            &["config", "core.hooksPath", &hooks.display().to_string()],
        )
        .unwrap();
        git(&root, &["config", "commit.gpgsign", "false"]).unwrap();

        fs::write(
            root.join("CHANGELOG.md"),
            "# Changelog\n\n## [Unreleased]\n\n### Fixed\n\n- Existing bullet (#1) (@houko)\n",
        )
        .unwrap();
        let section = root.join(changelog::FRAGMENT_DIR).join("fixed");
        fs::create_dir_all(&section).unwrap();
        fs::write(section.join(".gitkeep"), "").unwrap();
        fs::write(section.join("6623-probe.md"), "Probe. (#6623) (@houko)\n").unwrap();
        git(&root, &["add", "CHANGELOG.md", changelog::FRAGMENT_DIR]).unwrap();
        git(&root, &["commit", "-qm", "seed"]).unwrap();

        assert_eq!(changelog::collect_fragments_in(&root).unwrap(), 1);
        stage_release_files(&root).unwrap();
        let staged = git(&root, &["diff", "--cached", "--name-status"]).unwrap();

        // Clean up before asserting so a failure cannot leave the tree behind.
        let _ = fs::remove_dir_all(&root);

        assert!(
            staged.contains("M\tCHANGELOG.md"),
            "folded bullets not staged:\n{staged}"
        );
        assert!(
            staged.contains("D\tchangelog.d/fixed/6623-probe.md"),
            "consumed fragment's deletion not staged, so it would survive the release:\n{staged}"
        );
        assert!(
            !staged.contains("changelog.d/fixed/.gitkeep"),
            ".gitkeep must be untouched, or the section directory stops being tracked:\n{staged}"
        );
    }

    /// `schema-check gen` runs during the release, so a version bump that moves a schema surface rewrites its baseline.
    /// Staging the surface without the baseline is what left v2026.7.31's `openapi.sha256` uncommitted, which would fail `schema-check check` on every PR that followed it onto main.
    #[test]
    fn release_staging_includes_the_schema_baselines() {
        assert!(
            RELEASE_STAGED_PATHS.contains(&"xtask/baselines"),
            "the schema baselines must ship in the release commit alongside openapi.json"
        );
    }

    #[test]
    fn release_staging_includes_python_project_metadata() {
        assert!(
            RELEASE_STAGED_PATHS.contains(&"sdk/python/pyproject.toml"),
            "the version-bearing Python project metadata must ship in the release commit"
        );
    }

    #[test]
    fn release_staging_surfaces_git_add_failures() {
        let root =
            std::env::temp_dir().join(format!("lf-release-stage-error-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("Cargo.toml"), "[workspace]").unwrap();

        let error = stage_release_files(&root).unwrap_err().to_string();
        let _ = fs::remove_dir_all(&root);

        assert!(error.contains("git add -f Cargo.toml failed"), "{error}");
    }

    fn body_of(len: usize) -> String {
        (0..len).map(|i| format!("- bullet {i}\n")).collect()
    }

    #[test]
    fn short_pr_body_is_returned_unchanged() {
        let body = "<!-- release-tag:v1.0.0 -->\n## Release v1.0.0\n\n- one bullet";
        assert_eq!(truncate_pr_body(body, MAX_PR_BODY_CHARS), body);
    }

    /// The `release-tag` marker sits at position 0 and `tag_on_merge` in `release.yml` greps for it, so truncation has to keep the prefix — a tail-preserving cut would silently stop tagging releases.
    #[test]
    fn truncated_pr_body_fits_and_keeps_the_release_tag_marker() {
        let section = body_of(20_000);
        let body = build_release_pr_body("v1.0.0", &section, Some("v0.9.0"), MAX_PR_BODY_CHARS);

        assert!(
            body.chars().count() <= MAX_PR_BODY_CHARS,
            "body exceeded its configured budget"
        );
        assert!(body.starts_with("<!-- release-tag:v1.0.0 -->"));
        assert!(body.contains("_Changelog truncated"));
        assert!(
            body.ends_with("compare/v0.9.0...v1.0.0"),
            "the compare link is appended after truncation, so it always survives"
        );
    }

    /// An unbalanced fence would swallow the truncation notice and the compare link into a code block.
    #[test]
    fn truncation_closes_a_fence_the_kept_prefix_left_open() {
        let mut section = String::from("- intro\n\n```toml\n");
        section.push_str(&body_of(20_000));
        let body = build_release_pr_body("v1.0.0", &section, None, MAX_PR_BODY_CHARS);

        let fences = body
            .lines()
            .filter(|l| l.trim_start().starts_with("```"))
            .count();
        assert_eq!(fences % 2, 0, "unbalanced code fence");
    }

    /// The notice is written as a continued string literal; a stray indent there would render as a markdown code block.
    #[test]
    fn truncation_notice_is_a_single_unindented_line() {
        let body = build_release_pr_body("v1.0.0", &body_of(20_000), None, MAX_PR_BODY_CHARS);
        let notice = body
            .lines()
            .find(|l| l.contains("_Changelog truncated"))
            .expect("truncated body must carry the notice");
        assert!(!notice.starts_with(' '), "indented notice: {notice:?}");
        assert!(notice.contains("characters. The full section is in"));
    }

    /// Changelog entries are full of em dashes, so a byte-wise cut would panic on a char boundary or emit invalid UTF-8.
    #[test]
    fn truncation_cuts_on_char_boundaries() {
        let section: String = (0..20_000)
            .map(|i| format!("- entry {i} — note\n"))
            .collect();
        let body = build_release_pr_body("v1.0.0", &section, None, MAX_PR_BODY_CHARS);
        assert!(body.chars().count() <= MAX_PR_BODY_CHARS);
        assert!(body.contains('—'), "the em dashes should still be intact");
    }

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

    #[test]
    fn calver_validation_is_shared_by_dry_run_and_real_release() {
        assert!(validate_calver("2026.5.4-beta.1").is_ok());
        assert!(validate_calver("2026.5.4-beta").is_err());
        assert!(validate_calver("v2026.5.4").is_err());
        assert!(validate_calver("foo.0-lts").is_err());
    }

    #[test]
    fn lts_patch_advances_past_the_highest_existing_patch() {
        let tags = "v2026.5.0-lts\nv2026.5.2-lts\nv2026.5.7-lts\ninvalid";
        assert_eq!(next_lts_patch(tags, "2026.5"), 8);
        assert_eq!(next_lts_patch("", "2026.5"), 0);
    }

    #[test]
    fn stable_tag_filter_excludes_all_prerelease_channels() {
        assert!(eligible_release_tag("v2026.5.4", false));
        assert!(eligible_release_tag("v2026.5.4-lts", false));
        assert!(!eligible_release_tag("v2026.5.4-beta.1", false));
        assert!(!eligible_release_tag("v2026.5.4-rc.1", false));
        assert!(!eligible_release_tag("v2026.5.4-alpha.1", true));
        assert!(eligible_release_tag("v2026.5.4-beta.1", true));
    }

    #[test]
    fn tag_selection_uses_version_order_and_channel_filtering() {
        let root = std::env::temp_dir().join(format!(
            "lf-release-tags-{}-{}",
            std::process::id(),
            thread_scratch_slug()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        git(&root, &["init", "-q", "."]).unwrap();
        git(&root, &["config", "user.email", "release@example.invalid"]).unwrap();
        git(&root, &["config", "user.name", "release test"]).unwrap();
        fs::write(root.join("seed"), "seed").unwrap();
        git(&root, &["add", "seed"]).unwrap();
        git(&root, &["commit", "-qm", "seed"]).unwrap();
        for tag in [
            "v2026.7.9",
            "v2026.7.10",
            "v2026.7.10-alpha.9",
            "v2026.7.10-beta.2",
            "v2026.7.10-rc.1",
        ] {
            git(&root, &["tag", tag]).unwrap();
        }

        assert_eq!(
            find_latest_tag(&root, true).unwrap().as_deref(),
            Some("v2026.7.10-rc.1")
        );
        assert_eq!(
            find_latest_tag(&root, false).unwrap().as_deref(),
            Some("v2026.7.10")
        );
        assert_eq!(
            find_previous_tag(&root, "v2026.7.10", false)
                .unwrap()
                .as_deref(),
            Some("v2026.7.9")
        );
        assert!(tag_exists(&root, "v2026.7.10").unwrap());
        assert!(!tag_exists(&root, "v2026.7.11").unwrap());

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn worktree_cleanliness_includes_untracked_files() {
        let root = std::env::temp_dir().join(format!("lf-release-clean-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        git(&root, &["init", "-q", "."]).unwrap();

        assert!(is_worktree_clean(&root).unwrap());
        fs::write(root.join("untracked.tmp"), "stray release artifact").unwrap();
        assert!(!is_worktree_clean(&root).unwrap());

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn git_diff_change_detection_distinguishes_changes_from_errors() {
        let root = std::env::temp_dir().join(format!(
            "lf-release-diff-{}-{}",
            std::process::id(),
            thread_scratch_slug()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        git(&root, &["init", "-q", "."]).unwrap();
        git(&root, &["config", "user.email", "release@example.invalid"]).unwrap();
        git(&root, &["config", "user.name", "release test"]).unwrap();
        fs::write(root.join("seed"), "before").unwrap();
        git(&root, &["add", "seed"]).unwrap();
        git(&root, &["commit", "-qm", "seed"]).unwrap();

        assert!(!git_diff_has_changes(&root, false).unwrap());
        assert!(!git_diff_has_changes(&root, true).unwrap());

        fs::write(root.join("seed"), "after").unwrap();
        assert!(git_diff_has_changes(&root, false).unwrap());
        assert!(!git_diff_has_changes(&root, true).unwrap());

        git(&root, &["add", "seed"]).unwrap();
        assert!(!git_diff_has_changes(&root, false).unwrap());
        assert!(git_diff_has_changes(&root, true).unwrap());

        let non_repository = root.with_extension("not-a-repository");
        let _ = fs::remove_dir_all(&non_repository);
        fs::create_dir_all(&non_repository).unwrap();
        assert!(git_diff_has_changes(&non_repository, false).is_err());

        let _ = fs::remove_dir_all(&root);
        let _ = fs::remove_dir_all(&non_repository);
    }
}
