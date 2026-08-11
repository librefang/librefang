use crate::common::repo_root;
use clap::Parser;
#[cfg(windows)]
use process_wrap::std::JobObject;
#[cfg(unix)]
use process_wrap::std::ProcessGroup;
use process_wrap::std::{ChildWrapper, CommandWrap};
use regex::Regex;
use std::collections::BTreeSet;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::{mpsc, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

#[derive(Parser, Debug)]
pub struct ChangelogArgs {
    /// Version for the changelog entry (e.g. 2026.3.2114)
    pub version: String,

    /// Base tag to compare from (default: latest non-prerelease tag)
    pub base_tag: Option<String>,
}

#[derive(Parser, Debug)]
pub struct CollectFragmentsArgs {}

/// Directory under `changelog.d/` → the `### ` heading its fragments render under.
///
/// The order is the canonical order in which a *missing* subsection is created
/// inside `## [Unreleased]`. It is not alphabetical and not Keep-a-Changelog's
/// order: it is the order the repo already uses, which is both the live order of
/// the `### ` headings under `## [Unreleased]` and the order of `CATEGORY_ORDER`
/// below for the four categories the two share.
const FRAGMENT_SECTIONS: &[(&str, &str)] = &[
    ("added", "Added"),
    ("fixed", "Fixed"),
    ("changed", "Changed"),
    ("security", "Security"),
    ("documentation", "Documentation"),
];

/// The fragment directory, shared with `release::RELEASE_STAGED_PATHS` so the
/// path the fold deletes from and the path the release commit stages are one
/// definition rather than two string literals that can drift apart.
pub(crate) const FRAGMENT_DIR: &str = "changelog.d";

/// The heading of the curated section contributors append to.
const UNRELEASED_HEADING: &str = "## [Unreleased]";
const GIT_TIMEOUT: Duration = Duration::from_secs(30);
const GH_TIMEOUT: Duration = Duration::from_secs(30);
const CLAUDE_TIMEOUT: Duration = Duration::from_secs(120);
const COMMAND_POLL_INTERVAL: Duration = Duration::from_millis(10);
const COMMAND_KILL_GRACE: Duration = Duration::from_millis(250);

fn spawn_output_reader<R>(mut reader: R) -> mpsc::Receiver<Result<Vec<u8>, String>>
where
    R: Read + Send + 'static,
{
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let mut bytes = Vec::new();
        let result = reader
            .read_to_end(&mut bytes)
            .map(|_| bytes)
            .map_err(|error| error.to_string());
        let _ = sender.send(result);
    });
    receiver
}

fn take_reader_result(
    receiver: &mpsc::Receiver<Result<Vec<u8>, String>>,
    output: &mut Option<Vec<u8>>,
    stream_name: &str,
) -> Result<(), String> {
    if output.is_some() {
        return Ok(());
    }
    match receiver.try_recv() {
        Ok(Ok(bytes)) => {
            *output = Some(bytes);
            Ok(())
        }
        Ok(Err(error)) => Err(format!("failed to read command {stream_name}: {error}")),
        Err(mpsc::TryRecvError::Empty) => Ok(()),
        Err(mpsc::TryRecvError::Disconnected) => Err(format!(
            "command {stream_name} reader stopped before returning output"
        )),
    }
}

fn fail_and_terminate(child: &mut dyn ChildWrapper, description: &str, error: String) -> String {
    match child.start_kill() {
        Ok(()) => error,
        Err(kill_error) => format!(
            "{error}; additionally failed to terminate the process tree for {description}: {kill_error}"
        ),
    }
}

fn take_completed_output(
    status: &mut Option<std::process::ExitStatus>,
    stdout: &mut Option<Vec<u8>>,
    stderr: &mut Option<Vec<u8>>,
) -> Option<Output> {
    if status.is_none() || stdout.is_none() || stderr.is_none() {
        return None;
    }
    Some(Output {
        status: status.take().expect("status was checked above"),
        stdout: stdout.take().expect("stdout was checked above"),
        stderr: stderr.take().expect("stderr was checked above"),
    })
}

fn command_output_with_timeout(command: &mut Command, timeout: Duration) -> Result<Output, String> {
    let description = format!("{command:?}");
    let raw_command = std::mem::replace(command, Command::new(""));
    let mut command = CommandWrap::from(raw_command);
    #[cfg(unix)]
    command.wrap(ProcessGroup::leader());
    #[cfg(windows)]
    command.wrap(JobObject);
    command
        .command_mut()
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command
        .spawn()
        .map_err(|error| format!("failed to spawn {description}: {error}"))?;
    let stdout_reader = spawn_output_reader(
        child
            .stdout()
            .take()
            .expect("stdout was configured as piped"),
    );
    let stderr_reader = spawn_output_reader(
        child
            .stderr()
            .take()
            .expect("stderr was configured as piped"),
    );
    let started = Instant::now();
    let mut status = None;
    let mut stdout = None;
    let mut stderr = None;

    while started.elapsed() < timeout {
        if status.is_none() {
            status = match child.try_wait() {
                Ok(status) => status,
                Err(error) => {
                    return Err(fail_and_terminate(
                        child.as_mut(),
                        &description,
                        format!("failed while waiting for {description}: {error}"),
                    ));
                }
            };
        }
        if let Err(error) = take_reader_result(&stdout_reader, &mut stdout, "stdout") {
            return Err(fail_and_terminate(child.as_mut(), &description, error));
        }
        if let Err(error) = take_reader_result(&stderr_reader, &mut stderr, "stderr") {
            return Err(fail_and_terminate(child.as_mut(), &description, error));
        }
        if let Some(output) = take_completed_output(&mut status, &mut stdout, &mut stderr) {
            return Ok(output);
        }
        thread::sleep(COMMAND_POLL_INTERVAL.min(timeout.saturating_sub(started.elapsed())));
    }

    // Close a race where the command and both readers finished exactly as the deadline expired, before sending a kill signal to a now-empty group.
    if status.is_none() {
        status = match child.try_wait() {
            Ok(status) => status,
            Err(error) => {
                return Err(fail_and_terminate(
                    child.as_mut(),
                    &description,
                    format!("failed while waiting for {description}: {error}"),
                ));
            }
        };
    }
    if let Err(error) = take_reader_result(&stdout_reader, &mut stdout, "stdout") {
        return Err(fail_and_terminate(child.as_mut(), &description, error));
    }
    if let Err(error) = take_reader_result(&stderr_reader, &mut stderr, "stderr") {
        return Err(fail_and_terminate(child.as_mut(), &description, error));
    }
    if let Some(output) = take_completed_output(&mut status, &mut stdout, &mut stderr) {
        return Ok(output);
    }

    child.start_kill().map_err(|error| {
        format!(
            "command timed out after {}s and its process tree could not be terminated: {description}: {error}",
            timeout.as_secs_f64()
        )
    })?;

    // Give the killed tree a short, bounded opportunity to close its pipes and be reaped.
    // Never turn timeout cleanup into another unbounded wait.
    let cleanup_deadline = Instant::now() + COMMAND_KILL_GRACE;
    let mut cleanup_error = None;
    while Instant::now() < cleanup_deadline {
        if status.is_none() {
            match child.try_wait() {
                Ok(child_status) => status = child_status,
                Err(error) => {
                    cleanup_error = Some(format!("failed to reap the terminated command: {error}"));
                    break;
                }
            }
        }
        if let Err(error) = take_reader_result(&stdout_reader, &mut stdout, "stdout") {
            cleanup_error = Some(error);
            break;
        }
        if let Err(error) = take_reader_result(&stderr_reader, &mut stderr, "stderr") {
            cleanup_error = Some(error);
            break;
        }
        if status.is_some() && stdout.is_some() && stderr.is_some() {
            break;
        }
        thread::sleep(COMMAND_POLL_INTERVAL);
    }

    let cleanup_suffix = cleanup_error
        .map(|error| format!("; cleanup error: {error}"))
        .unwrap_or_default();
    Err(format!(
        "command timed out after {}s: {description}{cleanup_suffix}",
        timeout.as_secs_f64(),
    ))
}

fn pr_number_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"#(\d+)").unwrap())
}

fn breaking_title_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^\w+(?:\([^)]*\))?!:").unwrap())
}

fn conventional_title_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^(\w+)(?:\([^)]*\))?[!]?:\s*(.*)").unwrap())
}

fn skip_title_res() -> &'static [Regex; 3] {
    static RES: OnceLock<[Regex; 3]> = OnceLock::new();
    RES.get_or_init(|| {
        [
            Regex::new(r"(?i)Update contributors and star history").unwrap(),
            Regex::new(r"^v?\d+\.\d+\.\d+").unwrap(),
            Regex::new(r"(?i)^release:").unwrap(),
        ]
    })
}

