# Phase Execution: phase-7-wasi-plugin-host

**Backend selected:** `native-tool` (Claude Code, single-author flow).
**OpenSpec:** absent (`openspec/` directory not present at project root).
**Evolver:** absent (no `.evolver/` directory).

## Why native-tool

Phase 7 is a focused 8-change runtime/sandbox modification on a single
crate (`librefang-runtime`) plus its docs+CHANGELOG. No spec-backed
traceability requirement; no multi-tool handoff. Same shape as Phase
6's executed flow — Claude Code drives all changes serially against
the `feat/phase7-wasi-plugin-host` branch in worktree
`~/.claude/worktrees/librefang-phase7`.

## Dispatch contract

| Item | Value |
|---|---|
| Worktree | `~/.claude/worktrees/librefang-phase7` |
| Branch | `feat/phase7-wasi-plugin-host` (tracking `origin/main`) |
| Base commit | `2d5148af2` (post-PR-#58, post-upstream-merge) |
| Changes | 8 (`C-001`..`C-008`) per plan.md |
| Per-change QA | **skipped** — no `.refiner/` setup in this project, and quality is enforced inline by `cargo check --workspace --lib`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test -p librefang-runtime`, the pre-commit hook (rustfmt + CHANGELOG attribution + secrets scan), and `python3 scripts/enforce-branding.py --check`. Same posture as Phase 6. |
| Archive | Not invoked — Phase 6's progress.json kept change records inline and the existing pattern works fine. |
| Commit cadence | Per-change commit when the change's verification step passes; squash on PR merge if desired. |
| PR | Single PR at end of phase against `GQAdonis/librefang:main` (use `--repo GQAdonis/librefang --head GQAdonis:<branch>` per the existing auto-memory). |
