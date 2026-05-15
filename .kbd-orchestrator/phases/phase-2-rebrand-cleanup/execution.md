# Execution Log: phase-2-rebrand-cleanup / m8-m11-dashboard-prose-cleanup

**Change:** m8-m11-dashboard-prose-cleanup
**Milestones:** M8, M11
**Backend:** native-tool (claude-code)
**Worktree:** /tmp/librefang-m8-m11
**Branch:** feat/m8-m11-dashboard-prose-cleanup
**Started:** 2026-05-15

## Constraint coverage

| Constraint | Verification |
|---|---|
| Three-layer principle — no Layer Internal renames | `\bLibreFang\b` regex skips `LibreFangKernel`, `LibreFangError`, etc. by word boundary |
| Surface layer only | All changes are user-visible strings and dead code removal |
| enforce-branding.py extension preserves existing Color + Docs passes | New Pass 3 is additive; Passes 1 and 2 untouched |
| Dashboard `.ts`/`.tsx` no fence/inline-code awareness needed | TS/TSX has no markdown syntax; `replace_prose_in_tsx()` applies directly |
| App.tsx TS6133 — only dead code removed | USER_AVATAR_STYLE and BRAND_MARK_STYLE are declared and never read; removal only |

## Implementation plan

1. Extend `scripts/enforce-branding.py`
   - Add `DASHBOARD_PROSE_SCAN_DIRS` + `DASHBOARD_PROSE_EXTENSIONS`
   - Add Pass 3 loop in `main()` labeled "dashboard prose"
   - Update module docstring

2. Extend `scripts/test_enforce_branding.py`
   - Add `DashboardProseTests` class (4+ new tests)

3. Run extended script → auto-flips 5 M8 strings in dashboard src

4. Manually remove USER_AVATAR_STYLE + BRAND_MARK_STYLE from App.tsx (M11)

5. Verify
   - `python3 scripts/test_enforce_branding.py`
   - `python3 scripts/enforce-branding.py --check`
   - `pnpm test --run` (dashboard)
   - `pnpm typecheck` (TS6133 errors gone)
