//! Shared test harness for Phase-6 plugin example integration tests.
//!
//! Loaded by each `tests/plugin_example_*.rs` via:
//!     #[path = "support/plugin_example_harness.rs"]
//!     #[allow(dead_code)]
//!     mod support;
//!
//! The `#[allow(dead_code)]` on the `mod support` declaration silences
//! per-binary dead-code warnings: each test exercises a different
//! capability (fs / kv / env / time / none) and uses a different subset
//! of the harness, so "unused trait impl" warnings here are intrinsic,
//! not a real signal.
//!
//! Provides:
//! - `KernelHandleStub` — minimal `KernelHandle` implementation with an
//!   in-memory KV store for the `MemoryAccess` role trait. All other role
//!   traits use the defaults from `librefang-kernel-handle` (most return
//!   `KernelOpError::unavailable`).
//! - `wasm_bytes(dir)` — reads `examples/plugins/<dir>/pre-built/plugin.wasm`
//!   relative to the workspace root, panicking cleanly on missing files so
//!   test failure messages point at the pre-built slot.

// Phase-8 C-005: KernelHandleStub promoted to librefang-kernel-handle.
// Re-export from the canonical location so existing test code continues to
// compile with `use support::KernelHandleStub` unchanged. The `#[allow]`
// silences per-binary "unused import" warnings — each test binary uses a
// different subset of the harness (only js-kv-counter needs KernelHandleStub).
#[allow(unused_imports)]
pub use librefang_kernel_handle::test_stub::KernelHandleStub;

// ---------------------------------------------------------------------------
// Workspace helper
// ---------------------------------------------------------------------------

/// Locate the workspace root by walking up from CARGO_MANIFEST_DIR until
/// a directory containing Cargo.toml with `[workspace]` is found, or
/// until the Cargo.toml at the crate level itself is the root. In CI
/// the env var CARGO_MANIFEST_DIR reliably points at `crates/librefang-runtime/`.
pub fn workspace_root() -> std::path::PathBuf {
    // The test binary's CARGO_MANIFEST_DIR is always `crates/<crate>/`.
    // Walking up two levels gives the workspace root.
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_owned()
}

/// Read `examples/plugins/<dir>/pre-built/plugin.wasm` from the workspace root.
pub fn wasm_bytes(example_dir: &str) -> Vec<u8> {
    let path = workspace_root()
        .join("examples/plugins")
        .join(example_dir)
        .join("pre-built/plugin.wasm");
    std::fs::read(&path).unwrap_or_else(|e| {
        panic!(
            "Cannot read pre-built wasm for example '{}' at {}: {e}\n\
             Run: cargo xtask plugins-rebuild {example_dir}",
            example_dir,
            path.display()
        )
    })
}
