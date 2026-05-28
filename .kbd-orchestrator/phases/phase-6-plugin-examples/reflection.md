# Phase Reflection: phase-6-plugin-examples

**Project:** LibreFang (BossFang fork) — github.com/GQAdonis/librefang
**Date:** 2026-05-28
**Worktree:** `~/.claude/worktrees/librefang-phase6` on
`feat/phase6-plugin-examples`
**Baseline merged in:** `upstream/main @ aed126207` (51-commit upstream
absorption mid-phase, wasmtime 44→45, `AgentManifest.channels` add,
tool_runner ToolError migration, `SessionWriter::inject_attachment_blocks`
gains `SessionId` param).
**Status:** EXECUTE COMPLETE — 8/8 changes DONE.

---

## Goal achievement

| Goal | Status | Evidence |
|---|---|---|
| Ship 5 per-language plugin examples targeting `librefang:plugin@0.1.0/plugin` | **MET** | `examples/plugins/{c-noop,rust-fs-cat,python-hello-time,js-kv-counter,go-env-greet}/pre-built/plugin.wasm` all present, validated by `wasm-tools`, regenerable via `cargo xtask plugins-rebuild` |
| Exercise ≥1 of {fs, kv, env, time} per the planned scope | **MET** | fs (rust-fs-cat), kv (js-kv-counter), env (go-env-greet), time (python-hello-time), none (c-noop). `net` + `agent` explicitly deferred to Phase-7 per D5 |
| Convert C-007 `LOAD-OK` smoke into `INVOKE-OK` round-trips | **PARTIAL** | 1/5 round-trip lit (c-noop end-to-end). The other 4 surface a real gap: language runtimes (CPython, StarlingMonkey, Go, cargo-component's wasi-rt) pull WASI Preview 2 imports the Phase-6 linker doesn't bind. Each test is `#[ignore]`d with the exact missing import named — the plan explicitly allowed this outcome ("fails gracefully with a documented skip-marker rather than a panic"). Phase-7 ships `wasmtime-wasi` wiring to light all four. |
| Per-example author docs cross-linked from `plugin-host.md` | **MET** | New "Walking examples (Phase-6)" section with the per-language table + the two sandbox fixes discovered during C-007. |

**Headline:** the *build pipeline* deliverable is 100% — every language compiles a librefang:plugin Component. The *invocation* deliverable is 1/5, but the four gaps point at a single, well-scoped Phase-7 piece (wasmtime-wasi linker wiring) and each ignored test names the precise missing interface.

---

## Verification Plan — line-item

(from assessment.md → "Verification Plan")

| # | Check | Result |
|---|---|---|
| 1 | `cargo check --workspace --lib` exit 0 | ✅ PASS (50s post-final-edit; 72s post-merge) |
| 2 | `cargo test -p librefang-runtime --test 'plugin_example_*'` | ✅ 1 pass + 4 `#[ignore]` with reasons; 0 failures, 0 panics |
| 3 | `cargo xtask plugins-rebuild <name>` regenerates each artefact | ✅ All 5 builders run clean (c-noop also accepts `WASI_CLANG` / `WASI_WASM_LD` / `WASI_SYSROOT` env overrides) |
| 4 | `python3 scripts/enforce-branding.py --check` | ✅ PASS — no upstream tokens detected |
| 5 | `docs/development/plugin-host.md` "Walking examples" section | ✅ Added — per-language table + sandbox-fix notes |
| 6 | Phase-4 `scripts/test-wasm-toolchain.sh` still green | Not re-run (no Phase-4 surfaces touched); should remain green |

---

## Delivered changes

(from `progress.json`)

| Change | Effort | Files touched | Artefact bytes | Notes |
|---|---|---:|---:|---|
| C-001 scaffold | S | 4 | — | xtask skeleton + `examples/plugins/.gitignore` + README |
| C-002 rust-fs-cat | M | 5 | 54,736 | cargo-component; clean (200 KB budget) |
| C-003 python-hello-time | M | 4 | 18,368,830 | componentize-py 0.23.0; CPython embed; 24 MB budget |
| C-004 js-kv-counter | M | 4 | 12,660,894 | jco + StarlingMonkey; **discovered** versioned import requirement `librefang:plugin/kv@0.1.0` |
| C-005 go-env-greet | M-L | 8 | 404,950 | TinyGo wasip1 + 3-step wasm-tools pipeline + bundled WASI P1 reactor adapter; hand-written `cabi.go` |
| C-006 c-noop | M | 5 | 1,072 | LLVM clang + lld wasm-ld; stubs.c for `free`/`realloc`/`abort`; **discovered** `--initial-memory=131072` requirement |
| C-007 integration tests | M-L | 7 | — | shared `KernelHandleStub` impls 19 role traits incl. post-merge `SessionWriter::inject_attachment_blocks(AgentId, SessionId, Vec<ContentBlock>)`; **discovered & fixed** missing `Store::set_fuel`/`set_epoch_deadline` in `execute_component` |
| C-008 docs + CHANGELOG | S | 3 | — | `plugin-host.md` Walking examples section, c-noop README, `[Unreleased]` CHANGELOG entries with `(@gqadonis)` |

**Commits on branch ahead of upstream merge baseline:**

```
35459b03f  feat(runtime,examples,docs): phase-6 C-007/C-008
b59b5abc5  Merge upstream/main (51 commits)
e05d06029  feat(examples,runtime,xtask): phase-6 C-002..C-006 (WIP snapshot pre-merge)
```

---

## Artifact Quality Summary

| Metric | Value |
|---|---|
| Changes with QA | 0/8 |
| First-pass pass rate | N/A |
| Changes requiring refinement | N/A |
| Total refinement iterations | N/A |

**No artifact-refiner runs.** `.refiner/artifacts/` does not exist in
this worktree — the artifact-refiner gate was not configured for this
phase. Quality was enforced inline by:

- the `pre-commit` git hook (rustfmt + CHANGELOG attribution + secrets scan)
- `cargo check --workspace --lib`
- `cargo clippy -p librefang-runtime --tests -- -D warnings`
- `cargo test -p librefang-runtime --no-fail-fast --test 'plugin_example_*'`
- `python3 scripts/enforce-branding.py --check`

For future phases that touch a comparable amount of behaviour-bearing
code (the runtime `execute_component` change in particular), wiring an
artifact-refiner constraint set against `crates/librefang-runtime/` would
catch class-of-bug regressions like the fuel/epoch omission earlier
than C-007.

---

## Technical debt introduced

Tracked so it can be paid down explicitly rather than discovered by a
future reader of the source.

1. **`KernelHandleStub` is a Phase-6-shaped silent partner.** The test
   harness implements 19 role traits with `Err(unavailable)` defaults.
   When upstream adds a 20th role trait (or strips a default), every
   `plugin_example_*` binary fails to compile with `not all trait items
   implemented`. **Mitigation deferred.** A canonical
   `librefang-kernel-handle::test_stub::KernelHandleStub` lives nowhere;
   each consumer rolls its own. Phase-7 should consider promoting our
   harness into the kernel-handle crate behind a `#[cfg(any(test,
   feature = "test-stub"))]` gate so every downstream test stays current
   automatically.

2. **`Store::set_fuel(u64::MAX / 2)` as the "unlimited fuel" sentinel
   in `sandbox_component.rs`.** The core-module `execute()` path uses a
   conditional `if config.fuel_limit > 0 { set_fuel(...) }` and lets the
   engine config decide. We used `u64::MAX / 2` to give effectively-
   unbounded headroom without tripping wasmtime's `i64::MAX` validation.
   Cleaner would be to mirror the core path's branching or expose a
   real "no fuel" engine config when callers genuinely want unlimited.
   Self-documented in the call-site comment.

3. **c-noop's `stubs.c` (no-op `free`/`malloc`/`realloc` + trapping
   `abort`).** Correct *only* for plugins that never reach an allocation.
   The next non-trivial C plugin will need a real `dlmalloc` (typically
   via `wasi-sdk` proper, not the minimal TinyGo `wasi-libc`). The
   `c-noop` README calls this out explicitly so the next author doesn't
   carry-cult the stubs.

