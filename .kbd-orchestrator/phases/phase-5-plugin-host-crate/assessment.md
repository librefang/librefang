# Phase Assessment: phase-5-plugin-host-crate

**Project:** LibreFang (BossFang fork) — github.com/GQAdonis/librefang
**Date:** 2026-05-27
**Status:** NEW PHASE — proposed.
**Prior context:** `phase-4-multilang-wasm-toolchain/reflection.md` — toolchain
landed and verified for all 6 languages.
**Goal as originally framed:** "Phase 5 — Rust plugin host crate that
**consumes** the toolchain (wasmtime Engine + Component linker, AOT/JIT
switch, declared-capability permission model, WIT bindings for librefang
host APIs)."

---

## Summary — the framing was wrong

The phase-4 reflection assumed Phase 5 would land a **new** Rust crate
(`librefang-plugin-host`) on a greenfield. **It does not.** Phase 5 sits
on top of substantial existing infrastructure that already implements
most of the named goals — but at older versions and with the wrong
abstraction level for the polyglot toolchain Phase 4 just shipped.

### What already exists in `librefang-runtime`

| File | Lines | Role |
|---|---|---|
| `sandbox.rs` | 1876 | `WasmSandbox` (in-process), `SandboxConfig`, `GuestState`, `ExecutionResult`, `SandboxError`. Uses wasmtime **44** (one major behind phase-4's wasmtime 45). Uses **core** `wasmtime::Linker` — no Component Model. |
| `host_functions.rs` | 1965 | Capability-checked host bridge: `fs_{read,write,list}`, `net_fetch`, `kv_{get,set}`, `agent_{send,spawn}`, `shell_exec`, `env_read`, `time_now`. JSON-RPC dispatch shape. SSRF guards, path-traversal guards, allowlist envs, seccomp/landlock fallbacks. |
| `plugin_runtime.rs` | 1914 | **Subprocess** sidecar runtime (`PluginRuntime` enum) with seccomp/landlock/unshare wrapping. Not the in-process WASM path — runs Python/Node/Shell plugins out-of-process. |
| `python_runtime.rs` | 622 | Python-specific subprocess runtime + venv handling. |
| `plugin_manager/{install,registry,scaffold}.rs` | — | install + registry + scaffold for both in-process and sidecar plugins. |

`librefang-skills` already declares the manifest schema:

```rust
pub enum SkillRuntime { Python, Wasm, Node, Shell }
```

with `SkillRuntime::Wasm` documented as "WASM module executed in sandbox."

### Gap that justifies the phase

Phase-4 shipped a compile toolchain that **only produces WASI Preview 2
Components** for half its languages:

| Language | What phase-4 produces |
|---|---|
| Rust (nightly + cargo-component) | WASI 0.2 Component |
| Python (componentize-py) | WASI 0.2 Component |
| Go (TinyGo `-target=wasip2`) | WASI 0.2 Component |
| TypeScript (AssemblyScript) | core wasm module (raw fd_write) |
| JavaScript (Javy) | core wasm module |
| C (wasi-sdk) | core wasm module |

The existing `WasmSandbox` loads only **core modules** via `Linker::new` —
it **cannot load any of the three Component-Model outputs**. That's the
real Phase 5 gap. The half that produces core modules (TS/JS/C) does
work today; the Rust/Python/Go half does not.

A second-order gap: **the existing host functions (`fs_read`, `net_fetch`,
…) have no WIT description**. Component Model guests can't bind to them
without a `.wit` file declaring the world. Today host calls happen via a
JSON-RPC string method dispatch (`dispatch(state, method, params)`),
which works only with the existing core-module shim ABI — it's
incompatible with Component Model's typed imports.

**Gap count: 5 milestones. 2 architectural decisions required.**

---

## Codebase Scan Results

### M1 — wasmtime bump 44 → 45 (CONFIRMED gap)

`Cargo.toml` workspace pin: `wasmtime = "44"`. Phase-4's dev image and
prod runtime both pin wasmtime 45. The in-process `WasmSandbox` won't be
able to load `.cwasm` AOT artefacts produced by the phase-4 toolchain
until this bump lands. Risk: wasmtime 44→45 has a non-trivial API
delta (Component Model APIs moved); compile-only check is insufficient
— need to run the existing `WasmSandbox` test suite.

### M2 — Component Model loader path (CONFIRMED gap)

`sandbox.rs` uses `wasmtime::Linker` (core). For Component Model
artefacts, the API is `wasmtime::component::{Component, Linker as
ComponentLinker}`. Two viable shapes:

- **(a)** Add a parallel `execute_component()` method alongside the
  existing `execute()` for core modules. Routing decision made by sniffing
  the module preamble (`\0asm` followed by section-type byte) or by
  manifest declaration.
- **(b)** Replace `execute()` entirely with a Component-only path and
  componentize any remaining core modules at install time via
  `wasm-tools component new`.

Option (a) is the smaller blast radius — `librefang-skills` and every
installed Wasm skill in the wild stay working. Option (b) is cleaner
long-term but breaks backward compat.

### M3 — WIT world for host capabilities (CONFIRMED gap)

The existing `host_functions::dispatch()` exposes ~15 host capabilities
via a string-method JSON-RPC. Component guests need a typed WIT
description. Concretely:

```wit
package librefang:host@0.1.0;
interface fs {
    read: func(path: string) -> result<list<u8>, host-error>;
    write: func(path: string, body: list<u8>) -> result<_, host-error>;
    list: func(path: string) -> result<list<string>, host-error>;
}
interface net { fetch: func(req: http-request) -> result<http-response, host-error>; }
interface kv  { get: func(k: string) -> result<option<string>, host-error>;
                set: func(k: string, v: string) -> result<_, host-error>; }
interface agent { send: ...; spawn: ...; }
interface env { read: func(name: string) -> result<option<string>, host-error>; }
interface time { now: func() -> u64; }
world plugin {
    import fs; import net; import kv; import agent; import env; import time;
    export run: func() -> result<_, plugin-error>;
}
```

The WIT must live in a versioned package directory the `cargo-component`
/ `wit-bindgen` / `componentize-py` toolchains can consume. Suggested
location: `crates/librefang-skills/wit/` (so it ships alongside the
manifest crate that already defines `SkillRuntime`). Plugin authors
reference it from their build configs.

### M4 — Manifest capability declarations (CONFIRMED gap — partial)

`SandboxConfig` carries runtime capability flags today, but
`SkillManifest`'s `SkillRequirements` is a freeform struct (see
`librefang-skills/src/lib.rs`) — it does not declare which host
interfaces a plugin needs. Component Model best practice is **explicit
import declaration in the WIT world** + **manifest mirror** so the host
can decide whether to bind each import. Today every Wasm plugin gets the
full capability surface gated only at host-function dispatch time.
Tighten by:

