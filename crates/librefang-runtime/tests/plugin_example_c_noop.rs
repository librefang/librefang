//! Integration test for Phase-6 C-006: `c-noop` plugin example.
//!
//! Loads `examples/plugins/c-noop/pre-built/plugin.wasm` and invokes
//! `run()`. The plugin declares no host capabilities and always returns
//! `Ok(())`, so the test verifies the end-to-end Component Model load +
//! execute path with the capability gate in its most minimal configuration.

#[path = "support/plugin_example_harness.rs"]
#[allow(dead_code)]
mod support;
use support::wasm_bytes;

use librefang_runtime::sandbox::WasmSandbox;
use librefang_runtime::sandbox_component::ComponentExecuteOptions;

#[tokio::test]
async fn c_noop_returns_ok() {
    let bytes = wasm_bytes("c-noop");
    let sandbox = WasmSandbox::new().expect("WasmSandbox::new");

    let result = sandbox
        .execute_component(
            &bytes,
            serde_json::json!({}),
            Default::default(),
            None, // no kernel handle needed
            "test-agent",
            ComponentExecuteOptions {
                host_capabilities: vec![], // c-noop declares no imports
                ..Default::default()
            },
        )
        .await;

    assert!(
        result.is_ok(),
        "c-noop plugin run should succeed; got: {result:?}"
    );
}
