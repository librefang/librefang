//! MCP OAuth discovery and authentication support.
//!
//! Implements RFC 8414 (OAuth Authorization Server Metadata) discovery
//! for MCP Streamable HTTP connections, with WWW-Authenticate header parsing,
//! PKCE support, and three-tier metadata resolution.

use async_trait::async_trait;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use librefang_types::config::McpOAuthConfig;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use tracing::{debug, warn};
use url::Url;

// ---------------------------------------------------------------------------
// Core types
// ---------------------------------------------------------------------------

/// Resolved OAuth metadata for an MCP server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuthMetadata {
    pub authorization_endpoint: String,
    pub token_endpoint: String,
    pub client_id: Option<String>,
    /// RFC 7591 Dynamic Client Registration endpoint.
    /// Used to obtain a `client_id` when none is configured.
    #[serde(default)]
    pub registration_endpoint: Option<String>,
    #[serde(default)]
    pub scopes: Vec<String>,
    /// Slack-style user scopes. Appended to the authorization URL as
    /// `&user_scope=...` when non-empty.
    #[serde(default)]
    pub user_scopes: Vec<String>,
    pub server_url: String,
}

/// Current authentication state for an MCP connection.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum McpAuthState {
    NotRequired,
    Authorized {
        #[serde(default)]
        expires_at: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tokens: Option<OAuthTokens>,
    },
    /// Server requires OAuth but the user hasn't started the flow yet.
    /// Set at daemon boot when a 401 is detected.
    NeedsAuth,
    /// OAuth flow is in progress — user clicked Authorize.
    PendingAuth {
        auth_url: String,
    },
    Expired,
    Error {
        message: String,
    },
}

/// Shared map of per-server MCP OAuth authentication states.
pub type McpAuthStates = tokio::sync::Mutex<std::collections::HashMap<String, McpAuthState>>;

/// OAuth token response from the token endpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuthTokens {
    pub access_token: String,
    #[serde(default)]
    pub refresh_token: Option<String>,
    #[serde(default = "default_token_type")]
    pub token_type: String,
    #[serde(default)]
    pub expires_in: u64,
    #[serde(default)]
    pub scope: String,
}

fn default_token_type() -> String {
    "Bearer".to_string()
}

// ---------------------------------------------------------------------------
// WWW-Authenticate parsing
// ---------------------------------------------------------------------------

/// Split a parameter string on commas, respecting quoted values.
///
/// For example: `realm="OAuth", error="invalid_token"` splits into
/// `["realm=\"OAuth\"", "error=\"invalid_token\""]`.
fn split_auth_params(s: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;

    for ch in s.chars() {
        match ch {
            '"' => {
                in_quotes = !in_quotes;
                current.push(ch);
            }
            ',' if !in_quotes => {
                let trimmed = current.trim().to_string();
                if !trimmed.is_empty() {
                    parts.push(trimmed);
                }
                current.clear();
            }
            _ => {
                current.push(ch);
            }
        }
    }
    let trimmed = current.trim().to_string();
    if !trimmed.is_empty() {
        parts.push(trimmed);
    }
    parts
}

/// Parse a `WWW-Authenticate: Bearer ...` header into key-value pairs.
///
/// Strips the "Bearer " prefix (case-insensitive), splits on commas respecting
/// quoted strings, and parses `key=value` / `key="value"` pairs.
pub fn parse_www_authenticate(header: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    let body = header
        .strip_prefix("Bearer ")
        .or_else(|| header.strip_prefix("bearer "));
    let body = match body {
        Some(b) => b,
        None => return map,
    };

    for param in split_auth_params(body) {
        if let Some((key, value)) = param.split_once('=') {
            let key = key.trim().to_lowercase();
            let value = value.trim().trim_matches('"').to_string();
            map.insert(key, value);
        }
    }
    map
}

