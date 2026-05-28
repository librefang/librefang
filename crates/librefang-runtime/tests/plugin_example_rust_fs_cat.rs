//! Integration test for Phase-6 C-002: `rust-fs-cat` plugin example.
//!
//! The plugin reads `/tmp/test-input.txt` and writes `/tmp/test-output.txt`
//! via the `librefang:plugin/fs` interface. The test:
//!   1. Writes a known payload to `/tmp/test-input.txt`.
//!   2. Grants `FileRead` + `FileWrite` sandbox capabilities for those paths.
//!   3. Wires `HostCapability::Fs` so the `fs` interface is linked.
//!   4. Runs the component and asserts `Ok`.
//!   5. Reads `/tmp/test-output.txt` and asserts it matches the payload.

mod support {
    include!("support/plugin_example_harness.rs");
}
use support::wasm_bytes;

use librefang_runtime::sandbox::{SandboxConfig, WasmSandbox};
use librefang_runtime::sandbox_component::ComponentExecuteOptions;
use librefang_skills::HostCapability;
use librefang_types::capability::Capability;

const INPUT_PATH: &str = "/tmp/test-input.txt";
const OUTPUT_PATH: &str = "/tmp/test-output.txt";
const PAYLOAD: &str = "hello from rust-fs-cat integration test\n";

#[tokio::test]
async fn rust_fs_cat_copies_file() {
    // Arrange: seed the input file.
    std::fs::write(INPUT_PATH, PAYLOAD).expect("write test-input.txt");
    // Clean up output from any prior run.
    let _ = std::fs::remove_file(OUTPUT_PATH);

    let bytes = wasm_bytes("rust-fs-cat");
    let sandbox = WasmSandbox::new().expect("WasmSandbox::new");

    // Grant fine-grained FileRead + FileWrite capabilities for the exact paths
    // the plugin accesses. The fs dispatch canonicalises paths before checking,
    // so the capability must match the canonical (real) path.
    let config = SandboxConfig {
        capabilities: vec![
            Capability::FileRead(INPUT_PATH.into()),
            Capability::FileWrite(OUTPUT_PATH.into()),
        ],
        ..Default::default()
    };

    let result = sandbox
        .execute_component(
            &bytes,
            serde_json::json!({}),
            config,
            None,
            "test-agent",
            ComponentExecuteOptions {
                host_capabilities: vec![HostCapability::Fs],
                ..Default::default()
            },
        )
        .await;

    assert!(
        result.is_ok(),
        "rust-fs-cat plugin run should succeed; got: {result:?}"
    );

    // Verify the side effect: output file contains the same payload.
    let actual = std::fs::read_to_string(OUTPUT_PATH).expect("read test-output.txt");
    assert_eq!(
        actual, PAYLOAD,
        "rust-fs-cat should copy input → output verbatim"
    );
}
