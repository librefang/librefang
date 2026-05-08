//! LibreFang Extensions — MCP server catalog, credential vault, OAuth, health.
//!
//! This crate provides:
//! - **MCP Catalog**: read-only set of MCP server templates (GitHub, Slack, ...)
//!   cached at `~/.librefang/mcp/catalog/*.toml` and refreshed by `registry_sync`.
//! - **Credential Vault**: AES-256-GCM encrypted storage with OS keyring support
//! - **OAuth2 PKCE**: Localhost callback flows for Google/GitHub/Microsoft/Slack
//! - **Health Monitor**: Auto-reconnect with exponential backoff
//! - **Installer**: Pure transforms from a catalog entry to a new
//!   `McpServerConfigEntry` that the kernel can wire up.
//!
//! Installed MCP servers no longer live in a separate `integrations.toml`;
//! every configured server is an `[[mcp_servers]]` entry in
//! `~/.librefang/config.toml`. An optional `template_id` field records the
//! catalog entry it was installed from.
//!
//! Schema for catalog entries, transports, categories, statuses, and OAuth
//! templates lives in [`librefang_types::mcp`] and [`librefang_types::oauth`]
//! — this crate owns the *behaviour* (loading, installing, monitoring) only.

pub mod catalog;
pub mod credentials;
pub mod dotenv;
pub mod health;
pub mod http_client;
pub mod installer;
pub mod oauth;
pub mod vault;

// ─── Error types ─────────────────────────────────────────────────────────────

#[derive(Debug, thiserror::Error)]
pub enum ExtensionError {
    #[error("MCP catalog entry not found: {0}")]
    NotFound(String),
    #[error("MCP server already configured: {0}")]
    AlreadyInstalled(String),
    #[error("MCP server not configured: {0}")]
    NotInstalled(String),
    #[error("Credential not found: {0}")]
    CredentialNotFound(String),
    #[error("Vault error: {0}")]
    Vault(String),
    #[error("Vault locked — unlock with vault key or LIBREFANG_VAULT_KEY env var")]
    VaultLocked,
    /// The vault was opened with a key that does not match the key it was
    /// encrypted with. Surfaced from #3651: pre-fix the daemon would silently
    /// boot, then every subsequent vault read would error with a generic
    /// "Decryption failed" log line — the operator never learned the root
    /// cause was a mismatched `LIBREFANG_VAULT_KEY`.
    ///
    /// `hint` carries the recovery instruction for the operator (typically
    /// "restore the original env var, or rebuild from backup"). The
    /// boot-path translates this into a `KernelError::BootFailed` so the
    /// daemon refuses to start instead of corrupting downstream state.
    #[error("Vault key mismatch: {hint}")]
    VaultKeyMismatch { hint: String },
    #[error("OAuth error: {0}")]
    OAuth(String),
    #[error("TOML parse error: {0}")]
    TomlParse(String),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("HTTP error: {0}")]
    Http(String),
    #[error("Health check failed: {0}")]
    HealthCheck(String),
}

pub type ExtensionResult<T> = Result<T, ExtensionError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_display() {
        let err = ExtensionError::NotFound("github".to_string());
        assert!(err.to_string().contains("github"));
        let err = ExtensionError::VaultLocked;
        assert!(err.to_string().contains("vault"));
    }
}
