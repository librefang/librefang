# Plugin host — WASM Component Model in librefang-runtime

Phase 5 added a **Component Model execute path** to `WasmSandbox`,
side-by-side with the existing core-module path. This doc explains
how the two coexist, how plugin authors target the librefang plugin
contract, and how to migrate an existing core-module Wasm skill to
the Component path.

Companion to [`docs/development/polyglot-dev-image.md`](polyglot-dev-image.md),
which covers the WASM compile toolchain.

## The two execute paths

| | Core-module path | Component path |
|---|---|---|
| Entry point | `WasmSandbox::execute()` | `WasmSandbox::execute_component()` |
| Wasmtime API | `wasmtime::{Module, Linker}` | `wasmtime::component::{Component, Linker}` |
| Implementation | `crates/librefang-runtime/src/sandbox.rs` | `crates/librefang-runtime/src/sandbox_component.rs` |
| ABI | host shim — `host_call(method, json) -> json` | typed WIT — one trait per interface |
| Manifest `runtime` | `wasm` (existing) | `wasm` (existing) — disambiguated by `host_capabilities` presence |
| Capability gate | Runtime per-call (`Capability` enum) | Link-time per-interface (`HostCapability` enum) + runtime per-call |
| AOT cache | None (JIT every call) | `.cwasm` cache via `aot_cache` module |
| Status | Stable; existing skills work unchanged | NEW in Phase 5 |

The two paths are picked by the skill manifest:

- `host_capabilities` empty (or absent) → core-module path.
- `host_capabilities = [...]` non-empty → Component path. Plugins
  bind only the declared interfaces; importing an undeclared
  interface fails at instantiation with a clean error.

## Phase-5 components

| | File | Lines | Role |
|---|---|---|---|
| WIT | `crates/librefang-skills/wit/host.wit`, `world.wit` | ~150 | Typed Component Model interfaces (`librefang:plugin@0.1.0`) |
| Conversion helpers | `crates/librefang-runtime/src/wit_host.rs` | ~440 | `HostErrorRepr` + dispatch param builders + result parsers + `call_*` wrappers |
| Linker + execute | `crates/librefang-runtime/src/sandbox_component.rs` | ~530 | `bindgen!`, `Host` trait impls, per-interface gate, `WasmSandbox::execute_component()`, `ComponentExecuteOptions` |
| AOT cache | `crates/librefang-runtime/src/aot_cache.rs` | ~285 | `CompileMode` + `cache_path` + `load_or_compile` + `precompile` |
| Manifest field | `crates/librefang-skills/src/lib.rs` | (additive) | `HostCapability` enum + `SkillManifest.host_capabilities` |
| Smoke harness | `crates/librefang-runtime/examples/load_and_run.rs` | ~125 | Standalone `LOAD-OK` / `INVOKE-OK` reporter |

## The librefang:plugin world

Single WIT package, six host interfaces, one export:

```wit
package librefang:plugin@0.1.0;

interface host-types { variant host-error { ... 7 variants ... } }
interface fs { read, write, list-entries }
interface net { fetch }
interface kv { get, set }
interface agent { send, spawn }
interface env { read }
interface time { now }

interface plugin-types { variant plugin-error { invalid-input, internal } }

world plugin {
    import host-types;
    import fs;  import net;   import kv;
    import agent;  import env;   import time;
    use plugin-types.{plugin-error};
    export run: func() -> result<_, plugin-error>;
}
```

A plugin author declares which interfaces they need in `skill.toml`:

```toml
[skill]
name = "my-component-skill"
version = "0.1.0"
description = "Reads a file and posts it to a URL"

[runtime]
runtime_type = "wasm"
entry = "plugin.wasm"

# Phase-5 C-005: declare every host interface this plugin imports.
# Importing an interface NOT in this list fails at instantiation.
host_capabilities = ["fs", "net"]
```

The host then binds only `librefang:plugin/fs` and `librefang:plugin/net`
in the Component Model linker. An attempt to import `librefang:plugin/agent`
without declaring `"agent"` here fails with a clean error.

## Capability gate vs runtime capability

Two independent enforcement layers:

1. **`HostCapability` (link-time, coarse)** — decides whether the
   interface symbols (`fs.read`, `fs.write`, …) are bound to the
   Component linker at all. Decided once at instantiation. Set by
   the manifest's `host_capabilities` field.
