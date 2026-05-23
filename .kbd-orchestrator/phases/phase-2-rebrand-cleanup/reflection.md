# Phase Reflection: phase-2-rebrand-cleanup

**Project:** LibreFang (BossFang fork) — github.com/GQAdonis/librefang
**Date:** 2026-05-23
**Phase completion:** 100%
**Changes completed:** 3 / 3 (covering all 4 milestones: M8, M9b, M10, M11)

---

## Where the Phase Diverged from Plan

Five areas diverged from plan. None required rework; all were resolved cleanly
in the first pass. Surfaced here for institutional memory.

### 1. Rust function names misidentified in assessment

The assessment named `default_name()` as the function returning `"LibreFang Agent OS"`
in `crates/librefang-types/src/config/types.rs`. The actual function is
`default_a2a_name()` — for the A2A (agent-to-agent) service card name. The
`totp_issuer` default was similarly expected in `config/types.rs` but actually
lived in `crates/librefang-types/src/approval.rs` as `default_totp_issuer()`.

Root cause: assessment grepped for `LibreFang` in config-related files but did not
verify the exact function signatures before naming them in the plan. The execution
agent corrected this inline; no extra PR required.

**Lesson:** Assessment must `grep -n` and include the verbatim function signature
(not just the file path) for any Rust default function it plans to edit.

### 2. Two unplanned MDX files added to m9b scope

`docs/src/app/agent/auto-evolution/page.mdx` (EN) and its ZH mirror were not in
the plan's m9b file table. Upstream added these files after the M4 enforce-branding
sweep ran in Phase 1, so they contained prose `LibreFang` references that
`enforce-branding.py --check` would catch. The execution agent folded them into
m9b (PR #19) rather than opening a separate PR. Scope creep was minimal (2 files,
≤5 string flips total) and the fix was coherent with the rest of m9b.

**Lesson:** Auto-evolution docs are a recurring addition in upstream (they get
updated every release cycle). Add `docs/src/app/agent/` to the enforce-branding
watch list or document that `--check` after each upstream merge is the coverage
mechanism.

### 3. PR numbering offset

The plan projected PR #17 = M8+M11, PR #18 = M9b, PR #19 = M10. Actual PRs:

| Planned | Actual | Note |
|---|---|---|
| #17 = M8+M11 | #17 = assessment PR (phase planning) | Assessment landed as #17 before Phase-2 execution |
| #18 = M9b | #18 = M8+M11 | Shift by one |
| #19 = M10 | #19 = M9b | Shift by one |
| — | #20 = M10 | Extra PR slot consumed |

Cosmetic-only divergence; PRs are independent and were merged in the correct
logical order. `progress.json` records the actual PR numbers.

### 4. All three Phase-2 PRs required rebase before merge

Plan assumed `main` would be stable between PR-A, PR-B, and PR-C merges (no
upstream activity). In practice, upstream merges #21 (104 commits) and #22 (10
commits) landed on `main` between Phase-2 executions, causing three identical
conflict patterns on `.kbd-orchestrator/current-waypoint.json` and
`.kbd-orchestrator/phases/phase-2-rebrand-cleanup/progress.json`.

Resolution was consistent: `git checkout --theirs` on both orchestrator files (the
branch version always has the more advanced phase state), then `git rebase
--continue`. No substantive content was lost. All three rebases resolved in
< 2 minutes each.

**Lesson:** Orchestrator state files (`current-waypoint.json`, `progress.json`)
should be added to `.gitattributes` with `merge=ours` (branch-wins strategy) so
rebases self-resolve without manual intervention.

### 5. PR #16 (upstream merge, 3 commits) closed as superseded

PR #16 targeted 3 upstream commits (telegram OGG/Opus fix, trajectory-format.md,
TypeScript bump). CI for PR #16 failed because `universal-agent-runtime`'s
transitive dependency `kreuzberg` uses an SSH URL
(`ssh://git@github.com/GQAdonis/kreuzberg.git`) which CI runners cannot
authenticate. While investigating, it emerged that PRs #21 and #22 — organic
upstream merges that went through `main` during Phase 2 — already incorporated
all 3 of PR #16's commits. PR #16 was closed as superseded.

