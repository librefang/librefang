//! `set_self_handle` is a caller obligation, not something `boot` performs.
//!
//! `LibreFangKernel::boot` / `boot_with_config` return a bare `LibreFangKernel`;
//! the `self_handle` slot stays empty until a caller wraps the kernel in an
//! `Arc` and invokes `set_self_handle` on it. Seven production sites do
//! (`librefang-api/src/server.rs` ×3, `librefang-cli/src/tui/event.rs`,
//! `librefang-desktop/src/server.rs`, and the two `librefang-testing`
//! harnesses); the CLI's ACP and in-process MCP backends did not, and every
//! path from there into `kernel_handle()` aborts the process on an `.expect`.
//!
//! These tests pin the contract in both directions so the next surface that
//! boots a kernel has an executable statement of what it owes.

use librefang_kernel::LibreFangKernel;
use librefang_types::config::{KernelConfig, MemoryConfig};
use std::sync::Arc;

fn boot_arc() -> (Arc<LibreFangKernel>, tempfile::TempDir) {
    let tmp = tempfile::tempdir().expect("failed to create temp directory");
    let home_dir = tmp.path().to_path_buf();
    let data_dir = home_dir.join("data");
    std::fs::create_dir_all(&data_dir).expect("failed to create data directory");
    std::fs::create_dir_all(home_dir.join("skills")).unwrap();
    std::fs::create_dir_all(home_dir.join("workspaces").join("agents")).unwrap();
    std::fs::create_dir_all(home_dir.join("workspaces").join("hands")).unwrap();

    let config = KernelConfig {
        home_dir,
        data_dir: data_dir.clone(),
        network_enabled: false,
        memory: MemoryConfig {
            sqlite_path: Some(data_dir.join("test.db")),
            ..Default::default()
        },
        ..KernelConfig::default()
    };

    let kernel = LibreFangKernel::boot_with_config(config).expect("failed to boot test kernel");
    (Arc::new(kernel), tmp)
}

/// The failure the ACP and MCP backends hit.
///
/// `boot` does not populate `self_handle`, so `kernel_handle()` reaches its
/// `.expect` and aborts. This is what `KernelAdapter::new` did at ACP startup,
/// and what `send_message` did on the first in-process MCP tool call.
#[test]
#[should_panic(expected = "kernel self_handle accessed before set_self_handle")]
fn kernel_handle_panics_when_boot_is_not_followed_by_set_self_handle() {
    let (kernel, _tmp) = boot_arc();
    let _ = kernel.kernel_handle();
}

/// The obligation discharged: one `set_self_handle` after the `Arc` wrap is all
/// a new surface needs.
#[test]
fn kernel_handle_resolves_once_set_self_handle_has_run() {
    let (kernel, _tmp) = boot_arc();
    kernel.set_self_handle();
    let _handle = kernel.kernel_handle();
}

/// `set_self_handle` is idempotent, which is what makes it safe for a surface
/// to call defensively without coordinating with whatever booted the kernel.
///
/// The slot is a `OnceLock`, and the hook registrations inside are gated on
/// that first-call signal — so a second call is a no-op rather than a
/// double-registration of the `AgentLoopEnd` hooks.
#[test]
fn set_self_handle_is_idempotent() {
    let (kernel, _tmp) = boot_arc();
    kernel.set_self_handle();
    let first = kernel.kernel_handle();
    kernel.set_self_handle();
    let second = kernel.kernel_handle();
    assert!(
        Arc::ptr_eq(&first, &second),
        "the second call must not replace the handle"
    );
}
