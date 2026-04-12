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
    #[serde(default)]
    pub scopes: Vec<String>,
    pub server_url: String,
}

/// Current authentication state for an MCP connection.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum McpAuthState {
    NotRequired,
    Authorized { expires_at: Option<String> },
    PendingAuth { auth_url: String },
    Expired,
}

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
/// Returns `Some(url)` if the key exists and starts with `http://` or `https://`.
pub fn extract_metadata_url(params: &HashMap<String, String>) -> Option<String> {
    params.get("resource_metadata").and_then(|url| {
        if url.starts_with("http://") || url.starts_with("https://") {
            Some(url.clone())
        } else {
            None
        }
    })
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
        scopes: if config.scopes.is_empty() {
            discovered.scopes
        } else {
            config.scopes.clone()
        },
        server_url: discovered.server_url,
    }
}

// ---------------------------------------------------------------------------
// Auth flow handle + provider trait
// ---------------------------------------------------------------------------

/// Handle returned when initiating an OAuth authorization flow.
///
/// Contains the authorization URL to present to the user and a oneshot
/// receiver that resolves when the flow completes (e.g., callback received).
pub struct AuthFlowHandle {
    pub auth_url: String,
    pub completion: tokio::sync::oneshot::Receiver<Result<OAuthTokens, String>>,
}

/// Trait for OAuth token storage and flow management.
///
/// Implementors handle persistence of tokens (e.g., to SQLite or filesystem)
/// and orchestration of the authorization flow (e.g., spawning a local HTTP
/// server for the callback).
#[async_trait]
pub trait McpOAuthProvider: Send + Sync {
    /// Load a cached access token for the given server URL.
    async fn load_token(&self, server_url: &str) -> Option<String>;

    /// Store tokens received from the token endpoint.
    async fn store_tokens(&self, server_url: &str, tokens: OAuthTokens) -> Result<(), String>;

    /// Clear stored tokens for the given server URL.
    async fn clear_tokens(&self, server_url: &str) -> Result<(), String>;

    /// Initiate the OAuth authorization flow and return a handle.
    async fn start_auth_flow(
        &self,
        server_url: &str,
        metadata: OAuthMetadata,
    ) -> Result<AuthFlowHandle, String>;
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
            "https://auth.example.com/.well-known/oauth-authorization-server".to_string(),
        );
        let url = extract_metadata_url(&params);
        assert_eq!(
            url.unwrap(),
            "https://auth.example.com/.well-known/oauth-authorization-server"
        );
    }

    #[test]
    fn test_extract_metadata_url_missing() {
        let params = HashMap::new();
        assert!(extract_metadata_url(&params).is_none());
    }

    #[test]
    fn test_extract_metadata_url_invalid_scheme() {
        let mut params = HashMap::new();
        params.insert(
            "resource_metadata".to_string(),
            "ftp://bad.example.com".to_string(),
        );
        assert!(extract_metadata_url(&params).is_none());
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
            scopes: vec!["read".to_string()],
            server_url: "https://server.com/mcp".to_string(),
        };
        let config = McpOAuthConfig {
            auth_url: Some("https://override.com/auth".to_string()),
            token_url: Some("https://override.com/token".to_string()),
            client_id: Some("override-client".to_string()),
            scopes: vec!["admin".to_string()],
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
            scopes: vec!["read".to_string(), "write".to_string()],
            server_url: "https://server.com/mcp".to_string(),
        };
        let config = McpOAuthConfig::default();
        let merged = merge_metadata_with_config(discovered, &config);
        assert_eq!(merged.authorization_endpoint, "https://discovered.com/auth");
        assert_eq!(merged.token_endpoint, "https://discovered.com/token");
        assert_eq!(merged.client_id.unwrap(), "discovered-client");
        assert_eq!(merged.scopes, vec!["read", "write"]);
    }
}
