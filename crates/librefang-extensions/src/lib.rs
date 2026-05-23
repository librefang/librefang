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
    /// boot-path translates this into a `LibreFangError::BootFailed` so the
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

/// Bridge the extensions-crate error space to the dependency-free
/// [`librefang_types::integration::IntegrationError`] that the kernel's
/// HTTP-layer `install_integration` façade now returns. Keeping the
/// conversion here lets the real kernel impl `?`/`map_err` cleanly while
/// reimplementers of the trait (mocks, alternate kernels) construct
/// `IntegrationError` directly without ever touching this crate.
///
/// The mapping preserves the discriminant the API layer keys HTTP status
/// codes off (`NotFound` → 404; everything else → 500). Variants that don't
/// have a direct counterpart collapse into `IntegrationError::Other`, whose
/// `Display` carries the original `ExtensionError` message so operator-facing
/// responses are unchanged.
impl From<ExtensionError> for librefang_types::integration::IntegrationError {
    fn from(err: ExtensionError) -> Self {
        use librefang_types::integration::IntegrationError as IE;
        match err {
            ExtensionError::NotFound(s) => IE::NotFound(s),
            ExtensionError::AlreadyInstalled(s) => IE::AlreadyInstalled(s),
            ExtensionError::Vault(s) => IE::Vault(s),
            // `VaultLocked` / `VaultKeyMismatch` are still vault failures at
            // the trait boundary — fold them into `Vault`, carrying each
            // variant's own `Display` message so the operator-facing text is
            // unchanged.
            err @ (ExtensionError::VaultLocked | ExtensionError::VaultKeyMismatch { .. }) => {
                IE::Vault(err.to_string())
            }
            other => IE::Other(other.to_string()),
        }
    }
}

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

    /// The `From<ExtensionError>` bridge must preserve the `NotFound`
    /// discriminant the API layer keys its 404 response off, fold the vault
    /// family into `IntegrationError::Vault`, and collapse everything else
    /// into `Other` while keeping the original `Display` message.
    #[test]
    fn extension_error_maps_to_integration_error() {
        use librefang_types::integration::IntegrationError as IE;

        let mapped: IE = ExtensionError::NotFound("github".to_string()).into();
        assert!(matches!(mapped, IE::NotFound(ref s) if s == "github"));

        let mapped: IE = ExtensionError::AlreadyInstalled("slack".to_string()).into();
        assert!(matches!(mapped, IE::AlreadyInstalled(ref s) if s == "slack"));

        let mapped: IE = ExtensionError::Vault("disk full".to_string()).into();
        assert!(matches!(mapped, IE::Vault(ref s) if s == "disk full"));

        // VaultLocked folds into Vault, carrying its own Display message.
        let mapped: IE = ExtensionError::VaultLocked.into();
        assert!(matches!(&mapped, IE::Vault(s) if s.contains("vault")));

        // A variant with no direct counterpart collapses into Other but keeps
        // the original message verbatim.
        let original = ExtensionError::Http("502 from registry".to_string());
        let original_msg = original.to_string();
        let mapped: IE = original.into();
        match mapped {
            IE::Other(s) => assert_eq!(s, original_msg),
            other => panic!("expected Other, got {other:?}"),
        }
    }
}
