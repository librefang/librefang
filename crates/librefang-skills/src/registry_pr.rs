//! Propose an evolved skill back to the public skill registry as a GitHub PR.
//!
//! When an operator approves an evolved skill (via `auto_evolve` or the
//! skill workshop), this module contributes it back to the configured
//! registry repository (default `librefang/librefang-registry`) by:
//!
//! 1. forking the registry repo under the authenticated user (idempotent),
//! 2. creating a branch off the fork's default branch,
//! 3. committing the skill files (`skill.toml`, prompt context, supporting
//!    files) under `skills/<name>/` via the Contents API, and
//! 4. opening a pull request with an auto-generated description (what
//!    changed, why, version diff) back to the upstream registry.
//!
//! The whole flow runs against the GitHub REST API with `reqwest` — no
//! local `git` / `gh` binary is required, so it works from inside the
//! daemon process and inside containers. Authentication uses a GitHub
//! token (`GITHUB_TOKEN`); the caller resolves the token (env or vault)
//! and passes it in.

use crate::evolution::SkillEvolutionMeta;
use crate::{InstalledSkill, SkillError};
use base64::Engine as _;
use serde_json::{json, Value};
use std::path::Path;
use std::time::Duration;

/// Default upstream registry repository in `owner/name` form.
pub const DEFAULT_REGISTRY_REPO: &str = "librefang/librefang-registry";

/// GitHub REST API base URL.
const GITHUB_API: &str = "https://api.github.com";

/// Maximum size of a single supporting file we will push to the registry,
/// in bytes. The Contents API base64-inflates payloads ~33%, and large
/// binaries do not belong in a prompt-skill PR. Anything larger is
/// skipped with a warning rather than failing the whole proposal.
const MAX_FILE_BYTES: u64 = 1_000_000;

/// Outcome of a successful registry proposal.
#[derive(Debug, Clone)]
pub struct ProposedSkillPr {
    /// HTML URL of the opened pull request.
    pub pr_url: String,
    /// Upstream repository the PR targets (`owner/name`).
    pub repo: String,
    /// Head branch created on the fork.
    pub branch: String,
}

/// Inputs for [`propose_skill_to_registry`].
pub struct ProposeRequest<'a> {
    /// The installed skill snapshot to contribute.
    pub skill: &'a InstalledSkill,
    /// Evolution metadata used to build the PR description (version diff,
    /// changelog). Pass [`SkillEvolutionMeta::default`] when none exists.
    pub evolution: &'a SkillEvolutionMeta,
    /// Upstream registry repo in `owner/name` form.
    pub registry_repo: &'a str,
    /// GitHub token with `repo` scope (fork + push + open PR).
    pub token: &'a str,
}

/// Fork the registry, push the skill files to a branch, and open a PR.
///
/// Idempotent on the fork (a pre-existing fork is reused) but not on the
/// branch: each call creates a uniquely-named branch so repeated proposals
/// do not clobber each other.
///
/// **Not idempotent on failure.** The steps run in order — fork, create
/// branch, push files, then open the PR — and there is no rollback. If a
/// later step fails (network drop, or a 422 from `open_pull_request` when an
/// identical PR already exists), the fork and the already-pushed
/// `skill/<name>-<timestamp>` branch remain on the user's fork; the caller
/// gets the error but the partial state is not cleaned up. Because the branch
/// name is timestamped, each retry pushes a *new* branch, so repeated failures
/// accumulate orphan `skill/*` branches on the fork. Pruning those is a remote
/// GitHub housekeeping concern, out of scope for this crate.
pub async fn propose_skill_to_registry(
    req: ProposeRequest<'_>,
) -> Result<ProposedSkillPr, SkillError> {
    let name = req.skill.manifest.skill.name.clone();
    validate_repo_slug(req.registry_repo)?;
    if req.token.trim().is_empty() {
        return Err(SkillError::InvalidManifest(
            "A GitHub token is required to propose a skill to the registry".to_string(),
        ));
    }

    let client = RegistryGithubClient::new(req.token.to_string());

    // 1. Who are we? The fork lands under this login.
    let login = client.authenticated_login().await?;

    // 2. Ensure a fork exists under our account (idempotent).
    let fork_repo = format!("{login}/{}", repo_name(req.registry_repo));
    client.ensure_fork(req.registry_repo, &fork_repo).await?;

    // 3. Branch off the fork's default branch.
    let (default_branch, base_sha) = client.fork_default_branch_head(&fork_repo).await?;
    let branch = format!(
        "skill/{}-{}",
        sanitize_branch_component(&name),
        chrono::Utc::now().format("%Y%m%d%H%M%S")
    );
    client.create_branch(&fork_repo, &branch, &base_sha).await?;

    // 4. Push each skill file under skills/<name>/ on the new branch.
    let files = collect_skill_files(req.skill)?;
    if files.is_empty() {
        return Err(SkillError::InvalidManifest(format!(
            "Skill '{name}' has no files to propose"
        )));
    }
    for file in &files {
        let dest = format!("skills/{name}/{}", file.rel_path);
        client
            .put_file(
                &fork_repo,
                &branch,
                &dest,
                &file.contents,
                &format!("Add {dest}"),
            )
            .await?;
    }

    // 5. Open the PR upstream.
    let title = format!("skill: contribute `{name}`");
    let body = build_pr_body(req.skill, req.evolution);
    let head = format!("{login}:{branch}");
    let pr_url = client
        .open_pull_request(req.registry_repo, &default_branch, &head, &title, &body)
        .await?;

    Ok(ProposedSkillPr {
        pr_url,
        repo: req.registry_repo.to_string(),
        branch,
    })
}

