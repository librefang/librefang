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
//! Each of those steps takes its GitHub-side settings from
//! `[skills.promotion]` ([`RegistryPromotionConfig`]): the API base URL, the
//! owner the fork lands under, the branch the PR targets, the head-branch
//! prefix, the commit author, and whether to fork at all or push straight to
//! the registry.
//! Every setting is optional and reproduces the behaviour above when unset,
//! so an installation that configures nothing sees no change.
//!
//! The whole flow runs against the GitHub REST API with `reqwest` — no
//! local `git` / `gh` binary is required, so it works from inside the
//! daemon process and inside containers. Authentication uses a GitHub
//! token (`GITHUB_TOKEN`); the caller resolves the token (env or vault)
//! and passes it in.

use crate::evolution::SkillEvolutionMeta;
use crate::{InstalledSkill, SkillError};
use base64::Engine as _;
use librefang_types::config::{RegistryPromotionConfig, RegistryPromotionMode};
use serde_json::{json, Value};
use std::path::Path;
use std::time::Duration;

/// Default upstream registry repository in `owner/name` form.
pub const DEFAULT_REGISTRY_REPO: &str = "librefang/librefang-registry";

/// GitHub REST API base URL used when `skills.promotion.api_base_url` is unset.
pub const DEFAULT_GITHUB_API: &str = "https://api.github.com";

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
    /// Head branch created on the fork, or on the upstream repository itself
    /// under `skills.promotion.mode = "direct_push"`.
    pub branch: String,
}

/// Generic inputs for [`propose_files_to_registry`].
///
/// Unlike [`ProposeRequest`], this takes pre-built files and PR metadata
/// rather than a skill-specific snapshot, so the same fork/push/PR machinery
/// can be reused for agent types and any future registry content.
pub struct GenericProposeRequest<'a> {
    /// Identifier used for the branch name and the directory under `prefix/`.
    pub name: &'a str,
    /// Upstream registry repo in `owner/name` form.
    pub registry_repo: &'a str,
    /// GitHub token with `repo` scope.
    pub token: &'a str,
    /// Top-level directory under the registry root (e.g. `"skills"`, `"agent-types"`).
    pub prefix: &'a str,
    /// Files to push — `(relative_path, contents)`.
    pub files: Vec<(String, Vec<u8>)>,
    /// PR title.
    pub pr_title: String,
    /// PR body (markdown).
    pub pr_body: String,
    /// GitHub-side settings: API base URL, fork owner, base branch,
    /// head-branch prefix, commit author, fork-vs-direct-push.
    pub promotion: &'a RegistryPromotionConfig,
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
    /// GitHub-side settings: API base URL, fork owner, base branch,
    /// head-branch prefix, commit author, fork-vs-direct-push.
    pub promotion: &'a RegistryPromotionConfig,
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
    validate_registry_skill_name(&name)?;
    validate_repo_slug(req.registry_repo)?;
    if req.token.trim().is_empty() {
        return Err(SkillError::InvalidManifest(
            "A GitHub token is required to propose a skill to the registry".to_string(),
        ));
    }

    let client = RegistryGithubClient::new(req.token.to_string(), req.promotion)?;

    // 1-3. Resolve where the branch is pushed and what it is cut from —
    // fork under the configured (or derived) owner, or the upstream repo
    // itself in direct-push mode.
    let target = client
        .resolve_target(req.registry_repo, req.promotion)
        .await?;
    let branch = build_head_branch(req.promotion, "skill", &name);
    client
        .create_branch(&target.push_repo, &branch, &target.base_sha)
        .await?;

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
                &target.push_repo,
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
    let head = target.head_ref(&branch);
    let pr_url = client
        .open_pull_request(req.registry_repo, &target.base_branch, &head, &title, &body)
        .await?;

    Ok(ProposedSkillPr {
        pr_url,
        repo: req.registry_repo.to_string(),
        branch,
    })
}

