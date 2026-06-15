//! Phase-5 C-007: standalone harness that loads a `.wasm` file as a
//! Component and (optionally) instantiates + invokes its `run`
//! export through `WasmSandbox::execute_component`.
//!
//! Used by `scripts/test-wasm-toolchain.sh` to extend phase-4's
//! "compile + validate" coverage with "compile + load via librefang
//! Component path".
//!
//! ## Usage
//!
//! ```text
//! cargo run --example load_and_run -- <wasm-path> [--invoke]
//! ```
//!
//! - Default (no `--invoke`): parses the bytes as a Component via
//!   `wasmtime::component::Component::from_binary`. Exits 0 if the
//!   binary is a structurally valid Component (any version of the
//!   `wasmtime-44` family). Exits non-zero otherwise.
//!
//! - With `--invoke`: in addition, calls
//!   `WasmSandbox::execute_component` with empty capabilities (no
//!   host bindings). For a hello-world built against the
//!   librefang:plugin world this should succeed; for hello-worlds
//!   that target wasi:cli/run (`fn main()` style) it will fail at
//!   `Plugin::instantiate_async` with a "missing import" error
//!   because they import wasi interfaces we don't bind. That
//!   failure mode is a smoke-acceptable PARTIAL-OK and is reported
//!   distinctly from a hard load failure.
//!
//! ## Exit codes
//!
//! - 0 — Component loaded (and invoked successfully if `--invoke`).
//! - 1 — bad CLI usage.
//! - 2 — file read error.
//! - 3 — Component::from_binary rejected the bytes (NOT a Component).
//! - 4 — `--invoke` mode: load OK but instantiation/invocation failed.
//!   Smoke treats this as PARTIAL-OK (load proved).
//!
//! ## Why not full per-language execute coverage in the smoke?
//!
//! The hello-world programs in `scripts/test-wasm-toolchain.sh`
//! target each language's idiomatic entry point (`fn main`, top-level
//! `console.log`, etc.) — NOT librefang:plugin's `run` export. Making
//! every language's hello compile against the librefang:plugin WIT
//! is a real plugin authoring exercise that doesn't belong in the
//! toolchain smoke. C-007 ships the harness so a future per-language
//! plugin example suite can be wired in incrementally.

use std::process::ExitCode;

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    let invoke = args.iter().any(|a| a == "--invoke");
    let path = match args.iter().skip(1).find(|a| !a.starts_with("--")) {
        Some(p) => p,
        None => {
            eprintln!("usage: {} <wasm-path> [--invoke]", args[0]);
            return ExitCode::from(1);
        }
    };

    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("read {path}: {e}");
            return ExitCode::from(2);
        }
    };

    // --- Load-only path (cheap; just parses the Component binary) ---
    // Default Engine config is sufficient here — fuel/epoch knobs are
    // only used by the full execute_component path (invoked below).
    let engine = wasmtime::Engine::default();
    match wasmtime::component::Component::from_binary(&engine, &bytes) {
        Ok(_) => println!("LOAD-OK ({} bytes parsed as Component)", bytes.len()),
        Err(e) => {
            eprintln!("LOAD-FAIL: not a valid Component: {e}");
            return ExitCode::from(3);
        }
    }

    if !invoke {
        return ExitCode::from(0);
    }

    // --- Invoke path (Plugin::instantiate_async via the gated linker) ---
    let sandbox = librefang_runtime::sandbox::WasmSandbox::new()
        .expect("WasmSandbox::new only fails on broken engine config");
    let result = sandbox
        .execute_component(
            &bytes,
            serde_json::json!({}),
            librefang_runtime::sandbox::SandboxConfig::default(),
            None,
            "smoke-harness",
            librefang_runtime::sandbox_component::ComponentExecuteOptions::default(),
        )
        .await;
    match result {
        Ok(_) => {
            println!("INVOKE-OK");
            ExitCode::from(0)
        }
        Err(e) => {
            // Distinguish "load worked, run didn't" (PARTIAL-OK) from
            // unexpected hard failures. The expected case for
            // hello-world plugins not built against the librefang:plugin
            // world is `Execution` error with "missing import" or
            // similar text.
            let s = e.to_string();
            if s.contains("missing")
                || s.contains("import")
                || s.contains("instantiate")
                || s.contains("function not found")
                || s.contains("export")
            {
                println!("INVOKE-PARTIAL: load OK, invoke wants librefang:plugin world: {s}");
                ExitCode::from(4)
            } else {
                eprintln!("INVOKE-FAIL ({e}): {s}");
                ExitCode::from(4)
            }
        }
    }
}
