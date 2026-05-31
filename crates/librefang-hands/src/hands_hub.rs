//! HandsHub marketplace client — discover, download, and install hands from a
//! remote registry.
//!
//! This mirrors the skills marketplace client (`librefang_skills::clawhub`):
//! it fetches a remote index, lets callers browse / search entries, downloads a
//! hand bundle, and verifies the bundle's SHA-256 against the
//! registry-supplied digest before it ever reaches disk.
//!
//! # Trust model
//!
//! Parity with the *skills* marketplace means SHA-256 checksum verification.
//! The skills marketplace client (`clawhub.rs`) does **not** do Ed25519
//! signature verification — that chain belongs to a separate subsystem (the
//! plugin registry in `librefang-runtime::plugin_manager`) whose embedded
//! public keys are scoped to `librefang/librefang-registry` *plugins*, not
//! hands. We therefore verify each bundle's content hash against the digest
//! the index advertises, exactly as the skills installer does. A self-hosted
//! registry that wants stronger provenance can front the index with HTTPS and
//! a TOFU-pinned host; index-level Ed25519 signing for hands would need its
//! own key material and is out of scope for this client.
//!
//! # Bundle shape
//!
//! A hand bundle is a small JSON envelope returned by the registry's download
//! endpoint:
//!
//! ```json
//! { "toml": "<HAND.toml contents>", "skill": "<SKILL.md contents>" }
//! ```
//!
//! The `skill` field is optional (prompt-less hands omit it). This is the
//! exact pair the existing `install_from_content_persisted` already consumes,
//! so the installer reuses that path verbatim.

use crate::HandError;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use tracing::{debug, info, warn};

/// Default registry base URL. Self-hosted forks override via
/// [`HandsHubClient::with_url`]. The path mirrors the community hands repo
/// referenced in the project README (github.com/librefang-registry/hands).
const DEFAULT_BASE_URL: &str = "https://hands.librefang.ai/api/v1";

/// Maximum number of retry attempts for registry API calls (including the
/// first try). Mirrors the skills client.
const MAX_RETRIES: u32 = 5;

/// Base delay in milliseconds for exponential backoff (doubles each attempt).
const BASE_DELAY_MS: u64 = 1_500;

/// Maximum delay cap in milliseconds.
const MAX_DELAY_MS: u64 = 30_000;

/// Hard cap on a downloaded bundle (bytes). A hand bundle is two text files;
/// anything larger is almost certainly hostile or misconfigured and is refused
/// before it is buffered fully.
const MAX_BUNDLE_BYTES: usize = 8 * 1024 * 1024;

// ---------------------------------------------------------------------------
// Index / entry types
// ---------------------------------------------------------------------------

/// One hand entry in the remote registry index.
///
/// `tags` is an ordered map so the index stringifies deterministically when it
/// is surfaced to a prompt or cached (refs the deterministic-ordering rule).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HandsHubEntry {
    /// Stable hand id (also the download key), e.g. `"clip"`.
    pub id: String,
    /// Human-readable name.
    #[serde(default)]
    pub name: String,
    /// One-line summary for the marketplace listing.
    #[serde(default)]
    pub description: String,
    /// Category slug (matches `HandCategory` serialization, e.g. `"data"`).
    #[serde(default)]
    pub category: String,
    /// Latest published version (semver).
    #[serde(default)]
    pub version: String,
    /// Free-form tags (e.g. `{"latest": "1.2.0"}`). Ordered for determinism.
    #[serde(default)]
    pub tags: BTreeMap<String, String>,
    /// Expected SHA-256 hex digest of the hand bundle. When present the
    /// installer validates the download before it touches disk.
    #[serde(default)]
    pub expected_sha256: Option<String>,
}

/// The registry index document (`GET /api/v1/index`).
///
/// Entries are stored in a `Vec` as served, but [`HandsHubClient::browse`] and
/// [`HandsHubClient::search`] sort by id so callers get a deterministic order
/// regardless of how the registry happened to serialize the array.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct HandsHubIndex {
    #[serde(default)]
    pub hands: Vec<HandsHubEntry>,
}

/// A downloadable hand bundle (`GET /api/v1/hands/{id}/bundle`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HandsHubBundle {
    /// Raw HAND.toml contents.
    pub toml: String,
    /// Raw SKILL.md contents (optional — prompt-less hands omit it).
    #[serde(default)]
    pub skill: String,
}

/// Result of installing a hand from the remote registry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HandsHubInstallResult {
    /// Installed hand id.
    pub hand_id: String,
    /// Installed version (from the index entry, empty when unknown).
    pub version: String,
    /// Whether the bundle's SHA-256 was verified against the index digest.
    pub checksum_verified: bool,
}

// ---------------------------------------------------------------------------
// Client
// ---------------------------------------------------------------------------

/// Client for the remote hands marketplace.
pub struct HandsHubClient {
    base_url: String,
    client: reqwest::Client,
}

impl HandsHubClient {
    /// Create a client pointed at the default registry.
    pub fn new() -> Self {
        Self::with_url(DEFAULT_BASE_URL)
    }

