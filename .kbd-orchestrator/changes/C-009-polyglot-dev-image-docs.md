# Change C-009 — Polyglot dev image docs

**Phase:** phase-4-multilang-wasm-toolchain
**Status:** DONE
**Completed:** 2026-05-27
**Files touched:** `docs/development/polyglot-dev-image.md` (new),
                   `Dockerfile.rust-dev` (header pointer)

## What landed

- `docs/development/polyglot-dev-image.md` — per-language plugin recipes
  (Rust, Python, TypeScript, JavaScript, Go, C) plus build / use / verify
  flows. Each recipe matches the equivalent test in
  `scripts/test-wasm-toolchain.sh` so docs and smoke can't drift in
  isolation.
- `Dockerfile.rust-dev` header — added a 6-line pointer to the new doc
  so anyone reading the Dockerfile knows the polyglot story exists.

## Verification

Doc is 211 lines, references existing on-disk paths
(`.kbd-orchestrator/...`, `scripts/test-wasm-toolchain.sh`,
`.github/workflows/wasm-toolchain.yml`, `Dockerfile.rust-dev`). Every
shell snippet has been executed during C-008 smoke runs.

## QA-gate note

Documentation-only change → QA-gate skipped per kbd-execute rules.
