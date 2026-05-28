# Change C-008 — Plugin host docs + migration guide

**Phase:** phase-5-plugin-host-crate
**Status:** DONE
**Completed:** 2026-05-27
**Files touched:**
  - new `docs/development/plugin-host.md` (~285 lines)
  - `docs/development/polyglot-dev-image.md` — cross-link section
    pointing at the new doc
  - `CHANGELOG.md` — `[Unreleased] / ### Added` entry summarising
    the phase, with `(@GQAdonis)` attribution

## What landed

`plugin-host.md` is the definitive plugin-author + ops reference for
Phase-5. Sections:

1. **The two execute paths** — side-by-side table comparing
   core-module `execute()` and Component-model `execute_component()`
   along 7 axes.
2. **Phase-5 components** — file map (WIT / wit_host /
   sandbox_component / aot_cache / manifest field / smoke harness)
   with line counts and roles.
3. **The librefang:plugin world** — the WIT shape + a complete
   `skill.toml` example showing `host_capabilities = ["fs", "net"]`.
4. **Capability gate vs runtime capability** — the
   `HostCapability` (link-time, coarse) vs `Capability`
   (runtime, fine) two-layer model.
5. **AOT cache (`compile_mode`)** — the three modes (`Auto`,
   `Aot`, `Jit`), cache invalidation rules, and the
   wasmtime-version bump coupling that's load-bearing for safety.
6. **Per-language authoring** — five recipes (Rust via
   cargo-component, Python via componentize-py, JavaScript via
   jco, Go via TinyGo `--wit-package`, C via wit-bindgen). Each
   tells the reader exactly what to change in their phase-4
   hello-world recipe to make it a librefang:plugin Component.
7. **Migrating an existing core-module Wasm skill** — 4-step
   walkthrough for skills that pre-date Phase 5.
8. **Verification** — the commands to run: three unit-test
   modules (30 tests), the load_and_run harness, the multi-language
   smoke.
9. **Phase-5 traceability** — links to the per-change records and
   the phase plan/assessment/reflection.
10. **Deferred to follow-ups** — explicit list of what's NOT in
    Phase 5 (watchdog harmonisation, `store.limiter` accessor,
    per-language end-to-end plugin examples, WASI 0.3).

## CHANGELOG

Single `### Added` bullet under `## [Unreleased]` covering the full
Phase-5 surface in one self-contained paragraph (so a release-notes
reader doesn't need to chase eight separate change-record files).
Includes the `(@GQAdonis)` attribution that the pre-commit hook
expects.

## Polyglot dev image cross-link

`docs/development/polyglot-dev-image.md` covers WASM compile; it now
ends with a "Loading the compiled `.wasm` via librefang" pointer at
`plugin-host.md`. Closes the documentation loop: a plugin author
landing on either doc finds their way to the other.

## Verification

Docs-only change. Verification:

- `wc -l docs/development/plugin-host.md` → 285 lines, no broken
  internal references.
- Cross-link from `polyglot-dev-image.md` renders to a valid
  relative path (`plugin-host.md`).
- CHANGELOG bullet starts with `**runtime(plugin-host):` matching
  the project's conventional-commit-flavoured prefix style.
- `(@GQAdonis)` attribution present per the pre-commit hook's
  CHANGELOG attribution check.

## QA-gate note

Docs-only change. Per kbd-execute QA rules, `/refine-validate` is
skipped for documentation. The change touches 3 files (≥3 threshold)
but the artefact is prose — no executable behaviour to validate
beyond cross-link integrity and CHANGELOG format.
