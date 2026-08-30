//! ClawHub marketplace client — search and install skills from clawhub.ai.
//!
//! ClawHub hosts 3,000+ community skills in both SKILL.md (prompt-only)
//! and package.json (Node.js) formats. This client downloads, converts,
//! and security-scans skills before installation.
//!
//! API reference: <https://clawhub.ai/api/v1/>
//! - Search: `GET /api/v1/search?q=...&limit=20`
//! - Browse: `GET /api/v1/skills?limit=20&sort=trending`
//! - Detail: `GET /api/v1/skills/{slug}`
//! - Download: `GET /api/v1/download?slug=...`
//! - File: `GET /api/v1/skills/{slug}/file?path=SKILL.md`

use crate::openclaw_compat;
use crate::verify::{SkillVerifier, SkillWarning, WarningSeverity};
use crate::SkillError;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Component, Path, PathBuf};
use tracing::{debug, info, warn};

/// Official ClawHub API base URL.
pub const DEFAULT_CLAWHUB_URL: &str = "https://clawhub.ai/api/v1";

/// Environment variable that repoints [`ClawHubClient::new`] at a ClawHub mirror.
pub const ENV_CLAWHUB_URL: &str = "LIBREFANG_CLAWHUB_URL";

/// Read a marketplace URL override from the environment, falling back to `default`.
///
/// An unset variable and one set to whitespace are treated alike: a blank override in a shell profile or compose file is an operator mistake, not a request to fetch from the empty string.
pub fn env_url_or(var: &str, default: &str) -> String {
    std::env::var(var)
        .ok()
        .map(|value| value.trim().trim_end_matches('/').to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| default.to_string())
}

// ---------------------------------------------------------------------------
// Retry constants for ClawHub API rate-limit handling
// ---------------------------------------------------------------------------

/// Maximum number of retry attempts for ClawHub API calls (including the first try).
const MAX_RETRIES: u32 = 5;

/// Base delay in milliseconds for exponential backoff (doubles each attempt).
const BASE_DELAY_MS: u64 = 1_500;

/// Maximum delay cap in milliseconds.
const MAX_DELAY_MS: u64 = 30_000;

fn exponential_backoff_with_jitter(attempt: u32) -> u64 {
    let base = BASE_DELAY_MS.saturating_mul(1u64 << attempt.min(5));
    let delay_ms = base.min(MAX_DELAY_MS);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos();
    // Wrapping multiplication intentionally mixes the low-resolution clock bits into a cheap process-local jitter value.
    let mixed = nanos.wrapping_mul(2_654_435_761);
    let fraction = f64::from(mixed) / f64::from(u32::MAX);
    delay_ms + (delay_ms as f64 * fraction * 0.25) as u64
}

// ---------------------------------------------------------------------------
// API response types (matching actual ClawHub v1 API — verified Feb 2026)
// ---------------------------------------------------------------------------

// -- Shared nested types ---------------------------------------------------

/// Stats nested inside browse entries and skill detail.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClawHubStats {
    #[serde(default)]
    pub comments: u64,
    #[serde(default)]
    pub downloads: u64,
    #[serde(default)]
    pub installs_all_time: u64,
    #[serde(default)]
    pub installs_current: u64,
    #[serde(default)]
    pub installs: u64,
    #[serde(default)]
    pub stars: u64,
    #[serde(default)]
    pub versions: u64,
}

/// Version info nested inside browse entries and skill detail.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClawHubVersionInfo {
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub created_at: i64,
    #[serde(default)]
    pub changelog: String,
}

/// Owner info from the skill detail endpoint.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClawHubOwner {
    #[serde(default)]
    pub handle: String,
    #[serde(default)]
    pub user_id: String,
    #[serde(default)]
    pub display_name: String,
    #[serde(default)]
    pub image: Option<String>,
}

// -- Browse: GET /api/v1/skills?limit=N&sort=trending ----------------------

/// A skill entry from the browse endpoint (`GET /api/v1/skills`).
///
/// Tags is a string→string map (e.g. `{"latest": "1.0.0"}`), not a list.
/// Timestamps are Unix milliseconds.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClawHubBrowseEntry {
    pub slug: String,
    #[serde(default)]
    pub display_name: String,
    #[serde(default)]
    pub summary: String,
    /// Version tags (e.g. `{"latest": "1.0.0"}`).
    #[serde(default)]
    pub tags: std::collections::HashMap<String, String>,
    #[serde(default)]
    pub stats: ClawHubStats,
    /// Unix ms timestamp.
    #[serde(default)]
    pub created_at: i64,
    /// Unix ms timestamp.
    #[serde(default)]
    pub updated_at: i64,
    #[serde(default)]
    pub latest_version: Option<ClawHubVersionInfo>,
}

/// Paginated response from the browse endpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClawHubBrowseResponse {
    pub items: Vec<ClawHubBrowseEntry>,
    #[serde(default)]
    pub next_cursor: Option<String>,
}

// -- Search: GET /api/v1/search?q=...&limit=N ------------------------------

/// A skill entry from the search endpoint (`GET /api/v1/search`).
///
/// Search results are much flatter than browse results.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClawHubSearchEntry {
    #[serde(default)]
    pub score: f64,
    pub slug: String,
    #[serde(default)]
    pub display_name: String,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub version: Option<String>,
    /// Unix ms timestamp.
    #[serde(default)]
    pub updated_at: i64,
}

/// Response from the search endpoint. Uses `results`, **not** `items`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClawHubSearchResponse {
    pub results: Vec<ClawHubSearchEntry>,
}

// -- Detail: GET /api/v1/skills/{slug} -------------------------------------

/// The `skill` object nested inside the detail response.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClawHubSkillInfo {
    pub slug: String,
    #[serde(default)]
    pub display_name: String,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub tags: std::collections::HashMap<String, String>,
    #[serde(default)]
    pub stats: ClawHubStats,
    #[serde(default)]
    pub created_at: i64,
    #[serde(default)]
    pub updated_at: i64,
}

