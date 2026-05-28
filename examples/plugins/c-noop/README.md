# `c-noop` — C + wasi-clang, no capabilities

A minimal librefang:plugin Component written in C. Exports `run()`
and immediately returns `Ok(())` without using any host imports.
Exercises the baseline link-time gate with `host_capabilities = []`.

## Build

The committed `pre-built/plugin.wasm` is the canonical artefact. To
regenerate it:

```bash
cargo xtask plugins-rebuild c-noop
```

Under the hood this invokes:

```bash
cd examples/plugins/c-noop

# Generate C bindings from WIT
wit-bindgen c ../../../crates/librefang-skills/wit --world plugin --out-dir bindings/

# Compile the plugin
clang --target=wasm32-unknown-wasi \
    -O2 -nostdlib \
    -Wl,--no-entry -Wl,--export=run -Wl,--export=cabi_realloc \
    plugin.c bindings/plugin.c \
    -o /tmp/c-noop-core.wasm

# Lift to Component Model
wasm-tools component embed --world plugin \
    ../../../crates/librefang-skills/wit \
    /tmp/c-noop-core.wasm \
    -o /tmp/c-noop-embedded.wasm

wasm-tools component new /tmp/c-noop-embedded.wasm -o pre-built/plugin.wasm
```

The Phase-6 size budget is 200 KB; a C noop should be ≤ 10 KB.
