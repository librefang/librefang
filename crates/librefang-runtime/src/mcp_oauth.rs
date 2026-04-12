//! MCP OAuth discovery and authentication flow.
//!
//! Implements RFC 8414-style metadata discovery for MCP servers that require
//! OAuth 2.0 authorization, plus token lifecycle management.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tokio::sync::oneshot;

/// OAuth 2.0 authorization server metadata (RFC 8414 subset).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuthMetadata {
    pub issuer: String,
    pub authorization_endpoint: String,
    pub token_endpoint: String,
    #[serde(default)]
    pub registration_endpoint: Option<String>,
    #[serde(default)]
    pub revocation_endpoint: Option<String>,
    #[serde(default)]
    pub scopes_supported: Vec<String>,
    #[serde(default)]
    pub response_types_supported: Vec<String>,
    #[serde(default)]
    pub grant_types_supported: Vec<String>,
}

/// Tokens obtained from an OAuth 2.0 authorization flow.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuthTokens {
    pub access_token: String,
    #[serde(default)]
    pub refresh_token: Option<String>,
    #[serde(default)]
    pub token_type: Option<String>,
    #[serde(default)]
    pub expires_in: Option<u64>,
    /// Absolute epoch-second expiry (computed on receipt).
    #[serde(default)]
    pub expires_at: Option<u64>,
}

/// Per-server authentication state tracked by the kernel.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "state")]
pub enum McpAuthState {
    /// No authentication required or not yet probed.
    #[serde(rename = "not_required")]
    NotRequired,
    /// OAuth flow initiated — user needs to visit `auth_url`.
    #[serde(rename = "pending_auth")]
    PendingAuth { auth_url: String },
    /// Successfully authorized with valid tokens.
    #[serde(rename = "authorized")]
    Authorized {
        #[serde(skip)]
        tokens: Option<OAuthTokens>,
    },
    /// Tokens expired and could not be refreshed.
    #[serde(rename = "expired")]
    Expired,
    /// Authentication error.
    #[serde(rename = "error")]
    Error { message: String },
}

/// Handle returned when starting an auth flow — the API waits on `completion`
/// for the callback to deliver tokens.
pub struct AuthFlowHandle {
    /// URL the user should open in their browser.
    pub auth_url: String,
    /// Completes when the OAuth callback delivers tokens (or errors).
    pub completion: oneshot::Receiver<Result<OAuthTokens, String>>,
    /// Sender side — held by the callback handler.
    pub completion_tx: Option<oneshot::Sender<Result<OAuthTokens, String>>>,
}

/// Configuration for MCP OAuth (from config.toml `[mcp_oauth]`).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct McpOAuthConfig {
    /// OAuth client_id to use for MCP server authorization.
    #[serde(default)]
    pub client_id: Option<String>,
    /// OAuth client_secret (optional — public clients omit this).
    #[serde(default)]
    pub client_secret: Option<String>,
    /// Redirect URI for the OAuth callback.
    #[serde(default)]
    pub redirect_uri: Option<String>,
    /// Extra scopes to request.
    #[serde(default)]
    pub scopes: Vec<String>,
}

/// Trait for pluggable OAuth providers (testable / mockable).
#[async_trait::async_trait]
pub trait McpOAuthProvider: Send + Sync {
    /// Start an authorization flow given discovered metadata.
    async fn start_auth_flow(
        &self,
        server_name: &str,
        metadata: &OAuthMetadata,
        config: &McpOAuthConfig,
    ) -> Result<AuthFlowHandle, String>;

    /// Clear stored tokens for a server.
    async fn clear_tokens(&self, server_name: &str) -> Result<(), String>;
}

/// Discover OAuth metadata from an MCP server URL.
///
/// Tries `{base}/.well-known/oauth-authorization-server` first,
/// then falls back to `{base}/.well-known/openid-configuration`.
pub async fn discover_oauth_metadata(
    server_url: &str,
    _config: &McpOAuthConfig,
) -> Result<OAuthMetadata, String> {
    let base = server_url.trim_end_matches('/');
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| format!("HTTP client error: {e}"))?;

    // Try RFC 8414 well-known endpoint first
    let well_known_url = format!("{base}/.well-known/oauth-authorization-server");
    match client.get(&well_known_url).send().await {
        Ok(resp) if resp.status().is_success() => {
            let metadata: OAuthMetadata = resp
                .json()
                .await
                .map_err(|e| format!("Failed to parse OAuth metadata: {e}"))?;
            return Ok(metadata);
        }
        _ => {}
    }

    // Fallback to OpenID Connect discovery
    let oidc_url = format!("{base}/.well-known/openid-configuration");
    match client.get(&oidc_url).send().await {
        Ok(resp) if resp.status().is_success() => {
            let metadata: OAuthMetadata = resp
                .json()
                .await
                .map_err(|e| format!("Failed to parse OIDC metadata: {e}"))?;
            Ok(metadata)
        }
        Ok(resp) => Err(format!(
            "OAuth discovery failed: {} from {oidc_url}",
            resp.status()
        )),
        Err(e) => Err(format!("OAuth discovery request failed: {e}")),
    }
}

/// Default no-op OAuth provider (used when no real provider is configured).
pub struct NoOpOAuthProvider;

#[async_trait::async_trait]
impl McpOAuthProvider for NoOpOAuthProvider {
    async fn start_auth_flow(
        &self,
        _server_name: &str,
        _metadata: &OAuthMetadata,
        _config: &McpOAuthConfig,
    ) -> Result<AuthFlowHandle, String> {
        Err("OAuth provider not configured".to_string())
    }

    async fn clear_tokens(&self, _server_name: &str) -> Result<(), String> {
        Ok(())
    }
}

/// Type alias for the shared auth-state map used by the kernel.
pub type McpAuthStates = tokio::sync::Mutex<HashMap<String, McpAuthState>>;
