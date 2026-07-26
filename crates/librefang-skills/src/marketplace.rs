//! FangHub marketplace client — install skills from the registry.
//!
//! For Phase 1, uses GitHub releases as the registry backend.
//! Each skill is a GitHub repo with releases containing the skill bundle.

use crate::openclaw_compat;
use crate::supply_chain;
use crate::SkillError;
use reqwest::StatusCode;
use serde_json::json;
use std::io::{Cursor, Read};
use std::path::{Component, Path, PathBuf};
use tracing::info;

/// Maximum size of a downloaded release bundle (compressed bytes).
/// Bounds the in-memory download buffer against a huge release asset.
const MAX_DOWNLOAD_BYTES: u64 = 256 * 1024 * 1024;

/// Maximum number of entries permitted in a bundle zip.
/// Guards against an archive with an absurd number of tiny entries.
/// `pub(crate)` so the ClawHub/Skillhub install path shares the same caps
/// (single source of truth — see [`write_zip_entry_capped`]).
pub(crate) const MAX_ENTRIES: usize = 10_000;

/// Maximum uncompressed size of any single zip entry.
pub(crate) const MAX_ENTRY_UNCOMPRESSED_BYTES: u64 = 128 * 1024 * 1024;

/// Maximum cumulative uncompressed size across all entries in a bundle.
pub(crate) const MAX_TOTAL_UNCOMPRESSED_BYTES: u64 = 512 * 1024 * 1024;

/// Maximum per-entry uncompressed:compressed ratio before an entry is
/// treated as a decompression bomb.
pub(crate) const MAX_COMPRESSION_RATIO: u64 = 100;

/// Stream one zip entry to `out_path` with decompression-bomb guards, shared
/// by both skill-install zip extractors (marketplace bundles and
/// ClawHub/Skillhub skills) so the caps cannot drift between them.
///
/// Rejects (with [`SkillError::SecurityBlocked`], removing any partial file)
/// when the entry's declared size exceeds the per-entry cap, its
/// uncompressed:compressed ratio exceeds [`MAX_COMPRESSION_RATIO`], the
/// streamed bytes exceed the per-entry cap (defeats a lying header via a
/// bounded `take`), or the running `total_uncompressed` exceeds the bundle
/// cap. `std::io::copy` streams through a small buffer, so a bomb cannot
/// allocate its full decompressed length in RAM. The caller passes
/// `declared_size` / `compressed_size` (from the zip header) and a mutable
/// running total.
pub(crate) fn write_zip_entry_capped<R: std::io::Read>(
    entry: &mut R,
    declared_size: u64,
    compressed_size: u64,
    out_path: &Path,
    entry_label: &str,
    total_uncompressed: &mut u64,
) -> Result<(), SkillError> {
    if declared_size > MAX_ENTRY_UNCOMPRESSED_BYTES {
        return Err(SkillError::SecurityBlocked(format!(
            "zip entry '{entry_label}' declares {declared_size} uncompressed bytes, exceeding the {MAX_ENTRY_UNCOMPRESSED_BYTES}-byte per-entry limit"
        )));
    }
    if compressed_size > 0 && declared_size / compressed_size > MAX_COMPRESSION_RATIO {
        return Err(SkillError::SecurityBlocked(format!(
            "zip entry '{entry_label}' has a compression ratio above {MAX_COMPRESSION_RATIO}:1 (possible decompression bomb)"
        )));
    }
    let mut out_file = std::fs::File::create(out_path)?;
    let mut limited = entry.take(MAX_ENTRY_UNCOMPRESSED_BYTES + 1);
    let written = std::io::copy(&mut limited, &mut out_file).map_err(SkillError::Io)?;
    if written > MAX_ENTRY_UNCOMPRESSED_BYTES {
        let _ = std::fs::remove_file(out_path);
        return Err(SkillError::SecurityBlocked(format!(
            "zip entry '{entry_label}' exceeded the {MAX_ENTRY_UNCOMPRESSED_BYTES}-byte per-entry decompression limit"
        )));
    }
    *total_uncompressed = total_uncompressed.saturating_add(written);
    if *total_uncompressed > MAX_TOTAL_UNCOMPRESSED_BYTES {
        return Err(SkillError::SecurityBlocked(format!(
            "bundle exceeded the {MAX_TOTAL_UNCOMPRESSED_BYTES}-byte total decompression limit"
        )));
    }
    Ok(())
}

fn urlencoded(s: &str) -> String {
    use std::fmt::Write;

    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(char::from(b));
            }
            _ => {
                let _ = write!(&mut out, "%{:02X}", b);
            }
        }
    }
    out
}

/// FangHub registry configuration.
#[derive(Debug, Clone)]
pub struct MarketplaceConfig {
    /// Base URL for the registry API, used by the GitHub-releases fallback.
    pub registry_url: String,
    /// GitHub organization holding one repo per skill, each publishing bundles as releases.
    /// Only the [`MarketplaceClient::install`] fallback uses it — the first-class source is the synced registry checkout below.
    pub github_org: String,
    /// Local checkout of the LibreFang registry — `~/.librefang/registry`, kept current by `librefang_runtime::registry_sync`.
    /// Skills live at `<dir>/skills/<name>/` as `SKILL.md` (a few as `skill.toml`).
    ///
    /// This is the authoritative source for search and install (#6569): it is forge-agnostic (it honours `registry.registry_host`, so a Codeberg mirror works), needs no network at query time, and holds the same skills the dashboard's `GET /api/skills/registry` already lists.
    /// `None` means the caller has no home directory to read, which limits search to an error and install to the remote fallback.
    pub registry_dir: Option<PathBuf>,
}

impl Default for MarketplaceConfig {
    fn default() -> Self {
        Self {
            registry_url: "https://api.github.com".to_string(),
            github_org: "librefang-skills".to_string(),
            registry_dir: None,
        }
    }
}

impl MarketplaceConfig {
    /// Point the client at a synced registry checkout (`~/.librefang/registry`).
    pub fn with_registry_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.registry_dir = Some(dir.into());
        self
    }

    /// `<registry_dir>/skills`, when a registry directory is configured.
    fn registry_skills_dir(&self) -> Option<PathBuf> {
        self.registry_dir.as_ref().map(|d| d.join("skills"))
    }
}