/// Extract the `resource_metadata` URL from parsed WWW-Authenticate parameters.
///
/// Validates with three layered checks before returning:
/// 1. **HTTPS only** — RFC 8414 requires TLS; `http://` is rejected.
/// 2. **Same-origin** — the metadata URL must share scheme+host+port with `server_url`
///    to prevent a rogue MCP server from redirecting OAuth discovery cross-domain.
/// 3. **No loopback / link-local / private IPs** — belt-and-braces defence-in-depth
///    even in the (unlikely) case same-origin passes on a private-range server.
pub fn extract_metadata_url(params: &HashMap<String, String>, server_url: &str) -> Option<String> {
    let url_str = params.get("resource_metadata")?;

    // Layer 1: HTTPS only
    if !url_str.starts_with("https://") {
        return None;
    }

    let metadata_url = Url::parse(url_str).ok()?;
    let server_parsed = Url::parse(server_url).ok()?;

    // Layer 2: Same-origin — compares scheme, host, and port
    if metadata_url.origin() != server_parsed.origin() {
        return None;
    }

    // Layer 3: Block loopback / link-local / private addresses
    let host = metadata_url.host_str()?;
    if is_ssrf_blocked_host(host) {
        return None;
    }

    Some(url_str.clone())
}

/// Return `true` when the given host string resolves to a network range that
/// must not be reachable via OAuth metadata fetches (SSRF defence-in-depth).
///
/// Blocked ranges:
/// * Exact hostnames: `localhost`, `metadata.google.internal`
/// * IPv4 loopback      127.0.0.0/8
/// * IPv4 private       10.0.0.0/8, 172.16.0.0/12, 192.168.0.0/16
/// * IPv4 link-local    169.254.0.0/16
/// * IPv6 loopback      ::1
/// * IPv6 unique-local  fc00::/7
/// * IPv6 link-local    fe80::/10
fn is_ssrf_blocked_host(host: &str) -> bool {
    use std::net::IpAddr;

    let lower = host.to_lowercase();
    if lower == "localhost" || lower == "metadata.google.internal" {
        return true;
    }

    if let Ok(ip) = host.parse::<IpAddr>() {
        return match ip {
            IpAddr::V4(v4) => {
                let o = v4.octets();
                // 127.0.0.0/8 loopback
                o[0] == 127
                // 10.0.0.0/8
                || o[0] == 10
                // 172.16.0.0/12
                || (o[0] == 172 && (o[1] & 0xf0) == 16)
                // 192.168.0.0/16
                || (o[0] == 192 && o[1] == 168)
                // 169.254.0.0/16 link-local
                || (o[0] == 169 && o[1] == 254)
            }
            IpAddr::V6(v6) => {
                let segs = v6.segments();
                // ::1 loopback
                v6.is_loopback()
                // fc00::/7 unique-local
                || (segs[0] & 0xfe00) == 0xfc00
                // fe80::/10 link-local
                || (segs[0] & 0xffc0) == 0xfe80
            }
        };
    }

    false
}

/// Construct the `.well-known/oauth-authorization-server` URL for a given server URL.
///
/// Parses the URL, extracts the origin, and appends the well-known path.
pub fn well_known_url(server_url: &str) -> Option<String> {
    let parsed = Url::parse(server_url).ok()?;
    let origin = parsed.origin().unicode_serialization();
    Some(format!("{}/.well-known/oauth-authorization-server", origin))
}

// ---------------------------------------------------------------------------
// PKCE helpers
// ---------------------------------------------------------------------------

/// Generate a PKCE code verifier and challenge pair.
///
/// Returns `(verifier, challenge)` where:
/// - `verifier` is 32 random bytes encoded as base64url (no padding)
/// - `challenge` is SHA-256 of verifier encoded as base64url (no padding)
pub fn generate_pkce() -> (String, String) {
    let mut buf = [0u8; 32];
    rand::fill(&mut buf);
    let verifier = URL_SAFE_NO_PAD.encode(buf);
    let digest = Sha256::digest(verifier.as_bytes());
    let challenge = URL_SAFE_NO_PAD.encode(digest);
    (verifier, challenge)
}

/// Generate a random state parameter for OAuth flows.
///
/// Returns 16 random bytes encoded as base64url (no padding).
pub fn generate_state() -> String {
    let mut buf = [0u8; 16];
    rand::fill(&mut buf);
    URL_SAFE_NO_PAD.encode(buf)
}

// ---------------------------------------------------------------------------
// Metadata merge
// ---------------------------------------------------------------------------

