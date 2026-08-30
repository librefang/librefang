//! Skillhub marketplace client — search and install skills from skillhub.tencent.com.
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
    ClawHubClient, ClawHubInstallResult, ClawHubSearchEntry, ClawHubSearchResponse,
    ClawHubSkillDetail,
};
use crate::SkillError;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tracing::{info, warn};

/// Default Skillhub API base URL.
pub const DEFAULT_SKILLHUB_URL: &str = "https://skillhub.tencent.com/api/v1";

/// Default static skills index URL (Tencent COS).
pub const DEFAULT_SKILLHUB_INDEX_URL: &str =
    "https://skillhub-1388575217.cos.ap-guangzhou.myqcloud.com/skills.json";

/// Default COS accelerate base URL for skill zip downloads.
pub const DEFAULT_SKILLHUB_COS_BASE: &str =
    "https://skillhub-1388575217.cos.accelerate.myqcloud.com";

/// Environment variable for the Skillhub API base — search and skill detail.
pub const ENV_SKILLHUB_URL: &str = "LIBREFANG_SKILLHUB_URL";

/// Environment variable for the static skills index — browse, and the version lookup that install starts from.
pub const ENV_SKILLHUB_INDEX_URL: &str = "LIBREFANG_SKILLHUB_INDEX_URL";

/// Environment variable for the object-storage base that skill archives are downloaded from.
pub const ENV_SKILLHUB_COS_URL: &str = "LIBREFANG_SKILLHUB_COS_URL";

fn atomic_write_manifest(path: &Path, contents: &[u8]) -> Result<(), SkillError> {
    use std::io::Write;
    use std::sync::atomic::{AtomicU64, Ordering};

    static SEQ: AtomicU64 = AtomicU64::new(0);
    let parent = crate::resolve_parent_or_cwd(path);
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    let tmp = parent.join(format!(".skill.toml.tmp.{}.{}", std::process::id(), seq));

    let write_result = (|| -> std::io::Result<()> {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)?;
        file.write_all(contents)?;
        file.sync_all()
    })();
    if let Err(error) = write_result {
        let _ = std::fs::remove_file(&tmp);
        return Err(SkillError::InvalidManifest(format!(
            "Skillhub: write skill.toml staging file: {error}"
        )));
    }

    if let Err(error) = std::fs::rename(&tmp, path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(SkillError::InvalidManifest(format!(
            "Skillhub: replace skill.toml: {error}"
        )));
    }

    #[cfg(unix)]
    std::fs::File::open(parent)
        .and_then(|dir| dir.sync_all())
        .map_err(|error| {
            SkillError::InvalidManifest(format!(
                "Skillhub: sync skill.toml parent directory: {error}"
            ))
        })?;

    Ok(())
}

fn patch_skillhub_provenance(
    skill_dir: &Path,
    slug: &str,
    version: &str,
) -> Result<(), SkillError> {
    let manifest_path = skill_dir.join("skill.toml");
    if !manifest_path.exists() {
        return Ok(());
    }

    let toml_str = std::fs::read_to_string(&manifest_path).map_err(|e| {
        SkillError::InvalidManifest(format!(
            "Skillhub: read skill.toml for provenance patch: {e}"
        ))
    })?;
    let mut manifest: crate::SkillManifest = toml::from_str(&toml_str).map_err(|e| {
        SkillError::InvalidManifest(format!(
            "Skillhub: parse skill.toml for provenance patch: {e}"
        ))
    })?;
    manifest.source = Some(crate::SkillSource::Skillhub {
        slug: slug.to_string(),
        version: version.to_string(),
    });
    let updated = toml::to_string_pretty(&manifest).map_err(|e| {
        SkillError::InvalidManifest(format!(
            "Skillhub: serialize skill.toml for provenance patch: {e}"
        ))
    })?;
    atomic_write_manifest(&manifest_path, updated.as_bytes())
}