/// Copy a skill directory tree, skipping symlinks.
///
/// A symlink inside a registry checkout would otherwise be copied as a link pointing outside the installed skill directory (or followed into a cycle), which the supply-chain audit cannot reason about.
/// Skills are plain files; dropping links is the conservative choice.
fn copy_dir_recursive(src: &Path, dest: &Path) -> Result<(), SkillError> {
    std::fs::create_dir_all(dest)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        if file_type.is_symlink() {
            tracing::warn!(
                path = %entry.path().display(),
                "skipping symlink while installing skill"
            );
            continue;
        }
        let src_path = entry.path();
        let dest_path = dest.join(entry.file_name());
        if file_type.is_dir() {
            copy_dir_recursive(&src_path, &dest_path)?;
        } else {
            std::fs::copy(&src_path, &dest_path)?;
        }
    }
    Ok(())
}

/// Whether `source` looks like a git remote rather than a skill name or path.
///
/// The CLI advertises `librefang skill install https://github.com/user/skill.git` (see `cli.rs`'s `Install` long_about), but nothing implemented it: a URL fell through to the name-based marketplace install, which pasted it into `{org}/{name}` and produced a nonsense request URL (#6569).
pub fn looks_like_git_url(source: &str) -> bool {
    let s = source.trim();
    s.starts_with("git@")
        || s.starts_with("ssh://")
        || s.starts_with("git://")
        || ((s.starts_with("http://") || s.starts_with("https://")) && s.ends_with(".git"))
}

/// Derive a skill directory name from a git remote.
///
/// `https://github.com/user/my-skill.git` → `my-skill`.
/// Returns `None` when the last path segment is empty or is not a safe single path component.
pub fn skill_name_from_git_url(url: &str) -> Option<String> {
    let trimmed = url.trim().trim_end_matches('/');
    let without_git = trimmed.strip_suffix(".git").unwrap_or(trimmed);
    let last = without_git
        .rsplit(['/', ':'])
        .next()
        .map(str::trim)
        .filter(|s| !s.is_empty())?;
    if is_safe_component(last) {
        Some(last.to_string())
    } else {
        None
    }
}

/// Name + description read from a registry skill directory.
///
/// Accepts both on-disk shapes: `SKILL.md` with YAML frontmatter (what the registry actually ships — 61 of 61 entries at the time of writing) and the native `skill.toml`.
/// Returns `None` for a directory holding neither.
fn read_registry_skill_meta(dir: &Path) -> Option<(String, String)> {
    let dir_name = dir.file_name()?.to_string_lossy().to_string();

    if openclaw_compat::detect_skillmd(dir) {
        let path = dir.join("SKILL.md");
        if let Ok((frontmatter, _body)) = openclaw_compat::parse_skillmd(&path) {
            let name = if frontmatter.name.trim().is_empty() {
                dir_name
            } else {
                frontmatter.name.trim().to_string()
            };
            return Some((name, frontmatter.description));
        }
        // Frontmatter that doesn't parse still names an installable directory.
        return Some((dir_name, String::new()));
    }

    let manifest_path = dir.join("skill.toml");
    if manifest_path.exists() {
        if let Ok(content) = std::fs::read_to_string(&manifest_path) {
            if let Ok(manifest) = toml::from_str::<crate::SkillManifest>(&content) {
                return Some((manifest.skill.name, manifest.skill.description));
            }
        }
        return Some((dir_name, String::new()));
    }

    None
}

/// Client for the FangHub marketplace.
pub struct MarketplaceClient {
    config: MarketplaceConfig,
    http: reqwest::Client,
}

/// Parameters for publishing a bundle to a GitHub-backed FangHub repo.
pub struct MarketplacePublishRequest<'a> {
    /// GitHub repo in `owner/name` form.
    pub repo: &'a str,
    /// Release tag to create or update.
    pub tag: &'a str,
    /// Path to the bundle zip archive on disk.
    pub bundle_path: &'a Path,
    /// Release title shown on GitHub.
    pub release_name: &'a str,
    /// Release notes/body.
    pub release_notes: &'a str,
    /// GitHub token with repo release permissions.
    pub token: &'a str,
}

/// Result of publishing a skill bundle.
#[derive(Debug, Clone)]
pub struct PublishedRelease {
    /// GitHub repo that owns the release.
    pub repo: String,
    /// Release tag.
    pub tag: String,
    /// Uploaded asset file name.
    pub asset_name: String,
    /// GitHub HTML URL for the release page.
    pub html_url: String,
}

impl MarketplaceClient {
    /// Create a new marketplace client.
    pub fn new(config: MarketplaceConfig) -> Self {
        Self {
            config,
            http: crate::http_client::client_builder()
                .user_agent("librefang-skills/0.1")
                .build()
                .expect("Failed to build HTTP client"),
        }
    }

