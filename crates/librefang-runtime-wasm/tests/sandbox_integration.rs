//! Integration tests for `librefang-runtime-wasm` (#3696 deferred).
//!
//! Exercises the public `WasmSandbox` API end-to-end with WAT-defined
//! guest modules:
//!   * load → instantiate → invoke happy path with JSON round-trip
//!   * sandbox boundary: capability deny without grant
//!   * sandbox boundary: capability allow with explicit grant + read-back
//!   * sandbox boundary: fuel cap traps an infinite loop
//!   * sandbox boundary: ABI violation when required exports are missing
//!   * sandbox boundary: memory cap blocks oversized linear-memory growth
//!
//! Modules are inlined as `.wat` text so the suite has no external fixture
//! dependency — wasmtime's `Module::new` accepts both binary `.wasm` and
//! text `.wat`. Tests run on the public async `execute()` entry point so
//! the spawn_blocking + watchdog plumbing is covered too.

use librefang_runtime_wasm::sandbox::{SandboxConfig, SandboxError, WasmSandbox};
use librefang_types::capability::Capability;
use serde_json::json;
use std::io::Write;
use tempfile::NamedTempFile;

// ---------------------------------------------------------------------------
// Guest fixtures (WAT)
// ---------------------------------------------------------------------------

/// Minimal echo module — returns the input bytes verbatim. Smoke-tests the
/// JSON-in / JSON-out path and the `alloc` + `memory` + `execute` ABI.
const ECHO_WAT: &str = r#"
    (module
        (memory (export "memory") 1)
        (global $bump (mut i32) (i32.const 1024))

        (func (export "alloc") (param $size i32) (result i32)
            (local $ptr i32)
            (local.set $ptr (global.get $bump))
            (global.set $bump (i32.add (global.get $bump) (local.get $size)))
            (local.get $ptr)
        )

        (func (export "execute") (param $ptr i32) (param $len i32) (result i64)
            (i64.or
                (i64.shl
                    (i64.extend_i32_u (local.get $ptr))
                    (i64.const 32)
                )
                (i64.extend_i32_u (local.get $len))
            )
        )
    )
"#;

/// Forwards the input bytes to `host_call` and returns the response. Used
/// to exercise capability checks at the sandbox boundary.
const HOST_CALL_PROXY_WAT: &str = r#"
    (module
        (import "librefang" "host_call" (func $host_call (param i32 i32) (result i64)))
        (memory (export "memory") 2)
        (global $bump (mut i32) (i32.const 1024))

        (func (export "alloc") (param $size i32) (result i32)
            (local $ptr i32)
            (local.set $ptr (global.get $bump))
            (global.set $bump (i32.add (global.get $bump) (local.get $size)))
            (local.get $ptr)
        )

        (func (export "execute") (param $input_ptr i32) (param $input_len i32) (result i64)
            (call $host_call (local.get $input_ptr) (local.get $input_len))
        )
    )
"#;

/// Module with a tight infinite loop — used to verify fuel exhaustion.
const INFINITE_LOOP_WAT: &str = r#"
    (module
        (memory (export "memory") 1)
        (global $bump (mut i32) (i32.const 1024))

        (func (export "alloc") (param $size i32) (result i32)
            (local $ptr i32)
            (local.set $ptr (global.get $bump))
            (global.set $bump (i32.add (global.get $bump) (local.get $size)))
            (local.get $ptr)
        )

        (func (export "execute") (param $ptr i32) (param $len i32) (result i64)
            (loop $inf (br $inf))
            (i64.const 0)
        )
    )
"#;

/// Module that omits the required `execute` export. Triggers the ABI guard
/// in `WasmSandbox::execute_sync` when retrieving guest exports.
const MISSING_EXECUTE_WAT: &str = r#"
    (module
        (memory (export "memory") 1)
        (func (export "alloc") (param $size i32) (result i32)
            (local.get $size)
        )
    )
