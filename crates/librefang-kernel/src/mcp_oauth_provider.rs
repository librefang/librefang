//! Kernel-side OAuth provider for MCP servers.
//!
//! Implements `McpOAuthProvider` using the extensions vault for encrypted
//! token storage. The actual OAuth flow (PKCE, browser redirect) is driven
//! by the API layer — this provider handles token CRUD and client registration.

use async_trait::async_trait;
use librefang_runtime::mcp_oauth::{McpOAuthProvider, OAuthTokens};
use std::path::PathBuf;
use tracing::{debug, warn};

/// Vault key prefix for MCP OAuth tokens.
const VAULT_PREFIX: &str = "mcp_oauth";

/// All vault fields stored per MCP server under the mcp_oauth namespace.
/// Kept in sync with every `store`/`vault_set` call in auth_start, store_tokens,
/// and try_refresh so that `clear_tokens` is exhaustive by construction.
const ALL_VAULT_FIELDS: &[&str] = &[
    "access_token",
    "refresh_token",
    "expires_at",
    "token_endpoint",
    "client_id",
    "pkce_verifier",
    "pkce_state",
    "redirect_uri",
];

/// OAuth provider backed by the librefang encrypted credential vault.
///
/// Each instance is stateless — it opens and unlocks the vault on every
/// operation, mirroring the pattern used by `LibreFangKernel::vault_get`
/// and `vault_set`.
pub struct KernelOAuthProvider {
    /// Path to `~/.librefang` (home directory).
    home_dir: PathBuf,
}

impl KernelOAuthProvider {
    /// Create a new provider that stores tokens in the vault at `home_dir/vault.enc`.
    pub fn new(home_dir: PathBuf) -> Self {
        Self { home_dir }
    }

    /// Convenience: vault key for a specific server URL and field.
    pub fn vault_key(server_url: &str, field: &str) -> String {
        format!("{VAULT_PREFIX}:{server_url}:{field}")
    }

    /// Read a value from the vault. Returns `None` if the vault cannot be
    /// unlocked or the key is missing.
    pub fn vault_get(&self, key: &str) -> Option<String> {
        let vault_path = self.home_dir.join("vault.enc");
        let mut vault = librefang_extensions::vault::CredentialVault::new(vault_path);
        if let Err(e) = vault.unlock() {
            tracing::warn!(
                error = %e,
                key = %key,
                "MCP OAuth vault_get: unlock failed — returning None. \
                 Check that LIBREFANG_VAULT_KEY is set."
            );
            return None;
        }
        vault.get(key).map(|s| s.to_string())
    }

    /// Write a value to the vault. Creates the vault if it does not exist.
    pub fn vault_set(&self, key: &str, value: &str) -> Result<(), String> {
        let vault_path = self.home_dir.join("vault.enc");
        let mut vault = librefang_extensions::vault::CredentialVault::new(vault_path);
        if !vault.exists() {
            vault
                .init()
                .map_err(|e| format!("Vault init failed: {e}"))?;
        } else {
            vault
                .unlock()
                .map_err(|e| format!("Vault unlock failed: {e}"))?;
        }
        vault
            .set(key.to_string(), zeroize::Zeroizing::new(value.to_string()))
            .map_err(|e| format!("Vault write failed: {e}"))
    }

    /// Remove a value from the vault. Returns `Ok(true)` if the key existed.
    pub fn vault_remove(&self, key: &str) -> Result<bool, String> {
        let vault_path = self.home_dir.join("vault.enc");
        let mut vault = librefang_extensions::vault::CredentialVault::new(vault_path);
        if !vault.exists() {
            return Ok(false);
        }
        vault
            .unlock()
            .map_err(|e| format!("Vault unlock failed: {e}"))?;
        vault
            .remove(key)
            .map_err(|e| format!("Vault remove failed: {e}"))
    }

    /// Try to refresh the access token using a stored refresh token.
    async fn try_refresh(
        &self,
        server_url: &str,
        refresh_token: &str,
    ) -> Result<OAuthTokens, String> {
        let token_endpoint = self
            .vault_get(&Self::vault_key(server_url, "token_endpoint"))
            .ok_or_else(|| "No token_endpoint stored for refresh".to_string())?;

        // SSRF guard (#3623): re-validate the stored token_endpoint before
        // POSTing.  The stored value may predate policy tightening or have
        // been written by a compromised flow — always re-check before making
        // outbound requests.
        if let Err(reason) = librefang_runtime::mcp_oauth::is_ssrf_blocked_url(&token_endpoint) {
            return Err(format!(
                "SSRF: token_endpoint rejected for refresh: {reason}"
            ));
        }

        let client_id = self.vault_get(&Self::vault_key(server_url, "client_id"));

        let client = librefang_extensions::http_client::new_client();
        let mut params = vec![
            ("grant_type", "refresh_token".to_string()),
            ("refresh_token", refresh_token.to_string()),
        ];
        if let Some(cid) = &client_id {
            params.push(("client_id", cid.clone()));
        }

        let resp = client
            .post(&token_endpoint)
            .form(&params)
            .send()
            .await
            .map_err(|e| format!("Refresh token request failed: {e}"))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(format!("Token refresh failed (HTTP {status}): {body}"));
        }

