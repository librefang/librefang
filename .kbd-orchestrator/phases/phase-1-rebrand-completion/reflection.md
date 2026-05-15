# Phase Reflection: phase-1-rebrand-completion

**Project:** LibreFang (BossFang fork) — github.com/GQAdonis/librefang
**Date:** 2026-05-14
**Phase completion:** 100%
**Changes completed:** 5 / 5 (covering all 7 milestones)

---

## Where the Phase Diverged from Plan

Three areas diverged — two required rework, one expanded scope:

### 1. M1+M2: Two implementation-quality misses (rework required)

**Miss A — `LEGACY_PRODUCT_NAME` dead constant.**
Initial `desktop_install.rs` implementation introduced a public `const
LEGACY_PRODUCT_NAME: &str = "LibreFang"` that was never used. `cargo clippy -D
warnings` caught it as `E0063: constant is never used`. Rework required: remove
the constant. Root cause: the constant was added speculatively ("might be
useful") without a call site.

**Miss B — Premature `BOSSFANG_DASHBOARD_PASS` in config template.**
`init_default_config.toml` initially received a `BOSSFANG_DASHBOARD_PASS` comment
that promised a Rust-side alias not yet implemented (M7 scope). This violated
the "additive aliases only after the code-side alias lands" constraint. Caught
during review, reverted in the same worktree before commit. Root cause: M7 scope
items were mentally merged into M1+M2 during editing.

### 2. M3: Pre-existing 404 bug discovered and fixed out-of-scope

`install.sh` was downloading `librefang-${PLATFORM}.tar.gz` but `release-cli.yml`
publishes `bossfang-${PLATFORM}.tar.gz`. This was a pre-existing 404 for every
user attempting a fresh install. Fixed inline during M3 (correct change,
appropriate, but not planned). The fix also required updating all internal binary
invocations from `bossfang` (the tarball's single binary) to consistently use
`${INSTALL_DIR}/bossfang`.

### 3. M6: AuditPage.tsx rogue localStorage call found and fixed

During M6 a direct `localStorage.getItem("librefang-api-key")` in
`pages/AuditPage.tsx:109` was discovered. This call bypassed both the
sessionStorage-preference policy (#3620) and the migration shim. Fixed inline:
replaced with `getStoredApiKey()` (import added). Widened M6 scope by one file,
but the fix was correct and non-optional.

---

## Goals

| Goal | Status | Evidence |
|---|---|---|
| M1 — CLI banner + 15 install path strings (migration shim) | **MET** | `cargo test -p librefang-cli --bins` 217/217 PASS; 2 new migration tests |
| M2 — Init wizard / config template comment flips | **MET** | Verified in same change; `enforce-branding.py --check` 0 hits |
| M3 — install.sh echoes + `BOSSFANG_INSTALL_DIR` alias | **MET** | `bash -n` + `sh -n` PASS; also fixed pre-existing tarball URL bug |
| M4 — 120 docs MDX prose flips via extended enforce-branding.py | **MET** | 121 files modified; 15/15 unit tests; `--check` exits 0 |
| M5 — Outbound UA strings flipped | **MET** | `cargo check` + `cargo clippy -D warnings` exit 0 on 3 crates |
| M6 — Dashboard localStorage migration shim | **MET** | 588/588 vitest tests PASS |
| M7 — 4 `BOSSFANG_*` env-var aliases (additive) | **MET** | `cargo check` + `cargo clippy -D warnings` exit 0 |

**Overall: 7/7 goals MET (100%).**

Note: `goal_completion` in `progress.json` marks all 7 as `MET`, consistent with
individual change `verification` blocks. The two rework items in M1+M2 were
resolved in the same worktree before the PR was committed — they do not appear as
failures in progress.json but are surfaced here for institutional memory.

---

## Delivered Changes

- `m1-m2-cli-banner-install-paths-config-templates` — CLI banner + 13 install path strings + auto-migration shim + 2 new tests + template comment flips (by: claude-code) → **PR #10**
- `m3-install-script` — install.sh rebrand + BOSSFANG_INSTALL_DIR alias + tarball URL bug fix + back-compat symlink (by: claude-code) → **PR #11**
- `m4-docs-mdx-sweep` — enforce-branding.py prose extension + 15 unit tests + 121 files flipped (by: claude-code) → **PR #12**
- `m5-m7-ua-strings-env-aliases` — 4 UA string flips + 3 BOSSFANG_* env aliases in plugin_manager.rs + webchat.rs (by: claude-code) → **PR #13**
- `m6-dashboard-localstorage-shim` — read-old/write-new localStorage migration shim + AuditPage fix + test updates (by: claude-code) → **PR #14**

---

## Technical Debt

- **`API_KEY_LEGACY = "librefang-api-key"` fallback in `dashboard/src/api.ts`** — drop this branch in ~2 release cycles once the upgrade window closes. The sentinel is `API_KEY_LEGACY`; grep for it to find all four removal sites.

- **`shellcheck` not verified locally for install.sh** — deferred to CI. If shellcheck is not in the CI pipeline, the deferred audit has no gate. Recommend adding `shellcheck web/public/install.sh` to the pre-push hook.

- **4 dashboard JSX files still contain PascalCase `LibreFang` in user-visible strings** — `skillHubs.ts`, `UsersPage.tsx`, `SkillsPage.tsx`, `MobilePairingPage.test.tsx` under `crates/librefang-api/dashboard/src/`. These were explicitly excluded from M4 scope. Recommend tagging as M8 or bundling with the next dashboard PR.

- **Backtick-delimited docs references are stale** — inline code spans like `` `"LibreFang Agent OS"` `` in MDX docs document UI strings that now read "BossFang Agent OS". The M4 fence/inline-code skipping rule intentionally preserved them; they now document the wrong string. Each instance needs human judgment: update the string, or add prose context explaining the historical name.

- **`App.tsx` TS6133 errors pre-exist** — `USER_AVATAR_STYLE` and `BRAND_MARK_STYLE` are declared but never read (lines 57-58). `pnpm typecheck` fails with exit 2 in the base branch and in every phase PR. A spawn task was filed to fix these; they were not introduced by this phase.

- **`LIBREFANG_REGISTRY_VERIFY`, `LIBREFANG_REGISTRY_INDEX_URL`, and ~57 other deep-plumbing `LIBREFANG_*` env vars remain un-aliased** — deliberately out of scope per constraints.md. Merge-cost impact is near zero (upstream rarely touches these); left as-is.

---

## Architecture Integrity

- **AGENTS.md / CLAUDE.md "never rename Layer Internal" rule:** PASS — no Cargo crate names, Rust module names, or function identifiers were renamed. The `librefang_home()` function, `librefang-cli` crate, `librefang-storage` crate, and `librefang_runtime` module are untouched.
- **Default-path policy:** PASS — `~/.librefang/` remains the default home directory in all templates, install.sh uninstall hints, and error messages.
- **Default-value policy:** PASS — `dashboard_user = "librefang"` and `dashboard_pass = "librefang"` preserved in `init_default_config.toml` (after catching and reverting the M1+M2 miss described above).
- **Domain ownership:** PASS — `bossfang.ai`, `librefang.ai`, `librefang.com`, `discord.gg/librefang`, and upstream GitHub permalinks were not claimed or modified.
- **No CI attribution in commits:** PASS — all commit messages are free of `Co-Authored-By: Claude` and related strings.
- **Constraint violations:** NONE at point of commit. The two mid-session misses (LEGACY_PRODUCT_NAME, BOSSFANG_DASHBOARD_PASS) were caught and corrected before any commit landed.

---

## Artifact Quality Summary

No `.refiner/` infrastructure was configured for this project. Manual
constraint-coverage review was performed inline per `execution.md`.

| Metric | Value |
|---|---|
| Changes with formal QA | 0/5 (no artifact-refiner) |
| Constraint coverage (manual) | 5/5 changes — constraint table in execution.md |
| Implementation-quality misses caught pre-commit | 2 (LEGACY_PRODUCT_NAME, BOSSFANG_DASHBOARD_PASS) |
| Scope expansions fixed inline | 2 (install.sh tarball URL, AuditPage.tsx localStorage) |

**Recommendation for Phase 2:** Set up `.refiner/artifacts/` with at minimum a
`constraint-check.md` template so that the two M1+M2 misses (dead constant,
premature alias) would have been caught at QA gate rather than mid-session.

---

## Cross-Tool Coordination Notes

This phase ran entirely within a single Claude Code session with five sequential
worktrees:

- **Progress tracking:** RELIABLE — `progress.json` was updated at the end of each change before commit; waypoint advanced correctly at each milestone.
- **Handoff quality:** N/A (single tool). The chained-PR base strategy (M3 branched from M1+M2, M4 from M3, etc.) means each subsequent `/kbd-execute` invocation inherits the previous worktree's committed state — which is correct but introduces human merge-sequencing overhead.
- **State loss:** NONE — waypoint file survived context compaction cleanly; the summary preserved all file paths, line numbers, and constraint notes needed to resume M5+M7 accurately.
- **Recommendations:**
  - When PRs in a chain land and close, update the base branch of subsequent open PRs promptly (GitHub's "Update branch" button) so CI runs against an accurate diff.
  - Consider keeping the KBD worktrees alive until all PRs in a chain merge; stale worktrees accumulate and eat `/tmp` space (74 GB freed earlier in this session from prior merge worktrees).

---

## Lessons Learned

- **Pre-commit constraint self-check is cheap; missing it is expensive.** The two M1+M2 misses each required a `git add -p` + re-check cycle that cost ~5 minutes. A 30-second "check: any unused constants? any aliases without code-side backing?" prompt before committing would eliminate both categories.

- **Prose-sweep tooling pays for itself immediately.** The M4 `enforce-branding.py` extension (fence + inline-code skipping) took ~1 hour to build and 15 tests to validate. It then swept 121 files in ~30 seconds and will continue to do so on every upstream merge. Without it, M4 would have been manual and error-prone at every merge.

- **Out-of-scope discoveries during execution are normal.** Two inline fixes (tarball URL, AuditPage.tsx) were not in the plan but were clearly correct to fix. The three-layer principle provides a reliable "is this in scope?" heuristic: if it's a Surface or Boundary change touching an already-open crate, fix it. If it would introduce new conflict surface on internal symbols, defer.

- **The read-old/write-new shim pattern is reliable but has a time limit.** Both the file-on-disk migration (LibreFang.app → BossFang.app) and the localStorage key migration have an implicit expiry date for the legacy branch. The next phase should define "2 release cycles" concretely (e.g., v2026.7.x) and add TODO comments with that version number to make the removal unambiguous.

- **Chained PRs require human sequencing discipline.** Five PRs in a strict base-chain (#10 → #11 → #12 → #13 → #14) means a merge ordering error or base-update delay blocks CI for all subsequent PRs. If Phase 2 has more than 3 chained PRs, consider rebasing them all to `main` independently and accepting slightly larger diffs — simpler review, no ordering constraint.

---

## Next Phase Focus

**Recommended next phase: `phase-2-rebrand-cleanup`**

Priority areas (in order):

1. **Dashboard JSX prose cleanup (M8)** — `skillHubs.ts`, `UsersPage.tsx`, `SkillsPage.tsx`, `MobilePairingPage.test.tsx` still contain PascalCase `LibreFang` in user-visible strings. Small, contained, no conflict surface.

2. **Stale backtick-code docs audit** — Update or annotate inline code spans that document old UI strings (e.g. `` `"LibreFang Agent OS"` ``). Requires human judgment on each instance; recommend a grep pass + PR with explanatory prose context for each retained reference.

3. **Legacy localStorage key removal planning** — Define the concrete release version at which `API_KEY_LEGACY` is dropped from `api.ts`. Add a `// TODO: drop after v<version>` comment to the code, and add it to `CHANGELOG.md [Unreleased]` so it's visible to reviewers.

Architectural decisions needing human review before Phase 2:
- Should `enforce-branding.py` be extended to also cover the 4 dashboard JSX files (current scope excludes them because they contain `LibreFang` in identifier positions that need human review, not mechanical swap)?
- Should `shellcheck` be added to the pre-push git hook (scripts/hooks/pre-push) or remain CI-only?

---

## Context for Next Phase

Use this file as prior context for the next `/kbd-assess` invocation. Key
constraints that must carry forward:

- Three-layer principle (see `.kbd-orchestrator/constraints.md`)
- Default-path policy: `~/.librefang/` stays
- Domain ownership: `bossfang.ai` not claimed
- `API_KEY_LEGACY` removal target version must be decided and documented before it silently becomes permanent