    /// Search the synced registry checkout for skills matching `query`.
    ///
    /// An empty query lists everything.
    /// Matching is a case-insensitive substring test over name and description, mirroring `GET /api/marketplace/search`.
    ///
    /// Reads `~/.librefang/registry/skills/` rather than a forge search API (#6569): the previous implementation queried GitHub's `/search/repositories?q=…+org:librefang-skills`, and that organization does not exist — GitHub answers `422 Unprocessable Entity` for an `org:` qualifier naming a missing org, so every search failed.
    /// The checkout is also what `librefang skill install` and the dashboard read, so all three now agree on the same catalog.
    pub fn search_registry(&self, query: &str) -> Result<Vec<SkillSearchResult>, SkillError> {
        let Some(skills_dir) = self.config.registry_skills_dir() else {
            return Err(SkillError::NotFound(
                "No registry directory configured — cannot search the local skill registry"
                    .to_string(),
            ));
        };
        if !skills_dir.exists() {
            return Err(SkillError::NotFound(format!(
                "Registry not synced yet: {} does not exist. Start the daemon once, or run `librefang catalog update`, to fetch it.",
                skills_dir.display()
            )));
        }

        let needle = query.trim().to_lowercase();
        let mut results: Vec<SkillSearchResult> = Vec::new();
        let entries = std::fs::read_dir(&skills_dir).map_err(SkillError::Io)?;
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let Some((name, description)) = read_registry_skill_meta(&path) else {
                continue;
            };
            if !needle.is_empty()
                && !name.to_lowercase().contains(&needle)
                && !description.to_lowercase().contains(&needle)
            {
                continue;
            }
            let dir_name = path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            results.push(SkillSearchResult {
                name,
                description,
                // The registry carries no popularity signal; the field stays for the GitHub-releases shape that does.
                stars: 0,
                // Deliberately empty rather than the on-disk path.
                // A caller that serves these rows over HTTP (the dashboard's `GET /api/marketplace/search`) would otherwise leak the daemon's home directory layout to every client, and no caller needs it — `installable_id` is what you act on.
                url: String::new(),
                installable_id: Some(dir_name),
            });
        }
        // Deterministic output — `read_dir` order is filesystem-dependent.
        results.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(results)
    }

    /// Search a GitHub organization's skill repos (legacy remote shape).
    ///
    /// Retained for a deployment that actually hosts one repo per skill under `github_org`.
    /// `search_registry` is the path the CLI takes.
    pub async fn search(&self, query: &str) -> Result<Vec<SkillSearchResult>, SkillError> {
        let encoded_query = urlencoded(query);
        let url = format!(
            "{}/search/repositories?q={}+org:{}&sort=stars",
            self.config.registry_url, encoded_query, self.config.github_org
        );

        let resp = self
            .http
            .get(&url)
            .header("Accept", "application/vnd.github.v3+json")
            .send()
            .await
            .map_err(|e| SkillError::Network(format!("Search request failed: {e}")))?;

        if !resp.status().is_success() {
            return Err(SkillError::Network(format!(
                "Search returned status {}",
                resp.status()
            )));
        }

        let body: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| SkillError::Network(format!("Parse search response: {e}")))?;

        let results = body["items"]
            .as_array()
            .map(|items| {
                items
                    .iter()
                    .map(|item| SkillSearchResult {
                        name: item["name"].as_str().unwrap_or("").to_string(),
                        description: item["description"].as_str().unwrap_or("").to_string(),
                        stars: item["stargazers_count"].as_u64().unwrap_or(0),
                        url: item["html_url"].as_str().unwrap_or("").to_string(),
                        installable_id: None,
                    })
                    .collect()
            })
            .unwrap_or_default();

        Ok(results)
    }

    /// Install a skill by name, preferring the synced registry checkout.
    ///
    /// Resolution order (#6569):
    ///
    /// 1. `<registry_dir>/skills/<name>/` — copy the directory and normalize its manifest (`SKILL.md` is converted to `skill.toml`).
    ///    This is where the registry's skills actually live, and it matches what `POST /api/skills/install` already did.
    /// 2. GitHub releases under `github_org` — the historical path, kept for a deployment that publishes one repo per skill.
    ///    Note the default org (`librefang-skills`) does not exist, so this leg 404s on a stock install; the registry checkout is what makes `install` work.
    pub async fn install(&self, skill_name: &str, target_dir: &Path) -> Result<String, SkillError> {
        if let Some(registry_skills) = self.config.registry_skills_dir() {
            let source = resolve_skill_dir(&registry_skills, skill_name)?;
            if source.is_dir() {
                return self.install_from_registry_dir(&source, skill_name, target_dir);
            }
        }
        let repo = format!("{}/{}", self.config.github_org, skill_name);
        let url = format!(
            "{}/repos/{}/releases/latest",
            self.config.registry_url, repo
        );

        info!("Fetching skill info from {url}");

        let resp = self
            .http
            .get(&url)
            .header("Accept", "application/vnd.github.v3+json")
            .send()
            .await
            .map_err(|e| SkillError::Network(format!("Fetch release: {e}")))?;

        if !resp.status().is_success() {
            return Err(SkillError::NotFound(format!(
                "Skill '{skill_name}' not found in marketplace (status {})",
                resp.status()
            )));
        }

        let release: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| SkillError::Network(format!("Parse release: {e}")))?;

        let version = release["tag_name"]
            .as_str()
            .unwrap_or("unknown")
            .to_string();

        let skill_dir = resolve_skill_dir(target_dir, skill_name)?;
        if skill_dir.exists() {
            std::fs::remove_dir_all(&skill_dir)?;
        }
        std::fs::create_dir_all(&skill_dir)?;

        let (download_url, source_kind) = find_release_download_url(&release).ok_or_else(|| {
            SkillError::Network("No zip asset or zipball URL in release".to_string())
        })?;

        info!("Downloading skill {skill_name} {version} from {source_kind}...");
        let bundle_bytes = self.download_bytes(&download_url).await?;

        extract_bundle_zip_bytes(&bundle_bytes, &skill_dir)?;
        ensure_skill_manifest(&skill_dir)?;

        // Supply-chain audit — refuse install if any critical violation is found.
        // Override with LIBREFANG_SKIP_SUPPLY_CHAIN_AUDIT=1 for dev-mode only.
        if let Err(violations) = supply_chain::scan(&skill_dir) {
            // Clean up the partially-extracted directory so a failed install
            // does not leave a malicious bundle on disk.
            let _ = std::fs::remove_dir_all(&skill_dir);
            let summary = violations
                .iter()
                .map(|v| v.to_string())
                .collect::<Vec<_>>()
                .join("; ");
            return Err(SkillError::SecurityBlocked(format!(
                "supply-chain audit failed for '{skill_name}': {summary}"
            )));
        }

        let meta = serde_json::json!({
            "name": skill_name,
            "version": version,
            "source": download_url,
            "source_kind": source_kind,
            "installed_at": chrono::Utc::now().to_rfc3339(),
        });
        let meta_path = resolve_skill_child_path(&skill_dir, Path::new("marketplace_meta.json"))?;
        std::fs::write(
            meta_path,
            serde_json::to_string_pretty(&meta).unwrap_or_default(),
        )?;

        info!("Installed skill: {skill_name} {version}");
        Ok(version)
    }

    /// Copy one skill out of the synced registry checkout into `target_dir`.
    ///
    /// Normalizes the manifest through `ensure_skill_manifest`, so the registry's `SKILL.md` shape becomes a native `skill.toml` on install, and runs the same supply-chain audit as the remote path — a registry checkout is still third-party content pulled off a forge.
    fn install_from_registry_dir(
        &self,
        source: &Path,
        skill_name: &str,
        target_dir: &Path,
    ) -> Result<String, SkillError> {
        let skill_dir = resolve_skill_dir(target_dir, skill_name)?;
        if skill_dir.exists() {
            std::fs::remove_dir_all(&skill_dir)?;
        }
        std::fs::create_dir_all(&skill_dir)?;

        copy_dir_recursive(source, &skill_dir)?;
        ensure_skill_manifest(&skill_dir)?;

        if let Err(violations) = supply_chain::scan(&skill_dir) {
            let _ = std::fs::remove_dir_all(&skill_dir);
            let summary = violations
                .iter()
                .map(|v| v.to_string())
                .collect::<Vec<_>>()
                .join("; ");
            return Err(SkillError::SecurityBlocked(format!(
                "supply-chain audit failed for '{skill_name}': {summary}"
            )));
        }

        let version = std::fs::read_to_string(skill_dir.join("skill.toml"))
            .ok()
            .and_then(|c| toml::from_str::<crate::SkillManifest>(&c).ok())
            .map(|m| m.skill.version)
            .unwrap_or_else(|| "registry".to_string());

        let meta = serde_json::json!({
            "name": skill_name,
            "version": version,
            "source": source.display().to_string(),
            "source_kind": "registry",
            "installed_at": chrono::Utc::now().to_rfc3339(),
        });
        let meta_path = resolve_skill_child_path(&skill_dir, Path::new("marketplace_meta.json"))?;
        std::fs::write(
            meta_path,
            serde_json::to_string_pretty(&meta).unwrap_or_default(),
        )?;

        info!("Installed skill from registry: {skill_name} {version}");
        Ok(version)
    }

    /// Install a skill by cloning a git remote — the form the CLI advertises (`librefang skill install https://github.com/user/skill.git`) but that was never implemented (#6569).
    ///
    /// Shallow-clones into a temporary directory next to the target, drops the `.git` metadata, normalizes the manifest (so a `SKILL.md`-only repo works), then runs the same supply-chain audit as every other install path.
    pub fn install_from_git(&self, url: &str, target_dir: &Path) -> Result<String, SkillError> {
        let skill_name = skill_name_from_git_url(url).ok_or_else(|| {
            SkillError::InvalidManifest(format!(
                "Cannot derive a skill name from '{url}' — clone it manually and install the directory"
            ))
        })?;
        let skill_dir = resolve_skill_dir(target_dir, &skill_name)?;

        // Clone into a sibling staging dir so a failed clone never leaves a half-populated skill directory behind.
        // The `.staging-` prefix is the repo's existing convention, not a new one: `SkillRegistry::load_all` sweeps `.staging-*` / `.installing-*` leftovers on every load, so a clone interrupted by a crash or Ctrl-C gets cleaned up on the next registry load instead of accumulating forever (#3719).
        let staging = target_dir.join(format!(".staging-{skill_name}"));
        if staging.exists() {
            std::fs::remove_dir_all(&staging)?;
        }

        info!("Cloning skill from {url}...");
        let output = std::process::Command::new("git")
            .arg("clone")
            .arg("--depth")
            .arg("1")
            .arg("--")
            .arg(url)
            .arg(&staging)
            .output()
            .map_err(|e| SkillError::Network(format!("Failed to run git: {e}")))?;
        if !output.status.success() {
            let _ = std::fs::remove_dir_all(&staging);
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            return Err(SkillError::Network(format!(
                "git clone of '{url}' failed: {stderr}"
            )));
        }

        // The clone's history is not part of the skill.
        let _ = std::fs::remove_dir_all(staging.join(".git"));

        let install = (|| -> Result<String, SkillError> {
            if skill_dir.exists() {
                std::fs::remove_dir_all(&skill_dir)?;
            }
            std::fs::create_dir_all(&skill_dir)?;
            copy_dir_recursive(&staging, &skill_dir)?;
            ensure_skill_manifest(&skill_dir)?;

            if let Err(violations) = supply_chain::scan(&skill_dir) {
                let _ = std::fs::remove_dir_all(&skill_dir);
                let summary = violations
                    .iter()
                    .map(|v| v.to_string())
                    .collect::<Vec<_>>()
                    .join("; ");
                return Err(SkillError::SecurityBlocked(format!(
                    "supply-chain audit failed for '{skill_name}': {summary}"
                )));
            }

            let version = std::fs::read_to_string(skill_dir.join("skill.toml"))
                .ok()
                .and_then(|c| toml::from_str::<crate::SkillManifest>(&c).ok())
                .map(|m| m.skill.version)
                .unwrap_or_else(|| "git".to_string());

            let meta = serde_json::json!({
                "name": skill_name,
                "version": version,
                "source": url,
                "source_kind": "git",
                "installed_at": chrono::Utc::now().to_rfc3339(),
            });
            let meta_path =
                resolve_skill_child_path(&skill_dir, Path::new("marketplace_meta.json"))?;
            std::fs::write(
                meta_path,
                serde_json::to_string_pretty(&meta).unwrap_or_default(),
            )?;
            Ok(version)
        })();

        let _ = std::fs::remove_dir_all(&staging);
        let version = install?;
        info!("Installed skill from git: {skill_name} {version}");
        Ok(version)
    }

    /// Publish a skill bundle to a GitHub-backed FangHub repository release.
    pub async fn publish_bundle(
        &self,
        request: MarketplacePublishRequest<'_>,
    ) -> Result<PublishedRelease, SkillError> {
        let bundle_bytes = std::fs::read(request.bundle_path)?;
        let asset_name = request
            .bundle_path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| {
                SkillError::InvalidManifest(format!(
                    "Invalid bundle filename: {}",
                    request.bundle_path.display()
                ))
            })?
            .to_string();

        let release = match self
            .github_get_json(
                &format!(
                    "{}/repos/{}/releases/tags/{}",
                    self.config.registry_url, request.repo, request.tag
                ),
                request.token,
            )
            .await
        {
            Ok(release) => release,
            Err(SkillError::NotFound(_)) => {
                self.github_post_json(
                    &format!(
                        "{}/repos/{}/releases",
                        self.config.registry_url, request.repo
                    ),
                    request.token,
                    &json!({
                        "tag_name": request.tag,
                        "name": request.release_name,
                        "body": request.release_notes,
                        "draft": false,
                        "prerelease": false
                    }),
                )
                .await?
            }
            Err(err) => return Err(err),
        };

        if let Some(asset_id) = find_existing_asset_id(&release, &asset_name) {
            self.github_delete(
                &format!(
                    "{}/repos/{}/releases/assets/{}",
                    self.config.registry_url, request.repo, asset_id
                ),
                request.token,
            )
            .await?;
        }

        let upload_url = release["upload_url"]
            .as_str()
            .ok_or_else(|| SkillError::Network("Release missing upload URL".to_string()))?;
        let upload_url = upload_url
            .split('{')
            .next()
            .ok_or_else(|| SkillError::Network("Invalid release upload URL".to_string()))?;

        let upload_resp = self
            .http
            .post(format!("{upload_url}?name={asset_name}"))
            .header("Authorization", format!("Bearer {}", request.token))
            .header("Accept", "application/vnd.github+json")
            .header("Content-Type", "application/zip")
            .body(bundle_bytes)
            .send()
            .await
            .map_err(|e| SkillError::Network(format!("Upload asset: {e}")))?;

        if !upload_resp.status().is_success() {
            return Err(SkillError::Network(format!(
                "Upload asset failed with status {}",
                upload_resp.status()
            )));
        }

        Ok(PublishedRelease {
            repo: request.repo.to_string(),
            tag: request.tag.to_string(),
            asset_name,
            html_url: release["html_url"].as_str().unwrap_or_default().to_string(),
        })
    }

    async fn download_bytes(&self, url: &str) -> Result<Vec<u8>, SkillError> {
        let resp = self
            .http
            .get(url)
            .header("Accept", "application/vnd.github+json")
            .send()
            .await
            .map_err(|e| SkillError::Network(format!("Download request failed: {e}")))?;
        let mut resp = resp
            .error_for_status()
            .map_err(|e| SkillError::Network(format!("Download failed: {e}")))?;

        // Early reject when the server advertises a body larger than the cap.
        if let Some(len) = resp.content_length() {
            if len > MAX_DOWNLOAD_BYTES {
                return Err(SkillError::Network(format!(
                    "download size {len} bytes exceeds the {MAX_DOWNLOAD_BYTES}-byte limit"
                )));
            }
        }

        // Stream the body chunk-by-chunk so an absent or lying Content-Length
        // header cannot force an unbounded in-memory buffer.
        let mut buf: Vec<u8> = Vec::new();
        while let Some(chunk) = resp
            .chunk()
            .await
            .map_err(|e| SkillError::Network(format!("Download stream failed: {e}")))?
        {
            if buf.len() as u64 + chunk.len() as u64 > MAX_DOWNLOAD_BYTES {
                return Err(SkillError::Network(format!(
                    "download exceeded the {MAX_DOWNLOAD_BYTES}-byte limit"
                )));
            }
            buf.extend_from_slice(&chunk);
        }
        Ok(buf)
    }

    async fn github_get_json(
        &self,
        url: &str,
        token: &str,
    ) -> Result<serde_json::Value, SkillError> {
        let resp = self
            .http
            .get(url)
            .header("Authorization", format!("Bearer {token}"))
            .header("Accept", "application/vnd.github+json")
            .send()
            .await
            .map_err(|e| SkillError::Network(format!("GitHub GET failed: {e}")))?;

        if resp.status() == StatusCode::NOT_FOUND {
            return Err(SkillError::NotFound(format!(
                "GitHub resource not found: {url}"
            )));
        }
        if !resp.status().is_success() {
            return Err(SkillError::Network(format!(
                "GitHub GET returned status {}",
                resp.status()
            )));
        }

        resp.json()
            .await
            .map_err(|e| SkillError::Network(format!("Parse GitHub response: {e}")))
    }

    async fn github_post_json(
        &self,
        url: &str,
        token: &str,
        body: &serde_json::Value,
    ) -> Result<serde_json::Value, SkillError> {
        let resp = self
            .http
            .post(url)
            .header("Authorization", format!("Bearer {token}"))
            .header("Accept", "application/vnd.github+json")
            .json(body)
            .send()
            .await
            .map_err(|e| SkillError::Network(format!("GitHub POST failed: {e}")))?;

        if !resp.status().is_success() {
            return Err(SkillError::Network(format!(
                "GitHub POST returned status {}",
                resp.status()
            )));
        }

        resp.json()
            .await
            .map_err(|e| SkillError::Network(format!("Parse GitHub response: {e}")))
    }

    async fn github_delete(&self, url: &str, token: &str) -> Result<(), SkillError> {
        let resp = self
            .http
            .delete(url)
            .header("Authorization", format!("Bearer {token}"))
            .header("Accept", "application/vnd.github+json")
            .send()
            .await
            .map_err(|e| SkillError::Network(format!("GitHub DELETE failed: {e}")))?;

        if !resp.status().is_success() {
            return Err(SkillError::Network(format!(
                "GitHub DELETE returned status {}",
                resp.status()
            )));
        }

        Ok(())
    }
}

