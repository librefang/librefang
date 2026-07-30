//! Regression guard: every CLI surface that boots its own kernel must call
//! `set_self_handle` on it (#6651).
//!
//! `LibreFangKernel::boot` returns a bare kernel with an empty `self_handle`
//! slot, and `kernel_handle()` is an `.expect` that aborts the process when the
//! slot is empty. The ACP backend hit that at startup (`KernelAdapter::new`
//! resolves the handle immediately) and the in-process MCP backend hit it on the
//! first `librefang_agent_*` tool call (`send_message` resolves it to plumb
//! kernel tools into the turn).
//!
//! The kernel-side contract is pinned in
//! `librefang-kernel/tests/self_handle_bootstrap_test.rs`. That test proves
//! `kernel_handle()` panics without the call — it does **not** prove these two
//! files make it, so deleting either line would leave the whole kernel suite
//! green while reintroducing the abort. This guard closes that gap at the
//! injection site, per the repo's own rule about testing where the wiring is
//! rather than only where the mechanism is.
//!
//! Source-scanning rather than executing: `run_acp_server` and
//! `create_backend` are `fn(…) -> !`-shaped in practice — they `std::process::exit`
//! on failure, block on their own runtime, and speak a stdio protocol — so
//! neither can be driven from a unit test without restructuring code this fix
//! deliberately leaves alone. The same technique and comment-stripping already
//! guard `build.rs` in this crate (`build_rs_no_git_mutation.rs`, #3641).

use std::path::PathBuf;

/// Files that boot a kernel in-process and therefore owe the call.
///
/// Add an entry when a new CLI surface boots its own kernel; do not remove one
/// without checking that the surface no longer calls `LibreFangKernel::boot`.
const IN_PROCESS_BACKENDS: &[&str] = &["src/acp.rs", "src/mcp.rs", "src/tui/event.rs"];

fn read_source(rel: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(rel);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("failed to read {}: {e}", path.display()))
}

/// Drop `//` comments so the explanatory notes beside each call — which name
/// `set_self_handle` — cannot satisfy the assertion on their own.
fn strip_comments(src: &str) -> String {
    let mut out = String::with_capacity(src.len());
    for line in src.lines() {
        let cleaned = match line.find("//") {
            Some(idx) => &line[..idx],
            None => line,
        };
        out.push_str(cleaned);
        out.push('\n');
    }
    out
}

#[test]
fn every_in_process_backend_calls_set_self_handle() {
    for rel in IN_PROCESS_BACKENDS {
        let src = strip_comments(&read_source(rel));
        assert!(
            src.contains("set_self_handle()"),
            "{rel} boots a kernel but never calls `set_self_handle` — \
             `kernel_handle()` is an `.expect`, so every agent-turn path from \
             this surface aborts the process (#6651). Call it on the `Arc` \
             right after the wrap; it is idempotent."
        );
    }
}

/// The guard above is only meaningful while these files actually boot a kernel.
///
/// If a surface is refactored to receive an already-initialised kernel instead,
/// the assertion would keep passing on a stale premise — this catches that and
/// forces the list to be updated deliberately.
#[test]
fn the_guarded_files_still_boot_their_own_kernel() {
    for rel in IN_PROCESS_BACKENDS {
        let src = strip_comments(&read_source(rel));
        assert!(
            src.contains("LibreFangKernel::boot"),
            "{rel} no longer boots its own kernel, so it no longer owes \
             `set_self_handle`. Remove it from IN_PROCESS_BACKENDS rather than \
             leaving a guard that asserts nothing."
        );
    }
}
