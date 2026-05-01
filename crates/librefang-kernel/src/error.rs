//! Kernel-specific error types.

use librefang_types::error::LibreFangError;
use thiserror::Error;

/// Kernel error type wrapping LibreFangError with kernel-specific context.
#[derive(Error, Debug)]
pub enum KernelError {
    /// A wrapped LibreFangError.
    #[error(transparent)]
    LibreFang(#[from] LibreFangError),

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
