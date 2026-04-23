//! Integration tests for MCP OAuth discovery.

use async_trait::async_trait;
use librefang_runtime::mcp_oauth::*;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

#[tokio::test]
async fn test_discover_fallback_to_config() {
    let config = librefang_types::config::McpOAuthConfig {
        auth_url: Some("https://example.com/auth".into()),
        token_url: Some("https://example.com/token".into()),
        client_id: Some("test-id".into()),
        scopes: vec!["read".into()],
        user_scopes: Vec::new(),
    };
    let result =
        discover_oauth_metadata("https://nonexistent.example.com/mcp", None, Some(&config)).await;
    assert!(result.is_ok());
    let meta = result.unwrap();
    assert_eq!(meta.authorization_endpoint, "https://example.com/auth");
    assert_eq!(meta.token_endpoint, "https://example.com/token");
    assert_eq!(meta.client_id.unwrap(), "test-id");
}

#[tokio::test]
async fn test_discover_fails_without_any_source() {
    let result = discover_oauth_metadata("https://nonexistent.example.com/mcp", None, None).await;
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("OAuth metadata"));
}

// ---------------------------------------------------------------------------
// Regression test: verify the OAuth provider is actually invoked when an
// Http MCP connection fails with a 401.
//
// This catches the bug where `oauth_provider: None` was passed in kernel's
// `connect_mcp_servers`, silently disabling the entire OAuth flow.
// ---------------------------------------------------------------------------

/// Mock provider that records whether `load_token` was called.
struct TrackingOAuthProvider {
    load_token_called: AtomicBool,
}

impl TrackingOAuthProvider {
    fn new() -> Self {
        Self {
            load_token_called: AtomicBool::new(false),
        }
    }
}

#[async_trait]
impl McpOAuthProvider for TrackingOAuthProvider {
    async fn load_token(&self, _server_url: &str) -> Option<String> {
        self.load_token_called.store(true, Ordering::SeqCst);
        None // No cached token — force the connect to fail with 401
    }

    async fn store_tokens(&self, _server_url: &str, _tokens: OAuthTokens) -> Result<(), String> {
        Ok(())
    }

    async fn clear_tokens(&self, _server_url: &str) -> Result<(), String> {
        Ok(())
    }
}

/// Verify that `McpConnection::connect` calls the OAuth provider when
/// a Streamable HTTP server returns a 401-like error.
///
/// `load_token` MUST be called — proving the provider is wired in and
/// not silently `None`.
#[tokio::test]
async fn test_http_connect_calls_oauth_provider_load_token() {
    use librefang_runtime::mcp::{McpConnection, McpServerConfig, McpTransport};

    let provider = Arc::new(TrackingOAuthProvider::new());

    let config = McpServerConfig {
        name: "test-oauth-wiring".to_string(),
        transport: McpTransport::Http {
            url: "http://127.0.0.1:1/nonexistent-mcp".to_string(),
        },
        timeout_secs: 5,
        env: vec![],
        headers: vec![],
        oauth_provider: Some(provider.clone()),
        oauth_config: None,
        taint_scanning: true,
        taint_policy: None,
        roots: vec![],
    };

    let result = McpConnection::connect(config).await;
    assert!(result.is_err(), "Expected connection to fail");

    assert!(
        provider.load_token_called.load(Ordering::SeqCst),
        "OAuth provider's load_token was never called — oauth_provider is likely None"
    );
}

// ---------------------------------------------------------------------------
// Token lifecycle tests via mock provider
// ---------------------------------------------------------------------------

/// Mock provider that stores tokens in memory (no vault dependency).
struct InMemoryOAuthProvider {
    tokens: tokio::sync::Mutex<std::collections::HashMap<String, OAuthTokens>>,
}

impl InMemoryOAuthProvider {
    fn new() -> Self {
        Self {
            tokens: tokio::sync::Mutex::new(std::collections::HashMap::new()),
        }
    }
}

#[async_trait]
impl McpOAuthProvider for InMemoryOAuthProvider {
    async fn load_token(&self, server_url: &str) -> Option<String> {
        let tokens = self.tokens.lock().await;
        tokens.get(server_url).map(|t| t.access_token.clone())
    }

    async fn store_tokens(&self, server_url: &str, tokens: OAuthTokens) -> Result<(), String> {
        self.tokens
            .lock()
            .await
            .insert(server_url.to_string(), tokens);
        Ok(())
    }

    async fn clear_tokens(&self, server_url: &str) -> Result<(), String> {
        self.tokens.lock().await.remove(server_url);
        Ok(())
    }
}

/// Verify store_tokens followed by load_token returns the token.
#[tokio::test]
async fn test_provider_store_then_load() {
    let provider = InMemoryOAuthProvider::new();
    let url = "https://mcp.notion.com/mcp";

    // Initially no token
    assert!(provider.load_token(url).await.is_none());

    // Store a token
    let tokens = OAuthTokens {
        access_token: "test_access_token".to_string(),
        refresh_token: Some("test_refresh".to_string()),
        token_type: "Bearer".to_string(),
        expires_in: 3600,
        scope: "".to_string(),
    };
    provider.store_tokens(url, tokens).await.unwrap();

    // Should return the stored token
    assert_eq!(provider.load_token(url).await.unwrap(), "test_access_token");
}