fn patch_or_cleanup_skillhub_install(
    skill_dir: PathBuf,
    slug: String,
    version: String,
) -> Result<(), SkillError> {
    if let Err(error) = patch_skillhub_provenance(&skill_dir, &slug, &version) {
        if let Err(cleanup_err) = std::fs::remove_dir_all(&skill_dir) {
            warn!(
                slug = %slug,
                skill_dir = %skill_dir.display(),
                error = %cleanup_err,
                "Skillhub: provenance patch failed AND cleanup failed; skill directory left on disk with wrong source provenance, manual removal needed"
            );
        }
        return Err(error);
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Search response types (SkillHub-native format)
// ---------------------------------------------------------------------------

/// A skill entry from the SkillHub search API (snake_case, may differ from ClawHub).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillhubSearchEntry {
    pub slug: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub score: f64,
    #[serde(default)]
    pub updated_at: i64,
}

/// Response from the SkillHub search API.
/// Supports both `results` (ClawHub-compatible) and `skills` (SkillHub-native) keys.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillhubSearchResponse {
    #[serde(default, alias = "skills")]
    pub results: Vec<SkillhubSearchEntry>,
}

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

/// Client for the Skillhub marketplace (skillhub.tencent.com).
///
/// Delegates search, detail, and install to [`ClawHubClient`] (compatible API),
/// and provides browse via the static COS-hosted skills index.
pub struct SkillhubClient {
    /// Inner ClawHub client pointed at the Skillhub API URL.
    inner: ClawHubClient,
    /// Separate HTTP client for the static index fetch.
    http: reqwest::Client,
    /// Base API URL (e.g. `https://skillhub.tencent.com/api/v1`).
    base_url: String,
    /// Static skills index URL — the source for browse and for the version install resolves.
    index_url: String,
    /// Object-storage base that skill archives are downloaded from.
    cos_base: String,
}

impl SkillhubClient {
    /// Create a new Skillhub client.
    ///
    /// `base_url` is the Skillhub API base (default: `https://skillhub.tencent.com/api/v1`).
    pub fn new(base_url: &str, cache_dir: PathBuf) -> Self {
        Self {
            inner: ClawHubClient::with_url(base_url, cache_dir),
            http: crate::http_client::client_builder()
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .expect("HTTP client build"),
            base_url: base_url.trim_end_matches('/').to_string(),
            index_url: crate::clawhub::env_url_or(
                ENV_SKILLHUB_INDEX_URL,
                DEFAULT_SKILLHUB_INDEX_URL,
            ),
            cos_base: crate::clawhub::env_url_or(ENV_SKILLHUB_COS_URL, DEFAULT_SKILLHUB_COS_BASE),
        }
    }

    /// Create a Skillhub client from the configured Skillhub endpoints.
    ///
    /// Skillhub is not one host: search and detail come from the API base, browse and install's version lookup come from a static index, and archives come from object storage.
    /// All three are overridable — `LIBREFANG_SKILLHUB_URL`, `LIBREFANG_SKILLHUB_INDEX_URL`, `LIBREFANG_SKILLHUB_COS_URL` — because pointing only the API base at a mirror would leave browse and install still aimed at the dead host, which is the opposite of the recovery an override is for.
    pub fn with_defaults(cache_dir: PathBuf) -> Self {
        Self::new(
            &crate::clawhub::env_url_or(ENV_SKILLHUB_URL, DEFAULT_SKILLHUB_URL),
            cache_dir,
        )
    }

    // -- Delegated to ClawHubClient (compatible APIs) -----------------------