/// Full detail response from `GET /api/v1/skills/{slug}`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClawHubSkillDetail {
    pub skill: ClawHubSkillInfo,
    #[serde(default)]
    pub latest_version: Option<ClawHubVersionInfo>,
    #[serde(default)]
    pub owner: Option<ClawHubOwner>,
    /// Moderation status (null when clean).
    #[serde(default)]
    pub moderation: Option<serde_json::Value>,
    /// Expected SHA256 hex digest of the skill archive, provided by the registry.
    /// When present the installer validates the download before extraction.
    #[serde(default)]
    pub expected_sha256: Option<String>,
}

// -- Sort enum -------------------------------------------------------------

/// Sort order for browsing skills.
#[derive(Debug, Clone, Copy)]
pub enum ClawHubSort {
    Trending,
    Updated,
    Downloads,
    Stars,
    Rating,
}

impl ClawHubSort {
    fn as_str(self) -> &'static str {
        match self {
            Self::Trending => "trending",
            Self::Updated => "updated",
            Self::Downloads => "downloads",
            Self::Stars => "stars",
            Self::Rating => "rating",
        }
    }
}

// -- Backward compat aliases -----------------------------------------------

/// Alias kept for code that still references the old name.
pub type ClawHubListResponse = ClawHubBrowseResponse;
/// Alias kept for code that still references the old name.
pub type ClawHubSearchResults = ClawHubSearchResponse;
/// Alias kept for code that still references the old name.
pub type ClawHubEntry = ClawHubBrowseEntry;

/// Result of installing a skill from ClawHub.
#[derive(Debug, Clone)]
pub struct ClawHubInstallResult {
    /// Installed skill name.
    pub skill_name: String,
    /// Installed version.
    pub version: String,
    /// The skill slug on ClawHub.
    pub slug: String,
    /// Security warnings from the scan pipeline.
    pub warnings: Vec<SkillWarning>,
    /// Tool name translations applied (OpenClaw → LibreFang).
    pub tool_translations: Vec<(String, String)>,
    /// Whether this is a prompt-only skill.
    pub is_prompt_only: bool,
}

/// Client for the ClawHub marketplace (clawhub.ai).
pub struct ClawHubClient {
    /// Base URL for the ClawHub API.
    base_url: String,
    /// HTTP client.
    client: reqwest::Client,
    /// Local cache directory for downloaded skills.
    _cache_dir: PathBuf,
}

impl ClawHubClient {
    /// Create a new ClawHub client pointed at the configured ClawHub API.
    ///
    /// Defaults to the official API at [`DEFAULT_CLAWHUB_URL`]; `LIBREFANG_CLAWHUB_URL` overrides it so an operator can move to a mirror without recompiling when the official host stops answering with JSON.
    pub fn new(cache_dir: PathBuf) -> Self {
        Self::with_url(
            &crate::clawhub::env_url_or(ENV_CLAWHUB_URL, DEFAULT_CLAWHUB_URL),
            cache_dir,
        )
    }

