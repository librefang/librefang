# Phase Plan: phase-5-plugin-host-crate

**Phase:** `phase-5-plugin-host-crate`
**Backend:** Native KBD (no OpenSpec, no evolver bridge)
**Date:** 2026-05-27
**Source assessment:** `assessment.md` in this directory
**Soft prerequisite:** phase-4 PR #56 merged (provides the polyglot toolchain
the new Component Model loader path consumes). Phase 5 can begin in parallel
on a worktree, but C-008's smoke extension below depends on phase-4's
`scripts/test-wasm-toolchain.sh` being on `main`.

---

## Decisions resolved

| # | Decision | Choice | Constraint impact |
|---|---|---|---|
| D1 | Crate placement | **In place inside `librefang-runtime`** | **Additive-only**: new files (`sandbox_component.rs`, `wit_host.rs`, `aot_cache.rs`), new methods on existing types, no rewrites of `sandbox.rs` / `host_functions.rs` core paths. Goal: minimize merge-conflict surface with upstream librefang. |
| D2 | Execute path shape | **Side-by-side**: keep `WasmSandbox::execute()` for core modules; add `WasmSandbox::execute_component()` | Backward-compatible. Existing in-the-wild Wasm skills load unchanged. |
| D3 | WIT package location | **`crates/librefang-skills/wit/`** | Ships with the manifest schema crate so plugin authors have one place to look. |
| D4 | AOT cache location | **`~/.librefang/skills/<id>/<sha>.cwasm`** | Per-skill, easy to garbage-collect when uninstalling a skill. |
| D5 | Default `compile_mode` | **`"auto"`** | First run = JIT; install/refresh writes the `.cwasm`; subsequent runs = AOT. No regression for users who never pre-warm. |

### D1 elaboration — upstream-sync hygiene

The hard constraint: minimize the surface area that conflicts on upstream
merge. Rules every change in this phase MUST follow:

1. **New behavior lives in NEW files.** Don't expand `sandbox.rs` /
   `host_functions.rs`; create siblings (`sandbox_component.rs`,
   `wit_host.rs`, `aot_cache.rs`) and `pub mod` them from `lib.rs`.
2. **Existing public types stay binary-stable.** Add new methods via
   `impl WasmSandbox { ... }` blocks in the new files, not by modifying
   the existing `impl` in `sandbox.rs`.
3. **Wasmtime version bump is the ONE unavoidable cross-cut.** It will
   conflict with any upstream wasmtime work. Land it in M1 as a single
   focused change so future merge conflicts are small and obvious.
4. **WIT files are net-new.** Zero upstream conflict surface.
5. **Manifest extension uses `#[serde(default)]` on new fields.** A
   manifest written before Phase 5 must still deserialize.

---

## Ordered Change List

### C-001 — wasmtime workspace bump 44 → 45
**Branch:** `feat/phase5-wasmtime-bump`
**Files:** `Cargo.toml` (workspace), `crates/librefang-runtime/src/sandbox.rs` (only if API delta forces it)
**Effort:** S (assumes wasmtime 45 is API-compatible with the existing core-`Linker` usage) — M if `wasmtime::Linker` surface drifted.
**Recommended agent:** `claude`

The unavoidable cross-cut, landed first and in isolation so future
upstream-merge conflicts on the wasmtime pin are tiny.

- [ ] Bump `[workspace.dependencies] wasmtime = "44"` → `"45"`.
- [ ] `cargo check --workspace --lib` — fix any API delta in
  `sandbox.rs` minimally (no behavior changes, just API mapping).
- [ ] `cargo test -p librefang-runtime` — every existing sandbox test
  must stay green. **Do not** weaken any test to make it pass.
- [ ] Verify `Dockerfile.rust-dev` already has wasmtime 45 (it does, per
  phase-4 C-002).

**Done when:** workspace builds and `librefang-runtime` tests pass under
wasmtime 45 with zero functional regressions.

---

### C-002 — WIT package for librefang host capabilities
**Branch:** `feat/phase5-wit-host-world`
**Files:** new `crates/librefang-skills/wit/host.wit`, new `crates/librefang-skills/wit/world.wit`, `crates/librefang-skills/Cargo.toml` (`include` field), brief docs note in `crates/librefang-skills/README.md` if present.
**Depends on:** C-001
**Effort:** M
**Recommended agent:** `claude`

Declare the typed host surface that Component-Model guests bind against.