        let tokens: OAuthTokens = resp
            .json()
            .await
            .map_err(|e| format!("Failed to parse refresh response: {e}"))?;

        Ok(tokens)
    }

    /// RFC 7591 Dynamic Client Registration.
    ///
    /// POSTs to the registration endpoint to obtain a client_id.
    /// This is required by servers like Notion's MCP that don't provide
    /// a pre-configured client_id.
    pub async fn register_client(
        &self,
        registration_endpoint: &str,
        redirect_uri: &str,
        _server_url: &str,
    ) -> Result<String, String> {
        // SSRF guard (#3623): registration_endpoint may have come from a
        // discovered metadata document or a vault entry written before policy
        // tightened.  Re-check before POSTing — the parser also checks, but
        // this is the actual outbound-request site and the cheapest place to
        // be sure.
        if let Err(reason) =
            librefang_runtime::mcp_oauth::is_ssrf_blocked_url(registration_endpoint)
        {
            return Err(format!("SSRF: registration_endpoint rejected: {reason}"));
        }
        let client = librefang_extensions::http_client::new_client();

        let body = serde_json::json!({
            "client_name": "LibreFang",
            "redirect_uris": [redirect_uri],
            "grant_types": ["authorization_code", "refresh_token"],
            "response_types": ["code"],
            "token_endpoint_auth_method": "none"
        });

        let resp = client
            .post(registration_endpoint)
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("Client registration request failed: {e}"))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(format!(
                "Client registration failed (HTTP {status}): {body}"
            ));
        }

        // We register as a public client (`token_endpoint_auth_method: "none"`),
        // so any `client_secret` the AS echoes back is intentionally ignored —
        // it must not be persisted or used in subsequent token exchanges.
        #[derive(serde::Deserialize)]
        struct RegistrationResponse {
            client_id: String,
            #[allow(dead_code)]
            #[serde(default)]
            client_secret: Option<String>,
        }

        let reg: RegistrationResponse = resp
            .json()
            .await
            .map_err(|e| format!("Failed to parse registration response: {e}"))?;

        Ok(reg.client_id)
    }
}

#[async_trait]
impl McpOAuthProvider for KernelOAuthProvider {
    async fn load_token(&self, server_url: &str) -> Option<String> {
        let access_token = self.vault_get(&Self::vault_key(server_url, "access_token"))?;

        // Check expiration if stored
        if let Some(expires_at_str) = self.vault_get(&Self::vault_key(server_url, "expires_at")) {
            if let Ok(expires_at) = expires_at_str.parse::<i64>() {
                let now = chrono::Utc::now().timestamp();
                if now >= expires_at - 60 {
                    debug!(server = %server_url, "MCP OAuth token expired or near expiry, attempting refresh");

                    if let Some(refresh_token) =
                        self.vault_get(&Self::vault_key(server_url, "refresh_token"))
                    {
                        match self.try_refresh(server_url, &refresh_token).await {
                            Ok(new_tokens) => {
                                if let Err(e) =
                                    self.store_tokens(server_url, new_tokens.clone()).await
                                {
                                    warn!(error = %e, "Failed to store refreshed tokens");
                                }
                                return Some(new_tokens.access_token);
                            }
                            Err(e) => {
                                warn!(error = %e, "Token refresh failed");
                                return None;
                            }
                        }
                    }
                    return None;
                }
            }
        }
        // No expires_at stored (e.g. Notion) — return token as-is
        Some(access_token)
    }

    async fn store_tokens(&self, server_url: &str, tokens: OAuthTokens) -> Result<(), String> {
        self.vault_set(
            &Self::vault_key(server_url, "access_token"),
            &tokens.access_token,
        )?;

        if let Some(ref rt) = tokens.refresh_token {
            self.vault_set(&Self::vault_key(server_url, "refresh_token"), rt)?;
        }

        if tokens.expires_in > 0 {
            let expires_at = chrono::Utc::now().timestamp() + tokens.expires_in as i64;
            self.vault_set(
                &Self::vault_key(server_url, "expires_at"),
                &expires_at.to_string(),
            )?;
        }

        debug!(server = %server_url, "MCP OAuth tokens stored in vault");
        Ok(())
    }