    /// Create a ClawHub client with a custom API URL.
    pub fn with_url(base_url: &str, cache_dir: PathBuf) -> Self {
        // Check if we should skip TLS verification (for servers with expired certs)
        let use_dangerous = std::env::var("LIBREFANG_DANGEROUSLY_SKIP_TLS_VERIFICATION")
            .map(|v| v.eq_ignore_ascii_case("true") || v == "1")
            .unwrap_or(false);

        let builder = if use_dangerous {
            tracing::warn!("TLS verification disabled - use only for testing!");
            crate::http_client::dangerous_client_builder()
        } else {
            crate::http_client::client_builder()
        };

        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            client: builder
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .expect("HTTP client build"),
            _cache_dir: cache_dir,
        }
    }

    // -----------------------------------------------------------------------
    // Private: HTTP GET with retry on 429 / 5xx
    // -----------------------------------------------------------------------

    /// Issue a GET request with automatic retry on rate-limit (429) and
    /// server-error (5xx) responses. Respects the `Retry-After` header
    /// when present, otherwise uses exponential backoff with jitter.
    ///
    /// Returns the successful `reqwest::Response` or a `SkillError`.
    async fn get_with_retry(
        &self,
        url: &str,
        context: &str,
    ) -> Result<reqwest::Response, SkillError> {
        let mut last_status: Option<u16> = None;
        let mut next_delay_ms: Option<u64> = None;

        for attempt in 0..MAX_RETRIES {
            if let Some(delay_ms) = next_delay_ms.take() {
                debug!(
                    attempt,
                    delay_ms, context, "retrying ClawHub request after rate limit / server error"
                );
                tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
            }

            let result = self
                .client
                .get(url)
                .header("User-Agent", "LibreFang/0.1")
                .send()
                .await;

            match result {
                Ok(resp) => {
                    let status = resp.status();

                    if status.is_success() {
                        return Ok(resp);
                    }

                    // Rate-limited or server error — retryable.
                    if status.as_u16() == 429 || status.is_server_error() {
                        last_status = Some(status.as_u16());

                        let retry_after_ms = resp
                            .headers()
                            .get("retry-after")
                            .and_then(|v| v.to_str().ok())
                            .and_then(|v| v.parse::<u64>().ok())
                            .map(|seconds| seconds.saturating_mul(1000).min(MAX_DELAY_MS));

                        let is_last = attempt + 1 >= MAX_RETRIES;
                        if is_last {
                            if status.as_u16() == 429 {
                                return Err(SkillError::RateLimited(format!(
                                    "{context} returned 429 Too Many Requests after {MAX_RETRIES} attempts \
                                     — the ClawHub API rate limit has been exceeded, \
                                     please wait a few seconds and try again"
                                )));
                            }
                            return Err(SkillError::Network(format!(
                                "{context} returned {status} after {MAX_RETRIES} attempts"
                            )));
                        }
                        next_delay_ms = Some(
                            retry_after_ms
                                .unwrap_or_else(|| exponential_backoff_with_jitter(attempt + 1)),
                        );
                        continue;
                    }

                    // Non-retryable HTTP error (4xx other than 429).
                    return Err(SkillError::Network(format!("{context} returned {status}")));
                }
                Err(e) => {
                    // Network / timeout error — retryable.
                    last_status = None;
                    let is_last = attempt + 1 >= MAX_RETRIES;
                    if is_last {
                        return Err(SkillError::Network(format!(
                            "{context} failed after {MAX_RETRIES} attempts: {e}"
                        )));
                    }
                    next_delay_ms = Some(exponential_backoff_with_jitter(attempt + 1));
                    warn!(attempt, context, error = %e, "ClawHub request failed, will retry");
                }
            }
        }

        // Should be unreachable, but handle gracefully.
        Err(SkillError::Network(format!(
            "{context} failed (status: {last_status:?}) after {MAX_RETRIES} attempts"
        )))
    }

    // -----------------------------------------------------------------------
    // Public API methods — all use get_with_retry
    // -----------------------------------------------------------------------

    /// Search for skills on ClawHub using vector/semantic search.
    ///
    /// Uses `GET /api/v1/search?q=...&limit=...`.
    /// Returns `ClawHubSearchResponse` whose root key is `results` (not `items`).
    pub async fn search(
        &self,
        query: &str,
        limit: u32,
    ) -> Result<ClawHubSearchResponse, SkillError> {
        let url = format!(
            "{}/search?q={}&limit={}",
            self.base_url,
            urlencoded(query),
            limit.min(50)
        );

        let response = self.get_with_retry(&url, "ClawHub search").await?;
        let body = response.bytes().await.map_err(|e| {
            SkillError::Network(format!("Failed to read ClawHub search response: {e}"))
        })?;

        crate::parse_marketplace_json::<ClawHubSearchResponse>("ClawHub search", &url, &body)
    }

    /// Browse skills by sort order (trending, downloads, stars, etc.).
    ///
    /// Uses `GET /api/v1/skills?limit=...&sort=...`.
    pub async fn browse(
        &self,
        sort: ClawHubSort,
        limit: u32,
        cursor: Option<&str>,
    ) -> Result<ClawHubBrowseResponse, SkillError> {
        let mut url = format!(
            "{}/skills?limit={}&sort={}",
            self.base_url,
            limit.min(50),
            sort.as_str()
        );

        if let Some(c) = cursor {
            url.push_str(&format!("&cursor={}", urlencoded(c)));
        }

        let response = self.get_with_retry(&url, "ClawHub browse").await?;
        let body = response.bytes().await.map_err(|e| {
            SkillError::Network(format!("Failed to read ClawHub browse response: {e}"))
        })?;

        crate::parse_marketplace_json::<ClawHubBrowseResponse>("ClawHub browse", &url, &body)
    }

    /// Get detailed info about a specific skill.
    ///
    /// Uses `GET /api/v1/skills/{slug}`.
    /// Response is `{ skill: {...}, latestVersion: {...}, owner: {...}, moderation: null }`.
    pub async fn get_skill(&self, slug: &str) -> Result<ClawHubSkillDetail, SkillError> {
        validate_slug(slug)?;
        let url = format!("{}/skills/{}", self.base_url, urlencoded(slug));

        let response = self.get_with_retry(&url, "ClawHub skill detail").await?;
        let body = response.bytes().await.map_err(|e| {
            SkillError::Network(format!("Failed to read ClawHub detail response: {e}"))
        })?;

        crate::parse_marketplace_json::<ClawHubSkillDetail>("ClawHub skill detail", &url, &body)
    }

    /// Helper: extract the version string from a browse entry.
    pub fn entry_version(entry: &ClawHubBrowseEntry) -> &str {
        entry
            .latest_version
            .as_ref()
            .map(|v| v.version.as_str())
            .or_else(|| entry.tags.get("latest").map(|s| s.as_str()))
            .unwrap_or("")
    }

    /// Fetch a specific file from a skill (e.g., SKILL.md, README).
    ///
    /// Uses `GET /api/v1/skills/{slug}/file?path=SKILL.md`.
    pub async fn get_file(&self, slug: &str, path: &str) -> Result<String, SkillError> {
        validate_slug(slug)?;
        let url = format!(
            "{}/skills/{}/file?path={}",
            self.base_url,
            urlencoded(slug),
            urlencoded(path)
        );

        let response = self.get_with_retry(&url, "ClawHub file fetch").await?;

        let text = response
            .text()
            .await
            .map_err(|e| SkillError::Network(format!("Failed to read ClawHub file: {e}")))?;

        // A hub serving its SPA shell answers `200` here too, and the caller would happily display that HTML as the skill's source.
        // None of the three files this endpoint is asked for — `SKILL.md`, `package.json`, `skill.toml` — begins with `<`, so the same marker that identifies a dead JSON endpoint identifies a dead file endpoint.
        if crate::looks_like_markup(text.as_bytes()) {
            return Err(SkillError::MarketplaceUnavailable(format!(
                "ClawHub file fetch at {url} answered with a webpage instead of {path} — the marketplace is unreachable or has moved. Skill source cannot be shown until it returns; skills already installed locally are unaffected."
            )));
        }

        Ok(text)
    }

    /// Install a skill from ClawHub into the target directory.
    ///
    /// Security pipeline:
    /// 0. Fetch skill detail (to obtain `expected_sha256` when the registry provides it)
    /// 1. Download skill zip and compute SHA256; validate against expected when present
    /// 2. Detect format (SKILL.md vs package.json)
    /// 3. Convert to LibreFang manifest
    /// 4. Run manifest security scan
    /// 5. If prompt-only: run prompt injection scan
    /// 6. Check binary dependencies
    /// 7. Write skill.toml with `verified: false`
    pub async fn install(
        &self,
        slug: &str,
        target_dir: &Path,
    ) -> Result<ClawHubInstallResult, SkillError> {
        validate_slug(slug)?;

        // Step 0: Fetch skill detail before downloading the archive.
        // A failed detail request must not silently downgrade a verifiable install into the unverified path.
        let expected_sha256 = self.get_skill(slug).await?.expected_sha256;

        // Use /api/v1/download?slug=... endpoint
        let url = format!("{}/download?slug={}", self.base_url, urlencoded(slug));

        info!(slug, "Downloading skill from ClawHub");

        // Use get_with_retry for the download — same 429/5xx handling as all
        // other endpoints, with 5 attempts and exponential backoff.
        let response = self.get_with_retry(&url, "ClawHub download").await?;
        let bytes = response
            .bytes()
            .await
            .map_err(|e| SkillError::Network(format!("Failed to read download body: {e}")))?;

        self.install_with_expected_sha256(slug, target_dir, &bytes, expected_sha256.as_deref())
            .await
    }

    /// Install a skill from raw bytes (zip or SKILL.md).
    ///
    /// Shared extraction + security scan logic used by both ClawHub download
    /// and Skillhub COS download paths.  No checksum validation is performed
    /// because the Skillhub COS path does not provide an expected hash; use
    /// `install_with_expected_sha256` when a hash is available.
    pub async fn install_from_bytes(
        &self,
        slug: &str,
        target_dir: &Path,
        bytes: &[u8],
    ) -> Result<ClawHubInstallResult, SkillError> {
        self.install_with_expected_sha256(slug, target_dir, bytes, None)
            .await
    }

    /// Install a skill from raw bytes with optional SHA256 checksum validation.
    ///
    /// When `expected_sha256` is `Some`, the computed digest of `bytes` is
    /// compared against it before extraction.  A mismatch deletes any partially
    /// written files and returns a `SkillError::SecurityBlocked` error.
    /// When `expected_sha256` is `None`, a `warn!` is emitted and installation
    /// continues (backward-compatible behaviour).
    async fn install_with_expected_sha256(
        &self,
        slug: &str,
        target_dir: &Path,
        bytes: &[u8],
        expected_sha256: Option<&str>,
    ) -> Result<ClawHubInstallResult, SkillError> {
        validate_slug(slug)?;

        // Step 1: SHA256 of downloaded content
        let sha256 = {
            let mut hasher = Sha256::new();
            hasher.update(bytes);
            hex::encode(hasher.finalize())
        };
        info!(slug, sha256 = %sha256, "Downloaded skill");

        // Step 1a: Validate checksum against registry-supplied expected hash
        // BEFORE creating any directories so we fail fast on supply-chain
        // tampering (issue #3827).
        match expected_sha256 {
            Some(expected) => {
                let expected_lower = expected.to_lowercase();
                if sha256 != expected_lower {
                    return Err(SkillError::SecurityBlocked(format!(
                        "Skill {slug} hash mismatch: expected {expected_lower}, got {sha256}"
                    )));
                }
                info!(slug, "SHA256 checksum verified OK");
            }
            None => {
                warn!(
                    slug,
                    "ClawHub did not provide expected_sha256 — install unverified"
                );
            }
        }

        // Install into a sibling staging directory first, then atomically
        // rename to the final skill directory.  This prevents partial installs
        // from being loaded on the next daemon start if extraction is
        // interrupted.  #3719 — process-local AtomicU64 counter guarantees
        // every install in this process gets a unique staging path even when
        // two threads race within the OS clock resolution window (which a
        // bare nanosecond timestamp can't promise).  The pid disambiguates
        // across processes; the counter disambiguates within one process.
        static STAGING_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let seq = STAGING_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let skill_dir = resolve_skill_dir(target_dir, slug)?;
        let tmp_dir = target_dir.join(format!(".staging-{}-{}-{}", slug, std::process::id(), seq));
        // Extraction — tmp-dir setup, zip decompression, and per-entry writes
        // — is blocking and, for a zip skill, unbounded in size. Offload it to
        // a blocking thread so it does not stall the tokio worker for the full
        // archive (refs blocking-fs-on-executor). Returns `is_skillmd`, which
        // the conversion step below needs.
        let is_skillmd = {
            let tmp_dir = tmp_dir.clone();
            let bytes = bytes.to_vec();
            let slug = slug.to_string();
            tokio::task::spawn_blocking(move || -> Result<bool, SkillError> {
                let mut cleanup = prepare_staging_dir(&tmp_dir)?;

                // Detect content type and extract accordingly
                let content_str = String::from_utf8_lossy(&bytes);
                let is_skillmd = content_str.trim_start().starts_with("---");

                if is_skillmd {
                    let skill_md_path = resolve_skill_child_path(&tmp_dir, Path::new("SKILL.md"))?;
                    std::fs::write(skill_md_path, &bytes)?;
                } else if bytes.starts_with(&[0x50, 0x4b, 0x03, 0x04]) {
                    // Zip archive — extract all files
                    let cursor = std::io::Cursor::new(&bytes);
                    match zip::ZipArchive::new(cursor) {
                        Ok(mut archive) => {
                            // Decompression-bomb / entry-count guards, shared with
                            // the marketplace bundle extractor (#6441 follow-up):
                            // previously this path streamed every entry with an
                            // unbounded `std::io::copy`, so a malicious skill zip
                            // could exhaust disk / inodes.
                            if archive.len() > crate::marketplace::MAX_ENTRIES {
                                return Err(SkillError::SecurityBlocked(format!(
                                    "skill archive contains {} entries, exceeding the {}-entry limit",
                                    archive.len(),
                                    crate::marketplace::MAX_ENTRIES
                                )));
                            }
                            let mut total_uncompressed: u64 = 0;
                            for i in 0..archive.len() {
                                let mut file = match archive.by_index(i) {
                                    Ok(f) => f,
                                    Err(e) => {
                                        warn!(index = i, error = %e, "Skipping zip entry");
                                        continue;
                                    }
                                };
                                let Some(enclosed_name) = file.enclosed_name() else {
                                    warn!("Skipping zip entry with unsafe path");
                                    continue;
                                };
                                let out_path =
                                    match resolve_skill_child_path(&tmp_dir, &enclosed_name) {
                                        Ok(path) => path,
                                        Err(e) => {
                                            warn!(index = i, error = %e, "Skipping zip entry with unsafe path");
                                            continue;
                                        }
                                    };
                                if file.is_dir() {
                                    std::fs::create_dir_all(&out_path)?;
                                } else {
                                    if let Some(parent) = out_path.parent() {
                                        std::fs::create_dir_all(parent)?;
                                    }
                                    let declared = file.size();
                                    let compressed = file.compressed_size();
                                    crate::marketplace::write_zip_entry_capped(
                                        &mut file,
                                        declared,
                                        compressed,
                                        &out_path,
                                        &enclosed_name.display().to_string(),
                                        &mut total_uncompressed,
                                    )?;
                                }
                            }
                            info!(slug, entries = archive.len(), "Extracted skill zip");
                        }
                        Err(e) => {
                            warn!(slug, error = %e, "Failed to read zip, saving raw");
                            let zip_path =
                                resolve_skill_child_path(&tmp_dir, Path::new("skill.zip"))?;
                            std::fs::write(zip_path, &bytes)?;
                        }
                    }
                } else {
                    let package_path =
                        resolve_skill_child_path(&tmp_dir, Path::new("package.json"))?;
                    std::fs::write(package_path, &bytes)?;
                }
                cleanup.disarm();
                Ok(is_skillmd)
            })
            .await
            .map_err(|e| {
                SkillError::Io(std::io::Error::other(format!("extract task join: {e}")))
            })??
        };

        // Conversion, security scans, manifest writes, failure cleanup, and
        // final promotion all walk or mutate the extracted directory. Keep
        // the complete post-extraction phase off Tokio worker threads too.
        let result = {
            let tmp_dir = tmp_dir.clone();
            let skill_dir = skill_dir.clone();
            let slug = slug.to_string();
            tokio::task::spawn_blocking(move || -> Result<ClawHubInstallResult, SkillError> {
                let mut cleanup = StagingCleanup::new(&tmp_dir);
                let mut all_warnings = Vec::new();
                let mut tool_translations = Vec::new();
                let mut is_prompt_only = false;

                let manifest = if is_skillmd || openclaw_compat::detect_skillmd(&tmp_dir) {
                    let converted = openclaw_compat::convert_skillmd(&tmp_dir)?;
                    tool_translations = converted.tool_translations;
                    is_prompt_only =
                        converted.manifest.runtime.runtime_type == crate::SkillRuntime::PromptOnly;

                    let prompt_warnings =
                        SkillVerifier::scan_prompt_content(&converted.prompt_context);
                    if prompt_warnings
                        .iter()
                        .any(|warning| warning.severity == WarningSeverity::Critical)
                    {
                        let critical_msgs = prompt_warnings
                            .iter()
                            .filter(|warning| warning.severity == WarningSeverity::Critical)
                            .map(|warning| warning.message.clone())
                            .collect::<Vec<_>>();
                        return Err(SkillError::SecurityBlocked(format!(
                            "Skill blocked due to prompt injection: {}",
                            critical_msgs.join("; ")
                        )));
                    }
                    all_warnings.extend(prompt_warnings);

                    openclaw_compat::write_prompt_context(&tmp_dir, &converted.prompt_context)?;
                    for bin in &converted.required_bins {
                        if which_check(bin).is_none() {
                            all_warnings.push(SkillWarning {
                                severity: WarningSeverity::Warning,
                                message: format!("Required binary not found: {bin}"),
                            });
                        }
                    }
                    converted.manifest
                } else if openclaw_compat::detect_openclaw_skill(&tmp_dir) {
                    openclaw_compat::convert_openclaw_skill(&tmp_dir)?
                } else {
                    return Err(SkillError::InvalidManifest(
                        "Downloaded content is not a recognized skill format".to_string(),
                    ));
                };

                all_warnings.extend(SkillVerifier::security_scan(&manifest));
                openclaw_compat::write_librefang_manifest(&tmp_dir, &manifest)?;

                if let Err(violations) = crate::supply_chain::scan(&tmp_dir) {
                    let summary = violations
                        .iter()
                        .map(|violation| violation.to_string())
                        .collect::<Vec<_>>()
                        .join("; ");
                    return Err(SkillError::SecurityBlocked(format!(
                        "supply-chain audit failed for '{slug}': {summary}"
                    )));
                }

                promote_staged_skill(&tmp_dir, &skill_dir)?;
                cleanup.disarm();

                Ok(ClawHubInstallResult {
                    skill_name: manifest.skill.name.clone(),
                    version: manifest.skill.version.clone(),
                    slug,
                    warnings: all_warnings,
                    tool_translations,
                    is_prompt_only,
                })
            })
            .await
            .map_err(|e| {
                SkillError::Io(std::io::Error::other(format!(
                    "post-extraction task join: {e}"
                )))
            })??
        };

        info!(
            slug,
            skill_name = %result.skill_name,
            warnings = result.warnings.len(),
            "Installed skill from ClawHub"
        );

        Ok(result)
    }

    /// Check if a ClawHub skill is already installed locally.
    pub fn is_installed(&self, slug: &str, skills_dir: &Path) -> bool {
        if validate_slug(slug).is_err() {
            return false;
        }
        let skill_dir = match resolve_skill_dir(skills_dir, slug) {
            Ok(path) => path,
            Err(_) => return false,
        };
        let manifest_path = match resolve_skill_child_path(&skill_dir, Path::new("skill.toml")) {
            Ok(path) => path,
            Err(_) => return false,
        };
        manifest_path.exists()
    }
}