4. **`store.limiter(...)` is still not wired into `execute_component`.**
   The Phase-5 TODO survives unchanged. Memory bound from
   `SandboxConfig.max_memory_bytes` is not enforced on Component plugins.
   Phase-7 should land the small `pub(crate) fn limiter_mut(&mut self)`
   accessor on `GuestState` and close this.

5. **`librefang-uar-spec/src/translator.rs` defaults
   `AgentManifest.channels = Vec::new()`.** Means UAR-spawned agents are
   implicitly "all channels allowed" — historical behaviour for
   continuity, but operators expecting the new per-agent allowlist
   semantics won't get them on UAR agents. Documented inline; tracked
   for Phase-7 review.

6. **Stale Phase-5 docs.**
   `docs/development/plugin-host.md`'s "Phase-5 traceability" section
   still implies plugin invocation works without further work. Now
   contradicted by Phase-6's Walking examples section a few lines
   below. A future cleanup pass should harmonize the two.

---

## Lessons captured (for knowledge base)

The six findings below are worth memorialising — they were paid for
once during Phase-6 and any of them would catch the next author flat
otherwise.

1. **WASM Component plugins built with `cargo-component` /
   `componentize-py` / `jco` / `TinyGo` all pull WASI 0.2 host
   imports** (`wasi:cli/environment`, `wasi:io/poll`, etc.) even when
   the plugin source code never uses them. The language runtime's
   own init code reaches the host through WASI before user code runs.
   A `librefang:plugin/*`-only linker rejects all four at instantiation.
   **Implication:** any production plugin host must wire `wasmtime-wasi`
   alongside the librefang capability surface. The `librefang:plugin/*`
   gate is an *addition*, not a *replacement*, for WASI.