/// Merge discovered OAuth metadata with user-provided config overrides.
///
/// Config values take precedence over discovered values. Empty scopes in
/// config means use discovered scopes.
pub fn merge_metadata_with_config(
    discovered: OAuthMetadata,
    config: &McpOAuthConfig,
) -> OAuthMetadata {
    OAuthMetadata {
        authorization_endpoint: config
            .auth_url
            .clone()
            .unwrap_or(discovered.authorization_endpoint),
        token_endpoint: config
            .token_url
            .clone()
            .unwrap_or(discovered.token_endpoint),
        client_id: config.client_id.clone().or(discovered.client_id),
        registration_endpoint: discovered.registration_endpoint,
        scopes: if config.scopes.is_empty() {
            discovered.scopes
        } else {
            config.scopes.clone()
        },
        user_scopes: if config.user_scopes.is_empty() {
            discovered.user_scopes
        } else {
            config.user_scopes.clone()
        },
        server_url: discovered.server_url,
    }
}

// ---------------------------------------------------------------------------
// Auth flow handle + provider trait
// ---------------------------------------------------------------------------

/// Trait for OAuth token storage and management.
///
/// Implementors handle persistence of tokens (e.g., encrypted vault on disk).
/// The actual OAuth flow (PKCE, browser redirect) is driven by the API layer,
/// not by the provider — the provider only handles token CRUD.
#[async_trait]
pub trait McpOAuthProvider: Send + Sync {
    /// Load a cached access token for the given server URL.
    async fn load_token(&self, server_url: &str) -> Option<String>;

    /// Store tokens received from the token endpoint.
    async fn store_tokens(&self, server_url: &str, tokens: OAuthTokens) -> Result<(), String>;

    /// Clear stored tokens for the given server URL.
    async fn clear_tokens(&self, server_url: &str) -> Result<(), String>;
}

// ---------------------------------------------------------------------------
// .well-known metadata discovery
// ---------------------------------------------------------------------------

/// Raw OAuth Authorization Server Metadata (RFC 8414) response.
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct AuthorizationServerMetadata {
    authorization_endpoint: String,
    token_endpoint: String,
    #[serde(default)]
    registration_endpoint: Option<String>,
    #[serde(default)]
    code_challenge_methods_supported: Vec<String>,
}

/// Parse a JSON body into `OAuthMetadata`.
///
/// Expects the body to be a valid OAuth Authorization Server Metadata document
/// (RFC 8414). Extracts the required endpoints and converts to our internal type.
pub fn parse_authorization_server_metadata(
    body: &str,
    server_url: &str,
) -> Result<OAuthMetadata, String> {
    let raw: AuthorizationServerMetadata =
        serde_json::from_str(body).map_err(|e| format!("Failed to parse metadata JSON: {e}"))?;

    Ok(OAuthMetadata {
        authorization_endpoint: raw.authorization_endpoint,
        token_endpoint: raw.token_endpoint,
        client_id: None,
        registration_endpoint: raw.registration_endpoint,
        scopes: Vec::new(),
        user_scopes: Vec::new(),
        server_url: server_url.to_string(),
    })
}

