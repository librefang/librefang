//! Kernel-specific error types.

use librefang_hands::HandError;
use librefang_runtime::sandbox::SandboxError;
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

    /// A structured WASM-sandbox error.
    ///
    /// Restored as part of issue #3711 (2-of-21 slice): previously every
    /// `SandboxError` raised by `WasmSandbox::execute` was stringified
    /// into `LibreFangError::Internal(format!("WASM execution failed: {e}"))`
    /// at the kernel boundary, losing the typed kind (`FuelExhausted`,
    /// `Compilation`, `AbiError`, …). Carrying the typed variant lets
    /// upstream callers branch on it (e.g. surface 408/quota for
    /// `FuelExhausted` vs 500 for `Execution`) without string matching.
    ///
    /// The Display prefix `"WASM execution failed: "` is preserved
    /// byte-for-byte to keep existing log/UI output identical.
    #[error("WASM execution failed: {0}")]
    WasmSandbox(#[from] SandboxError),

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

    /// Regression for #3711 (slice 1 follow-up): the additional collapse
    /// sites migrated in `deactivate_hand`, `pause_hand`, `resume_hand`,
    /// `set_agents`, `merge_agent_runtime_override`,
    /// `clear_agent_runtime_override`, and `persist_hand_state_result`
    /// most commonly surface `HandError::InstanceNotFound(Uuid)` when the
    /// caller passes a stale id. Before the migration the boundary
    /// rendered this as `LibreFangError::Internal("Hand instance not
    /// found: <uuid>")` via `e.to_string()`. With `KernelError::Hand`
    /// `#[error(transparent)]` the rendering must be byte-identical, and
    /// the typed variant must survive so upstream can map it to 404.
    #[test]
    fn hand_error_instance_not_found_survives_and_displays_unchanged() {
        let id = uuid::Uuid::nil();
        let inner = HandError::InstanceNotFound(id);
        let prev_collapsed_display = inner.to_string();
        let kerr: KernelError = inner.into();
        // Display preserved (matches the pre-refactor `e.to_string()`).
        assert_eq!(format!("{kerr}"), prev_collapsed_display);
        // Typed variant preserved across the boundary.
        match kerr {
            KernelError::Hand(HandError::InstanceNotFound(got)) => assert_eq!(got, id),
            other => panic!("expected KernelError::Hand(InstanceNotFound), got {other:?}"),
        }
    }

    /// Regression for #3711 (2-of-21 slice): a `SandboxError::FuelExhausted`
    /// surfaced through the kernel boundary must keep its typed kind, not
    /// be flattened to `LibreFangError::Internal(String)`. Upstream callers
    /// rely on this to distinguish a CPU-budget overrun (recoverable —
    /// surface to the user as a quota error) from a generic execution
    /// failure (500).
    #[test]
    fn sandbox_error_kind_survives_kernel_boundary() {
        let inner = SandboxError::FuelExhausted;
        let kerr: KernelError = inner.into();
        match kerr {
            KernelError::WasmSandbox(SandboxError::FuelExhausted) => {}
            other => panic!("expected KernelError::WasmSandbox(FuelExhausted), got {other:?}"),
        }

        let inner = SandboxError::AbiError("bad export".to_string());
        let kerr: KernelError = inner.into();
        assert!(matches!(
            kerr,
            KernelError::WasmSandbox(SandboxError::AbiError(_))
        ));
    }

    /// Regression for #3711: human-readable `Display` output must remain
    /// byte-identical to the previous
    /// `LibreFangError::Internal(format!("WASM execution failed: {e}"))`
    /// rendering so logs / UI surfaces don't shift. The `#[error("WASM
    /// execution failed: {0}")]` attribute on `KernelError::WasmSandbox`
    /// reproduces the old prefix, while `SandboxError`'s own Display
    /// supplies the variant-specific tail.
    #[test]
    fn sandbox_error_display_is_unchanged() {
        let kerr: KernelError = SandboxError::FuelExhausted.into();
        assert_eq!(
            format!("{kerr}"),
            "WASM execution failed: Fuel exhausted: skill exceeded CPU budget"
        );

        let kerr: KernelError = SandboxError::Compilation("bad opcode".to_string()).into();
        assert_eq!(
            format!("{kerr}"),
            "WASM execution failed: WASM compilation failed: bad opcode"
        );
    }
}