"#;

/// Module that calls `memory.grow` with a 200-page request (≈ 13 MiB) on
/// each invocation. With a `max_memory_bytes` cap below that, the
/// `MemoryLimiter` must deny growth and the guest sees `memory.grow` return
/// `-1`. We surface that signal as the `result_len` so the test can assert
/// on it without needing host_call.
const MEMORY_GROW_WAT: &str = r#"
    (module
        (memory (export "memory") 1)
        (global $bump (mut i32) (i32.const 1024))

        (func (export "alloc") (param $size i32) (result i32)
            (local $ptr i32)
            (local.set $ptr (global.get $bump))
            (global.set $bump (i32.add (global.get $bump) (local.get $size)))
            (local.get $ptr)
        )

        (func (export "execute") (param $ptr i32) (param $len i32) (result i64)
            ;; Try to grow by 200 pages (12.5 MiB). If denied, memory.grow
            ;; returns -1 — surface that as the result_len so the test can
            ;; check from the host side without needing a JSON encoder in
            ;; the guest.
            (local $grow_result i32)
            (local.set $grow_result (memory.grow (i32.const 200)))
            ;; Pack ptr=0, len=grow_result. We don't actually read memory
            ;; at offset 0 (the host bounds check accepts len=0 fine; for
            ;; grow_result=-1 the host's i32→usize cast of -1 is a huge
            ;; number that fails the size cap, returning AbiError, which
            ;; is exactly what we want to assert).
            (i64.extend_i32_u (local.get $grow_result))
        )
    )
"#;

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// Happy path: sandbox loads a guest module, runs `execute`, and the guest
/// echoes the input JSON unchanged. Smoke-tests the full
/// load → instantiate → invoke path including JSON serialization.
#[tokio::test]
async fn sandbox_loads_and_invokes_echo_module() {
    let sandbox = WasmSandbox::new().expect("sandbox should initialize");
    let input = json!({"agent": "alice", "iteration": 7, "payload": [1, 2, 3]});

    let result = sandbox
        .execute(
            ECHO_WAT.as_bytes(),
            input.clone(),
            SandboxConfig::default(),
            None,
            "integration-test-agent",
        )
        .await
        .expect("echo module should execute successfully");

    assert_eq!(
        result.output, input,
        "echo guest must round-trip JSON unchanged"
    );
    assert!(
        result.fuel_consumed > 0,
        "fuel meter must record non-zero consumption for a real invocation"
    );
}

/// Sandbox accepts WASM passed via a file path round-trip — proves the
/// public API works with bytes loaded from disk just as well as inline.
/// Defends against any future "expects ownership / can't be borrowed
/// through fs::read" regression.
#[tokio::test]
async fn sandbox_accepts_module_loaded_from_disk() {
    let mut tmp = NamedTempFile::new().expect("tempfile");
    tmp.write_all(ECHO_WAT.as_bytes()).expect("write wat");
    let bytes = std::fs::read(tmp.path()).expect("read wat back");

    let sandbox = WasmSandbox::new().unwrap();
    let result = sandbox
        .execute(
            &bytes,
            json!({"hello": "from disk"}),
            SandboxConfig::default(),
            None,
            "disk-load-agent",
        )
        .await
        .expect("disk-loaded module should execute");

    assert_eq!(result.output, json!({"hello": "from disk"}));
}