    /// Create a client pointed at a custom registry base URL.
    pub fn with_url(base_url: &str) -> Self {
        let use_dangerous = std::env::var("LIBREFANG_DANGEROUSLY_SKIP_TLS_VERIFICATION")
            .map(|v| v.eq_ignore_ascii_case("true") || v == "1")
            .unwrap_or(false);

        let builder = if use_dangerous {
            warn!("TLS verification disabled - use only for testing!");
            reqwest::ClientBuilder::new()
                .danger_accept_invalid_certs(true)
                .danger_accept_invalid_hostnames(true)
        } else {
            reqwest::ClientBuilder::new()
        };

        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            client: builder
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .expect("HTTP client build"),
        }
    }

    // -----------------------------------------------------------------------
    // HTTP GET with retry on 429 / 5xx (mirrors clawhub's get_with_retry)
    // -----------------------------------------------------------------------

    async fn get_with_retry(
        &self,
        url: &str,
        context: &str,
    ) -> Result<reqwest::Response, HandError> {
        let mut last_status: Option<u16> = None;

        for attempt in 0..MAX_RETRIES {
            if attempt > 0 {
                let base = BASE_DELAY_MS.saturating_mul(1u64 << attempt.min(5));
                let delay_ms = base.min(MAX_DELAY_MS);
                let jitter = {
                    let nanos = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .subsec_nanos();
                    let frac = (nanos.wrapping_mul(2654435761) as f64) / (u32::MAX as f64);
                    (delay_ms as f64 * frac * 0.25) as u64
                };
                let total = delay_ms + jitter;
                debug!(
                    attempt,
                    delay_ms = total,
                    context,
                    "retrying HandsHub request after rate limit / server error"
                );
                tokio::time::sleep(std::time::Duration::from_millis(total)).await;
            }

            match self
                .client
                .get(url)
                .header("User-Agent", "LibreFang/0.1")
                .send()
                .await
            {
                Ok(resp) => {
                    let status = resp.status();
                    if status.is_success() {
                        return Ok(resp);
                    }

                    if status.as_u16() == 429 || status.is_server_error() {
                        last_status = Some(status.as_u16());
                        if let Some(ra) = resp
                            .headers()
                            .get("retry-after")
                            .and_then(|v| v.to_str().ok())
                            .and_then(|v| v.parse::<u64>().ok())
                        {
                            let capped = (ra * 1000).min(MAX_DELAY_MS);
                            if attempt + 1 < MAX_RETRIES {
                                tokio::time::sleep(std::time::Duration::from_millis(capped)).await;
                            }
                        }
                        if attempt + 1 >= MAX_RETRIES {
                            return Err(HandError::Config(format!(
                                "{context} returned {status} after {MAX_RETRIES} attempts"
                            )));
                        }
                        continue;
                    }

                    return Err(HandError::Config(format!("{context} returned {status}")));
                }
                Err(e) => {
                    if attempt + 1 >= MAX_RETRIES {
                        return Err(HandError::Config(format!(
                            "{context} failed after {MAX_RETRIES} attempts: {e}"
                        )));
                    }
                    warn!(attempt, context, error = %e, "HandsHub request failed, will retry");
                }
            }
        }

        Err(HandError::Config(format!(
            "{context} failed (status: {last_status:?}) after {MAX_RETRIES} attempts"
        )))
    }

    // -----------------------------------------------------------------------
    // Public API
    // -----------------------------------------------------------------------

    /// Fetch the full registry index.
    pub async fn fetch_index(&self) -> Result<HandsHubIndex, HandError> {
        let url = format!("{}/index", self.base_url);
        let response = self.get_with_retry(&url, "HandsHub index").await?;
        let index: HandsHubIndex = response
            .json()
            .await
            .map_err(|e| HandError::Config(format!("Failed to parse HandsHub index: {e}")))?;
        Ok(index)
    }

    /// Browse the registry, returning up to `limit` entries sorted by id.
    pub async fn browse(&self, limit: usize) -> Result<Vec<HandsHubEntry>, HandError> {
        let mut entries = self.fetch_index().await?.hands;
        entries.sort_by(|a, b| a.id.cmp(&b.id));
        entries.truncate(limit);
        Ok(entries)
    }

    /// Search the index by a case-insensitive substring match over id, name,
    /// and description. Results are sorted by id for determinism.
    pub async fn search(&self, query: &str, limit: usize) -> Result<Vec<HandsHubEntry>, HandError> {
        let needle = query.trim().to_ascii_lowercase();
        let mut entries: Vec<HandsHubEntry> = self
            .fetch_index()
            .await?
            .hands
            .into_iter()
            .filter(|e| {
                needle.is_empty()
                    || e.id.to_ascii_lowercase().contains(&needle)
                    || e.name.to_ascii_lowercase().contains(&needle)
                    || e.description.to_ascii_lowercase().contains(&needle)
            })
            .collect();
        entries.sort_by(|a, b| a.id.cmp(&b.id));
        entries.truncate(limit);
        Ok(entries)
    }

    /// Look up a single index entry by id.
    pub async fn get_entry(&self, hand_id: &str) -> Result<Option<HandsHubEntry>, HandError> {
        validate_hand_id(hand_id)?;
        let index = self.fetch_index().await?;
        Ok(index.hands.into_iter().find(|e| e.id == hand_id))
    }

    /// Download a hand bundle and verify its SHA-256 against `expected_sha256`
    /// when provided.
    ///
    /// Returns the parsed bundle plus a flag recording whether the checksum was
    /// actually verified (false when the registry advertised no digest).
    pub async fn download_bundle(
        &self,
        hand_id: &str,
        expected_sha256: Option<&str>,
    ) -> Result<(HandsHubBundle, bool), HandError> {
        validate_hand_id(hand_id)?;
        let url = format!("{}/hands/{}/bundle", self.base_url, hand_id);
        info!(hand = hand_id, "Downloading hand bundle from HandsHub");

        let response = self
            .get_with_retry(&url, "HandsHub bundle download")
            .await?;
        let bytes = response
            .bytes()
            .await
            .map_err(|e| HandError::Config(format!("Failed to read bundle body: {e}")))?;

        if bytes.len() > MAX_BUNDLE_BYTES {
            return Err(HandError::Config(format!(
                "Hand bundle for '{hand_id}' is {} bytes, exceeds the {MAX_BUNDLE_BYTES}-byte cap",
                bytes.len()
            )));
        }

        // SHA-256 over the exact bytes served, validated BEFORE we parse or
        // write anything — fail fast on supply-chain tampering.
        let sha256 = {
            let mut hasher = Sha256::new();
            hasher.update(&bytes);
            hex::encode(hasher.finalize())
        };

        let checksum_verified = match expected_sha256 {
            Some(expected) => {
                let expected_lower = expected.trim().to_lowercase();
                if sha256 != expected_lower {
                    return Err(HandError::Config(format!(
                        "Hand '{hand_id}' bundle hash mismatch: expected {expected_lower}, got {sha256}"
                    )));
                }
                info!(hand = hand_id, "Hand bundle SHA-256 verified OK");
                true
            }
            None => {
                warn!(
                    hand = hand_id,
                    "HandsHub did not advertise expected_sha256 — bundle installed unverified"
                );
                false
            }
        };

        let bundle: HandsHubBundle = serde_json::from_slice(&bytes)
            .map_err(|e| HandError::Config(format!("Failed to parse hand bundle: {e}")))?;

        Ok((bundle, checksum_verified))
    }
}