/// A skill file staged for the registry PR.
struct StagedFile {
    /// Path relative to the skill directory, forward-slashed.
    rel_path: String,
    /// Raw file bytes.
    contents: Vec<u8>,
}

/// Collect `skill.toml` plus all supporting files under the skill dir,
/// skipping VCS / build junk and oversized blobs.
fn collect_skill_files(skill: &InstalledSkill) -> Result<Vec<StagedFile>, SkillError> {
    let dir = &skill.path;
    let mut out = Vec::new();
    for entry in walkdir::WalkDir::new(dir)
        .into_iter()
        .filter_entry(|e| !is_excluded(dir, e.path()))
    {
        let entry = entry.map_err(|e| SkillError::Io(std::io::Error::other(e)))?;
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path();
        let Ok(rel) = path.strip_prefix(dir) else {
            continue;
        };
        // `.evolution.json` is local bookkeeping (counters, content
        // hashes) — it does not belong in a public contribution.
        if rel == Path::new(".evolution.json") {
            continue;
        }
        let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
        if size > MAX_FILE_BYTES {
            tracing::warn!(
                file = %rel.display(),
                size,
                "skipping oversized file when proposing skill to registry"
            );
            continue;
        }
        let contents = std::fs::read(path)?;
        out.push(StagedFile {
            rel_path: forward_slash(rel),
            contents,
        });
    }
    // Deterministic ordering so repeated proposals produce identical diffs.
    out.sort_by(|a, b| a.rel_path.cmp(&b.rel_path));
    Ok(out)
}

/// Whether a path component is VCS / build noise we never contribute.
fn is_excluded(root: &Path, path: &Path) -> bool {
    if path == root {
        return false;
    }
    let Ok(rel) = path.strip_prefix(root) else {
        return true;
    };
    for component in rel.components() {
        let name = component.as_os_str().to_string_lossy();
        if matches!(
            name.as_ref(),
            ".git"
                | ".github"
                | "node_modules"
                | "target"
                | "__pycache__"
                | ".pytest_cache"
                | ".venv"
                | "venv"
                | ".DS_Store"
        ) {
            return true;
        }
    }
    false
}

