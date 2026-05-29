# Phase Plan: phase-7-wasi-plugin-host

**Phase:** phase-7-wasi-plugin-host
**Date:** 2026-05-28
**Backend:** native KBD (no OpenSpec, no evolver bridge)
**Worktree:** `~/.claude/worktrees/librefang-phase7` on
`feat/phase7-wasi-plugin-host`
**Based on:** `assessment.md` written same date.

---

## Decisions baked

Per the assessment's recommended path, with one explicit "verify first" on D4:

| # | Decision | Baked as |
|---|---|---|
| D1 | wasmtime-wasi binding | **Async** (`wasmtime_wasi::p2::add_to_linker_async`) |
| D2 | WasiCtx scope | **Deny-by-default** (no preopens, no env, no stdio inherit, no network) |
| D3 | WASI linker opt-in | **Always-on** (every Component store gets WASI bound) |
| D4 | 0.2.0 vs 0.2.6 | **Latest-only**, with C-001 dedicated to the version-aliasing smoke before sinking time into anything else |
| D5 | `store.limiter` shape | **`pub(crate) fn limiter_mut(&mut self) -> &mut MemoryLimiter` on `GuestState`** |
| D6 | js-kv-counter shim quirk | **Stub-bind ungranted librefang:plugin interfaces** with `HostError::CapabilityDenied` |
| D7 | WASI capability gating | **Builder-time** (deny-by-default WasiCtx is the gate; no per-call checks added) |
| Scope | Net + agent + surrealdb-core pin + KernelHandleStub promotion + dlmalloc | **Deferred** to follow-up phases / standalone PRs |

D4 is the only one with a verify-first guard. If wasmtime's per-import
version aliasing can't satisfy `wasi:cli/environment@0.2.0` against a
`wasmtime-wasi` 0.2.6 host, C-001 surfaces it before any other work
commits — the fallback (vendored older `wasmtime-wasi` or fixture
rebuild) becomes its own decision, not a Phase-7 surprise.

---

## Ordered change list