    async fn clear_tokens(&self, server_url: &str) -> Result<(), String> {
        // #3369: aggregate per-field failures and surface them. Returning Ok
        // when vault_remove failed lets the UI display "logged out" while the
        // refresh/access tokens still sit in the vault — daemon keeps using
        // them on the next request.
        let mut failures: Vec<String> = Vec::new();
        for field in ALL_VAULT_FIELDS {
            let key = Self::vault_key(server_url, field);
            if let Err(e) = self.vault_remove(&key) {
                warn!(server = %server_url, field = %field, error = %e, "MCP OAuth clear_tokens: vault_remove failed");
                failures.push(format!("{field}: {e}"));
            }
        }
        if !failures.is_empty() {
            return Err(format!(
                "Sign-out failed to fully clear vault for {server_url}; tokens may still be valid. Retry. Details: {}",
                failures.join("; ")
            ));
        }
        debug!(server = %server_url, "MCP OAuth tokens cleared from vault");
        Ok(())
    }
}

impl KernelOAuthProvider {
    /// Returns the canonical list of vault fields cleared by `clear_tokens`.
    /// Exposed for tests to assert exhaustiveness without a live vault.
    #[cfg(test)]
    pub(crate) fn clear_token_fields() -> &'static [&'static str] {
        ALL_VAULT_FIELDS
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vault_key_format() {
        let key = KernelOAuthProvider::vault_key("https://example.com/mcp", "access_token");
        assert_eq!(key, "mcp_oauth:https://example.com/mcp:access_token");
    }

    #[test]
    fn vault_key_refresh_token() {
        let key = KernelOAuthProvider::vault_key("https://example.com/mcp", "refresh_token");
        assert_eq!(key, "mcp_oauth:https://example.com/mcp:refresh_token");
    }

    #[test]
    fn vault_key_all_fields_namespaced() {
        let url = "https://mcp.notion.com/mcp";
        // All fields that should be cleaned up on delete — driven by ALL_VAULT_FIELDS
        for field in ALL_VAULT_FIELDS {
            let key = KernelOAuthProvider::vault_key(url, field);
            assert!(
                key.starts_with("mcp_oauth:"),
                "Key for '{}' should be prefixed with 'mcp_oauth:'",
                field
            );
            assert!(
                key.contains(url),
                "Key for '{}' should contain the server URL",
                field
            );
            assert!(
                key.ends_with(field),
                "Key for '{}' should end with the field name",
                field
            );
        }
    }

    #[test]
    fn vault_keys_are_isolated_per_server() {
        let key_a = KernelOAuthProvider::vault_key("https://server-a.com/mcp", "access_token");
        let key_b = KernelOAuthProvider::vault_key("https://server-b.com/mcp", "access_token");
        assert_ne!(
            key_a, key_b,
            "Different servers should have different vault keys"
        );
    }

    /// #3369: when vault_remove fails, clear_tokens MUST return Err so the
    /// API layer can tell the user sign-out is incomplete. Pre-fix, this
    /// returned Ok(()) and the UI showed "logged out" while the access token
    /// stayed in the vault.
    #[tokio::test]
    async fn clear_tokens_returns_err_when_vault_unlock_fails() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let home = tmp.path().to_path_buf();
        // Write a garbage vault.enc so unlock() fails for every vault_remove call.
        std::fs::write(home.join("vault.enc"), b"not-a-real-vault").expect("seed bad vault");
        // Provide a syntactically valid master key so unlock() reaches the
        // decrypt step and fails on the corrupt ciphertext rather than on a
        // missing key.
        unsafe {
            std::env::set_var(
                "LIBREFANG_VAULT_KEY",
                "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=",
            );
        }

        let provider = KernelOAuthProvider::new(home);
        let result = provider.clear_tokens("https://example.com/mcp").await;
        unsafe {
            std::env::remove_var("LIBREFANG_VAULT_KEY");
        }

        assert!(
            result.is_err(),
            "clear_tokens must propagate vault failures (#3369), got {:?}",
            result
        );
        let err = result.unwrap_err();
        assert!(
            err.to_lowercase().contains("retry") || err.to_lowercase().contains("sign-out"),
            "error message should prompt the caller to retry, got: {err}"
        );
    }

    #[test]
    fn clear_tokens_covers_all_stored_fields() {
        // Verifies that ALL_VAULT_FIELDS (used by clear_tokens) covers every field
        // that store_tokens or auth_start might write. If a new field is added to
        // those functions, add it to ALL_VAULT_FIELDS and this assertion will pass;
        // if it's forgotten in ALL_VAULT_FIELDS, this test will fail.
        let fields = KernelOAuthProvider::clear_token_fields();
        for expected in &[
            "access_token",
            "refresh_token",
            "expires_at",
            "token_endpoint",
            "client_id",
            "pkce_verifier",
            "pkce_state",
            "redirect_uri",
        ] {
            assert!(
                fields.contains(expected),
                "ALL_VAULT_FIELDS is missing '{}' — clear_tokens won't wipe it",
                expected
            );
        }
        assert_eq!(
            fields.len(),
            8,
            "Unexpected field count in ALL_VAULT_FIELDS — update this assertion if new fields are intentionally added"
        );
    }
}