fn forward_slash(path: &Path) -> String {
    path.components()
        .map(|c| c.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

/// Build the auto-generated PR body: what the skill is, what changed, why,
/// and the version diff drawn from the evolution changelog.
fn build_pr_body(skill: &InstalledSkill, evolution: &SkillEvolutionMeta) -> String {
    let m = &skill.manifest.skill;
    let mut body = String::new();
    body.push_str(&format!(
        "Contributes the `{}` skill to the registry.\n\n",
        m.name
    ));
    body.push_str(&format!("- **Version**: {}\n", m.version));
    if !m.description.trim().is_empty() {
        body.push_str(&format!("- **Description**: {}\n", m.description));
    }
    if !m.author.trim().is_empty() {
        body.push_str(&format!("- **Author**: {}\n", m.author));
    }
    if !m.tags.is_empty() {
        body.push_str(&format!("- **Tags**: {}\n", m.tags.join(", ")));
    }
    body.push_str(&format!(
        "- **Runtime**: {:?}\n",
        skill.manifest.runtime.runtime_type
    ));

    // Version diff / changelog from the evolution history.
    if !evolution.versions.is_empty() {
        body.push_str("\n## Evolution history\n\n");
        // Newest last in storage; show newest first for readers.
        for entry in evolution.versions.iter().rev() {
            let who = entry
                .author
                .as_deref()
                .filter(|a| !a.is_empty())
                .map(|a| format!(" by {a}"))
                .unwrap_or_default();
            body.push_str(&format!(
                "- `{}`{} — {}\n",
                entry.version, who, entry.changelog
            ));
        }
    }

    body.push_str(&format!(
        "\nThis skill was evolved through {} mutation(s) and used {} time(s) before being proposed.\n",
        evolution.mutation_count, evolution.use_count
    ));

    body
}

// ── Validation helpers ──────────────────────────────────────────────────

/// Reject a registry slug that is not a clean `owner/name`. Guards against
/// path traversal and URL injection when the slug is interpolated into API
/// paths.
fn validate_repo_slug(slug: &str) -> Result<(), SkillError> {
    let parts: Vec<&str> = slug.split('/').collect();
    let ok = parts.len() == 2
        && parts.iter().all(|p| {
            !p.is_empty()
                && p.chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
        });
    if ok {
        Ok(())
    } else {
        Err(SkillError::InvalidManifest(format!(
            "Invalid registry repo slug '{slug}' (expected owner/name)"
        )))
    }
}

fn repo_name(slug: &str) -> &str {
    slug.split('/').next_back().unwrap_or(slug)
}

/// Turn a skill name into a safe branch-path component.
fn sanitize_branch_component(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect();
    let trimmed = cleaned.trim_matches('-');
    if trimmed.is_empty() {
        "skill".to_string()
    } else {
        trimmed.to_string()
    }
}

// ── GitHub REST client ──────────────────────────────────────────────────

/// Thin GitHub REST wrapper scoped to what the proposal flow needs.
struct RegistryGithubClient {
    http: reqwest::Client,
    token: String,
}

impl RegistryGithubClient {
    fn new(token: String) -> Self {
        Self {
            // Local timeouts (not on the shared `client_builder` default, which
            // four other callers rely on) so a hung TCP connection to GitHub
            // can't pin the `POST /api/skills/{name}/propose` handler — and a
            // Trigger lane slot — open indefinitely.
            http: crate::http_client::client_builder()
                .user_agent("librefang-skills/registry-pr")
                .connect_timeout(Duration::from_secs(10))
                .timeout(Duration::from_secs(30))
                .build()
                .expect("Failed to build HTTP client"),
            token,
        }
    }

    fn auth(&self, rb: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        rb.header("Authorization", format!("Bearer {}", self.token))
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
    }

    async fn get_json(&self, url: &str) -> Result<Value, SkillError> {
        let resp = self
            .auth(self.http.get(url))
            .send()
            .await
            .map_err(|e| SkillError::Network(format!("GitHub GET {url}: {e}")))?;
        let status = resp.status();
        if status == reqwest::StatusCode::NOT_FOUND {
            return Err(SkillError::NotFound(format!("GitHub resource: {url}")));
        }
        if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
            return Err(SkillError::SecurityBlocked(format!(
                "GitHub rejected the token ({status}) for {url}"
            )));
        }
        if !status.is_success() {
            return Err(SkillError::Network(format!(
                "GitHub GET {url} returned {status}"
            )));
        }
        resp.json()
            .await
            .map_err(|e| SkillError::Network(format!("parse GitHub response: {e}")))
    }

    async fn post_json(&self, url: &str, body: &Value) -> Result<Value, SkillError> {
        let resp = self
            .auth(self.http.post(url))
            .json(body)
            .send()
            .await
            .map_err(|e| SkillError::Network(format!("GitHub POST {url}: {e}")))?;
        self.json_or_error(resp, url).await
    }

    async fn put_json(&self, url: &str, body: &Value) -> Result<Value, SkillError> {
        let resp = self
            .auth(self.http.put(url))
            .json(body)
            .send()
            .await
            .map_err(|e| SkillError::Network(format!("GitHub PUT {url}: {e}")))?;
        self.json_or_error(resp, url).await
    }

    async fn json_or_error(&self, resp: reqwest::Response, url: &str) -> Result<Value, SkillError> {
        let status = resp.status();
        if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
            return Err(SkillError::SecurityBlocked(format!(
                "GitHub rejected the token ({status}) for {url}"
            )));
        }
        if !status.is_success() {
            let detail = resp.text().await.unwrap_or_default();
            let snippet: String = detail.chars().take(300).collect();
            return Err(SkillError::Network(format!(
                "GitHub request to {url} returned {status}: {snippet}"
            )));
        }
        resp.json()
            .await
            .map_err(|e| SkillError::Network(format!("parse GitHub response: {e}")))
    }

    async fn authenticated_login(&self) -> Result<String, SkillError> {
        let user = self.get_json(&format!("{GITHUB_API}/user")).await?;
        user["login"]
            .as_str()
            .map(|s| s.to_string())
            .ok_or_else(|| {
                SkillError::SecurityBlocked("GitHub token has no user login".to_string())
            })
    }

    /// Ensure a fork of `upstream` exists under our account. If
    /// `fork_repo` already resolves, reuse it; otherwise request a fork
    /// and poll until GitHub finishes creating it.
    async fn ensure_fork(&self, upstream: &str, fork_repo: &str) -> Result<(), SkillError> {
        if self
            .get_json(&format!("{GITHUB_API}/repos/{fork_repo}"))
            .await
            .is_ok()
        {
            return Ok(());
        }
        // Kick off the fork.
        self.post_json(&format!("{GITHUB_API}/repos/{upstream}/forks"), &json!({}))
            .await?;
        // Fork creation is async on GitHub's side — poll briefly.
        for _ in 0..20 {
            if self
                .get_json(&format!("{GITHUB_API}/repos/{fork_repo}"))
                .await
                .is_ok()
            {
                return Ok(());
            }
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        }
        Err(SkillError::Network(format!(
            "fork {fork_repo} did not become available in time"
        )))
    }

    /// Return `(default_branch, head_sha)` for the fork.
    async fn fork_default_branch_head(
        &self,
        fork_repo: &str,
    ) -> Result<(String, String), SkillError> {
        let repo = self
            .get_json(&format!("{GITHUB_API}/repos/{fork_repo}"))
            .await?;
        let default_branch = repo["default_branch"]
            .as_str()
            .unwrap_or("main")
            .to_string();
        let reference = self
            .get_json(&format!(
                "{GITHUB_API}/repos/{fork_repo}/git/ref/heads/{default_branch}"
            ))
            .await?;
        let sha = reference["object"]["sha"]
            .as_str()
            .ok_or_else(|| SkillError::Network("fork ref missing sha".to_string()))?
            .to_string();
        Ok((default_branch, sha))
    }

    async fn create_branch(
        &self,
        fork_repo: &str,
        branch: &str,
        base_sha: &str,
    ) -> Result<(), SkillError> {
        self.post_json(
            &format!("{GITHUB_API}/repos/{fork_repo}/git/refs"),
            &json!({ "ref": format!("refs/heads/{branch}"), "sha": base_sha }),
        )
        .await?;
        Ok(())
    }

    /// Create or update a file on `branch` via the Contents API. If the
    /// file already exists on the branch its blob SHA is supplied so the
    /// PUT updates rather than 422s.
    async fn put_file(
        &self,
        fork_repo: &str,
        branch: &str,
        dest_path: &str,
        contents: &[u8],
        message: &str,
    ) -> Result<(), SkillError> {
        let url = format!("{GITHUB_API}/repos/{fork_repo}/contents/{dest_path}");
        let existing_sha = self
            .get_json(&format!("{url}?ref={branch}"))
            .await
            .ok()
            .and_then(|v| v["sha"].as_str().map(|s| s.to_string()));

        let encoded = base64::engine::general_purpose::STANDARD.encode(contents);
        let mut body = json!({
            "message": message,
            "content": encoded,
            "branch": branch,
        });
        if let Some(sha) = existing_sha {
            body["sha"] = Value::String(sha);
        }
        self.put_json(&url, &body).await?;
        Ok(())
    }

    /// Open a PR upstream and return its HTML URL.
    async fn open_pull_request(
        &self,
        upstream: &str,
        base: &str,
        head: &str,
        title: &str,
        body: &str,
    ) -> Result<String, SkillError> {
        let resp = self
            .post_json(
                &format!("{GITHUB_API}/repos/{upstream}/pulls"),
                &json!({
                    "title": title,
                    "head": head,
                    "base": base,
                    "body": body,
                    "maintainer_can_modify": true,
                }),
            )
            .await?;
        resp["html_url"]
            .as_str()
            .map(|s| s.to_string())
            .ok_or_else(|| SkillError::Network("PR response missing html_url".to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::evolution::SkillVersionEntry;
    use crate::SkillManifest;
    use std::path::PathBuf;

    fn manifest_from(name: &str) -> SkillManifest {
        let toml_str = format!(
            r#"
[skill]
name = "{name}"
version = "1.2.0"
description = "Test skill"
author = "tester"
tags = ["test", "demo"]
"#
        );
        toml::from_str(&toml_str).expect("manifest parses")
    }

    fn skill_with(name: &str) -> InstalledSkill {
        InstalledSkill {
            manifest: manifest_from(name),
            path: PathBuf::from("/tmp/does-not-matter"),
            enabled: true,
        }
    }

    #[test]
    fn validate_repo_slug_accepts_owner_name() {
        assert!(validate_repo_slug("librefang/librefang-registry").is_ok());
        assert!(validate_repo_slug("acme/my.repo_1").is_ok());
    }

    #[test]
    fn validate_repo_slug_rejects_bad_input() {
        assert!(validate_repo_slug("no-slash").is_err());
        assert!(validate_repo_slug("a/b/c").is_err());
        assert!(validate_repo_slug("../../etc").is_err());
        assert!(validate_repo_slug("owner/").is_err());
        assert!(validate_repo_slug("/name").is_err());
        assert!(validate_repo_slug("owner/na me").is_err());
    }

    #[test]
    fn repo_name_extracts_trailing_segment() {
        assert_eq!(
            repo_name("librefang/librefang-registry"),
            "librefang-registry"
        );
        assert_eq!(repo_name("plain"), "plain");
    }

    #[test]
    fn sanitize_branch_component_is_path_safe() {
        assert_eq!(
            sanitize_branch_component("web/summarizer"),
            "web-summarizer"
        );
        assert_eq!(sanitize_branch_component("--weird--"), "weird");
        assert_eq!(sanitize_branch_component("///"), "skill");
        assert_eq!(sanitize_branch_component("ok_name-1"), "ok_name-1");
    }

    #[test]
    fn build_pr_body_includes_metadata_and_changelog() {
        let skill = skill_with("web-summarizer");
        let evolution = SkillEvolutionMeta {
            versions: vec![
                SkillVersionEntry {
                    version: "1.0.0".to_string(),
                    timestamp: "2026-01-01T00:00:00Z".to_string(),
                    changelog: "initial".to_string(),
                    content_hash: "h0".to_string(),
                    author: Some("cli".to_string()),
                },
                SkillVersionEntry {
                    version: "1.2.0".to_string(),
                    timestamp: "2026-02-01T00:00:00Z".to_string(),
                    changelog: "handle edge cases".to_string(),
                    content_hash: "h1".to_string(),
                    author: Some("agent:42".to_string()),
                },
            ],
            use_count: 7,
            evolution_count: 2,
            mutation_count: 1,
        };
        let body = build_pr_body(&skill, &evolution);
        assert!(body.contains("web-summarizer"));
        assert!(body.contains("1.2.0"));
        assert!(body.contains("handle edge cases"));
        assert!(body.contains("agent:42"));
        // Newest version listed before the oldest.
        let newest = body.find("1.2.0").unwrap();
        let oldest = body.find("initial").unwrap();
        assert!(newest < oldest);
        assert!(body.contains("used 7 time(s)"));
    }

    #[test]
    fn collect_skill_files_skips_junk_and_evolution_meta() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("skill.toml"), "[skill]\nname=\"x\"").unwrap();
        std::fs::write(dir.path().join("PROMPT.md"), "body").unwrap();
        std::fs::write(dir.path().join(".evolution.json"), "{}").unwrap();
        std::fs::create_dir_all(dir.path().join(".git")).unwrap();
        std::fs::write(dir.path().join(".git/config"), "junk").unwrap();
        std::fs::create_dir_all(dir.path().join("__pycache__")).unwrap();
        std::fs::write(dir.path().join("__pycache__/x.pyc"), "junk").unwrap();

        let skill = InstalledSkill {
            manifest: manifest_from("x"),
            path: dir.path().to_path_buf(),
            enabled: true,
        };
        let files = collect_skill_files(&skill).unwrap();
        let names: Vec<&str> = files.iter().map(|f| f.rel_path.as_str()).collect();
        assert!(names.contains(&"skill.toml"));
        assert!(names.contains(&"PROMPT.md"));
        assert!(!names.iter().any(|n| n.contains(".evolution.json")));
        assert!(!names.iter().any(|n| n.contains(".git")));
        assert!(!names.iter().any(|n| n.contains("__pycache__")));
        // Deterministic ordering.
        let mut sorted = names.clone();
        sorted.sort();
        assert_eq!(names, sorted);
    }
}
