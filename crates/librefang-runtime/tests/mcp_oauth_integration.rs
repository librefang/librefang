//! Integration tests for MCP OAuth discovery.

use librefang_runtime::mcp_oauth::*;

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
    assert!(result.unwrap_err().contains("no OAuth metadata"));
}
