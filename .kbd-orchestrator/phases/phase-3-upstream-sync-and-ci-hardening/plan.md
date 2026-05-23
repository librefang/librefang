# Phase Plan: phase-3-upstream-sync-and-ci-hardening

**Project:** LibreFang (BossFang fork) — github.com/GQAdonis/librefang
**Date:** 2026-05-23
**Backend:** native-tool (claude-code)
**Decisions recorded:**
- upstream-merge-strategy: YES — proper `git merge upstream/main` (not squash)
- gitattributes-orchestrator: YES — implement `merge=ours` for `.kbd-orchestrator/`

---

## Change Inventory (3 milestones, 2 PRs)

### Change 1: `m1-m3-ci-fix-and-gitattributes`
**Milestones:** M1, M3
**Priority:** HIGH (M1) / LOW (M3)
**Estimated PR:** <1 hour, near-zero risk

**Scope:**

1. **`.cargo/config.toml` URL rewrite (M1):**
   - Add `[url."https://github.com/GQAdonis/kreuzberg.git"] insteadOf = "ssh://git@github.com/GQAdonis/kreuzberg.git"`
   - Run `cargo update -p kreuzberg` to refresh Cargo.lock entry from SSH → HTTPS
   - Verify: `cargo check -p librefang-llm-drivers --features uar-driver` exits 0

2. **`.gitattributes` orchestrator merge strategy (M3):**
   - Append to `.gitattributes`:
     ```gitattributes
     # Orchestrator state files always keep branch version on merge/rebase.
     .kbd-orchestrator/**/*.json merge=ours
     .kbd-orchestrator/**/*.md merge=ours
     ```
   - Add one-time setup note to CLAUDE.md: `git config merge.ours.driver true`
     (under "before any work" section, alongside `just setup`)

**Verification checklist:**
- [ ] `cargo check -p librefang-llm-drivers --features uar-driver` — exits 0
- [ ] `grep "kreuzberg" Cargo.lock | grep "ssh://"` — 0 hits
- [ ] `grep "kreuzberg" Cargo.lock | grep "https://"` — 1+ hits
- [ ] `cat .gitattributes | grep kbd-orchestrator` — 2 hits
- [ ] `cargo check --workspace --lib` — exits 0 (no regressions)

**Layer constraint check:**
- M1: Build config only — no Rust source change. PASS.
- M3: `.gitattributes` + CLAUDE.md comment. PASS.

---

### Change 2: `m2-upstream-merge-224-commits`
**Milestones:** M2
**Priority:** HIGH
**Estimated PR:** ~2–3 hours, medium complexity

**Scope:**

