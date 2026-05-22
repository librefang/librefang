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

/// OAuth provider configuration template — embedded in [`crate::mcp::McpCatalogEntry`]
/// to describe how an MCP catalog entry should perform an OAuth2 PKCE flow.
///
/// The runtime PKCE machinery lives in `librefang-extensions::oauth`; this
/// type only carries the template values declared in
/// `~/.librefang/mcp/catalog/*.toml`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuthTemplate {
    /// OAuth provider (google, github, microsoft, slack).
    pub provider: String,
    /// OAuth scopes required.
    pub scopes: Vec<String>,
    /// Authorization URL.
    pub auth_url: String,
    /// Token exchange URL.
    pub token_url: String,
}

/// OAuth2 token response from the token endpoint.
///
/// Field defaults follow the providers we have observed:
/// - `refresh_token` is optional (Slack and many bot tokens omit it)
/// - `token_type` defaults to `"Bearer"` when the server omits it
/// - `expires_in` defaults to `0` (some providers omit when token doesn't expire)
/// - `scope` defaults to empty (some providers omit when no scopes apply)
///
/// `Debug` is hand-written to redact the access and refresh tokens —
/// see the impl below. Audit: oauth-tokens-derive-debug-serialize.
#[derive(Clone, Serialize, Deserialize)]
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

impl std::fmt::Debug for OAuthTokens {
    /// Hand-written `Debug` that redacts the cleartext tokens.
    /// Mirrors the pattern at
    /// `librefang-llm-drivers::credential_pool::PooledCredential` —
    /// emits `<redacted len=N hint=****XXXX>` so logs and panic
    /// messages stay diagnosable (length + last-4 chars) without
    /// leaking the secret. Audit: oauth-tokens-derive-debug-serialize.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OAuthTokens")
            .field("access_token", &redact(&self.access_token))
            .field("refresh_token", &self.refresh_token.as_deref().map(redact))
            .field("token_type", &self.token_type)
            .field("expires_in", &self.expires_in)
            .field("scope", &self.scope)
            .finish()
    }
}

/// Build a redaction-safe display string for a secret. Echoes the
/// secret's length and last 4 characters (or fewer when the secret
/// itself is shorter than 4) so logs can correlate "same token as
/// last call" without exposing the value. Empty inputs render as
/// `<redacted len=0>`.
fn redact(secret: &str) -> String {
    let len = secret.chars().count();
    if len == 0 {
        return "<redacted len=0>".to_string();
    }
    // Show at most 4 trailing chars; never the whole value. For
    // secrets ≤ 4 chars the hint window is 0 — we won't echo any
    // portion of a very short secret. For 5..=7 chars the window
    // grows linearly so logs still have a fingerprint to correlate
    // on. At 8+ it caps at 4.
    let hint_chars = len.saturating_sub(4).min(4);
    if hint_chars == 0 {
        return format!("<redacted len={len}>");
    }
    let hint: String = secret
        .chars()
        .rev()
        .take(hint_chars)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    format!("<redacted len={len} hint=****{hint}>")
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

    #[test]
    fn debug_redacts_access_and_refresh_tokens() {
        // Audit: oauth-tokens-derive-debug-serialize. The auto-derived
        // `Debug` was a silent leak path for any caller that did
        // `tracing::debug!(?tokens, …)` / `error!(?tokens, …)` /
        // `format!("{:?}", tokens)`. The hand-written impl must never
        // echo the cleartext secret — only length + last-4 hint —
        // and the non-secret metadata (token_type, expires_in, scope)
        // must still come through so diagnostics work.
        let tokens = OAuthTokens {
            access_token: "supersecret-1234567890".to_string(),
            refresh_token: Some("refresh-secret-abcdef".to_string()),
            token_type: "Bearer".to_string(),
            expires_in: 3600,
            scope: "read write".to_string(),
        };
        let dbg = format!("{tokens:?}");
        assert!(
            !dbg.contains("supersecret-1234567890"),
            "access_token must not appear verbatim in Debug output: {dbg}"
        );
        assert!(
            !dbg.contains("refresh-secret-abcdef"),
            "refresh_token must not appear verbatim in Debug output: {dbg}"
        );
        assert!(
            dbg.contains("<redacted"),
            "Debug output must include redaction marker: {dbg}"
        );
        assert!(
            dbg.contains("Bearer"),
            "non-secret token_type must still appear: {dbg}"
        );
        assert!(
            dbg.contains("3600"),
            "non-secret expires_in must still appear: {dbg}"
        );
        assert!(
            dbg.contains("read write"),
            "non-secret scope must still appear: {dbg}"
        );
    }

    #[test]
    fn debug_redacts_none_refresh_token_without_panic() {
        let tokens = OAuthTokens {
            access_token: "abc123".to_string(),
            refresh_token: None,
            token_type: "Bearer".to_string(),
            expires_in: 0,
            scope: String::new(),
        };
        let dbg = format!("{tokens:?}");
        assert!(!dbg.contains("abc123"), "access still leaking: {dbg}");
        assert!(
            dbg.contains("None"),
            "missing refresh must render as None: {dbg}"
        );
    }

    #[test]
    fn redact_hint_handles_short_secrets_without_exposing_full_value() {
        // Single char: never show the actual char.
        assert_eq!(redact("a"), "<redacted len=1>");
        // Exactly 4 chars: still safe — hint window collapses to 0.
        assert_eq!(redact("abcd"), "<redacted len=4>");
        // Length 5: show only the last char (one beyond the safe
        // window of 4) so the hint is non-empty without revealing
        // the whole thing.
        assert_eq!(redact("abcde"), "<redacted len=5 hint=****e>");
        // Length 8: show 4 trailing chars.
        assert_eq!(redact("abcdefgh"), "<redacted len=8 hint=****efgh>");
        // Empty input.
        assert_eq!(redact(""), "<redacted len=0>");
    }

    #[test]
    fn redact_uses_char_count_not_byte_count_for_unicode_secrets() {
        // 5 logical chars, more bytes — len reported must match the
        // user-perceived count so the redaction looks consistent.
        let secret = "数据库的钥匙";
        let len_chars = secret.chars().count();
        let rendered = redact(secret);
        assert!(
            rendered.contains(&format!("len={len_chars}")),
            "char count expected: {rendered}"
        );
    }
}