/// Verify clear_tokens removes the token.
#[tokio::test]
async fn test_provider_clear_removes_token() {
    let provider = InMemoryOAuthProvider::new();
    let url = "https://mcp.notion.com/mcp";

    let tokens = OAuthTokens {
        access_token: "tok".to_string(),
        refresh_token: None,
        token_type: "Bearer".to_string(),
        expires_in: 0,
        scope: "".to_string(),
    };
    provider.store_tokens(url, tokens).await.unwrap();
    assert!(provider.load_token(url).await.is_some());

    provider.clear_tokens(url).await.unwrap();
    assert!(
        provider.load_token(url).await.is_none(),
        "Token should be gone after clear"
    );
}

/// Verify clear_tokens only affects the target server.
#[tokio::test]
async fn test_provider_clear_is_isolated() {
    let provider = InMemoryOAuthProvider::new();
    let url_a = "https://server-a.com/mcp";
    let url_b = "https://server-b.com/mcp";

    let make_token = |name: &str| OAuthTokens {
        access_token: name.to_string(),
        refresh_token: None,
        token_type: "Bearer".to_string(),
        expires_in: 0,
        scope: "".to_string(),
    };

    provider
        .store_tokens(url_a, make_token("tok_a"))
        .await
        .unwrap();
    provider
        .store_tokens(url_b, make_token("tok_b"))
        .await
        .unwrap();

    // Clear only A
    provider.clear_tokens(url_a).await.unwrap();

    assert!(provider.load_token(url_a).await.is_none());
    assert_eq!(provider.load_token(url_b).await.unwrap(), "tok_b");
}

/// Verify the expected state transition: store → clear → store should work.
/// This ensures that after revoking (clear), re-authorizing (store) works.
#[tokio::test]
async fn test_provider_reauthorize_after_clear() {
    let provider = InMemoryOAuthProvider::new();
    let url = "https://mcp.notion.com/mcp";

    let make_token = |name: &str| OAuthTokens {
        access_token: name.to_string(),
        refresh_token: None,
        token_type: "Bearer".to_string(),
        expires_in: 0,
        scope: "".to_string(),
    };

    // First auth
    provider
        .store_tokens(url, make_token("tok_v1"))
        .await
        .unwrap();
    assert_eq!(provider.load_token(url).await.unwrap(), "tok_v1");

    // Revoke
    provider.clear_tokens(url).await.unwrap();
    assert!(provider.load_token(url).await.is_none());

    // Re-authorize with new token
    provider
        .store_tokens(url, make_token("tok_v2"))
        .await
        .unwrap();
    assert_eq!(
        provider.load_token(url).await.unwrap(),
        "tok_v2",
        "Re-authorization after revoke should work with the new token"
    );
}

/// Verify the auth state lifecycle: NeedsAuth → PendingAuth → Authorized → NeedsAuth (after revoke).
/// Regression test for the bug where revoking removed the auth state entirely,
/// leaving no "Authorize" button in the dashboard.
#[test]
fn test_auth_state_lifecycle() {
    // Boot: server returns 401 → NeedsAuth
    let state = McpAuthState::NeedsAuth;
    let json = serde_json::to_value(&state).unwrap();
    assert_eq!(json["state"].as_str().unwrap(), "needs_auth");

    // User clicks Authorize → PendingAuth
    let state = McpAuthState::PendingAuth {
        auth_url: "https://example.com/auth".to_string(),
    };
    let json = serde_json::to_value(&state).unwrap();
    assert_eq!(json["state"].as_str().unwrap(), "pending_auth");

    // Callback succeeds → Authorized
    let state = McpAuthState::Authorized {
        expires_at: None,
        tokens: None,
    };
    let json = serde_json::to_value(&state).unwrap();
    assert_eq!(json["state"].as_str().unwrap(), "authorized");

    // User revokes → back to NeedsAuth (NOT removed)
    let state = McpAuthState::NeedsAuth;
    let json = serde_json::to_value(&state).unwrap();
    assert_eq!(
        json["state"].as_str().unwrap(),
        "needs_auth",
        "After revoke, state should be NeedsAuth so the Authorize button appears"
    );
}

/// Verify that NeedsAuth is a distinct state from PendingAuth.
/// This is a regression test for the bug where the dashboard showed
/// "Authorizing..." at boot before the user clicked Authorize.
#[test]
fn test_needs_auth_serializes_differently_from_pending_auth() {
    let needs = serde_json::to_value(McpAuthState::NeedsAuth).unwrap();
    let pending = serde_json::to_value(McpAuthState::PendingAuth {
        auth_url: "https://example.com/auth".to_string(),
    })
    .unwrap();

    assert_eq!(needs["state"].as_str().unwrap(), "needs_auth");
    assert_eq!(pending["state"].as_str().unwrap(), "pending_auth");
    assert_ne!(
        needs["state"], pending["state"],
        "NeedsAuth and PendingAuth must serialize to different state values"
    );
}
