# MCP OAuth Discovery Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add automatic OAuth discovery and authentication for MCP Streamable HTTP connections, so servers like Notion's hosted MCP work with zero config.

**Architecture:** Trait-based injection — `librefang-runtime` defines `McpOAuthProvider` trait with token load/store/auth operations. `librefang-kernel` implements it using extensions vault + PKCE. OAuth discovery, `WWW-Authenticate` parsing, and retry logic live in a new `runtime/mcp_oauth.rs`. API endpoints in `librefang-api` expose auth state and trigger flows from the dashboard.

**Tech Stack:** Rust, rmcp 1.3, reqwest, tokio, axum, serde, zeroize, sha2/base64 (PKCE), React/TanStack Query (dashboard)

**Design Spec:** `docs/superpowers/specs/2026-04-12-mcp-oauth-discovery-design.md`

---

## File Map

### New Files

| File | Crate | Responsibility |
|------|-------|----------------|
| `crates/librefang-runtime/src/mcp_oauth.rs` | runtime | `McpOAuthProvider` trait, `OAuthMetadata`, `McpAuthState`, `WWW-Authenticate` parser, `.well-known` fetcher, PKCE verifier/challenge generation |
| `crates/librefang-kernel/src/mcp_oauth_provider.rs` | kernel | `KernelOAuthProvider` implementing `McpOAuthProvider` using vault + extensions PKCE |
| `crates/librefang-api/src/routes/mcp_auth.rs` | api | `/api/mcp/{name}/auth/*` route handlers |

### Modified Files

| File | Change |
|------|--------|
| `crates/librefang-types/src/config/types.rs` | Add `McpOAuthConfig` struct, add `oauth` field to `McpServerConfigEntry` |
| `crates/librefang-runtime/src/mcp.rs` | Add `oauth_provider` param to `connect()` and `connect_streamable_http()`, add auth state to `McpConnection`, add 401 retry logic |
| `crates/librefang-runtime/src/lib.rs` | Add `pub mod mcp_oauth;` |
| `crates/librefang-kernel/src/kernel.rs` | Build `KernelOAuthProvider`, pass to `connect_mcp_servers()`, track auth state per server |
| `crates/librefang-api/src/routes/skills.rs` | Add `auth_state` to MCP server list/detail responses, register auth routes |
| `crates/librefang-api/dashboard/src/pages/McpServersPage.tsx` | Auth state badges, authorize/revoke buttons |

---

## Task 1: Config Types — `McpOAuthConfig`

**Files:**
- Modify: `crates/librefang-types/src/config/types.rs:3136-3155`

- [ ] **Step 1: Add `McpOAuthConfig` struct**

Add after the `McpTransportEntry` enum (after line ~3241) in `crates/librefang-types/src/config/types.rs`:

```rust
/// Optional OAuth configuration for an MCP server.
///
/// Used as fallback when the server doesn't support `.well-known` discovery,
/// or to override specific values from discovery. All fields are optional —
/// discovery results fill gaps, config values take precedence.
///
/// # Example (config.toml)
///
/// ```toml
/// [[mcp_servers]]
/// name = "custom-server"
/// transport = { type = "http", url = "https://my-server.com/mcp" }
///
/// [mcp_servers.oauth]
/// auth_url = "https://my-server.com/oauth/authorize"
/// token_url = "https://my-server.com/oauth/token"
/// client_id = "my-client-id"
/// scopes = ["read", "write"]
/// ```
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct McpOAuthConfig {
    /// OAuth authorization endpoint URL.
    #[serde(default)]
    pub auth_url: Option<String>,

    /// OAuth token endpoint URL.
    #[serde(default)]
    pub token_url: Option<String>,

    /// OAuth client ID. If omitted, uses the value from server discovery.
    #[serde(default)]
    pub client_id: Option<String>,

    /// OAuth scopes to request.
    #[serde(default)]
    pub scopes: Vec<String>,
}
```

- [ ] **Step 2: Add `oauth` field to `McpServerConfigEntry`**

In the same file, add the field to `McpServerConfigEntry` (after the `headers` field at line ~3150):

```rust
    /// Optional OAuth configuration for servers requiring authentication.
    /// If omitted, OAuth metadata is discovered from the server's
    /// `WWW-Authenticate` header or `.well-known/oauth-authorization-server`.
    #[serde(default)]
    pub oauth: Option<McpOAuthConfig>,
```

- [ ] **Step 3: Verify it compiles**

Run: `cargo build --workspace --lib`
Expected: Success — all fields are `Option` or `Default`, so existing configs without `oauth` are unchanged.

- [ ] **Step 4: Commit**

```bash
git add crates/librefang-types/src/config/types.rs
git commit -m "feat(types): add McpOAuthConfig for MCP server OAuth configuration"
```

---

## Task 2: Runtime — `mcp_oauth.rs` Core Types and WWW-Authenticate Parser

**Files:**
- Create: `crates/librefang-runtime/src/mcp_oauth.rs`
- Modify: `crates/librefang-runtime/src/lib.rs:38`

- [ ] **Step 1: Write tests for WWW-Authenticate parsing**

Create `crates/librefang-runtime/src/mcp_oauth.rs` with test module first:

```rust
//! MCP OAuth discovery and authentication.
//!
//! Handles automatic OAuth for MCP Streamable HTTP servers:
//! - Parses `WWW-Authenticate` headers from 401 responses
//! - Fetches `.well-known/oauth-authorization-server` metadata
//! - Defines `McpOAuthProvider` trait for token management (implemented by kernel)
//! - Generates PKCE challenges for authorization flows

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// OAuth metadata discovered from a server or provided via config.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuthMetadata {
    /// OAuth authorization endpoint.
    pub authorization_endpoint: String,
    /// OAuth token endpoint.
    pub token_endpoint: String,
    /// Client ID to use (from discovery or config).
    pub client_id: Option<String>,
    /// Scopes to request.
    #[serde(default)]
    pub scopes: Vec<String>,
    /// The MCP server URL this metadata is for.
    pub server_url: String,
}

/// Authentication state for an MCP server connection.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum McpAuthState {
    /// Server connected without requiring auth.
    NotRequired,
    /// Server authenticated with a valid token.
    Authorized {
        /// ISO 8601 expiry timestamp, if known.
        expires_at: Option<String>,
    },
    /// Waiting for user to complete browser auth flow.
    PendingAuth {
        /// URL the user must open to authorize.
        auth_url: String,
    },
    /// Token expired and no refresh token available.
    Expired,
}

/// Tokens returned by the OAuth token endpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuthTokens {
    pub access_token: String,
    #[serde(default)]
    pub refresh_token: Option<String>,
    #[serde(default)]
    pub token_type: String,
    #[serde(default)]
    pub expires_in: u64,
    #[serde(default)]
    pub scope: String,
}

/// Parse a `WWW-Authenticate` header value into key-value parameters.
///
/// Handles the format: `Bearer realm="OAuth", error="invalid_token", resource_metadata="https://..."`
pub fn parse_www_authenticate(header: &str) -> HashMap<String, String> {
    let mut params = HashMap::new();

    // Strip the "Bearer " prefix if present
    let body = header
        .strip_prefix("Bearer")
        .map(|s| s.trim_start())
        .unwrap_or(header);

    // Parse comma-separated key="value" or key=value pairs
    for part in split_auth_params(body) {
        let part = part.trim();
        if let Some((key, value)) = part.split_once('=') {
            let key = key.trim().to_lowercase();
            let value = value.trim().trim_matches('"').to_string();
            params.insert(key, value);
        }
    }

    params
}

/// Split auth header parameters on commas, respecting quoted strings.
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
                parts.push(std::mem::take(&mut current));
            }
            _ => current.push(ch),
        }
    }
    if !current.is_empty() {
        parts.push(current);
    }
    parts
}

/// Extract the OAuth metadata URL from a parsed `WWW-Authenticate` header.
///
/// Looks for the `resource_metadata` parameter first (MCP spec),
/// then falls back to deriving from `realm` if it looks like a URL.
pub fn extract_metadata_url(params: &HashMap<String, String>) -> Option<String> {
    // MCP spec: resource_metadata parameter points to OAuth metadata
    if let Some(url) = params.get("resource_metadata") {
        if url.starts_with("http://") || url.starts_with("https://") {
            return Some(url.clone());
        }
    }
    None
}