    /// Search skills on Skillhub.
    ///
    /// Overrides the ClawHub delegation to add `Accept: application/json` header,
    /// which prevents Skillhub from returning HTML instead of JSON. Also handles
    /// the SkillHub-native response format (snake_case, `skills` key) as a fallback
    /// to the ClawHub-compatible format (camelCase, `results` key).
    pub async fn search(
        &self,
        query: &str,
        limit: u32,
    ) -> Result<ClawHubSearchResponse, SkillError> {
        let url = format!(
            "{}/search?q={}&limit={}",
            self.base_url,
            percent_encode(query),
            limit.min(50)
        );

        let resp = self
            .http
            .get(&url)
            .header("User-Agent", "LibreFang/0.1")
            .header("Accept", "application/json")
            .send()
            .await
            .map_err(|e| SkillError::Network(format!("Skillhub search request failed: {e}")))?;

        if !resp.status().is_success() {
            return Err(SkillError::Network(format!(
                "Skillhub search returned {}",
                resp.status()
            )));
        }

        let body = resp.bytes().await.map_err(|e| {
            SkillError::Network(format!("Failed to read Skillhub search response: {e}"))
        })?;

        parse_skillhub_search_body(&body, &url)
    }

    /// Get detailed info about a specific skill.
    pub async fn get_skill(&self, slug: &str) -> Result<ClawHubSkillDetail, SkillError> {
        self.inner.get_skill(slug).await
    }