1. Add `capabilities: Vec<HostCapability>` to `SkillManifest`.
2. At link time, only `linker.func_wrap` the interfaces the manifest
   declares — undeclared imports fail loudly.
3. CLI: `librefang skill install` prompts the user when a manifest
   declares capabilities the user hasn't approved at this trust level.

### M5 — AOT (`.cwasm`) load + cache + JIT fallback (CONFIRMED gap)

`sandbox.rs` builds a fresh `Engine` per execution
(`Engine::new(&make_engine_config())`) and compiles WASM JIT-style every
call. Phase-4's plan envisioned an AOT path:

- At install time, `engine.precompile_component(bytes)` → `.cwasm`
  stored alongside the source `.wasm` in the skill install dir.
- At execution time, prefer `Component::deserialize_file(engine,
  cwasm_path)` (fast; no compile). Fall back to `Component::from_file`
  (JIT) if the `.cwasm` is missing, stale, or wasmtime-version-mismatched.
- Config knob: `[runtime.wasm] compile_mode = "aot" | "jit" | "auto"`
  (default `"auto"`).

`.cwasm` files are **wasmtime-version-locked** — a wasmtime bump
invalidates the cache. Cache-bust on the wasmtime version pin.

---

## What's NOT a gap (don't re-build)

- ✅ Capability-checked host functions exist (`host_functions.rs`).
  Phase 5 adds WIT shape over them, doesn't re-implement.
- ✅ SSRF / path-traversal / env allowlist guards are mature.
- ✅ Sidecar (Python / Node / Shell subprocess) plugin path exists in
  `plugin_runtime.rs` and is orthogonal to the wasm path.
- ✅ Plugin install / registry / scaffold flows exist
  (`plugin_manager/`).
- ✅ `SkillManifest` / `InstalledSkill` types in `librefang-skills`.

---

## Architectural Decision Points

### D1. Keep wasm code in `librefang-runtime` or extract to a new crate?

Two paths:

(a) **In place** — extend `sandbox.rs` + `host_functions.rs` with
Component Model support; bump wasmtime; add AOT cache. Smaller diff,
preserves git history, no new workspace member.

(b) **Extract `librefang-plugin-host`** — split sandbox + host_functions
+ wit-bindings into a dedicated crate that depends on
`librefang-kernel-handle`. Larger refactor surface; gains testability
(can fuzz the host bridge in isolation); allows external crates to embed
the plugin host. Aligns with the phase-4 reflection's naming.

**Recommendation:** **(a) in place** for this phase. The existing 3,800
lines of sandbox + host code work; extracting them is a separate
refactoring change that wants its own KBD phase. Phase 5 stays focused
on the Component Model + WIT + AOT additions.

### D2. Component Model only, or core + Component side-by-side?

(a) **Side-by-side** — keep `execute()` for core modules, add
`execute_component()` for Components. Manifest declares which one.

