# `rust-fs-cat` — Rust + cargo-component, exercises `fs`

A minimal librefang:plugin Component written in Rust. Reads
`/tmp/test-input.txt` via `fs.read` and writes its content back to
`/tmp/test-output.txt` via `fs.write`.

## Build

The committed `pre-built/plugin.wasm` is the canonical artefact. To
regenerate it (e.g. after updating the WIT contract):

```bash
cargo xtask plugins-rebuild rust-fs-cat
```

Under the hood this invokes:

```bash
cd examples/plugins/rust-fs-cat
cargo component build --release --target wasm32-wasip2
cp target/wasm32-wasip2/release/rust_fs_cat.wasm pre-built/plugin.wasm
```

The Phase-6 size budget is 200 KB per artefact. With
`opt-level = "z"` + LTO + strip, the rust-fs-cat plugin sits well
under that ceiling.

## Run via the load_and_run harness

```bash
cargo run --example load_and_run -p librefang-runtime -- \
    examples/plugins/rust-fs-cat/pre-built/plugin.wasm --invoke
```

`INVOKE-OK` means the Component loaded AND the `run` export was
invoked successfully — note this requires a host that grants the
`fs` HostCapability AND the fine-grained
`Capability::FileRead`/`FileWrite` for the two paths. Phase-6
`load_and_run` uses `ComponentExecuteOptions::default()` (empty
caps), so a direct `--invoke` will surface `INVOKE-PARTIAL`
("missing import") — that's expected. The integration test in
`crates/librefang-runtime/tests/plugin_example_rust_fs_cat.rs`
(Phase-6 C-007) sets up a host with the required grants.

## How the source maps to the librefang:plugin world

The `[package.metadata.component.target]` block in `Cargo.toml`
points `cargo-component` at
`../../../crates/librefang-skills/wit` (the WIT directory landed in
Phase-5 C-002). `cargo component build` generates `src/bindings.rs`
from that directory and our crate implements `Guest::run`.

The two calls into `bindings::librefang::plugin::fs::{read, write}`
are the bindgen-typed Rust shims; at WASM runtime they cross the
component boundary into the librefang host where
`sandbox_component.rs::PluginHostState`'s `fs::Host` impl forwards
through `wit_host::call_*` to `host_functions::dispatch` (the
Phase-5 single source of truth for capability checks).
