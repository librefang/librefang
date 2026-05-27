# Phase Execution: phase-4-multilang-wasm-toolchain

**Backend:** `native-tool` (Claude Code direct edits)
**Rationale:** No OpenSpec; single tool; small change surface (Dockerfiles
only). Hybrid not warranted.
**Started:** 2026-05-27
**Plan source:** `plan.md` in this directory
**Change records:** `.kbd-orchestrator/changes/C-NNN-*.md`

## Dispatch contract

Per change:

1. Edit only the files named in the change's "Files touched" list.
2. Verify via `docker build` (no daemon side-effects; image-only).
3. Flip change record `Status:` from `EXECUTING` → `DONE`.
4. Update `progress.json.changes_completed`.
5. Per kbd-execute QA rule: skip artifact-refiner for changes touching
   fewer than 3 files (applies to C-001, C-005, C-006, C-007, C-009).
   C-002, C-003, C-004, C-008 run the QA gate.

## Phase-level dependencies

- Phase 3 M2 (upstream merge) — soft blocker on the assessment; C-001
  through C-007 only touch `Dockerfile` and `Dockerfile.rust-dev`, and
  upstream librefang doesn't carry `Dockerfile.rust-dev`. Real conflict
  surface is limited to the production `Dockerfile` if Phase-3 M2 lands
  upstream changes there. C-007 is the only change that touches the
  production `Dockerfile` — defer C-007 specifically until M2 lands.
  C-001…C-006 + C-008…C-009 can proceed immediately.
