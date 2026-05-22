//! Error type for the MCP-integration install façade.
//!
//! `KernelApi::install_integration` (the HTTP-layer trait surface in
//! `librefang-kernel`) used to return `librefang_extensions::ExtensionResult`,
//! which forced every reimplementer of the trait — mocks, alternate kernels —
//! to depend on `librefang-extensions` even when they had no other reason to.
//! `IntegrationError` lives here, in the dependency-free types crate, so the
//! trait can speak a shared error vocabulary while the concrete kernel impl
//! keeps converting from its internal `ExtensionError` via `From` (the
//! conversion impl lives in `librefang-extensions`).
//!
//! The variants intentionally preserve the discriminants the API layer maps
//! to HTTP status codes (`NotFound` → 404; everything else → 500), so the
//! switch from `ExtensionError` does not silently change response shapes.

use thiserror::Error;

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
}