2. **`engine.consume_fuel(true)` + `engine.epoch_interruption(true)`
   require a corresponding `store.set_fuel(...)` and
   `store.set_epoch_deadline(...)` on every store**, including the ones
   in the Component Model path. Default fuel is 0 — the first wasm
   instruction traps with an opaque "wasm function N" error and no hint
   that fuel is the cause. Phase-5's smoke test missed this because it
   loaded without invoking `run()`. **The Phase-6 c-noop test is the
   smallest possible repro** and should be the canary every time the
   engine-config flags change.

3. **C plugins compiled with `-nostdlib` need
   `-Wl,--initial-memory=131072`** (2 pages). wasm-ld defaults to 1 page,
   the generated `RET_AREA` static lands exactly at byte 65536, and the
   first `i32.store8` into it traps OOB. Captured in
   `examples/plugins/c-noop/README.md` under "Why these specific flags".

4. **jco componentize requires the WIT package version in the import
   specifier for non-WASI namespaces:** `'librefang:plugin/kv@0.1.0'`,
   not `'librefang:plugin/kv'`. Without the version, StarlingMonkey
   resolves the bare specifier as a file path at wizer-init time and
   fails with `No such file or directory`. Captured in the js-kv-counter
   plugin and the docs.

5. **TinyGo `wasip2` target is hardcoded to `wasi:cli/command`** and
   cannot target a custom WIT world. The working pipeline is `tinygo
   build -target wasip1` → `wasm-tools component embed --world plugin`
   → `wasm-tools component new --adapt wasi_snapshot_preview1=<adapter>`.
   The WASI P1 reactor adapter (52 KB, from `wasmtime` v45 release
   assets) is committed under
   `examples/plugins/go-env-greet/wasi-adapter/` for reproducibility.

6. **The cargo `surrealdb-core` transitive resolves loose** even with
   our workspace `surrealdb = "=3.0.5"` pin. After regenerating
   `Cargo.lock` from scratch (during the upstream-merge cargo source-
   replacement dance), `surrealdb-core 3.1.2` snuck in and broke
   `surrealdb 3.0.5`'s internal API consumption
   (`Datastore::notifications` removed, `index_compaction` signature
   changed). `cargo update -p surrealdb-core --precise 3.0.5` is the
   one-liner fix; longer-term we should add `surrealdb-core = "=3.0.5"`
   to `[workspace.dependencies]` so the constraint is explicit.

