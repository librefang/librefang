use crate::common::repo_root;
use clap::Parser;
use regex::Regex;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

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
    parse_pr_numbers(&stdout)
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
    let re = Regex::new(r"#(\d+)").unwrap();
    let mut nums: Vec<u64> = log
        .lines()
        .filter_map(|line| {
            re.captures_iter(line)
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
        if Regex::new(&format!(r"(?m)^## \[{}\]", regex::escape(version)))?.is_match(&content) {
            println!("Replacing existing changelog entry for {}", version);
        }
        fs::write(
            changelog_path,
            render_changelog(&content, version, &section),
        )?;
    }

    Ok(())
}

/// Insert (or replace) the `version` section into an existing CHANGELOG body.
///
/// A leading `## [Unreleased]` section always stays at the top: a freshly cut dated release is inserted *below* it, before the first dated `## [YYYY...]` heading.
/// This matches the contributor workflow documented in `CONTRIBUTING.md`, where `[Unreleased]` is the curated section humans append to.
/// Inserting before the very first heading of any kind (the previous behaviour) buried `[Unreleased]` deeper under every release.
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
    } else if let Some(m) = dated_heading_re
        .find(content)
        .or_else(|| heading_re.find(content))
    {
        // Insert before the first dated release heading so a leading `## [Unreleased]` section stays on top.
        // Fall back to the first heading of any kind when no dated release exists yet.
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

#[cfg(test)]
mod tests {
    use super::{
        collect_fragments_in, parse_pr_numbers, render_changelog, render_fragment_bullet,
        write_changelog, FRAGMENT_DIR, FRAGMENT_SECTIONS,
    };
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

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
    #[test]
    fn folds_into_the_repos_own_changelog() {
        let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("xtask/ has a parent directory");
        let real = fs::read_to_string(repo.join("CHANGELOG.md")).unwrap();
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
        // Nothing structural moved: `### Changed` already exists in
        // `[Unreleased]`, so no heading was created and none was displaced.
        assert_eq!(out.matches("## [Unreleased]").count(), 1);
        assert_eq!(
            out.lines().filter(|l| l.starts_with("## [")).count(),
            real.lines().filter(|l| l.starts_with("## [")).count(),
            "release heading count changed"
        );
        assert_eq!(
            out.lines().filter(|l| l.starts_with("### ")).count(),
            real.lines().filter(|l| l.starts_with("### ")).count(),
            "subsection heading count changed"
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

        // Cut the dated section the way the release flow does, on the folded file.
        let path = t.0.join("CHANGELOG.md");
        let classified = "### Fixed\n\n- Generated from a PR title (#6623) (@houko)\n\n";
        write_changelog(&path, "2026.2.2", classified).unwrap();
        let out = fs::read_to_string(&path).unwrap();

        // The extractor must find content between `## [2026.2.2]` and the next
        // `## [` — that slice is verbatim the GitHub release body.
        let slice = awk_extract(&out, "2026.2.2");
        assert!(!slice.trim().is_empty(), "extractor sliced nothing:\n{out}");
        assert!(
            slice.contains("- Generated from a PR title (#6623) (@houko)"),
            "{slice}"
        );
        // It terminates at the next `## [`, so neither `[Unreleased]` nor an
        // older release leaks into the body.
        assert!(!slice.contains("## ["), "{slice}");
        assert!(!slice.contains("Fragment-authored bullet."), "{slice}");
        assert!(!slice.contains("Existing fixed bullet"), "{slice}");
        assert!(!slice.contains("Shipped bullet"), "{slice}");

        // `[Unreleased]` is still the first `## [` heading and the folded
        // fragment sits inside its range: the fold must never push a bullet past
        // the section boundary into the freshly cut release.
        assert_eq!(first_section_heading(&out), "## [Unreleased]", "{out}");
        assert!(
            awk_extract(&out, "Unreleased")
                .contains("- Fragment-authored bullet. (#6623) (@houko)"),
            "{out}"
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
