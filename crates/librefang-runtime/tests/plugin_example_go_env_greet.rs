//! Integration test for Phase-6 C-005: `go-env-greet` plugin example.
//!
//! The plugin reads the `GREETING_NAME` environment variable via the
//! `librefang:plugin/env` interface. The test:
//!   1. Sets `GREETING_NAME=LibreFang` in the current process env.
//!   2. Grants `EnvRead("GREETING_NAME")` sandbox capability.
//!   3. Wires `HostCapability::Env` so the `env` interface is linked.
//!   4. Runs the component and asserts `Ok`.
//!
//! The plugin succeeds whether or not the var is present — it does not
//! propagate env-read absence as an error. The capability gate itself is
//! the behavioural contract tested here.

#[path = "support/plugin_example_harness.rs"]
#[allow(dead_code)]
mod support;
use support::{wasm_bytes, workspace_root};

use librefang_runtime::sandbox::{SandboxConfig, WasmSandbox};
use librefang_runtime::sandbox_component::ComponentExecuteOptions;
use librefang_skills::HostCapability;
use librefang_types::capability::Capability;

// Phase-7 known limitation: go-env-greet's pre-built plugin.wasm was compiled
// with TinyGo wasip1 command semantics (-buildmode=default, _start only).
// Rebuilding with -buildmode=c-shared produces _initialize, but TinyGo's
// asyncify goroutine scheduler panics during _initialize when run via the
// component-model sandbox (runtime.scanConservative GC fault). With
// -scheduler=none the binary builds correctly, but the bundled WASI P1
// reactor adapter (from wit-bindgen-cli, 94 KB) fails to bridge
// clock_time_get during _initialize — a version mismatch vs the wasmtime v45
// release adapter (52 KB). Requires fetching wasi_snapshot_preview1.reactor.wasm
// from the wasmtime v45 GitHub release, then rebuilding with:
//   TINYGOROOT=/tmp/tinygo cargo xtask plugins-rebuild go-env-greet
// (the xtask builder now uses -buildmode=c-shared -scheduler=none).
// Tracked as Phase-8 fixture-rebuild task.
#[tokio::test]
#[ignore = "go-env-greet: TinyGo reactor adapter mismatch — Phase-8 fixture rebuild"]
async fn go_env_greet_returns_ok_with_env_var() {
    // Skip if the pre-built wasm is absent (TinyGo not installed in CI).
    let wasm_path = workspace_root().join("examples/plugins/go-env-greet/pre-built/plugin.wasm");
    if !wasm_path.exists() {
        eprintln!(
            "SKIP go_env_greet_returns_ok_with_env_var: {} not found; \
             run `TINYGOROOT=... cargo xtask plugins-rebuild go-env-greet` to build.",
            wasm_path.display()
        );
        return;
    }

    // Set the env var the plugin reads.
    std::env::set_var("GREETING_NAME", "LibreFang");

    let bytes = wasm_bytes("go-env-greet");
    let sandbox = WasmSandbox::new().expect("WasmSandbox::new");

    let config = SandboxConfig {
        capabilities: vec![Capability::EnvRead("GREETING_NAME".into())],
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
                host_capabilities: vec![HostCapability::Env],
                ..Default::default()
            },
        )
        .await;

    assert!(
        result.is_ok(),
        "go-env-greet plugin run should succeed; got: {result:?}"
    );
}
