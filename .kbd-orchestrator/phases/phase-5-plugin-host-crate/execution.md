# Phase Execution: phase-5-plugin-host-crate

**Backend:** `native-tool` (Claude Code direct edits)
**Started:** 2026-05-27
**Plan source:** `plan.md` in this directory
**Change records:** `.kbd-orchestrator/changes/C-NNN-*.md`
**Worktree:** `/Users/gqadonis/.claude/worktrees/librefang-phase5` on `feat/phase5-plugin-host`

## Dispatch contract

Per change:

1. Edit only the files named in the change's "Files touched" list.
2. Verify per change (cargo check / wasm-tools / docker run smoke).
3. Flip change record `Status:` to DONE.
4. Update `progress.json.changes_completed`.

## Phase-5-specific hygiene rules (from D1)

- New behavior in NEW files only. No edits to `sandbox.rs` /
  `host_functions.rs` core paths.
- Existing public types stay binary-stable.
- New methods on `WasmSandbox` go in `impl` blocks inside the new
  files (`sandbox_component.rs`, etc.).
- WIT files net-new.
- Manifest extensions use `#[serde(default)]` on new fields.