/// Derive the `.well-known/oauth-authorization-server` URL from a server URL.
pub fn well_known_url(server_url: &str) -> Option<String> {
    let parsed = url::Url::parse(server_url).ok()?;
    let origin = parsed.origin().unicode_serialization();
    Some(format!(
        "{}/.well-known/oauth-authorization-server",
        origin
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_www_authenticate_basic() {
        let header = r#"Bearer realm="OAuth", error="invalid_token", error_description="Missing or invalid access token""#;
        let params = parse_www_authenticate(header);
        assert_eq!(params.get("realm").unwrap(), "OAuth");
        assert_eq!(params.get("error").unwrap(), "invalid_token");
        assert_eq!(
            params.get("error_description").unwrap(),
            "Missing or invalid access token"
        );
    }

    #[test]
    fn test_parse_www_authenticate_with_resource_metadata() {
        let header = r#"Bearer realm="mcp", resource_metadata="https://mcp.example.com/.well-known/oauth-authorization-server""#;
        let params = parse_www_authenticate(header);
        assert_eq!(
            params.get("resource_metadata").unwrap(),
            "https://mcp.example.com/.well-known/oauth-authorization-server"
        );
    }

    #[test]
    fn test_parse_www_authenticate_no_bearer_prefix() {
        let header = r#"realm="test", error="unauthorized""#;
        let params = parse_www_authenticate(header);
        assert_eq!(params.get("realm").unwrap(), "test");
    }

    #[test]
    fn test_extract_metadata_url_from_resource_metadata() {
        let mut params = HashMap::new();
        params.insert(
            "resource_metadata".into(),
            "https://mcp.notion.com/.well-known/oauth-authorization-server".into(),
        );
        assert_eq!(
            extract_metadata_url(&params).unwrap(),
            "https://mcp.notion.com/.well-known/oauth-authorization-server"
        );
    }

    #[test]
    fn test_extract_metadata_url_none_when_missing() {
        let params = HashMap::new();
        assert!(extract_metadata_url(&params).is_none());
    }

    #[test]
    fn test_extract_metadata_url_ignores_non_url() {
        let mut params = HashMap::new();
        params.insert("resource_metadata".into(), "not-a-url".into());
        assert!(extract_metadata_url(&params).is_none());
    }

    #[test]
    fn test_well_known_url() {
        assert_eq!(
            well_known_url("https://mcp.notion.com/mcp").unwrap(),
            "https://mcp.notion.com/.well-known/oauth-authorization-server"
        );
    }

    #[test]
    fn test_well_known_url_with_port() {
        assert_eq!(
            well_known_url("https://localhost:8080/mcp").unwrap(),
            "https://localhost:8080/.well-known/oauth-authorization-server"
        );
    }

    #[test]
    fn test_well_known_url_invalid() {
        assert!(well_known_url("not-a-url").is_none());
    }
}
```

- [ ] **Step 2: Register the module**

In `crates/librefang-runtime/src/lib.rs`, add after line 38 (`pub mod mcp;`):

```rust
pub mod mcp_oauth;
```

- [ ] **Step 3: Run tests to verify they pass**

Run: `cargo test --lib -p librefang-runtime mcp_oauth`
Expected: All 8 tests pass.

- [ ] **Step 4: Commit**

```bash
git add crates/librefang-runtime/src/mcp_oauth.rs crates/librefang-runtime/src/lib.rs
git commit -m "feat(runtime): add mcp_oauth module with WWW-Authenticate parser and core types"
```

---

## Task 3: Runtime — `McpOAuthProvider` Trait and PKCE Helpers

**Files:**
- Modify: `crates/librefang-runtime/src/mcp_oauth.rs`

- [ ] **Step 1: Write tests for PKCE generation**

Add to the test module in `mcp_oauth.rs`:

```rust
    #[test]
    fn test_generate_pkce_verifier_length() {
        let (verifier, challenge) = generate_pkce();
        // Verifier is 43 chars (32 bytes base64url-encoded, no padding)
        assert!(verifier.len() >= 43);
        // Challenge is base64url-encoded SHA256 (43 chars)
        assert_eq!(challenge.len(), 43);
    }

    #[test]
    fn test_generate_pkce_different_each_time() {
        let (v1, _) = generate_pkce();
        let (v2, _) = generate_pkce();
        assert_ne!(v1, v2);
    }

    #[test]
    fn test_generate_state_length() {
        let state = generate_state();
        // 16 bytes → 22 chars base64url (no padding)
        assert!(state.len() >= 22);
    }

    #[test]
    fn test_merge_metadata_config_overrides_discovery() {
        let discovered = OAuthMetadata {
            authorization_endpoint: "https://discovered.com/auth".into(),
            token_endpoint: "https://discovered.com/token".into(),
            client_id: Some("discovered-id".into()),
            scopes: vec!["read".into()],
            server_url: "https://server.com/mcp".into(),
        };
        let config = librefang_types::config::McpOAuthConfig {
            auth_url: None,
            token_url: None,
            client_id: Some("override-id".into()),
            scopes: vec!["read".into(), "write".into()],
        };
        let merged = merge_metadata_with_config(discovered, &config);
        // Config client_id overrides discovered
        assert_eq!(merged.client_id.unwrap(), "override-id");
        // Config scopes override discovered
        assert_eq!(merged.scopes, vec!["read", "write"]);
        // Discovered endpoints preserved when config has None
        assert_eq!(merged.authorization_endpoint, "https://discovered.com/auth");
        assert_eq!(merged.token_endpoint, "https://discovered.com/token");
    }

    #[test]
    fn test_merge_metadata_config_overrides_endpoints() {
        let discovered = OAuthMetadata {
            authorization_endpoint: "https://discovered.com/auth".into(),
            token_endpoint: "https://discovered.com/token".into(),
            client_id: None,
            scopes: vec![],
            server_url: "https://server.com/mcp".into(),
        };
        let config = librefang_types::config::McpOAuthConfig {
            auth_url: Some("https://override.com/auth".into()),
            token_url: Some("https://override.com/token".into()),
            client_id: None,
            scopes: vec![],
        };
        let merged = merge_metadata_with_config(discovered, &config);
        assert_eq!(merged.authorization_endpoint, "https://override.com/auth");
        assert_eq!(merged.token_endpoint, "https://override.com/token");
    }
```

- [ ] **Step 2: Implement PKCE generation and metadata merge**

Add before the `#[cfg(test)]` module in `mcp_oauth.rs`:

```rust
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use sha2::{Digest, Sha256};

/// Generate PKCE code verifier and challenge (S256).
///
/// Returns `(verifier, challenge)` where challenge is `BASE64URL(SHA256(verifier))`.
pub fn generate_pkce() -> (String, String) {
    let mut bytes = [0u8; 32];
    rand::fill(&mut bytes);
    let verifier = URL_SAFE_NO_PAD.encode(bytes);

    let mut hasher = Sha256::new();
    hasher.update(verifier.as_bytes());
    let digest = hasher.finalize();
    let challenge = URL_SAFE_NO_PAD.encode(digest);

    (verifier, challenge)
}

/// Generate a random state parameter for CSRF protection.
pub fn generate_state() -> String {
    let mut bytes = [0u8; 16];
    rand::fill(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

/// Merge discovered OAuth metadata with config.toml overrides.
///
/// Config values take precedence where both exist. This allows overriding
/// a discovered `client_id` while using discovered endpoints.
pub fn merge_metadata_with_config(
    discovered: OAuthMetadata,
    config: &librefang_types::config::McpOAuthConfig,
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
```

- [ ] **Step 3: Add the `McpOAuthProvider` trait**

Add after the `merge_metadata_with_config` function:

```rust
/// Handle returned by `start_auth_flow` — allows waiting for the user
/// to complete the browser-based authorization.
pub struct AuthFlowHandle {
    /// URL the user needs to open in their browser.
    pub auth_url: String,
    /// Receiver that resolves when the user completes auth (or it times out).
    pub completion: tokio::sync::oneshot::Receiver<Result<OAuthTokens, String>>,
}

/// Trait for OAuth token management — implemented by kernel using extensions.
///
/// Follows the `KernelHandle` pattern: runtime defines the interface,
/// kernel provides the implementation. This avoids runtime depending on
/// the extensions crate (which would create a circular dependency).
#[async_trait::async_trait]
pub trait McpOAuthProvider: Send + Sync {
    /// Load a cached access token for this server URL.
    ///
    /// Returns `Some(token)` if a valid (non-expired) token is cached.
    /// If the cached token is near expiry and a refresh token exists,
    /// the implementation should refresh transparently and return the new token.
    /// Returns `None` if no token is cached or refresh failed.
    async fn load_token(&self, server_url: &str) -> Option<String>;

    /// Store OAuth tokens in the vault, keyed by server URL.
    async fn store_tokens(&self, server_url: &str, tokens: OAuthTokens) -> Result<(), String>;

    /// Clear cached tokens for a server URL (used by revoke).
    async fn clear_tokens(&self, server_url: &str) -> Result<(), String>;

    /// Start the PKCE authorization flow.
    ///
    /// The implementation should:
    /// 1. Generate PKCE verifier/challenge (use `generate_pkce()`)
    /// 2. Build the authorization URL
    /// 3. Set up a localhost callback server to receive the code
    /// 4. Optionally attempt to open the URL in a browser
    /// 5. Return an `AuthFlowHandle` with the URL and a completion channel
    ///
    /// The caller is responsible for logging the URL and tracking state.
    async fn start_auth_flow(
        &self,
        server_url: &str,
        metadata: OAuthMetadata,
    ) -> Result<AuthFlowHandle, String>;
}
```

- [ ] **Step 4: Run tests**

Run: `cargo test --lib -p librefang-runtime mcp_oauth`
Expected: All 13 tests pass (8 existing + 5 new).

- [ ] **Step 5: Commit**

```bash
git add crates/librefang-runtime/src/mcp_oauth.rs
git commit -m "feat(runtime): add McpOAuthProvider trait, PKCE generation, and metadata merge"
```

---

## Task 4: Runtime — `.well-known` Metadata Fetcher

**Files:**
- Modify: `crates/librefang-runtime/src/mcp_oauth.rs`

- [ ] **Step 1: Write test for metadata parsing**

Add to the test module:

```rust
    #[test]
    fn test_parse_authorization_server_metadata() {
        let json = serde_json::json!({
            "issuer": "https://mcp.notion.com",
            "authorization_endpoint": "https://mcp.notion.com/oauth/authorize",
            "token_endpoint": "https://mcp.notion.com/oauth/token",
            "response_types_supported": ["code"],
            "code_challenge_methods_supported": ["S256"],
            "registration_endpoint": "https://mcp.notion.com/oauth/register"
        });
        let meta = parse_authorization_server_metadata(
            &json.to_string(),
            "https://mcp.notion.com/mcp",
        )
        .unwrap();
        assert_eq!(
            meta.authorization_endpoint,
            "https://mcp.notion.com/oauth/authorize"
        );
        assert_eq!(
            meta.token_endpoint,
            "https://mcp.notion.com/oauth/token"
        );
        assert_eq!(meta.server_url, "https://mcp.notion.com/mcp");
    }

    #[test]
    fn test_parse_authorization_server_metadata_missing_fields() {
        let json = serde_json::json!({
            "issuer": "https://mcp.notion.com",
        });
        let result = parse_authorization_server_metadata(
            &json.to_string(),
            "https://mcp.notion.com/mcp",
        );
        assert!(result.is_err());
    }
```

- [ ] **Step 2: Implement metadata parsing and fetching**

Add before the trait definition in `mcp_oauth.rs`:

```rust
/// Raw OAuth Authorization Server Metadata (RFC 8414) response.
#[derive(Debug, Deserialize)]
struct AuthorizationServerMetadata {
    authorization_endpoint: String,
    token_endpoint: String,
    #[serde(default)]
    registration_endpoint: Option<String>,
    #[serde(default)]
    code_challenge_methods_supported: Vec<String>,
}

/// Parse an OAuth Authorization Server Metadata JSON response into `OAuthMetadata`.
pub fn parse_authorization_server_metadata(
    body: &str,
    server_url: &str,
) -> Result<OAuthMetadata, String> {
    let raw: AuthorizationServerMetadata =
        serde_json::from_str(body).map_err(|e| format!("Failed to parse OAuth metadata: {e}"))?;

    Ok(OAuthMetadata {
        authorization_endpoint: raw.authorization_endpoint,
        token_endpoint: raw.token_endpoint,
        client_id: None,
        scopes: Vec::new(),
        server_url: server_url.to_string(),
    })
}

/// Fetch OAuth metadata for an MCP server using the three-tier discovery strategy.
///
/// 1. Try `resource_metadata` URL from the `WWW-Authenticate` header
/// 2. Try `.well-known/oauth-authorization-server` at the server's origin
/// 3. Fall back to explicit config (if provided)
///
/// Returns `Err` if all tiers fail and no config fallback is available.
pub async fn discover_oauth_metadata(
    server_url: &str,
    www_authenticate: Option<&str>,
    config: Option<&librefang_types::config::McpOAuthConfig>,
) -> Result<OAuthMetadata, String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| format!("HTTP client error: {e}"))?;

    // Tier 1: resource_metadata from WWW-Authenticate header
    if let Some(header) = www_authenticate {
        let params = parse_www_authenticate(header);
        if let Some(metadata_url) = extract_metadata_url(&params) {
            tracing::debug!(url = %metadata_url, "Fetching OAuth metadata from WWW-Authenticate resource_metadata");
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
                                tracing::warn!(error = %e, "Failed to parse resource_metadata response");
                            }
                        }
                    }
                }
                Ok(resp) => {
                    tracing::warn!(status = %resp.status(), "resource_metadata URL returned non-success");
                }
                Err(e) => {
                    tracing::warn!(error = %e, "Failed to fetch resource_metadata URL");
                }
            }
        }
    }

    // Tier 2: .well-known/oauth-authorization-server
    if let Some(wk_url) = well_known_url(server_url) {
        tracing::debug!(url = %wk_url, "Fetching OAuth metadata from .well-known");
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
                            tracing::warn!(error = %e, "Failed to parse .well-known response");
                        }
                    }
                }
            }
            Ok(resp) => {
                tracing::debug!(status = %resp.status(), ".well-known not available");
            }
            Err(e) => {
                tracing::debug!(error = %e, ".well-known fetch failed");
            }
        }
    }

    // Tier 3: config.toml fallback
    if let Some(cfg) = config {
        if let (Some(auth_url), Some(token_url)) = (&cfg.auth_url, &cfg.token_url) {
            tracing::debug!("Using OAuth config from config.toml");
            return Ok(OAuthMetadata {
                authorization_endpoint: auth_url.clone(),
                token_endpoint: token_url.clone(),
                client_id: cfg.client_id.clone(),
                scopes: cfg.scopes.clone(),
                server_url: server_url.to_string(),
            });
        }
    }

    Err(format!(
        "MCP server at {server_url} requires authentication but no OAuth metadata could be \
         discovered. Configure [mcp_servers.oauth] in config.toml with auth_url and token_url."
    ))
}
```

- [ ] **Step 3: Run tests**

Run: `cargo test --lib -p librefang-runtime mcp_oauth`
Expected: All 15 tests pass.

- [ ] **Step 4: Commit**

```bash
git add crates/librefang-runtime/src/mcp_oauth.rs
git commit -m "feat(runtime): add OAuth metadata discovery with three-tier resolution"
```

---

## Task 5: Runtime — Wire OAuth into `McpConnection`

**Files:**
- Modify: `crates/librefang-runtime/src/mcp.rs:26-49` (McpServerConfig)
- Modify: `crates/librefang-runtime/src/mcp.rs:93-101` (McpConnection)
- Modify: `crates/librefang-runtime/src/mcp.rs:215` (connect)
- Modify: `crates/librefang-runtime/src/mcp.rs:413` (connect_streamable_http)

- [ ] **Step 1: Add OAuth fields to `McpServerConfig`**

In `crates/librefang-runtime/src/mcp.rs`, add to the `McpServerConfig` struct (after `headers` field, line ~49):

```rust
    /// Optional OAuth provider for automatic authentication.
    /// If set and the server returns 401, the provider handles token
    /// acquisition and retry transparently.
    #[serde(skip)]
    pub oauth_provider: Option<std::sync::Arc<dyn crate::mcp_oauth::McpOAuthProvider>>,

    /// Optional OAuth config from config.toml (discovery fallback).
    #[serde(default)]
    pub oauth_config: Option<librefang_types::config::McpOAuthConfig>,
```

- [ ] **Step 2: Add auth state to `McpConnection`**

Add a field to `McpConnection` (after `inner` at line ~101):

```rust
    /// Authentication state for this connection.
    auth_state: crate::mcp_oauth::McpAuthState,
```

And add a public accessor:

```rust
    /// Get the authentication state.
    pub fn auth_state(&self) -> &crate::mcp_oauth::McpAuthState {
        &self.auth_state
    }
```

- [ ] **Step 3: Update `connect_streamable_http` to accept OAuth params**

Change the signature at line ~413 from:

```rust
    async fn connect_streamable_http(
        url: &str,
        headers: &[String],
    ) -> Result<(McpInner, Option<Vec<rmcp::model::Tool>>), String> {
```

To:

```rust
    async fn connect_streamable_http(
        url: &str,
        headers: &[String],
        oauth_provider: Option<&std::sync::Arc<dyn crate::mcp_oauth::McpOAuthProvider>>,
        oauth_config: Option<&librefang_types::config::McpOAuthConfig>,
    ) -> Result<(McpInner, Option<Vec<rmcp::model::Tool>>, crate::mcp_oauth::McpAuthState), String> {
```

- [ ] **Step 4: Implement 401 detection and OAuth retry in `connect_streamable_http`**

Replace the body of `connect_streamable_http` with:

```rust
    {
        use crate::mcp_oauth::{self, McpAuthState};
        use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
        use rmcp::transport::StreamableHttpClientTransport;
        use rmcp::ServiceExt;

        Self::check_ssrf(url, "Streamable HTTP")?;

        // Parse custom headers
        let mut custom_headers: HashMap<HeaderName, HeaderValue> = HashMap::new();
        for header_str in headers {
            if let Some((name, value)) = header_str.split_once(':') {
                let name = name.trim();
                let value = value.trim();
                if let (Ok(hn), Ok(hv)) = (
                    HeaderName::from_bytes(name.as_bytes()),
                    HeaderValue::from_str(value),
                ) {
                    custom_headers.insert(hn, hv);
                }
            }
        }

        // Try loading cached token from vault
        if let Some(provider) = oauth_provider {
            if let Some(token) = provider.load_token(url).await {
                tracing::debug!(server = %url, "Using cached OAuth token");
                custom_headers.insert(
                    HeaderName::from_static("authorization"),
                    HeaderValue::from_str(&format!("Bearer {token}"))
                        .map_err(|e| format!("Invalid token header value: {e}"))?,
                );
            }
        }

        // Attempt connection
        let mut config = StreamableHttpClientTransportConfig::default();
        config.uri = Arc::from(url);
        config.custom_headers = custom_headers.clone();

        let transport = StreamableHttpClientTransport::from_config(config);

        match ().into_dyn().serve(transport).await {
            Ok(client) => {
                // Connected — discover tools
                let timeout = std::time::Duration::from_secs(60);
                let tools = tokio::time::timeout(timeout, client.list_all_tools())
                    .await
                    .map_err(|_| "MCP tools/list timed out after 60s for Streamable HTTP".to_string())?
                    .map_err(|e| format!("MCP tools/list failed: {e}"))?;

                // Determine auth state based on whether we injected a token
                let auth_state = if custom_headers.contains_key("authorization") {
                    McpAuthState::Authorized { expires_at: None }
                } else {
                    McpAuthState::NotRequired
                };

                Ok((McpInner::Rmcp(client), Some(tools), auth_state))
            }
            Err(e) => {
                let err_str = format!("{e}");

                // Check if this is an auth error
                if err_str.contains("Auth required")
                    || err_str.contains("401")
                    || err_str.contains("Unauthorized")
                {
                    // Try to extract WWW-Authenticate header from the error
                    let www_auth = Self::extract_www_authenticate(&err_str);

                    tracing::info!(
                        server = %url,
                        "MCP server requires OAuth authentication, discovering metadata"
                    );

                    // Discover OAuth metadata
                    let metadata = mcp_oauth::discover_oauth_metadata(
                        url,
                        www_auth.as_deref(),
                        oauth_config,
                    )
                    .await?;

                    // If we have a provider, start the auth flow
                    if let Some(provider) = oauth_provider {
                        let handle = provider.start_auth_flow(url, metadata).await?;

                        tracing::warn!(
                            server = %url,
                            auth_url = %handle.auth_url,
                            "MCP server requires authorization. Open this URL in your browser."
                        );

                        // Return pending state — the connection will be retried
                        // after the user completes auth
                        return Err(format!(
                            "OAUTH_PENDING:{}",
                            handle.auth_url
                        ));
                    }

                    Err(format!("MCP Streamable HTTP connection failed (auth required): {e}"))
                } else {
                    Err(format!("MCP Streamable HTTP connection failed: {e}"))
                }
            }
        }
    }
```

- [ ] **Step 5: Add helper to extract WWW-Authenticate from error string**

Add as a method on `McpConnection`:

```rust
    /// Try to extract a `WWW-Authenticate` header value from an rmcp error message.
    ///
    /// The rmcp SDK formats `AuthRequired` errors as:
    /// `AuthRequired(AuthRequiredError { www_authenticate_header: "Bearer ..." })`
    fn extract_www_authenticate(error: &str) -> Option<String> {
        // Look for the www_authenticate_header field in the error string
        let marker = "www_authenticate_header: \"";
        let start = error.find(marker)? + marker.len();
        let rest = &error[start..];
        let end = rest.find('"')?;
        Some(rest[..end].to_string())
    }
```

- [ ] **Step 6: Update `connect()` to pass OAuth params through**

Update the `McpTransport::Http` arm in `connect()` (line ~221) from:

```rust
            McpTransport::Http { url } => {
                Self::connect_streamable_http(url, &config.headers).await?
            }
```

To:

```rust
            McpTransport::Http { url } => {
                let (inner, tools, auth_state) = Self::connect_streamable_http(
                    url,
                    &config.headers,
                    config.oauth_provider.as_ref(),
                    config.oauth_config.as_ref(),
                )
                .await?;
                initial_auth_state = Some(auth_state);
                (inner, tools)
            }
```

Add `let mut initial_auth_state: Option<crate::mcp_oauth::McpAuthState> = None;` before the match, and set `auth_state` on the connection struct:

```rust
        let mut conn = Self {
            config,
            tools: Vec::new(),
            original_names: HashMap::new(),
            inner,
            auth_state: initial_auth_state.unwrap_or(crate::mcp_oauth::McpAuthState::NotRequired),
        };
```

- [ ] **Step 7: Run build**

Run: `cargo build --workspace --lib`
Expected: Success. Existing tests should still pass since `oauth_provider` defaults to `None`.

- [ ] **Step 8: Run existing tests**

Run: `cargo test --workspace`
Expected: All tests pass. Existing MCP connection code paths are unaffected when `oauth_provider` is `None`.

- [ ] **Step 9: Commit**

```bash
git add crates/librefang-runtime/src/mcp.rs
git commit -m "feat(runtime): wire OAuth provider into MCP Streamable HTTP connect with 401 retry"
```

---

## Task 6: Kernel — `KernelOAuthProvider` Implementation

**Files:**
- Create: `crates/librefang-kernel/src/mcp_oauth_provider.rs`
- Modify: `crates/librefang-kernel/src/kernel.rs:8854-8926`

- [ ] **Step 1: Create the provider implementation**

Create `crates/librefang-kernel/src/mcp_oauth_provider.rs`:

```rust
//! Kernel-side implementation of `McpOAuthProvider`.
//!
//! Bridges the runtime's OAuth trait to the extensions crate's vault
//! and PKCE flow. This avoids runtime depending on extensions directly.

use librefang_extensions::oauth::OAuthTokens as ExtOAuthTokens;
use librefang_extensions::vault::Vault;
use librefang_runtime::mcp_oauth::{
    AuthFlowHandle, McpOAuthProvider, OAuthMetadata, OAuthTokens,
};
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::{debug, info, warn};
use zeroize::Zeroizing;

/// Vault key prefix for MCP OAuth tokens.
const VAULT_PREFIX: &str = "mcp_oauth";

fn vault_key(server_url: &str, field: &str) -> String {
    format!("{VAULT_PREFIX}:{server_url}:{field}")
}

/// Kernel's implementation of `McpOAuthProvider` using the extensions vault
/// for token storage and the extensions PKCE flow for browser auth.
pub struct KernelOAuthProvider {
    vault: Arc<Mutex<Option<Vault>>>,
}

impl KernelOAuthProvider {
    pub fn new(vault: Arc<Mutex<Option<Vault>>>) -> Self {
        Self { vault }
    }
}

#[async_trait::async_trait]
impl McpOAuthProvider for KernelOAuthProvider {
    async fn load_token(&self, server_url: &str) -> Option<String> {
        let mut guard = self.vault.lock().await;
        let vault = guard.as_mut()?;

        let token = vault.get(&vault_key(server_url, "access_token"))?;

        // Check expiry
        if let Some(expires_at_str) = vault.get(&vault_key(server_url, "expires_at")) {
            if let Ok(expires_at) = expires_at_str.parse::<i64>() {
                let now = chrono::Utc::now().timestamp();
                if now >= expires_at - 60 {
                    // Token expired or near expiry — try refresh
                    debug!(server = %server_url, "OAuth token near expiry, attempting refresh");
                    if let Some(refresh_token) =
                        vault.get(&vault_key(server_url, "refresh_token"))
                    {
                        if let Some(metadata_json) =
                            vault.get(&vault_key(server_url, "metadata"))
                        {
                            if let Ok(metadata) =
                                serde_json::from_str::<OAuthMetadata>(&metadata_json)
                            {
                                drop(guard); // Release lock before HTTP call
                                return self
                                    .refresh_token(server_url, &refresh_token, &metadata)
                                    .await;
                            }
                        }
                    }
                    debug!(server = %server_url, "No refresh token or metadata, token expired");
                    return None;
                }
            }
        }

        // No expiry info (e.g. Notion) or token still valid
        Some(token.to_string())
    }

    async fn store_tokens(&self, server_url: &str, tokens: OAuthTokens) -> Result<(), String> {
        let mut guard = self.vault.lock().await;
        let vault = guard
            .as_mut()
            .ok_or("Vault not available — tokens will not persist across restarts")?;

        vault
            .set(
                vault_key(server_url, "access_token"),
                Zeroizing::new(tokens.access_token.clone()),
            )
            .map_err(|e| format!("Failed to store access token: {e}"))?;

        if let Some(ref refresh) = tokens.refresh_token {
            vault
                .set(
                    vault_key(server_url, "refresh_token"),
                    Zeroizing::new(refresh.clone()),
                )
                .map_err(|e| format!("Failed to store refresh token: {e}"))?;
        }

        if tokens.expires_in > 0 {
            let expires_at = chrono::Utc::now().timestamp() + tokens.expires_in as i64;
            vault
                .set(
                    vault_key(server_url, "expires_at"),
                    Zeroizing::new(expires_at.to_string()),
                )
                .map_err(|e| format!("Failed to store expiry: {e}"))?;
        }

        info!(server = %server_url, "OAuth tokens stored in vault");
        Ok(())
    }

    async fn clear_tokens(&self, server_url: &str) -> Result<(), String> {
        let mut guard = self.vault.lock().await;
        let vault = guard
            .as_mut()
            .ok_or("Vault not available")?;

        for field in &["access_token", "refresh_token", "expires_at", "metadata"] {
            let _ = vault.remove(&vault_key(server_url, field));
        }

        info!(server = %server_url, "OAuth tokens cleared from vault");
        Ok(())
    }

    async fn start_auth_flow(
        &self,
        server_url: &str,
        metadata: OAuthMetadata,
    ) -> Result<AuthFlowHandle, String> {
        let (verifier, challenge) =
            librefang_runtime::mcp_oauth::generate_pkce();
        let state = librefang_runtime::mcp_oauth::generate_state();

        let client_id = metadata
            .client_id
            .as_deref()
            .unwrap_or("librefang");

        let scopes_str = if metadata.scopes.is_empty() {
            String::new()
        } else {
            metadata.scopes.join(" ")
        };

        // Build authorization URL
        let mut auth_url = url::Url::parse(&metadata.authorization_endpoint)
            .map_err(|e| format!("Invalid auth URL: {e}"))?;
        auth_url
            .query_pairs_mut()
            .append_pair("response_type", "code")
            .append_pair("client_id", client_id)
            .append_pair("code_challenge", &challenge)
            .append_pair("code_challenge_method", "S256")
            .append_pair("state", &state);

        if !scopes_str.is_empty() {
            auth_url
                .query_pairs_mut()
                .append_pair("scope", &scopes_str);
        }

        // Set up callback — bind localhost listener
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .map_err(|e| format!("Failed to bind callback listener: {e}"))?;
        let callback_port = listener
            .local_addr()
            .map_err(|e| format!("Failed to get listener addr: {e}"))?
            .port();

        let redirect_uri = format!("http://127.0.0.1:{callback_port}/callback");
        auth_url
            .query_pairs_mut()
            .append_pair("redirect_uri", &redirect_uri);

        let auth_url_str = auth_url.to_string();

        // Store metadata in vault for later refresh
        {
            let mut guard = self.vault.lock().await;
            if let Some(vault) = guard.as_mut() {
                let metadata_json = serde_json::to_string(&metadata)
                    .unwrap_or_default();
                let _ = vault.set(
                    vault_key(server_url, "metadata"),
                    Zeroizing::new(metadata_json),
                );
            }
        }

        // Try opening in browser
        let _ = open_browser(&auth_url_str);

        let (tx, rx) = tokio::sync::oneshot::channel();
        let token_endpoint = metadata.token_endpoint.clone();
        let client_id_owned = client_id.to_string();

        // Spawn callback handler
        tokio::spawn(async move {
            let timeout = std::time::Duration::from_secs(300); // 5 minute timeout
            match tokio::time::timeout(timeout, accept_oauth_callback(
                listener,
                &state,
                &token_endpoint,
                &client_id_owned,
                &verifier,
                &redirect_uri,
            ))
            .await
            {
                Ok(Ok(tokens)) => {
                    let _ = tx.send(Ok(tokens));
                }
                Ok(Err(e)) => {
                    let _ = tx.send(Err(e));
                }
                Err(_) => {
                    let _ = tx.send(Err("OAuth callback timed out after 5 minutes".into()));
                }
            }
        });

        Ok(AuthFlowHandle {
            auth_url: auth_url_str,
            completion: rx,
        })
    }
}

impl KernelOAuthProvider {
    /// Refresh an expired token using the refresh_token grant.
    async fn refresh_token(
        &self,
        server_url: &str,
        refresh_token: &str,
        metadata: &OAuthMetadata,
    ) -> Option<String> {
        let client = reqwest::Client::new();
        let resp = client
            .post(&metadata.token_endpoint)
            .form(&[
                ("grant_type", "refresh_token"),
                ("refresh_token", refresh_token),
            ])
            .send()
            .await
            .ok()?;

        if !resp.status().is_success() {
            warn!(
                server = %server_url,
                status = %resp.status(),
                "Token refresh failed"
            );
            return None;
        }

        let tokens: OAuthTokens = resp.json().await.ok()?;
        let new_token = tokens.access_token.clone();

        if let Err(e) = self.store_tokens(server_url, tokens).await {
            warn!(error = %e, "Failed to store refreshed tokens");
        }

        Some(new_token)
    }
}

/// Accept the OAuth callback on the localhost listener.
async fn accept_oauth_callback(
    listener: tokio::net::TcpListener,
    expected_state: &str,
    token_endpoint: &str,
    client_id: &str,
    verifier: &str,
    redirect_uri: &str,
) -> Result<OAuthTokens, String> {
    // Accept one HTTP connection
    let (stream, _) = listener
        .accept()
        .await
        .map_err(|e| format!("Callback accept failed: {e}"))?;

    // Read the HTTP request
    let mut buf = vec![0u8; 4096];
    let n = tokio::io::AsyncReadExt::read(&mut tokio::io::BufReader::new(&stream), &mut buf)
        .await
        .map_err(|e| format!("Callback read failed: {e}"))?;

    let request = String::from_utf8_lossy(&buf[..n]);

    // Parse the request line to get the path + query
    let first_line = request.lines().next().unwrap_or("");
    let path = first_line.split_whitespace().nth(1).unwrap_or("/");

    let parsed = url::Url::parse(&format!("http://localhost{path}"))
        .map_err(|e| format!("Failed to parse callback URL: {e}"))?;

    let params: std::collections::HashMap<_, _> = parsed.query_pairs().collect();

    // Check for error
    if let Some(error) = params.get("error") {
        let desc = params.get("error_description").map(|s| s.to_string()).unwrap_or_default();
        // Send error response
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\n\r\n<html><body><h2>Authorization Failed</h2><p>{}: {}</p></body></html>",
            error, desc
        );
        let _ = tokio::io::AsyncWriteExt::write_all(&mut &stream, response.as_bytes()).await;
        return Err(format!("OAuth error: {error} — {desc}"));
    }

    // Validate state
    let state = params
        .get("state")
        .ok_or("Missing state parameter in callback")?;
    if state.as_ref() != expected_state {
        let response = "HTTP/1.1 400 Bad Request\r\nContent-Type: text/html\r\n\r\n<html><body><h2>Invalid State</h2></body></html>";
        let _ = tokio::io::AsyncWriteExt::write_all(&mut &stream, response.as_bytes()).await;
        return Err("OAuth state mismatch (possible CSRF)".into());
    }

    // Get authorization code
    let code = params
        .get("code")
        .ok_or("Missing code parameter in callback")?;

    // Exchange code for tokens
    let client = reqwest::Client::new();
    let resp = client
        .post(token_endpoint)
        .form(&[
            ("grant_type", "authorization_code"),
            ("code", code.as_ref()),
            ("redirect_uri", redirect_uri),
            ("client_id", client_id),
            ("code_verifier", verifier),
        ])
        .send()
        .await
        .map_err(|e| format!("Token exchange failed: {e}"))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("Token exchange returned {status}: {body}"));
    }

    let tokens: OAuthTokens = resp
        .json()
        .await
        .map_err(|e| format!("Failed to parse token response: {e}"))?;

    // Send success response to browser
    let response = "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\n\r\n<html><body><h2>Authorization Complete</h2><p>You can close this tab.</p><script>window.close()</script></body></html>";
    let _ = tokio::io::AsyncWriteExt::write_all(&mut &stream, response.as_bytes()).await;

    Ok(tokens)
}

/// Try to open a URL in the default browser.
fn open_browser(url: &str) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    let result = std::process::Command::new("open").arg(url).spawn();
    #[cfg(target_os = "linux")]
    let result = std::process::Command::new("xdg-open").arg(url).spawn();
    #[cfg(target_os = "windows")]
    let result = std::process::Command::new("cmd").args(["/C", "start", url]).spawn();
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    let result: Result<std::process::Child, std::io::Error> = Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "unsupported platform",
    ));

    result.map(|_| ()).map_err(|e| format!("Failed to open browser: {e}"))
}
```

- [ ] **Step 2: Register the module in kernel**

Find the module declarations in `crates/librefang-kernel/src/kernel.rs` (or `lib.rs` if modules are declared there) and add:

```rust
pub mod mcp_oauth_provider;
```

- [ ] **Step 3: Update `connect_mcp_servers` to pass the provider**

In `crates/librefang-kernel/src/kernel.rs`, around line 8854 in `connect_mcp_servers`, after building `McpServerConfig` (line ~8890-8895), add the OAuth fields:

```rust
            let mcp_config = McpServerConfig {
                name: server_config.name.clone(),
                transport,
                timeout_secs: server_config.timeout_secs,
                env: server_config.env.clone(),
                headers: server_config.headers.clone(),
                oauth_provider: Some(Arc::clone(&oauth_provider)),
                oauth_config: server_config.oauth.clone(),
            };
```

And at the top of `connect_mcp_servers`, create the provider:

```rust
        // Create OAuth provider for MCP servers that need authentication
        let vault = self.vault.clone(); // Assumes kernel has an Arc<Mutex<Option<Vault>>>
        let oauth_provider: Arc<dyn librefang_runtime::mcp_oauth::McpOAuthProvider> =
            Arc::new(crate::mcp_oauth_provider::KernelOAuthProvider::new(vault));
```

- [ ] **Step 4: Handle `OAUTH_PENDING` errors in `connect_mcp_servers`**

Update the error handling in the connect loop (line ~8898-8926). After the existing `Err(e)` arm, add pending auth handling:

```rust
                Err(e) if e.starts_with("OAUTH_PENDING:") => {
                    let auth_url = e.strip_prefix("OAUTH_PENDING:").unwrap_or("").to_string();
                    warn!(
                        server = %server_config.name,
                        auth_url = %auth_url,
                        "MCP server requires authorization — open the URL in your browser"
                    );
                    // Track as pending — the auth completion callback will retry
                    self.mcp_auth_states.lock().await.insert(
                        server_config.name.clone(),
                        librefang_runtime::mcp_oauth::McpAuthState::PendingAuth { auth_url },
                    );
                }
```

This requires adding an `mcp_auth_states` field to the kernel struct:

```rust
    pub(crate) mcp_auth_states: tokio::sync::Mutex<
        HashMap<String, librefang_runtime::mcp_oauth::McpAuthState>,
    >,
```

Initialize it as `tokio::sync::Mutex::new(HashMap::new())` in the kernel constructor.

- [ ] **Step 5: Verify it compiles**

Run: `cargo build --workspace --lib`
Expected: Success.

- [ ] **Step 6: Commit**

```bash
git add crates/librefang-kernel/src/mcp_oauth_provider.rs crates/librefang-kernel/src/kernel.rs
git commit -m "feat(kernel): implement KernelOAuthProvider with vault storage and PKCE flow"
```

---

## Task 7: Kernel — Auth Completion Listener and Auto-Reconnect

**Files:**
- Modify: `crates/librefang-kernel/src/kernel.rs`

- [ ] **Step 1: Add auth completion watcher to `connect_mcp_servers`**

When a server enters `OAUTH_PENDING`, the `start_auth_flow` returns an `AuthFlowHandle` with a `completion` channel. We need to spawn a task that waits on it and retries the connection.

In the `OAUTH_PENDING` error handler from Task 6, before inserting the auth state, spawn the completion watcher. This requires restructuring slightly — the `start_auth_flow` call now happens in `connect_streamable_http`, so we need to capture the completion receiver.

Update the approach: instead of returning `OAUTH_PENDING` as an error string, change `connect_streamable_http` to return a richer error type. Add to `mcp_oauth.rs`:

```rust
/// Error from an MCP connection attempt that needed OAuth.
pub enum McpConnectError {
    /// Non-auth error.
    Failed(String),
    /// Server requires OAuth. Auth flow has been started.
    OAuthPending {
        auth_url: String,
        completion: tokio::sync::oneshot::Receiver<Result<OAuthTokens, String>>,
    },
}
```

Then in `connect_mcp_servers`, handle it:

```rust
                Err(e) if e.starts_with("OAUTH_PENDING:") => {
                    // ... (handled by the new richer error type above)
                }
```

Actually, since the error passes through several layers as `Result<_, String>`, the simplest approach that avoids changing the entire error chain is to handle the auth flow **in the kernel** rather than inside `connect_streamable_http`. 

Revised approach for the `connect_mcp_servers` OAUTH_PENDING handler:

```rust
                Err(e) if e.starts_with("OAUTH_PENDING:") => {
                    let auth_url = e.strip_prefix("OAUTH_PENDING:")
                        .unwrap_or("")
                        .to_string();

                    // Store pending state
                    self.mcp_auth_states.lock().await.insert(
                        server_config.name.clone(),
                        librefang_runtime::mcp_oauth::McpAuthState::PendingAuth {
                            auth_url: auth_url.clone(),
                        },
                    );

                    // Spawn watcher that retries after auth completes
                    let kernel = Arc::clone(self);
                    let server_name = server_config.name.clone();
                    let server_cfg = server_config.clone();
                    let provider = Arc::clone(&oauth_provider);
                    tokio::spawn(async move {
                        // Wait up to 5 minutes for auth callback
                        tokio::time::sleep(std::time::Duration::from_secs(300)).await;
                        // Check if token appeared in vault (set by the callback handler)
                        if let Some(token) = provider.load_token(
                            server_cfg.transport.as_ref()
                                .and_then(|t| match t {
                                    librefang_types::config::McpTransportEntry::Http { url } => Some(url.as_str()),
                                    _ => None,
                                })
                                .unwrap_or("")
                        ).await {
                            tracing::info!(server = %server_name, "OAuth token received, retrying MCP connection");
                            kernel.retry_mcp_connection(&server_name).await;
                        }
                    });
                }
```

- [ ] **Step 2: Add `retry_mcp_connection` method to kernel**

```rust
    /// Retry connecting a single MCP server (after OAuth completion).
    pub async fn retry_mcp_connection(self: &Arc<Self>, server_name: &str) {
        use librefang_runtime::mcp::{McpConnection, McpServerConfig, McpTransport};
        use librefang_types::config::McpTransportEntry;

        let servers = self
            .effective_mcp_servers
            .read()
            .map(|s| s.clone())
            .unwrap_or_default();

        let server_config = match servers.iter().find(|s| s.name == server_name) {
            Some(s) => s,
            None => return,
        };

        let transport_entry = match &server_config.transport {
            Some(t) => t,
            None => return,
        };

        let transport = match transport_entry {
            McpTransportEntry::Stdio { command, args } => McpTransport::Stdio {
                command: command.clone(),
                args: args.clone(),
            },
            McpTransportEntry::Sse { url } => McpTransport::Sse { url: url.clone() },
            McpTransportEntry::Http { url } => McpTransport::Http { url: url.clone() },
            McpTransportEntry::HttpCompat {
                base_url,
                headers,
                tools,
            } => McpTransport::HttpCompat {
                base_url: base_url.clone(),
                headers: headers.clone(),
                tools: tools.clone(),
            },
        };

        let vault = self.vault.clone();
        let oauth_provider: Arc<dyn librefang_runtime::mcp_oauth::McpOAuthProvider> =
            Arc::new(crate::mcp_oauth_provider::KernelOAuthProvider::new(vault));

        let mcp_config = McpServerConfig {
            name: server_config.name.clone(),
            transport,
            timeout_secs: server_config.timeout_secs,
            env: server_config.env.clone(),
            headers: server_config.headers.clone(),
            oauth_provider: Some(oauth_provider),
            oauth_config: server_config.oauth.clone(),
        };

        match McpConnection::connect(mcp_config).await {
            Ok(conn) => {
                let tool_count = conn.tools().len();
                if let Ok(mut tools) = self.mcp_tools.lock() {
                    tools.extend(conn.tools().iter().cloned());
                    self.mcp_generation
                        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                }
                tracing::info!(
                    server = %server_name,
                    tools = tool_count,
                    "MCP server connected after OAuth"
                );
                self.extension_health.report_ok(server_name, tool_count);
                self.mcp_connections.lock().await.push(conn);
                self.mcp_auth_states.lock().await.insert(
                    server_name.to_string(),
                    librefang_runtime::mcp_oauth::McpAuthState::Authorized { expires_at: None },
                );
            }
            Err(e) => {
                tracing::warn!(
                    server = %server_name,
                    error = %e,
                    "Failed to connect to MCP server after OAuth"
                );
            }
        }
    }
```

- [ ] **Step 3: Verify it compiles**

Run: `cargo build --workspace --lib`
Expected: Success.

- [ ] **Step 4: Commit**

```bash
git add crates/librefang-kernel/src/kernel.rs
git commit -m "feat(kernel): add OAuth completion watcher and auto-reconnect for MCP servers"
```

---

## Task 8: API — Auth Endpoints

**Files:**
- Create: `crates/librefang-api/src/routes/mcp_auth.rs`
- Modify: `crates/librefang-api/src/routes/skills.rs` (register routes, add auth_state to list)

- [ ] **Step 1: Create the auth route handlers**

Create `crates/librefang-api/src/routes/mcp_auth.rs`:

```rust
//! MCP OAuth authentication API endpoints.
//!
//! Provides routes for managing OAuth state on MCP server connections:
//! - `GET  /api/mcp/{name}/auth/status` — current auth state
//! - `POST /api/mcp/{name}/auth/start`  — initiate OAuth flow
//! - `GET  /api/mcp/{name}/auth/callback` — OAuth redirect callback
//! - `DELETE /api/mcp/{name}/auth/revoke` — clear tokens and disconnect

use crate::routes::ApiErrorResponse;
use crate::AppState;
use axum::extract::{Path, Query, State};
use axum::response::IntoResponse;
use axum::Json;
use std::sync::Arc;

/// GET /api/mcp/{name}/auth/status
pub async fn auth_status(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    // Check if server exists in config
    let cfg = state.kernel.config_snapshot();
    if !cfg.mcp_servers.iter().any(|s| s.name == name) {
        return ApiErrorResponse::not_found(format!("MCP server '{}' not found", name))
            .into_json_tuple();
    }

    let auth_states = state.kernel.mcp_auth_states_ref().lock().await;
    let auth_state = auth_states.get(&name);

    let state_json = match auth_state {
        Some(s) => serde_json::to_value(s).unwrap_or(serde_json::json!({"state": "unknown"})),
        None => {
            // Check if connected (no auth needed)
            let connections = state.kernel.mcp_connections_ref().lock().await;
            if connections.iter().any(|c| c.name() == name) {
                serde_json::json!({"state": "not_required"})
            } else {
                serde_json::json!({"state": "not_required"})
            }
        }
    };

    (
        axum::http::StatusCode::OK,
        Json(serde_json::json!({
            "server": name,
            "auth": state_json,
        })),
    )
}

/// POST /api/mcp/{name}/auth/start
pub async fn auth_start(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    // Find the server config
    let cfg = state.kernel.config_snapshot();
    let server = match cfg.mcp_servers.iter().find(|s| s.name == name) {
        Some(s) => s.clone(),
        None => {
            return ApiErrorResponse::not_found(format!("MCP server '{}' not found", name))
                .into_json_tuple();
        }
    };

    // Get the server URL from transport
    let server_url = match &server.transport {
        Some(librefang_types::config::McpTransportEntry::Http { url }) => url.clone(),
        _ => {
            return ApiErrorResponse::bad_request(
                "OAuth is only supported for HTTP (Streamable) transport".into(),
            )
            .into_json_tuple();
        }
    };

    // Discover metadata and start auth flow
    let metadata = match librefang_runtime::mcp_oauth::discover_oauth_metadata(
        &server_url,
        None,
        server.oauth.as_ref(),
    )
    .await
    {
        Ok(m) => m,
        Err(e) => {
            return ApiErrorResponse::internal(format!("OAuth discovery failed: {e}"))
                .into_json_tuple();
        }
    };

    let oauth_provider = state.kernel.oauth_provider_ref();
    match oauth_provider.start_auth_flow(&server_url, metadata).await {
        Ok(handle) => {
            let auth_url = handle.auth_url.clone();

            // Track pending state
            state.kernel.mcp_auth_states_ref().lock().await.insert(
                name.clone(),
                librefang_runtime::mcp_oauth::McpAuthState::PendingAuth {
                    auth_url: auth_url.clone(),
                },
            );

            // Spawn completion watcher
            let kernel = Arc::clone(&state.kernel);
            let server_name = name.clone();
            let provider = Arc::clone(&oauth_provider);
            let url = server_url.clone();
            tokio::spawn(async move {
                match handle.completion.await {
                    Ok(Ok(tokens)) => {
                        if let Err(e) = provider.store_tokens(&url, tokens).await {
                            tracing::warn!(error = %e, "Failed to store OAuth tokens");
                        }
                        kernel.retry_mcp_connection(&server_name).await;
                    }
                    Ok(Err(e)) => {
                        tracing::warn!(server = %server_name, error = %e, "OAuth flow failed");
                        kernel.mcp_auth_states_ref().lock().await.insert(
                            server_name,
                            librefang_runtime::mcp_oauth::McpAuthState::Expired,
                        );
                    }
                    Err(_) => {
                        tracing::warn!(server = %server_name, "OAuth completion channel dropped");
                    }
                }
            });

            (
                axum::http::StatusCode::OK,
                Json(serde_json::json!({
                    "auth_url": auth_url,
                    "server": name,
                })),
            )
        }
        Err(e) => {
            ApiErrorResponse::internal(format!("Failed to start auth flow: {e}"))
                .into_json_tuple()
        }
    }
}

/// DELETE /api/mcp/{name}/auth/revoke
pub async fn auth_revoke(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    let cfg = state.kernel.config_snapshot();
    let server = match cfg.mcp_servers.iter().find(|s| s.name == name) {
        Some(s) => s.clone(),
        None => {
            return ApiErrorResponse::not_found(format!("MCP server '{}' not found", name))
                .into_json_tuple();
        }
    };

    let server_url = match &server.transport {
        Some(librefang_types::config::McpTransportEntry::Http { url }) => url.clone(),
        _ => {
            return ApiErrorResponse::bad_request("OAuth is only supported for HTTP transport".into())
                .into_json_tuple();
        }
    };

    // Clear tokens
    let provider = state.kernel.oauth_provider_ref();
    if let Err(e) = provider.clear_tokens(&server_url).await {
        tracing::warn!(error = %e, "Failed to clear OAuth tokens");
    }

    // Remove from auth states
    state.kernel.mcp_auth_states_ref().lock().await.remove(&name);

    // Disconnect the MCP server
    {
        let mut connections = state.kernel.mcp_connections_ref().lock().await;
        connections.retain(|c| c.name() != name);
    }

    (
        axum::http::StatusCode::OK,
        Json(serde_json::json!({
            "server": name,
            "state": "not_required",
        })),
    )
}
```

- [ ] **Step 2: Add accessor methods to kernel**

In `kernel.rs`, add public accessors for the auth states and OAuth provider:

```rust
    /// Get a reference to the MCP auth states map.
    pub fn mcp_auth_states_ref(
        &self,
    ) -> &tokio::sync::Mutex<HashMap<String, librefang_runtime::mcp_oauth::McpAuthState>> {
        &self.mcp_auth_states
    }

    /// Get the kernel's OAuth provider.
    pub fn oauth_provider_ref(
        &self,
    ) -> Arc<dyn librefang_runtime::mcp_oauth::McpOAuthProvider> {
        Arc::new(crate::mcp_oauth_provider::KernelOAuthProvider::new(
            self.vault.clone(),
        ))
    }
```

- [ ] **Step 3: Register routes in skills.rs router**

In `crates/librefang-api/src/routes/skills.rs`, after the existing MCP server routes (line ~119), add:

```rust
        // MCP OAuth auth management
        .route(
            "/mcp/servers/{name}/auth/status",
            axum::routing::get(super::mcp_auth::auth_status),
        )
        .route(
            "/mcp/servers/{name}/auth/start",
            axum::routing::post(super::mcp_auth::auth_start),
        )
        .route(
            "/mcp/servers/{name}/auth/revoke",
            axum::routing::delete(super::mcp_auth::auth_revoke),
        )
```

- [ ] **Step 4: Register the module**

In `crates/librefang-api/src/routes/mod.rs` (or wherever routes are declared), add:

```rust
pub mod mcp_auth;
```

- [ ] **Step 5: Add `auth_state` to MCP server list response**

In `list_mcp_servers` (skills.rs, line ~2754), add auth state to each configured server:

```rust
    // Get auth states
    let auth_states = state.kernel.mcp_auth_states_ref().lock().await;

    let config_servers: Vec<serde_json::Value> = state
        .kernel
        .config_ref()
        .mcp_servers
        .iter()
        .map(|s| {
            let transport = s.transport.as_ref().map(serialize_mcp_transport);
            let auth = auth_states
                .get(&s.name)
                .and_then(|a| serde_json::to_value(a).ok())
                .unwrap_or(serde_json::json!(null));
            serde_json::json!({
                "name": s.name,
                "transport": transport,
                "timeout_secs": s.timeout_secs,
                "env": s.env,
                "auth_state": auth,
            })
        })
        .collect();
```

- [ ] **Step 6: Verify it compiles**

Run: `cargo build --workspace --lib`
Expected: Success.

- [ ] **Step 7: Commit**

```bash
git add crates/librefang-api/src/routes/mcp_auth.rs crates/librefang-api/src/routes/skills.rs crates/librefang-api/src/routes/mod.rs crates/librefang-kernel/src/kernel.rs
git commit -m "feat(api): add MCP OAuth auth endpoints and auth state in server list"
```

---

## Task 9: Dashboard — Auth State Badges and Actions

**Files:**
- Modify: `crates/librefang-api/dashboard/src/pages/McpServersPage.tsx`

- [ ] **Step 1: Read the current component**

Read `crates/librefang-api/dashboard/src/pages/McpServersPage.tsx` fully to understand the current component structure, query hooks, and rendering before making changes.

- [ ] **Step 2: Add auth API calls**

Add API functions (either inline or in the existing API module used by the page):

```typescript
async function getAuthStatus(name: string) {
  const res = await fetch(`/api/mcp/servers/${encodeURIComponent(name)}/auth/status`);
  return res.json();
}

async function startAuth(name: string) {
  const res = await fetch(`/api/mcp/servers/${encodeURIComponent(name)}/auth/start`, {
    method: "POST",
  });
  return res.json();
}

async function revokeAuth(name: string) {
  const res = await fetch(`/api/mcp/servers/${encodeURIComponent(name)}/auth/revoke`, {
    method: "DELETE",
  });
  return res.json();
}
```

- [ ] **Step 3: Add auth state badge component**

Add an `AuthBadge` component to the file:

```tsx
function AuthBadge({ server }: { server: { name: string; auth_state?: { state: string; auth_url?: string } } }) {
  const queryClient = useQueryClient();
  const [polling, setPolling] = React.useState(false);

  // Poll for auth completion when pending
  useQuery({
    queryKey: ["mcp-auth-status", server.name],
    queryFn: () => getAuthStatus(server.name),
    enabled: polling,
    refetchInterval: 2000,
    onSuccess: (data: any) => {
      if (data.auth?.state === "authorized" || data.auth?.state === "not_required") {
        setPolling(false);
        queryClient.invalidateQueries({ queryKey: ["mcp-servers"] });
      }
    },
  });

  const handleAuthorize = async () => {
    const result = await startAuth(server.name);
    if (result.auth_url) {
      window.open(result.auth_url, "_blank");
      setPolling(true);
    }
  };

  const handleRevoke = async () => {
    await revokeAuth(server.name);
    queryClient.invalidateQueries({ queryKey: ["mcp-servers"] });
  };

  const authState = server.auth_state?.state;

  if (!authState || authState === "not_required") {
    return null;
  }

  if (authState === "authorized") {
    return (
      <span className="inline-flex items-center gap-1">
        <span className="inline-block w-2 h-2 rounded-full bg-green-500" />
        <span className="text-xs text-green-600">Authorized</span>
        <button
          onClick={handleRevoke}
          className="text-xs text-gray-400 hover:text-red-500 ml-1"
        >
          Revoke
        </button>
      </span>
    );
  }

  if (authState === "pending_auth") {
    return (
      <button
        onClick={handleAuthorize}
        className="inline-flex items-center gap-1 px-2 py-1 text-xs rounded bg-amber-100 text-amber-700 hover:bg-amber-200"
      >
        <span className="inline-block w-2 h-2 rounded-full bg-amber-500 animate-pulse" />
        Authorize
      </button>
    );
  }

  if (authState === "expired") {
    return (
      <button
        onClick={handleAuthorize}
        className="inline-flex items-center gap-1 px-2 py-1 text-xs rounded bg-red-100 text-red-700 hover:bg-red-200"
      >
        <span className="inline-block w-2 h-2 rounded-full bg-red-500" />
        Re-authorize
      </button>
    );
  }

  return null;
}
```

- [ ] **Step 4: Integrate badge into server list rows**

In the existing server list rendering, add `<AuthBadge server={server} />` next to each server name or status indicator. The exact JSX location depends on the current component structure found in Step 1.

- [ ] **Step 5: Build the dashboard**

Run: `cd crates/librefang-api/dashboard && npm run build`
Expected: Success — dashboard builds with new components.

- [ ] **Step 6: Commit**

```bash
git add crates/librefang-api/dashboard/
git commit -m "feat(dashboard): add OAuth auth badges and authorize/revoke actions to MCP servers"
```

---

## Task 10: Integration Tests

**Files:**
- Create: `crates/librefang-runtime/tests/mcp_oauth_integration.rs` (or add to existing test file)

- [ ] **Step 1: Write integration test for full discovery flow**

```rust
//! Integration tests for MCP OAuth discovery.
//!
//! Uses mock HTTP servers to simulate the OAuth flow without a real provider.

use librefang_runtime::mcp_oauth::*;

#[tokio::test]
async fn test_discover_from_well_known() {
    // Start a mock server that serves .well-known
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let base_url = format!("http://127.0.0.1:{port}");

    let mock_metadata = serde_json::json!({
        "issuer": base_url,
        "authorization_endpoint": format!("{base_url}/oauth/authorize"),
        "token_endpoint": format!("{base_url}/oauth/token"),
        "response_types_supported": ["code"],
        "code_challenge_methods_supported": ["S256"]
    });
    let body = mock_metadata.to_string();

    // Spawn mock server
    tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut buf = vec![0u8; 4096];
        let n = tokio::io::AsyncReadExt::read(
            &mut tokio::io::BufReader::new(&stream),
            &mut buf,
        )
        .await
        .unwrap();
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
            body.len(),
            body
        );
        tokio::io::AsyncWriteExt::write_all(&mut &stream, response.as_bytes())
            .await
            .unwrap();
    });

    let server_url = format!("{base_url}/mcp");
    let result = discover_oauth_metadata(&server_url, None, None).await;
    assert!(result.is_ok());
    let meta = result.unwrap();
    assert_eq!(
        meta.authorization_endpoint,
        format!("{base_url}/oauth/authorize")
    );
    assert_eq!(meta.token_endpoint, format!("{base_url}/oauth/token"));
}

#[tokio::test]
async fn test_discover_fallback_to_config() {
    let config = librefang_types::config::McpOAuthConfig {
        auth_url: Some("https://example.com/auth".into()),
        token_url: Some("https://example.com/token".into()),
        client_id: Some("test-id".into()),
        scopes: vec!["read".into()],
    };

    // No www_authenticate, no .well-known, so should fall back to config
    let result = discover_oauth_metadata(
        "https://nonexistent.example.com/mcp",
        None,
        Some(&config),
    )
    .await;
    assert!(result.is_ok());
    let meta = result.unwrap();
    assert_eq!(meta.authorization_endpoint, "https://example.com/auth");
    assert_eq!(meta.token_endpoint, "https://example.com/token");
    assert_eq!(meta.client_id.unwrap(), "test-id");
}

#[tokio::test]
async fn test_discover_fails_without_any_source() {
    let result = discover_oauth_metadata(
        "https://nonexistent.example.com/mcp",
        None,
        None,
    )
    .await;
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("no OAuth metadata"));
}
```

- [ ] **Step 2: Run integration tests**

Run: `cargo test --lib -p librefang-runtime mcp_oauth && cargo test -p librefang-runtime --test mcp_oauth_integration`
Expected: All tests pass.

- [ ] **Step 3: Commit**

```bash
git add crates/librefang-runtime/tests/mcp_oauth_integration.rs
git commit -m "test(runtime): add integration tests for MCP OAuth discovery"
```

---

## Task 11: Full Build Verification and Clippy

**Files:** None (verification only)

- [ ] **Step 1: Full workspace build**

Run: `cargo build --workspace --lib`
Expected: Success, no errors.

- [ ] **Step 2: All tests**

Run: `cargo test --workspace`
Expected: All tests pass (2100+ existing + new OAuth tests).

- [ ] **Step 3: Clippy**

Run: `cargo clippy --workspace --all-targets -- -D warnings`
Expected: Zero warnings.

- [ ] **Step 4: Fix any issues found**

If any step fails, fix the issues and re-run.

- [ ] **Step 5: Final commit if any fixes**

```bash
git add -A
git commit -m "fix: address clippy warnings and test failures from MCP OAuth feature"
```

---

## Task Summary

| Task | Description | Key Files |
|------|-------------|-----------|
| 1 | Config types | `types/config/types.rs` |
| 2 | Core types + WWW-Authenticate parser | `runtime/mcp_oauth.rs` |
| 3 | McpOAuthProvider trait + PKCE helpers | `runtime/mcp_oauth.rs` |
| 4 | .well-known metadata fetcher | `runtime/mcp_oauth.rs` |
| 5 | Wire OAuth into McpConnection | `runtime/mcp.rs` |
| 6 | KernelOAuthProvider implementation | `kernel/mcp_oauth_provider.rs`, `kernel/kernel.rs` |
| 7 | Auth completion + auto-reconnect | `kernel/kernel.rs` |
| 8 | API auth endpoints | `api/routes/mcp_auth.rs`, `api/routes/skills.rs` |
| 9 | Dashboard auth badges | `dashboard/src/pages/McpServersPage.tsx` |
| 10 | Integration tests | `runtime/tests/mcp_oauth_integration.rs` |
| 11 | Full build verification | (verification only) |
| 12 | **DELTA:** UI-driven auth + API callback | `mcp.rs`, `kernel.rs`, `mcp_oauth_provider.rs`, `mcp_auth.rs` |

---

## Task 12 (DELTA): UI-Driven Auth Flow + API Callback

**Supersedes:** The daemon-initiated PKCE flow from Tasks 5-7. The daemon
no longer starts OAuth flows at boot — it only detects 401 and marks the
server as `NeedsAuth`. The full OAuth handshake is UI-driven via
`POST /api/mcp/servers/{name}/auth/start`, with the callback routed through
`GET /api/mcp/servers/{name}/auth/callback` on the API port (4545).

### Rationale

The previous approach bound a localhost TCP listener on a random ephemeral
port for the OAuth callback. In Docker/remote server deployments, this port
is unreachable from the user's browser, making the auth flow impossible to
complete. Routing the callback through the API server's existing port solves
this without extra port forwarding.

### Changes Required

**A) `crates/librefang-runtime/src/mcp.rs` — daemon boot stops at detection**

In `connect_streamable_http`, when auth is required and no cached token:
- Do NOT call `provider.start_auth_flow()`
- Return `Err("OAUTH_NEEDS_AUTH")` (not `OAUTH_PENDING:{url}`)
- The kernel sets state to `NeedsAuth` without any auth URL

**B) `crates/librefang-kernel/src/kernel.rs` — handle `OAUTH_NEEDS_AUTH`**