/// A search result from the marketplace.
#[derive(Debug, Clone)]
pub struct SkillSearchResult {
    /// Skill name — the manifest / frontmatter name, which may differ from the directory name.
    pub name: String,
    /// Description.
    pub description: String,
    /// Star count.
    /// Always 0 for registry-checkout results (no popularity signal on disk); populated by the GitHub-org search.
    pub stars: u64,
    /// Repository URL, or the on-disk path for a registry-checkout result.
    pub url: String,
    /// The identifier to pass back to `install` — the registry directory name.
    /// `None` for a GitHub-org result, whose repo name is already `name`.
    pub installable_id: Option<String>,
}

fn find_release_download_url(release: &serde_json::Value) -> Option<(String, &'static str)> {
    if let Some(assets) = release["assets"].as_array() {
        if let Some(asset) = assets.iter().find(|asset| {
            asset["name"]
                .as_str()
                .map(|name| name.ends_with(".zip"))
                .unwrap_or(false)
        }) {
            let url = asset["browser_download_url"].as_str()?.to_string();
            return Some((url, "release-asset"));
        }
    }

    release["zipball_url"]
        .as_str()
        .map(|url| (url.to_string(), "release-zipball"))
}

fn find_existing_asset_id(release: &serde_json::Value, asset_name: &str) -> Option<u64> {
    release["assets"].as_array()?.iter().find_map(|asset| {
        let name = asset["name"].as_str()?;
        if name == asset_name {
            asset["id"].as_u64()
        } else {
            None
        }
    })
}

