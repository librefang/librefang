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

# Generate C bindings from WIT (re-emits bindings/plugin.{c,h} + plugin_component_type.o)
wit-bindgen c ../../../crates/librefang-skills/wit --world plugin --out-dir bindings/

# Compile the plugin (needs LLVM clang + wasm-ld + a WASI sysroot)
/opt/homebrew/opt/llvm/bin/clang \
    --target=wasm32-wasip1 \
    --sysroot=${WASI_SYSROOT:-/tmp/tinygo/lib/wasi-libc/sysroot} \
    -fuse-ld=/opt/homebrew/opt/lld/bin/wasm-ld \
    -O2 -nostdlib \
    -Wl,--no-entry \
    -Wl,--export-dynamic \
    -Wl,--initial-memory=131072 \
    bindings/plugin.c plugin.c stubs.c \
    bindings/plugin_component_type.o \
    -o /tmp/c-noop-core.wasm

# Lift to Component Model
wasm-tools component embed --world plugin \
    ../../../crates/librefang-skills/wit \
    /tmp/c-noop-core.wasm \
    -o /tmp/c-noop-embedded.wasm

wasm-tools component new /tmp/c-noop-embedded.wasm -o pre-built/plugin.wasm
```

### Why these specific linker flags?

- `-nostdlib` — no libc; saves ~6 KB. Forces us to ship `stubs.c` with
  no-op `free` / `malloc` / `realloc` + a trapping `abort`. The
  generated `cabi_realloc` reaches `realloc` only on string returns,
  and we never return strings, so the stubs are dead at runtime —
  they're only there to satisfy the linker.
- `-Wl,--export-dynamic` — exports `__attribute__((export_name(...)))`
  symbols (`run`, `cabi_realloc`, `cabi_post_run`) without having to
  enumerate them on the command line.
- `-Wl,--initial-memory=131072` — bumps initial memory to **2 pages
  (128 KiB)**. Without this, wasm-ld emits a 1-page module, but the
  generated `RET_AREA` static lands at byte 65536 — exactly past the
  end of page 1, so the first `i32.store8` into RET_AREA traps OOB on
  every invocation. **2 pages is the minimum that works**; we don't
  need more. If you add real allocations to the plugin, scale this up.

### Toolchain provenance

- `clang` / `wasm-ld` — Homebrew LLVM 22.x (system clang has no wasm32
  backend; the `lld` formula provides `wasm-ld`).
- WASI sysroot — borrowed from the TinyGo installation
  (`${TINYGOROOT}/lib/wasi-libc/sysroot`). A dedicated wasi-sdk install
  works just as well — point `WASI_SYSROOT` at it.

The Phase-6 size budget is 200 KB; the current c-noop is **1,072 bytes**.
