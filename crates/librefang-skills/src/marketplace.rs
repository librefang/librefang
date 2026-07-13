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
    /// Base URL for the registry API.
    pub registry_url: String,
    /// GitHub organization for community skills.
    pub github_org: String,
}

impl Default for MarketplaceConfig {
    fn default() -> Self {
        Self {
            registry_url: "https://api.github.com".to_string(),
            github_org: "librefang-skills".to_string(),
        }
    }
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

    /// Search for skills by query string.
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
                    })
                    .collect()
            })
            .unwrap_or_default();

        Ok(results)
    }

    /// Install a skill from a GitHub repo by name.
    ///
    /// Downloads the latest release tarball and extracts it to the target directory.
    pub async fn install(&self, skill_name: &str, target_dir: &Path) -> Result<String, SkillError> {
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
    /// Skill name.
    pub name: String,
    /// Description.
    pub description: String,
    /// Star count.
    pub stars: u64,
    /// Repository URL.
    pub url: String,
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