**Debt carried forward:** The `kreuzberg` SSH dependency blocks all CI jobs that
run `cargo metadata --all-features`. A `.cargo/config.toml` `insteadOf` rewrite
(`ssh://git@github.com/GQAdonis/kreuzberg.git` → HTTPS equivalent) would fix CI
without touching the kreuzberg codebase. This was not addressed in Phase 2.

---

## Goals

| Goal | Status | Evidence |
|---|---|---|
| M8 — Dashboard JSX prose (5 string flips in 4 files) | **MET** | enforce-branding.py Pass 3 auto-flipped all 5; `pnpm test --run` 635/635 PASS |
| M9b — Config defaults + doc sweep (2 Rust fns + 17 MDX files EN/ZH) | **MET** | `cargo check -p librefang-types` exit 0; 4 doc spot-checks confirmed |
| M10 — Legacy-key removal planning (4 TODO comments + CHANGELOG) | **MET** | `grep TODO(drop-legacy-key) api.ts` — 4 hits; `pnpm typecheck` exit 0 |
| M11 — App.tsx TS6133 (2 dead-code declarations removed) | **MET** | `pnpm typecheck` exit 0 (TS6133 gone); no test regression |

**Overall: 4/4 goals MET (100%).**

---

## Delivered Changes

| Change | Milestones | PRs | Backend | Notable additions vs plan |
|---|---|---|---|---|
| `m8-m11-dashboard-prose-cleanup` | M8, M11 | #18 | claude-code | +2 auto-evolution MDX flips; enforce-branding.py Pass 3 with 7 new unit tests (22/22 total); App.tsx TS6133 cleared |
| `m9b-config-defaults-doc-sweep` | M9b | #19 | claude-code | Actual fns: `default_a2a_name()` + `default_totp_issuer()` in `approval.rs`; 2 unplanned auto-evolution MDX files folded in |
| `m10-legacy-key-removal-planning` | M10 | #20 | claude-code | 4 TODO comments + CHANGELOG `### Upcoming Breaking Changes`; comment-only, no logic change |

---

## Technical Debt Inventory

### Carried forward (intentional)

| Item | Location | Scheduled resolution |
|---|---|---|
| `TODO(drop-legacy-key)` at 4 api.ts removal sites | `crates/librefang-api/dashboard/src/api.ts` | v2026.8.x (≈2 release cycles from v2026.6.x) |
| shellcheck pre-push hook | `scripts/hooks/pre-push` | Explicitly DEFERRED per 2026-05-15 human decision |

### Carried forward (unresolved, Phase-3 candidates)

| Item | Location | Root cause |
|---|---|---|
| kreuzberg SSH URL in Cargo.lock | `Cargo.lock` → `universal-agent-runtime` → `kreuzberg` transitive | CI runners lack SSH key; need `.cargo/config.toml` `insteadOf` rewrite |
| `execution.md` gaps for m9b and m10 | `.kbd-orchestrator/phases/phase-2-rebrand-cleanup/execution.md` | Only the first change of a phase gets a full execution log; m9b and m10 have no execution.md entries |
| enforce-branding.py Pass 3 test paths hardcoded | `scripts/test_enforce_branding.py` `DashboardProseTests` | Tests use literal `crates/librefang-api/dashboard/src/` paths; will need updating if dashboard moves |

---

## Architecture Integrity Check

| Constraint | Status |
|---|---|
| Three-layer principle — no Layer Internal renames | PASS — all M8–M11 changes are Surface layer (display strings, dead code removal); no Rust identifiers, crate names, or module paths renamed |
| Layer Internal preservation | PASS — `LibreFangKernel`, `librefang_home`, `librefang-types`, `librefang-api`, `LIBREFANG_*` env vars untouched |
| Default-path policy (`~/.librefang/`) | PASS — not touched in any Phase-2 change |
| Boundary additive-only | PASS — no Boundary aliases added or removed; `BOSSFANG_HOME` / `BOSSFANG_VAULT_KEY` remain from Phase 1 |
| Domain ownership | PASS — no `bossfang.ai` claims; all links stay on `github.com/GQAdonis` |
| `API_KEY_LEGACY` timeline | PASS — v2026.8.x gate locked in M10; read-old/write-new shim from Phase 1 M6 still in place |