(b) **Component only** — auto-componentize any core-module plugins at
install time via `wasm-tools component new` (which wraps a core module
in an adapter that satisfies the Component world). Single execution
path going forward.

**Recommendation:** **(a) side-by-side** initially; revisit (b) once
all marketplace skills are Components. Forced migration of in-the-wild
core-module skills is a much bigger conversation than Phase 5 can carry.

---

## Verification Plan (what "done" looks like)

After Phase 5 implements all 5 milestones:

1. `cargo check --workspace --lib` green.
2. `cargo test -p librefang-runtime` green (existing sandbox tests
   still pass after wasmtime bump).
3. New test in `librefang-runtime`: compile a smoke Component using
   `cargo component`, load it via the new path, run it under wasmtime,
   assert host-bound `fs_read` returned the expected bytes.
4. Extend `scripts/test-wasm-toolchain.sh` (from phase-4) with a 7th
   step: **load and execute** each language's `.wasm` via the new
   `execute_component()`. Today the smoke only proves compile + validate;
   post-Phase-5 it proves compile + load + invoke via the librefang host.
5. CI lane `wasm-toolchain.yml` extended to run #4.
6. `python3 scripts/enforce-branding.py --check` still clean.
7. Image still slim — Phase 5 adds **no** new dependencies to the
   production runtime (wasmtime is already linked statically; this just
   exercises Component Model APIs that were already compiled in).

---

## Out of scope (deferred)

- Extracting wasm code into a `librefang-plugin-host` crate (D1 option
  b). Open a follow-up phase if/when the testability case becomes
  pressing.
- Forced migration of existing core-module Wasm skills to Components
  (D2 option b). Coordinate with marketplace owners separately.
- Plugin marketplace UI for capability prompts (M4 introduces the data;
  UI is a dashboard phase).
- WASI 0.3 (async) — upstream still pre-stable.
- Hot-reload of plugins. Today each execution gets a fresh `Engine`;
  reuse with hot-reload is its own design conversation.

---

## Risks

| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| wasmtime 44→45 API breakage propagates into `sandbox.rs` | High | Medium | Bump in isolation as M1; let `cargo test -p librefang-runtime` surface every signature change before adding new features. |
| Existing in-the-wild Wasm skills break under any sandbox refactor | Medium | High | D2 = side-by-side. The `execute()` path stays binary-compatible. |
| WIT package versioning gotcha (semver discipline at the import level) | Medium | Medium | Pin to `librefang:host@0.1.0`; never break within 0.x — additive only. Document the deprecation policy in the new crate's README. |
| AOT cache corruption / staleness after wasmtime bump | Medium | Low | Cache key = wasmtime version + plugin SHA; verify both at load; fall back to JIT on mismatch. |
| Manifest capability declarations break existing skills | High | Medium | Default missing `capabilities: Vec<HostCapability>` to "all current Wasm interfaces" for backward compat; require explicit list only for new installs. |
| WIT divergence between librefang versions — plugins built for v1.0 don't load on v0.9 | Medium | Medium | Strict semver on the WIT package; only additive imports within a major. |
| Host-function JSON-RPC dispatch and new typed-WIT dispatch diverge in subtle ways | High | High | Single source of truth: implement WIT bindings as a thin shim that calls the existing `dispatch(state, method, params)` so capability checks aren't duplicated. |

---

## Decisions Required (before Plan phase)

1. **D1**: In place (recommended) vs new `librefang-plugin-host` crate.
2. **D2**: Side-by-side execute paths (recommended) vs Component-only
   with auto-componentization.
3. WIT package location: `crates/librefang-skills/wit/` (recommended,
   ships with the manifest schema) vs new `wit/` at workspace root.
4. AOT cache location: `~/.librefang/skills/<id>/<hash>.cwasm`
   (recommended, alongside the source) vs central
   `~/.librefang/cache/cwasm/<hash>` (smaller but harder to gc per-skill).
5. Default for `[runtime.wasm] compile_mode`: `"auto"` (recommended)
   vs `"aot"` (faster steady-state, slower first-run).

---

## Sources / prior art consulted

- `crates/librefang-runtime/src/sandbox.rs` (existing `WasmSandbox`,
  uses wasmtime 44, core `Linker` only)
- `crates/librefang-runtime/src/host_functions.rs` (15 capability-checked
  host functions; JSON-RPC dispatch shape)
- `crates/librefang-runtime/src/plugin_runtime.rs` (subprocess sidecar
  runtime — out of scope for Phase 5, but the seccomp/landlock guards are
  reference material for the wasm path's defense-in-depth posture)
- `crates/librefang-skills/src/lib.rs` (`SkillManifest`, `SkillRuntime`)
- Phase-4 deliverables: `Dockerfile.rust-dev`, `scripts/test-wasm-toolchain.sh`