/// Fork the registry, push arbitrary files to a branch, and open a PR.
///
/// This is the generic counterpart of [`propose_skill_to_registry`]: it
/// takes pre-built files and PR metadata rather than a skill-specific
/// snapshot, so the same GitHub machinery can be reused for agent types
/// (and any future registry content).
pub async fn propose_files_to_registry(
    req: GenericProposeRequest<'_>,
) -> Result<ProposedSkillPr, SkillError> {
    validate_registry_skill_name(req.name)?;
    validate_repo_slug(req.registry_repo)?;
    if req.token.trim().is_empty() {
        return Err(SkillError::InvalidManifest(
            "A GitHub token is required to propose content to the registry".to_string(),
        ));
    }
    if req.files.is_empty() {
        return Err(SkillError::InvalidManifest(format!(
            "'{}' has no files to propose",
            req.name
        )));
    }

    let client = RegistryGithubClient::new(req.token.to_string(), req.promotion)?;

    let target = client
        .resolve_target(req.registry_repo, req.promotion)
        .await?;
    let branch = build_head_branch(req.promotion, req.prefix, req.name);
    client
        .create_branch(&target.push_repo, &branch, &target.base_sha)
        .await?;

    for (rel_path, contents) in &req.files {
        let dest = format!("{}/{}/{}", req.prefix, req.name, rel_path);
        client
            .put_file(
                &target.push_repo,
                &branch,
                &dest,
                contents,
                &format!("Add {dest}"),
            )
            .await?;
    }

    let head = target.head_ref(&branch);
    let pr_url = client
        .open_pull_request(
            req.registry_repo,
            &target.base_branch,
            &head,
            &req.pr_title,
            &req.pr_body,
        )
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

/// Reject skill names that are not safe as one GitHub Contents API path
/// component. Branch-name sanitization is not sufficient because the original
/// name is also interpolated into `skills/<name>/<relative-path>`.
fn validate_registry_skill_name(name: &str) -> Result<(), SkillError> {
    let valid = !name.is_empty()
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_');
    if valid {
        Ok(())
    } else {
        Err(SkillError::InvalidManifest(format!(
            "Invalid registry skill name '{name}' (expected one [A-Za-z0-9_-] path component)"
        )))
    }
}

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

/// Reject a configured fork owner that is not a clean GitHub login or
/// organisation name. It is interpolated into API paths and into the PR's
/// `head`, so the same characters `validate_repo_slug` allows per segment are
/// the ones allowed here.
fn validate_owner(owner: &str) -> Result<(), SkillError> {
    let ok = !owner.is_empty()
        && owner
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.');
    if ok {
        Ok(())
    } else {
        Err(SkillError::InvalidManifest(format!(
            "Invalid skills.promotion.fork_owner '{owner}' (expected a GitHub login or org name)"
        )))
    }
}

/// Reject an API base URL that is not a plain `http`/`https` origin.
///
/// The value is the prefix of every request the promotion flow makes with the
/// GitHub token attached, so a scheme-less or otherwise malformed value must
/// fail loudly here rather than send the token somewhere unintended.
fn validate_api_base_url(url: &str) -> Result<(), SkillError> {
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"));
    let ok = rest.is_some_and(|host_and_path| {
        !host_and_path.is_empty()
            && !host_and_path.starts_with('/')
            && !host_and_path.contains(char::is_whitespace)
    });
    if ok {
        Ok(())
    } else {
        Err(SkillError::InvalidManifest(format!(
            "Invalid skills.promotion.api_base_url '{url}' (expected an http(s) URL such as https://github.example.com/api/v3)"
        )))
    }
}

/// Build the `author` / `committer` object sent with every Contents API
/// write, or `None` when the operator configured neither.
///
/// GitHub requires both a name and an email, so a half-configured identity is
/// dropped with a warning rather than sent as a request GitHub would reject.
fn resolve_commit_identity(cfg: &RegistryPromotionConfig) -> Option<Value> {
    let name = cfg
        .commit_author_name
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty());
    let email = cfg
        .commit_author_email
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty());
    match (name, email) {
        (Some(name), Some(email)) => Some(json!({ "name": name, "email": email })),
        (None, None) => None,
        _ => {
            tracing::warn!(
                "skills.promotion sets only one of commit_author_name / commit_author_email; \
                 GitHub requires both, so the commit will be attributed to the token owner"
            );
            None
        }
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

/// Build the timestamped head branch name, `<prefix>/<name>-<timestamp>`.
///
/// `default_prefix` is what the call site used before the setting existed:
/// the literal `skill` for skill promotions, and the registry directory for
/// the generic path.
/// A configured `head_branch_prefix` replaces it.
fn build_head_branch(cfg: &RegistryPromotionConfig, default_prefix: &str, name: &str) -> String {
    let prefix = cfg
        .head_branch_prefix
        .as_deref()
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .unwrap_or(default_prefix);
    format!(
        "{}/{}-{}",
        sanitize_branch_component(prefix),
        sanitize_branch_component(name),
        chrono::Utc::now().format("%Y%m%d%H%M%S")
    )
}

/// Where one promotion run pushes, and what its pull request targets.
struct PromotionTarget {
    /// Repository the head branch and the files are pushed to, `owner/name`.
    /// The fork in `fork` mode, the upstream registry itself in `direct_push`.
    push_repo: String,
    /// Branch the pull request targets upstream, and the branch the head
    /// branch was cut from.
    base_branch: String,
    /// Commit the head branch is cut from.
    base_sha: String,
    /// Owner qualifying the `head` field of the pull request. `Some` for a
    /// cross-repository PR from a fork, `None` when pushing directly.
    head_owner: Option<String>,
}

impl PromotionTarget {
    /// The `head` value GitHub expects: `owner:branch` across repositories,
    /// a bare branch name within one.
    fn head_ref(&self, branch: &str) -> String {
        match &self.head_owner {
            Some(owner) => format!("{owner}:{branch}"),
            None => branch.to_string(),
        }
    }
}

// ── GitHub REST client ──────────────────────────────────────────────────

/// Thin GitHub REST wrapper scoped to what the proposal flow needs.
struct RegistryGithubClient {
    http: reqwest::Client,
    token: String,
    /// REST API root, no trailing slash. `api.github.com` unless the
    /// operator points the flow at a GitHub Enterprise installation.
    api_base: String,
    /// `{"name": …, "email": …}` sent as both `author` and `committer` on
    /// every Contents API write, or `None` to let GitHub attribute the
    /// commit to the account owning the token.
    commit_identity: Option<Value>,
}

impl RegistryGithubClient {
    fn new(token: String, cfg: &RegistryPromotionConfig) -> Result<Self, SkillError> {
        let api_base = match cfg.api_base_url.as_deref().map(str::trim) {
            Some(url) if !url.is_empty() => {
                validate_api_base_url(url)?;
                url.trim_end_matches('/').to_string()
            }
            _ => DEFAULT_GITHUB_API.to_string(),
        };
        let commit_identity = resolve_commit_identity(cfg);
        Ok(Self {
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
            api_base,
            commit_identity,
        })
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
        let api = &self.api_base;
        let user = self.get_json(&format!("{api}/user")).await?;
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
        let api = &self.api_base;
        if self
            .get_json(&format!("{api}/repos/{fork_repo}"))
            .await
            .is_ok()
        {
            return Ok(());
        }
        // Kick off the fork.
        self.post_json(&format!("{api}/repos/{upstream}/forks"), &json!({}))
            .await?;
        // Fork creation is async on GitHub's side — poll briefly.
        for _ in 0..20 {
            if self
                .get_json(&format!("{api}/repos/{fork_repo}"))
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

    /// Resolve which repository the head branch is pushed to and which
    /// branch it is cut from, creating the fork on the way when the mode
    /// calls for one.
    ///
    /// In `fork` mode the push target is `<fork_owner>/<upstream repo name>`,
    /// with `fork_owner` falling back to the login `GET /user` reports — the
    /// derivation the flow has always used.
    /// In `direct_push` mode the push target is the upstream registry itself
    /// and no fork is requested.
    async fn resolve_target(
        &self,
        upstream: &str,
        cfg: &RegistryPromotionConfig,
    ) -> Result<PromotionTarget, SkillError> {
        let (push_repo, head_owner) = match cfg.mode {
            RegistryPromotionMode::DirectPush => (upstream.to_string(), None),
            RegistryPromotionMode::Fork => {
                let owner = match cfg.fork_owner.as_deref().map(str::trim) {
                    Some(o) if !o.is_empty() => {
                        validate_owner(o)?;
                        o.to_string()
                    }
                    _ => self.authenticated_login().await?,
                };
                let fork_repo = format!("{owner}/{}", repo_name(upstream));
                self.ensure_fork(upstream, &fork_repo).await?;
                (fork_repo, Some(owner))
            }
        };

        let (base_branch, base_sha) = self
            .branch_head(&push_repo, cfg.base_branch.as_deref())
            .await?;
        Ok(PromotionTarget {
            push_repo,
            base_branch,
            base_sha,
            head_owner,
        })
    }

    /// Return `(branch, head_sha)` for `repo`, using the configured base
    /// branch when there is one and the repository's own default branch
    /// otherwise.
    async fn branch_head(
        &self,
        repo_slug: &str,
        configured: Option<&str>,
    ) -> Result<(String, String), SkillError> {
        let api = &self.api_base;
        let branch = match configured.map(str::trim) {
            Some(b) if !b.is_empty() => b.to_string(),
            _ => {
                let repo = self.get_json(&format!("{api}/repos/{repo_slug}")).await?;
                repo["default_branch"]
                    .as_str()
                    .unwrap_or("main")
                    .to_string()
            }
        };
        let reference = self
            .get_json(&format!("{api}/repos/{repo_slug}/git/ref/heads/{branch}"))
            .await?;
        let sha = reference["object"]["sha"]
            .as_str()
            .ok_or_else(|| {
                SkillError::Network(format!("ref heads/{branch} on {repo_slug} missing sha"))
            })?
            .to_string();
        Ok((branch, sha))
    }

    async fn create_branch(
        &self,
        fork_repo: &str,
        branch: &str,
        base_sha: &str,
    ) -> Result<(), SkillError> {
        let api = &self.api_base;
        self.post_json(
            &format!("{api}/repos/{fork_repo}/git/refs"),
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
        let api = &self.api_base;
        let url = format!("{api}/repos/{fork_repo}/contents/{dest_path}");
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
        // Without these GitHub credits the commit to whoever owns the token,
        // which is the wrong identity for a shared or service token.
        if let Some(identity) = &self.commit_identity {
            body["author"] = identity.clone();
            body["committer"] = identity.clone();
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
        let api = &self.api_base;
        let resp = self
            .post_json(
                &format!("{api}/repos/{upstream}/pulls"),
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
    use wiremock::matchers::{method, path, path_regex};
    use wiremock::{Mock, MockServer, Request, ResponseTemplate};

    /// A skill directory with one file, ready to propose.
    fn staged_skill(dir: &tempfile::TempDir, name: &str) -> InstalledSkill {
        std::fs::write(dir.path().join("skill.toml"), "[skill]\nname=\"x\"").unwrap();
        InstalledSkill {
            manifest: manifest_from(name),
            path: dir.path().to_path_buf(),
            enabled: true,
        }
    }

    /// Mount the full happy path of a promotion run against a fake GitHub.
    ///
    /// Every response is the shape the real API returns for the one field the
    /// flow reads, so the request recorder is the only thing the tests assert
    /// on — which is the point: it is the wire, not an internal, that proves a
    /// configured value actually left the process.
    async fn mock_github() -> MockServer {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/user"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"login": "tokenowner"})))
            .mount(&server)
            .await;
        // Contents lookups 404 so `put_file` treats every write as a create.
        Mock::given(method("GET"))
            .and(path_regex(r"^/repos/[^/]+/[^/]+/contents/.*"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path_regex(r"^/repos/[^/]+/[^/]+/git/ref/heads/.*"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(json!({"object": {"sha": "basesha"}})),
            )
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path_regex(r"^/repos/[^/]+/[^/]+$"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(json!({"default_branch": "trunk"})),
            )
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path_regex(r"^/repos/[^/]+/[^/]+/git/refs$"))
            .respond_with(ResponseTemplate::new(201).set_body_json(json!({})))
            .mount(&server)
            .await;
        Mock::given(method("PUT"))
            .and(path_regex(r"^/repos/[^/]+/[^/]+/contents/.*"))
            .respond_with(ResponseTemplate::new(201).set_body_json(json!({})))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path_regex(r"^/repos/[^/]+/[^/]+/pulls$"))
            .respond_with(
                ResponseTemplate::new(201)
                    .set_body_json(json!({"html_url": "https://example.invalid/pr/1"})),
            )
            .mount(&server)
            .await;
        server
    }

    /// Every request the run made, in order.
    async fn requests(server: &MockServer) -> Vec<Request> {
        server.received_requests().await.unwrap()
    }

    /// The single request matching `method` and a path predicate.
    fn find<'a>(
        reqs: &'a [Request],
        verb: &str,
        pred: impl Fn(&str) -> bool,
    ) -> Option<&'a Request> {
        reqs.iter()
            .find(|r| r.method.as_str() == verb && pred(r.url.path()))
    }

    fn body_json(req: &Request) -> Value {
        serde_json::from_slice(&req.body).expect("request body is JSON")
    }

    async fn propose_with(cfg: &RegistryPromotionConfig) -> Result<ProposedSkillPr, SkillError> {
        let dir = tempfile::TempDir::new().unwrap();
        let skill = staged_skill(&dir, "web-summarizer");
        let evolution = SkillEvolutionMeta::default();
        propose_skill_to_registry(ProposeRequest {
            skill: &skill,
            evolution: &evolution,
            registry_repo: "acme/registry",
            token: "t0ken",
            promotion: cfg,
        })
        .await
    }

    /// With nothing configured but the API base — which every test must set to
    /// reach the mock — the run must be byte-for-byte the one the flow made
    /// before `[skills.promotion]` existed: fork under the login `GET /user`
    /// reports, base branch taken from that fork, `skill/` head prefix, and no
    /// commit author.
    #[tokio::test]
    async fn defaults_reproduce_the_pre_config_behaviour() {
        let server = mock_github().await;
        let cfg = RegistryPromotionConfig {
            api_base_url: Some(server.uri()),
            ..Default::default()
        };
        let pr = propose_with(&cfg).await.expect("proposal succeeds");
        let reqs = requests(&server).await;

        // The login was derived rather than configured.
        assert!(find(&reqs, "GET", |p| p == "/user").is_some());
        // Branch and files landed on the fork under that login.
        let create = find(&reqs, "POST", |p| p.ends_with("/git/refs")).unwrap();
        assert_eq!(create.url.path(), "/repos/tokenowner/registry/git/refs");
        assert!(pr.branch.starts_with("skill/web-summarizer-"));
        let put = find(&reqs, "PUT", |p| p.contains("/contents/")).unwrap();
        assert_eq!(
            put.url.path(),
            "/repos/tokenowner/registry/contents/skills/web-summarizer/skill.toml"
        );
        // No author is sent, so GitHub credits the token owner as before.
        let body = body_json(put);
        assert!(body.get("author").is_none(), "unexpected author: {body}");
        assert!(
            body.get("committer").is_none(),
            "unexpected committer: {body}"
        );
        // The PR targets the fork's default branch, from a cross-repo head.
        let pull = find(&reqs, "POST", |p| p.ends_with("/pulls")).unwrap();
        assert_eq!(pull.url.path(), "/repos/acme/registry/pulls");
        let body = body_json(pull);
        assert_eq!(body["base"], "trunk");
        assert_eq!(body["head"], format!("tokenowner:{}", pr.branch));
    }

    /// The injection-site test: set every value and prove each one reaches the
    /// wire. A default of `None` compiles happily while leaving the setting
    /// dead, so asserting on the request is the only check that means anything.
    #[tokio::test]
    async fn every_configured_value_reaches_the_github_request() {
        let server = mock_github().await;
        let cfg = RegistryPromotionConfig {
            api_base_url: Some(server.uri()),
            fork_owner: Some("acme-bots".to_string()),
            base_branch: Some("release".to_string()),
            head_branch_prefix: Some("promo".to_string()),
            commit_author_name: Some("LibreFang Bot".to_string()),
            commit_author_email: Some("bot@example.invalid".to_string()),
            mode: RegistryPromotionMode::Fork,
        };
        let pr = propose_with(&cfg).await.expect("proposal succeeds");
        let reqs = requests(&server).await;

        // api_base_url: the mock recorded the run, which it could not have
        // done had the requests gone to the compiled-in api.github.com — the
        // call would have failed against the real host long before returning.
        assert!(
            !reqs.is_empty(),
            "no request reached the configured API base"
        );
        // And `GET /user` was not needed, because the owner was configured.
        assert!(find(&reqs, "GET", |p| p == "/user").is_none());

        // fork_owner: the fork, the branch and the files live under it.
        let create = find(&reqs, "POST", |p| p.ends_with("/git/refs")).unwrap();
        assert_eq!(create.url.path(), "/repos/acme-bots/registry/git/refs");

        // base_branch: cut from `release`, and the PR targets `release`.
        assert!(find(&reqs, "GET", |p| p
            == "/repos/acme-bots/registry/git/ref/heads/release")
        .is_some());
        let pull = body_json(find(&reqs, "POST", |p| p.ends_with("/pulls")).unwrap());
        assert_eq!(pull["base"], "release");

        // head_branch_prefix.
        assert!(
            pr.branch.starts_with("promo/web-summarizer-"),
            "branch was {}",
            pr.branch
        );
        assert_eq!(pull["head"], format!("acme-bots:{}", pr.branch));

        // commit author / committer.
        let put = body_json(find(&reqs, "PUT", |p| p.contains("/contents/")).unwrap());
        let expected = json!({"name": "LibreFang Bot", "email": "bot@example.invalid"});
        assert_eq!(put["author"], expected);
        assert_eq!(put["committer"], expected);
    }

    /// `direct_push` skips the fork entirely: the branch is created on the
    /// upstream registry and the PR head carries no owner prefix.
    #[tokio::test]
    async fn direct_push_mode_never_forks() {
        let server = mock_github().await;
        let cfg = RegistryPromotionConfig {
            api_base_url: Some(server.uri()),
            mode: RegistryPromotionMode::DirectPush,
            ..Default::default()
        };
        let pr = propose_with(&cfg).await.expect("proposal succeeds");
        let reqs = requests(&server).await;

        assert!(
            find(&reqs, "POST", |p| p.ends_with("/forks")).is_none(),
            "direct_push must not request a fork"
        );
        assert!(find(&reqs, "GET", |p| p == "/user").is_none());
        let create = find(&reqs, "POST", |p| p.ends_with("/git/refs")).unwrap();
        assert_eq!(create.url.path(), "/repos/acme/registry/git/refs");
        let pull = body_json(find(&reqs, "POST", |p| p.ends_with("/pulls")).unwrap());
        assert_eq!(pull["head"], pr.branch);
    }

    #[test]
    fn half_configured_commit_author_is_dropped() {
        let both = RegistryPromotionConfig {
            commit_author_name: Some("Bot".to_string()),
            commit_author_email: Some("bot@example.invalid".to_string()),
            ..Default::default()
        };
        assert_eq!(
            resolve_commit_identity(&both),
            Some(json!({"name": "Bot", "email": "bot@example.invalid"}))
        );
        let name_only = RegistryPromotionConfig {
            commit_author_name: Some("Bot".to_string()),
            ..Default::default()
        };
        assert_eq!(resolve_commit_identity(&name_only), None);
        let blank = RegistryPromotionConfig {
            commit_author_name: Some("  ".to_string()),
            commit_author_email: Some("  ".to_string()),
            ..Default::default()
        };
        assert_eq!(resolve_commit_identity(&blank), None);
    }

    /// The GitHub Enterprise shape: the base carries a path prefix, and a
    /// trailing slash must not double up when the request path is appended.
    #[test]
    fn enterprise_api_base_is_stored_verbatim_minus_the_trailing_slash() {
        let cfg = RegistryPromotionConfig {
            api_base_url: Some("https://github.example.invalid/api/v3/".to_string()),
            ..Default::default()
        };
        let client = RegistryGithubClient::new("t0ken".to_string(), &cfg).unwrap();
        assert_eq!(client.api_base, "https://github.example.invalid/api/v3");

        let unset = RegistryGithubClient::new("t0ken".to_string(), &Default::default()).unwrap();
        assert_eq!(unset.api_base, DEFAULT_GITHUB_API);
    }

    #[test]
    fn api_base_url_must_be_an_http_origin() {
        for good in [
            "https://api.github.com",
            "http://localhost:8080",
            "https://github.example.com/api/v3",
        ] {
            assert!(validate_api_base_url(good).is_ok(), "{good}");
        }
        for bad in [
            "",
            "api.github.com",
            "ftp://example.invalid",
            "https://",
            "http:///api",
            "https://exa mple.invalid",
            "file:///etc/passwd",
        ] {
            assert!(validate_api_base_url(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn fork_owner_must_be_a_plain_login() {
        assert!(validate_owner("acme-bots").is_ok());
        assert!(validate_owner("acme.bots_1").is_ok());
        for bad in ["", "acme/bots", "../etc", "acme bots", "acme?x=1"] {
            assert!(validate_owner(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn head_branch_prefix_falls_back_to_the_call_site_default() {
        let cfg = RegistryPromotionConfig::default();
        assert!(build_head_branch(&cfg, "agent-types", "helper").starts_with("agent-types/helper-"));
        let blank = RegistryPromotionConfig {
            head_branch_prefix: Some("   ".to_string()),
            ..Default::default()
        };
        assert!(build_head_branch(&blank, "skill", "helper").starts_with("skill/helper-"));
        let set = RegistryPromotionConfig {
            head_branch_prefix: Some("contrib/x".to_string()),
            ..Default::default()
        };
        assert!(build_head_branch(&set, "skill", "helper").starts_with("contrib-x/helper-"));
    }

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
    fn registry_skill_name_must_be_one_safe_path_component() {
        for valid in ["web-summarizer", "skill_1", "Skill2"] {
            assert!(validate_registry_skill_name(valid).is_ok(), "{valid}");
        }
        for invalid in [
            "",
            "../escape",
            "nested/skill",
            r"nested\skill",
            "bad.name",
            "bad name",
            "nul\0name",
            "ümlaut",
        ] {
            assert!(
                validate_registry_skill_name(invalid).is_err(),
                "{invalid:?}"
            );
        }
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