---

## Cross-Tool Coordination

- **enforce-branding.py** was extended in M8 (Pass 3 for dashboard `.ts`/`.tsx`).
  The extension is additive — Passes 1 and 2 unchanged. All future upstream merges
  now get dashboard prose coverage automatically on `--check`.
- **No artifact-refiner** was used in this phase (simple string flips + dead-code
  removal — no spec generation, no schema evolution).
- **No evolver-bridge.json** was needed (no evolver agent coordination).
- **pr #16 superseded by #21 / #22**: upstream merge cadence is fast. The Phase-2
  plan should account for organic upstream merges arriving mid-phase and proactively
  check `git log origin/main..upstream/main --oneline` before opening any upstream-
  merge PR.

---

## Lessons Learned

1. **Grep before naming functions in the assessment.** Assessment-level guesses at
   function names (e.g. `default_name()`) create discrepancy noise during execution.
   One `grep -n 'LibreFang' crates/librefang-types/src/config/types.rs` would have
   surfaced `default_a2a_name()` and located `default_totp_issuer()` in `approval.rs`.

2. **Orchestrator state files need a `merge=ours` gitattribute.** All three
   Phase-2 PRs hit the same `.kbd-orchestrator/` conflict. An `.gitattributes`
   entry resolves it automatically and eliminates a manual step per PR.

3. **Check `git log origin/main..upstream/main` before opening upstream-merge PRs.**
   PR #16 was invalidated by PRs #21 and #22. Organic upstream merges can supersede
   planned merge PRs faster than the planning cadence.

4. **One `execution.md` per change, not per phase.** The current convention leaves
   m9b and m10 with no execution log. Future phases should open (or append to)
   an `execution.md` at the start of each change, not once per phase.

5. **Phase-2 PR overhead was lower than Phase-1.** Three independent PRs targeting
   `main` (no chaining) worked smoothly. The only merge friction was orchestrator
   file conflicts — not actual code conflicts. Independent targeting is the right
   strategy for housekeeping phases.

---

## Next Phase Recommendation

Phase 2 cleaned up the four categories of technical debt surfaced in the Phase-1
reflection. The BossFang rebrand surface is now substantially complete for the
main codebase.

**Recommended Phase 3 scope:**

1. **kreuzberg SSH→HTTPS CI fix** (HIGH — blocks all `--all-features` CI jobs):
   Add `.cargo/config.toml` with `[net] git-fetch-with-cli = true` or an
   `[url "https://github.com/GQAdonis/kreuzberg.git"] insteadOf =
   ssh://git@github.com/GQAdonis/kreuzberg.git` stanza. Verify with
   `cargo metadata --all-features` in CI.

2. **Upstream merge audit after PRs #21/#22** (HIGH — 114 upstream commits consumed
   since last audit): Run the full librefang-upstream-merge skill checklist:
   SurrealDB schema parity scan, hardcoded URL audit, Tauri drift check, branding
   enforcement pass. Size: 1 PR if clean, N PRs if schema drift detected.

3. **`execution.md` tooling** (LOW): Add a kbd-execute convention that opens one
   execution log per change at change start (not one per phase). Could be a simple
   process addition to the skill.

4. **`.gitattributes` orchestrator merge strategy** (LOW): Add
   `.kbd-orchestrator/**/*.json merge=ours` and
   `.kbd-orchestrator/**/*.md merge=ours` to `.gitattributes` to eliminate the
   per-PR orchestrator rebase conflict.

5. **shellcheck pre-push hook** (DEFERRED — per 2026-05-15 human decision):
   Revisit if `web/public/install.sh` gains complexity that makes lint enforcement
   valuable.

**Suggested Phase 3 name:** `phase-3-upstream-sync-and-ci-hardening`
