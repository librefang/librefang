//! Kernel-specific error types.

use librefang_hands::HandError;
use librefang_types::error::LibreFangError;
use thiserror::Error;

/// Kernel error type wrapping LibreFangError with kernel-specific context.
#[derive(Error, Debug)]
pub enum KernelError {
    /// A wrapped LibreFangError.
    #[error(transparent)]
    LibreFang(#[from] LibreFangError),

    /// A structured Hands-registry error.
    ///
    /// Restored as part of issue #3711 (1-of-21 slice): previously every
    /// `HandError` was stringified into `LibreFangError::Internal(String)`
    /// at the kernel boundary, losing the typed kind (`AlreadyActive`,
    /// `NotFound`, `BuiltinHand`, …). Carrying the typed variant lets
    /// upstream callers branch on it (e.g. surface 409 Conflict for
    /// `AlreadyActive` vs 500 for `Io`).
    #[error(transparent)]
    Hand(#[from] HandError),

    /// The kernel failed to boot.
    #[error("Boot failed: {0}")]
    BootFailed(String),

    /// A bounded internal channel was full and the request could not be
    /// enqueued. Callers (e.g. the API layer) should surface this as
    /// HTTP 503 Service Unavailable so clients know to retry, rather
    /// than silently dropping the request.
    #[error("Backpressure: {0}")]
    Backpressure(String),
}

/// Alias for kernel results.
pub type KernelResult<T> = Result<T, KernelError>;

#[cfg(test)]
mod tests {
    use super::*;

    /// Regression for #3711 (1-of-21 slice): a `HandError::AlreadyActive`
    /// surfaced through the kernel boundary must keep its typed kind, not
    /// be flattened to `LibreFangError::Internal(String)`. Upstream
    /// callers rely on this to differentiate 409 Conflict from a generic
    /// 500 Internal.
    #[test]
    fn hand_error_kind_survives_kernel_boundary() {
        let inner = HandError::AlreadyActive("clip".to_string());
        let kerr: KernelError = inner.into();
        match kerr {
            KernelError::Hand(HandError::AlreadyActive(id)) => assert_eq!(id, "clip"),
            other => panic!("expected KernelError::Hand(AlreadyActive), got {other:?}"),
        }
    }

    /// Regression for #3711: human-readable `Display` output must remain
    /// identical to the previous `LibreFangError::Internal(format!(...))`
    /// rendering so logs / UI surfaces don't shift. `#[error(transparent)]`
    /// on `KernelError::Hand` delegates to `HandError`'s own Display, which
    /// already produces "Hand already active: {id}" — the exact string the
    /// pre-refactor code constructed by hand.
    #[test]
    fn hand_error_display_is_unchanged() {
        let kerr: KernelError = HandError::AlreadyActive("clip".to_string()).into();
        assert_eq!(format!("{kerr}"), "Hand already active: clip");

        let kerr: KernelError = HandError::NotFound("missing".to_string()).into();
        assert_eq!(format!("{kerr}"), "Hand not found: missing");
    }
}