2. **`Capability` (runtime, fine)** — per-call argument check inside
   `host_functions::dispatch`. Decides whether THIS plugin can read
   THIS path or fetch THIS URL. Already in librefang from day one;
   unchanged by Phase 5.

A plugin granted `HostCapability::Fs` at link time still gets
per-path checks via `Capability::FileRead("/etc/passwd")` at every
call. Component Model doesn't know about path arguments; the runtime
check protects those.

## AOT cache (`compile_mode`)

Component instantiation can be slow (JIT compile of the whole
module on every call). The Phase-5 cache writes a `.cwasm`
pre-compiled artefact to
`~/.librefang/skills/<id>/<sha256>.<wasmtime-version>.cwasm`.

Three modes via `ComponentExecuteOptions::compile_mode`:

| Mode | First call | Cache hit | Cache miss |
|---|---|---|---|
| `Auto` (default) | JIT + opportunistic cache write | AOT load (fast) | JIT fallback + cache write |
| `Aot` | hard error if no cache | AOT load | hard error |
| `Jit` | always JIT, never cache | n/a | n/a |

Cache invalidation is automatic: filename embeds both the wasm SHA
and the wasmtime version. Source drift or wasmtime bump → cleanly
missed cache → JIT recompile.

**Important**: bumping wasmtime in workspace `Cargo.toml` MUST be
paired with a bump of `WASMTIME_CACHE_VERSION` in `aot_cache.rs`,
else stale `.cwasm` files would deserialize under the new wasmtime
binary (undefined behavior).

## Per-language authoring

