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

/// Mock provider that records whether `start_auth_flow` was called.
struct TrackingOAuthProvider {
    start_auth_called: AtomicBool,
    load_token_called: AtomicBool,
}

impl TrackingOAuthProvider {
    fn new() -> Self {
        Self {
            start_auth_called: AtomicBool::new(false),
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

    async fn start_auth_flow(
        &self,
        _server_url: &str,
        _metadata: OAuthMetadata,
    ) -> Result<AuthFlowHandle, String> {
        self.start_auth_called.store(true, Ordering::SeqCst);
        // Return an error to stop the flow — we just want to verify it was called
        Err("test: auth flow triggered".to_string())
    }
}

/// Verify that `McpConnection::connect` calls the OAuth provider when
/// a Streamable HTTP server returns a 401-like error.
///
/// This is an indirect test: we connect to a URL that will fail (no server),
/// and the error won't be a 401, so the provider's `start_auth_flow` won't
/// be called. But `load_token` MUST be called — proving the provider is
/// wired in and not silently `None`.
#[tokio::test]
async fn test_http_connect_calls_oauth_provider_load_token() {
    use librefang_runtime::mcp::{McpConnection, McpServerConfig, McpTransport};

    let provider = Arc::new(TrackingOAuthProvider::new());

    let config = McpServerConfig {
        name: "test-oauth-wiring".to_string(),
        transport: McpTransport::Http {
            url: "http://127.0.0.1:1/nonexistent-mcp".to_string(), // Will fail to connect
        },
        timeout_secs: 5,
        env: vec![],
        headers: vec![],
        oauth_provider: Some(provider.clone()),
        oauth_config: None,
    };

    // Connection will fail (no server at port 1), but the provider
    // should still be consulted for a cached token BEFORE the attempt.
    let result = McpConnection::connect(config).await;
    assert!(result.is_err(), "Expected connection to fail");

    // The critical assertion: load_token was called, proving the provider
    // is wired in. If oauth_provider were None, this would be false.
    assert!(
        provider.load_token_called.load(Ordering::SeqCst),
        "OAuth provider's load_token was never called — oauth_provider is likely None"
    );
}

/// Verify that `NoOpOAuthProvider` returns an error from `start_auth_flow`.
/// This ensures that if someone accidentally uses NoOp, the error message
/// is clear rather than a silent failure.
#[tokio::test]
async fn test_noop_provider_returns_clear_error() {
    let noop = NoOpOAuthProvider;
    let metadata = OAuthMetadata {
        authorization_endpoint: "https://example.com/auth".into(),
        token_endpoint: "https://example.com/token".into(),
        client_id: None,
        scopes: vec![],
        server_url: "https://example.com/mcp".into(),
    };
    let result = noop
        .start_auth_flow("https://example.com/mcp", metadata)
        .await;
    assert!(result.is_err());
    let err = result.err().unwrap();
    assert!(
        err.contains("No OAuth provider configured"),
        "NoOpOAuthProvider should return a clear error message, got: {err}"
    );
}