fn validate_slug(slug: &str) -> Result<(), SkillError> {
    if slug.is_empty()
        || !slug
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return Err(SkillError::InvalidManifest(format!(
            "Invalid skill slug '{slug}'"
        )));
    }
    Ok(())
}

fn resolve_skill_dir(target_dir: &Path, slug: &str) -> Result<PathBuf, SkillError> {
    validate_slug(slug)?;
    Ok(target_dir.join(slug))
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
            "Unsafe path component '{}'",
            relative.display()
        )));
    }
    Ok(skill_dir.join(relative))
}

struct StagingCleanup {
    path: PathBuf,
    armed: bool,
}

impl StagingCleanup {
    fn new(path: &Path) -> Self {
        Self {
            path: path.to_path_buf(),
            armed: true,
        }
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for StagingCleanup {
    fn drop(&mut self) {
        if self.armed {
            if let Err(error) = std::fs::remove_dir_all(&self.path) {
                if error.kind() != std::io::ErrorKind::NotFound {
                    warn!(path = %self.path.display(), %error, "Could not remove failed skill staging directory");
                }
            }
        }
    }
}

fn prepare_staging_dir(path: &Path) -> std::io::Result<StagingCleanup> {
    match std::fs::remove_dir_all(path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }

    let cleanup = StagingCleanup::new(path);
    std::fs::create_dir_all(path)?;
    Ok(cleanup)
}

fn promote_staged_skill(staged: &Path, target: &Path) -> std::io::Result<()> {
    static PROMOTION_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _guard = PROMOTION_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let target_exists = match std::fs::symlink_metadata(target) {
        Ok(_) => true,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
        Err(error) => return Err(error),
    };
    if !target_exists {
        return std::fs::rename(staged, target);
    }

    static BACKUP_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let sequence = BACKUP_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let target_name = target
        .file_name()
        .ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::InvalidInput, "missing target name")
        })?
        .to_string_lossy();
    let backup = target.with_file_name(format!(
        ".backup-{target_name}-{}-{sequence}",
        std::process::id()
    ));

    std::fs::rename(target, &backup)?;
    match std::fs::rename(staged, target) {
        Ok(()) => {
            if let Err(error) = std::fs::remove_dir_all(&backup) {
                warn!(path = %backup.display(), %error, "Installed skill but could not remove backup directory");
            }
            Ok(())
        }
        Err(promotion_error) => match std::fs::rename(&backup, target) {
            Ok(()) => Err(promotion_error),
            Err(restore_error) => Err(std::io::Error::other(format!(
                "skill promotion failed: {promotion_error}; restoring prior install from {} failed: {restore_error}",
                backup.display()
            ))),
        },
    }
}

