//! Skillhub marketplace client — search and install skills from lightmake.site.
//!
//! Skillhub shares the same API format as ClawHub for search, detail, and download.
//! Browse uses a static index hosted on Tencent COS.
//!
//! API endpoints:
//! - Search: `GET /api/v1/search?q=...&limit=20`
//! - Detail: `GET /api/v1/skills/{slug}`
//! - Download: `GET /api/v1/download?slug=...`
//! - Browse: static JSON at COS bucket

use crate::clawhub::{
    ClawHubClient, ClawHubInstallResult, ClawHubSearchResponse, ClawHubSkillDetail,
};
use crate::SkillError;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tracing::info;

/// Default Skillhub API base URL.
pub const DEFAULT_SKILLHUB_URL: &str = "https://lightmake.site/api/v1";

/// Static skills index URL (Tencent COS).
const SKILLHUB_INDEX_URL: &str =
    "https://skillhub-1388575217.cos.ap-guangzhou.myqcloud.com/skills.json";

// ---------------------------------------------------------------------------
// Browse response types (static index format)
// ---------------------------------------------------------------------------

/// A skill entry from the Skillhub static index.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillhubBrowseEntry {
    #[serde(default)]
    pub rank: u32,
    pub slug: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub homepage: String,
    #[serde(default)]
    pub downloads: u64,
    #[serde(default)]
    pub stars: u64,
    #[serde(default)]
    pub score: f64,
    #[serde(default)]
    pub categories: Vec<String>,
}

/// Response from the Skillhub static skills index.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillhubIndexResponse {
    #[serde(default)]
    pub total: u32,
    #[serde(default)]
    pub skills: Vec<SkillhubBrowseEntry>,
}

// ---------------------------------------------------------------------------
// Client
// ---------------------------------------------------------------------------

/// Client for the Skillhub marketplace (lightmake.site).
///
/// Delegates search, detail, and install to [`ClawHubClient`] (compatible API),
/// and provides browse via the static COS-hosted skills index.
pub struct SkillhubClient {
    /// Inner ClawHub client pointed at the Skillhub API URL.
    inner: ClawHubClient,
    /// Separate HTTP client for the static index fetch.
    http: reqwest::Client,
}

impl SkillhubClient {
    /// Create a new Skillhub client.
    ///
    /// `base_url` is the Skillhub API base (default: `https://lightmake.site/api/v1`).
    pub fn new(base_url: &str, cache_dir: PathBuf) -> Self {
        Self {
            inner: ClawHubClient::with_url(base_url, cache_dir),
            http: crate::http_client::client_builder()
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .expect("HTTP client build"),
        }
    }

    /// Create a Skillhub client with the default URL.
    pub fn with_defaults(cache_dir: PathBuf) -> Self {
        Self::new(DEFAULT_SKILLHUB_URL, cache_dir)
    }

    // -- Delegated to ClawHubClient (compatible APIs) -----------------------

    /// Search skills on Skillhub (compatible with ClawHub search API).
    pub async fn search(
        &self,
        query: &str,
        limit: u32,
    ) -> Result<ClawHubSearchResponse, SkillError> {
        self.inner.search(query, limit).await
    }

    /// Get detailed info about a specific skill.
    pub async fn get_skill(&self, slug: &str) -> Result<ClawHubSkillDetail, SkillError> {
        self.inner.get_skill(slug).await
    }

    /// Install a skill from Skillhub.
    ///
    /// After download and security scan, patches the source provenance to
    /// `SkillSource::Skillhub` in the generated `skill.toml`.
    pub async fn install(
        &self,
        slug: &str,
        target_dir: &Path,
    ) -> Result<ClawHubInstallResult, SkillError> {
        let result = self.inner.install(slug, target_dir).await?;

        // Post-install fixup: update source provenance from ClawHub -> Skillhub
        let skill_dir = target_dir.join(slug);
        let manifest_path = skill_dir.join("skill.toml");
        if manifest_path.exists() {
            if let Ok(toml_str) = std::fs::read_to_string(&manifest_path) {
                if let Ok(mut manifest) = toml::from_str::<crate::SkillManifest>(&toml_str) {
                    manifest.source = Some(crate::SkillSource::Skillhub {
                        slug: slug.to_string(),
                        version: result.version.clone(),
                    });
                    if let Ok(updated) = toml::to_string_pretty(&manifest) {
                        let _ = std::fs::write(&manifest_path, updated);
                    }
                }
            }
        }

        Ok(result)
    }