- [ ] `host.wit` — package `librefang:host@0.1.0`. Interfaces: `fs`,
  `net`, `kv`, `agent`, `env`, `time`. Mirror the parameter shapes of
  the existing `host_functions::dispatch` methods exactly (don't redesign
  the surface in this phase; that's a separate Wit-API-design effort).
- [ ] `world.wit` — package `librefang:plugin@0.1.0`. Defines world
  `plugin` that imports each `librefang:host` interface and exports
  `run: func() -> result<_, plugin-error>`.
- [ ] `host-error` and `plugin-error` variants modeled on the existing
  `SandboxError` enum variants.
- [ ] Strict semver discipline noted in a `wit/README.md`: 0.1.x is
  additive-only; any breaking change goes to 0.2.0 with a deprecation
  window.
- [ ] Validate via `wasm-tools` (already in the dev image):
  `wasm-tools component wit crates/librefang-skills/wit/`.

**Done when:** `wasm-tools` parses the WIT clean and a hand-written
hello plugin in Rust (built via `cargo component`, not part of this PR)
binds the imports successfully.

---

### C-003 — `wit_host.rs` shim: WIT bindings → existing dispatch
**Branch:** `feat/phase5-wit-host-shim`
**Files:** new `crates/librefang-runtime/src/wit_host.rs`, one-line
`pub mod wit_host;` in `crates/librefang-runtime/src/lib.rs`.
**Depends on:** C-002
**Effort:** M
**Recommended agent:** `claude`

The capability-check-deduplication mitigation from the assessment: WIT
bindings are a thin shim that **calls the existing
`host_functions::dispatch(state, method, params)`**. No new capability
enforcement code — guests get exactly the same checks they get today.

- [ ] `wasmtime::component::bindgen!` macro generates host trait from
  `librefang-skills/wit/`.
- [ ] Implement each interface's `Host` trait by marshalling the typed
  args into `serde_json::Value`, calling the existing `dispatch()`, and
  marshalling the JSON result back into the typed return.
- [ ] One unit test per interface verifying the shim round-trip
  (typed-in → JSON → dispatch → JSON → typed-out).
- [ ] `wit_host.rs` is the ONLY new file with bindgen output. Don't
  spread bindgen across the crate.

**Done when:** `cargo test -p librefang-runtime wit_host` passes; the
existing dispatch test suite is untouched.

---

### C-004 — `WasmSandbox::execute_component()` (new in `sandbox_component.rs`)
**Branch:** `feat/phase5-execute-component`
**Files:** new `crates/librefang-runtime/src/sandbox_component.rs`,
one-line `pub mod sandbox_component;` in `lib.rs`.
**Depends on:** C-003
**Effort:** M-L
**Recommended agent:** `claude`

Per D1 hygiene: NO edits to `sandbox.rs` core paths. The new method
lives in an `impl WasmSandbox` block inside `sandbox_component.rs`.

- [ ] `pub async fn execute_component(&self, wasm_bytes: &[u8], input:
  serde_json::Value, config: SandboxConfig, kernel: Option<Arc<dyn
  KernelHandle>>, agent_id: &str) -> Result<ExecutionResult,
  SandboxError>` — signature mirrors `execute()` exactly.
- [ ] Internals: `Component::from_binary(&engine, wasm_bytes)`,
  `ComponentLinker::new(&engine)`, wire the shim impls from C-003,
  instantiate, invoke `run`.
- [ ] Reuse the existing `Engine` config builder
  (`make_engine_config()` from `sandbox.rs`) — no new resource-limit
  story, no new epoch-isolation story. Component path inherits both.
- [ ] Integration test: a 10-line Rust component (built via
  `cargo component build` in a `tests/fixtures/` subdir) executes,
  binds `fs/read`, returns the file contents. The fixture is checked in
  as a pre-built `.wasm` so CI doesn't depend on cargo-component being
  on the runner.

**Done when:** the integration test passes; `WasmSandbox::execute()`
behavior is unchanged (regression-checked by re-running the existing
sandbox test suite).

---

### C-005 — Manifest `capabilities` declaration + link-time gating
**Branch:** `feat/phase5-manifest-capabilities`
**Files:** `crates/librefang-skills/src/lib.rs` (additive: new enum
`HostCapability`, new field `capabilities: Vec<HostCapability>` on
`SkillManifest` with `#[serde(default)]`),
`crates/librefang-runtime/src/sandbox_component.rs` (consume the
declaration).
**Depends on:** C-004
**Effort:** M
**Recommended agent:** `claude`

Tighten the capability surface for Component plugins. Core-module
plugins go unchanged (the manifest field defaults to "all" for them so
no in-the-wild skill breaks).

- [ ] Define `pub enum HostCapability { Fs, Net, Kv, Agent, Env, Time }`
  (mirror the WIT interfaces).
- [ ] Add `#[serde(default)] pub capabilities: Vec<HostCapability>` to
  `SkillManifest`. Empty default means "no caps" for Components and
  "all caps" for core modules (compatibility shim documented inline).
- [ ] In `sandbox_component.rs`, before linker binding: only
  `linker.instance("librefang:host/<iface>")` for interfaces in the
  declared `capabilities`. Undeclared imports cause a clean
  `SandboxError::MissingCapability { iface, requested_by }` at
  instantiate time.
- [ ] One test per capability subset: declare `[Fs]`, build a Component
  that imports `kv`, assert instantiation fails with the new error.

**Done when:** integration tests cover at least 3 capability subsets;
all-caps Component skills still work; missing-cap Component skills
produce a clean error.

---

### C-006 — AOT (`.cwasm`) cache + JIT fallback
**Branch:** `feat/phase5-aot-cache`
**Files:** new `crates/librefang-runtime/src/aot_cache.rs`,
one-line `pub mod aot_cache;` in `lib.rs`,
`crates/librefang-runtime/src/sandbox_component.rs` (consume the cache),
new config block in `crates/librefang-types/src/config.rs` or wherever
`KernelConfig` lives (`#[serde(default)]` field on a new
`WasmRuntimeConfig` struct).
**Depends on:** C-004 (needs the Component execute path to wire the cache against)
**Effort:** M
**Recommended agent:** `claude`

The first-class win for users: skills load in milliseconds after the
first run.

- [ ] `aot_cache.rs` exposes `pub fn cache_path(skill_id: &str, sha: &str,
  wasmtime_version: &str) -> PathBuf` and `pub fn load_or_compile(engine,
  wasm_bytes, cache_path) -> Result<Component, SandboxError>`.
- [ ] Cache key includes the wasmtime version pin so `.cwasm` files
  invalidate cleanly on M1 bumps. Stored at
  `~/.librefang/skills/<id>/<sha>.<wasmtime-version>.cwasm`.
- [ ] On load: try `Component::deserialize_file(engine, path)`; on any
  failure, fall back to `Component::from_binary` (JIT), log a `WARN`,
  optionally rewrite the cache.
- [ ] Config: `[runtime.wasm] compile_mode = "auto" | "aot" | "jit"`,
  default `"auto"`. `"aot"` errors when the cache is missing rather
  than falling back; `"jit"` skips the cache entirely.
- [ ] On `librefang skill install`: precompile the Component eagerly
  so the first execution is already AOT.
- [ ] Test matrix: cache hit, cache miss → JIT fallback, wasmtime
  version mismatch → JIT fallback + rewrite, `compile_mode = "aot"` +
  missing cache → hard error.

**Done when:** all four scenarios above pass; bench (informal) shows
cache-hit Component load ≥ 10x faster than JIT for a non-trivial plugin.

---

### C-007 — Extend `scripts/test-wasm-toolchain.sh` with **load + execute** per language
**Branch:** `feat/phase5-smoke-load-execute`
**Files:** `scripts/test-wasm-toolchain.sh` (extend with a new
post-compile step), new
`crates/librefang-runtime/examples/load_and_run.rs` (a tiny binary the
smoke calls into to do the load-and-execute via the new Component path).
**Depends on:** C-004 (needs the execute path) + phase-4 PR merged so
the script exists on `main`.
**Effort:** S
**Recommended agent:** `claude`

Phase 4 proved compile + validate. Phase 5 proves the full **compile →
load → invoke via librefang host** chain.

- [ ] Add a `load_and_run` example binary in `librefang-runtime` that
  reads a `.wasm` path from argv, calls
  `WasmSandbox::execute_component(...)`, and prints `ExecutionResult`
  stdout.
- [ ] In the smoke script, after the existing compile+validate step
  for each language, run:
  `cargo run --quiet --example load_and_run -- "$d/hello.wasm"` and
  assert stdout matches `hello-${lang}-wasm`.
- [ ] Python special case: the smoke's componentize-py output binds
  `time/now` for its hello (no fs/net) — minimal capability surface.
- [ ] CI lane `wasm-toolchain.yml` picks this up automatically (the
  file path is already in its trigger list).

**Done when:** smoke prints "All language WASM toolchains compile + load
+ invoke green" for at least 4/6 languages. TS / JS / C produce core
modules and stay on the existing `execute()` path; Rust / Python / Go
exercise the new `execute_component()` path.

---

### C-008 — Documentation + migration note
**Branch:** `docs/phase5-plugin-host`
**Files:** new `docs/development/plugin-host.md`, update
`docs/development/polyglot-dev-image.md` (link in), brief CHANGELOG line.
**Depends on:** C-007
**Effort:** S
**Recommended agent:** `claude`

- [ ] `plugin-host.md` — explain the two execute paths, the WIT world,
  the capability model, the AOT cache, and how to declare
  `capabilities` in a `skill.toml`.
- [ ] Add a "Migrating an existing Wasm skill to Component Model"
  section with a worked example.
- [ ] Update the phase-4 polyglot doc to point at the new doc for the
  "what runs the .wasm files" question.
- [ ] CHANGELOG `[Unreleased]` entry per the repo's attribution rules.

**Done when:** docs render clean; a new contributor can build, install,
and run a Component-Model Wasm skill end-to-end using only the docs.

---

## Out of scope for this phase

Tracked as future KBD changes if/when the case becomes pressing:

- Extracting wasm code into a `librefang-plugin-host` crate (D1
  alternative).
- Forced migration of in-the-wild Wasm skills to Components (D2
  alternative).
- Dashboard UI for the new `capabilities` manifest field.
- Hot-reload of running plugins.
- WASI 0.3 async support.
- WIT-binding generation for other host crates (mcp servers etc.) —
  this phase establishes the pattern; others can follow.

---

## Risks to manage during execution

| Risk | Affects | Mitigation |
|---|---|---|
| wasmtime 44→45 API delta surface larger than expected | C-001 | Land bump alone; do not stack other changes on top of C-001 until the test suite is green. Roll back to 44 if delta is too large for one focused diff. |
| `wasmtime::component::bindgen!` macro output shape changes per wasmtime version | C-003 | Pin generated bindings to a re-run on every wasmtime bump (note in `wit_host.rs` header). |
| WIT semver discipline drifts | C-002 | `wit/README.md` policy doc; require WIT review in every Phase-5+ PR description. |
| Manifest capability default break in-the-wild Wasm skills | C-005 | Default for core-module plugins = "all caps"; explicit list required only for Component plugins. Document loudly. |
| AOT cache corruption causes plugin failures | C-006 | Always fall back to JIT on any deserialize error; log loudly; never hard-fail in `compile_mode = "auto"`. |
| Pre-built `.wasm` fixtures checked into git bloat the repo | C-004, C-005 | Fixtures must be < 16 KB each; for larger needs use git-lfs (would need a new `.gitattributes` rule — defer that conversation). |
| Smoke step adds notable CI time | C-007 | The compile pass already dominates; adding 6 quick `cargo run --example` invocations adds < 30s. Acceptable. |
| Upstream librefang merges a parallel Component Model implementation while Phase 5 is in flight | All | The upstream-sync hygiene rules (additive-only, new files only) keep the merge cost bounded to a single rebase. The wasmtime pin (C-001) is the only certain conflict. |

---

## Verification matrix

After all 8 changes merge:

| Check | Command | Acceptance |
|---|---|---|
| Workspace compiles | `cargo check --workspace --lib` | exit 0 |
| Runtime tests pass | `cargo test -p librefang-runtime` | exit 0; existing tests unchanged |
| Skills tests pass | `cargo test -p librefang-skills` | exit 0; manifest backward compat verified |
| WIT validates | `wasm-tools component wit crates/librefang-skills/wit/` | parses clean |
| Smoke (compile + load + run) | `scripts/test-wasm-toolchain.sh` inside dev image | 6/6 green (TS/JS/C via execute, Rust/Python/Go via execute_component) |
| BossFang brand audit | `python3 scripts/enforce-branding.py --check` | exit 0 |
| End-to-end: install a Component skill, run it | manual smoke per `docs/development/plugin-host.md` | works |

---

## Suggested execution order summary

```
C-001 wasmtime 44→45 (must land alone)
      │
      ▼
C-002 WIT host world (new files only — zero conflict)
      │
      ▼
C-003 wit_host shim (depends on WIT)
      │
      ▼
C-004 execute_component (depends on shim) ──┬── C-005 manifest capabilities
                                            │
                                            ▼
                                        C-006 AOT cache (depends on Component path)
                                            │
                                            ▼
                                        C-007 smoke load+execute (depends on Component path + AOT cache to be optional)
                                            │
                                            ▼
                                        C-008 docs
```

C-004 and C-005 can land in either order once C-003 is in. C-006 depends
on C-004 only. C-007 depends on C-004 (capability subset tests in C-005
exercise the same path).

---

## Next action

1. After phase-4 PR #56 merges to `main`, run `/kbd-execute C-001`.
2. Each subsequent change opens its own focused PR. Don't bundle the
   wasmtime bump with anything else.