fn extract_bundle_zip_bytes(bytes: &[u8], skill_dir: &Path) -> Result<(), SkillError> {
    let reader = Cursor::new(bytes);
    let mut archive = zip::ZipArchive::new(reader)
        .map_err(|err| SkillError::InvalidManifest(format!("Read bundle zip: {err}")))?;

    if archive.len() > MAX_ENTRIES {
        return Err(SkillError::SecurityBlocked(format!(
            "bundle contains {} entries, exceeding the {MAX_ENTRIES}-entry limit",
            archive.len()
        )));
    }

    let mut safe_paths = Vec::new();
    for index in 0..archive.len() {
        let file = archive
            .by_index(index)
            .map_err(|err| SkillError::InvalidManifest(format!("Read zip entry: {err}")))?;
        if let Some(path) = sanitize_zip_path(file.name()) {
            safe_paths.push(path);
        }
    }
    let shared_root = detect_shared_root(&safe_paths);

    let mut total_uncompressed: u64 = 0;
    for index in 0..archive.len() {
        let mut file = archive
            .by_index(index)
            .map_err(|err| SkillError::InvalidManifest(format!("Read zip entry: {err}")))?;
        let Some(mut relative_path) = sanitize_zip_path(file.name()) else {
            continue;
        };
        if let Some(ref root) = shared_root {
            if let Ok(stripped) = relative_path.strip_prefix(root) {
                relative_path = stripped.to_path_buf();
            }
        }
        if relative_path.as_os_str().is_empty() {
            continue;
        }

        let out_path = resolve_skill_child_path(skill_dir, &relative_path)?;
        if file.is_dir() {
            std::fs::create_dir_all(&out_path)?;
            continue;
        }

        if let Some(parent) = out_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        // Stream to disk with the shared decompression-bomb guards (declared
        // size, ratio, bounded `take`, running total).
        let declared = file.size();
        let compressed = file.compressed_size();
        write_zip_entry_capped(
            &mut file,
            declared,
            compressed,
            &out_path,
            &relative_path.display().to_string(),
            &mut total_uncompressed,
        )?;
    }

    Ok(())
}