    /// Install a skill from Skillhub.
    ///
    /// Downloads the skill zip directly from Tencent COS (the static index
    /// provides slug + version, and the zip lives at a predictable COS path).
    /// After extraction, delegates to ClawHub's install_from_bytes for security
    /// scanning and manifest generation, then patches source provenance.
    pub async fn install(
        &self,
        slug: &str,
        target_dir: &Path,
    ) -> Result<ClawHubInstallResult, SkillError> {
        // Step 1: Look up the version from the static index
        let index_resp = self
            .http
            .get(&self.index_url)
            .header("User-Agent", "LibreFang/0.1")
            .header("Accept", "application/json")
            .send()
            .await
            .map_err(|e| SkillError::Network(format!("Skillhub index fetch failed: {e}")))?;
        if !index_resp.status().is_success() {
            return Err(SkillError::Network(format!(
                "Skillhub index returned {}",
                index_resp.status()
            )));
        }
        let index_body = index_resp
            .bytes()
            .await
            .map_err(|e| SkillError::Network(format!("Failed to read Skillhub index: {e}")))?;
        let index: SkillhubIndexResponse = parse_skillhub_index_body(&index_body, &self.index_url)?;

        let entry = index
            .skills
            .iter()
            .find(|s| s.slug == slug)
            .ok_or_else(|| {
                SkillError::NotFound(format!("Skill '{slug}' not found in Skillhub index"))
            })?;
        let version = &entry.version;

        // Step 2: Download zip from COS
        let cos_url = format!("{}/skills/{slug}/{version}.zip", self.cos_base);
        info!(slug, version = %version, "Downloading skill from Skillhub COS");

        let dl_resp = self
            .http
            .get(&cos_url)
            .header("User-Agent", "LibreFang/0.1")
            .send()
            .await
            .map_err(|e| SkillError::Network(format!("Skillhub COS download failed: {e}")))?;
        if !dl_resp.status().is_success() {
            return Err(SkillError::Network(format!(
                "Skillhub COS download returned {}",
                dl_resp.status()
            )));
        }
        let bytes = dl_resp
            .bytes()
            .await
            .map_err(|e| SkillError::Network(format!("Failed to read download body: {e}")))?;

        // Step 3: Delegate to ClawHub client for extraction + security scan
        let result = self
            .inner
            .install_from_bytes(slug, target_dir, &bytes)
            .await?;

        // Step 4: Patch source provenance to Skillhub.
        //
        // #3675 — propagate every step's error (read / parse / serialize /
        // write) instead of swallowing with `if let Ok`.  A failure here means
        // the manifest's `source` field is wrong, which silently breaks later
        // upgrade and sync logic that switches behavior on `SkillSource`.  On
        // failure we tear down the freshly-extracted skill directory so the
        // installer doesn't leave a manifest with the wrong provenance behind.
        let skill_dir = target_dir.join(slug);
        let patch_slug = slug.to_string();
        let patch_version = result.version.clone();
        tokio::task::spawn_blocking(move || {
            patch_or_cleanup_skillhub_install(skill_dir, patch_slug, patch_version)
        })
        .await
        .map_err(|e| {
            SkillError::InvalidManifest(format!("Skillhub provenance patch task failed: {e}"))
        })??;

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
            .get(&self.index_url)
            .header("User-Agent", "LibreFang/0.1")
            .header("Accept", "application/json")
            .send()
            .await
            .map_err(|e| SkillError::Network(format!("Skillhub index fetch failed: {e}")))?;

        if !resp.status().is_success() {
            return Err(SkillError::Network(format!(
                "Skillhub index returned {}",
                resp.status()
            )));
        }

        let body = resp
            .bytes()
            .await
            .map_err(|e| SkillError::Network(format!("Failed to read Skillhub index: {e}")))?;
        let mut data: SkillhubIndexResponse = parse_skillhub_index_body(&body, &self.index_url)?;

        // Client-side sort
        match sort {
            "downloads" => data.skills.sort_by_key(|b| std::cmp::Reverse(b.downloads)),
            "stars" => data.skills.sort_by_key(|b| std::cmp::Reverse(b.stars)),
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

/// Parse a Skillhub `/search` response body, degrading gracefully when the
/// upstream sends back markup instead of JSON.
///
/// See #7387: `skillhub.tencent.com` now answers every API path with a
/// `200 OK` full of its SPA shell instead of JSON. Handing that straight to
/// `serde_json` produces a cryptic "expected value at line 1 column 1"
/// failure with no indication of the real cause, so this checks for markup
/// first and returns a clear, actionable [`SkillError::MarketplaceUnavailable`].
///
/// Split out from [`SkillhubClient::search`] so the parsing logic — including
/// the SkillHub-native vs. ClawHub-compatible format fallback — is testable
/// without a live network call.
///
/// The markup gate is spelled out here rather than delegated to
/// [`crate::parse_marketplace_json`] because neither of the two parses below is
/// the single deserialization that helper performs; the message is the same one
/// it emits, so an operator sees one offline state whichever hub or endpoint
/// went dark.
fn parse_skillhub_search_body(body: &[u8], url: &str) -> Result<ClawHubSearchResponse, SkillError> {
    // Markup first: both parses below fail on an HTML body, and the second one's
    // `serde_json` complaint is exactly the cryptic "expected value at line 1 column 1"
    // this check exists to replace (#7387).
    if crate::looks_like_markup(body) {
        return Err(SkillError::MarketplaceUnavailable(format!(
            "Skillhub search at {url} answered with a webpage instead of JSON — the marketplace is unreachable or has moved. Searching, browsing and installing from it are unavailable until it returns; skills already installed locally are unaffected."
        )));
    }

    // Try SkillHub-native format first (snake_case, `skills` or `results` key).
    // We parse this first because ClawHubSearchResponse with serde(default)
    // would accept any JSON as empty results, masking the real data.
    if let Ok(skillhub_resp) = serde_json::from_slice::<SkillhubSearchResponse>(body) {
        if !skillhub_resp.results.is_empty() {
            return Ok(ClawHubSearchResponse {
                results: skillhub_resp
                    .results
                    .into_iter()
                    .map(|e| ClawHubSearchEntry {
                        score: e.score,
                        slug: e.slug,
                        display_name: e.name,
                        summary: e.description,
                        version: if e.version.is_empty() {
                            None
                        } else {
                            Some(e.version)
                        },
                        updated_at: e.updated_at,
                    })
                    .collect(),
            });
        }
    }

    // Fall back to ClawHub-compatible format (camelCase, `results` key).
    serde_json::from_slice::<ClawHubSearchResponse>(body)
        .map_err(|e| SkillError::Network(format!("Failed to parse Skillhub search response: {e}")))
}

/// Parse the static Skillhub COS index body, degrading gracefully when the
/// bucket answers with markup instead of JSON (same #7387 failure mode as
/// [`parse_skillhub_search_body`], applied to `browse` and `install`'s
/// version lookup).
fn parse_skillhub_index_body(
    body: &[u8],
    index_url: &str,
) -> Result<SkillhubIndexResponse, SkillError> {
    crate::parse_marketplace_json("Skillhub index", index_url, body)
}

/// URL query parameter encoding (`application/x-www-form-urlencoded`).
/// Unreserved characters pass through unchanged, space becomes `+`,
/// everything else is `%XX` encoded.
fn percent_encode(s: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut out = String::with_capacity(s.len() * 3);
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            b' ' => out.push('+'),
            _ => {
                out.push('%');
                out.push(HEX[(b >> 4) as usize] as char);
                out.push(HEX[(b & 0xf) as usize] as char);
            }
        }
    }
    out
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

    #[test]
    fn test_skillhub_search_response_results_key() {
        // SkillHub-native format using `results` key (same as alias)
        let json = r#"{
            "results": [
                {
                    "slug": "rust-helper",
                    "name": "Rust Helper",
                    "description": "Helps with Rust",
                    "version": "1.2.0",
                    "score": 0.95,
                    "updated_at": 1700000000
                }
            ]
        }"#;
        let resp: SkillhubSearchResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.results.len(), 1);
        assert_eq!(resp.results[0].slug, "rust-helper");
        assert_eq!(resp.results[0].name, "Rust Helper");
        assert!((resp.results[0].score - 0.95).abs() < f64::EPSILON);
    }

    #[test]
    fn test_skillhub_search_response_skills_key() {
        // SkillHub-native format using `skills` key (alias)
        let json = r#"{
            "skills": [
                {
                    "slug": "python-expert",
                    "name": "Python Expert",
                    "description": "Expert Python assistance",
                    "version": "2.0.0",
                    "score": 0.88,
                    "updated_at": 0
                }
            ]
        }"#;
        let resp: SkillhubSearchResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.results.len(), 1);
        assert_eq!(resp.results[0].slug, "python-expert");
        assert_eq!(resp.results[0].version, "2.0.0");
    }

    #[test]
    fn test_skillhub_search_entry_minimal() {
        // Only slug is required; all other fields have defaults
        let json = r#"{"slug": "minimal"}"#;
        let entry: SkillhubSearchEntry = serde_json::from_str(json).unwrap();
        assert_eq!(entry.slug, "minimal");
        assert_eq!(entry.name, "");
        assert_eq!(entry.score, 0.0);
        assert_eq!(entry.updated_at, 0);
    }

    #[test]
    fn test_percent_encode() {
        assert_eq!(percent_encode("hello world"), "hello+world");
        assert_eq!(percent_encode("rust"), "rust");
        assert_eq!(percent_encode("a&b=c"), "a%26b%3Dc");
        assert_eq!(
            percent_encode("hello-world_2.0~test"),
            "hello-world_2.0~test"
        );
    }

    #[test]
    fn provenance_patch_is_atomic_and_durable() {
        let dir = tempfile::tempdir().unwrap();
        let skill_dir = dir.path().join("example");
        std::fs::create_dir(&skill_dir).unwrap();
        let manifest_path = skill_dir.join("skill.toml");
        std::fs::write(
            &manifest_path,
            "[skill]\nname = \"example\"\nversion = \"1.0.0\"\ndescription = \"test\"\n",
        )
        .unwrap();

        patch_or_cleanup_skillhub_install(
            skill_dir.clone(),
            "example-skill".to_string(),
            "2.0.0".to_string(),
        )
        .unwrap();

        let manifest: crate::SkillManifest =
            toml::from_str(&std::fs::read_to_string(&manifest_path).unwrap()).unwrap();
        assert!(matches!(
            manifest.source,
            Some(crate::SkillSource::Skillhub { slug, version })
                if slug == "example-skill" && version == "2.0.0"
        ));
        assert_eq!(std::fs::read_dir(&skill_dir).unwrap().count(), 1);
    }

    #[test]
    fn malformed_manifest_removes_half_installed_skill() {
        let dir = tempfile::tempdir().unwrap();
        let skill_dir = dir.path().join("broken");
        std::fs::create_dir(&skill_dir).unwrap();
        std::fs::write(skill_dir.join("skill.toml"), "not = [valid").unwrap();

        let result = patch_or_cleanup_skillhub_install(
            skill_dir.clone(),
            "broken".to_string(),
            "1.0.0".to_string(),
        );

        assert!(matches!(result, Err(SkillError::InvalidManifest(_))));
        assert!(!skill_dir.exists());
    }

    // -- #7387: Skillhub API is dead — graceful-degradation regression tests ---

    #[test]
    fn search_body_html_response_degrades_to_marketplace_unavailable() {
        let html = b"<!DOCTYPE html><html><head><title>SkillHub</title></head></html>";
        let result = parse_skillhub_search_body(html, DEFAULT_SKILLHUB_URL);
        match result {
            Err(SkillError::MarketplaceUnavailable(msg)) => {
                assert!(msg.contains(DEFAULT_SKILLHUB_URL), "{msg}");
                assert!(msg.contains("webpage instead of JSON"), "{msg}");
            }
            other => panic!("expected MarketplaceUnavailable, got {other:?}"),
        }
    }

    #[test]
    fn search_body_valid_json_still_parses() {
        let json = br#"{"results": [{"slug": "rust-helper", "name": "Rust Helper"}]}"#;
        let resp = parse_skillhub_search_body(json, DEFAULT_SKILLHUB_URL).unwrap();
        assert_eq!(resp.results.len(), 1);
        assert_eq!(resp.results[0].slug, "rust-helper");
    }

    #[test]
    fn search_body_genuinely_malformed_json_is_a_network_error_not_marketplace_unavailable() {
        let garbage = b"{not valid json";
        let result = parse_skillhub_search_body(garbage, DEFAULT_SKILLHUB_URL);
        assert!(matches!(result, Err(SkillError::Network(_))));
    }

    #[test]
    fn index_body_html_response_degrades_to_marketplace_unavailable() {
        let html = b"<html><body>not json</body></html>";
        let result = parse_skillhub_index_body(html, DEFAULT_SKILLHUB_INDEX_URL);
        assert!(matches!(result, Err(SkillError::MarketplaceUnavailable(_))));
    }

    #[test]
    fn index_body_valid_json_still_parses() {
        let json = br#"{"total": 1, "skills": [{"slug": "rust"}]}"#;
        let resp = parse_skillhub_index_body(json, DEFAULT_SKILLHUB_INDEX_URL).unwrap();
        assert_eq!(resp.total, 1);
        assert_eq!(resp.skills.len(), 1);
    }

    #[test]
    #[serial_test::serial(skillhub_env_url)]
    fn with_defaults_honors_env_var_override() {
        std::env::set_var(
            "LIBREFANG_SKILLHUB_URL",
            "https://mirror.example.test/api/v1",
        );
        let client = SkillhubClient::with_defaults(PathBuf::from("/tmp/cache"));
        assert_eq!(client.base_url, "https://mirror.example.test/api/v1");
        std::env::remove_var("LIBREFANG_SKILLHUB_URL");
    }

    #[test]
    #[serial_test::serial(skillhub_env_url)]
    fn with_defaults_falls_back_when_env_var_unset() {
        std::env::remove_var("LIBREFANG_SKILLHUB_URL");
        let client = SkillhubClient::with_defaults(PathBuf::from("/tmp/cache"));
        assert_eq!(client.base_url, DEFAULT_SKILLHUB_URL);
    }
}