1. **Full upstream merge:**
   ```bash
   git fetch upstream --no-tags
   git merge upstream/main --no-edit
   ```
   Expected conflicts (resolve per table below):

   | File | Strategy |
   |---|---|
   | `crates/librefang-api/dashboard/src/index.css` | Take ours + append upstream's `input[type="number"]` spinner CSS block (23 lines after EOF) |
   | `crates/librefang-types/src/config/types.rs` | Take upstream (additive fields) — then verify BossFang defaults `default_a2a_name()` + `default_totp_issuer()` still present |
   | `crates/librefang-types/src/approval.rs` | Take upstream if changed — then verify `default_totp_issuer()` returns `"BossFang"` |
   | `scripts/enforce-branding.py` | Take ours (we added Pass 3 in Phase 2) — then verify the file still runs clean |
   | Any `.kbd-orchestrator/` file | Take ours (upstream doesn't have this dir; conflicts impossible — but if they appear, take ours) |
   | All other files | Take upstream as-is |

2. **SurrealDB migration v030 — composite indexes (M2a):**
   - Create `crates/librefang-storage/src/migrations/sql/030_composite_indexes.surql`:
     ```surql
     -- Migration v30: Composite indexes for per-agent recency hot-paths.
     -- Mirrors SQLite migration v41 (#5641).
     DEFINE INDEX IF NOT EXISTS idx_sessions_agent_updated
         ON sessions COLUMNS agent_id, updated_at;
     DEFINE INDEX IF NOT EXISTS idx_audit_agent_timestamp
         ON audit_entries COLUMNS agent_id, timestamp;
     ```
   - Register in `crates/librefang-storage/src/migrations/mod.rs` as v30

3. **Golden fixture regeneration (M2b):**
   ```bash
   cargo test -p librefang-api --test config_schema_golden -- \
       --ignored regenerate_golden --nocapture
   ```
   `AutoRouteStrategy` was removed from upstream's KernelConfig shape; commit the new bytes.

4. **enforce-branding.py pass (M2c):**
   ```bash
   python3 scripts/enforce-branding.py        # idempotent fix pass
   python3 scripts/enforce-branding.py --check # must exit 0
   ```

**Verification checklist:**
- [ ] `cargo check --workspace --lib` — exits 0
- [ ] `cargo check -p librefang-storage -p librefang-uar-spec -p librefang-memory` — exits 0
- [ ] `python3 scripts/enforce-branding.py --check` — exits 0
- [ ] `cargo test -p librefang-api --test config_schema_golden` — exits 0
- [ ] `grep "default_a2a_name\|BossFang Agent OS" crates/librefang-types/src/config/types.rs` — confirms BossFang default preserved
- [ ] `grep "BossFang" crates/librefang-types/src/approval.rs` — confirms totp_issuer preserved
- [ ] `grep -c "idx_sessions_agent_updated\|idx_audit_agent_timestamp" crates/librefang-storage/src/migrations/sql/030_composite_indexes.surql` — 2 hits
- [ ] Tauri: `productName: "BossFang"`, `identifier: "ai.bossfang.desktop"` — confirmed post-merge
- [ ] `grep "kreuzberg" Cargo.lock | grep "ssh://"` — 0 hits (M1 must land first or be part of same merge)

**Layer constraint check:**
- SurrealDB migration v030: new migration file only, no Rust identifiers renamed. PASS.
- Golden fixture: auto-generated bytes, not authored. PASS.
- enforce-branding pass: Surface layer only. PASS.
- No Layer Internal renames. PASS.

---

## PR Sequencing

```
main
 └─ PR-A: feat/m1-m3-ci-fix-and-gitattributes
     (kreuzberg + .gitattributes — can land before or after PR-B)
 └─ PR-B: chore/m2-upstream-merge
     (proper git merge upstream/main — targets main independently)
```

Merge order: PR-A first (unblocks CI for PR-B's CI run), then PR-B.

---

## Execution Notes

- M1: `cargo update -p kreuzberg` may touch other Cargo.lock entries transitively.
  Commit the full Cargo.lock diff — do not cherry-pick or filter it.
- M2: Use a dedicated worktree (`/tmp/librefang-upstream-merge-p3`) since the
  merge creates many changed files. Do NOT merge in the main worktree.
- M2: After `git merge upstream/main --no-edit`, count conflicts before resolving:
  `grep -l "<<<<<<" $(git diff --name-only --diff-filter=U)`. Resolve in this order:
  (1) take-ours files, (2) take-upstream files, (3) manual blends (index.css).
- M2: If golden regen produces different bytes than expected, check whether
  upstream changed any config struct field names (not just removing AutoRouteStrategy).
- M3: The `merge.ours.driver` git config only activates when the driver is registered.
  Add `git config merge.ours.driver true` to the CLAUDE.md `just setup` section
  OR add it to `scripts/hooks/` (same file that registers `core.hooksPath`).

---

## Constraints Carried Forward

- Three-layer principle (Internal never renamed, Boundary additive, Surface OK)
- Default-path `~/.librefang/` unchanged
- `API_KEY_LEGACY` removal version: v2026.8.x (from M10)
- shellcheck pre-push: DEFERRED (explicit human decision 2026-05-15)
- SurrealDB version pin: `surrealdb = "=3.0.5"` — NEVER change without coordinating surreal-memory + UAR versions
