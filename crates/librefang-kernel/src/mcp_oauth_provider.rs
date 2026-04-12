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
        if vault.unlock().is_err() {
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

        let client_id = self.vault_get(&Self::vault_key(server_url, "client_id"));

        let client = reqwest::Client::new();
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
        let client = reqwest::Client::new();

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
        for field in &[
            "access_token",
            "refresh_token",
            "expires_at",
            "token_endpoint",
            "client_id",
        ] {
            let _ = self.vault_remove(&Self::vault_key(server_url, field));
        }
        debug!(server = %server_url, "MCP OAuth tokens cleared from vault");
        Ok(())
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
}