8 changes. C-002 and C-001 can in principle run in parallel (the
limiter work doesn't touch WASI), but the sequence below keeps a
single-author flow simple. Branch is the same worktree throughout.

### C-001 — Workspace dep: `wasmtime-wasi` + version-alias smoke

**Branch:** same worktree
**Files:**
- `Cargo.toml` (workspace `[workspace.dependencies]`)
- `crates/librefang-runtime/Cargo.toml`
- `crates/librefang-runtime/src/sandbox_component.rs` (smoke test only)

**Depends on:** none
**Effort:** S
**Recommended agent:** `claude`

- [ ] Add `wasmtime-wasi = "=<version>"` to workspace, pinned `=`
  mirroring how `surrealdb` is pinned (per the assessment risk
  about WasiView signature drift between minor versions). Pick the
  exact version that pairs with `wasmtime 45.x`; verify via
  `cargo tree -p wasmtime-wasi -p wasmtime` shows exactly one
  `wasmtime` instance in the graph.
- [ ] Forward to `librefang-runtime` as `wasmtime-wasi = { workspace = true }`.
- [ ] Add a new `#[tokio::test]` in
  `sandbox_component.rs::tests::wasi_version_alias_smoke` that builds
  a minimal Component (via the existing `wat` dev-dep) importing
  `wasi:cli/environment@0.2.0`, instantiates against
  `wasmtime_wasi::p2::add_to_linker_async`, calls a no-op export,
  asserts no instantiation error. If this fails, **STOP** and
  re-decide D4 before continuing.

**Done when:** `cargo check --workspace --lib` exit 0 AND the smoke
test passes (proving the version alias works against
`wasi:cli/environment` specifically).

---

### C-002 — `pub(crate) fn limiter_mut()` + wire `store.limiter` in Component path

**Branch:** same worktree
**Files:**
- `crates/librefang-runtime/src/sandbox.rs` (`GuestState` accessor)
- `crates/librefang-runtime/src/sandbox_component.rs` (`execute_component`
  + replace the surviving `TODO(phase-5 follow-up)` block)

**Depends on:** none (independent of WASI work)
**Effort:** S
**Recommended agent:** `claude`

- [ ] Add `pub(crate) fn limiter_mut(&mut self) -> &mut MemoryLimiter`
  to `GuestState` in `sandbox.rs`. Pure mechanical accessor.
- [ ] In `execute_component`, after `let mut store = Store::new(...)`
  and before `store.set_fuel(...)`, insert:
  `store.limiter(|s| s.guest.limiter_mut());`
- [ ] Delete the `TODO(phase-5 follow-up)` comment block (lines
  ~442–447 of `sandbox_component.rs`) — its replacement is the new
  one-liner.
- [ ] No new tests yet — C-007 ships the memory-bound enforcement
  test that proves the wiring works end-to-end.

**Done when:** `cargo check --workspace --lib` exit 0; existing
`bindgen_emits_plugin_and_add_to_linker` and Phase-6 c-noop test
still pass; comment grep confirms no `TODO(phase-5 follow-up)` strings
survive in `sandbox_component.rs`.

---

### C-003 — `WasiCtx` field + `WasiView` impl on `PluginHostState`

**Branch:** same worktree
**Files:**
- `crates/librefang-runtime/src/sandbox_component.rs` (struct +
  constructor + trait impl)

**Depends on:** C-001
**Effort:** S–M
**Recommended agent:** `claude`

- [ ] Add `pub wasi: wasmtime_wasi::p2::WasiCtx` to `PluginHostState`.
- [ ] In `PluginHostState::new(guest)`, build the WasiCtx via
  `WasiCtxBuilder::new().build()` — **no `.env(...)`, no
  `.preopened_dir(...)`, no `.inherit_stdio()`, no
  `.allow_tcp(true)`, no `.allow_udp(true)`** — that's the literal
  deny-by-default per D2.
- [ ] `impl wasmtime_wasi::p2::WasiView for PluginHostState` returning
  `&mut self.wasi` and `&mut self.table`.
- [ ] No call-site changes yet — C-004 wires the linker.

**Done when:** `cargo check --workspace --lib` exit 0; `PluginHostState`
satisfies `WasiView` (compile proof).

---

### C-004 — Always-on `wasmtime_wasi::p2::add_to_linker_async` in `execute_component`

**Branch:** same worktree
**Files:**
- `crates/librefang-runtime/src/sandbox_component.rs` (one new
  call in `execute_component`, immediately before
  `add_to_linker_per_capability`)

**Depends on:** C-003
**Effort:** S
**Recommended agent:** `claude`

- [ ] Add `wasmtime_wasi::p2::add_to_linker_async(&mut linker)
  .map_err(|e| SandboxError::Execution(e.to_string()))?;` directly
  after `let mut linker = Linker::<PluginHostState>::new(&engine);`,
  before the librefang per-capability binding.
- [ ] **No skill manifest changes** (per D3 always-on). Existing
  `host_capabilities = []` plugins (like c-noop) get WASI bound for
  free; they don't import any `wasi:*` interface so nothing changes
  observably.
- [ ] Inline doc comment on the new call linking to the assessment's
  D2/D3/D7 explanation so the next reader doesn't reopen the
  decision.

**Done when:** `cargo check --workspace --lib` exit 0; existing
Phase-6 c-noop integration test still passes
(`cargo test -p librefang-runtime --test plugin_example_c_noop`); a
quick `wasm-tools component wit` of `c-noop/pre-built/plugin.wasm`
still works (no component-shape changes).

---

### C-005 — Stub-bind ungranted `librefang:plugin/*` interfaces (D6)

**Branch:** same worktree
**Files:**
- `crates/librefang-runtime/src/sandbox_component.rs`
  (refactor `add_to_linker_per_capability` → new
  `add_librefang_linker_with_stubs`)
- `crates/librefang-runtime/src/wit_host.rs` (new
  `HostErrorRepr::capability_denied("<iface>")` helper if not
  already there)

**Depends on:** C-004 (need the linker shape stable from WASI add)
**Effort:** M
**Recommended agent:** `claude`

- [ ] Rename the function to clarify intent:
  `add_librefang_linker_with_stubs(linker, granted_capabilities)`.
  Old name stays via a `#[deprecated]` re-export only if any
  out-of-crate caller exists (grep first; expected: none).
- [ ] For each of the 6 librefang interfaces (fs, net, kv, agent,
  env, time), unconditionally call its `add_to_linker`. The
  per-interface `Host` trait impl on `PluginHostState` already
  routes to `dispatch`, which checks `state.guest.capabilities`
  before doing the actual work and returns `HostError::CapabilityDenied`
  when not granted. **This means stubbing happens at the dispatch
  layer, not the linker layer — preferred because it reuses one
  code path for both denied and not-declared-in-manifest cases.**
- [ ] **No new "stub" types needed** — the existing `dispatch` error
  path IS the stub. The "refactor" here is mostly conceptual:
  drop the `capabilities` parameter, drop the `match cap { ... }`
  scaffold, bind all 6 unconditionally.
- [ ] Update `add_to_linker_per_capability`'s sole call site in
  `execute_component` accordingly.
- [ ] Update the inline test
  `add_to_linker_per_capability_with_empty_caps_succeeds` (if any)
  to reflect the new always-bind-all shape.

**Done when:** `cargo check --workspace --lib` exit 0; the existing
Phase-5 capability-gating tests in `sandbox_component.rs::tests`
still pass with the trivially-adjusted assertions; c-noop test
still green.

---

### C-006 — Remove all 4 `#[ignore]` markers + verify 5/5 green

**Branch:** same worktree
**Files:**
- `crates/librefang-runtime/tests/plugin_example_rust_fs_cat.rs`
- `crates/librefang-runtime/tests/plugin_example_python_hello_time.rs`
- `crates/librefang-runtime/tests/plugin_example_js_kv_counter.rs`
- `crates/librefang-runtime/tests/plugin_example_go_env_greet.rs`

**Depends on:** C-004 (WASI bound) + C-005 (librefang stubs)
**Effort:** M–L (per-language debug is unpredictable)
**Recommended agent:** `claude`

- [ ] Remove `#[ignore = "..."]` from all four tests.
- [ ] Remove the matching reason-strings from the prefix comments
  so future readers don't grep a stale "needs Phase-7" note.
- [ ] Run each test and triage:
  - [ ] **rust-fs-cat**: with WASI bound, `wasi:io/poll` should
    resolve. Plugin proceeds to its `fs::read` / `fs::write` via
    `librefang:plugin/fs` (capability-gated as before). Test sets
    up `/tmp/test-input.txt` + grants `FileRead`/`FileWrite` caps —
    no test changes expected. If failing, document in the change
    record.
  - [ ] **python-hello-time**: with WASI bound + empty WasiCtx,
    CPython init's `get-environment` returns `[]`. CPython may emit
    site-package warnings to stderr; test asserts `Ok(())` only.
    If CPython init genuinely fails on empty env, narrow C-003's
    WasiCtxBuilder to seed a minimal `PYTHONHOME` or document the
    finding and consider re-opening D2 for Python only.
  - [ ] **go-env-greet**: with WASI bound, TinyGo runtime init
    succeeds; plugin's `env.Read("GREETING_NAME")` returns `none`
    via librefang (capability-gated). Plugin returns Ok regardless
    (see `main.go:34`). Test grants `EnvRead("GREETING_NAME")` and
    sets the env var; verify the librefang path still works.
  - [ ] **js-kv-counter**: with the C-005 unconditional librefang
    binding + dispatcher-level capability-deny stubbing, StarlingMonkey's
    auto-imported `librefang:plugin/fs` resolves at link time. The
    plugin never calls fs; capability gate never fires. Plugin
    proceeds to its real `kv.get/set` calls.
- [ ] Final acceptance:
  `cargo test -p librefang-runtime --no-fail-fast --test plugin_example_c_noop --test plugin_example_rust_fs_cat --test plugin_example_python_hello_time --test plugin_example_js_kv_counter --test plugin_example_go_env_greet`
  → **5 passed, 0 ignored, 0 failed.**

**Done when:** the acceptance command runs 5 passes and zero ignored.
A subset failing in a way that can't be fixed without scope creep
gets documented + the test stays `#[ignore]` with a new reason
string — but that's a redirect signal, not an OK outcome.

---

### C-007 — Memory bound enforcement test

**Branch:** same worktree
**Files:**
- `crates/librefang-runtime/src/sandbox_component.rs` (new
  `#[tokio::test] component_respects_max_memory_bytes` in the
  existing `tests` module)

**Depends on:** C-002 (limiter wired) + C-004 (WASI bound — keeps
the test realistic)
**Effort:** S–M
**Recommended agent:** `claude`

- [ ] Inline a small `wat` Component that declares `(memory 1)` and
  exports `run` calling `memory.grow` by N pages where N > the
  `SandboxConfig::max_memory_bytes`-derived page count.
- [ ] Build a `SandboxConfig` with `max_memory_bytes: 256 * 1024`
  (4 pages); the Component requests 32 pages.
- [ ] Assert `execute_component(...)` returns
  `Err(SandboxError::Execution(_))` with a message string mentioning
  memory growth / limiter (don't pin the exact message — wasmtime
  may shift wording).
- [ ] **Negative-side test**: same Component but max_memory_bytes
  bumped to fit; assert `Ok`.

**Done when:** `cargo test -p librefang-runtime --lib
component_respects_max_memory_bytes` passes both directions; the
test fails predictably when the C-002 `store.limiter` line is
commented out (manual verification: do this once, confirm, then
restore).

---

### C-008 — Docs + CHANGELOG

**Branch:** same worktree
**Files:**
- `docs/development/plugin-host.md` (new "WASI Preview 2 host
  integration" section after the existing "Walking examples
  (Phase-6)" section, before "Phase-5 traceability")
- `CHANGELOG.md` (`[Unreleased]` block — Added + Fixed entries)
- `.kbd-orchestrator/phases/phase-7-wasi-plugin-host/execution.md`
  (auto-created by /kbd-execute)

**Depends on:** C-006 (docs describe a working system, not an
aspirational one)
**Effort:** S
**Recommended agent:** `claude`

- [ ] Plugin-host doc section covers: (a) what WASI 0.2 interfaces
  are bound; (b) why WasiCtx is deny-by-default (D2 rationale);
  (c) how to widen WASI access if a future plugin legitimately
  needs e.g. an env var visible to CPython init — point at where
  `WasiCtxBuilder` is constructed in `sandbox_component.rs`;
  (d) that the `librefang:plugin/*` interfaces are now always
  bound and capability-gated at dispatch time (D6 rationale).
- [ ] Update the Phase-6 "Walking examples" table: drop the four
  "skipped pending …" entries; replace with "runs end-to-end".
- [ ] CHANGELOG `[Unreleased]` / `### Added`:
  - "wasmtime-wasi integration: Component plugin host now binds
    WASI Preview 2 with a deny-by-default WasiCtx — Python /
    JavaScript / Go / cargo-component-rt plugins instantiate without
    any host-side fixture changes."
  - "`store.limiter(...)` wired into `execute_component` — Component
    plugins now enforce `SandboxConfig::max_memory_bytes` (paid-
    forward Phase-5 TODO)."
- [ ] `### Changed`:
  - "Component path: `librefang:plugin/*` interfaces are now
    unconditionally bound at link time; capability gating moved to
    dispatch (closes the StarlingMonkey-shim auto-import quirk
    surfaced by Phase-6's js-kv-counter)."

**Done when:** docs render with working internal links; CHANGELOG
attribution clean (`(@gqadonis)` per the pre-commit hook contract);
the four Phase-6 "skipped pending …" lines are gone from the doc.

---

## Out of scope (re-confirmed from assessment)

- **Net + agent example plugins** — Phase-8 candidate.
- **`surrealdb-core = "=3.0.5"` workspace pin** — standalone follow-up PR.
- **`KernelHandleStub` promotion into `librefang-kernel-handle`** —
  standalone cleanup PR.
- **`dlmalloc` linkage example for C plugins** — after WASI lands
  so the C plugin can allocate against a real heap.
- **WASI 0.3** — upstream pre-stable.
- **WASI Preview 1 retrofit** — Phase-6 examples are all 0.2.
- **wit-component / wac composition** — host-side work only.
- **AOT cache invalidation deep-dive** — accept first-run JIT cost;
  document in CHANGELOG.

---

## Risks to manage during execution

(from the assessment, copied for execute-phase visibility)

1. **D4 version alias broken** — C-001's smoke test catches this
   before any real work commits. Fallback: vendor older
   `wasmtime-wasi` or rebuild the Python fixture — re-decide D4 if
   so.
2. **CPython init on empty env** — if it genuinely fails (not just
   warns), C-006 narrows the WasiCtx to seed a minimal `PYTHONHOME`
   for python-hello-time only, or documents the WasiCtxBuilder
   exposure pattern for future python plugins.
3. **`store.limiter` breaks Phase-5 component tests** — bump test
   configs' `max_memory_bytes` if needed (Phase-5 tests didn't
   exercise memory pressure, but the limiter rejects any growth
   attempt at all if max=0).
4. **D6 stub semantic surprise** — confirm via grep that no
   existing test relies on undeclared-capability imports failing at
   instantiation rather than at first-call.

---

## Verification — line-item (mirrors assessment.md)

After all 8 changes land:

| # | Check | Expected |
|---|---|---|
| 1 | `cargo check --workspace --lib` | exit 0 |
| 2 | `cargo clippy --workspace --all-targets -- -D warnings` | exit 0 |
| 3 | `cargo test -p librefang-runtime --no-fail-fast --test 'plugin_example_*'` | **5 passed, 0 ignored, 0 failed** (load-bearing) |
| 4 | grep `#[ignore` in `plugin_example_*.rs` | zero matches |
| 5 | `cargo test -p librefang-runtime --lib component_respects_max_memory_bytes` | pass |
| 6 | `python3 scripts/enforce-branding.py --check` | clean |
| 7 | `cargo xtask plugins-rebuild` | regenerates 5/5 byte-identical |
| 8 | WASI deny-by-default smoke (manual or new unit test) | open-at returns typed WASI error, not host panic |
| 9 | `docs/development/plugin-host.md` "WASI Preview 2" section | renders, internal links resolve |
| 10 | CHANGELOG `[Unreleased]` block | Added + Changed entries with `(@gqadonis)` |

---

## Execution sequencing

For the executor:

```
C-001 ── C-003 ── C-004 ┐
                        ├── C-006 ── C-008
       C-002 ───────────┤
                        └── C-007
                C-005 ───┘
```

Critical path: C-001 → C-003 → C-004 → C-005 → C-006 → C-008.
C-002 and C-007 sit off the critical path; C-007 needs C-004 + C-002
done before it can run but neither blocks anything else downstream.

---

## Next action

`/kbd-execute C-001`