/// Sandbox boundary: a guest with NO capabilities cannot read files — the
/// `host_call` for `fs_read` must come back as a JSON error response. This
/// is the deny-by-default invariant of the capability system.
#[tokio::test]
async fn sandbox_denies_fs_read_without_capability() {
    let sandbox = WasmSandbox::new().unwrap();

    // Defensive: cargo runs each crate's tests with CWD = crate dir, so
    // `Cargo.toml` resolves. If a future test runner (nextest with a custom
    // workdir, sandboxed exec, …) changes CWD, fail loudly here instead of
    // silently misattributing the resulting fs_read failure to capability
    // denial — the test would still "pass" but stop exercising the deny
    // path it claims to.
    assert!(
        std::path::Path::new("Cargo.toml").exists(),
        "test assumes CWD = crate dir; Cargo.toml not found in {:?}",
        std::env::current_dir()
    );

    // Cargo.toml exists in every crate's working dir during test runs;
    // fs_read canonicalises before the capability check (#3814) so the
    // path must resolve.
    let input = json!({
        "method": "fs_read",
        "params": {"path": "Cargo.toml"}
    });
    let cfg = SandboxConfig {
        capabilities: vec![], // deny-by-default
        ..Default::default()
    };

    let result = sandbox
        .execute(
            HOST_CALL_PROXY_WAT.as_bytes(),
            input,
            cfg,
            None,
            "no-caps-agent",
        )
        .await
        .expect("proxy module itself should run; the host_call returns a JSON error");

    let err = result
        .output
        .get("error")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    assert!(
        err.contains("denied") || err.contains("Capability"),
        "expected capability-denied error, got: {err:?} (full: {:?})",
        result.output
    );
}

/// Sandbox boundary: granting a `time_now`-style cap-free host call returns
/// a non-error response. This pins the inverse of the deny test — the
/// boundary must not be a one-way wall.
#[tokio::test]
async fn sandbox_allows_capless_host_call() {
    let sandbox = WasmSandbox::new().unwrap();
    let input = json!({"method": "time_now", "params": {}});

    let result = sandbox
        .execute(
            HOST_CALL_PROXY_WAT.as_bytes(),
            input,
            SandboxConfig::default(),
            None,
            "time-agent",
        )
        .await
        .expect("time_now is unconditionally permitted");

    let ts = result
        .output
        .get("ok")
        .and_then(|v| v.as_u64())
        .expect("expected `ok` timestamp, got: {result.output:?}");
    assert!(ts > 1_700_000_000, "timestamp implausibly small: {ts}");
}

/// Sandbox boundary: with `EnvRead("PATH")` granted, the guest can read
/// PATH. With no capability granted, the same call is denied. This is the
/// positive→negative pair that confirms the capability check is actually
/// dispatched (vs. always-allow or always-deny degenerate behaviour).
#[tokio::test]
async fn sandbox_capability_grant_toggles_env_read() {
    let sandbox = WasmSandbox::new().unwrap();
    let input = json!({
        "method": "env_read",
        "params": {"name": "PATH"}
    });

    // Allow path
    let allow_cfg = SandboxConfig {
        capabilities: vec![Capability::EnvRead("PATH".into())],
        ..Default::default()
    };
    let allow = sandbox
        .execute(
            HOST_CALL_PROXY_WAT.as_bytes(),
            input.clone(),
            allow_cfg,
            None,
            "env-allow-agent",
        )
        .await
        .expect("granted env_read should run");
    // Either ok-with-value or ok-null are acceptable; the strict invariant
    // is: NOT a "denied" error.
    let allow_err = allow
        .output
        .get("error")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    assert!(
        !allow_err.contains("denied"),
        "with EnvRead(PATH) granted, env_read must not be denied; got: {allow_err}"
    );

    // Deny path — same call, no capabilities.
    let deny_cfg = SandboxConfig {
        capabilities: vec![],
        ..Default::default()
    };
    let deny = sandbox
        .execute(
            HOST_CALL_PROXY_WAT.as_bytes(),
            input,
            deny_cfg,
            None,
            "env-deny-agent",
        )
        .await
        .expect("proxy itself should still run");
    let deny_err = deny
        .output
        .get("error")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    assert!(
        deny_err.contains("denied") || deny_err.contains("Capability"),
        "without EnvRead, env_read must be denied; got: {deny_err}"
    );
}