/// RFC 3986 percent-encoding for query parameters.
/// Unreserved characters pass through and every other byte becomes `%XX`.
fn urlencoded(s: &str) -> String {
    const HEX_UPPER: &[u8; 16] = b"0123456789ABCDEF";
    let mut result = String::with_capacity(s.len() * 3);
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                result.push(b as char);
            }
            _ => {
                result.push('%');
                result.push(HEX_UPPER[(b >> 4) as usize] as char);
                result.push(HEX_UPPER[(b & 0xf) as usize] as char);
            }
        }
    }
    result
}

/// Check if a binary is available on PATH.
fn which_check(name: &str) -> Option<PathBuf> {
    let result = if cfg!(target_os = "windows") {
        std::process::Command::new("where").arg(name).output()
    } else {
        std::process::Command::new("which").arg(name).output()
    };

    match result {
        Ok(output) if output.status.success() => {
            let path_str = String::from_utf8_lossy(&output.stdout);
            let first_line = path_str.lines().next()?;
            Some(PathBuf::from(first_line.trim()))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    async fn scripted_http_server(responses: Vec<String>) -> (String, tokio::task::JoinHandle<()>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let task = tokio::spawn(async move {
            for response in responses {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut request = [0_u8; 4096];
                let _ = stream.read(&mut request).await.unwrap();
                stream.write_all(response.as_bytes()).await.unwrap();
                stream.shutdown().await.unwrap();
            }
        });
        (format!("http://{address}"), task)
    }

    #[test]
    fn test_browse_entry_serde_real_format() {
        // Matches actual ClawHub browse API response (verified Feb 2026)
        let json = r#"{
            "slug": "sonoscli",
            "displayName": "Sonoscli",
            "summary": "Control Sonos speakers.",
            "tags": {"latest": "1.0.0"},
            "stats": {
                "comments": 1,
                "downloads": 19736,
                "installsAllTime": 455,
                "installsCurrent": 437,
                "stars": 15,
                "versions": 1
            },
            "createdAt": 1767545381030,
            "updatedAt": 1771777535889,
            "latestVersion": {
                "version": "1.0.0",
                "createdAt": 1767545381030,
                "changelog": ""
            }
        }"#;

        let entry: ClawHubBrowseEntry = serde_json::from_str(json).unwrap();
        assert_eq!(entry.slug, "sonoscli");
        assert_eq!(entry.display_name, "Sonoscli");
        assert_eq!(entry.stats.downloads, 19736);
        assert_eq!(entry.stats.stars, 15);
        assert_eq!(entry.tags.get("latest").unwrap(), "1.0.0");
        assert_eq!(entry.latest_version.as_ref().unwrap().version, "1.0.0");
        assert_eq!(entry.updated_at, 1771777535889);
    }

    #[test]
    fn test_browse_response_serde() {
        let json = r#"{
            "items": [{
                "slug": "test",
                "displayName": "Test",
                "summary": "A test",
                "tags": {},
                "stats": {"downloads": 100, "stars": 5},
                "createdAt": 0,
                "updatedAt": 0
            }],
            "nextCursor": null
        }"#;

        let resp: ClawHubBrowseResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.items.len(), 1);
        assert_eq!(resp.items[0].slug, "test");
        assert_eq!(resp.items[0].stats.downloads, 100);
        assert!(resp.next_cursor.is_none());
    }

    #[test]
    fn test_search_entry_serde_real_format() {
        // Matches actual ClawHub search API response (verified Feb 2026)
        let json = r#"{
            "score": 3.7110556674218,
            "slug": "github",
            "displayName": "Github",
            "summary": "Interact with GitHub using the gh CLI.",
            "version": "1.0.0",
            "updatedAt": 1771777539580
        }"#;

        let entry: ClawHubSearchEntry = serde_json::from_str(json).unwrap();
        assert_eq!(entry.slug, "github");
        assert_eq!(entry.display_name, "Github");
        assert!(entry.score > 3.0);
        assert_eq!(entry.version.as_deref(), Some("1.0.0"));
        assert_eq!(entry.updated_at, 1771777539580);
    }

    #[test]
    fn test_search_response_serde() {
        // Search uses "results" not "items"
        let json = r#"{
            "results": [{
                "score": 3.5,
                "slug": "test",
                "displayName": "Test",
                "summary": "A test",
                "version": "1.0.0",
                "updatedAt": 0
            }]
        }"#;

        let resp: ClawHubSearchResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.results.len(), 1);
        assert_eq!(resp.results[0].slug, "test");
    }

    #[test]
    fn test_skill_detail_serde_real_format() {
        // Matches actual ClawHub detail API response (verified Feb 2026)
        let json = r##"{
            "skill": {
                "slug": "github",
                "displayName": "Github",
                "summary": "Interact with GitHub using the gh CLI.",
                "tags": {"latest": "1.0.0"},
                "stats": {
                    "comments": 3,
                    "downloads": 23790,
                    "installsAllTime": 428,
                    "installsCurrent": 417,
                    "stars": 67,
                    "versions": 1
                },
                "createdAt": 1767545344344,
                "updatedAt": 1771777539580
            },
            "latestVersion": {
                "version": "1.0.0",
                "createdAt": 1767545344344,
                "changelog": ""
            },
            "owner": {
                "handle": "steipete",
                "userId": "kn70pywhg0fyz996kpa8xj89s57yhv26",
                "displayName": "Peter Steinberger",
                "image": "https://avatars.githubusercontent.com/u/58493?v=4"
            },
            "moderation": null
        }"##;

        let detail: ClawHubSkillDetail = serde_json::from_str(json).unwrap();
        assert_eq!(detail.skill.slug, "github");
        assert_eq!(detail.skill.display_name, "Github");
        assert_eq!(detail.skill.stats.downloads, 23790);
        assert_eq!(detail.skill.stats.stars, 67);
        assert_eq!(detail.latest_version.as_ref().unwrap().version, "1.0.0");
        assert_eq!(detail.owner.as_ref().unwrap().handle, "steipete");
        assert!(detail.moderation.is_none());
    }

    #[test]
    fn test_clawhub_install_result() {
        let result = ClawHubInstallResult {
            skill_name: "test-skill".to_string(),
            version: "1.0.0".to_string(),
            slug: "test-skill".to_string(),
            warnings: vec![],
            tool_translations: vec![("Read".to_string(), "file_read".to_string())],
            is_prompt_only: true,
        };

        assert_eq!(result.skill_name, "test-skill");
        assert!(result.is_prompt_only);
        assert_eq!(result.tool_translations.len(), 1);
    }

    #[test]
    fn test_urlencoded() {
        assert_eq!(urlencoded("hello world"), "hello%20world");
        assert_eq!(urlencoded("a&b=c"), "a%26b%3Dc");
        assert_eq!(urlencoded("path/to#frag"), "path%2Fto%23frag");
        // Previously missed characters
        assert_eq!(urlencoded("100%"), "100%25");
        assert_eq!(urlencoded("a+b"), "a%2Bb");
        // Unreserved chars pass through
        assert_eq!(urlencoded("hello-world_2.0~test"), "hello-world_2.0~test");
    }

    #[test]
    fn test_clawhub_sort_str() {
        assert_eq!(ClawHubSort::Trending.as_str(), "trending");
        assert_eq!(ClawHubSort::Downloads.as_str(), "downloads");
        assert_eq!(ClawHubSort::Stars.as_str(), "stars");
    }

    #[test]
    fn test_clawhub_client_url() {
        let client = ClawHubClient::new(PathBuf::from("/tmp/cache"));
        assert_eq!(client.base_url, "https://clawhub.ai/api/v1");
    }

    #[test]
    fn test_entry_version_helper() {
        let entry = ClawHubBrowseEntry {
            slug: "test".to_string(),
            display_name: "Test".to_string(),
            summary: String::new(),
            tags: [("latest".to_string(), "2.0.0".to_string())]
                .into_iter()
                .collect(),
            stats: ClawHubStats::default(),
            created_at: 0,
            updated_at: 0,
            latest_version: Some(ClawHubVersionInfo {
                version: "2.0.0".to_string(),
                created_at: 0,
                changelog: String::new(),
            }),
        };
        assert_eq!(ClawHubClient::entry_version(&entry), "2.0.0");
    }

    #[tokio::test]
    async fn install_rejects_bundle_with_pth_via_supply_chain_audit() {
        use std::io::Write as _;
        use zip::write::SimpleFileOptions;

        // zip with a valid SKILL.md and a .pth entry — triggers the supply-chain audit.
        let mut buf = Vec::new();
        {
            let mut zip = zip::ZipWriter::new(std::io::Cursor::new(&mut buf));
            let opts = SimpleFileOptions::default();
            zip.start_file("SKILL.md", opts).unwrap();
            zip.write_all(b"---\nname: evil-skill\ndescription: test\n---\n# Evil\nBody\n")
                .unwrap();
            zip.start_file("evil.pth", opts).unwrap();
            zip.write_all(b"import os\n").unwrap();
            zip.finish().unwrap();
        }

        let dir = tempfile::tempdir().unwrap();
        let client = ClawHubClient::new(dir.path().join("cache"));
        let target = dir.path().join("skills");
        std::fs::create_dir_all(&target).unwrap();

        let err = client
            .install_with_expected_sha256("evil-skill", &target, &buf, None)
            .await
            .expect_err("a bundle containing a .pth must be refused by the supply-chain audit");
        assert!(
            matches!(err, SkillError::SecurityBlocked(ref m) if m.contains("supply-chain")),
            "expected a supply-chain SecurityBlocked error, got: {err:?}"
        );
        assert!(!target.join("evil-skill").exists());
    }

    #[tokio::test]
    async fn install_promotes_prepared_skill_and_removes_staging_directory() {
        let dir = tempfile::tempdir().unwrap();
        let client = ClawHubClient::new(dir.path().join("cache"));
        let target = dir.path().join("skills");
        std::fs::create_dir_all(&target).unwrap();
        let body = b"---\nname: safe-skill\ndescription: safe test skill\nversion: 1.2.3\n---\n# Safe Skill\nUse this skill safely.\n";

        let result = client
            .install_with_expected_sha256("safe-skill", &target, body, None)
            .await
            .unwrap();

        assert_eq!(result.skill_name, "safe-skill");
        assert_eq!(result.version, env!("CARGO_PKG_VERSION"));
        assert!(target.join("safe-skill").join("skill.toml").is_file());
        assert!(target.join("safe-skill").join("SKILL.md").is_file());
        let entries = std::fs::read_dir(&target)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect::<Vec<_>>();
        assert_eq!(entries, vec![std::ffi::OsString::from("safe-skill")]);
    }

    #[tokio::test]
    async fn retry_after_replaces_exponential_backoff() {
        let body = r#"{"results":[]}"#;
        let responses = vec![
            "HTTP/1.1 429 Too Many Requests\r\nRetry-After: 0\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_string(),
            format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            ),
        ];
        let (base_url, server) = scripted_http_server(responses).await;
        let client = ClawHubClient::with_url(&base_url, PathBuf::new());

        let result =
            tokio::time::timeout(std::time::Duration::from_secs(2), client.search("test", 1))
                .await
                .expect("Retry-After: 0 must not be followed by exponential backoff")
                .unwrap();

        assert!(result.results.is_empty());
        server.await.unwrap();
    }

    #[tokio::test]
    async fn install_fails_closed_when_skill_detail_is_unavailable() {
        let response = "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
        let (base_url, server) = scripted_http_server(vec![response.to_string()]).await;
        let client = ClawHubClient::with_url(&base_url, PathBuf::new());
        let target = tempfile::tempdir().unwrap();

        let error = client
            .install("missing-skill", target.path())
            .await
            .expect_err("detail failure must stop before an unverified download");

        assert!(matches!(error, SkillError::Network(_)));
        assert!(std::fs::read_dir(target.path()).unwrap().next().is_none());
        server.await.unwrap();
    }

    // -- #7387: marketplace-answers-with-HTML degrades gracefully -------------

    #[tokio::test]
    async fn search_degrades_gracefully_when_marketplace_returns_html() {
        let html_body = "<!DOCTYPE html><html><body>Not an API</body></html>";
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{html_body}",
            html_body.len()
        );
        let (base_url, server) = scripted_http_server(vec![response]).await;
        let client = ClawHubClient::with_url(&base_url, PathBuf::new());

        let error = client
            .search("test", 5)
            .await
            .expect_err("HTML body must not be silently accepted as JSON");

        match error {
            SkillError::MarketplaceUnavailable(msg) => {
                assert!(
                    msg.contains(&base_url),
                    "message should name the dead endpoint: {msg}"
                );
            }
            other => panic!("expected MarketplaceUnavailable, got {other:?}"),
        }
        server.await.unwrap();
    }

    #[tokio::test]
    async fn get_skill_degrades_gracefully_when_marketplace_returns_html() {
        let html_body = "<!DOCTYPE html><html><body>Not an API</body></html>";
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{html_body}",
            html_body.len()
        );
        let (base_url, server) = scripted_http_server(vec![response]).await;
        let client = ClawHubClient::with_url(&base_url, PathBuf::new());

        let error = client
            .get_skill("some-skill")
            .await
            .expect_err("HTML body must not be silently accepted as JSON");

        assert!(matches!(error, SkillError::MarketplaceUnavailable(_)));
        server.await.unwrap();
    }

    #[tokio::test]
    async fn install_fails_closed_when_marketplace_returns_html_instead_of_detail_json() {
        let html_body = "<!DOCTYPE html><html><body>Not an API</body></html>";
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{html_body}",
            html_body.len()
        );
        let (base_url, server) = scripted_http_server(vec![response]).await;
        let client = ClawHubClient::with_url(&base_url, PathBuf::new());
        let target = tempfile::tempdir().unwrap();

        let error = client
            .install("missing-skill", target.path())
            .await
            .expect_err("HTML detail response must stop before an unverified download");

        assert!(matches!(error, SkillError::MarketplaceUnavailable(_)));
        assert!(std::fs::read_dir(target.path()).unwrap().next().is_none());
        server.await.unwrap();
    }

    #[tokio::test]
    async fn late_conversion_failure_removes_staging_directory() {
        let target = tempfile::tempdir().unwrap();
        let client = ClawHubClient::new(PathBuf::new());
        let malformed = b"---\nname: [unterminated\n---\nbody\n";

        client
            .install_from_bytes("broken-skill", target.path(), malformed)
            .await
            .expect_err("malformed frontmatter must fail conversion");

        assert!(std::fs::read_dir(target.path()).unwrap().next().is_none());
    }

    #[test]
    fn failed_skill_promotion_restores_prior_install() {
        let dir = tempfile::tempdir().unwrap();
        let staged = dir.path().join("missing-staged");
        let target = dir.path().join("skill");
        std::fs::create_dir(&target).unwrap();
        std::fs::write(target.join("version"), "old").unwrap();

        promote_staged_skill(&staged, &target).expect_err("missing staging must fail promotion");

        assert_eq!(
            std::fs::read_to_string(target.join("version")).unwrap(),
            "old"
        );
        let entries = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect::<Vec<_>>();
        assert_eq!(entries, vec![std::ffi::OsString::from("skill")]);
    }

    #[test]
    fn successful_skill_promotion_removes_prior_backup() {
        let dir = tempfile::tempdir().unwrap();
        let staged = dir.path().join("staged");
        let target = dir.path().join("skill");
        std::fs::create_dir(&staged).unwrap();
        std::fs::write(staged.join("version"), "new").unwrap();
        std::fs::create_dir(&target).unwrap();
        std::fs::write(target.join("version"), "old").unwrap();

        promote_staged_skill(&staged, &target).unwrap();

        assert_eq!(
            std::fs::read_to_string(target.join("version")).unwrap(),
            "new"
        );
        let entries = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect::<Vec<_>>();
        assert_eq!(entries, vec![std::ffi::OsString::from("skill")]);
    }

    #[test]
    fn staging_setup_replaces_a_stale_directory() {
        let dir = tempfile::tempdir().unwrap();
        let staging = dir.path().join("staging");
        std::fs::create_dir(&staging).unwrap();
        std::fs::write(staging.join("stale"), "old").unwrap();

        let mut cleanup = prepare_staging_dir(&staging).unwrap();

        assert!(staging.is_dir());
        assert!(!staging.join("stale").exists());
        cleanup.disarm();
    }
}