impl Default for HandsHubClient {
    fn default() -> Self {
        Self::new()
    }
}

/// Reject ids that are not safe to interpolate into a URL path or a directory
/// name. Same character class the local registry uses for hand ids.
pub(crate) fn validate_hand_id(hand_id: &str) -> Result<(), HandError> {
    if hand_id.is_empty()
        || !hand_id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return Err(HandError::Config(format!("Invalid hand id '{hand_id}'")));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn index_entry_serde_round_trip() {
        let json = r#"{
            "id": "clip",
            "name": "Clip Hand",
            "description": "Clips things.",
            "category": "content",
            "version": "1.2.0",
            "tags": {"latest": "1.2.0"},
            "expected_sha256": "abc123"
        }"#;
        let entry: HandsHubEntry = serde_json::from_str(json).unwrap();
        assert_eq!(entry.id, "clip");
        assert_eq!(entry.version, "1.2.0");
        assert_eq!(entry.tags.get("latest").unwrap(), "1.2.0");
        assert_eq!(entry.expected_sha256.as_deref(), Some("abc123"));
    }

    #[test]
    fn index_entry_minimal_defaults() {
        // Only `id` is required; everything else defaults.
        let entry: HandsHubEntry = serde_json::from_str(r#"{"id":"bare"}"#).unwrap();
        assert_eq!(entry.id, "bare");
        assert!(entry.name.is_empty());
        assert!(entry.expected_sha256.is_none());
    }

    #[test]
    fn bundle_skill_field_optional() {
        let bundle: HandsHubBundle = serde_json::from_str(r#"{"toml":"id = \"x\""}"#).unwrap();
        assert_eq!(bundle.toml, "id = \"x\"");
        assert!(bundle.skill.is_empty());
    }

    #[test]
    fn validate_hand_id_rejects_traversal() {
        assert!(validate_hand_id("clip").is_ok());
        assert!(validate_hand_id("my-hand_2").is_ok());
        assert!(validate_hand_id("../etc").is_err());
        assert!(validate_hand_id("a/b").is_err());
        assert!(validate_hand_id("").is_err());
    }

    #[test]
    fn client_default_base_url() {
        let c = HandsHubClient::new();
        assert_eq!(c.base_url, "https://hands.librefang.ai/api/v1");
    }

    #[test]
    fn client_trims_trailing_slash() {
        let c = HandsHubClient::with_url("https://example.com/api/v1/");
        assert_eq!(c.base_url, "https://example.com/api/v1");
    }
}
