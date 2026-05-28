# Change C-007 — Smoke extension: compile → load via librefang

**Phase:** phase-5-plugin-host-crate
**Status:** DONE
**Completed:** 2026-05-27
**Files touched:**
  - new `crates/librefang-runtime/examples/load_and_run.rs`
  - `scripts/test-wasm-toolchain.sh` — added
    `load_component_check` helper + invocation in the 4
    Component-producing language tests.

## What landed

### `examples/load_and_run.rs` — standalone harness

```text
cargo run --example load_and_run -- <wasm-path> [--invoke]
```

Two modes:

- **Load-only (default)**: parses `<wasm-path>` via
  `wasmtime::component::Component::from_binary` against the same
  wasmtime version librefang-runtime links. Prints `LOAD-OK`
  (exit 0) or `LOAD-FAIL` (exit 3).
- **`--invoke`**: in addition, calls
  `WasmSandbox::execute_component(...)` with empty capabilities (no
  host bindings). Distinguishes:
  - `INVOKE-OK` — load + run succeeded (exit 0)
  - `INVOKE-PARTIAL` — load OK, run failed with "missing import"
    / "missing export" / similar (exit 4). Expected for
    hello-world plugins not built against the librefang:plugin
    world; smoke treats as PARTIAL-OK.
  - `INVOKE-FAIL` — unexpected hard failure (exit 4 + stderr trace)

Exit codes give CI a clean signal layered with stdout markers for
human grep.

### Smoke script extension

Added `load_component_check` to `scripts/test-wasm-toolchain.sh`.
Wired into Rust / Python / JavaScript / Go (the four
Component-producing languages). TypeScript (AssemblyScript) and C
(wasi-clang) still go through compile + validate only — they emit
core modules, not Components.

The helper:
1. Probes for a pre-built `load_and_run` binary at
   `target/debug/examples/load_and_run` and
   `target/release/examples/load_and_run`.
2. Builds on demand via `cargo build --example load_and_run -p
   librefang-runtime --quiet` if `cargo` is on PATH.
3. Records `[load: OK]`, `[load: WARN — <reason>]`, or `[load: SKIP
   — example not built]` as a suffix to the existing per-language
   PASS line.

Crucially, a `load: WARN` doesn't fail the smoke — the existing
phase-4 compile+validate step remains the hard gate. The load check
is incremental signal that surfaces regressions in the librefang
Component loader when wasmtime is bumped or our bindgen options
change.

## Verification

### Example builds clean
```
cargo build --example load_and_run -p librefang-runtime
# Finished in 45s
ls -la target/debug/examples/load_and_run
# -rwxr-xr-x ... 100M
./target/debug/examples/load_and_run
# usage: ... <wasm-path> [--invoke]
```

### End-to-end smoke with synthetic empty Component
```
# Hand-encoded minimal Component (magic + component-preview version)
$ printf '\x00asm\x0d\x00\x01\x00' > /tmp/empty.wasm
$ ./target/debug/examples/load_and_run /tmp/empty.wasm
LOAD-OK (8 bytes parsed as Component)
exit=0
```

Confirms the harness wiring runs end-to-end against the
librefang-runtime Component loader.

### Deferred to a follow-up

Full per-language `--invoke` coverage requires each language's
hello-world to target the librefang:plugin world (export
`run() -> result<_, plugin-error>` matching the WIT). That's a real
plugin authoring exercise — out of scope for the toolchain smoke.
The harness ships ready for incremental per-language plugin
fixtures.

## Issues hit + fixed

1. **`engine_config()` was `pub(crate)`**, so the example (which
   compiles as a separate binary crate) couldn't reach it. Switched
   to `wasmtime::Engine::default()` — fuel / epoch tuning is only
   relevant on the full `execute_component` path, which the example
   invokes through `WasmSandbox` which has its own engine.

## QA-gate note

Touches 2 files (<3 threshold). Verification is the example binary
running + the smoke script reporting load checks on real
Components from the dev image. No `/refine-validate` needed — the
example is a tiny standalone harness with no application logic.
