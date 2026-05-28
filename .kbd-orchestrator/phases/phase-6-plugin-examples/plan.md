# Phase Plan: phase-6-plugin-examples

**Phase:** `phase-6-plugin-examples`
**Backend:** Native KBD (no OpenSpec, no evolver bridge)
**Date:** 2026-05-28
**Source assessment:** `assessment.md` in this directory
**Worktree:** `~/.claude/worktrees/librefang-phase6` on
`feat/phase6-plugin-examples`
**Base:** `origin/main @ a3bd9a3a1` (post-PR-#57)

---

## Decisions baked in

| # | Decision | Choice | Rationale |
|---|---|---|---|
| D1 | Example layout | `examples/plugins/<lang>-<feature>/` | Matches the existing `examples/custom-skill-*` convention |
| D2 | `.wasm` artefacts | **Committed pre-built** + `cargo xtask plugins-rebuild` regen recipe | Tests run deterministically without dev image; rebuild scripted |
| D3 | Test shape | One integration test per language (5 tests) | Cleaner per-language assertion variation; easier to grep |
| D4 | Side-effect verification | `KernelHandle`-stub-driven tests | Reuses an existing test affordance; doesn't touch `host_functions.rs` |
| D5 | Regen recipe | `cargo xtask plugins-rebuild` | Matches existing xtask infrastructure |
| Caps | Coverage | `fs`, `time`, `kv`, `env` (4/6) | `net` + `agent` deferred — Phase 7 candidates |

If you want any of these flipped, redirect on the affected change.

---

## Phase-6-specific hygiene rules

Inherits phase-5's "additive only inside librefang-runtime/librefang-skills" stance. New requirements:

- **Each example is a self-contained subdirectory** with its own
  build manifest (Cargo.toml / package.json / go.mod / Makefile)
  — never shells out to the workspace's build system.
- **Each example carries a `skill.toml`** declaring its
  `host_capabilities`. The skill.toml's WIT-world declaration is
  inert (Phase-6 doesn't wire `skill install` to load Components
  yet — that's Phase 7); the manifest serves as documentation +
  the integration test's source of truth for capability subset.
- **Pre-built `plugin.wasm` lives at
  `examples/plugins/<dir>/pre-built/plugin.wasm`.** The
  `.gitignore` keeps build outputs out everywhere else;
  `pre-built/` is the one whitelisted location, committed
  deliberately. Each `.wasm` budget: ≤ 200 KB (full Phase-6 budget
  ≤ 500 KB total — Rust will dominate at ~150 KB; C/Go/Python should
  sit at 30-60 KB).
- **Integration tests** live at
  `crates/librefang-runtime/tests/plugin_examples_<lang>.rs` —
  one file per language so a single failure is named per the
  language, not buried in a multi-language file.
- **No edits to phase-5 files** beyond an additive
  `librefang-runtime` test crate. WIT, sandbox_component, wit_host,
  aot_cache stay untouched.

---

## Ordered Change List

### C-001 — `examples/plugins/` directory scaffolding + xtask command
**Branch:** working in `feat/phase6-plugin-examples`
**Files:**
- new `examples/plugins/README.md` — index page pointing at each
  language subdir
- new `xtask/src/plugins.rs` — `plugins-rebuild` subcommand
  scaffold (no-op for now; per-language commands land with each
  example change)
- `xtask/src/main.rs` — wire the new subcommand
- `.gitignore` — add `target/`, `node_modules/`, `dist/`,
  `.pnpm-store/` etc. patterns under `examples/plugins/` (and
  explicitly whitelist `pre-built/plugin.wasm`)
**Effort:** S
**Recommended agent:** `claude`

- [ ] Top-level `examples/plugins/README.md` documenting the layout
  contract: each subdir's manifest, where the pre-built `.wasm`
  lives, how to rebuild, how to load via `load_and_run`.
- [ ] `cargo xtask plugins-rebuild [<lang>]` scaffold — accepts an
  optional language filter, prints "no per-language builders
  registered yet" until C-002+ fills them in.
- [ ] `.gitignore` updates so source-tree builds don't pollute the
  index.

**Done when:** `cargo xtask plugins-rebuild` runs cleanly (no-op);
`examples/plugins/README.md` renders; `.gitignore` rules cover the
expected build outputs.

---

### C-002 — `rust-fs-cat` (cargo-component, exercises `fs`)
**Branch:** same worktree
**Files:**
- new `examples/plugins/rust-fs-cat/Cargo.toml`
- new `examples/plugins/rust-fs-cat/src/lib.rs`
- new `examples/plugins/rust-fs-cat/skill.toml`
- new `examples/plugins/rust-fs-cat/README.md`
- new `examples/plugins/rust-fs-cat/pre-built/plugin.wasm`
- `xtask/src/plugins.rs` — implement `rust-fs-cat` builder
**Depends on:** C-001
**Effort:** M
**Recommended agent:** `claude`

- [ ] `Cargo.toml` with `[package.metadata.component]` block
  pointing at `../../../crates/librefang-skills/wit` and world
  `plugin`.
- [ ] `lib.rs` (~30 lines) — `impl Guest` that calls
  `bindings::librefang::plugin::fs::read("/tmp/test-input.txt")`,
  then `fs::write("/tmp/test-output.txt", contents)`. Returns
  `Ok(())` on success, `PluginError::Io(...)` on host-error.
- [ ] `skill.toml` declaring `host_capabilities = ["fs"]`.
- [ ] `README.md` — what it does, how to rebuild, how to run via
  `load_and_run --invoke`.
- [ ] `pre-built/plugin.wasm` committed (size budget ≤ 200 KB).
- [ ] xtask command builds via
  `cargo component build --release --target wasm32-wasip2` inside
  the example dir, copies output to `pre-built/plugin.wasm`,
  asserts size ≤ 200 KB.

**Done when:** `cargo xtask plugins-rebuild rust-fs-cat` reproduces
the committed `plugin.wasm` bit-identically (or within a
documented determinism budget); manual smoke via `load_and_run
--invoke` against a host with `fs` capability granted prints
INVOKE-OK.

---

### C-003 — `python-hello-time` (componentize-py, exercises `time`)
**Branch:** same worktree
**Files:**
- new `examples/plugins/python-hello-time/{plugin.py,skill.toml,README.md}`
- new `examples/plugins/python-hello-time/pre-built/plugin.wasm`
- `xtask/src/plugins.rs` — implement `python-hello-time` builder
**Depends on:** C-001
**Effort:** M
**Recommended agent:** `claude`

- [ ] `plugin.py` (~10 lines) — `class WitWorld:` with a `run`
  method that calls `host.time.now()` and returns Ok.
- [ ] `skill.toml` declaring `host_capabilities = ["time"]`.
- [ ] `README.md` reusing the rust-fs-cat template (consistent
  structure across examples).
- [ ] xtask builder invokes `componentize-py -d <wit-dir> -w
  plugin componentize plugin -o pre-built/plugin.wasm`.

**Done when:** xtask reproduces the committed `.wasm`; load+invoke
returns Ok against a host with `time` granted.

---

### C-004 — `js-kv-counter` (jco componentize, exercises `kv`)
**Branch:** same worktree
**Files:**
- new `examples/plugins/js-kv-counter/{plugin.js,package.json,skill.toml,README.md}`
- new `examples/plugins/js-kv-counter/pre-built/plugin.wasm`
- `xtask/src/plugins.rs` — implement `js-kv-counter` builder
**Depends on:** C-001
**Effort:** M
**Recommended agent:** `claude`

- [ ] `plugin.js` (~15 lines) — exports `run` that does
  `const cur = parseInt(kv.get("counter") ?? "0", 10);
   kv.set("counter", String(cur + 1));`
- [ ] `package.json` with `@bytecodealliance/jco` and any required
  type packages as devDeps. Declares `"type": "module"`.
- [ ] `skill.toml` with `host_capabilities = ["kv"]`.
- [ ] xtask builder invokes `npx jco componentize plugin.js --wit
  <wit-dir> --world-name plugin -o pre-built/plugin.wasm`.

**Done when:** xtask reproduces the committed `.wasm`; load+invoke
twice increments `kv["counter"]` from 1 → 2.

---

### C-005 — `go-env-greet` (TinyGo + wit-bindgen-go, exercises `env`)
**Branch:** same worktree
**Files:**
- new `examples/plugins/go-env-greet/{plugin.go,go.mod,skill.toml,README.md}`
- new `examples/plugins/go-env-greet/pre-built/plugin.wasm`
- `xtask/src/plugins.rs` — implement `go-env-greet` builder
**Depends on:** C-001
**Effort:** M-L
**Recommended agent:** `claude`

- [ ] `plugin.go` — `//export run` Go function calling
  `env.Read("PLUGIN_NAME")`; returns formatted greeting via
  plugin-error.Internal on miss. Use `wit-bindgen-go` to generate
  bindings into a `gen/` subdir.
- [ ] `go.mod` with the TinyGo + wit-bindgen-go module hints.
- [ ] `skill.toml` with `host_capabilities = ["env"]`.
- [ ] xtask builder: `wit-bindgen go --out-dir gen <wit-dir> &&
  tinygo build -target=wasip2 -o pre-built/plugin.wasm ./...`
- [ ] FALLBACK if TinyGo + wit-bindgen-go combination proves too
  rough at execution time: drop to a `wat`-encoded shim that
  emulates the Go output (documented in the change record);
  capability story remains valid because the wasm itself, not the
  author flow, is what the integration test exercises.

**Done when:** xtask produces a working `.wasm`; load+invoke
returns Ok against a host that has `PLUGIN_NAME` in the env
allowlist.

---

### C-006 — `c-noop` (wasi-clang + wit-bindgen-c, no host caps)
**Branch:** same worktree
**Files:**
- new `examples/plugins/c-noop/{plugin.c,Makefile,skill.toml,README.md}`
- new `examples/plugins/c-noop/pre-built/plugin.wasm`
- `xtask/src/plugins.rs` — implement `c-noop` builder
**Depends on:** C-001
**Effort:** M
**Recommended agent:** `claude`

- [ ] `plugin.c` (~20 lines) — implements the bindgen-generated
  `run` export; body is `return 0;`.
- [ ] `Makefile` that invokes `wit-bindgen c --out-dir gen
  <wit-dir>` then `${WASI_SDK_PATH}/bin/clang
  --target=wasm32-wasi --sysroot=${WASI_SDK_PATH}/share/wasi-sysroot
  -o plugin.wasm plugin.c gen/plugin.c`.
- [ ] `skill.toml` with `host_capabilities = []` (no caps —
  proves the empty-cap path through the linker).
- [ ] xtask builder shells out to `make` inside the dir.

**Done when:** xtask produces a working `.wasm` ≤ 50 KB;
load+invoke returns Ok.

---

### C-007 — Integration tests: `crates/librefang-runtime/tests/plugin_examples_*.rs`
**Branch:** same worktree
**Files:** 5 new test files, one per language:
- `tests/plugin_example_rust_fs_cat.rs`
- `tests/plugin_example_python_hello_time.rs`
- `tests/plugin_example_js_kv_counter.rs`
- `tests/plugin_example_go_env_greet.rs`
- `tests/plugin_example_c_noop.rs`

Plus shared support:
- new `tests/support/plugin_example_harness.rs` — `KernelHandle`
  test stub with `Vec<KvEvent>` / `Vec<FsEvent>` recorders for
  the integration tests to assert against
**Depends on:** C-002 … C-006 (needs all `pre-built/plugin.wasm` files)
**Effort:** M-L
**Recommended agent:** `claude`

- [ ] `support/plugin_example_harness.rs` — `KernelHandleStub`
  implementing the kernel-handle trait surface with in-memory
  side-effect recorders. Reuses any existing kernel-handle
  stubs in `librefang-runtime/tests/` if they exist; otherwise
  creates a minimal new one.
- [ ] Each test file:
  - Reads its `examples/plugins/<dir>/pre-built/plugin.wasm`
  - Builds a `KernelHandleStub` configured with the expected
    inputs (file contents for fs-cat, env vars for go-env-greet,
    seeded kv for kv-counter, nothing for time/noop)
  - Builds `ComponentExecuteOptions` with the matching
    `host_capabilities`
  - Calls `WasmSandbox::execute_component`
  - Asserts the language-specific side effect (per assessment D4)
- [ ] If any test fails because the live `host_functions::dispatch`
  doesn't accept a stub-driven invocation, document the
  limitation in the test and assert only what is reachable
  (likely `Ok(ExecutionResult)` without side-effect verification).

**Done when:** `cargo test -p librefang-runtime --test
'plugin_example_*'` passes 5/5 round-trips, OR fails gracefully
with a documented "this assertion requires X infrastructure"
skip-marker rather than a panic.

---

### C-008 — Docs cross-link + reflection seeds
**Branch:** same worktree
**Files:**
- `docs/development/plugin-host.md` — new "Walking examples" section
  pointing at each `examples/plugins/<dir>` with a one-line
  description
- `CHANGELOG.md` — `[Unreleased] / ### Added` entry
- `.kbd-orchestrator/phases/phase-6-plugin-examples/execution.md`
  (created automatically by /kbd-execute)
**Depends on:** C-007
**Effort:** S
**Recommended agent:** `claude`

- [ ] Walking examples table in plugin-host.md, one row per
  language pointing to its dir, capability, and a one-line
  description of what the plugin does.
- [ ] CHANGELOG bullet summarising the 5-example set.

**Done when:** docs render with working internal links; CHANGELOG
attribution clean.

---

## Out of scope (deferred)

- `net` and `agent` capability examples — Phase 7 (or follow-up).
- Skill installer wiring (loading a Component skill via
  `librefang skill install`) — Phase 7.
- Plugin marketplace integration — Phase 7+.
- Cross-runtime validation (wasmer / wasmedge for each example) —
  out of scope for the toolchain validation phase.
- WASI 0.3 examples — upstream pre-stable.

---

## Risks to manage during execution

| Risk | Affects | Mitigation |
|---|---|---|
| TinyGo + wit-bindgen-go has rougher edges than other toolchains | C-005 | Documented fallback to a wat-encoded shim; commit a working artefact even if author flow is imperfect. |
| wit-bindgen-c invocation has subtle CLI changes between versions | C-006 | Pin wit-bindgen-cli at a known-good version in the dev image; document in Makefile header. |
| Pre-built `.wasm` non-determinism (compiler version drift) | C-002…C-006 | `cargo xtask plugins-rebuild` is required to run inside the pinned dev image; deviation is a regression. Add a CI step (Phase-7 candidate) that re-runs the rebuild and diffs against committed copies. |
| `KernelHandle` stub doesn't expose enough surface for kv-counter side-effect tracking | C-007 | Assessment D4 fallback: assert `Ok(ExecutionResult)` only; widen to side-effect verification when the stub gains the relevant trait method. |
| componentize-py / jco / TinyGo are not installed in the local cargo-test environment (only in the dev image) | C-007 | Tests load committed `pre-built/plugin.wasm`, NOT rebuild on test run. Test-time deps stay at: wasmtime (already linked) + serde + tokio. |
| `pre-built/plugin.wasm` files inflate the repo | All | 200 KB per-file budget; 500 KB total budget. If any single one busts, simplify the example. |
| Live `host_functions::dispatch` requires real kernel state for kv/agent calls | C-007 | KV via `KernelHandle` is the cleanest stub surface; if it isn't ergonomic, fall back to per-test in-memory state injection at the dispatch boundary (last resort — touches host_functions.rs which violates phase-5 D1). Prefer assertion narrowing. |
| Examples reference a WIT path via relative `../../..` — fragile if examples ever move | All | Documented in each example's README; consider a `cargo xtask plugins-wit-path` helper if it bites. |

---

## Verification matrix

After all 8 changes merge:

| Check | Command | Acceptance |
|---|---|---|
| Workspace compiles | `cargo check --workspace --lib` | exit 0 |
| Integration tests | `cargo test -p librefang-runtime --test 'plugin_example_*'` | 5/5 pass (or graceful skip with documented reason) |
| Rebuild produces matching artefacts | `cargo xtask plugins-rebuild` inside dev image | byte-identical to committed `.wasm` (or within determinism budget) |
| Sizes within budget | `du -sh examples/plugins/*/pre-built/plugin.wasm` | each ≤ 200 KB; total ≤ 500 KB |
| Brand audit | `python3 scripts/enforce-branding.py --check` | exit 0 |
| Phase-4 smoke unaffected | `scripts/test-wasm-toolchain.sh` inside dev image | green |
| Docs | `docs/development/plugin-host.md` renders | walking-examples section present |

---

## Suggested execution order summary

```
C-001 scaffold + xtask command (foundation)
   │
   ▼
C-002 rust-fs-cat ──┬── C-003 python-hello-time
                   ├── C-004 js-kv-counter
                   ├── C-005 go-env-greet
                   └── C-006 c-noop
                          │
                          ▼
                       C-007 integration tests (needs all 5 .wasm)
                          │
                          ▼
                       C-008 docs + CHANGELOG
```

C-002…C-006 can land in any order once C-001 is in. C-007 depends
on all 5. C-008 finalises.

---

## Next action

`/kbd-execute C-001` to scaffold the directory + xtask. The 5
example changes can then proceed independently.
