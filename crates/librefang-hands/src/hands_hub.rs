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
//! # SSRF / DNS-rebind hardening
//!
//! A caller-supplied `registry_url` is SSRF-checked at the API boundary
//! (`routes::skills::install_hand_from_marketplace`) before a client is built.
//! Two further guards live here so the check cannot be bypassed after the
//! string passes:
//!
//! - **Auto-redirects are disabled** ([`reqwest::redirect::Policy::none`]).
//!   A registry that passed the string check could otherwise 302-redirect
//!   `/index` or `/bundle` to `169.254.169.254` / an RFC1918 address. With
//!   the policy off, a 3xx is surfaced as an error instead of being followed
//!   into an internal target. The registry serves both endpoints directly, so
//!   no legitimate flow needs a redirect.
//! - **DNS is pinned** to the exact addresses the SSRF check already
//!   validated (via [`HandsHubClient::with_pinned_url`]). This closes the
//!   DNS-rebinding TOCTOU window between the check and the fetch — the IP we
//!   validated is the IP the client connects to.
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
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::net::IpAddr;
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
/// anything larger is almost certainly hostile or misconfigured. The download
/// is streamed and aborted the moment the running total exceeds this cap, so
/// a hostile registry cannot force the daemon to buffer an unbounded body
/// before the limit is enforced.
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
        Self::build(DEFAULT_BASE_URL, &[])
    }

    /// Create a client pointed at a custom registry base URL **without** DNS
    /// pinning.
    ///
    /// Prefer [`HandsHubClient::with_pinned_url`] for any caller-supplied URL:
    /// it pins DNS to the addresses the SSRF check already validated, closing
    /// the rebinding window. This unpinned constructor is retained for the
    /// in-crate tests that bind a loopback mock on a fixed IP.
    pub fn with_url(base_url: &str) -> Self {
        Self::build(base_url, &[])
    }

    /// Create a client pointed at a custom registry base URL with DNS pinned
    /// to `resolved` — the exact addresses the SSRF check validated for
    /// `hostname`. Auto-redirects are disabled regardless, so the client can
    /// only ever connect to a validated address for the registry host.
    pub fn with_pinned_url(base_url: &str, hostname: &str, resolved: &[IpAddr]) -> Self {
        let pins: Vec<(String, IpAddr)> = resolved
            .iter()
            .map(|ip| (hostname.to_string(), *ip))
            .collect();
        Self::build(base_url, &pins)
    }

    /// Shared constructor. `dns_pins` maps a hostname to a validated IP; when
    /// non-empty the reqwest client resolves that hostname only to the pinned
    /// addresses. Auto-redirects are always disabled so a 3xx from the
    /// registry is surfaced as an error rather than followed into an
    /// attacker-chosen (possibly internal) target.
    fn build(base_url: &str, dns_pins: &[(String, IpAddr)]) -> Self {
        let use_dangerous = std::env::var("LIBREFANG_DANGEROUSLY_SKIP_TLS_VERIFICATION")
            .map(|v| v.eq_ignore_ascii_case("true") || v == "1")
            .unwrap_or(false);

        let mut builder = if use_dangerous {
            warn!("TLS verification disabled - use only for testing!");
            reqwest::ClientBuilder::new()
                .danger_accept_invalid_certs(true)
                .danger_accept_invalid_hostnames(true)
        } else {
            reqwest::ClientBuilder::new()
        };

        // Disable auto-redirect: the registry serves /index and /bundle
        // directly, so a 3xx is either a misconfiguration or an SSRF-redirect
        // attempt (302 → 169.254.169.254 / RFC1918). `get_with_retry` treats
        // the surfaced 3xx as an error.
        builder = builder.redirect(reqwest::redirect::Policy::none());

        // Pin DNS to the SSRF-validated addresses. reqwest connects to the URL
        // port; the pinned SocketAddr only supplies the IP, so a port mismatch
        // in the validated address is irrelevant.
        for (host, ip) in dns_pins {
            builder = builder.resolve(host, std::net::SocketAddr::new(*ip, 0));
        }

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

                    // Auto-redirect is disabled (see `build`). A 3xx therefore
                    // reaches us unfollowed — refuse it rather than chase a
                    // Location header that may point at an internal address.
                    if status.is_redirection() {
                        return Err(HandError::Config(format!(
                            "{context} returned redirect {status} — the registry must serve \
                             {url} directly (redirects are refused to prevent SSRF)"
                        )));
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
    /// The body is streamed and the running total is checked against
    /// [`MAX_BUNDLE_BYTES`] on every chunk, so the download aborts the moment
    /// the cap is exceeded — a hostile registry cannot force the daemon to
    /// buffer an unbounded body. A truthful `Content-Length` is used as a
    /// fast pre-reject, but the streaming guard is authoritative because the
    /// header can lie or be absent.
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

        // Fast pre-reject on an honest Content-Length. The streaming guard
        // below is authoritative — a hostile registry can omit or understate
        // this header, so it is an optimisation, not the size enforcement.
        if let Some(len) = response.content_length() {
            if len > MAX_BUNDLE_BYTES as u64 {
                return Err(HandError::Config(format!(
                    "Hand bundle for '{hand_id}' advertises {len} bytes, exceeds the \
                     {MAX_BUNDLE_BYTES}-byte cap"
                )));
            }
        }

        // Stream the body, hashing as we go, and abort the instant the running
        // total exceeds the cap — before the oversized body is fully buffered.
        let mut stream = response.bytes_stream();
        let mut buf: Vec<u8> = Vec::new();
        let mut hasher = Sha256::new();
        while let Some(chunk) = stream.next().await {
            let chunk =
                chunk.map_err(|e| HandError::Config(format!("Failed to read bundle body: {e}")))?;
            if buf.len() + chunk.len() > MAX_BUNDLE_BYTES {
                return Err(HandError::Config(format!(
                    "Hand bundle for '{hand_id}' exceeds the {MAX_BUNDLE_BYTES}-byte cap"
                )));
            }
            hasher.update(&chunk);
            buf.extend_from_slice(&chunk);
        }

        // SHA-256 over the exact bytes served, validated BEFORE we parse or
        // write anything — fail fast on supply-chain tampering.
        let sha256 = hex::encode(hasher.finalize());

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

        let bundle: HandsHubBundle = serde_json::from_slice(&buf)
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

    #[test]
    fn pinned_client_trims_trailing_slash() {
        let c = HandsHubClient::with_pinned_url(
            "https://example.com/api/v1/",
            "example.com",
            &["93.184.216.34".parse().unwrap()],
        );
        assert_eq!(c.base_url, "https://example.com/api/v1");
    }

    /// F1 (#5954): the client must NOT follow a 302 on /bundle. A registry that
    /// passed the SSRF string check could otherwise redirect the fetch at an
    /// internal address. `download_bundle` surfaces the redirect as an error,
    /// and the mock's redirect target is never contacted.
    #[tokio::test]
    async fn download_bundle_refuses_redirect() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        // /bundle 302s to an internal-looking target.
        Mock::given(method("GET"))
            .and(path("/hands/clip/bundle"))
            .respond_with(
                ResponseTemplate::new(302)
                    .insert_header("location", "http://169.254.169.254/latest/meta-data/"),
            )
            .expect(1)
            .mount(&server)
            .await;

        let client = HandsHubClient::with_url(&server.uri());
        let err = client
            .download_bundle("clip", None)
            .await
            .expect_err("a 302 on /bundle must be refused, not followed");
        let msg = err.to_string();
        assert!(
            msg.contains("redirect") || msg.to_lowercase().contains("ssrf"),
            "error should name the refused redirect, got: {msg}"
        );
    }

    /// F2 (#5954): a bundle body larger than the cap is refused. With an
    /// honest Content-Length the fast pre-reject trips; the streaming guard in
    /// `download_bundle` is the authoritative backstop for an absent/lying
    /// header (it accumulates and aborts the instant the running total exceeds
    /// the cap). Either way the security property — oversized bodies never
    /// reach the parser — holds.
    #[tokio::test]
    async fn download_bundle_rejects_oversized_body() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        let oversized = vec![b'a'; MAX_BUNDLE_BYTES + 1024];
        Mock::given(method("GET"))
            .and(path("/hands/clip/bundle"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(oversized))
            .mount(&server)
            .await;

        let client = HandsHubClient::with_url(&server.uri());
        let err = client
            .download_bundle("clip", None)
            .await
            .expect_err("an oversized body must be refused");
        assert!(
            err.to_string().contains("cap"),
            "error should name the byte cap, got: {err}"
        );
    }
}