Each language's hello-world from the polyglot dev image's
[per-language recipes](polyglot-dev-image.md#plugin-recipes-one-per-language)
needs ONE thing changed to become a librefang:plugin Component:
export the `run` function from `librefang:plugin@0.1.0/plugin` world
instead of the language's default entry point.

### Rust (cargo-component)

```toml
# Cargo.toml
[package.metadata.component]
package = "local:my-skill"

[package.metadata.component.target]
path = "../../librefang/crates/librefang-skills/wit"
world = "plugin"
```

```rust
// src/lib.rs
#[allow(warnings)]
mod bindings;
use bindings::Guest;
struct Component;
impl Guest for Component {
    fn run() -> Result<(), bindings::exports::librefang::plugin::plugin_types::PluginError> {
        // host calls available via bindings::librefang::plugin::{fs, net, kv, ...}
        Ok(())
    }
}
bindings::export!(Component with_types_in bindings);
```

Build: `cargo component build --release --target wasm32-wasip2`.

### Python (componentize-py)

```python
# my_skill.py
class Plugin:  # name = world name in PascalCase
    def run(self):
        # bindings available via `import componentize_py`
        pass
```

```bash
componentize-py -d /path/to/librefang/crates/librefang-skills/wit \
    -w plugin componentize my_skill -o plugin.wasm
```

### TypeScript / JavaScript (jco)

```js
// plugin.js
export function run() {
    // host call bindings injected by jco
}
```

```bash
jco componentize plugin.js \
    --wit /path/to/librefang/crates/librefang-skills/wit \
    --world-name plugin \
    -o plugin.wasm
```

### Go (TinyGo)

```go
package main
//go:wasmexport run
func run() {
    // host call bindings via wit-bindgen-go
}
func main() {}
```

```bash
tinygo build -target=wasip2 \
    --wit-package /path/to/librefang/crates/librefang-skills/wit \
    --wit-world plugin \
    -o plugin.wasm
```

### C (wasi-sdk)

Less ergonomic — `wit-bindgen c` generates the binding header. See
the [wit-bindgen docs](https://github.com/bytecodealliance/wit-bindgen)
for the full flow.

## Migrating an existing core-module Wasm skill

Existing skills with `runtime_type = "wasm"` and the old
`host_call(method, json)` shim continue to work unchanged through
`WasmSandbox::execute()`. To migrate one to Component Model:

1. **Add a `host_capabilities` field** to `skill.toml`. Empty list
   keeps the old path; non-empty list activates the new path.
2. **Rewrite the entry point** from the old shim style to the
   Component `run` export in your language of choice (see per-language
   recipes above).
3. **Replace `host_call("fs_read", {"path": "..."})`** with the
   typed call generated by bindgen: `fs::read(path)`.
4. **Test**: build, then `cargo run --example load_and_run -- path/to/plugin.wasm --invoke`
   should report `INVOKE-OK`. (`INVOKE-PARTIAL` means the load worked
   but the run failed — usually missing an interface declaration.)

## Verification

Phase-5 ships with comprehensive test coverage:

```bash
# Unit tests (instant)
cargo test -p librefang-runtime --lib wit_host         # 17 tests
cargo test -p librefang-runtime --lib sandbox_component  # 6 tests
cargo test -p librefang-runtime --lib aot_cache        # 7 tests

# Standalone harness (validates a single .wasm)
cargo build --example load_and_run -p librefang-runtime
./target/debug/examples/load_and_run path/to/plugin.wasm           # load-only
./target/debug/examples/load_and_run path/to/plugin.wasm --invoke  # load + run

# Multi-language toolchain regression (inside the dev image)
docker run --rm -v "$PWD":/workspace -w /workspace \
    librefang-rust-dev:latest \
    /workspace/scripts/test-wasm-toolchain.sh
```

The per-language smoke (`test-wasm-toolchain.sh`) reports a
`[load: OK | WARN | SKIP]` annotation for each Component-producing
language on top of the existing `compile + validate` checks.

## Walking examples (Phase-6)

Per-language minimal plugins live under
[`examples/plugins/`](../../examples/plugins/). Each ships a
checked-in `pre-built/plugin.wasm` (regen via
`cargo xtask plugins-rebuild <name>`) plus an integration test under
[`crates/librefang-runtime/tests/plugin_example_*.rs`](../../crates/librefang-runtime/tests/).

| Example | Language | Capability | Toolchain | Pre-built size | Integration test |
|---|---|---|---|---:|---|
| [`c-noop`](../../examples/plugins/c-noop/) | C | none | LLVM clang + `wasm-ld` + wit-bindgen-c | 1,072 B | ✅ runs end-to-end |
| [`rust-fs-cat`](../../examples/plugins/rust-fs-cat/) | Rust | `fs` | cargo-component | 54,736 B | ✅ runs end-to-end |
| [`python-hello-time`](../../examples/plugins/python-hello-time/) | Python | `time` | componentize-py | 18,368,830 B | ✅ runs end-to-end |
| [`js-kv-counter`](../../examples/plugins/js-kv-counter/) | JavaScript | `kv` | jco componentize | 12,660,894 B | ⏸ Phase-8 (WASI 0.2.10 runtime mismatch) |
| [`go-env-greet`](../../examples/plugins/go-env-greet/) | Go | `env` | TinyGo + wasi P1 adapter | 409,819 B | ⏸ Phase-8 (TinyGo reactor adapter) |

**Phase-7 status (post-wasmtime-wasi wiring):**

3 of 5 examples now run end-to-end. The remaining 2 are fixture build issues
(not sandbox wiring issues) — their `#[ignore = "..."]` reasons name the
exact Phase-8 fix:

- **js-kv-counter**: compiled against jco 1.20 + StarlingMonkey that targets
  WASI 0.2.10; wasmtime-wasi 45 provides 0.2.6. The version aliasing resolves
  at link time but causes a runtime trap inside StarlingMonkey's JIT. Fix:
  rebuild with a jco whose StarlingMonkey targets ≤ WASI 0.2.6.
- **go-env-greet**: the Phase-6 xtask used `-buildmode=default` (→ `_start`,
  command semantics); the reactor adapter needs `_initialize`. Rebuilt with
  `-buildmode=c-shared -scheduler=none` in Phase-7, but the bundled
  `wasi_snapshot_preview1.reactor.wasm` (from `wit-bindgen-cli-0.57.1`, 94 KB)
  traps on `clock_time_get` during `_initialize`. Needs the wasmtime v45
  reactor adapter (52 KB) from the GitHub release. Fix: `cargo xtask
  plugins-rebuild go-env-greet` after bundling the correct adapter.

### Phase-6 sandbox fixes that landed alongside the examples

- **`execute_component` now seeds fuel + epoch on every store** —
  `engine_config()` enables `consume_fuel(true)` and
  `epoch_interruption(true)`, but the Component path was instantiating
  stores without `set_fuel(...)` / `set_epoch_deadline(...)`. Result:
  fuel = 0, first wasm instruction trapped with an opaque "wasm function
  N" error. Fixed in
  [Phase-6 C-007](../../.kbd-orchestrator/phases/phase-6-plugin-examples/progress.json);
  parity with the core-module `execute()` path.
- **C plugins with `-nostdlib` need `--initial-memory=131072`** to
  give RET_AREA a valid home past the default 1-page boundary —
  documented in [`examples/plugins/c-noop/README.md`](../../examples/plugins/c-noop/README.md)
  so the next C example author doesn't re-discover it.

## Phase-5 traceability

Per-change records: [`.kbd-orchestrator/changes/C-001…C-008-*.md`](../../.kbd-orchestrator/changes/).
Phase plan + assessment + reflection:
[`.kbd-orchestrator/phases/phase-5-plugin-host-crate/`](../../.kbd-orchestrator/phases/phase-5-plugin-host-crate/).

## WASI Preview 2 host integration (Phase-7)

Phase-7 wired `wasmtime-wasi` and `wasmtime-wasi-http` into the Component
plugin host so language runtimes can call `wasi:*` interfaces during init.

### What's bound

Every Component store now gets:
- **`wasmtime_wasi::p2::add_to_linker_async`** — wires the full WASI Preview 2
  surface (`wasi:cli`, `wasi:io`, `wasi:clocks`, `wasi:filesystem`,
  `wasi:random`, `wasi:sockets`).
- **`wasmtime_wasi_http::p2::add_only_http_to_linker_async`** — wires
  `wasi:http/types` + `wasi:http/outgoing-handler` without re-binding the
  base WASI interfaces that `add_to_linker_async` already registered.

### Security posture (D2 — deny-by-default WasiCtx)

The `WasiCtx` on every `PluginHostState` is constructed with
`WasiCtxBuilder::new().build()` — **nothing** is enabled by default:

- No preopened directories (filesystem calls return empty or WASI_ERRNO_BADF)
- No environment variables (`get-environment` returns `[]`)
- No inherited stdio (reads/writes to fd 0/1/2 fail)
- No TCP/UDP networking
- No outbound HTTP handler (`WasiHttpCtx::new()`)

The librefang capability list (`SandboxConfig::capabilities`) remains the
real gate. Plugins reach host capabilities through `librefang:plugin/*`
(fs, kv, env, time, net, agent), not through WASI. WASI is present only to
satisfy language runtime init code that calls `wasi:*` internally.

### D6 — librefang interface binding

All 6 `librefang:plugin/*` interfaces are unconditionally bound at link time
(Phase-7 C-005). Capability gating happens at dispatch time in
`host_functions::dispatch`, which returns `HostError::CapabilityDenied` for
calls to ungranted interfaces. This replaced the Phase-5 link-time gating
(which prevented StarlingMonkey's auto-imported `librefang:plugin/fs` from
resolving at instantiation).

### How to widen WASI access for a specific plugin

If a future plugin legitimately needs to read an env var at the WASI level
(vs via `librefang:plugin/env`), update `PluginHostState::new()` in
[`sandbox_component.rs`](../../crates/librefang-runtime/src/sandbox_component.rs):

```rust
// Currently:
wasi: WasiCtxBuilder::new().build(),

// With a specific env var exposed:
wasi: WasiCtxBuilder::new()
    .env("MY_VAR", "my_value")
    .build(),
```

Do NOT use `inherit_env()` — that would expose the host process's full
environment (including secrets) to every plugin.

### Memory limiter (D5 — Phase-7 C-002)

Phase-7 C-002 wired `store.limiter(|s| s.guest.limiter_mut())` in
`execute_component`, closing the Phase-5 TODO. `SandboxConfig::max_memory_bytes`
is now enforced on Component plugins with the same semantics as the
core-module `execute()` path.

## Deferred to follow-up KBD changes

- **Watchdog / epoch-callback timeout** on the Component path
  (parity with `execute()`'s #3864 dance). Needs sandbox.rs refactor
  outside Phase-5 hygiene; for now plugin authors should keep `run`
  bodies cooperative.
- **`store.limiter(...)` wire-up** for the Component path. Needs a
  small `pub(crate)` accessor on `GuestState`; deferred to keep
  Phase 5 strictly additive.
- **Per-language end-to-end plugin examples**, one per language, in
  `examples/plugins/`. The toolchain ships ready; building the
  fixtures is its own KBD change.
- **WASI 0.3 (async-by-default Components)**. Upstream still
  pre-stable.