fn find_latest_stable_tag(root: &Path) -> Result<Option<String>, String> {
    let output = command_output_with_timeout(
        Command::new("git")
            .args(["tag", "--sort=-creatordate"])
            .current_dir(root),
        GIT_TIMEOUT,
    )?;
    if !output.status.success() {
        return Err(format!(
            "git tag failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let version_re = Regex::new(r"^v[0-9]").unwrap();
    let prerelease_re = Regex::new(r"(alpha|beta|rc)").unwrap();
    for line in stdout.lines() {
        let tag = line.trim();
        if version_re.is_match(tag) && !prerelease_re.is_match(tag) {
            return Ok(Some(tag.to_string()));
        }
    }
    Ok(None)
}

fn require_pr_numbers(pr_numbers: Vec<u64>, git_range: &str) -> Result<Vec<u64>, String> {
    if pr_numbers.is_empty() {
        Err(format!(
            "No PRs found in non-empty release range {git_range}"
        ))
    } else {
        Ok(pr_numbers)
    }
}

fn extract_pr_numbers(root: &Path, git_range: &str) -> Result<Vec<u64>, String> {
    let args = if git_range == "HEAD" {
        vec!["log", "--oneline", "HEAD"]
    } else {
        vec!["log", "--oneline", git_range]
    };
    let output = command_output_with_timeout(
        Command::new("git").args(args).current_dir(root),
        GIT_TIMEOUT,
    )?;
    if !output.status.success() {
        return Err(format!(
            "git log failed for {git_range}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(parse_pr_numbers(&String::from_utf8_lossy(&output.stdout)))
}

/// Pull the PR number out of each `git log --oneline` subject.
///
/// A GitHub squash merge appends the PR reference as the *trailing* `(#N)` of
/// the subject line. Any earlier `#N` on the same line is an in-title
/// cross-reference — an issue (`fixes #5740`), a prior PR (`post-#5053`), or a
/// "part N of M" marker (`(#2)`) — not the PR that introduced the commit.
/// Taking only the last `#N` per line keeps those unrelated references out of
/// the release notes; the old "every `#N` in the whole log" approach pulled
/// them in and resolved them to ancient or unmerged PRs.
fn parse_pr_numbers(log: &str) -> Vec<u64> {
    let mut nums: Vec<u64> = log
        .lines()
        .filter_map(|line| {
            pr_number_re()
                .captures_iter(line)
                .last()
                .and_then(|cap| cap.get(1)?.as_str().parse().ok())
        })
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

fn fetch_pr_info(num: u64) -> Result<PrInfo, String> {
    let output = command_output_with_timeout(
        Command::new("gh").args([
            "pr",
            "view",
            &num.to_string(),
            "--json",
            "number,title,author",
        ]),
        GH_TIMEOUT,
    )?;
    if !output.status.success() {
        return Err(format!(
            "gh pr view {num} failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let json: serde_json::Value = serde_json::from_slice(&output.stdout)
        .map_err(|error| format!("invalid gh JSON for PR #{num}: {error}"))?;
    let title = json["title"]
        .as_str()
        .ok_or_else(|| format!("gh JSON for PR #{num} has no title"))?
        .to_string();
    Ok(PrInfo {
        number: json["number"]
            .as_u64()
            .ok_or_else(|| format!("gh JSON for PR #{num} has no number"))?,
        breaking: breaking_title_re().is_match(&title),
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
    skip_title_res().iter().any(|re| re.is_match(title))
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

/// Render one `- <title> (#N) (@author)` line per PR, grouped by conventional-commit category.
///
/// `suppressed` names the PRs the curated `## [Unreleased]` prose already
/// documents. Those are skipped rather than filtered out of the rendered string
/// afterwards, so a category the curated prose covered entirely leaves no
/// dangling `### ` heading and an emptied secondary group leaves no empty
/// `<details>` block. An empty set is the pre-#6628 behaviour: every PR in range
/// gets a generated line.
fn generate_classified_output(prs: &[PrInfo], suppressed: &BTreeSet<u64>) -> String {
    let mut categories: std::collections::HashMap<&str, Vec<String>> =
        std::collections::HashMap::new();

    for pr in prs {
        let title = pr.title.trim();
        if should_skip(title) {
            continue;
        }
        // A PR the curated prose already describes must not also appear as a
        // generated title line, or the release body carries it twice.
        if suppressed.contains(&pr.number) {
            continue;
        }

        let credit = if pr.author.is_empty() {
            String::new()
        } else {
            format!(" (@{})", pr.author)
        };

        let (category, desc) = if let Some(caps) = conventional_title_re().captures(title) {
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
        let desc = conventional_title_re()
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

/// Validate and format a `claude`-generated Highlights block.
/// Rejects output that does not open with exactly `### Highlights`, or that carries any other markdown heading, so a prompt-injected heading (`## [Unreleased]`, a forged `### Fixed`, etc.) can never reach the assembled changelog.
fn format_highlights_output(raw: &str) -> Option<String> {
    let text = raw.trim();
    let mut lines = text.lines();
    if lines.next()? != "### Highlights" || lines.any(|line| line.trim_start().starts_with('#')) {
        return None;
    }
    Some(format!("{text}\n\n"))
}

/// Summarize the classified changelog into a `### Highlights` block via local
/// `claude` CLI. Returns `None` if claude isn't installed, the call fails, or
/// the response is empty — never propagates errors to gate the release.
fn generate_highlights(classified: &str) -> Option<String> {
    if classified.trim().is_empty() {
        return None;
    }

    let version = command_output_with_timeout(
        Command::new("claude").arg("--version"),
        Duration::from_secs(10),
    );
    if !version.is_ok_and(|output| output.status.success()) {
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

    let output = match command_output_with_timeout(
        Command::new("claude")
            .args([
                "-p",
                "--model",
                "claude-sonnet-4-6",
                "--output-format",
                "text",
                &prompt,
            ])
            .env_remove("CLAUDECODE"),
        CLAUDE_TIMEOUT,
    ) {
        Ok(output) => output,
        Err(error) => {
            eprintln!("  {error}; skipping Highlights generation");
            return None;
        }
    };

    if !output.status.success() {
        println!("  claude call failed, skipping Highlights generation");
        return None;
    }

    let Some(block) = format_highlights_output(&String::from_utf8_lossy(&output.stdout)) else {
        eprintln!("  claude returned an invalid Highlights block; skipping it");
        return None;
    };
    println!("  Generated Highlights via claude");
    Some(block)
}

/// Any `## [...]` heading — `## [Unreleased]` or a dated release.
fn is_section_heading(line: &str) -> bool {
    line.starts_with("## [")
}

/// A *dated* release heading: `## [` followed by a digit, so never `## [Unreleased]`.
fn is_dated_release_heading(line: &str) -> bool {
    line.strip_prefix("## [")
        .is_some_and(|rest| rest.starts_with(|c: char| c.is_ascii_digit()))
}

/// Line range of the section introduced by the first line starting with `heading`.
///
/// Returns `(heading_index, end)` where `end` is the index of the first line
/// after the heading for which `ends` holds, or the line count when none does.
/// The body is therefore `lines[heading_index + 1..end]`.
fn section_range(
    lines: &[&str],
    heading: &str,
    ends: impl Fn(&str) -> bool,
) -> Option<(usize, usize)> {
    let start = lines.iter().position(|l| l.starts_with(heading))?;
    let end = lines[start + 1..]
        .iter()
        .position(|l| ends(l))
        .map(|i| start + 1 + i)
        .unwrap_or(lines.len());
    Some((start, end))
}

/// Split a CHANGELOG section body into its top-level bullets, each as its lines.
///
/// A bullet starts at a `- ` in column 0 and ends at the first blank line, `#`
/// heading, or next top-level bullet. That is the same block rule
/// `bullet_block_has_attribution` in `scripts/check-changelog-attribution.py`
/// applies, so a bullet ends here exactly where the attribution gate says it
/// ends. An indented `- ` is a nested list item and stays a continuation line.
fn split_bullets(section: &str) -> Vec<Vec<&str>> {
    let mut bullets: Vec<Vec<&str>> = Vec::new();
    let mut current: Option<Vec<&str>> = None;
    for line in section.lines() {
        if line.starts_with("- ") {
            if let Some(bullet) = current.take() {
                bullets.push(bullet);
            }
            current = Some(vec![line]);
        } else if line.trim().is_empty() || line.starts_with('#') {
            if let Some(bullet) = current.take() {
                bullets.push(bullet);
            }
        } else if let Some(bullet) = current.as_mut() {
            bullet.push(line);
        }
    }
    if let Some(bullet) = current {
        bullets.push(bullet);
    }
    bullets
}

/// First `max` characters of a bullet, for a diagnostic that has to stay readable.
///
/// CHANGELOG bullets in this repo run to several hundred characters, so a warning
/// that echoed one whole would bury everything around it.
fn bullet_excerpt(line: &str, max: usize) -> String {
    let mut out: String = line.chars().take(max).collect();
    if line.chars().count() > max {
        out.push('…');
    }
    out
}

/// Which PRs the curated `[Unreleased]` prose documents, bullet by bullet.
struct CuratedRefs {
    /// Every PR number any curated bullet claims — the set whose generated
    /// entries are suppressed so the release body does not carry them twice.
    refs: BTreeSet<u64>,
    /// The PRs claimed by each curated bullet, in section order, empty for a
    /// bullet that names none. Bullet-level counts need this: one bullet can
    /// document two PRs and one PR can be documented by two bullets, so the
    /// flattened `refs` cannot answer "how many bullets are about this release".
    per_bullet: Vec<BTreeSet<u64>>,
    /// The marker line of every bullet that named no PR at all.
    unreferenced: Vec<String>,
}

/// The PRs each curated bullet documents, and the bullets that name none.
///
/// A bullet's reference is the **last** `(#N)` group on its **last non-empty
/// line**, and nothing else in the bullet is consulted. Both halves of that rule
/// are load-bearing against entries already in this repo's CHANGELOG:
///
/// - One bullet reads `... (#6492): ... (the latter via #6441). (@houko)`, ending
///   on a bare cross-reference. A "last `#N` anywhere" rule would credit it to
///   #6441 and suppress the generated entry for a PR this bullet does not
///   describe — silently dropping it from the release notes.
/// - Another ends `... (#6594, #6595) (@houko)`, documenting two PRs in one
///   group, so reading a single number would leave the other one duplicated.
///
/// A bullet naming no PR fails open **on its own** and does not disarm the
/// others: there is no way to know which entry it replaces, so that PR keeps its
/// generated line, while every bullet that did name a PR still suppresses it.
/// Discarding the whole set instead — the earlier behaviour — turned three
/// unreferenced bullets out of 160 into no suppression at all, which is strictly
/// worse at zero benefit, since keeping the other 157 bullets' references drops
/// nothing that all-or-nothing would have kept.
fn curated_pr_refs(curated: &str) -> CuratedRefs {
    let group_re = Regex::new(r"\(#\d+(?:\s*,\s*#\d+)*\)").unwrap();
    let number_re = Regex::new(r"#(\d+)").unwrap();
    let mut refs = BTreeSet::new();
    let mut per_bullet = Vec::new();
    let mut unreferenced = Vec::new();
    for bullet in split_bullets(curated) {
        let last_group = bullet
            .last()
            .and_then(|line| group_re.find_iter(line).last());
        let claimed: BTreeSet<u64> = match last_group {
            Some(group) => number_re
                .captures_iter(group.as_str())
                .filter_map(|c| c[1].parse::<u64>().ok())
                .collect(),
            None => {
                unreferenced.push(bullet[0].to_string());
                BTreeSet::new()
            }
        };
        refs.extend(claimed.iter().copied());
        per_bullet.push(claimed);
    }
    CuratedRefs {
        refs,
        per_bullet,
        unreferenced,
    }
}

/// Every top-level bullet the file's `## [Unreleased]` section holds, each as a
/// whole block: the `- ` marker line plus its continuation lines, joined with `\n`.
///
/// Blocks rather than marker lines because the marker line is only the bullet's
/// first sentence. This repo's `[Unreleased]` section holds 160 bullets of which
/// 67 are multi-line — the one-sentence-per-line prose rule makes that the norm,
/// not the exception — so a guard that compared marker lines alone would call a
/// bullet "preserved" while every sentence after the first had been dropped.
/// `split_bullets` supplies the block boundary, the same one the attribution gate
/// in `scripts/check-changelog-attribution.py` uses.
///
/// Deliberately a *different* parse from `drain_unreleased`: this one ends the
/// section at the next **dated** `## [YYYY...]` heading, the drain ends it at the
/// next `## [` of any kind. The two agree on a well-formed file. They disagree
/// exactly when something inside the section looks like a heading — a bullet
/// continuation line that starts in column 0 with `## [`, say — which is the case
/// where the drain stops early and every bullet after it would quietly miss the
/// release. Checking the composed body against this parse is what makes the
/// no-loss guard an independent check rather than a restatement of the drain.
fn unreleased_bullet_blocks(content: &str) -> Vec<String> {
    let lines: Vec<&str> = content.lines().collect();
    let Some((start, end)) = section_range(&lines, UNRELEASED_HEADING, is_dated_release_heading)
    else {
        return Vec::new();
    };
    let section = lines[start + 1..end].join("\n");
    split_bullets(&section)
        .iter()
        .map(|bullet| bullet.join("\n"))
        .collect()
}

/// The `## [Unreleased]` section lifted out of a CHANGELOG.
struct DrainedUnreleased {
    /// The section body, blank-trimmed and terminated with one blank line, ready
    /// to concatenate with the generated sections. Empty when the section held
    /// nothing but blank lines.
    curated: String,
    /// The CHANGELOG with that body removed and the `## [Unreleased]` heading
    /// left in place. Byte-identical to the input when there was nothing to take.
    content: String,
    /// Every top-level bullet the section held, whole, per `unreleased_bullet_blocks`.
    bullets: Vec<String>,
}

/// Lift the `## [Unreleased]` body out of `content` so a dated release can carry it.
///
/// The `## [Unreleased]` heading itself always survives, with one blank line under it.
/// In-flight PRs append bullets under that heading and `fold_fragments` folds `changelog.d/` fragments into it, so both break the moment it disappears.
/// The blank line is what keeps a freshly drained heading a valid fold target: `fold_fragments` looks back at the last line before a new `### ` subsection to decide whether it still owes a separator.
///
/// The curated text is a verbatim slice of the body, so the `### ` subsection headings and their order survive exactly as the contributor wrote them.
/// It is terminated with a blank line because every generated section is, and because the next `## [` heading would otherwise be glued onto the last curated line — where the release workflows' `awk '/^## \[/'` extractor stops recognising it as a boundary.
///
/// The section ends at the next `## [` heading of any kind, the same boundary `fold_fragments` uses.
/// That is the conservative choice for a *destructive* step: a stray `## [` in column 0 inside a bullet stops the drain early and leaves the rest of the section in the file, rather than sweeping content out from under a heading.
/// `unreleased_bullet_blocks` draws the boundary at the next *dated* heading instead, so the no-loss guard notices whatever a truncated drain left behind.
fn drain_unreleased(content: &str) -> DrainedUnreleased {
    let lines: Vec<&str> = content.lines().collect();
    let bullets = unreleased_bullet_blocks(content);
    let intact = |bullets: Vec<String>| DrainedUnreleased {
        curated: String::new(),
        content: content.to_string(),
        bullets,
    };

    let Some((start, end)) = section_range(&lines, UNRELEASED_HEADING, is_section_heading) else {
        return intact(bullets);
    };
    let body = &lines[start + 1..end];
    let Some(first) = body.iter().position(|l| !l.trim().is_empty()) else {
        return intact(bullets);
    };
    let last = body
        .iter()
        .rposition(|l| !l.trim().is_empty())
        .expect("a body with a first non-blank line has a last one");

    let mut curated = body[first..=last].join("\n");
    curated.push_str("\n\n");

    let mut kept: Vec<&str> = lines[..=start].to_vec();
    kept.push("");
    kept.extend_from_slice(&lines[end..]);
    let mut drained = kept.join("\n");
    if content.ends_with('\n') {
        drained.push('\n');
    }

    DrainedUnreleased {
        curated,
        content: drained,
        bullets,
    }
}

/// The PR-metadata-derived pieces of a dated release section, in write order.
///
/// A struct rather than four positional `&str` parameters: they are all the same
/// type, and swapping two of them would reorder a release body silently.
struct GeneratedSections<'a> {
    stats: &'a str,
    breaking: &'a str,
    highlights: &'a str,
    classified: &'a str,
}

impl GeneratedSections<'_> {
    /// The dated section's body: generated preamble, then the curated
    /// `[Unreleased]` prose, then the generated entries that prose did not cover.
    ///
    /// The curated text goes in ahead of `classified` and is not merged into it.
    /// A `### Added` written by hand and a `### Added` of generated title lines
    /// can therefore both appear — deliberately, because merging them would mean
    /// re-grouping bullets whose order a human chose.
    fn body(&self, curated: &str) -> String {
        format!(
            "{}{}{}{}{}",
            self.stats, self.breaking, self.highlights, curated, self.classified
        )
    }
}

/// Refuse to write a dated section that dropped hand-written `[Unreleased]` prose.
///
/// Draining is destructive: the prose is removed from one part of the file on its way into another.
/// A parsing mistake anywhere between the two loses text a contributor wrote, and the loss surfaces only once a tag exists — at which point the words are in no commit at all.
/// So the composed body is checked against the section as it stood on disk, before anything is written, and a missing bullet aborts the release.
/// A failed `cargo xtask release` is recoverable by re-running it; a CHANGELOG that quietly shed a paragraph is not.
fn verify_no_curated_bullet_lost(
    bullets: &[String],
    body: &str,
    version: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    // Compared whole, reported by marker line: a bullet is preserved only if
    // every continuation sentence made it, but echoing 160 whole blocks would
    // bury the message under the section it is complaining about.
    let missing: Vec<String> = bullets
        .iter()
        .filter(|bullet| !body.contains(bullet.as_str()))
        .map(|bullet| bullet_excerpt(bullet.lines().next().unwrap_or(bullet), 160))
        .collect();
    if missing.is_empty() {
        return Ok(());
    }
    Err(format!(
        "refusing to write CHANGELOG.md: {} hand-written [Unreleased] bullet(s) would not reach the [{}] section and would be lost. \
         Nothing has been written. Check the [Unreleased] section for a line that starts with `## [` in column 0 — that ends the section early — then re-run. Missing: {}",
        missing.len(),
        version,
        missing.join(" | ")
    )
    .into())
}

/// Attributed bullets in an existing `## [VERSION]` section that regenerating it would drop.
///
/// `render_changelog` replaces a section for a version that already exists, which was harmless while that section held nothing but generated PR titles.
/// Now that the curated `[Unreleased]` prose is carried into it, a second `cargo xtask release` for the same version regenerates the section without that prose — and `[Unreleased]` has already been drained, so no other part of the file holds it either.
/// That second run is not hypothetical: `release.rs` deletes and re-creates the tag, the bump branch, and the GitHub release when the tag already exists, three steps after `changelog::run` hard-fail (`codegen --openapi`, `codegen-sdks.py`, `schema-check gen`), and the preflight then refuses a dirty tree with "Commit or stash changes first" — so the natural recovery is to commit the drained CHANGELOG and re-run.
/// `verify_no_curated_bullet_lost` cannot see this, because on the re-run it derives its expectation from an `[Unreleased]` section that is already empty.
///
/// Generated entries come back identical, so only a bullet carrying a `(@login)` attribution that the new body does not contain is reported: that is contributor prose, and after a fold the deleted `changelog.d/` fragment means `git` is the only place it still exists.
/// Attribution is looked for anywhere in the bullet's block, not just on the `- ` marker line — the one-sentence-per-line prose rule pushes `(@login)` onto a continuation line for every multi-sentence bullet, which is 67 of the 160 in this repo's section today.
/// The returned marker lines are for the diagnostic; the comparison behind them is on whole blocks.
fn prose_dropped_by_regeneration(content: &str, version: &str, body: &str) -> Vec<String> {
    let attribution_re = Regex::new(r"\(@[A-Za-z0-9_][A-Za-z0-9_-]*\)").unwrap();
    let lines: Vec<&str> = content.lines().collect();
    let heading = format!("## [{}]", version);
    let Some((start, end)) = section_range(&lines, &heading, is_section_heading) else {
        return Vec::new();
    };
    let section = lines[start + 1..end].join("\n");
    split_bullets(&section)
        .iter()
        .filter(|bullet| {
            bullet.iter().any(|l| attribution_re.is_match(l))
                && !body.contains(bullet.join("\n").as_str())
        })
        .map(|bullet| bullet[0].to_string())
        .collect()
}

/// Write the dated `## [VERSION]` section, carrying the curated `[Unreleased]` prose into it.
///
/// `drained` is the `[Unreleased]` section already lifted out by the caller —
/// `run` needs it before this point, because which PRs the curated prose covers
/// decides which generated entries `sections.classified` may contain.
fn write_changelog(
    changelog_path: &Path,
    version: &str,
    drained: &DrainedUnreleased,
    sections: &GeneratedSections<'_>,
) -> Result<(), Box<dyn std::error::Error>> {
    let date = chrono::Local::now().format("%Y-%m-%d").to_string();
    let body = sections.body(&drained.curated);

    // Ahead of every write: a curated bullet missing from the body is prose about
    // to disappear, and the operator can still fix the section by hand.
    verify_no_curated_bullet_lost(&drained.bullets, &body, version)?;

    let section = if body.is_empty() {
        format!("## [{}] - {}\n\n_No notable changes._\n", version, date)
    } else {
        format!("## [{}] - {}\n\n{}", version, date, body)
    };

    if !changelog_path.exists() {
        let content = format!("# Changelog\n\n{}\n", section);
        fs::write(changelog_path, content)?;
    } else {
        if Regex::new(&format!(r"(?m)^## \[{}\]", regex::escape(version)))?
            .is_match(&drained.content)
        {
            println!("Replacing existing changelog entry for {}", version);
            // The second no-loss guard, and the only one that can see this case:
            // `verify_no_curated_bullet_lost` above derives its expectation from
            // `[Unreleased]`, which a previous run for this same version already
            // emptied. Aborts rather than warns — a warning scrolls past in a
            // multi-minute release script, and what it would be warning about is
            // prose leaving the file entirely.
            let dropped = prose_dropped_by_regeneration(&drained.content, version, &body);
            if !dropped.is_empty() {
                return Err(format!(
                    "refusing to write CHANGELOG.md: regenerating the existing [{}] section would drop {} attributed bullet(s) it holds and the new section does not, and [Unreleased] no longer holds them either. \
                     Nothing has been written. This is what a second release run for one version looks like once the first drained [Unreleased] into [{}]. \
                     `cargo xtask release` folds and DELETES changelog.d fragments before this step, so for prose that arrived as a fragment this section and git history are now the only copies. \
                     To keep it: move the prose from `## [{}]` back under `## [Unreleased]`, then re-run. \
                     Delete the `## [{}]` section and re-run only if you mean to discard that prose — correct when [Unreleased] already holds the current text and this section is a stale artefact, destructive otherwise. Would be dropped: {}",
                    version,
                    dropped.len(),
                    version,
                    version,
                    version,
                    dropped
                        .iter()
                        .map(|b| bullet_excerpt(b, 120))
                        .collect::<Vec<_>>()
                        .join(" | ")
                )
                .into());
            }
        }
        fs::write(
            changelog_path,
            render_changelog(&drained.content, version, &section),
        )?;
    }

    Ok(())
}

/// Insert (or replace) the `version` section into an existing CHANGELOG body.
///
/// A leading `## [Unreleased]` section always stays at the top: a freshly cut dated release is inserted *below* it, before the first dated `## [YYYY...]` heading.
/// This matches the contributor workflow documented in `CONTRIBUTING.md`, where `[Unreleased]` is the curated section humans append to.
/// Inserting before the very first heading of any kind (the previous behaviour) buried `[Unreleased]` deeper under every release.
///
/// By the time this runs, `drain_unreleased` has already lifted that section's body into `section`, so the heading kept on top is an empty one waiting for the next cycle's bullets.
/// This function neither reads nor rewrites `[Unreleased]` itself.
fn render_changelog(content: &str, version: &str, section: &str) -> String {
    let heading_re = Regex::new(r"(?m)^## \[").unwrap();
    // A dated release heading is `## [` followed by a digit (e.g. `## [2026.6.29]`), never `## [Unreleased]`.
    let dated_heading_re = Regex::new(r"(?m)^## \[\d").unwrap();
    let version_re = Regex::new(&format!(r"(?m)^## \[{}\]", regex::escape(version))).unwrap();

    if version_re.is_match(content) {
        // Replace the existing section for this version in place.
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
            result.push_str(section);
            result.push('\n');
            if let Some(e) = end {
                for line in &lines[e..] {
                    result.push_str(line);
                    result.push('\n');
                }
            }
            return result;
        }
        content.to_string()
    } else if let Some(m) = dated_heading_re.find(content).or_else(|| {
        heading_re
            .find_iter(content)
            .find(|heading| !content[heading.start()..].starts_with(UNRELEASED_HEADING))
    }) {
        // Insert before the first dated release heading so a leading `## [Unreleased]` section stays on top.
        // If there is no dated heading, skip the Unreleased heading itself and insert before any later custom section.
        let pos = m.start();
        let mut result = String::new();
        result.push_str(&content[..pos]);
        result.push_str(section);
        result.push('\n');
        result.push_str(&content[pos..]);
        result
    } else {
        // No headings at all: append.
        let mut result = content.to_string();
        result.push('\n');
        result.push_str(section);
        result.push('\n');
        result
    }
}

/// One `changelog.d/<section>/*.md` fragment: where it came from, and the
/// CHANGELOG bullet it renders as.
struct Fragment {
    path: PathBuf,
    bullet: String,
}

/// Position of `heading` in the canonical subsection order, or `None` when the
/// heading is not one this mechanism owns (`### Performance`, say).
fn canonical_section_index(heading: &str) -> Option<usize> {
    FRAGMENT_SECTIONS.iter().position(|(_, h)| *h == heading)
}

/// Drop a leading Markdown list marker from an already-left-trimmed line.
///
/// The documented fragment format is the bullet body *without* a marker, but
/// every CHANGELOG bullet a contributor has ever read carries one, so writing it
/// anyway is the likely mistake — and it is not one the `(@user)` check can catch,
/// because a marker-carrying fragment is otherwise perfectly valid. Stripping it
/// here renders the bullet the author meant instead of `- - Fix foo`.
fn strip_list_marker(line: &str) -> &str {
    for marker in ["- ", "* ", "+ "] {
        if let Some(rest) = line.strip_prefix(marker) {
            return rest.trim_start();
        }
    }
    line
}

/// Turn a fragment file's body into the lines of one CHANGELOG bullet.
///
/// The body is the bullet text *without* the leading `- `. Leading and trailing
/// blank lines are dropped, the first line gains the `- ` marker, and a
/// continuation line that carries no indentation of its own is indented two
/// spaces to match the surrounding style. A line that is already indented is
/// copied byte-for-byte, so the documented two-space continuation indent — and
/// any deeper nesting built on top of it — survives assembly unchanged.
///
/// A first line that carries a list marker of its own has it stripped rather
/// than doubled; see `strip_list_marker`.
///
/// Returns `None` for a body with no content at all.
fn render_fragment_bullet(body: &str) -> Option<String> {
    let lines: Vec<&str> = body.lines().map(|l| l.trim_end()).collect();
    let first = lines.iter().position(|l| !l.is_empty())?;
    let last = lines.iter().rposition(|l| !l.is_empty())?;
    let mut out = String::new();
    for (i, line) in lines[first..=last].iter().enumerate() {
        if i == 0 {
            out.push_str("- ");
            out.push_str(strip_list_marker(line.trim_start()));
        } else if line.is_empty() || line.starts_with(' ') || line.starts_with('\t') {
            out.push_str(line);
        } else {
            out.push_str("  ");
            out.push_str(line);
        }
        out.push('\n');
    }
    Some(out)
}

/// Read one `changelog.d/<section>/` directory's fragments, ordered by file name.
///
/// `.gitkeep` (and any other dotfile) is infrastructure rather than a fragment,
/// and anything without a `.md` extension is skipped.
fn read_section_fragments(dir: &Path) -> Result<Vec<Fragment>, Box<dyn std::error::Error>> {
    if !dir.is_dir() {
        return Ok(Vec::new());
    }
    let mut paths: Vec<PathBuf> = Vec::new();
    for entry in fs::read_dir(dir)? {
        let path = entry?.path();
        if !path.is_file() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if name.starts_with('.') {
            continue;
        }
        if path.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }
        paths.push(path);
    }
    // Sort by file name rather than trusting `read_dir`: directory iteration
    // order is filesystem-defined, so an unsorted read would make the assembled
    // CHANGELOG depend on the order the fragments happened to be created in.
    // Same determinism rule the prompt-facing registries follow (see #3298).
    paths.sort_by(|a, b| a.file_name().cmp(&b.file_name()));

    let mut fragments = Vec::with_capacity(paths.len());
    for path in paths {
        let body = fs::read_to_string(&path)?;
        let bullet = render_fragment_bullet(&body).ok_or_else(|| {
            format!(
                "changelog fragment {} is empty; it must hold one bullet body ending with `(#PR) (@user)`",
                path.display()
            )
        })?;
        fragments.push(Fragment { path, bullet });
    }
    Ok(fragments)
}

/// A `### ` subsection of `## [Unreleased]`, or the preamble ahead of the first one.
struct Subsection {
    /// The heading text (`Added`), or `None` for the preamble.
    heading: Option<String>,
    /// Every line of the subsection, `### ` heading line included.
    lines: Vec<String>,
}

/// Splice fragment bullets into the `## [Unreleased]` section of a CHANGELOG body.
///
/// `additions` is `(heading, bullets)` in canonical order. Existing subsections
/// are appended to, never replaced, so a bullet a contributor wrote into
/// `[Unreleased]` by hand and a bullet that arrived as a fragment end up
/// side by side — the two paths stay interchangeable. A subsection with
/// fragments but no existing heading is created at its canonical position.
fn fold_fragments(
    content: &str,
    additions: &[(&str, Vec<String>)],
) -> Result<String, Box<dyn std::error::Error>> {
    if additions.iter().all(|(_, bullets)| bullets.is_empty()) {
        return Ok(content.to_string());
    }

    let lines: Vec<&str> = content.lines().collect();
    let unreleased_re = Regex::new(r"^## \[Unreleased\]").unwrap();
    let release_re = Regex::new(r"^## \[").unwrap();
    // `###` but not `####`: the char after the third `#` must be whitespace.
    let sub_re = Regex::new(r"^###\s+(.+?)\s*$").unwrap();

    let start = lines.iter().position(|l| unreleased_re.is_match(l)).ok_or(
        "CHANGELOG.md has no `## [Unreleased]` section to fold changelog.d fragments into",
    )?;
    let end = lines[start + 1..]
        .iter()
        .position(|l| release_re.is_match(l))
        .map(|i| start + 1 + i)
        .unwrap_or(lines.len());

    let mut subs: Vec<Subsection> = vec![Subsection {
        heading: None,
        lines: Vec::new(),
    }];
    for line in &lines[start + 1..end] {
        match sub_re.captures(line) {
            Some(caps) => subs.push(Subsection {
                heading: Some(caps[1].to_string()),
                lines: vec![line.to_string()],
            }),
            None => subs
                .last_mut()
                .expect("subs is seeded with the preamble")
                .lines
                .push(line.to_string()),
        }
    }

    for (heading, bullets) in additions {
        if bullets.is_empty() {
            continue;
        }
        let idx = match subs
            .iter()
            .position(|s| s.heading.as_deref() == Some(*heading))
        {
            Some(i) => i,
            None => {
                // Create the subsection ahead of the first existing subsection
                // that sorts after it, else at the end. Headings this mechanism
                // does not own carry no canonical position and never displace a
                // new one.
                let want = canonical_section_index(heading).unwrap_or(usize::MAX);
                let at = subs
                    .iter()
                    .position(|s| {
                        s.heading
                            .as_deref()
                            .and_then(canonical_section_index)
                            .is_some_and(|i| i > want)
                    })
                    .unwrap_or(subs.len());
                // Keep one blank line between the new heading and whatever
                // precedes it, including the degenerate case of an
                // `## [Unreleased]` with no body at all.
                let prev_is_blank = subs[..at]
                    .iter()
                    .rev()
                    .flat_map(|s| s.lines.iter().rev())
                    .next()
                    .is_some_and(|l| l.trim().is_empty());
                if !prev_is_blank && at > 0 {
                    subs[at - 1].lines.push(String::new());
                }
                subs.insert(
                    at,
                    Subsection {
                        heading: Some((*heading).to_string()),
                        lines: vec![format!("### {}", heading)],
                    },
                );
                at
            }
        };

        let sub = &mut subs[idx];
        // Drop trailing blanks so the new bullets attach to the existing list,
        // then restore exactly one as the separator the next heading expects.
        // The rhythm is load-bearing: `bullet_block_has_attribution` in
        // scripts/check-changelog-attribution.py ends a bullet block at the
        // first blank line, and the `--all-unreleased` gate runs over the
        // result on every push to main.
        while sub.lines.last().is_some_and(|l| l.trim().is_empty()) {
            sub.lines.pop();
        }
        // A subsection whose heading is all that is left — freshly created here,
        // or an empty placeholder already in the file — needs the blank line
        // between the heading and the list restored before the bullets go in.
        if sub.lines.last().is_some_and(|l| l.starts_with("###")) {
            sub.lines.push(String::new());
        }
        for bullet in bullets {
            for bullet_line in bullet.lines() {
                sub.lines.push(bullet_line.to_string());
            }
        }
        sub.lines.push(String::new());
    }

    let mut out_lines: Vec<String> = lines[..=start].iter().map(|l| l.to_string()).collect();
    for sub in &subs {
        out_lines.extend(sub.lines.iter().cloned());
    }
    out_lines.extend(lines[end..].iter().map(|l| l.to_string()));

    let mut out = out_lines.join("\n");
    if content.ends_with('\n') {
        out.push('\n');
    }
    Ok(out)
}

/// Warn about `changelog.d/` subdirectories that hold fragments but are not one
/// of the five recognised sections.
///
/// Assembly can only render a fragment it knows a heading for, so a typo like
/// `changelog.d/fix/` would otherwise be dropped without a word and the entry
/// would vanish from the release notes. This is a warning rather than an error
/// because it must not fail a release: the gate that rejects the typo is
/// `scripts/check-changelog-attribution.py`, which runs per-PR and on every
/// push to main, long before a tag is cut.
fn warn_unrecognised_sections(fragment_root: &Path) {
    let Ok(entries) = fs::read_dir(fragment_root) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if FRAGMENT_SECTIONS.iter().any(|(dir, _)| *dir == name) {
            continue;
        }
        match read_section_fragments(&path) {
            Ok(fragments) if !fragments.is_empty() => eprintln!(
                "warning: {} is not a recognised changelog.d section; {} fragment(s) left in place and NOT folded in. \
                 Move them under one of: {}.",
                path.display(),
                fragments.len(),
                FRAGMENT_SECTIONS
                    .iter()
                    .map(|(dir, _)| *dir)
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            _ => {}
        }
    }
}

/// Fold every `changelog.d/<section>/*.md` fragment into `## [Unreleased]` and
/// delete the files consumed. Returns how many fragments were folded in.
///
/// Deleting the fragments leaves the working tree with changes the caller has to
/// stage — see `release::RELEASE_STAGED_PATHS`, which the release flow relies on
/// to get those deletions into the release commit.
pub(crate) fn collect_fragments_in(root: &Path) -> Result<usize, Box<dyn std::error::Error>> {
    let fragment_root = root.join(FRAGMENT_DIR);
    warn_unrecognised_sections(&fragment_root);

    let mut consumed: Vec<PathBuf> = Vec::new();
    let mut additions: Vec<(&str, Vec<String>)> = Vec::new();
    for (dir, heading) in FRAGMENT_SECTIONS {
        let mut bullets = Vec::new();
        for fragment in read_section_fragments(&fragment_root.join(dir))? {
            bullets.push(fragment.bullet);
            consumed.push(fragment.path);
        }
        additions.push((*heading, bullets));
    }
    if consumed.is_empty() {
        return Ok(0);
    }

    let changelog_path = root.join("CHANGELOG.md");
    let content = fs::read_to_string(&changelog_path)?;
    let folded = fold_fragments(&content, &additions)?;
    fs::write(&changelog_path, folded)?;

    // Try every deletion, then report the survivors as one error. Bailing on the
    // first failure would leave the bullets already written to CHANGELOG.md AND a
    // fragment still on disk, so the next run would fold that fragment in a
    // second time — the silent loss this mechanism exists to prevent, inverted
    // into silent duplication. Naming the survivors makes the state recoverable
    // by hand instead.
    let mut survivors: Vec<String> = Vec::new();
    for path in &consumed {
        if let Err(e) = fs::remove_file(path) {
            survivors.push(format!("{} ({})", path.display(), e));
        }
    }
    if !survivors.is_empty() {
        return Err(format!(
            "CHANGELOG.md has already been updated, but {} fragment(s) could not be deleted: {}. \
             Delete them by hand before re-running, or the next run will fold the same entries in twice.",
            survivors.len(),
            survivors.join("; ")
        )
        .into());
    }
    Ok(consumed.len())
}

/// Assemble `changelog.d/` fragments into the `## [Unreleased]` section.
///
/// A no-op that exits 0 when `changelog.d` is absent or holds no fragments, so
/// it is safe to call unconditionally from the release flow.
pub fn collect_fragments(_args: CollectFragmentsArgs) -> Result<(), Box<dyn std::error::Error>> {
    let root = repo_root();
    match collect_fragments_in(&root)? {
        0 => println!("No changelog.d fragments to fold in."),
        n => println!(
            "Folded {} changelog.d fragment(s) into the [Unreleased] section of CHANGELOG.md",
            n
        ),
    }
    Ok(())
}

pub fn run(args: ChangelogArgs) -> Result<(), Box<dyn std::error::Error>> {
    let root = repo_root();
    let changelog_path = root.join("CHANGELOG.md");

    let base_tag = match args.base_tag {
        Some(tag) => Some(tag),
        None => find_latest_stable_tag(&root)?,
    };

    println!(
        "Generating changelog: {} (since {})",
        args.version,
        base_tag.as_deref().unwrap_or("beginning")
    );

    // Check for gh CLI without allowing a wedged executable to block release tooling.
    let gh_version =
        command_output_with_timeout(Command::new("gh").arg("--version"), Duration::from_secs(10))?;
    if !gh_version.status.success() {
        return Err(format!(
            "gh CLI check failed: {}",
            String::from_utf8_lossy(&gh_version.stderr).trim()
        )
        .into());
    }

    let git_range = match &base_tag {
        Some(tag) => format!("{}..HEAD", tag),
        None => "HEAD".to_string(),
    };

    let pr_numbers = require_pr_numbers(extract_pr_numbers(&root, &git_range)?, &git_range)?;

    // Fetch every PR fail-closed: partial metadata would silently publish an incomplete release section.
    let prs: Vec<PrInfo> = pr_numbers
        .iter()
        .map(|&num| fetch_pr_info(num))
        .collect::<Result<_, _>>()?;

    // Lift the curated `[Unreleased]` prose out first: which PRs it already
    // documents decides which generated entries the dated section may repeat, and
    // that has to be settled before `generate_classified_output` renders anything.
    let existing = if changelog_path.exists() {
        fs::read_to_string(&changelog_path)?
    } else {
        String::new()
    };
    let drained = drain_unreleased(&existing);
    let curated_refs = curated_pr_refs(&drained.curated);
    let suppressed = &curated_refs.refs;
    if !curated_refs.unreferenced.is_empty() {
        eprintln!(
            "warning: {} curated [Unreleased] bullet(s) carry no `(#N)` PR reference, so there is no way to tell which generated entry they replace. \
             Those PRs keep their generated title line and may therefore appear twice in this release body; every other curated bullet still suppresses its own entry. \
             Add the reference to: {}",
            curated_refs.unreferenced.len(),
            curated_refs
                .unreferenced
                .iter()
                .map(|b| bullet_excerpt(b, 120))
                .collect::<Vec<_>>()
                .join(" | ")
        );
    }

    // The generated list twice over: the full one feeds the summarizer, the
    // deduped one goes into the section body. Highlights are picked from
    // everything in the release on purpose — hiding the PRs the curated prose
    // covers would hide exactly the changes someone cared enough to write about.
    let full_classified = generate_classified_output(&prs, &BTreeSet::new());
    let classified = generate_classified_output(&prs, suppressed);
    let breaking = generate_breaking_changes(&prs).unwrap_or_default();
    let stats = generate_stats_line(&prs, base_tag.as_deref()).unwrap_or_default();

    // Feed breaking + classified to claude so highlights can flag breaking items.
    let highlights_input = format!("{}{}", breaking, full_classified);
    let highlights = generate_highlights(&highlights_input).unwrap_or_default();

    write_changelog(
        &changelog_path,
        &args.version,
        &drained,
        &GeneratedSections {
            stats: &stats,
            breaking: &breaking,
            highlights: &highlights,
            classified: &classified,
        },
    )?;

    println!("Updated {}", changelog_path.display());
    if !drained.curated.is_empty() {
        // Count PRs actually in range, not references found: curated prose cites
        // plenty of older PRs that were never going to get a generated entry here.
        // Metadata collection is fail-closed, so `prs` contains every extracted in-range PR or generation returned before touching the changelog.
        let in_range: BTreeSet<u64> = prs.iter().map(|p| p.number).collect();
        let suppressed_in_range = suppressed.intersection(&in_range).count();
        // Broken out by bullet, because the totals mean different things. Nothing
        // prunes `[Unreleased]`, so it accumulates across releases: a bullet whose
        // PRs are all outside this release's git range describes work that already
        // shipped, and carrying it into a dated section re-announces it as new.
        let about_this_release = curated_refs
            .per_bullet
            .iter()
            .filter(|r| !r.is_disjoint(&in_range))
            .count();
        let already_shipped = curated_refs
            .per_bullet
            .iter()
            .filter(|r| !r.is_empty() && r.is_disjoint(&in_range))
            .count();
        // Total taken from the same split as the three parts, so they always sum.
        println!(
            "Carried {} curated [Unreleased] bullet(s) into the {} section: {} reference a PR in {} ({} generated entry/entries suppressed as already covered), {} reference only PRs outside that range, {} carry no reference at all",
            curated_refs.per_bullet.len(),
            args.version,
            about_this_release,
            git_range,
            suppressed_in_range,
            already_shipped,
            curated_refs.unreferenced.len()
        );
        if already_shipped > 0 {
            eprintln!(
                "warning: {} of those bullet(s) reference no PR in {}, so they describe work that shipped in an earlier release. \
                 Nothing prunes [Unreleased], so it accumulates until someone does: this release body will announce them as new, contradicting the `Full diff` compare link published beside it. \
                 Prune the already-shipped bullets from [Unreleased] before cutting if that is not what you want.",
                already_shipped, git_range
            );
        }
    }

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

#[cfg(test)]
mod tests {
    use super::{
        bullet_excerpt, collect_fragments_in, command_output_with_timeout, curated_pr_refs,
        drain_unreleased, extract_pr_numbers, format_highlights_output, generate_classified_output,
        parse_pr_numbers, prose_dropped_by_regeneration, render_changelog, render_fragment_bullet,
        require_pr_numbers, unreleased_bullet_blocks, verify_no_curated_bullet_lost,
        write_changelog, GeneratedSections, PrInfo, FRAGMENT_DIR, FRAGMENT_SECTIONS,
        UNRELEASED_HEADING,
    };
    use std::collections::BTreeSet;
    use std::fs;
    use std::io::Write;
    use std::path::PathBuf;
    use std::process::{Command, Stdio};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{Duration, Instant};

    fn first_section_heading(s: &str) -> &str {
        s.lines().find(|l| l.starts_with("## [")).unwrap()
    }

    struct TmpTree(PathBuf);
    impl Drop for TmpTree {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    /// A CHANGELOG fixture shaped like the real file: an `## [Unreleased]`
    /// section with hand-written bullets under two of the five subsections,
    /// followed by a dated release.
    const BASE: &str = "# Changelog\n\n## [Unreleased]\n\n### Added\n\n- Existing added bullet (#1) (@houko)\n\n### Fixed\n\n- Existing fixed bullet (#2) (@houko)\n\n## [2026.1.1] - 2026-01-01\n\n### Fixed\n\n- Shipped bullet (#0) (@houko)\n";

    /// A repo-shaped scratch tree: `CHANGELOG.md` plus the five `changelog.d/`
    /// section directories, each seeded with the `.gitkeep` the real tree has.
    fn make_tree(changelog: &str) -> TmpTree {
        // A process-wide counter keeps parallel `cargo test` threads from
        // sharing a directory and contaminating each other's fragments.
        static SEQ: AtomicU64 = AtomicU64::new(0);
        let seq = SEQ.fetch_add(1, Ordering::Relaxed);
        let root =
            std::env::temp_dir().join(format!("lf-changelog-frag-{}-{seq}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("CHANGELOG.md"), changelog).unwrap();
        for (dir, _) in FRAGMENT_SECTIONS {
            let section = root.join(FRAGMENT_DIR).join(dir);
            fs::create_dir_all(&section).unwrap();
            fs::write(section.join(".gitkeep"), "").unwrap();
        }
        TmpTree(root)
    }

    fn fragment(t: &TmpTree, section: &str, name: &str, body: &str) {
        fs::write(t.0.join(FRAGMENT_DIR).join(section).join(name), body).unwrap();
    }

    fn changelog_of(t: &TmpTree) -> String {
        fs::read_to_string(t.0.join("CHANGELOG.md")).unwrap()
    }

    /// Mirror of the release-notes extractor used by
    /// `.github/workflows/release.yml` and `.github/workflows/release-notify.yml`:
    /// `awk '/^## \[VERSION\]/{found=1; next} found && /^## \[/{exit} found{print}'`.
    fn awk_extract(content: &str, version: &str) -> String {
        let heading = format!("## [{}]", version);
        let mut found = false;
        let mut out = String::new();
        for line in content.lines() {
            if !found {
                found = line.starts_with(&heading);
                continue;
            }
            if line.starts_with("## [") {
                break;
            }
            out.push_str(line);
            out.push('\n');
        }
        out
    }

    #[test]
    fn fragment_assembly_is_deterministic_across_creation_order() {
        // One tree creates the fragments in reverse-alphabetical order, the
        // other in alphabetical order. `read_dir` order is filesystem-defined,
        // so only the explicit sort in `read_section_fragments` can make the two
        // assembled files agree byte for byte.
        let reverse = make_tree(BASE);
        for (name, body) in [
            ("3-third.md", "Third fragment. (#33) (@houko)\n"),
            ("2-second.md", "Second fragment. (#22) (@houko)\n"),
            ("1-first.md", "First fragment. (#11) (@houko)\n"),
        ] {
            fragment(&reverse, "fixed", name, body);
        }
        assert_eq!(collect_fragments_in(&reverse.0).unwrap(), 3);

        let forward = make_tree(BASE);
        for (name, body) in [
            ("1-first.md", "First fragment. (#11) (@houko)\n"),
            ("2-second.md", "Second fragment. (#22) (@houko)\n"),
            ("3-third.md", "Third fragment. (#33) (@houko)\n"),
        ] {
            fragment(&forward, "fixed", name, body);
        }
        assert_eq!(collect_fragments_in(&forward.0).unwrap(), 3);

        let out = changelog_of(&reverse);
        assert_eq!(
            out,
            changelog_of(&forward),
            "assembly output depends on fragment creation order"
        );

        // Bullets are ordered by file name within the section.
        let first = out.find("- First fragment.").unwrap();
        let second = out.find("- Second fragment.").unwrap();
        let third = out.find("- Third fragment.").unwrap();
        assert!(first < second && second < third, "{out}");

        // Consumed fragments are deleted; `.gitkeep` is left alone.
        let section = reverse.0.join(FRAGMENT_DIR).join("fixed");
        assert!(!section.join("1-first.md").exists());
        assert!(section.join(".gitkeep").exists());
    }

    #[test]
    fn fold_appends_to_existing_subsection_and_creates_missing_one_in_canonical_order() {
        let t = make_tree(BASE);
        fragment(
            &t,
            "fixed",
            "6623-wire-max-content-chars.md",
            "Wire max_content_chars through. (#6623) (@houko)\n",
        );
        fragment(
            &t,
            "security",
            "6624-scrub-token.md",
            "Scrub the relay token from the log line. (#6624) (@houko)\n",
        );
        assert_eq!(collect_fragments_in(&t.0).unwrap(), 2);
        let out = changelog_of(&t);

        // `### Fixed` already existed: the fragment is appended after the
        // hand-written bullet, which survives untouched.
        let existing = out.find("- Existing fixed bullet (#2) (@houko)").unwrap();
        let appended = out.find("- Wire max_content_chars through.").unwrap();
        assert!(existing < appended, "{out}");
        assert!(
            out.contains("- Existing added bullet (#1) (@houko)"),
            "{out}"
        );

        // `### Security` did not exist and is created at its canonical position,
        // after `### Fixed` (order: Added, Fixed, Changed, Security, Documentation)
        // and still inside `[Unreleased]`, above the first dated release.
        let fixed_heading = out.find("### Fixed").unwrap();
        let security_heading = out.find("### Security").unwrap();
        let dated = out.find("## [2026.1.1]").unwrap();
        assert!(fixed_heading < security_heading, "{out}");
        assert!(security_heading < dated, "{out}");

        // Exactly one blank line separates the appended bullet from the new
        // heading, and the new heading from its first bullet.
        assert!(
            out.contains("- Wire max_content_chars through. (#6623) (@houko)\n\n### Security\n\n- Scrub the relay token from the log line. (#6624) (@houko)\n"),
            "{out}"
        );
        // The duplicate-`[Unreleased]` guard in scripts/hooks/pre-commit (#3395)
        // must stay satisfied.
        assert_eq!(out.matches("## [Unreleased]").count(), 1, "{out}");
    }

    #[test]
    fn collect_fragments_is_a_noop_without_fragments() {
        // Section directories present but empty (only `.gitkeep`).
        let t = make_tree(BASE);
        assert_eq!(collect_fragments_in(&t.0).unwrap(), 0);
        assert_eq!(changelog_of(&t), BASE);

        // `changelog.d` absent entirely.
        fs::remove_dir_all(t.0.join(FRAGMENT_DIR)).unwrap();
        assert_eq!(collect_fragments_in(&t.0).unwrap(), 0);
        assert_eq!(changelog_of(&t), BASE);
    }

    /// A fragment that cannot be deleted must surface as an error naming it, not
    /// leave the tree in a state where the next run folds the same bullet in
    /// twice.
    ///
    /// Skips itself where the platform or the running uid will not let a
    /// read-only directory block `remove_file` (Windows, or root in a container):
    /// the assertion is only meaningful once the delete has actually failed.
    #[test]
    fn an_undeletable_fragment_is_reported_instead_of_silently_left() {
        let t = make_tree(BASE);
        fragment(
            &t,
            "fixed",
            "9999-stuck.md",
            "Stuck fragment. (#9999) (@houko)\n",
        );
        let section = t.0.join(FRAGMENT_DIR).join("fixed");
        let original = fs::metadata(&section).unwrap().permissions();
        let mut readonly = original.clone();
        readonly.set_readonly(true);
        if fs::set_permissions(&section, readonly).is_err() {
            return;
        }

        let result = collect_fragments_in(&t.0);
        // Restore before asserting so `TmpTree::drop` can still clean up.
        let _ = fs::set_permissions(&section, original);

        if !section.join("9999-stuck.md").exists() {
            return; // deletion succeeded anyway; nothing to assert
        }
        let message = result
            .expect_err("an undeletable fragment must surface as an error")
            .to_string();
        assert!(message.contains("already been updated"), "{message}");
        assert!(message.contains("9999-stuck.md"), "{message}");
        // The bullet really is in the file — which is exactly what the error
        // warns the operator about.
        assert!(
            changelog_of(&t).contains("- Stuck fragment. (#9999) (@houko)"),
            "{}",
            changelog_of(&t)
        );
    }

    #[test]
    fn fragments_in_an_unrecognised_section_are_left_in_place() {
        let t = make_tree(BASE);
        let bogus = t.0.join(FRAGMENT_DIR).join("fix");
        fs::create_dir_all(&bogus).unwrap();
        fs::write(
            bogus.join("9999-typo.md"),
            "Typo'd section directory. (#9999) (@houko)\n",
        )
        .unwrap();
        // Assembly has no heading to render `fix/` under, so it folds nothing,
        // leaves the CHANGELOG untouched, warns, and — crucially — does not
        // delete the fragment, so the entry is recoverable. The gate that
        // rejects the typo before it reaches main is
        // scripts/check-changelog-attribution.py.
        assert_eq!(collect_fragments_in(&t.0).unwrap(), 0);
        assert_eq!(changelog_of(&t), BASE);
        assert!(bogus.join("9999-typo.md").exists());
    }

    #[test]
    fn multiline_fragment_keeps_its_two_space_continuation_indent() {
        let t = make_tree(BASE);
        fragment(
            &t,
            "added",
            "6630-thing.md",
            "Add the thing.\n  It exists because the other thing could not.\n  Third sentence. (#6630) (@houko)\n",
        );
        assert_eq!(collect_fragments_in(&t.0).unwrap(), 1);
        assert!(
            changelog_of(&t).contains(
                "- Add the thing.\n  It exists because the other thing could not.\n  Third sentence. (#6630) (@houko)\n"
            ),
            "{}",
            changelog_of(&t)
        );
    }

    #[test]
    fn render_fragment_bullet_indents_an_unindented_continuation() {
        assert_eq!(
            render_fragment_bullet("\nFirst sentence.\nSecond sentence. (@houko)\n\n").unwrap(),
            "- First sentence.\n  Second sentence. (@houko)\n"
        );
        assert!(render_fragment_bullet("\n  \n").is_none());
    }

    #[test]
    fn render_fragment_bullet_strips_a_marker_the_author_wrote_anyway() {
        // The format says "no leading `- `", but a contributor copying the shape
        // of an existing CHANGELOG bullet writes one, and nothing rejects it —
        // the entry is otherwise valid. Doubling the marker would render
        // `- - Fix foo` in the release notes.
        for marker in ["- ", "-   ", "* ", "+ "] {
            let body = format!("{marker}Fix the thing.\nSecond sentence. (#1) (@houko)\n");
            assert_eq!(
                render_fragment_bullet(&body).unwrap(),
                "- Fix the thing.\n  Second sentence. (#1) (@houko)\n",
                "marker {marker:?} was not stripped"
            );
        }
        // A hyphen that is not a list marker is content and stays put.
        assert_eq!(
            render_fragment_bullet("-Wall is now passed to the linker (@houko)\n").unwrap(),
            "- -Wall is now passed to the linker (@houko)\n"
        );
    }

    /// `FRAGMENT_SECTIONS` here and `FRAGMENT_SECTIONS` in
    /// `scripts/check-changelog-attribution.py` are a cross-language contract,
    /// and the two drift directions are asymmetric. A section added only here
    /// makes the validator reject every fragment written under it — loud, and
    /// nothing is lost. A section added only to the validator makes the entries
    /// pass review and then vanish at assembly time, because this list is what
    /// decides which directories are read at all. The second one is the failure
    /// this whole mechanism exists to prevent, so the lists are compared rather
    /// than commented at each other.
    #[test]
    fn fragment_sections_match_the_python_validator() {
        let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("xtask/ has a parent directory");
        let script = repo.join("scripts/check-changelog-attribution.py");
        let src = fs::read_to_string(&script)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", script.display()));

        let literal = src
            .lines()
            .find_map(|l| l.strip_prefix("FRAGMENT_SECTIONS = frozenset({"))
            .and_then(|rest| rest.split_once("})"))
            .map(|(inner, _)| inner)
            .unwrap_or_else(|| {
                panic!(
                    "no `FRAGMENT_SECTIONS = frozenset({{...}})` line in {}",
                    script.display()
                )
            });
        let mut python: Vec<&str> = literal
            .split(',')
            .map(|s| s.trim().trim_matches('"').trim_matches('\''))
            .filter(|s| !s.is_empty())
            .collect();
        python.sort_unstable();

        let mut rust: Vec<&str> = FRAGMENT_SECTIONS.iter().map(|(dir, _)| *dir).collect();
        rust.sort_unstable();

        assert_eq!(
            rust,
            python,
            "changelog.d section lists disagree: xtask/src/changelog.rs has {rust:?}, {} has {python:?}",
            script.display()
        );
    }

    /// Fold a fragment into a copy of the repo's OWN `CHANGELOG.md`.
    ///
    /// The fixtures above are a dozen lines; the real file is thousands, with a
    /// long `[Unreleased]` section and `<details>` blocks inside the dated
    /// releases — and it is the file assembly actually runs against. Mirrors the
    /// intent of `ci::tests::real_repo_tree_is_green`. Only the temp copy is
    /// written; the repo's file is read-only here.
    ///
    /// Reads through `repo_changelog_with_populated_unreleased` so the fold is
    /// exercised against real prose on a release branch too, where the file's
    /// own `[Unreleased]` has just been drained. The subsection assertion is a
    /// delta rather than an equality because the fold legitimately creates
    /// `### Changed` when the section does not already have one — asserting the
    /// mid-cycle count outright is what failed on the v2026.7.31 release PR
    /// (#6688).
    #[test]
    fn folds_into_the_repos_own_changelog() {
        let real = repo_changelog_with_populated_unreleased();
        let t = make_tree(&real);
        fragment(
            &t,
            "changed",
            "9999-probe.md",
            "Probe bullet. (#9999) (@houko)\n",
        );
        assert_eq!(collect_fragments_in(&t.0).unwrap(), 1);
        let out = changelog_of(&t);

        // The bullet landed inside the real file's `[Unreleased]` range.
        assert!(
            awk_extract(&out, "Unreleased").contains("- Probe bullet. (#9999) (@houko)"),
            "probe did not land inside [Unreleased]"
        );
        // Column 0 only. `matches()` counts substrings, so a bullet that quotes
        // the heading in its prose — the #6628 entry does, on an indented
        // continuation line — inflates this to the number of such references.
        assert_eq!(
            out.lines()
                .filter(|l| l.starts_with(UNRELEASED_HEADING))
                .count(),
            1
        );
        assert_eq!(
            out.lines().filter(|l| l.starts_with("## [")).count(),
            real.lines().filter(|l| l.starts_with("## [")).count(),
            "release heading count changed"
        );

        // `### Changed` is created when `[Unreleased]` is empty (release
        // branch) and reused when it is not (mid-cycle). Either way the fold
        // must not displace or invent any *other* subsection.
        let unreleased_had_changed = awk_extract(&real, "Unreleased")
            .lines()
            .any(|l| l.trim() == "### Changed");
        let expected_delta = usize::from(!unreleased_had_changed);
        assert_eq!(
            out.lines().filter(|l| l.starts_with("### ")).count(),
            real.lines().filter(|l| l.starts_with("### ")).count() + expected_delta,
            "subsection heading count changed by more than the one `### Changed` the fold may add"
        );
    }

    /// A generated entry list with the other three generated pieces empty.
    fn sections_of(classified: &str) -> GeneratedSections<'_> {
        GeneratedSections {
            stats: "",
            breaking: "",
            highlights: "",
            classified,
        }
    }

    fn pr(number: u64, title: &str) -> PrInfo {
        PrInfo {
            number,
            title: title.to_string(),
            author: "houko".to_string(),
            breaking: false,
        }
    }

    /// Cut the dated section the way `run` does, minus the `gh` calls: drain the
    /// file on disk, then hand the drained section plus a generated entry list to
    /// the writer. Returns the CHANGELOG as written.
    fn cut_release(
        t: &TmpTree,
        version: &str,
        classified: &str,
    ) -> Result<String, Box<dyn std::error::Error>> {
        let path = t.0.join("CHANGELOG.md");
        let existing = fs::read_to_string(&path)?;
        let drained = drain_unreleased(&existing);
        write_changelog(&path, version, &drained, &sections_of(classified))?;
        Ok(fs::read_to_string(&path)?)
    }

    /// The curated `[Unreleased]` prose is what the release body is *for*: it
    /// explains why a change was made, which a PR title cannot. Until this landed
    /// the section was write-only — the dated release was assembled from PR
    /// metadata alone and the hand-written bullets reached nothing.
    #[test]
    fn curated_unreleased_prose_reaches_the_dated_section() {
        let t = make_tree(BASE);
        let out = cut_release(
            &t,
            "2026.2.2",
            "### Changed\n\n- Generated from a PR title (#3) (@houko)\n\n",
        )
        .unwrap();
        let slice = awk_extract(&out, "2026.2.2");

        // Both curated subsections, with their bullets, are in the release body.
        assert!(
            slice.contains("### Added\n\n- Existing added bullet (#1) (@houko)\n"),
            "{slice}"
        );
        assert!(
            slice.contains("### Fixed\n\n- Existing fixed bullet (#2) (@houko)\n"),
            "{slice}"
        );
        // In the order the contributor wrote them, ahead of the generated list.
        // Nothing is re-sorted or re-grouped: a human chose that order.
        let added = slice.find("### Added").unwrap();
        let fixed = slice.find("### Fixed").unwrap();
        let generated = slice.find("- Generated from a PR title").unwrap();
        assert!(added < fixed && fixed < generated, "{slice}");
    }

    /// The `## [Unreleased]` heading has to outlive the drain: in-flight PRs
    /// append bullets under it, and `collect_fragments` folds `changelog.d/`
    /// fragments into it. Either one breaks against a file where the heading is
    /// gone — the fold errors outright.
    #[test]
    fn draining_leaves_unreleased_a_bare_heading_fragments_still_fold_into() {
        let t = make_tree(BASE);
        let out = cut_release(&t, "2026.2.2", "").unwrap();

        assert_eq!(out.lines().filter(|l| *l == "## [Unreleased]").count(), 1);
        assert_eq!(first_section_heading(&out), "## [Unreleased]", "{out}");
        assert!(
            awk_extract(&out, "Unreleased").trim().is_empty(),
            "the drained section still holds bullets:\n{out}"
        );

        // Still a valid fold target after the cut.
        fragment(
            &t,
            "fixed",
            "6623-after-the-cut.md",
            "Folded in after the release was cut. (#6623) (@houko)\n",
        );
        assert_eq!(collect_fragments_in(&t.0).unwrap(), 1);
        let folded = changelog_of(&t);
        assert!(
            awk_extract(&folded, "Unreleased")
                .contains("- Folded in after the release was cut. (#6623) (@houko)"),
            "{folded}"
        );
        // And the fold left the freshly cut release alone.
        assert!(
            awk_extract(&folded, "2026.2.2").contains("- Existing fixed bullet (#2) (@houko)"),
            "{folded}"
        );
        assert_eq!(folded.matches("## [Unreleased]").count(), 1, "{folded}");
    }

    /// Every PR appears exactly once: as curated prose where one was written, as
    /// a generated title line otherwise.
    #[test]
    fn a_curated_pr_reference_suppresses_only_its_own_generated_entry() {
        let curated = "### Fixed\n\n- Curated prose about the approvals fix (#6605) (@houko)\n\n";
        let refs = curated_pr_refs(curated).refs;
        assert_eq!(refs, BTreeSet::from([6605]));

        let prs = vec![
            pr(6605, "fix(cli): send a Content-Type on approvals approve"),
            pr(6606, "fix(api): scrub the relay token from the log line"),
        ];
        let classified = generate_classified_output(&prs, &refs);
        assert!(
            !classified.contains("(#6605)"),
            "the curated bullet's PR still got a generated line:\n{classified}"
        );
        assert!(
            classified.contains("- Scrub the relay token from the log line (#6606) (@houko)"),
            "a PR with no curated bullet lost its generated line:\n{classified}"
        );

        let body = sections_of(&classified).body(curated);
        assert_eq!(body.matches("(#6605)").count(), 1, "{body}");
        assert_eq!(body.matches("(#6606)").count(), 1, "{body}");
    }

    /// The reference is the last `(#N)` group on the bullet's last non-empty
    /// line. Both halves of that rule are load-bearing against bullets already in
    /// this repo's `[Unreleased]` section, reproduced here in miniature.
    #[test]
    fn curated_pr_reference_is_the_last_group_on_the_last_line() {
        // A bare `#N` cross-reference at the end of the bullet is not the PR the
        // bullet documents. Reading the last `#N` anywhere would credit this
        // bullet to #6441 and drop #6441's own generated entry.
        let trailing_xref = "- Fix approvals approve (#6492): the re-resolve 400 was already correct (the latter via #6441). (@houko)\n";
        assert_eq!(curated_pr_refs(trailing_xref).refs, BTreeSet::from([6492]));

        // One bullet can document two PRs in a single group.
        let two_prs = "- Carry `schedule` and `[autonomous]` through the flat format (#6594, #6595) (@houko)\n";
        assert_eq!(curated_pr_refs(two_prs).refs, BTreeSet::from([6594, 6595]));

        // A mid-bullet reference on an earlier line is not consulted at all.
        let multiline = "- First sentence, as in (#983).\n  Second sentence (#6630) (@houko)\n";
        assert_eq!(curated_pr_refs(multiline).refs, BTreeSet::from([6630]));
    }

    /// Fail safe, not clever, and **per bullet**: a bullet naming no PR keeps that
    /// PR's generated line, and does not disarm suppression for the bullets that
    /// did name one. A duplicated entry in a release body is cosmetic; a silently
    /// dropped PR is not — but the fix for the former must not cost the latter.
    #[test]
    fn a_curated_bullet_with_no_pr_reference_fails_open_only_for_itself() {
        let curated = "### Fixed\n\n- Curated prose with no reference at all (@houko)\n\n### Added\n\n- Curated prose about the approvals fix (#6605) (@houko)\n\n";
        let curated_refs = curated_pr_refs(curated);
        assert_eq!(
            curated_refs.unreferenced,
            vec!["- Curated prose with no reference at all (@houko)".to_string()],
            "the warning must name the bullet that needs the reference"
        );
        assert_eq!(
            curated_refs.refs,
            BTreeSet::from([6605]),
            "one unreferenced bullet must not discard the references the others carry"
        );

        // #6605 is covered by curated prose, so its generated line is suppressed.
        // #6606 is not mentioned at all, so it keeps one.
        let prs = vec![
            pr(6605, "fix(cli): send a Content-Type on approvals approve"),
            pr(6606, "fix(api): scrub the relay token from the log line"),
        ];
        let classified = generate_classified_output(&prs, &curated_refs.refs);
        assert!(
            !classified.contains("(#6605)"),
            "suppression was disarmed by an unrelated unreferenced bullet:\n{classified}"
        );
        assert!(classified.contains("(#6606)"), "{classified}");

        // The unreferenced bullet's own PR — whichever it is — cannot be suppressed,
        // which is the whole point of failing open rather than guessing.
        let body = sections_of(&classified).body(curated);
        assert_eq!(body.matches("(#6605)").count(), 1, "{body}");
    }

    /// The empty-`[Unreleased]` path is what every release before this change
    /// took, and it must still produce the same bytes: generated pieces only, the
    /// rest of the file untouched.
    #[test]
    fn an_empty_unreleased_section_cuts_a_byte_identical_release() {
        const EMPTY_UNRELEASED: &str = "# Changelog\n\n## [Unreleased]\n\n## [2026.1.1] - 2026-01-01\n\n### Fixed\n\n- Shipped bullet (#0) (@houko)\n";
        let stats = "_2 PRs from 1 contributor since v2026.1.1._\n\n";
        let breaking = "### Breaking Changes\n\n- Break the thing (#4) (@houko)\n\n";
        let highlights = "### Highlights\n\n- **Thing** — it works now\n\n";
        let classified = "### Fixed\n\n- Generated from a PR title (#3) (@houko)\n\n";

        let drained = drain_unreleased(EMPTY_UNRELEASED);
        assert!(drained.curated.is_empty());
        assert!(drained.bullets.is_empty());
        assert_eq!(
            drained.content, EMPTY_UNRELEASED,
            "an empty [Unreleased] section must not reshape the file at all"
        );

        let t = make_tree(EMPTY_UNRELEASED);
        let path = t.0.join("CHANGELOG.md");
        write_changelog(
            &path,
            "2026.2.2",
            &drained,
            &GeneratedSections {
                stats,
                breaking,
                highlights,
                classified,
            },
        )
        .unwrap();
        let out = fs::read_to_string(&path).unwrap();

        // Byte-identical to the pre-change composition: stats + breaking +
        // highlights + classified under the dated heading, inserted by the
        // untouched `render_changelog`. The date is read back out of the file so
        // a midnight rollover between two `Local::now()` calls cannot flake this.
        let heading = out
            .lines()
            .find(|l| l.starts_with("## [2026.2.2] - "))
            .unwrap();
        let expected_section = format!("{heading}\n\n{stats}{breaking}{highlights}{classified}");
        assert_eq!(
            out,
            render_changelog(EMPTY_UNRELEASED, "2026.2.2", &expected_section)
        );
    }

    /// The no-loss guard has to be able to fire, and has to fire before the write.
    ///
    /// The reachable way to lose a curated bullet is a continuation line that
    /// starts in column 0 with `## [`: that ends the section as far as the drain
    /// is concerned, so everything after it stays behind and would miss the
    /// release. The guard scans to the next *dated* heading instead and notices.
    #[test]
    fn a_curated_bullet_the_drain_cannot_reach_aborts_the_write() {
        const TRUNCATING: &str = "# Changelog\n\n## [Unreleased]\n\n### Changed\n\n- Document the release heading shape (#6627) (@houko)\n## [Unreleased] must stay the first heading in the file\n\n### Fixed\n\n- Bullet the drain never reaches (#6629) (@houko)\n\n## [2026.1.1] - 2026-01-01\n\n- old (#1) (@me)\n";
        let t = make_tree(TRUNCATING);
        let path = t.0.join("CHANGELOG.md");

        let drained = drain_unreleased(TRUNCATING);
        assert!(
            drained
                .curated
                .contains("- Document the release heading shape (#6627) (@houko)"),
            "{:?}",
            drained.curated
        );
        assert!(
            !drained.curated.contains("- Bullet the drain never reaches"),
            "the drain stopping early is the premise of this test: {:?}",
            drained.curated
        );

        let err = write_changelog(&path, "2026.2.2", &drained, &sections_of(""))
            .expect_err("a curated bullet that cannot reach the section must abort the release");
        let message = err.to_string();
        assert!(
            message.contains("- Bullet the drain never reaches (#6629) (@houko)"),
            "the error must name the bullet: {message}"
        );
        assert!(message.contains("Nothing has been written"), "{message}");
        assert_eq!(
            changelog_of(&t),
            TRUNCATING,
            "the guard must run before the write, leaving the file untouched"
        );
    }

    #[test]
    fn release_cut_stays_sliceable_by_the_awk_extractor() {
        let t = make_tree(BASE);
        fragment(
            &t,
            "fixed",
            "6623-fragment.md",
            "Fragment-authored bullet. (#6623) (@houko)\n",
        );
        assert_eq!(collect_fragments_in(&t.0).unwrap(), 1);

        let out = cut_release(
            &t,
            "2026.2.2",
            "### Fixed\n\n- Generated from a PR title (#6605) (@houko)\n\n",
        )
        .unwrap();

        // `awk '/^## \[VERSION\]/{found=1;next} found && /^## \[/{exit} found{print}'`
        // depends on two things: the dated heading starts a line, and the section
        // is terminated by another `## [` that also starts a line.
        assert!(out.contains("\n## [2026.2.2] - "), "{out}");
        let slice = awk_extract(&out, "2026.2.2");
        assert!(!slice.trim().is_empty(), "extractor sliced nothing:\n{out}");

        // Curated and generated bullets each occupy a line of their own. Exact
        // line equality rather than `contains`: a curated body that did not end in
        // a blank line would glue its last bullet onto the generated `### Fixed`
        // heading, which `contains` would still find as a substring.
        let lines: Vec<&str> = slice.lines().collect();
        for expected in [
            "### Added",
            "- Existing added bullet (#1) (@houko)",
            "### Fixed",
            "- Existing fixed bullet (#2) (@houko)",
            "- Fragment-authored bullet. (#6623) (@houko)",
            "- Generated from a PR title (#6605) (@houko)",
        ] {
            assert!(
                lines.contains(&expected),
                "{expected:?} is not a line of the release body:\n{slice}"
            );
        }

        // The slice terminates at the next `## [`, so neither `[Unreleased]` nor
        // an older release leaks into the body.
        assert!(!slice.contains("## ["), "{slice}");
        assert!(!slice.contains("Shipped bullet"), "{slice}");
        // And what follows the slice is that heading, at the start of a line.
        let after = &out[out.find(slice.as_str()).unwrap() + slice.len()..];
        assert!(
            after.starts_with("## ["),
            "the extractor's terminating heading does not start a line: {:?}",
            &after[..after.len().min(80)]
        );

        // `[Unreleased]` stays on top, now emptied of the bullets the release took.
        assert_eq!(first_section_heading(&out), "## [Unreleased]", "{out}");
        assert!(awk_extract(&out, "Unreleased").trim().is_empty(), "{out}");
    }

    #[test]
    fn curated_multiline_bullets_keep_their_continuation_indent() {
        const MULTILINE: &str = "# Changelog\n\n## [Unreleased]\n\n### Added\n\n- First sentence of the curated bullet.\n  Second sentence, indented two spaces.\n  Third sentence (#6630) (@houko)\n\n## [2026.1.1] - 2026-01-01\n\n- old (#1) (@me)\n";
        let t = make_tree(MULTILINE);
        let out = cut_release(&t, "2026.2.2", "").unwrap();
        assert!(
            awk_extract(&out, "2026.2.2").contains(
                "- First sentence of the curated bullet.\n  Second sentence, indented two spaces.\n  Third sentence (#6630) (@houko)\n"
            ),
            "{out}"
        );
    }

    /// Regenerating a section for a version that already exists replaces it
    /// wholesale, which was harmless while it held nothing but generated PR
    /// titles. Now that curated prose is carried into it, a second
    /// `cargo xtask release` for one version would delete that prose from the
    /// only place left holding it — `[Unreleased]` was drained by the first run.
    /// `verify_no_curated_bullet_lost` is blind to this: on the re-run it derives
    /// its expectation from an already-empty `[Unreleased]`. So this is a second,
    /// independent abort.
    #[test]
    fn regenerating_a_section_aborts_rather_than_dropping_its_curated_prose() {
        const CUT: &str = "# Changelog\n\n## [Unreleased]\n\n## [2026.2.2] - 2026-02-02\n\n### Highlights\n\n- **Thing** — it works now\n\n### Fixed\n\n- Curated prose already carried into the release (#6605) (@houko)\n- Generated from a PR title (#6606) (@houko)\n\n## [2026.1.1] - 2026-01-01\n\n- old (#1) (@me)\n";

        // The regenerated body reproduces the generated line but not the curated prose.
        let dropped = prose_dropped_by_regeneration(
            CUT,
            "2026.2.2",
            "### Fixed\n\n- Generated from a PR title (#6606) (@houko)\n\n",
        );
        assert_eq!(
            dropped,
            vec!["- Curated prose already carried into the release (#6605) (@houko)".to_string()],
            "the abort must name the prose that would vanish, and only that"
        );

        // A body that reproduces everything attributed reports nothing. The
        // unattributed Highlights bullet is never reported either way: it is
        // regenerated on every run, not written by a contributor.
        assert!(prose_dropped_by_regeneration(
            CUT,
            "2026.2.2",
            "### Fixed\n\n- Curated prose already carried into the release (#6605) (@houko)\n- Generated from a PR title (#6606) (@houko)\n\n",
        )
        .is_empty());

        // Nothing is written, and the prose is still in the file afterwards.
        let t = make_tree(CUT);
        let err = cut_release(
            &t,
            "2026.2.2",
            "### Fixed\n\n- Generated from a PR title (#6606) (@houko)\n\n",
        )
        .expect_err("regenerating over curated prose must abort, not write a lossy file");
        assert!(
            err.to_string().contains("Curated prose already carried"),
            "the abort must name the bullet: {err}"
        );
        assert_eq!(
            changelog_of(&t),
            CUT,
            "the file must be untouched when the write is refused"
        );
    }

    /// The attribution that marks a bullet as contributor prose sits wherever the
    /// sentence-per-line rule put it — for a multi-sentence bullet that is a
    /// continuation line, never the `- ` marker. 67 of the 160 bullets in this
    /// repo's `[Unreleased]` section are shaped exactly this way, so a per-line
    /// attribution test would have waved 42% of the prose at risk straight
    /// through, while reporting a count that read as if it were safe.
    #[test]
    fn attribution_on_a_continuation_line_still_marks_a_bullet_as_prose() {
        const CUT: &str = "# Changelog\n\n## [Unreleased]\n\n## [2026.2.2] - 2026-02-02\n\n### Fixed\n\n- First sentence of the curated bullet, with the marker line carrying no attribution.\n  Second sentence, and the credit lands here (#6605) (@houko)\n\n## [2026.1.1] - 2026-01-01\n\n- old (#1) (@me)\n";

        let dropped = prose_dropped_by_regeneration(CUT, "2026.2.2", "### Fixed\n\n");
        assert_eq!(
            dropped,
            vec![
                "- First sentence of the curated bullet, with the marker line carrying no attribution."
                    .to_string()
            ],
            "a bullet whose `(@login)` is on a continuation line is still prose"
        );

        // And the guard compares whole blocks, so losing only the continuation
        // sentences counts as losing the bullet.
        let marker_only =
            "### Fixed\n\n- First sentence of the curated bullet, with the marker line carrying no attribution.\n\n";
        assert_eq!(
            prose_dropped_by_regeneration(CUT, "2026.2.2", marker_only).len(),
            1,
            "a body holding only the marker line must not count the bullet as preserved"
        );
    }

    /// The repo's own `CHANGELOG.md`, with a populated `## [Unreleased]`.
    ///
    /// Mid-cycle that is the file verbatim when `[Unreleased]` already contains
    /// a multi-line bullet. On a `chore/bump-version-*` branch `cargo xtask
    /// release` has drained that prose into the dated section it just cut, and
    /// immediately after a release `[Unreleased]` may contain only a new
    /// single-line bullet. In either shape a test of whole-block preservation
    /// would assert nothing. The multi-line prose is not gone though — it is
    /// sitting in the newest dated section, so move it back to reconstitute the
    /// pre-release shape.
    ///
    /// This keeps the real-file coverage on release branches instead of skipping
    /// it there, which is what the v2026.7.31 release PR (#6688) exposed: both
    /// real-file tests failed on the release commit, blocking the release PR on a
    /// state the release flow itself creates.
    fn repo_changelog_with_populated_unreleased() -> String {
        let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("xtask/ has a parent directory");
        let real = fs::read_to_string(repo.join("CHANGELOG.md")).unwrap();

        if drain_unreleased(&real)
            .bullets
            .iter()
            .any(|bullet| bullet.lines().count() > 1)
        {
            return real;
        }

        // Find the newest dated section and hoist its body into `[Unreleased]`.
        let dated = real
            .lines()
            .find(|l| l.starts_with("## [") && !l.starts_with(UNRELEASED_HEADING))
            .expect("a drained CHANGELOG still has at least one dated release");
        let version = dated
            .trim_start_matches("## [")
            .split(']')
            .next()
            .expect("a `## [` heading always has a closing bracket");
        let body = awk_extract(&real, version);

        let (head, tail) = real
            .split_once(UNRELEASED_HEADING)
            .expect("the file always carries an [Unreleased] heading");
        format!("{head}{UNRELEASED_HEADING}\n\n{}{tail}", body.trim_end())
    }

    /// Drain the repo's OWN `CHANGELOG.md`.
    ///
    /// The fixtures above are a dozen lines; the real section is 160 hand-written
    /// bullets with mid-bullet `(#N)` cross-references and multi-PR groups, and it
    /// is the text the next release actually carries. It is also where the drain's
    /// boundary and the guard's boundary could disagree in production — a `## [`
    /// in column 0 anywhere inside the section would abort every release until
    /// someone indented it. Only the temp copy is written; the repo's file is read
    /// here, never touched.
    #[test]
    fn drains_the_repos_own_unreleased_section_without_tripping_the_guard() {
        let real = repo_changelog_with_populated_unreleased();
        let drained = drain_unreleased(&real);

        assert!(
            !drained.bullets.is_empty(),
            "the repo's [Unreleased] section is empty, so this test asserts nothing"
        );
        let body = sections_of("").body(&drained.curated);
        verify_no_curated_bullet_lost(&drained.bullets, &body, "0.0.0").unwrap();

        // One `[Unreleased]` heading, now holding no bullets.
        assert_eq!(
            drained
                .content
                .lines()
                .filter(|l| l.starts_with(UNRELEASED_HEADING))
                .count(),
            1
        );
        assert!(unreleased_bullet_blocks(&drained.content).is_empty());
        // Nothing structural moved: no heading was swallowed and none invented.
        assert_eq!(
            drained
                .content
                .lines()
                .filter(|l| l.starts_with("## ["))
                .count(),
            real.lines().filter(|l| l.starts_with("## [")).count(),
            "release heading count changed"
        );

        // Cut a release from it and slice the result with the workflows' extractor:
        // every curated bullet has to come out of that slice, on its own line.
        let t = make_tree(&real);
        let out = cut_release(
            &t,
            "2026.2.2",
            "### Fixed\n\n- Generated from a PR title (#1) (@houko)\n\n",
        )
        .unwrap();
        let slice = awk_extract(&out, "2026.2.2");
        let slice_lines: Vec<&str> = slice.lines().collect();
        // Whole blocks, verbatim: a multi-line bullet has to arrive with every
        // continuation line intact and in order, not just its marker line.
        for bullet in &drained.bullets {
            assert!(
                slice.contains(bullet.as_str()),
                "curated bullet did not reach the sliced release body: {}",
                bullet_excerpt(bullet.lines().next().unwrap_or(bullet), 120)
            );
        }
        // At least some of the real section is multi-line, or the block-level
        // assertion above is silently only testing marker lines.
        assert!(
            drained.bullets.iter().any(|b| b.lines().count() > 1),
            "no multi-line curated bullet in the real section, so blocks are untested"
        );
        assert!(
            slice_lines.contains(&"- Generated from a PR title (#1) (@houko)"),
            "the generated list did not survive alongside the curated prose"
        );
        // Column 0 only, matching the `awk` extractor's `/^## \[/` boundary. A
        // substring test reads a bullet that *quotes* a heading — the #6628
        // entry says "appended its bullet to the single `## [Unreleased]`
        // section" on an indented continuation line — as an overrun, failing on
        // prose the extractor handles correctly.
        assert!(
            !slice_lines.iter().any(|l| l.starts_with("## [")),
            "the slice ran past its boundary"
        );
        assert_eq!(
            first_section_heading(&out),
            "## [Unreleased]",
            "{}",
            &out[..200]
        );
    }

    #[test]
    fn keeps_unreleased_on_top_when_cutting_release() {
        // Regression: a new dated release must land BELOW `## [Unreleased]`, not above it.
        // The old behaviour inserted before the first heading of any kind, burying `[Unreleased]` deeper under every release.
        let content = "# Changelog\n\n## [Unreleased]\n\n### Added\n\n- pending (#9) (@me)\n\n## [2026.1.1] - 2026-01-01\n\n- old (#1) (@me)\n";
        let section = "## [2026.2.2] - 2026-02-02\n\n### Fixed\n\n- thing (#10) (@me)\n";
        let out = render_changelog(content, "2026.2.2", section);

        assert_eq!(first_section_heading(&out), "## [Unreleased]");
        let unrel = out.find("## [Unreleased]").unwrap();
        let new = out.find("## [2026.2.2]").unwrap();
        let old = out.find("## [2026.1.1]").unwrap();
        assert!(
            unrel < new && new < old,
            "order was unrel={unrel} new={new} old={old}"
        );
        // Existing content is preserved verbatim.
        assert!(out.contains("- pending (#9) (@me)"));
        assert!(out.contains("- old (#1) (@me)"));
    }

    #[test]
    fn keeps_unreleased_on_top_for_first_release_without_dated_heading() {
        let content = "# Changelog\n\n## [Unreleased]\n\n### Added\n\n- pending (#9) (@me)\n";
        let section = "## [2026.2.2] - 2026-02-02\n\n### Fixed\n\n- thing (#10) (@me)\n";
        let out = render_changelog(content, "2026.2.2", section);

        assert_eq!(first_section_heading(&out), "## [Unreleased]");
        assert!(out.find("## [Unreleased]").unwrap() < out.find("## [2026.2.2]").unwrap());
    }

    #[test]
    fn rejects_structural_headings_in_generated_highlights() {
        let injected = "### Highlights\n\n- **Safe** — summary\n\n## [Unreleased]\n\n- injected";
        assert!(format_highlights_output(injected).is_none());
        assert!(format_highlights_output("### Highlights\n\n  ## [Unreleased]").is_none());
        assert!(format_highlights_output("# Changelog\n\n- replacement").is_none());
    }

    #[test]
    fn accepts_well_formed_generated_highlights() {
        let text = "### Highlights\n\n- **Feature** — concise summary";
        assert_eq!(format_highlights_output(text), Some(format!("{text}\n\n")));
    }

    #[test]
    fn extract_pr_numbers_reports_git_failure() {
        let tree = make_tree("");
        let error = extract_pr_numbers(&tree.0, "missing-tag..HEAD").unwrap_err();
        assert!(error.contains("git log"), "{error}");
    }

    #[test]
    fn rejects_an_empty_pr_range_before_writing() {
        let error = require_pr_numbers(Vec::new(), "v1..HEAD").unwrap_err();
        assert!(error.contains("No PRs found"), "{error}");
    }

    #[test]
    fn command_timeout_terminates_hung_child() {
        let mut command = Command::new(std::env::current_exe().unwrap());
        let module = module_path!().split_once("::").unwrap().1;
        command
            .args(["--exact", &format!("{module}::timeout_probe")])
            .env("LIBREFANG_CHANGELOG_TIMEOUT_PROBE", "1");
        let error =
            command_output_with_timeout(&mut command, Duration::from_millis(50)).unwrap_err();
        assert!(error.contains("timed out"), "{error}");
    }

    #[test]
    fn timeout_probe() {
        if std::env::var_os("LIBREFANG_CHANGELOG_TIMEOUT_PROBE").is_some() {
            std::thread::sleep(Duration::from_secs(10));
        }
    }

    #[test]
    fn command_timeout_covers_descendants_holding_output_pipes() {
        let module = module_path!().split_once("::").unwrap().1;
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args(["--exact", &format!("{module}::descendant_pipe_probe")])
            .env("LIBREFANG_CHANGELOG_DESCENDANT_PROBE", "1");

        let started = Instant::now();
        let error =
            command_output_with_timeout(&mut command, Duration::from_millis(100)).unwrap_err();
        assert!(error.contains("timed out"), "{error}");
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn descendant_pipe_probe() {
        if std::env::var_os("LIBREFANG_CHANGELOG_DESCENDANT_PROBE").is_none() {
            return;
        }
        let module = module_path!().split_once("::").unwrap().1;
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", &format!("{module}::pipe_holder_probe")])
            .env("LIBREFANG_CHANGELOG_PIPE_HOLDER", "1")
            .spawn()
            .unwrap();
        std::thread::spawn(move || child.wait());
    }

    #[test]
    fn pipe_holder_probe() {
        if std::env::var_os("LIBREFANG_CHANGELOG_PIPE_HOLDER").is_some() {
            std::thread::sleep(Duration::from_secs(2));
        }
    }

    #[test]
    fn command_capture_drains_large_stdout_and_stderr() {
        let module = module_path!().split_once("::").unwrap().1;
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args(["--exact", &format!("{module}::large_output_probe")])
            .env("LIBREFANG_CHANGELOG_LARGE_OUTPUT_PROBE", "1");
        let output = command_output_with_timeout(&mut command, Duration::from_secs(5)).unwrap();
        assert!(output.status.success());
        assert!(output.stdout.len() >= 1024 * 1024);
        assert!(output.stderr.len() >= 1024 * 1024);
    }

    #[test]
    fn command_capture_preserves_a_stream_that_finishes_first() {
        let module = module_path!().split_once("::").unwrap().1;
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args(["--exact", &format!("{module}::staggered_output_probe")])
            .env("LIBREFANG_CHANGELOG_STAGGERED_PROBE", "1");

        let output = command_output_with_timeout(&mut command, Duration::from_secs(2)).unwrap();
        assert!(output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains("delayed stderr"));
    }

    #[test]
    fn staggered_output_probe() {
        if std::env::var_os("LIBREFANG_CHANGELOG_STAGGERED_PROBE").is_none() {
            return;
        }
        let module = module_path!().split_once("::").unwrap().1;
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                &format!("{module}::delayed_stderr_probe"),
                "--nocapture",
            ])
            .env("LIBREFANG_CHANGELOG_DELAYED_STDERR_PROBE", "1")
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        std::thread::spawn(move || child.wait());
    }

    #[test]
    fn delayed_stderr_probe() {
        if std::env::var_os("LIBREFANG_CHANGELOG_DELAYED_STDERR_PROBE").is_some() {
            std::thread::sleep(Duration::from_millis(100));
            eprintln!("delayed stderr");
        }
    }

    #[test]
    fn large_output_probe() {
        if std::env::var_os("LIBREFANG_CHANGELOG_LARGE_OUTPUT_PROBE").is_none() {
            return;
        }
        let bytes = vec![b'x'; 1024 * 1024];
        std::io::stdout().write_all(&bytes).unwrap();
        std::io::stderr().write_all(&bytes).unwrap();
    }

    #[test]
    fn inserts_at_top_when_no_unreleased_section() {
        let content = "# Changelog\n\n## [2026.1.1] - 2026-01-01\n\n- old (#1) (@me)\n";
        let section = "## [2026.2.2] - 2026-02-02\n\n- new (#2) (@me)\n";
        let out = render_changelog(content, "2026.2.2", section);
        assert_eq!(first_section_heading(&out), "## [2026.2.2] - 2026-02-02");
    }

    #[test]
    fn replaces_existing_version_section_in_place() {
        let content = "# Changelog\n\n## [Unreleased]\n\n- pending (#9) (@me)\n\n## [2026.2.2] - 2026-02-02\n\n- stale (#5) (@me)\n\n## [2026.1.1] - 2026-01-01\n\n- old (#1) (@me)\n";
        let section = "## [2026.2.2] - 2026-02-02\n\n- regenerated (#6) (@me)\n";
        let out = render_changelog(content, "2026.2.2", section);
        assert!(out.contains("- regenerated (#6) (@me)"));
        assert!(!out.contains("- stale (#5) (@me)"));
        // `[Unreleased]` stays on top and the older release is preserved.
        assert_eq!(first_section_heading(&out), "## [Unreleased]");
        assert!(out.contains("## [2026.1.1]"));
    }

    #[test]
    fn takes_trailing_pr_number_per_line() {
        let log = "abc1234 fix(api): scrub internal errors (#5863)\n\
                   def5678 feat(dashboard): kanban board (#5805)\n";
        assert_eq!(parse_pr_numbers(log), vec![5805, 5863]);
    }

    #[test]
    fn ignores_in_title_cross_references() {
        // Each line carries an earlier `#N` that is NOT the merge PR: a
        // "part N" marker, an issue ref, and a prior-PR ref. Only the trailing
        // `(#N)` is the real squash-merge PR number.
        let log = "a1 fix(runtime): make subprocess sandbox secure-by-default (#2) (#5862)\n\
                   b2 feat: support custom-URL STT/TTS (fixes #5740) (#5814)\n\
                   c3 fix: reconcile cascade-leak THEMATIC_HEADERS with post-#5053 builder (#5351)\n";
        assert_eq!(parse_pr_numbers(log), vec![5351, 5814, 5862]);
    }

    #[test]
    fn handles_merge_commit_subjects() {
        let log = "e5f6a7b Merge pull request #1234 from contributor/branch\n";
        assert_eq!(parse_pr_numbers(log), vec![1234]);
    }

    #[test]
    fn skips_lines_without_a_pr_reference() {
        let log = "deadbeef chore: tidy up\n\
                   cafef00d fix: real change (#4242)\n";
        assert_eq!(parse_pr_numbers(log), vec![4242]);
    }

    #[test]
    fn sorts_and_dedupes() {
        // Duplicate trailing refs (e.g. a follow-up that re-states a number)
        // collapse; output is ascending.
        let log = "1 c (#30)\n2 b (#10)\n3 a (#30)\n";
        assert_eq!(parse_pr_numbers(log), vec![10, 30]);
    }

    #[test]
    fn empty_log_yields_no_numbers() {
        assert!(parse_pr_numbers("").is_empty());
    }
}
