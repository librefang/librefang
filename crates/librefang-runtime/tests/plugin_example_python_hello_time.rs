//! Integration test for Phase-6 C-003: `python-hello-time` plugin example.
//!
//! The plugin calls `time.now()` (always allowed — no capability gate on
//! the host side) and returns `Ok(())`. The test verifies the full CPython-
//! embedded component loads and executes successfully.
//!
//! Size note: componentize-py bundles CPython (~18 MB). Wasmtime JIT-compiles
//! this on first load; the test may take 10–30 s depending on the machine.
//! The pre-built artefact is committed; if absent, the test skips with a
//! message rather than failing.

#[path = "support/plugin_example_harness.rs"]
#[allow(dead_code)]
mod support;
use support::{wasm_bytes, workspace_root};

use librefang_runtime::sandbox::WasmSandbox;
use librefang_runtime::sandbox_component::ComponentExecuteOptions;
use librefang_skills::HostCapability;

#[tokio::test]
async fn python_hello_time_returns_ok() {
    // Skip if the pre-built wasm is absent (e.g., componentize-py not installed).
    let wasm_path =
        workspace_root().join("examples/plugins/python-hello-time/pre-built/plugin.wasm");
    if !wasm_path.exists() {
        eprintln!(
            "SKIP python_hello_time_returns_ok: {} not found; \
             run `componentize-py --wit-path ... componentize app -o ...` to build.",
            wasm_path.display()
        );
        return;
    }

    let bytes = wasm_bytes("python-hello-time");
    let sandbox = WasmSandbox::new().expect("WasmSandbox::new");

    let result = sandbox
        .execute_component(
            &bytes,
            serde_json::json!({}),
            Default::default(),
            None,
            "test-agent",
            ComponentExecuteOptions {
                // time_now has no host-side capability check but the
                // librefang:plugin/time interface must be linked.
                host_capabilities: vec![HostCapability::Time],
                ..Default::default()
            },
        )
        .await;

    assert!(
        result.is_ok(),
        "python-hello-time plugin run should succeed; got: {result:?}"
    );
}
