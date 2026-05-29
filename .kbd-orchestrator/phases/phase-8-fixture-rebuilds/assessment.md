# Phase Assessment: phase-8-fixture-rebuilds

**Project:** LibreFang (BossFang fork) — github.com/GQAdonis/librefang
**Date:** 2026-05-29
**Status:** NEW PHASE — proposed.
**Worktree:** `~/.claude/worktrees/librefang-phase8` on
`feat/phase8-fixture-rebuilds`, based on `origin/main @ 1f3b3cd8c`
(post-PR-#59, Phase-7 merged).
**Prior context:**
[`phase-7-wasi-plugin-host/reflection.md`](../phase-7-wasi-plugin-host/reflection.md)
— Phase-7 shipped WASI Preview 2 host wiring; 3/5 plugin tests green.
Two remain `#[ignore]`d with precise Phase-8 root causes.

---

## Goal

Close the two remaining `#[ignore]`d integration tests by fixing their
root causes — both of which are **fixture build issues**, not sandbox
issues — and land three housekeeping items identified during Phase-6/7
that were explicitly deferred.

Concretely:

1. Rebuild **go-env-greet** with the correct `wasi_snapshot_preview1.reactor.wasm`
   adapter from the wasmtime v45 release (the Phase-7 build used the
   wrong adapter from `wit-bindgen-cli-0.57.1`).
2. Rebuild **js-kv-counter** with a `jco`/`componentize-js` version whose
   embedded StarlingMonkey targets WASI ≤ 0.2.6 (current `jco 1.20.0`
   embeds WASI 0.2.10 which causes a runtime ABI mismatch against
   `wasmtime-wasi 45.0.0`).
3. Add `surrealdb-core = "=3.0.5"` to `[workspace.dependencies]` so
   `cargo generate-lockfile` can never silently resolve past the
   `surrealdb 3.0.5` API surface.
4. Promote `KernelHandleStub` into `librefang-kernel-handle` behind a
   `#[cfg(any(test, feature = "test-stub"))]` gate so downstream
   integration tests don't each roll 200 lines of boilerplate.

## Out-of-scope (intentional)

- **Net + agent example plugins** — own phase; needs WIT + capability
  surface design that Phase-8 doesn't want to gate on.
- **WASI 0.3** — upstream pre-stable.
- **Watchdog/epoch-callback timeout** on Component path — deferred from
  Phase-5; not touched here.
- **Outbound HTTP capability gate** — Phase-9; Phase-7 wired
  `WasiHttpCtx` deny-by-default; the real gate (connecting the
  librefang net capability to an outbound handler) is distinct work.
- **wasmtime version bump** — no new upstream release relevant to the
  current goals.

---

## Summary — what the codebase has today

### Ignored tests (the load-bearing gap)

```
crates/librefang-runtime/tests/plugin_example_go_env_greet.rs:29
#[ignore = "go-env-greet: TinyGo reactor adapter mismatch — Phase-8 fixture rebuild"]

crates/librefang-runtime/tests/plugin_example_js_kv_counter.rs:50
#[ignore = "js-kv-counter: StarlingMonkey WASI 0.2.10 vs wasmtime-wasi 0.2.6 runtime mismatch — Phase-8 fixture rebuild"]
```

### go-env-greet root cause (confirmed in Phase-7)

The `wasi_snapshot_preview1.reactor.wasm` adapter currently bundled at
`examples/plugins/go-env-greet/wasi-adapter/` is from
`wit-bindgen-cli-0.57.1/tests/` (94 KB). The correct adapter for
`wasmtime = "45"` is from the `wasi-preview1-component-adapter-provider`
crate at version **45.0.0** (52 KB, same family as the 43.0.2 version
already on disk in `~/.cargo/registry`).

The Phase-7 xtask builder is already correct: `-buildmode=c-shared
-scheduler=none`. Only the adapter needs replacing.

**Available adapter source (no network needed):**
`wasi-preview1-component-adapter-provider = "45.0.0"` is available on
crates.io. Add it as a `[dev-dependencies]` or standalone cargo project
and copy the bundled `artefacts/wasi_snapshot_preview1.reactor.wasm`
into `examples/plugins/go-env-greet/wasi-adapter/`.

### js-kv-counter root cause (confirmed in Phase-7)

`wasm-tools component wit examples/plugins/js-kv-counter/pre-built/plugin.wasm`
shows `import wasi:io/poll@0.2.10` — meaning the StarlingMonkey engine
embedded by `jco 1.20.0` + `@bytecodealliance/componentize-js@0.21.0`
targets WASI 0.2.10. wasmtime-wasi 45 serves WASI 0.2.6
(`include wasi:cli/imports@0.2.6` in `wasmtime-wasi-45.0.0/src/p2/wit/world.wit`).
Wasmtime's aliasing resolves interface names at link time but StarlingMonkey's
JIT has hard-coded ABI offsets for WASI 0.2.10 structures that differ
from 0.2.6 layouts.

**Fix:** rebuild `js-kv-counter` with a `componentize-js` version that
targets ≤ WASI 0.2.6. Research needed on which `componentize-js` version
is the right boundary — likely `0.18.x` or `0.19.x` (before 0.2.10 was
adopted). Phase-8 C-002 includes a verify-first step to check this.

**Alternative:** bump `wasmtime` / `wasmtime-wasi` to a version that
serves WASI 0.2.10. Risk: unknown API surface changes in a major wasmtime
version bump; deferred to a separate phase.

### surrealdb-core pin gap

`Cargo.toml` has `surrealdb = { version = "=3.0.5", ... }` but NOT
an explicit `surrealdb-core` pin. When `cargo generate-lockfile` (or
`cargo update`) runs on a freshly-stripped lock file, the resolver
pulls `surrealdb-core 3.1.x` (the latest matching `^3.0.5`'s internal
dep range), which breaks `surrealdb 3.0.5`'s API consumption. This was
hit during the Phase-6 upstream merge and fixed with
`cargo update -p surrealdb-core --precise 3.0.5` but the root cause
(no `=` pin on `surrealdb-core`) was not addressed.

**Current state:** `Cargo.lock` has the right version locked, but any
lock-file re-generation would break the build.

### KernelHandleStub location

```
crates/librefang-runtime/tests/support/plugin_example_harness.rs
```

This file contains 200+ lines implementing all 19 `KernelHandle` role
traits. It's `include!`d / `#[path]`d from each `tests/plugin_example_*.rs`
test binary. There is no canonical re-usable stub anywhere in
`librefang-kernel-handle`. Every new test crate or integration test binary
that needs a `KernelHandle` must either copy this file or depend on
`librefang-runtime` (a heavyweight transitive dep) to share it.

**Current state:** the stub exists but is siloed in `librefang-runtime`.

---

## Architectural Decision Points

### D1. Adapter source for go-env-greet

| Option | What | Cost |
|---|---|---|
| **Add `wasi-preview1-component-adapter-provider = "45.0.0"` as a build-time dep** (recommended) | Cargo fetches and unpacks the 45.0.0 crate; the xtask builder reads the adapter bytes from `~/.cargo/registry`. Adapter is not committed to git. | Clean — adapter version tracks workspace wasmtime pin automatically |
| Commit the 52 KB binary | Copy from the crate artefacts and commit to `wasi-adapter/`. | Simple; already the current (wrong) pattern. Works but is brittle: must be manually updated every wasmtime bump |
| Build from source | Clone wasmtime, run `cargo build -p wasi-preview1-component-adapter`. | Expensive; out of scope |

**Recommended: D1 = cargo dep + xtask reads from registry.** The adapter
binary is deterministic given the crate version, so not committing it keeps
git clean and ties the version to the workspace wasmtime pin automatically.
The xtask builder finds it via `cargo metadata` or a hard-coded registry
path using `CARGO_HOME`.

### D2. js-kv-counter rebuild approach

| Option | What | Verify step |
|---|---|---|
| **Downgrade to `componentize-js 0.19.x` via npx** (recommended) | `npx --yes @bytecodealliance/componentize-js@0.19.4 ...` — runs the older version without touching the globally-installed jco | Check `wasm-tools component wit js-kv-counter/pre-built/plugin.wasm | grep wasi:io/poll` — target `@0.2.6` |
| Downgrade global `jco` | `npm install -g jco@<version>` | Disruptive to other projects; not recommended |
| Pin `jco` version in a local `package.json` | Add `examples/plugins/js-kv-counter/package.json` with `jco` version pinned | Adds a lockfile; overkill for a single component |
| Upgrade wasmtime to ≥ 0.2.10 | Not a fixture issue — a major wasmtime upgrade; Phase-9 | Unknown API surface changes |

**Recommended: D2 = `npx` with pinned `componentize-js 0.19.4`.**
`componentize-js 0.19.x` predates the WASI 0.2.10 bump; the xtask builder
is updated to use `npx @bytecodealliance/componentize-js@0.19.4` instead
of the globally-installed `jco`. The verify-first step confirms the rebuilt
component imports `wasi:io/poll@0.2.6` before running the test.

### D3. surrealdb-core pin shape

Two approaches:

| Option | What |
|---|---|
| **`surrealdb-core = { version = "=3.0.5", default-features = false }`** (recommended) | Explicit pin in `[workspace.dependencies]`; mirrors the `surrealdb` pin |
| Comment in `Cargo.toml` only | Explains the risk but doesn't enforce it |

**Recommended: D3 = explicit `=3.0.5` pin in `[workspace.dependencies]`.**

### D4. KernelHandleStub target crate

| Option | What | Trade-off |
|---|---|---|
| `librefang-kernel-handle` with `feature = "test-stub"` | Lives where it belongs — alongside the traits it implements | Adds `serde_json`, `async-trait` to `librefang-kernel-handle`'s dev-dep tree |
| `librefang-testing` (existing testing utilities crate) | Already has dev-time utilities; clean separation | Adds a dep from every test that needs the stub |
| Leave in `librefang-runtime/tests/support/` | Zero work; but stays siloed | Stays siloed |

**Recommended: D4 = `librefang-kernel-handle` with `feature = "test-stub"`.**
The stub lives next to the trait it implements; feature-gating keeps it
off production builds; the existing `#[cfg(test)]` mod in
`librefang-kernel-handle/src/lib.rs` (which already has `StubKernel`)
shows the pattern is accepted there.

---

## Verification Plan

After all 4 changes:

1. `cargo check --workspace --lib` exit 0.
2. `cargo clippy --workspace --all-targets -- -D warnings` exit 0.
3. **`cargo test -p librefang-runtime --no-fail-fast --test 'plugin_example_*'`
   → 4 active-green, 1 still ignored (go-env-greet or js-kv-counter if
   only one fixture is fixable), 0 failures.** Ideally 5 active-green.
4. `grep '#\[ignore' crates/librefang-runtime/tests/plugin_example_*.rs`
   should return zero matches (or exactly the unfixable subset with new
   reasons).
5. `cargo generate-lockfile` (or `cargo update -p surrealdb`) does NOT
   upgrade `surrealdb-core` past 3.0.5.
6. New stub test: `cargo test -p librefang-kernel-handle --features test-stub`
   builds and the `StubKernel` moved from the inline `#[cfg(test)]` mod
   passes its existing compile proofs.
7. `python3 scripts/enforce-branding.py --check` clean.
8. `cargo xtask plugins-rebuild go-env-greet` reproduces the rebuilt wasm
   deterministically (same or similar byte count).
9. `cargo xtask plugins-rebuild js-kv-counter` produces a component with
   `wasi:io/poll@0.2.6` (not 0.2.10).

---

## Risks

| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| `componentize-js 0.19.4` doesn't produce a working component for our WIT | Medium | Medium | Verify-first: build and check WIT version before removing the `#[ignore]`. Fall back to 0.18.5 if 0.19.4 doesn't work. |
| `wasi-preview1-component-adapter-provider 45.0.0` is not available via `cargo add` / registry | Low | Low | Fallback: commit the 52 KB adapter from the 43.0.2 crate (same adapter family, slightly older version — check whether it works with TinyGo 0.41.1 c-shared output) |
| `KernelHandleStub` promotion breaks an existing test that imports it from `librefang-runtime` | Medium | Low | The `#[path]`-based import pattern is local to the runtime tests; after promotion, update the import path in those tests |
| `surrealdb-core` pin conflicts with a transitive dependency | Low | Medium | Run `cargo tree -p surrealdb-core` before and after to confirm one copy |

---

## Decisions Required (before Plan phase)

1. **D1**: Adapter source — cargo dep (recommended) vs committed binary.
2. **D2**: js-kv-counter rebuild — `npx componentize-js@0.19.4` (recommended) vs global jco downgrade vs wasmtime upgrade.
3. **D3**: `surrealdb-core = "=3.0.5"` explicit pin (recommended).
4. **D4**: KernelHandleStub target — `librefang-kernel-handle` feature (recommended) vs `librefang-testing`.
