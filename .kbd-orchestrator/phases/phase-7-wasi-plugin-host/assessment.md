# Phase Assessment: phase-7-wasi-plugin-host

**Project:** LibreFang (BossFang fork) — github.com/GQAdonis/librefang
**Date:** 2026-05-28
**Status:** NEW PHASE — proposed.
**Worktree:** `~/.claude/worktrees/librefang-phase7` on
`feat/phase7-wasi-plugin-host`, based on `origin/main @ 2d5148af2`
(post-PR-#58, includes wasmtime 45 from the upstream merge).
**Prior context:**
[`phase-6-plugin-examples/reflection.md`](../phase-6-plugin-examples/reflection.md)
— Phase-6 shipped the build pipeline (5/5 examples regenerate via
`cargo xtask plugins-rebuild`) but only 1/5 integration tests light
green; the other 4 are `#[ignore]`d with documented missing-host-import
reasons.

---

## Goal

Take the four `#[ignore]`d integration tests in
`crates/librefang-runtime/tests/plugin_example_*.rs` from "skip with
documented reason" to "active green," and close the Phase-5 TODO that
left `store.limiter(...)` unwired on the Component path.

Concretely:

1. Wire **`wasmtime-wasi`** into `sandbox_component.rs` so any plugin
   whose language runtime (CPython, StarlingMonkey, Go,
   `cargo-component`'s wasi-rt) reaches for a `wasi:*` host interface
   finds it bound to a real implementation rather than failing
   instantiation.
2. Attach a `wasmtime::ResourceLimiter` to every Component store so
   `SandboxConfig::max_memory_bytes` is actually enforced on Component
   plugins (parity with the core-module `execute()` path).
3. Decide and implement how the **librefang capability list**
   (`SandboxConfig.capabilities`) gates what the wired `WasiCtx`
   actually allows the plugin to do — a plugin that grants only the
   `librefang:plugin/time` host capability must NOT be able to read
   `/etc/passwd` via `wasi:filesystem/types`.

## Out-of-scope (intentional)

- **Net + agent example plugins** (the two Phase-6-deferred caps).
  Pulling them in here would double the design surface; they get their
  own KBD phase.
- **`surrealdb-core = "=3.0.5"` workspace pin.** Phase-6 reflection
  recommended this; it's a one-line `[workspace.dependencies]` edit
  better landed as a standalone follow-up PR than bundled with a
  multi-day plugin-host change.
- **Promoting `KernelHandleStub` into `librefang-kernel-handle` behind
  a `test-stub` feature.** Separate cleanup PR; cheap once the test
  surface stabilises, not before.
- **`dlmalloc` linkage example for C plugins.** Better as an addition
  to `examples/plugins/` once the WASI wiring lands so the C plugin
  can actually allocate against a real heap.
- **WASI 0.3.** Upstream still pre-stable; `wasmtime-wasi` ships 0.2.
- **WASI Preview 1 retrofit.** Components in this workspace are 0.2.x
  via cargo-component / componentize-py / jco / TinyGo+adapter. The
  WASI P1 reactor adapter we bundle for go-env-greet bridges P1 →
  P2; we never bind P1 host imports directly.
- **wit-component / wac composition surface.** Out of scope; Phase-7
  is host-side runtime work.
- **AOT cache invalidation when WASI is added to the linker.** The
  existing cache key (`wasmtime version + sha256(source)`) already
  changes whenever the engine config changes; landing wasmtime-wasi
  doesn't touch the cache key shape itself. Phase-8 candidate if cache
  hits regress visibly.

---

## Summary — what the codebase has today

| Component | Lives at | State going into Phase-7 |
|---|---|---|
| Engine config | `crates/librefang-runtime/src/sandbox.rs::make_engine_config()` | `consume_fuel(true)` + `epoch_interruption(true)` — unchanged from Phase-6 |
| Per-store fuel + epoch | `crates/librefang-runtime/src/sandbox_component.rs::execute_component()` | **Wired in Phase-6** (`set_fuel`, `set_epoch_deadline`). Was the load-bearing fix that let c-noop run. |
| Per-store memory limiter | `crates/librefang-runtime/src/sandbox_component.rs:442` | **TODO survives** — comment says "deferred to keep this PR additive-only"; never wired. |
| `librefang:plugin/*` linker | `crates/librefang-runtime/src/sandbox_component.rs::add_to_linker_per_capability()` | 6 interfaces wired; capability-gated. Working. |
| **`wasi:*` linker** | (nothing) | **Not wired.** No `wasmtime-wasi` dep in the workspace; no `WasiCtx` field on `PluginHostState`. |
| `PluginHostState` | `crates/librefang-runtime/src/sandbox_component.rs:96` | Holds `GuestState` + `ResourceTable`. Needs to also impl `WasiView` to satisfy `wasmtime_wasi::add_to_linker_async`. |
| SkillManifest opt-in to Component path | `crates/librefang-skills/src/lib.rs::SkillManifest.host_capabilities` | `Vec<HostCapability>` with 6 variants. Pre-Phase-5 manifests deserialize without it and stay on core-module path. |
| Phase-6 integration tests | `crates/librefang-runtime/tests/plugin_example_*.rs` | 1 active (c-noop), 4 `#[ignore]` with precise reasons (see below). |
| KernelHandleStub harness | `crates/librefang-runtime/tests/support/plugin_example_harness.rs` | In-tree, Phase-6-shaped. Will need a `WasiView`-shaped extension if PluginHostState gains a WasiCtx. |

### The four ignored tests — exact reasons

(from `grep '#\[ignore' crates/librefang-runtime/tests/plugin_example_*.rs`)

| Test | Required host import | Provided by |
|---|---|---|
| `rust_fs_cat_copies_file` | `wasi:io/poll@0.2.6` | `wasmtime-wasi` (latest 0.2.x) |
| `python_hello_time_returns_ok` | `wasi:cli/environment@0.2.0` | `wasmtime-wasi` (with version aliasing or older shim) |
| `go_env_greet_returns_ok_with_env_var` | `wasi:cli/environment@0.2.6` | `wasmtime-wasi` (latest 0.2.x) |
| `js_kv_counter_increments_from_zero` | `librefang:plugin/fs@0.1.0` | **Not WASI** — StarlingMonkey preview2-shim auto-imports our own `fs` interface even when the JS source doesn't use it |

**Note:** js-kv-counter is the odd one out — its missing import is our
own `librefang:plugin/fs`, not a `wasi:*` import. Three possible fixes
(see D6).

---

## Codebase Scan Results

### Phase-6 entry point — `execute_component`

```
crates/librefang-runtime/src/sandbox_component.rs
  L 96–115   PluginHostState — needs WasiCtx + WasiView impl
  L 298–335  add_to_linker_per_capability — currently wires librefang:plugin/* only
  L 408      add_to_linker_per_capability(&mut linker, &options.host_capabilities)
  L 410–411  let state = PluginHostState::new(new_guest_state(...))
  L 412      let mut store = Store::new(&engine, state)
  L 421–442  Phase-6 set_fuel + set_epoch_deadline + TODO for store.limiter
  L 442–447  TODO(phase-5 follow-up): store.limiter wiring
```

### Core-module sandbox — what we're aiming for parity with

```
crates/librefang-runtime/src/sandbox.rs
  L 162–180  GuestState struct (limiter is module-private)
  L 460–512  execute() — wires fuel, epoch_deadline_callback,
             store.limiter, set_epoch_deadline. The dance Phase-6
             called out as "intricate but not safely portable".
```

The Component path is now ~80% there (fuel + epoch base done in
Phase-6); Phase-7 lifts the remaining ~20% (limiter + WASI).

### Capability surface — what `Capability::*` means today

```
crates/librefang-types/src/capability.rs (existing pre-Phase-7)
  FileRead(path), FileWrite(path), EnvRead(name),
  NetConnect(host), ShellExec, MemoryRead/Write(key),
  AgentMessage(id), AgentSpawn
```

Today `host_functions::dispatch` checks `state.capabilities` before
each host call. With WASI bound, a plugin could bypass that entirely
by calling `wasi:filesystem/types::open-at(...)` rather than
`librefang:plugin/fs::read(...)`. **D2/D7 below pin down how we
prevent that.**

### What `wasmtime-wasi 45.x` ships

(from `wasmtime-wasi` crate documentation; assessment-time verification)

- `WasiCtxBuilder` — builder for a per-store `WasiCtx` (env vars,
  preopens, stdio, network, clocks). Default-deny: nothing exposed
  until you call `.env(...)`, `.preopened_dir(...)`, etc.
- `wasmtime_wasi::p2::add_to_linker_async(&mut linker)` — wires every
  `wasi:*` 0.2.x interface in one call. Sync variant exists for
  non-async stores.
- `WasiView` trait — your store-state type implements this to expose
  `&mut WasiCtx` + `&mut ResourceTable`. Our `PluginHostState`
  already has the ResourceTable.
- Version aliasing: wasmtime's component-model linker accepts a
  component built against an older 0.2.x WIT against a newer 0.2.x
  host **iff** the component only uses interfaces that didn't change
  shape. CPython 0.2.0 → wasmtime-wasi 0.2.6 mostly works for
  `wasi:cli/environment` (no shape change in the `get-environment` /
  `get-arguments` signatures since 0.2.0).

### What `wasmtime::ResourceLimiterAsync` looks like at v45

Same trait as v44; `memory_growing` is the only method we care about
(table growth is unbounded by policy; see existing `MemoryLimiter` in
`sandbox.rs:136`). The Component path needs an async variant because
`Plugin::instantiate_async` is async — `ResourceLimiterAsync` is the
matching trait.

---

## Architectural Decision Points

### D1. Sync vs async wasmtime-wasi binding

`Plugin::instantiate_async` and `call_run` are both async in the
Component path. We MUST use `wasmtime_wasi::p2::add_to_linker_async`
(NOT the sync variant). This also implies `WasiView` rather than
`WasiCtxView`. **Recommended: async.** No real alternative.

### D2. WasiCtx scope vs librefang capability list

Three viable shapes:

| Option | What | Pros | Cons |
|---|---|---|---|
| **Deny-by-default WasiCtx** (recommended) | `WasiCtxBuilder` with NO preopens, NO env, NO inheriting stdio, NO network. Wired purely so language runtime init can call `wasi:cli/environment::get-environment` and get back an empty list, etc. | Plugin can't bypass librefang gating because WASI surface returns empty. | CPython init may fail or behave oddly with no env. Needs verification per language. |
| **Mirror librefang caps into WasiCtx** | When `FileRead("/tmp/x")` granted, add `/tmp/x` as a preopen. When `EnvRead("FOO")` granted, expose `FOO` in WasiCtx env. | One conceptual model: librefang grants → WASI exposes. | Lots of bridge code; per-call mapping is messy when librefang grants change shape (e.g. `EnvRead("*")`). |
| **WasiCtx open-by-default** | Inherit host env, stdio, full network. | Easiest. | Plugin can bypass every librefang capability check. Unacceptable. |

**Recommended: D2 = Deny-by-default WasiCtx**, augmented per-plugin by
new `SkillManifest` fields when an example/test genuinely needs e.g.
an env var visible to CPython init. This matches the security model
of librefang's own host_functions::dispatch — start from deny, opt in
explicitly.

### D3. How does a plugin opt in to WASI?

| Option | What |
|---|---|
| **Always on** (recommended) | Component path always binds the WASI Preview 2 linker. Plugins that don't import any `wasi:*` are unaffected. |
| Opt-in via SkillManifest | New field `wasi_preview2: bool`; only Components with `wasi_preview2 = true` get the WASI linker. |
| Opt-in via HostCapability variant | Add `HostCapability::WasiPreview2`. |

**Recommended: D3 = Always on.** WASI is part of "what a Component
Model plugin imports"; binding it is the host's job. Opt-in adds a
config knob nobody wants to think about. The deny-by-default WasiCtx
from D2 means turning it on doesn't open security holes.

### D4. Multi-version WASI (0.2.0 + 0.2.6)

Phase-6 saw python-hello-time importing `wasi:cli/environment@0.2.0`
(componentize-py 0.23 was built against an older WASI WIT) while
go-env-greet imports `wasi:cli/environment@0.2.6`. Three options:

| Option | What | Cost |
|---|---|---|
| **Latest-only host** (recommended) | Pin one wasmtime-wasi version, rely on wasmtime's per-import version aliasing. | None if wasmtime accepts the 0.2.0 import against 0.2.6 host (must verify per interface). |
| Multiple wasmtime-wasi versions | Vendor wasmtime-wasi 0.2.0 + 0.2.6 side-by-side. | Two copies in the dep graph; fragile. |
| Rebuild fixtures | Re-run componentize-py against a newer WIT to bump the embedded 0.2.0 to 0.2.6. | Pushes the version-management problem onto every plugin author. |

**Recommended: D4 = Latest-only host with verification per import.**
Verify in C-001 with a quick `wasmtime serve`-style smoke that the
0.2.0 → 0.2.6 alias resolves for `wasi:cli/environment.get-environment`
specifically. If it fails, fall back to option 2 (vendored older
wasmtime-wasi).

### D5. `store.limiter(...)` wiring shape

The TODO at `sandbox_component.rs:442` blocks on a
`pub(crate) fn limiter_mut(&mut GuestState) -> &mut MemoryLimiter`
accessor. Two ways to land it:

| Option | What |
|---|---|
| **Accessor on `GuestState`** (recommended) | Add the `pub(crate) fn limiter_mut(&mut self)` to `sandbox.rs::GuestState`. The Component path calls `store.limiter(|s| s.guest.limiter_mut())`. |
| Duplicate limiter on `PluginHostState` | Construct a separate `MemoryLimiter` directly in `PluginHostState`. |

**Recommended: D5 = Accessor on GuestState.** Single source of truth
for memory bounds across both sandbox paths.

### D6. js-kv-counter's `librefang:plugin/fs` import

StarlingMonkey's preview2-shim auto-imports our own fs interface even
though the JS source only calls `kv.get/set`. Three viable fixes:

| Option | What | Trade-off |
|---|---|---|
| Grant `HostCapability::Fs` in the test | Cheap one-line test edit. | Misleading: the skill.toml says only `kv` is needed, but the test pretends otherwise. |
| Tighten jco componentize config | Pass flags to drop unused interfaces from the StarlingMonkey shim. | Real fix. Requires `--disable-feature` exploration; jco may not expose the knob. |
| **Stub the unused interface** (recommended) | Always bind a noop `librefang:plugin/fs` implementation that returns `host-error("not granted")` when called. The plugin links fine; calling it errors at runtime in the same observable way as if the capability gate denied it. | Cleanest: matches the capability-gate semantic users already understand. |

**Recommended: D6 = Stub the unused interface.** Implementation: add
a parallel `add_to_linker_per_capability_with_stubs` that binds the
6 librefang:plugin interfaces unconditionally, where granted caps go
to the real dispatcher and ungranted caps go to a stub that returns
`HostError::CapabilityDenied`. js-kv-counter then activates with its
existing `host_capabilities = ["kv"]` declaration intact.

### D7. WASI capability gating

Tied to D2. If WasiCtx is deny-by-default (D2 recommendation),
WASI capability gating happens at builder time, not at call time:
the plugin can call `wasi:cli/environment::get-environment` freely,
but it returns an empty list because the WasiCtx wasn't given any
env vars. **Recommended: D7 = builder-time gating, follows D2.**
No per-call check needed in the WASI path.

### D8. Test fixture updates needed?

If D2 (deny-by-default WasiCtx) is adopted:

- **python-hello-time:** The plugin reads no env vars, only calls
  `time.now()`. CPython's startup might still try to read env vars
  for site-packages discovery etc.; the empty env may surface as a
  Python warning but shouldn't fail `run()`. **Verify in C-005.**
- **go-env-greet:** The plugin explicitly reads `GREETING_NAME`.
  With deny-by-default, `env.read("GREETING_NAME")` returns `none`
  via librefang:plugin/env (already capability-checked there), and
  also returns `none` via `wasi:cli/environment::get-environment`.
  Plugin already returns Ok in either case (see go-env-greet/main.go
  L34: `out.SetOK(struct{}{})` regardless of result). Test should
  still pass.
- **rust-fs-cat:** The plugin needs to read `/tmp/test-input.txt`
  and write `/tmp/test-output.txt`. cargo-component's wasi-rt only
  needs `wasi:io/poll` for runtime init (no actual fs calls during
  init). With deny-by-default WasiCtx, `wasi:io/poll` returns valid
  no-op results, then the plugin's actual fs reads/writes go through
  `librefang:plugin/fs` which is capability-checked normally.
  **Test should pass unchanged.**

---

## Verification Plan

After Phase-7 implements wasmtime-wasi + store.limiter + the librefang
plugin/fs stub:

1. `cargo check --workspace --lib` exit 0 (no breakage).
2. `cargo clippy --workspace --all-targets -- -D warnings` exit 0.
3. **`cargo test -p librefang-runtime --no-fail-fast --test 'plugin_example_*'`
   → 5/5 active, 0 ignored, 0 failed.** This is the load-bearing
   acceptance criterion.
4. Remove the four `#[ignore = "..."]` attributes; confirm the
   reasons-strings are deleted from source so a future grep doesn't
   find stale references.
5. Memory bound: add a new test
   `crates/librefang-runtime/src/sandbox_component.rs::tests`
   that loads a synthetic Component asking for >`max_memory_bytes`,
   asserts `SandboxError::Execution` (limiter rejection) rather than
   uncapped allocation.
6. `python3 scripts/enforce-branding.py --check` clean.
7. `cargo xtask plugins-rebuild` regenerates all 5 Phase-6 artefacts
   byte-deterministically (regression guard — no fixture rebuild needed).
8. WASI deny-by-default smoke: tiny ad-hoc Component that calls
   `wasi:filesystem/types::open-at("/etc/passwd")` should get a
   typed WASI error (no preopens), NOT a host panic.
9. Docs: `docs/development/plugin-host.md` gains a "WASI Preview 2 host
   integration" section explaining D2 + D3 + D7 so plugin authors know
   the WasiCtx is empty by default.
10. CHANGELOG: `[Unreleased] / ### Added` entry with `(@gqadonis)`.

---

## Risks

| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| `wasmtime-wasi 45.x` semver pairing with `wasmtime 45.0.0` has a hole (e.g. wasmtime-wasi requires 45.1+) | Low | Low | C-001 dep-check task: pick `wasmtime-wasi` version explicitly aligned with `wasmtime = "45"`. Run `cargo tree -p wasmtime-wasi` to confirm one copy of wasmtime in the graph. |
| WASI 0.2.0 → 0.2.6 version aliasing in wasmtime doesn't cover `wasi:cli/environment` | Medium | Medium | If alias breaks: vendor older wasmtime-wasi. Alternative: rebuild python-hello-time fixture (Phase-7 scope creep — record decision). |
| CPython init reads env vars or stdio paths we didn't expose; runs but produces noisy warnings | High | Low | Acceptable: the test asserts `Ok(())` from `run()`. Warnings on stderr are not failure. Document. |
| Adding wasmtime-wasi inflates `librefang-runtime`'s compile time noticeably | Medium | Low | Acceptable; measured against the baseline we already pay for wasmtime+component-model. CI lane will tell. |
| `WasiView` trait signature changes between wasmtime-wasi 45 minor versions | Medium | Low | Pin the wasmtime-wasi version `=` in workspace Cargo.toml mirroring how we pin `surrealdb = "=3.0.5"`. |
| AOT `.cwasm` cache invalidates everywhere because engine config now includes WASI imports | Medium | Low | Acceptable — first run after upgrade pays JIT cost, second run is back to AOT-fast. Document in CHANGELOG so operators aren't surprised. |
| Per-language fixture re-verification finds an unexpected interface beyond `wasi:cli/environment` and `wasi:io/poll` | Medium | Medium | C-005's per-language verify-loop is the safety net. Each test failing surfaces the precise missing import name (Phase-6 pattern). |
| `store.limiter` breaks an existing Phase-5 component test by capping at 16 MiB | Low | Low | C-004 grep + bump-config-if-needed. |
| **D6 stub interface for js-kv-counter changes semantics for legitimate plugins** | Low | Medium | Stubs return `HostError::CapabilityDenied`; this is identical to the existing capability-deny path. No semantic change for plugins that DO use a capability they've been granted. |
| Phase-7 work bumps integration test runtime to a noticeable fraction of CI | Low | Low | Tests load multi-MB Components; with JIT-on-first-run + AOT-on-second, runtime is dominated by the Python (18 MB) + JS (12 MB) compile. Document if it gets near 60s. |

---

## Decisions Required (before Plan phase)

1. **D1**: Async wasmtime-wasi binding (recommended) — confirm.
2. **D2**: Deny-by-default WasiCtx (recommended) vs mirror-librefang-caps vs open-by-default.
3. **D3**: Always-on WASI linker binding (recommended) vs opt-in.
4. **D4**: Latest-only wasmtime-wasi host + version aliasing
   (recommended) vs vendored multi-version vs fixture rebuilds.
5. **D5**: `pub(crate) fn limiter_mut()` accessor on GuestState
   (recommended) vs duplicate construction in PluginHostState.
6. **D6**: Stub unused librefang:plugin interfaces (recommended) vs
   grant-in-test vs tighten-jco-config.
7. **D7**: Builder-time WASI gating (follows D2).
8. **Scope**: confirm net + agent + surrealdb-core pin + KernelHandleStub
   promotion + dlmalloc C example all stay deferred per
   `current-waypoint.json` `scope_note`.

---

## Sources / prior art consulted

- `phase-6-plugin-examples/reflection.md` — the entire "Recommended
  focus for next phase" + "Technical debt introduced" sections.
- `phase-6-plugin-examples/progress.json` — C-007 verification
  results and `#[ignore]` reasons captured at completion time.
- `crates/librefang-runtime/src/sandbox_component.rs` — Phase-6
  baseline.
- `crates/librefang-runtime/src/sandbox.rs::execute()` — core-module
  parity reference.
- `crates/librefang-skills/src/lib.rs::SkillManifest` — opt-in field
  shape (no new field expected per D3).
- `crates/librefang-runtime/tests/plugin_example_*.rs` — the four
  `#[ignore]` reason strings naming the precise missing imports.
- Workspace `Cargo.toml` — wasmtime 45 pin established by upstream
  merge; wasmtime-wasi absent.
