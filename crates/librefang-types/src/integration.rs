//! Outcome and error types for the MCP-integration install façade.
//!
//! `KernelApi::install_integration` (the HTTP-layer trait surface in
//! `librefang-kernel`) used to return
//! `librefang_extensions::ExtensionResult<librefang_extensions::installer::InstallResult>`,
//! which forced every reimplementer of the trait — mocks, alternate kernels —
//! to depend on `librefang-extensions` even when they had no other reason to.
//! `IntegrationError` (the error half, #5622) and `IntegrationOutcome` (the
//! success half) both live here, in the dependency-free types crate, so the
//! trait can speak a shared vocabulary while the concrete kernel impl keeps
//! converting from its internal `ExtensionError` / `InstallResult` via `From`
//! (the conversion impls live in `librefang-extensions`).
//!
//! The variants intentionally preserve the discriminants the API layer maps
//! to HTTP status codes (`NotFound` → 404; everything else → 500), so the
//! switch from `ExtensionError` does not silently change response shapes.

use crate::config::McpServerConfigEntry;
use crate::mcp::McpStatus;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Success outcome of the kernel's MCP-integration install façade.
///
/// `KernelApi::install_integration` used to return
/// `librefang_extensions::installer::InstallResult` on the `Ok` side, which —
/// like the error half before #5622 — forced every reimplementer of the trait
/// (mocks, alternate kernels) to depend on `librefang-extensions` even when
/// they need nothing else from it. `IntegrationOutcome` lives here, in the
/// dependency-free types crate, and mirrors `InstallResult`'s public fields
/// one-for-one. The concrete kernel impl converts via the
/// `From<InstallResult>` bridge defined in `librefang-extensions`;
/// reimplementers that don't depend on that crate construct this struct
/// directly.
///
/// All field types (`McpServerConfigEntry`, `McpStatus`) already live in
/// `librefang-types`, so this introduces no new dependency on the extensions
/// crate.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IntegrationOutcome {
    /// MCP server id (matches `McpServerConfigEntry.name`).
    pub id: String,
    /// The `[[mcp_servers]]` entry the caller should persist into config.toml.
    pub server: McpServerConfigEntry,
    /// Final status of the installed integration.
    pub status: McpStatus,
    /// Names of required env vars that still have no credential.
    pub missing_credentials: Vec<String>,
    /// Message to display to the user.
    pub message: String,
}

/// Failure surfaced by the kernel's MCP-integration install façade.
///
/// Mirrors the subset of the extensions-crate error space that can reach the
/// trait boundary. `librefang-extensions` provides
/// `impl From<ExtensionError> for IntegrationError` so the real kernel impl
/// converts cleanly; reimplementers that don't depend on the extensions crate
/// construct these variants directly.
#[derive(Error, Debug)]
#[non_exhaustive]
pub enum IntegrationError {
    /// The requested MCP catalog template id was not found.
    #[error("MCP catalog entry not found: {0}")]
    NotFound(String),

    /// An MCP server with this id / name is already configured.
    #[error("MCP server already configured: {0}")]
    AlreadyInstalled(String),

    /// The credential vault was unavailable or rejected the operation.
    #[error("Vault error: {0}")]
    Vault(String),

    /// Any other install failure (IO, parse, HTTP, health-check, …) that does
    /// not need its own discriminant at the trait boundary. The full message
    /// is preserved for the operator-facing response.
    #[error("Install failed: {0}")]
    Other(String),
}

/// Convenience `Result` alias over [`IntegrationError`], mirroring the shape
/// of the extensions-crate `ExtensionResult` the trait used to return.
pub type IntegrationResult<T> = Result<T, IntegrationError>;

#[cfg(test)]
mod tests {
    use super::*;

    /// `Display` strings are part of the operator-facing API contract (the
    /// `install_integration` route renders them straight into the JSON
    /// `error` field). Pin them so a refactor can't silently shift response
    /// text.
    #[test]
    fn display_strings_are_stable() {
        assert_eq!(
            IntegrationError::NotFound("github".into()).to_string(),
            "MCP catalog entry not found: github"
        );
        assert_eq!(
            IntegrationError::AlreadyInstalled("slack".into()).to_string(),
            "MCP server already configured: slack"
        );
        assert_eq!(
            IntegrationError::Vault("locked".into()).to_string(),
            "Vault error: locked"
        );
        assert_eq!(
            IntegrationError::Other("502".into()).to_string(),
            "Install failed: 502"
        );
    }

    /// Acceptance for the typed-error refactor: a reimplementer of the
    /// kernel's `install_integration` façade can model the contract's error
    /// half and key HTTP status off the discriminant using
    /// only `librefang-types` — no `librefang-extensions` dependency. This
    /// crate has no dependency on `librefang-extensions`, so the fact that
    /// this test compiles and runs *is* the proof.
    #[test]
    fn error_is_usable_without_extensions_crate() {
        // Stand in for a mock kernel's install path returning the typed error.
        fn mock_install(template_id: &str) -> IntegrationResult<()> {
            if template_id == "github" {
                Ok(())
            } else {
                Err(IntegrationError::NotFound(template_id.to_string()))
            }
        }

        assert!(mock_install("github").is_ok());

        let err = mock_install("does-not-exist").unwrap_err();
        // The discriminant the API layer maps to HTTP 404 survives.
        assert!(matches!(err, IntegrationError::NotFound(ref s) if s == "does-not-exist"));
    }

    /// Acceptance for the success-type refactor: a reimplementer of the
    /// kernel's `install_integration` façade can construct the full success
    /// outcome — including the fields the API handler reads (`server`, `id`) —
    /// using only `librefang-types`, with no `librefang-extensions`
    /// dependency. This crate has no dependency on `librefang-extensions`, so
    /// the fact that this test compiles and runs *is* the proof.
    #[test]
    fn outcome_is_constructible_without_extensions_crate() {
        use crate::config::{McpServerConfigEntry, McpTransportEntry};
        use crate::mcp::McpStatus;

        // Stand in for a mock kernel's install path returning the typed
        // success outcome.
        fn mock_install(template_id: &str) -> IntegrationResult<IntegrationOutcome> {
            if template_id != "github" {
                return Err(IntegrationError::NotFound(template_id.to_string()));
            }
            let server = McpServerConfigEntry {
                name: template_id.to_string(),
                template_id: Some(template_id.to_string()),
                transport: Some(McpTransportEntry::Stdio {
                    command: "github-mcp".to_string(),
                    args: vec![],
                }),
                timeout_secs: 30,
                env: vec![],
                headers: vec![],
                oauth: None,
                taint_scanning: true,
                taint_policy: None,
            };
            Ok(IntegrationOutcome {
                id: template_id.to_string(),
                server,
                status: McpStatus::Ready,
                missing_credentials: vec![],
                message: "github added.".to_string(),
            })
        }

        let outcome = mock_install("github").expect("install should succeed");
        // The two fields the API handler reads off the success outcome.
        assert_eq!(outcome.id, "github");
        assert_eq!(outcome.server.name, "github");
        assert_eq!(outcome.status, McpStatus::Ready);
        assert!(outcome.missing_credentials.is_empty());

        assert!(mock_install("nope").is_err());
    }
}