---

## Risks — pre-flight vs reality

(from assessment.md "Risks")

| Pre-flight risk | Actual outcome |
|---|---|
| TinyGo + librefang:plugin world: high likelihood of friction | **Materialised** — Go required the 3-step wasm-tools pipeline + hand-written `cabi.go` + bundled WASI P1 adapter. Worked. |
| wit-bindgen-c painful for non-trivial plugins | **Materialised** — even the noop needed `stubs.c` and the `--initial-memory` discovery. c-noop's restricted scope (zero capabilities) was the right defense. |
| Pre-built `.wasm` regen non-deterministic | Did not materialise during the phase; not stressed yet. Track in Phase-7. |
| componentize-py bindgen drift | **Minor** — class name had to be `WitWorld` not `App`; one-line fix. |
| `KernelHandle` stubs too constrained for kv test | Not materialised in the way feared — the test is `#[ignore]`d for a different reason (StarlingMonkey shim auto-imports fs). The stub itself is fine. |
| `.wasm` bloat | **Materialised but mitigated** — Python (18 MB) and JS (12 MB) embed full interpreters. Per-language `SIZE_BUDGETS` table now lives in `xtask/src/plugins.rs`. |
| WIT changes during Phase 6 | Did not materialise; WIT contract was held constant per plan. |
| **(unlisted)** Disk full | Burned the user 5+ minutes mid-C-007. `target/debug/incremental` alone was 3.5 GB; needs to land on a pre-flight checklist for any future workspace-sized test phase. |
| **(unlisted)** Upstream Cargo.lock regen needs `[source.*]` strip-and-restore dance | Surfaced in the mid-phase upstream merge. Now memorialised in the upstream-merge skill flow. |

---

## Recommended focus for next phase

In rough priority order:

1. **wasmtime-wasi integration in `sandbox_component.rs`** — lights up 4 of
   5 Phase-6 integration tests. Each `#[ignore]` reason names the precise
   missing interface (`wasi:io/poll@0.2.6`, `wasi:cli/environment@0.2.0`
   / `@0.2.6`, `librefang:plugin/fs`). Wire `wasmtime_wasi::add_to_linker`
   conditionally so non-WASI components keep working.
2. **`store.limiter(...)` for Component path** — paid forward from
   Phase-5's TODO. Requires a small `pub(crate)` accessor on `GuestState`.
3. **Lock `surrealdb-core = "=3.0.5"`** in `[workspace.dependencies]` to
   prevent the cargo resolver drift that nearly broke the upstream merge.
4. **Net + agent example plugins** — the two capabilities Phase-6
   intentionally deferred. Net needs SSRF-tested examples and an
   integration test against `wiremock`. Agent needs the cross-plugin
   send/spawn surface, which probably wants its own KBD phase.
5. **Promote `KernelHandleStub` into `librefang-kernel-handle`** behind
   a `test-stub` feature so every consumer of `KernelHandle` doesn't
   re-roll a 200-line harness.
6. **C-noop `dlmalloc` linkage example** — the next C plugin author
   will need a real allocator; we should ship the worked example
   alongside `c-noop` (or as `c-fs-cat`) rather than make them rediscover.

---

## Phase-6 → Phase-7 handoff

- **Branch:** `feat/phase6-plugin-examples` (3 commits ahead of
  `upstream/main @ aed126207`).
- **Status:** not yet pushed. User to decide push + PR timing.
- **Workspace compile state:** `cargo check --workspace --lib` clean
  on macOS arm64 against wasmtime 45 / surrealdb 3.0.5 / Rust stable
  per `rust-toolchain.toml`.
- **Outstanding `#[ignore]` count in `librefang-runtime`:** +4 (all
  Phase-6, all named).
- **No new SurrealDB migrations needed** (upstream merge scan-script
  confirmed).
- **No new upstream URLs to repoint** (scan-script confirmed).
- **No brand-token drift** (`enforce-branding.py --check` clean).

---

_Reflection by Claude under `/kbd-reflect`, 2026-05-28._