In `connect_mcp_servers`, replace the `OAUTH_PENDING:` handler:
- On `OAUTH_NEEDS_AUTH`: set state to `McpAuthState::PendingAuth { auth_url: String::new() }`
- No watcher task spawned — flow is entirely UI-driven
- Remove `watch_oauth_completion` if unused

**C) `crates/librefang-kernel/src/mcp_oauth_provider.rs` — remove localhost listener**

Remove the `start_auth_flow` implementation's TCP listener, callback handler,
and `open_browser` logic. The `start_auth_flow` trait method can be simplified
or replaced with a new method that just generates the PKCE challenge and
returns it without starting a server.

Instead, add a new method or restructure so the API layer can:
1. Call `generate_pkce()` and `generate_state()` (already public in runtime)
2. Store verifier + state in vault keyed by server name
3. Build the auth URL with `redirect_uri` pointing to the API callback
4. Return the auth URL

**D) `crates/librefang-api/src/routes/mcp_auth.rs` — full flow in API**

`POST /api/mcp/servers/{name}/auth/start`:
1. Discover OAuth metadata
2. Dynamic Client Registration if needed
3. Generate PKCE verifier/challenge + state
4. Store verifier + state in vault: `mcp_oauth:{url}:pkce_verifier`, `mcp_oauth:{url}:pkce_state`
5. Build auth URL with `redirect_uri` derived from the request's `Origin`,
   `X-Forwarded-Host`, or `Host` header: `{origin}/api/mcp/servers/{name}/auth/callback`
   - No hardcoded port — works behind reverse proxies and in Docker
