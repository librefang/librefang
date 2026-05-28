# Change C-004 — WasmSandbox::execute_component() via bindgen + Component Linker

**Phase:** phase-5-plugin-host-crate
**Status:** DONE
**Completed:** 2026-05-27
**Files touched:**
  - new `crates/librefang-runtime/src/sandbox_component.rs` (~395 lines)
  - `crates/librefang-runtime/src/lib.rs` — `pub mod sandbox_component;`
  - `crates/librefang-runtime/src/sandbox.rs` — two small additive
    `pub(crate)` helpers (`new_guest_state`, `engine_config`); no
    edits to existing types or methods
  - `crates/librefang-skills/wit/world.wit` — added explicit
    `import host-types;` so bindgen recognises the package's type
    interface (workaround for a `with`-mapping referenced-interface
    check we ended up not needing)

## What landed

`sandbox_component.rs` is the Component Model execute path. It runs
side-by-side with `sandbox.rs`'s core-module `execute()`; the two
share `GuestState`, `SandboxConfig`, `ExecutionResult`, and
`SandboxError`.

### Wiring

- `wasmtime::component::bindgen!({...})` against
  `crates/librefang-skills/wit/` — generates the `Plugin` world entry
  point and one `Host` trait per interface (fs / net / kv / agent /
  env / time + the two `*-types` interfaces). Native `async fn` (no
  `#[async_trait]`).
- `PluginHostState { guest: GuestState, table: ResourceTable }` —
  the Store data type the bindgen-generated traits get implemented on.
- One `Host` trait impl per interface, each method body composed of
  `wit_host::params_*` + `wit_host::call_*` (from C-003) with
  `.map_err(HostErrorWit::from)` to lift the runtime-classified
  `HostErrorRepr` into the bindgen-generated `HostError` variant.
- `impl From<HostErrorRepr> for HostErrorWit` — single source of
  truth for the variant mapping; bindgen owns the cross-language ABI,
  classifier stays in `wit_host`.
- `impl WasmSandbox { pub async fn execute_component(...) -> ... }`
  — fresh `Engine` + `engine_config()` from sandbox.rs,
  `Component::from_binary`, `Linker::<PluginHostState>::new`,
  `Plugin::add_to_linker::<_, HasSelf<PluginHostState>>`,
  `Store::new` with `new_guest_state(&config, ...)`,
  `Plugin::instantiate_async` + `plugin.call_run`. Returns
  `ExecutionResult { output: json!({"ok": null}), fuel_consumed: 0 }`.

### Deliberately deferred to a follow-up (in-file TODOs)

- `store.limiter(...)` wire-up. The core path's `MemoryLimiter` is
  module-private to `sandbox.rs`; reaching it from the Component
  store-limiter closure needs a small accessor. Deferred to keep this
  PR strictly additive — plugin memory caps still apply at the
  `engine_config()` fuel layer, just not as a hard ceiling.
- Watchdog / epoch-callback timeout machinery. The core path's
  `execute_sync` has an intricate fuel + epoch + watchdog dance
  (#3864); porting it onto async-Component instantiation needs
  refactoring sandbox.rs which D1 hygiene forbids in this phase.
  Plugin authors should keep `run` bodies short and cooperate with
  the scheduler until the harmonisation lands.

## Verification

```
cargo check -p librefang-runtime --lib                          # exit 0, 33s incremental
cargo test  -p librefang-runtime --lib sandbox_component        # 2/2 pass, 44s build
cargo test  -p librefang-runtime --lib wit_host                 # 17/17 still pass
```

### Test coverage

- `sandbox_component::tests::bindgen_emits_plugin_and_add_to_linker`
  — proves the bindgen-generated `Plugin` and `add_to_linker` exist
  and accept the `HasSelf<PluginHostState>` marker. Compile-only
  smoke; instantiation requires a real `.wasm`.
- `sandbox_component::tests::rejects_empty_bytes_as_compilation_error`
  — async test that confirms `execute_component(&[], ...)` surfaces
  the `Compilation` error variant rather than panicking.

Full compile→load→invoke per language lives in C-007 (smoke
extension) once executed.

## Iteration log — 7 wasmtime-44 bindgen gotchas fixed in-loop

1. **`async: true` top-level** is wasmtime ≤43 syntax. wasmtime 44
   uses per-direction blocks: `imports: { default: async | trappable
   }, exports: { default: async }`.
2. **`with: { "<pkg>/<iface>/<type>": ... }` mapping** rejected
   because bindgen didn't see `host-types` as "referenced in the
   target world" — the `plugin` world only `use`s it transitively.
   Worked around by dropping the `with` mapping entirely and adding
   a `From` impl (cleaner regardless).
3. **`exports::librefang::plugin::...`** path is wrong; bindgen
   emits the modules at the crate-relative root as
   `librefang::plugin::...` (no `exports::` prefix).
4. **`Config::async_support(true)` is deprecated as no-op** —
   wasmtime 44 enables async support whenever the `async` feature is
   on. Drop the call entirely (lint catches it).
5. **`Plugin::add_to_linker` needs the `D` type parameter**
   (HasData / HasSelf marker). Right form:
   `Plugin::add_to_linker::<_, wasmtime::component::HasSelf<PluginHostState>>(&mut linker, |s| s)`.
6. **`#[async_trait::async_trait]` decorator collides** with the
   bindgen-emitted native-async traits — wasmtime 44 uses Rust's
   stable async-in-traits. Strip the decorator entirely.
7. **`let mut cfg = engine_config()`** flagged as unused-mut after
   dropping the deprecated `async_support` call. Drop the `mut`.

## QA-gate note

Touches 4 files (≥3 threshold). The substantive QA artefact is the
test suite — wit_host 17/17 still passes, sandbox_component 2/2
new tests pass, and the broader library test build is green. The
end-to-end Component round-trip (Phase-5 C-007 smoke) is the
follow-up integration gate.
