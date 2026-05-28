# Phase Assessment: phase-6-plugin-examples

**Project:** LibreFang (BossFang fork) — github.com/GQAdonis/librefang
**Date:** 2026-05-28
**Status:** NEW PHASE — proposed.
**Worktree:** `~/.claude/worktrees/librefang-phase6` on
`feat/phase6-plugin-examples`, based on `origin/main @ a3bd9a3a1`
(post-PR-#57).
**Prior context:** `phase-5-plugin-host-crate/reflection.md` — Component
Model plugin host shipped; per-language plugin examples explicitly
deferred to "a future KBD change".

---

## Goal

Ship a small, runnable set of per-language plugin examples that each
target the `librefang:plugin@0.1.0/plugin` world (the WIT contract
landed in phase 5) and exercise at least one of the six host
capabilities (`fs`, `net`, `kv`, `agent`, `env`, `time`).

The deliverables convert C-007's `LOAD-OK` smoke checks into
**`INVOKE-OK` end-to-end round-trips**: each example compiles to a
`.wasm` Component, declares its `host_capabilities` in `skill.toml`,
and `librefang_runtime::sandbox_component::execute_component` loads
+ invokes its `run` export against the real `host_functions::dispatch`.

## Out-of-scope (intentional)

- Adding new host capabilities to the librefang:plugin WIT —
  Phase 6 ships *consumers* of the existing surface, not extensions
  to it.
- Plugin distribution / registry integration — Phase 7 candidate.
- WASI 0.3 (async components) — upstream still pre-stable.
- Per-language CI lanes — phase-4's `wasm-toolchain.yml` already
  covers the dev image; phase-6's examples ride that.

---

## Summary — what the codebase has today

Phase 5 shipped the host machinery; phase 6 must show authors how to
*consume* it. After PR #57:

| Component | Lives at | Role |
|---|---|---|
| WIT contract | `crates/librefang-skills/wit/{host,world}.wit` | librefang:plugin@0.1.0; 6 host interfaces + plugin world |
| Typed shim | `crates/librefang-runtime/src/wit_host.rs` | JSON ↔ typed Component args |
| Execute path | `crates/librefang-runtime/src/sandbox_component.rs` | `WasmSandbox::execute_component()` + Host trait impls + per-cap link gate |
| AOT cache | `crates/librefang-runtime/src/aot_cache.rs` | `.cwasm` precompile + load |
| Manifest field | `crates/librefang-skills::SkillManifest.host_capabilities` | `#[serde(default)]`, opt-in to Component path |
| Standalone harness | `crates/librefang-runtime/examples/load_and_run.rs` | `LOAD-OK` / `INVOKE-OK` reporter |
| Polyglot toolchain | `Dockerfile.rust-dev` (phase 4) | cargo-component + componentize-py + jco + TinyGo + wit-bindgen all in-image |
| Author docs | `docs/development/plugin-host.md` | Per-language recipes (incl. the WIT path arg for each toolchain) |

What's missing:

- **No example in-tree plugin actually targets the librefang:plugin world.**
  The existing `examples/custom-skill-{python,prompt}` use the old
  `runtime_type = "python"` / prompt-only paths, not the new
  Component path.
- **No end-to-end round-trip test exists.** All phase-5 tests are
  either pure unit tests (wit_host, aot_cache) or
  type-system-only (sandbox_component's `bindgen_emits_plugin_and_add_to_linker`).
  We never actually instantiate + invoke a real `.wasm` Component
  built against our WIT.
- **The `load_and_run --invoke` path is unverified against a real
  plugin.** It works on a hand-encoded 8-byte empty Component but
  hasn't met a librefang:plugin Component yet.

**Gap count: 5 per-language examples + 1 integration test harness + 1 docs
linkage update.**

---

## Codebase Scan Results

### What `examples/` looks like today

```
examples/
├── custom-agent/        ← agent.toml fixture
├── custom-channel/      ← custom channel adapter
├── custom-skill-prompt/ ← prompt-only skill
├── custom-skill-python/ ← Python subprocess skill (NOT Component)
├── sidecar-channel-bash/
├── sidecar-channel-go/
├── sidecar-channel-node/
└── sidecar-channel-python/
```

Existing skill examples use `[runtime] type = "python"` (subprocess
runtime, not in-process WASM). Phase 6 adds a parallel set under a
clearly-named subdir.

### Recommended layout

```
examples/plugins/
├── rust-fs-cat/         ← cargo-component; uses fs.read
├── python-hello-time/   ← componentize-py; uses time.now
├── js-kv-counter/       ← jco componentize; uses kv.get/set
├── go-env-greet/        ← tinygo + wit-bindgen-go; uses env.read
└── c-noop/              ← wasi-clang + wit-bindgen c; minimal
```

Each is small (under 100 lines source), each exercises ONE host
capability (so the matrix is `5 examples × 1 cap each = 5 of 6 caps
covered`, with `agent` deliberately deferred — exercising it
requires multi-agent fixtures beyond Phase 6's scope).

Capability coverage matrix:

| Capability | Example | Why this one |
|---|---|---|
| `fs` | rust-fs-cat | Most-used capability in the wild; Rust gives the cleanest cargo-component story |
| `time` | python-hello-time | No I/O dependency; isolates the bindgen+invoke path for componentize-py |
| `kv` | js-kv-counter | Demonstrates stateful host calls (read-modify-write) |
| `env` | go-env-greet | TinyGo's wit-bindgen-go support is comparatively newer — keep the cap simple |
| `time` | c-noop | C path is the most painful to author; minimise scope to just "instantiate + run" |
| `net` | (none) | Deferred — requires SSRF guard wiring + a real or mocked endpoint |
| `agent` | (none) | Deferred — requires multi-agent test fixture |

### Integration test wiring

A new integration test in `crates/librefang-runtime/tests/` walks
each `examples/plugins/<lang>/` dir, looks for a checked-in
`plugin.wasm` (the `.gitignore` keeps build artefacts out, but a
post-build `pre-built/plugin.wasm` can be committed deliberately at
~30-100 KB each), and runs each through
`WasmSandbox::execute_component`. Asserts `Ok(ExecutionResult)` per
language with the appropriate `host_capabilities` set.

Alternative: build each example as part of the test run via the dev
image. Cleaner but slower (each test would shell out to cargo-component
/ componentize-py / jco / tinygo / wit-bindgen). The plan-phase
should pick one.

---

## Architectural Decision Points

### D1. Where do per-language examples live?

(a) **`examples/plugins/<lang>-<feature>/`** (recommended).
    Visible alongside the existing `examples/custom-skill-*` set;
    matches the convention readers already know.
(b) `crates/librefang-runtime/examples/plugins/<lang>/`. Closer to
    the runtime that loads them but breaks the cargo `examples/`
    convention (which means `cargo run --example`-compatible Rust
    binaries, not multi-language source trees).

→ **Recommendation: (a).**

### D2. How are `.wasm` artefacts checked in?

(a) **Commit pre-built `plugin.wasm` per example** (~30-100 KB each;
    total < 500 KB additional repo size). Round-trip tests run
    deterministically without needing the dev image at test time.
    Risk: stale artefacts if the WIT changes; needs a regeneration
    step in CI or a `just rebuild-plugin-examples` recipe.
(b) **Build at test time inside the dev image.** No checked-in
    binaries. Tests skip-on-missing-toolchain. Slower but always
    fresh.
(c) **Hybrid**: commit pre-built artefacts AND a regeneration
    script; CI verifies the commit matches the built output.

→ **Recommendation: (a) with a `just rebuild-plugin-examples` recipe**
   and a doc note that bumping the librefang:plugin WIT requires
   running it. (c) is overkill for Phase 6's scope; revisit if WIT
   churn becomes painful.

### D3. Integration test scope

(a) **One test per language** (5 tests). Direct, easy to grep, each
    asserts INVOKE-OK + expected stdout via a custom host that
    captures fs writes / kv writes / etc.
(b) **One parametrised test that iterates `examples/plugins/<lang>/`**.
    Less code, but per-language assertion variation makes the
    parametrisation messy.

→ **Recommendation: (a).**

### D4. What does "INVOKE-OK" assert per language?

Each plugin's `run` function should produce an observable side-effect
through its declared capability. The test asserts the side-effect:

| Plugin | Side-effect to assert |
|---|---|
| rust-fs-cat | Reads a host-provided file `/tmp/test-input.txt` (created by the test) and writes its content back to a host-provided file `/tmp/test-output.txt`. Test asserts file contents round-trip. |
| python-hello-time | Calls `time.now()`, formats it, returns via plugin-error variant. Test asserts the returned timestamp is within a 60s window. |
| js-kv-counter | Increments `counter` key. Test asserts kv["counter"] == 1 after one call, 2 after two. |
| go-env-greet | Reads `PLUGIN_NAME` env var, returns formatted greeting. Test asserts the greeting contains the value. |
| c-noop | Just returns Ok(()). Test asserts execute_component returns Ok. |

For the rust/js/go cases, we need a fake `host_functions::dispatch`
that records side-effects. Either:
- Inject via the existing `KernelHandle` trait surface (preferred —
  reuses the existing test affordance)
- Or expand `dispatch` to accept a closure for testing (out of
  scope; would touch host_functions.rs which violates Phase-5 D1
  hygiene)

→ **Recommendation: extend `KernelHandle`-stub-driven tests** —
   already proven in `librefang-runtime/tests/` for other features.

### D5. Pre-built artefact regeneration

Either a `just` recipe or a `cargo xtask` subcommand. The repo
already has `just setup` and `cargo xtask` infrastructure; tying it
into `cargo xtask plugins:rebuild` matches the existing pattern
(see `.kbd-orchestrator/project.json` references).

→ **Recommendation: `cargo xtask plugins-rebuild`.**

---

## Verification Plan

After Phase 6 implements all 5 examples + 1 integration test + docs
linkage:

1. `cargo check --workspace --lib` exit 0 (no new lib deps).
2. `cargo test -p librefang-runtime --test plugin_examples` → 5/5
   round-trips pass (assuming pre-built artefacts present).
3. `cargo xtask plugins-rebuild` regenerates all 5 `plugin.wasm`
   files inside the dev image; produces a byte-deterministic diff
   against the checked-in copies (regression guard).
4. `python3 scripts/enforce-branding.py --check` clean.
5. `docs/development/plugin-host.md` — augmented with a "**Walking
   examples**" section pointing at each new `examples/plugins/<dir>`.
6. Phase-4 smoke (`scripts/test-wasm-toolchain.sh`) still passes
   inside the dev image — Phase 6 doesn't touch it.

---

## Risks

| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| TinyGo + librefang:plugin world binding has rough edges (TinyGo wit-bindgen-go is comparatively newer) | High | Medium | Pick `env` (simplest interface) for Go; fall back to wat-encoded shim if Go authoring proves intractable. |
| wit-bindgen-c is genuinely painful to author by hand | High | Medium | C example is `c-noop`: doesn't call any host functions; just a Component that returns Ok. Proves the build pipeline. |
| Pre-built `.wasm` regeneration is non-deterministic (compiler version drift) | Medium | Low | Document `cargo xtask plugins-rebuild` must be run inside the pinned dev image; commit the dev image SHA in the regen note. |
| componentize-py's bindgen surface has changed since Phase 5 docs were written | Low | Low | The python-hello-time example uses only `time.now()`; minimal surface to test. |
| `KernelHandle` stubs don't expose enough side-effect tracking for kv test | Medium | Medium | If the stub model is too constrained, fall back to a per-test in-memory KV store passed via `host_functions::dispatch` test injection. |
| Pre-built `plugin.wasm` files bloat the repo | Low | Low | Budget ~500 KB total. If a single example exceeds 200 KB, treat as a signal to drop its complexity. |
| WIT changes during Phase 6 require all 5 artefacts to be regenerated | Medium | Low | Lock the WIT contract for the phase. WIT changes go in their own future phase. |

---

## Decisions Required (before Plan phase)

1. **D1**: `examples/plugins/<lang>-<feature>/` layout (recommended) vs nested under `crates/librefang-runtime/`.
2. **D2**: Pre-built `.wasm` artefacts committed (recommended) vs built-at-test-time vs hybrid.
3. **D3**: One test per language (recommended) vs parametrised iterator.
4. **D4**: KernelHandle-stub-driven side-effect assertions (recommended) vs dispatch closure injection.
5. **D5**: Regeneration via `cargo xtask plugins-rebuild` (recommended) vs `just plugin-examples-rebuild`.
6. **Capability matrix**: confirm `fs`/`time`/`kv`/`env` covered, `net`/`agent` deferred. Or expand to include `net` (would add a real-or-mocked HTTP fixture).

---

## Sources / prior art consulted

- `crates/librefang-skills/wit/{host,world}.wit` — Phase-5 WIT contract.
- `crates/librefang-runtime/src/sandbox_component.rs` — `execute_component` signature and `ComponentExecuteOptions`.
- `crates/librefang-runtime/examples/load_and_run.rs` — Phase-5 harness pattern.
- `docs/development/plugin-host.md` — Per-language recipes that Phase 6 turns into real, building examples.
- `examples/custom-skill-{python,prompt}/skill.toml` — Existing manifest layout reference.
- `.kbd-orchestrator/phases/phase-5-plugin-host-crate/reflection.md` — "Per-language end-to-end plugin examples" explicit follow-up.