6. Set state to `PendingAuth { auth_url }`
7. Return `{ "auth_url": "..." }`

`GET /api/mcp/servers/{name}/auth/callback` (NEW):
1. Read `code` and `state` from query params
2. Load stored `pkce_state` from vault — validate it matches
3. Load stored `pkce_verifier` from vault
4. Load stored `client_id` and `token_endpoint` from vault
5. POST to token_endpoint: `grant_type=authorization_code`, `code`, `code_verifier`, `redirect_uri`, `client_id`
6. Store tokens in vault
7. Call `kernel.retry_mcp_connection(name)`
8. Set state to `Authorized`
9. Return HTML: "Authorization complete. You can close this tab."

Register the callback route:
```rust
.route("/mcp/servers/{name}/auth/callback", axum::routing::get(super::mcp_auth::auth_callback))
```

**E) Dashboard — no changes needed**

The dashboard already calls `POST /auth/start` and opens the returned URL.
The callback now goes through the API, so the polling will pick up the
state change to `Authorized` automatically.

### Verification

1. `cargo build --workspace --lib`
2. `cargo test --workspace`
3. `cargo clippy --workspace --all-targets -- -D warnings`
4. Manual test: deploy to Docker, click "Authorize" in dashboard, complete
   Notion consent, verify callback reaches API and server connects