fn sanitize_zip_path(name: &str) -> Option<std::path::PathBuf> {
    let mut clean = std::path::PathBuf::new();
    for component in std::path::Path::new(name).components() {
        match component {
            std::path::Component::Normal(part) => clean.push(part),
            std::path::Component::CurDir => {}
            _ => return None,
        }
    }

    if clean.as_os_str().is_empty() {
        None
    } else {
        Some(clean)
    }
}

fn is_safe_component(name: &str) -> bool {
    !name.is_empty()
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

fn resolve_skill_dir(target_dir: &Path, skill_name: &str) -> Result<PathBuf, SkillError> {
    if !is_safe_component(skill_name) {
        return Err(SkillError::InvalidManifest(format!(
            "Invalid skill name '{skill_name}'"
        )));
    }
    Ok(target_dir.join(skill_name))
}

fn resolve_skill_child_path(skill_dir: &Path, relative: &Path) -> Result<PathBuf, SkillError> {
    if relative.is_absolute() {
        return Err(SkillError::InvalidManifest(
            "Absolute paths are not allowed in skill bundles".to_string(),
        ));
    }
    if relative
        .components()
        .any(|c| !matches!(c, Component::Normal(_)))
    {
        return Err(SkillError::InvalidManifest(format!(
            "Unsafe path component in bundle entry '{}'",
            relative.display()
        )));
    }
    Ok(skill_dir.join(relative))
}

fn detect_shared_root(paths: &[std::path::PathBuf]) -> Option<std::path::PathBuf> {
    let first_component = paths.iter().find_map(|path| {
        path.components()
            .next()
            .map(|component| component.as_os_str().to_owned())
    })?;

    if paths.iter().all(|path| {
        path.components()
            .next()
            .map(|component| component.as_os_str() == first_component.as_os_str())
            .unwrap_or(false)
    }) && paths.iter().any(|path| path.components().count() > 1)
    {
        Some(std::path::PathBuf::from(first_component))
    } else {
        None
    }
}

fn ensure_skill_manifest(skill_dir: &Path) -> Result<(), SkillError> {
    if skill_dir.join("skill.toml").exists() {
        return Ok(());
    }

    if openclaw_compat::detect_skillmd(skill_dir) {
        let converted = openclaw_compat::convert_skillmd(skill_dir)?;
        openclaw_compat::write_librefang_manifest(skill_dir, &converted.manifest)?;
        return Ok(());
    }

    if openclaw_compat::detect_openclaw_skill(skill_dir) {
        let manifest = openclaw_compat::convert_openclaw_skill(skill_dir)?;
        openclaw_compat::write_librefang_manifest(skill_dir, &manifest)?;
        return Ok(());
    }

    Err(SkillError::InvalidManifest(format!(
        "Installed bundle in {} did not contain a loadable skill",
        skill_dir.display()
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::TempDir;
    use zip::write::SimpleFileOptions;

    #[test]
    fn test_default_config() {
        let config = MarketplaceConfig::default();
        assert!(config.registry_url.contains("github"));
        assert_eq!(config.github_org, "librefang-skills");
        assert!(config.registry_dir.is_none());
    }

    // ---------------------------------------------------------------------
    // #6569 — registry-checkout search + install
    // ---------------------------------------------------------------------

    /// Write a SKILL.md-shaped registry entry — the format all 61 registry skills actually use.
    fn write_skillmd_entry(registry_skills: &Path, dir: &str, name: &str, description: &str) {
        let skill_dir = registry_skills.join(dir);
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(
            skill_dir.join("SKILL.md"),
            format!(
                "---\nname: {name}\ndescription: {description}\n---\n\n# {name}\n\nDo the thing.\n"
            ),
        )
        .unwrap();
    }

    fn client_with_registry(registry_dir: &Path) -> MarketplaceClient {
        MarketplaceClient::new(MarketplaceConfig::default().with_registry_dir(registry_dir))
    }

    #[test]
    fn search_registry_lists_skillmd_entries() {
        let tmp = TempDir::new().unwrap();
        let registry = tmp.path().join("registry");
        let skills = registry.join("skills");
        write_skillmd_entry(&skills, "web-search", "web-search", "Search the web");
        write_skillmd_entry(&skills, "code-reviewer", "code-reviewer", "Review diffs");

        let results = client_with_registry(&registry).search_registry("").unwrap();
        let names: Vec<&str> = results.iter().map(|r| r.name.as_str()).collect();
        // Sorted, so the assertion is stable across filesystems.
        assert_eq!(names, vec!["code-reviewer", "web-search"]);
        assert_eq!(
            results[1].installable_id.as_deref(),
            Some("web-search"),
            "the install id is the registry directory name"
        );
    }

    #[test]
    fn search_registry_matches_name_and_description_case_insensitively() {
        let tmp = TempDir::new().unwrap();
        let registry = tmp.path().join("registry");
        let skills = registry.join("skills");
        write_skillmd_entry(&skills, "web-search", "web-search", "Search the web");
        write_skillmd_entry(&skills, "postgres-expert", "postgres-expert", "SQL tuning");

        let client = client_with_registry(&registry);
        // By name.
        let by_name = client.search_registry("WEB").unwrap();
        assert_eq!(by_name.len(), 1);
        assert_eq!(by_name[0].name, "web-search");
        // By description.
        let by_desc = client.search_registry("tuning").unwrap();
        assert_eq!(by_desc.len(), 1);
        assert_eq!(by_desc[0].name, "postgres-expert");
        // No match.
        assert!(client.search_registry("nonexistent").unwrap().is_empty());
    }

    #[test]
    fn search_registry_also_reads_skill_toml_entries() {
        let tmp = TempDir::new().unwrap();
        let registry = tmp.path().join("registry");
        let skills = registry.join("skills");
        let dir = skills.join("native-skill");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("skill.toml"),
            "[skill]\nname = \"native-skill\"\nversion = \"0.2.0\"\ndescription = \"Native manifest\"\n",
        )
        .unwrap();

        let results = client_with_registry(&registry)
            .search_registry("native")
            .unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].description, "Native manifest");
    }

    /// The git-clone staging directory must use the repo's `.staging-` prefix so `SkillRegistry::load_all`'s stale-dir sweep reclaims it after an interrupted clone (#3719).
    #[test]
    fn git_staging_dir_uses_the_sweepable_prefix() {
        // Asserted on the constructed name rather than by running a clone, which
        // would need a live remote.
        let name = skill_name_from_git_url("https://github.com/user/my-skill.git").unwrap();
        let staging = format!(".staging-{name}");
        assert!(
            staging.starts_with(".staging-"),
            "load_all only sweeps `.staging-*` / `.installing-*`, got {staging}"
        );
    }

    /// A registry row must not carry the daemon's filesystem layout: the same rows are served by `GET /api/marketplace/search`.
    #[test]
    fn search_registry_does_not_leak_on_disk_paths() {
        let tmp = TempDir::new().unwrap();
        let registry = tmp.path().join("registry");
        write_skillmd_entry(&registry.join("skills"), "web-search", "web-search", "");

        let results = client_with_registry(&registry).search_registry("").unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].url, "", "got {:?}", results[0].url);
    }

    #[test]
    fn search_registry_skips_directories_with_no_manifest() {
        let tmp = TempDir::new().unwrap();
        let registry = tmp.path().join("registry");
        let skills = registry.join("skills");
        std::fs::create_dir_all(skills.join("not-a-skill")).unwrap();
        write_skillmd_entry(&skills, "real", "real", "");

        let results = client_with_registry(&registry).search_registry("").unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "real");
    }

    #[test]
    fn search_registry_reports_an_unsynced_registry() {
        let tmp = TempDir::new().unwrap();
        let err = client_with_registry(&tmp.path().join("missing"))
            .search_registry("anything")
            .expect_err("an unsynced registry must not look like an empty catalog");
        assert!(matches!(err, SkillError::NotFound(_)), "got {err:?}");
    }

    #[test]
    fn search_registry_without_a_configured_dir_is_an_error() {
        let client = MarketplaceClient::new(MarketplaceConfig::default());
        assert!(client.search_registry("x").is_err());
    }

    /// The reported failure: `librefang skill install web-search` 404'd because install went to the nonexistent `librefang-skills` GitHub org, while the skill sat in the synced registry checkout the whole time.
    #[tokio::test]
    async fn install_prefers_the_registry_checkout_and_converts_skillmd() {
        let tmp = TempDir::new().unwrap();
        let registry = tmp.path().join("registry");
        write_skillmd_entry(
            &registry.join("skills"),
            "web-search",
            "web-search",
            "Search the web",
        );
        let target = tmp.path().join("installed");
        std::fs::create_dir_all(&target).unwrap();

        let version = client_with_registry(&registry)
            .install("web-search", &target)
            .await
            .expect("install from the registry checkout");

        let installed = target.join("web-search");
        assert!(
            installed.join("SKILL.md").exists(),
            "source files are copied"
        );
        assert!(
            installed.join("skill.toml").exists(),
            "SKILL.md must be normalized into a native manifest"
        );
        let meta: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(installed.join("marketplace_meta.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(meta["source_kind"], "registry");
        assert!(!version.is_empty());
    }

    #[tokio::test]
    async fn install_rejects_a_path_traversal_name() {
        let tmp = TempDir::new().unwrap();
        let registry = tmp.path().join("registry");
        std::fs::create_dir_all(registry.join("skills")).unwrap();
        let target = tmp.path().join("installed");
        std::fs::create_dir_all(&target).unwrap();

        let err = client_with_registry(&registry)
            .install("../escape", &target)
            .await
            .expect_err("a traversal name must not reach Path::join");
        assert!(matches!(err, SkillError::InvalidManifest(_)), "got {err:?}");
    }

    #[test]
    fn looks_like_git_url_recognises_the_advertised_forms() {
        // The CLI's own help text advertises this one.
        assert!(looks_like_git_url("https://github.com/user/skill.git"));
        assert!(looks_like_git_url("git@github.com:user/skill.git"));
        assert!(looks_like_git_url("ssh://git@host/user/skill"));
        assert!(looks_like_git_url("git://host/user/skill"));
        // A bare name or a local path is not a remote.
        assert!(!looks_like_git_url("web-search"));
        assert!(!looks_like_git_url("./my-skill"));
        // A plain https URL without .git is a web page, not a clone target.
        assert!(!looks_like_git_url("https://github.com/user/skill"));
    }

    #[test]
    fn skill_name_from_git_url_takes_the_last_segment() {
        assert_eq!(
            skill_name_from_git_url("https://github.com/user/my-skill.git").as_deref(),
            Some("my-skill")
        );
        assert_eq!(
            skill_name_from_git_url("git@github.com:user/my-skill.git").as_deref(),
            Some("my-skill")
        );
        assert_eq!(
            skill_name_from_git_url("https://host/user/my-skill/").as_deref(),
            Some("my-skill")
        );
        // Nothing usable to name a directory after.
        assert_eq!(skill_name_from_git_url("https://host/.git"), None);
        assert_eq!(skill_name_from_git_url(""), None);
    }

    #[test]
    fn copy_dir_recursive_skips_symlinks() {
        let tmp = TempDir::new().unwrap();
        let src = tmp.path().join("src");
        std::fs::create_dir_all(src.join("nested")).unwrap();
        std::fs::write(src.join("SKILL.md"), "---\nname: x\n---\n").unwrap();
        std::fs::write(src.join("nested/file.txt"), "data").unwrap();
        let outside = tmp.path().join("outside.txt");
        std::fs::write(&outside, "secret").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&outside, src.join("link.txt")).unwrap();

        let dest = tmp.path().join("dest");
        copy_dir_recursive(&src, &dest).unwrap();

        assert!(dest.join("SKILL.md").exists());
        assert!(dest.join("nested/file.txt").exists());
        #[cfg(unix)]
        assert!(
            !dest.join("link.txt").exists(),
            "a symlink pointing outside the skill must not be copied"
        );
    }

    /// Regression (#6441 follow-up): the shared zip-entry writer — used by both
    /// the marketplace and ClawHub/Skillhub install paths — must reject
    /// decompression bombs (oversized declared header, high compression ratio)
    /// while still writing a normal entry.
    #[test]
    fn write_zip_entry_capped_blocks_bombs() {
        let dir = TempDir::new().unwrap();

        // Oversized declared header → blocked before any bytes are streamed.
        let mut total = 0u64;
        let mut src = &b"tiny"[..];
        let err = write_zip_entry_capped(
            &mut src,
            MAX_ENTRY_UNCOMPRESSED_BYTES + 1,
            1,
            &dir.path().join("big"),
            "big",
            &mut total,
        )
        .unwrap_err();
        assert!(matches!(err, SkillError::SecurityBlocked(_)), "got {err:?}");

        // High compression ratio (declared within the per-entry cap) → blocked.
        let mut src = &b"tiny"[..];
        let err = write_zip_entry_capped(
            &mut src,
            1_000_000,
            100, // ratio 10_000:1
            &dir.path().join("ratio"),
            "ratio",
            &mut total,
        )
        .unwrap_err();
        assert!(matches!(err, SkillError::SecurityBlocked(_)), "got {err:?}");

        // A normal small entry writes and advances the running total.
        let mut src = &b"hello world"[..];
        write_zip_entry_capped(
            &mut src,
            11,
            11,
            &dir.path().join("ok.txt"),
            "ok.txt",
            &mut total,
        )
        .expect("a normal entry must be written");
        assert_eq!(total, 11, "running total must track written bytes");
        assert_eq!(
            std::fs::read_to_string(dir.path().join("ok.txt")).unwrap(),
            "hello world"
        );
    }

    #[test]
    fn test_client_creation() {
        let client = MarketplaceClient::new(MarketplaceConfig::default());
        assert_eq!(client.config.github_org, "librefang-skills");
    }

    #[test]
    fn test_urlencoded() {
        assert_eq!(urlencoded("twitter"), "twitter");
        assert_eq!(urlencoded("hello world"), "hello%20world");
        assert_eq!(urlencoded("social&media"), "social%26media");
        assert_eq!(urlencoded("key=value"), "key%3Dvalue");
        assert_eq!(urlencoded("what?now#frag"), "what%3Fnow%23frag");
    }

    #[test]
    fn test_search_query_encoding() {
        let client = MarketplaceClient::new(MarketplaceConfig::default());
        let query = "social&media tools";
        let url = format!(
            "{}/search/repositories?q={}+org:{}&sort=stars",
            client.config.registry_url,
            urlencoded(query),
            client.config.github_org
        );

        assert!(url.contains("q=social%26media%20tools+org:librefang-skills"));
        assert!(!url.contains("social&media tools"));
    }

    #[test]
    fn test_find_release_download_url_prefers_zip_asset() {
        let release = json!({
            "assets": [
                {
                    "name": "skill.zip",
                    "browser_download_url": "https://example.com/skill.zip"
                }
            ],
            "zipball_url": "https://example.com/source.zip"
        });

        let (url, kind) = find_release_download_url(&release).unwrap();
        assert_eq!(url, "https://example.com/skill.zip");
        assert_eq!(kind, "release-asset");
    }

    #[test]
    fn test_extract_bundle_zip_bytes_strips_single_root_directory() {
        let dir = TempDir::new().unwrap();
        let zip_path = dir.path().join("bundle.zip");

        {
            let file = std::fs::File::create(&zip_path).unwrap();
            let mut zip = zip::ZipWriter::new(file);
            let options = SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated)
                .unix_permissions(0o644);
            zip.start_file("repo-root/skill.toml", options).unwrap();
            zip.write_all(
                br#"[skill]
name = "zip-skill"
version = "0.1.0"
"#,
            )
            .unwrap();
            zip.finish().unwrap();
        }

        let bytes = std::fs::read(&zip_path).unwrap();
        let skill_dir = dir.path().join("installed");
        std::fs::create_dir_all(&skill_dir).unwrap();
        extract_bundle_zip_bytes(&bytes, &skill_dir).unwrap();

        assert!(skill_dir.join("skill.toml").exists());
    }

    #[test]
    fn test_extract_bundle_zip_bytes_rejects_decompression_bomb() {
        let dir = TempDir::new().unwrap();
        let zip_path = dir.path().join("bomb.zip");

        // 1 MiB of zeros deflates to a few hundred bytes — a ratio well above
        // MAX_COMPRESSION_RATIO — modelling a classic decompression bomb.
        {
            let file = std::fs::File::create(&zip_path).unwrap();
            let mut zip = zip::ZipWriter::new(file);
            let options = SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated)
                .unix_permissions(0o644);
            zip.start_file("skill.toml", options).unwrap();
            let chunk = vec![0u8; 64 * 1024];
            for _ in 0..16 {
                zip.write_all(&chunk).unwrap();
            }
            zip.finish().unwrap();
        }

        let bytes = std::fs::read(&zip_path).unwrap();
        let skill_dir = dir.path().join("installed");
        std::fs::create_dir_all(&skill_dir).unwrap();

        let err = extract_bundle_zip_bytes(&bytes, &skill_dir)
            .expect_err("decompression bomb must be rejected");
        assert!(
            matches!(err, SkillError::SecurityBlocked(_)),
            "expected SecurityBlocked, got {err:?}"
        );
    }
}