/// Discover OAuth metadata for an MCP server using three-tier resolution.
///
/// 1. **Tier 1**: Parse `www_authenticate` header -> extract `resource_metadata` URL -> fetch -> parse.
/// 2. **Tier 2**: Construct `.well-known/oauth-authorization-server` URL from server_url -> fetch -> parse.
/// 3. **Tier 3**: Fall back to config (requires both `auth_url` and `token_url`).
///
/// If config is provided, it is merged with discovery results (config values take precedence).
/// Returns an error if all tiers fail.
pub async fn discover_oauth_metadata(
    server_url: &str,
    www_authenticate: Option<&str>,
    config: Option<&McpOAuthConfig>,
) -> Result<OAuthMetadata, String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| format!("Failed to build HTTP client: {e}"))?;

    // Tier 1: WWW-Authenticate header -> resource_metadata URL
    if let Some(header) = www_authenticate {
        let params = parse_www_authenticate(header);
        if let Some(metadata_url) = extract_metadata_url(&params, server_url) {
            debug!(url = %metadata_url, "Tier 1: fetching metadata from WWW-Authenticate resource_metadata");
            match client.get(&metadata_url).send().await {
                Ok(resp) if resp.status().is_success() => {
                    if let Ok(body) = resp.text().await {
                        match parse_authorization_server_metadata(&body, server_url) {
                            Ok(meta) => {
                                let meta = if let Some(cfg) = config {
                                    merge_metadata_with_config(meta, cfg)
                                } else {
                                    meta
                                };
                                return Ok(meta);
                            }
                            Err(e) => {
                                warn!(error = %e, "Tier 1: failed to parse metadata");
                            }
                        }
                    }
                }
                Ok(resp) => {
                    warn!(status = %resp.status(), "Tier 1: metadata fetch returned non-success");
                }
                Err(e) => {
                    warn!(error = %e, "Tier 1: metadata fetch failed");
                }
            }
        }
    }

    // Tier 2: .well-known URL
    if let Some(wk_url) = well_known_url(server_url) {
        debug!(url = %wk_url, "Tier 2: fetching .well-known metadata");
        match client.get(&wk_url).send().await {
            Ok(resp) if resp.status().is_success() => {
                if let Ok(body) = resp.text().await {
                    match parse_authorization_server_metadata(&body, server_url) {
                        Ok(meta) => {
                            let meta = if let Some(cfg) = config {
                                merge_metadata_with_config(meta, cfg)
                            } else {
                                meta
                            };
                            return Ok(meta);
                        }
                        Err(e) => {
                            warn!(error = %e, "Tier 2: failed to parse .well-known metadata");
                        }
                    }
                }
            }
            Ok(resp) => {
                warn!(status = %resp.status(), "Tier 2: .well-known fetch returned non-success");
            }
            Err(e) => {
                warn!(error = %e, "Tier 2: .well-known fetch failed");
            }
        }
    }

    // Tier 3: Config fallback
    if let Some(cfg) = config {
        if let (Some(auth_url), Some(token_url)) = (&cfg.auth_url, &cfg.token_url) {
            debug!("Tier 3: using config fallback");
            return Ok(OAuthMetadata {
                authorization_endpoint: auth_url.clone(),
                token_endpoint: token_url.clone(),
                client_id: cfg.client_id.clone(),
                registration_endpoint: None,
                scopes: cfg.scopes.clone(),
                user_scopes: cfg.user_scopes.clone(),
                server_url: server_url.to_string(),
            });
        }
    }

    Err(format!(
        "OAuth metadata discovery failed for {server_url}: \
         no resource_metadata in WWW-Authenticate, .well-known fetch failed, \
         and no config fallback (auth_url + token_url) provided"
    ))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -- split_auth_params tests --

    #[test]
    fn test_split_auth_params_simple() {
        let parts = split_auth_params(r#"realm="OAuth", error="invalid_token""#);
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[0], r#"realm="OAuth""#);
        assert_eq!(parts[1], r#"error="invalid_token""#);
    }

    #[test]
    fn test_split_auth_params_commas_in_quotes() {
        let parts = split_auth_params(r#"realm="OAuth, v2", error="bad""#);
        assert_eq!(parts.len(), 2);
        assert!(parts[0].contains("OAuth, v2"));
    }

    #[test]
    fn test_split_auth_params_empty() {
        let parts = split_auth_params("");
        assert!(parts.is_empty());
    }

    // -- parse_www_authenticate tests --

    #[test]
    fn test_parse_www_authenticate_basic() {
        let map = parse_www_authenticate(
            r#"Bearer realm="OAuth", error="invalid_token", error_description="Token expired""#,
        );
        assert_eq!(map.get("realm").unwrap(), "OAuth");
        assert_eq!(map.get("error").unwrap(), "invalid_token");
        assert_eq!(map.get("error_description").unwrap(), "Token expired");
    }

    #[test]
    fn test_parse_www_authenticate_with_resource_metadata() {
        let map = parse_www_authenticate(
            r#"Bearer realm="mcp", resource_metadata="https://auth.example.com/.well-known/oauth-authorization-server""#,
        );
        assert_eq!(map.get("realm").unwrap(), "mcp");
        assert_eq!(
            map.get("resource_metadata").unwrap(),
            "https://auth.example.com/.well-known/oauth-authorization-server"
        );
    }

    #[test]
    fn test_parse_www_authenticate_no_bearer_prefix() {
        let map = parse_www_authenticate("Basic realm=\"test\"");
        assert!(map.is_empty());
    }

    #[test]
    fn test_parse_www_authenticate_case_insensitive_prefix() {
        let map = parse_www_authenticate(r#"bearer realm="test""#);
        assert_eq!(map.get("realm").unwrap(), "test");
    }

    // -- extract_metadata_url tests --

    #[test]
    fn test_extract_metadata_url_present() {
        let mut params = HashMap::new();
        params.insert(
            "resource_metadata".to_string(),
            "https://example.com/.well-known/oauth-authorization-server".to_string(),
        );
        // Same origin: metadata and server both on example.com
        let url = extract_metadata_url(&params, "https://example.com/mcp");
        assert_eq!(
            url.unwrap(),
            "https://example.com/.well-known/oauth-authorization-server"
        );
    }

    #[test]
    fn test_extract_metadata_url_missing() {
        let params = HashMap::new();
        assert!(extract_metadata_url(&params, "https://example.com/mcp").is_none());
    }

    #[test]
    fn test_extract_metadata_url_invalid_scheme() {
        let mut params = HashMap::new();
        params.insert(
            "resource_metadata".to_string(),
            "ftp://bad.example.com".to_string(),
        );
        assert!(extract_metadata_url(&params, "https://example.com/mcp").is_none());
    }

    // -- B2: SSRF hardening tests for extract_metadata_url --

    #[test]
    fn extract_metadata_url_rejects_http() {
        let mut params = HashMap::new();
        params.insert(
            "resource_metadata".to_string(),
            "http://example.com/meta".to_string(),
        );
        assert!(extract_metadata_url(&params, "https://example.com/mcp").is_none());
    }

    #[test]
    fn extract_metadata_url_rejects_cross_origin() {
        let mut params = HashMap::new();
        params.insert(
            "resource_metadata".to_string(),
            "https://evil.com/meta".to_string(),
        );
        assert!(extract_metadata_url(&params, "https://example.com/mcp").is_none());
    }

    #[test]
    fn extract_metadata_url_accepts_same_origin_https() {
        let mut params = HashMap::new();
        params.insert(
            "resource_metadata".to_string(),
            "https://example.com/meta".to_string(),
        );
        let result = extract_metadata_url(&params, "https://example.com/mcp");
        assert_eq!(result.unwrap(), "https://example.com/meta");
    }

    #[test]
    fn extract_metadata_url_rejects_loopback_literal() {
        // Same-origin already rejects this (different hosts), but layer 3 also blocks
        // loopback IPs as defence-in-depth.
        let mut params = HashMap::new();
        params.insert(
            "resource_metadata".to_string(),
            "https://127.0.0.1/meta".to_string(),
        );
        assert!(extract_metadata_url(&params, "https://example.com/mcp").is_none());
    }

    #[test]
    fn extract_metadata_url_rejects_link_local() {
        let mut params = HashMap::new();
        params.insert(
            "resource_metadata".to_string(),
            "https://169.254.169.254/latest/meta-data/".to_string(),
        );
        assert!(extract_metadata_url(&params, "https://example.com/mcp").is_none());
    }

    #[test]
    fn extract_metadata_url_rejects_missing_scheme() {
        let mut params = HashMap::new();
        params.insert(
            "resource_metadata".to_string(),
            "example.com/meta".to_string(),
        );
        assert!(extract_metadata_url(&params, "https://example.com/mcp").is_none());
    }

    // -- well_known_url tests --

    #[test]
    fn test_well_known_url_basic() {
        let url = well_known_url("https://my-server.com/mcp").unwrap();
        assert_eq!(
            url,
            "https://my-server.com/.well-known/oauth-authorization-server"
        );
    }

    #[test]
    fn test_well_known_url_with_port() {
        let url = well_known_url("https://my-server.com:8443/mcp/v1").unwrap();
        assert_eq!(
            url,
            "https://my-server.com:8443/.well-known/oauth-authorization-server"
        );
    }

    #[test]
    fn test_well_known_url_invalid() {
        assert!(well_known_url("not-a-url").is_none());
    }

    #[test]
    fn test_well_known_url_http() {
        let url = well_known_url("http://localhost:3000/mcp").unwrap();
        assert_eq!(
            url,
            "http://localhost:3000/.well-known/oauth-authorization-server"
        );
    }

    // -- PKCE tests --

    #[test]
    fn test_generate_pkce_length() {
        let (verifier, challenge) = generate_pkce();
        // 32 bytes -> 43 base64url chars (no padding)
        assert_eq!(verifier.len(), 43);
        // SHA-256 -> 32 bytes -> 43 base64url chars
        assert_eq!(challenge.len(), 43);
    }

    #[test]
    fn test_generate_pkce_uniqueness() {
        let (v1, c1) = generate_pkce();
        let (v2, c2) = generate_pkce();
        assert_ne!(v1, v2);
        assert_ne!(c1, c2);
    }

    #[test]
    fn test_generate_pkce_challenge_is_sha256_of_verifier() {
        let (verifier, challenge) = generate_pkce();
        let digest = Sha256::digest(verifier.as_bytes());
        let expected = URL_SAFE_NO_PAD.encode(digest);
        assert_eq!(challenge, expected);
    }

    // -- state generation tests --

    #[test]
    fn test_generate_state_length() {
        let state = generate_state();
        // 16 bytes -> 22 base64url chars (no padding)
        assert_eq!(state.len(), 22);
    }

    #[test]
    fn test_generate_state_uniqueness() {
        let s1 = generate_state();
        let s2 = generate_state();
        assert_ne!(s1, s2);
    }

    // -- metadata merge tests --

    #[test]
    fn test_merge_metadata_config_overrides_endpoints() {
        let discovered = OAuthMetadata {
            authorization_endpoint: "https://discovered.com/auth".to_string(),
            token_endpoint: "https://discovered.com/token".to_string(),
            client_id: Some("discovered-client".to_string()),
            registration_endpoint: None,
            scopes: vec!["read".to_string()],
            user_scopes: Vec::new(),
            server_url: "https://server.com/mcp".to_string(),
        };
        let config = McpOAuthConfig {
            auth_url: Some("https://override.com/auth".to_string()),
            token_url: Some("https://override.com/token".to_string()),
            client_id: Some("override-client".to_string()),
            scopes: vec!["admin".to_string()],
            user_scopes: Vec::new(),
        };
        let merged = merge_metadata_with_config(discovered, &config);
        assert_eq!(merged.authorization_endpoint, "https://override.com/auth");
        assert_eq!(merged.token_endpoint, "https://override.com/token");
        assert_eq!(merged.client_id.unwrap(), "override-client");
        assert_eq!(merged.scopes, vec!["admin"]);
        assert_eq!(merged.server_url, "https://server.com/mcp");
    }

    #[test]
    fn test_merge_metadata_empty_config_keeps_discovered() {
        let discovered = OAuthMetadata {
            authorization_endpoint: "https://discovered.com/auth".to_string(),
            token_endpoint: "https://discovered.com/token".to_string(),
            client_id: Some("discovered-client".to_string()),
            registration_endpoint: None,
            scopes: vec!["read".to_string(), "write".to_string()],
            user_scopes: Vec::new(),
            server_url: "https://server.com/mcp".to_string(),
        };
        let config = McpOAuthConfig::default();
        let merged = merge_metadata_with_config(discovered, &config);
        assert_eq!(merged.authorization_endpoint, "https://discovered.com/auth");
        assert_eq!(merged.token_endpoint, "https://discovered.com/token");
        assert_eq!(merged.client_id.unwrap(), "discovered-client");
        assert_eq!(merged.scopes, vec!["read", "write"]);
    }

    // -- parse_authorization_server_metadata tests --

    #[test]
    fn test_parse_authorization_server_metadata_success() {
        let body = r#"{
            "authorization_endpoint": "https://auth.example.com/authorize",
            "token_endpoint": "https://auth.example.com/token",
            "registration_endpoint": "https://auth.example.com/register",
            "code_challenge_methods_supported": ["S256"]
        }"#;
        let meta = parse_authorization_server_metadata(body, "https://server.com/mcp").unwrap();
        assert_eq!(
            meta.authorization_endpoint,
            "https://auth.example.com/authorize"
        );
        assert_eq!(meta.token_endpoint, "https://auth.example.com/token");
        assert!(meta.client_id.is_none());
        assert!(meta.scopes.is_empty());
        assert_eq!(meta.server_url, "https://server.com/mcp");
    }

    #[test]
    fn test_parse_authorization_server_metadata_missing_fields() {
        let body = r#"{"authorization_endpoint": "https://auth.example.com/authorize"}"#;
        let result = parse_authorization_server_metadata(body, "https://server.com/mcp");
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .contains("Failed to parse metadata JSON"));
    }

    #[test]
    fn test_parse_authorization_server_metadata_invalid_json() {
        let result = parse_authorization_server_metadata("not json", "https://server.com/mcp");
        assert!(result.is_err());
    }
}
