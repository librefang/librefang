//! Canonical OAuth2 token types shared across LibreFang crates.
//!
//! [`OAuthTokens`] is the deserialized response from an OAuth2 token endpoint
//! (`access_token`, optional `refresh_token`, `token_type`, `expires_in`,
//! `scope`).  It is used both by the public PKCE flow in `librefang-extensions`
//! (Google / GitHub / Microsoft / Slack one-click integrations) and by the
//! MCP OAuth flow in `librefang-runtime-mcp` / `librefang-api`.
//!
//! The struct lives here to avoid two divergent definitions and to give every
//! caller a single canonical type for trait signatures (`McpOAuthProvider`,
//! `ExtensionResult`, etc.).

use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

/// OAuth2 token response from the token endpoint.
///
/// Field defaults follow the providers we have observed:
/// - `refresh_token` is optional (Slack and many bot tokens omit it)
/// - `token_type` defaults to `"Bearer"` when the server omits it
/// - `expires_in` defaults to `0` (some providers omit when token doesn't expire)
/// - `scope` defaults to empty (some providers omit when no scopes apply)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuthTokens {
    /// Access token used to authenticate API calls.
    pub access_token: String,
    /// Refresh token used to obtain a new access token (when provided).
    #[serde(default)]
    pub refresh_token: Option<String>,
    /// Token type, typically `"Bearer"`.
    #[serde(default = "default_token_type")]
    pub token_type: String,
    /// Seconds until `access_token` expires.
    #[serde(default)]
    pub expires_in: u64,
    /// Space-delimited scopes granted by the authorization server.
    #[serde(default)]
    pub scope: String,
}

/// Default `token_type` value applied when the provider omits it. RFC 6749
/// allows the field to be absent for `Bearer` tokens.
pub fn default_token_type() -> String {
    "Bearer".to_string()
}

impl OAuthTokens {
    /// Wrap the access token in a [`Zeroizing`] string so it is wiped on drop.
    pub fn access_token_zeroizing(&self) -> Zeroizing<String> {
        Zeroizing::new(self.access_token.clone())
    }

    /// Wrap the refresh token in a [`Zeroizing`] string when present.
    pub fn refresh_token_zeroizing(&self) -> Option<Zeroizing<String>> {
        self.refresh_token
            .as_ref()
            .map(|t| Zeroizing::new(t.clone()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deserialize_minimal_response() {
        let body = r#"{"access_token": "abc"}"#;
        let tokens: OAuthTokens = serde_json::from_str(body).unwrap();
        assert_eq!(tokens.access_token, "abc");
        assert!(tokens.refresh_token.is_none());
        assert_eq!(tokens.token_type, "Bearer");
        assert_eq!(tokens.expires_in, 0);
        assert_eq!(tokens.scope, "");
    }

    #[test]
    fn deserialize_full_response() {
        let body = r#"{
            "access_token": "abc",
            "refresh_token": "rrr",
            "token_type": "bearer",
            "expires_in": 3600,
            "scope": "read write"
        }"#;
        let tokens: OAuthTokens = serde_json::from_str(body).unwrap();
        assert_eq!(tokens.access_token, "abc");
        assert_eq!(tokens.refresh_token.as_deref(), Some("rrr"));
        assert_eq!(tokens.token_type, "bearer");
        assert_eq!(tokens.expires_in, 3600);
        assert_eq!(tokens.scope, "read write");
    }

    #[test]
    fn access_token_zeroizing_returns_value() {
        let tokens = OAuthTokens {
            access_token: "secret".to_string(),
            refresh_token: None,
            token_type: "Bearer".to_string(),
            expires_in: 0,
            scope: String::new(),
        };
        let z = tokens.access_token_zeroizing();
        assert_eq!(&*z, "secret");
    }

    #[test]
    fn refresh_token_zeroizing_some() {
        let tokens = OAuthTokens {
            access_token: "a".to_string(),
            refresh_token: Some("r".to_string()),
            token_type: "Bearer".to_string(),
            expires_in: 0,
            scope: String::new(),
        };
        assert_eq!(&*tokens.refresh_token_zeroizing().unwrap(), "r");
    }

    #[test]
    fn refresh_token_zeroizing_none() {
        let tokens = OAuthTokens {
            access_token: "a".to_string(),
            refresh_token: None,
            token_type: "Bearer".to_string(),
            expires_in: 0,
            scope: String::new(),
        };
        assert!(tokens.refresh_token_zeroizing().is_none());
    }
}