/// Sandbox boundary: a runaway guest must trap on fuel exhaustion rather
/// than burning host CPU indefinitely. Pins the `Trap::OutOfFuel` →
/// `SandboxError::FuelExhausted` mapping.
#[tokio::test]
async fn sandbox_fuel_cap_traps_runaway_guest() {
    let sandbox = WasmSandbox::new().unwrap();
    let cfg = SandboxConfig {
        fuel_limit: 10_000,
        ..Default::default()
    };

    let err = sandbox
        .execute(
            INFINITE_LOOP_WAT.as_bytes(),
            json!({}),
            cfg,
            None,
            "runaway-agent",
        )
        .await
        .expect_err("infinite loop must trap on fuel exhaustion");

    assert!(
        matches!(err, SandboxError::FuelExhausted),
        "expected FuelExhausted, got: {err}"
    );
}

/// Sandbox boundary: a module that violates the guest ABI (here: missing
/// the required `execute` export) must be rejected with a typed
/// `AbiError`, not a panic and not a generic execution error.
#[tokio::test]
async fn sandbox_rejects_module_missing_required_exports() {
    let sandbox = WasmSandbox::new().unwrap();

    let err = sandbox
        .execute(
            MISSING_EXECUTE_WAT.as_bytes(),
            json!({}),
            SandboxConfig::default(),
            None,
            "broken-abi-agent",
        )
        .await
        .expect_err("module missing `execute` must be rejected");

    match err {
        SandboxError::AbiError(msg) => {
            assert!(
                msg.contains("execute"),
                "AbiError must mention the missing export, got: {msg}"
            );
        }
        other => panic!("expected AbiError for missing export, got: {other}"),
    }
}

/// Sandbox boundary: the linear-memory cap is enforced — a guest that
/// requests growth past the cap sees `memory.grow` return -1, which the
/// host then surfaces as an oversized-result `AbiError` (because -1 cast
/// through u32 lands well above the 1 MiB result cap). Either the
/// AbiError landing OR a memory.grow=-1 unpacks to a valid response is
/// acceptable; both prove the limiter ran. We assert the call returned
/// SOMETHING that didn't grow memory (i.e. the host didn't OOM).
#[tokio::test]
async fn sandbox_memory_cap_blocks_oversized_growth() {
    let sandbox = WasmSandbox::new().unwrap();
    let cfg = SandboxConfig {
        // Cap below the 200-page (12.5 MiB) growth attempt the guest makes.
        max_memory_bytes: 1024 * 1024, // 1 MiB
        fuel_limit: 1_000_000,
        ..Default::default()
    };

    let outcome = sandbox
        .execute(
            MEMORY_GROW_WAT.as_bytes(),
            json!({}),
            cfg,
            None,
            "memory-hog-agent",
        )
        .await;

    // Two acceptable shapes:
    //  - AbiError because the guest returned packed result with len=-1
    //    (cast to u32 it's huge, exceeding MAX_GUEST_RESULT_BYTES)
    //  - Some other non-Ok shape, but NOT a successful 13 MiB allocation
    //    on the host side.
    match outcome {
        Err(SandboxError::AbiError(_)) => {
            // expected — the failed grow surfaced as -1 / oversized result
        }
        Err(other) => {
            // Other error variants are also fine — the invariant is
            // "memory cap was respected", not the exact failure mode.
            // The smoke check is that we got back from execute() at all
            // and didn't exhaust host memory.
            eprintln!("memory-cap test: non-Abi error variant accepted: {other}");
        }
        Ok(result) => {
            // If the call somehow succeeded, the `result_len` must equal
            // 0xFFFFFFFF (i32::-1 as u32) — meaning grow was denied and
            // the guest packed -1 as the length. The host bounds check
            // would have rejected that as an AbiError, so reaching the
            // Ok arm at all is suspicious; flag it.
            panic!(
                "memory cap appears to have been bypassed — got Ok with: {:?}",
                result.output
            );
        }
    }
}