    /// Check if a skill is already installed locally.
    pub fn is_installed(&self, slug: &str, skills_dir: &Path) -> bool {
        self.inner.is_installed(slug, skills_dir)
    }

    // -- Skillhub-specific: browse via static index -------------------------

    /// Browse skills from the static Skillhub index.
    ///
    /// Supports client-side sorting by "downloads", "stars", "score", or
    /// default rank order ("trending").
    pub async fn browse(
        &self,
        sort: &str,
        limit: u32,
    ) -> Result<SkillhubIndexResponse, SkillError> {
        let resp = self
            .http
            .get(SKILLHUB_INDEX_URL)
            .header("User-Agent", "LibreFang/0.1")
            .send()
            .await
            .map_err(|e| SkillError::Network(format!("Skillhub index fetch failed: {e}")))?;

        if !resp.status().is_success() {
            return Err(SkillError::Network(format!(
                "Skillhub index returned {}",
                resp.status()
            )));
        }

        let mut data: SkillhubIndexResponse = resp
            .json()
            .await
            .map_err(|e| SkillError::Network(format!("Skillhub index parse error: {e}")))?;

        // Client-side sort
        match sort {
            "downloads" => data.skills.sort_by(|a, b| b.downloads.cmp(&a.downloads)),
            "stars" => data.skills.sort_by(|a, b| b.stars.cmp(&a.stars)),
            "score" => data.skills.sort_by(|a, b| {
                b.score
                    .partial_cmp(&a.score)
                    .unwrap_or(std::cmp::Ordering::Equal)
            }),
            _ => {} // default rank order = "trending"
        }

        data.skills.truncate(limit as usize);
        info!(
            sort,
            limit,
            total = data.total,
            returned = data.skills.len(),
            "Skillhub browse loaded"
        );
        Ok(data)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_skillhub_index_parse() {
        let json = r#"{
            "total": 2,
            "skills": [
                {
                    "rank": 1,
                    "slug": "rust",
                    "name": "Rust",
                    "description": "Write idiomatic Rust",
                    "version": "1.0.1",
                    "homepage": "",
                    "downloads": 1271,
                    "stars": 4,
                    "score": 0.85,
                    "categories": ["coding"]
                },
                {
                    "rank": 2,
                    "slug": "python",
                    "name": "Python",
                    "description": "Python best practices",
                    "version": "1.0.0",
                    "homepage": "",
                    "downloads": 500,
                    "stars": 10,
                    "score": 0.70,
                    "categories": ["coding"]
                }
            ]
        }"#;

        let resp: SkillhubIndexResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.total, 2);
        assert_eq!(resp.skills.len(), 2);
        assert_eq!(resp.skills[0].slug, "rust");
        assert_eq!(resp.skills[0].downloads, 1271);
        assert_eq!(resp.skills[1].stars, 10);
    }

    #[test]
    fn test_skillhub_browse_entry_minimal() {
        // Minimal fields — everything except slug has defaults
        let json = r#"{"slug": "test"}"#;
        let entry: SkillhubBrowseEntry = serde_json::from_str(json).unwrap();
        assert_eq!(entry.slug, "test");
        assert_eq!(entry.rank, 0);
        assert_eq!(entry.downloads, 0);
    }

    #[test]
    fn test_skillhub_client_creation() {
        let client = SkillhubClient::with_defaults(PathBuf::from("/tmp/cache"));
        // Just verify it doesn't panic
        assert!(!client.is_installed("nonexistent", Path::new("/tmp/nope")));
    }
}
