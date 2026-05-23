# Phase Assessment: phase-3-upstream-sync-and-ci-hardening

**Project:** LibreFang (BossFang fork) — github.com/GQAdonis/librefang
**Date:** 2026-05-23
**Prior context:** `.kbd-orchestrator/phases/phase-2-rebrand-cleanup/reflection.md`
**Assessed against:** `origin/main` (HEAD: `348db3101`, PR #23 merged) vs `upstream/main` (HEAD: `debb31501`)
**Merge base:** `090909b2aa` — "docs: update contributors and star history (#5276)" (2026-05-14)

---

## Summary

Phase 3 has two primary obligations and one process improvement:

1. **CI is broken** for any job that exercises `--all-features` due to a
   kreuzberg SSH transitive dependency (`ssh://git@github.com/GQAdonis/kreuzberg.git`).
   CI runners have no SSH key. Fix: `.cargo/config.toml` URL rewrite (minutes).

2. **224 upstream commits** are unmerged. BossFang's squash-merge approach for PRs
   #21/#22 (114 commits) means git's merge base is still at `090909b2a` — but the
   content of those 114 commits is already in our tree. The true "new" upstream work
   since our last sync is **110 commits** (2026-05-18 → 2026-05-23 upstream activity
   + backlog from the squash-gap). A proper `git merge upstream/main` will auto-resolve
   the 114-commit squash overlap (content matches) and require manual resolution only on
   BossFang overlay files.

3. **Process hardening**: `.gitattributes` orchestrator merge-strategy prevents the
   recurring rebase conflicts that required manual intervention on all 3 Phase-2 PRs.

**Gap count: 3 milestones, 1 Decisions required.**

---

## Codebase Scan Results

### M1 — kreuzberg SSH CI block (CONFIRMED — immediate fix)

**Root cause:** `universal-agent-runtime` (BossFang-exclusive UAR crate) depends
on `kreuzberg` via SSH URL. The Cargo.lock entry:

```
name = "kreuzberg"
source = "git+ssh://git@github.com/GQAdonis/kreuzberg.git?branch=main#84b77f914738a9fe9dcaa3a05fc6ea7f637ae5f3"
```

CI `cargo check --workspace --lib` and any `cargo metadata --all-features` call
fail with `failed to authenticate when downloading repository` on all CI runners.
This blocked PR #16 (closed as superseded) and would block any future PR that
touches a crate with `uar-driver` in its feature graph.

**Direct dependency chain:**
```
librefang-llm-drivers [feature: uar-driver]
  └─ universal-agent-runtime (https://github.com/GQAdonis/universal-agent-runtime.git)
       └─ kreuzberg (ssh://git@github.com/GQAdonis/kreuzberg.git) ← blocks CI
```

Note: `librefang-llm-drivers/Cargo.toml` already uses HTTPS for UAR:
```toml
universal-agent-runtime = { git = "https://github.com/GQAdonis/universal-agent-runtime.git", branch = "main", optional = true, ... }
```
The kreuzberg SSH URL is a transitive dep introduced by UAR's own `Cargo.lock` /
`Cargo.toml`. We cannot fix it by editing our own Cargo.toml.

**Fix options:**

| Option | Effort | Risk | Notes |
|---|---|---|---|
| **A: `.cargo/config.toml` URL rewrite** | 5 min | Very low | Standard Cargo `[url."https://..."] insteadOf = "ssh://..."` pattern; affects Cargo.lock on next `cargo update` for that dep |
| B: Fork kreuzberg, change its URL to HTTPS in UAR | Hours | Medium | Requires updating UAR fork + re-pinning |
| C: Pin UAR to a ref where kreuzberg uses HTTPS | Medium | Low | Requires checking UAR commit history |

**Recommended: Option A.** Add to `.cargo/config.toml`:
```toml
[url."https://github.com/GQAdonis/kreuzberg.git"]
insteadOf = "ssh://git@github.com/GQAdonis/kreuzberg.git"
```

After the rewrite, run `cargo update -p kreuzberg` to refresh the Cargo.lock entry
to the HTTPS form. Verify with `cargo check -p librefang-llm-drivers --features uar-driver`.

**Layer constraint:** Build-system configuration only. No Rust code changes. PASS.

---

### M2 — Upstream merge (224 commits, ~110 genuinely new)

**Commit accounting:**
- Merge base: `090909b2a` (our last real upstream ancestor, 2026-05-14)
- PRs #21/#22 squash-applied: 114 upstream commits (content already in our tree, git unaware)
- New upstream commits since 2026-05-18 content sync: **110 commits**
- Git-visible gap: 224 commits (will resolve as ~114 auto-merge + ~110 real work)

**BossFang overlay file impact — ALL CLEAN:**

| Overlay file | Upstream touched? | Action |
|---|---|---|
| `crates/librefang-api/dashboard/src/index.css` | **YES — additive only** | Take ours + append upstream's `input[type="number"]` spinner CSS (23 lines) |
| `crates/librefang-api/dashboard/src/App.tsx` | No | Take ours |
| `crates/librefang-api/dashboard/index.html` | No | Take ours |
| `crates/librefang-api/dashboard/public/manifest.json` | No | Take ours |
| `crates/librefang-desktop/tauri.conf.json` | No | Take ours |
| `crates/librefang-desktop/tauri.desktop.conf.json` | No | Take ours |
| `crates/librefang-api/src/webchat.rs` | No | Take ours |
| `crates/librefang-types/src/config/types.rs` | **YES — additive** | Take upstream, verify BossFang Surface defaults preserved |
| `crates/librefang-skills/src/marketplace.rs` | No | Take ours |

**Tauri drift check (run as part of Phase 3 audit):**
All four Tauri configs currently correct: `productName: "BossFang"`,
`identifier: "ai.bossfang.desktop"` / `"ai.bossfang.app"`. Upstream did not touch
them in the 224-commit window. Verify again post-merge.

**enforce-branding.py audit (run as part of Phase 3 audit):**
Current state: `--check` exits 0. New dashboard components added by upstream:
`AgentSchedulePanel.tsx`, `NotificationCenter.tsx`, `OperatorActionBar.tsx`,
`PendingOperatorReviewsBanner.tsx`, `ScheduleModal.tsx`. Manual spot-check found
**no LibreFang strings or sky-blue tokens** in any of them. Run `--check` after
merge to confirm.

**Required substantive changes:**

#### 2a — SurrealDB migration v030: composite indexes

Upstream SQLite migration v41 adds two composite indexes for query performance:

```sql
CREATE INDEX IF NOT EXISTS idx_sessions_agent_updated
    ON sessions(agent_id, updated_at);

CREATE INDEX IF NOT EXISTS idx_audit_agent_timestamp
    ON audit_entries(agent_id, timestamp);
```

BossFang SurrealDB already has:
- `sessions`: `sessions_agent_idx ON sessions COLUMNS agent_id` (single-column)
- `audit_entries`: `audit_entries_agent_idx` (single-column on `agent_id`)

No composite (agent_id + timestamp/updated_at) index exists. Need:

**File to create:** `crates/librefang-storage/src/migrations/sql/030_composite_indexes.surql`

```surql
-- Migration v30: Composite indexes for per-agent recency hot-paths (#5641).
-- Mirrors SQLite migration v41.
--
-- `sessions(agent_id, updated_at)`: used by count_agent_sessions_touched_since
-- (concurrent-trigger admission check, runs on every cron/trigger fire).
-- `audit_entries(agent_id, timestamp)`: used by per-agent audit history queries.
-- Both are `IF NOT EXISTS` — idempotent on fresh installs.

DEFINE INDEX IF NOT EXISTS idx_sessions_agent_updated
    ON sessions COLUMNS agent_id, updated_at;

DEFINE INDEX IF NOT EXISTS idx_audit_agent_timestamp
    ON audit_entries COLUMNS agent_id, timestamp;
```

Also register in `crates/librefang-storage/src/migrations/mod.rs` as migration v30.

#### 2b — KernelConfig new fields (additive, no storage required)

Upstream added two new fields to existing structs:

| Struct | Field | Type | Default | Storage |
|---|---|---|---|---|
| `SidecarChannelConfig` | `default_agent` | `Option<String>` | `None` | No |
| `KernelConfig` | `external_auth_proxy` | `bool` | `false` | No |

Both have `#[serde(default)]` in upstream. The `Default` impl for `KernelConfig`
needs `external_auth_proxy: false`. The `SidecarChannelConfig` uses serde default.

Note: `cache_approvals_per_session` was added to `ApprovalConfig` (in-memory cache
for per-session approval results — no storage). The golden fixture will need
regeneration regardless since `AutoRouteStrategy` was removed from the schema.

#### 2c — Golden fixture regeneration

`crates/librefang-api/tests/fixtures/kernel_config_schema.golden.json` diverged:
upstream removed `AutoRouteStrategy` from the schema. After merging and updating
the `Default` impl, regenerate:

```bash
cargo test -p librefang-api --test config_schema_golden -- \
    --ignored regenerate_golden --nocapture
```

Commit the regenerated bytes along with the merge commit.

#### 2d — Merge strategy decision (REQUIRES HUMAN DECISION)

The squash-merge approach used for PRs #21 and #22 prevents git from tracking
which upstream commits are incorporated. Every subsequent `git merge upstream/main`
will compute the same `090909b2a` merge base, growing the "224 commit" gap even
after we've applied the content. This compounds with each squash-merge cycle.

**Option X (recommended): Proper merge commit.**
Use `git merge upstream/main --no-edit` which creates a real merge commit,
making all upstream commits direct ancestors of our main. Future merges will
correctly compute a merge base at the tip of the previous upstream merge.
Conflict surface: 1 BossFang overlay file (`index.css` — additive only) plus
orchestrator JSON files. Expected <15 conflicts, all in known files.

**Option Y (status quo): Continue squash-merges.**
The growing merge-base problem persists. Every future upstream merge will appear
to git as "N commits" even if we've already applied most of them. Conflict
resolution remains manual but follows the same pattern (content matches = auto).
Risk grows as N increases.

---

### M3 — .gitattributes orchestrator merge strategy

**Current state:** `.gitattributes` only covers:
```gitattributes
crates/librefang-api/tests/fixtures/*.json text eol=lf
```

**Required addition:**
```gitattributes
# Orchestrator state files: always keep our branch version on rebase/merge.
# The branch version always has the more advanced phase state.
.kbd-orchestrator/**/*.json merge=ours
.kbd-orchestrator/**/*.md merge=ours
```

This requires `git config merge.ours.driver true` in the repo or CI. Since the
hook is declared in `.gitattributes`, git will use it automatically when the
attribute matches — but developers must enable it once per clone with:
```bash
git config merge.ours.driver true
```

**Alternative: just document the pattern.**
Given the low frequency of orchestrator PRs (3 per phase), the merge=ours
gitattribute adds complexity for minimal gain. The manual `git checkout --theirs`
pattern is understood and takes < 30 seconds.

**Layer constraint:** `.gitattributes` only. No code change. PASS.

---

### Out-of-scope items confirmed

**Hardcoded URL audit result:** Only one new upstream URL reference found:
```
`@librefang/whatsapp-gateway` as a separate process
```
This is documentation prose in a comment — not a network URL. No action required.
The three origin-repoint knobs (registry base_url, dashboard tarball URL, marketplace
github_org) are unchanged in upstream's 224-commit window.

**UAR / LlmDriver compatibility:** The `LlmDriver` trait requires only `complete()`.
`UarDriver` implements both `complete()` and `stream()`. No compatibility work needed
for the merge.

**Memory isolation feature (#5071):** No new storage tables. The per-agent isolation
is a runtime parameter passed to existing memory lookup functions. No SurrealDB
migration needed.

**Approval caching (#5663):** In-memory HashMap only (`cache_approvals_per_session`
field controls it). No storage schema change.

**Channel sidecar migrations (bluesky, reddit):** Take upstream as-is.
Layer Internal channel adapter code — no BossFang overlay.

**shellcheck pre-push hook:** Remains DEFERRED per 2026-05-15 human decision.

---

## Layer Integrity Check

| Constraint | Status |
|---|---|
| Three-layer principle | PASS — M1 is build config, M2 SurrealDB migration is new-table, M3 is gitattributes |
| No Layer Internal renames | PASS — kreuzberg fix doesn't rename any Rust symbol |
| Default-path policy (`~/.librefang/`) | PASS — not touched in any Phase-3 change |
| `API_KEY_LEGACY` timeline | PASS — v2026.8.x gate from M10 unchanged |
| BossFang Surface defaults | VERIFY — confirm `default_a2a_name()` and `default_totp_issuer()` survive the merge (they're in `config/types.rs` and `approval.rs` which upstream also touched) |

---

## Decisions Required Before Plan

### Decision 1: Upstream merge strategy (M2) — proper merge vs continue squash

**Context:** Squash-merging continues to grow the merge-base debt. Each future
upstream merge will git-see 100–300+ commits even if most are already applied.
Proper merge eliminates this permanently.

**Options:**

- **YES (recommended):** Use `git merge upstream/main --no-edit` for M2. Creates a
  real merge commit. Conflict surface is bounded: 1 overlay file (`index.css` —
  additive only), orchestrator JSON files (always-take-ours). Expected total
  conflict resolution time: <30 minutes.

- **NO:** Continue squash-merge pattern. Operationally identical result, but the
  merge-base problem compounds with every cycle.

### Decision 2: .gitattributes orchestrator merge=ours (M3) — implement or document

**Context:** The `merge=ours` gitattribute automates what we do manually (`git
checkout --theirs` on orchestrator files). It requires a one-time `git config
merge.ours.driver true` per clone, which adds a setup step.

**Options:**

- **YES — implement:** Add `.gitattributes` entries + document in CLAUDE.md "just
  setup" note. Saves ~2 minutes per future orchestrator-file conflict.

- **NO — document only:** Add to CLAUDE.md that orchestrator files use
  `git checkout --theirs`. No `.gitattributes` change.

---

## Recommended Milestone Sequencing

| Milestone | Scope | PR Strategy | Upstream conflict risk |
|---|---|---|---|
| **M1** — kreuzberg CI fix | `.cargo/config.toml` + Cargo.lock update | Standalone | Near zero |
| **M2** — upstream merge (224 commits) | Full `git merge upstream/main` + SurrealDB v030 + golden regeneration | Standalone merge PR | Medium (known overlay files only) |
| **M3** — .gitattributes orchestrator | `.gitattributes` + CLAUDE.md note | Bundle with M1 (both small) | Near zero |

**Phase 3 PRs:** 2 total (PR-A = M1+M3, PR-B = M2 upstream merge)

M1+M3 can land before or after M2 — no ordering dependency.
M2 should be the final commit into main (so enforce-branding.py runs against
the full merged state, not an intermediate state).

---

## Files Requiring Changes

**M1 (`.cargo/config.toml` fix):**
- `.cargo/config.toml` — add `[url."https://github.com/GQAdonis/kreuzberg.git"]` insteadOf block
- `Cargo.lock` — regenerate kreuzberg entry from SSH → HTTPS (via `cargo update -p kreuzberg`)

**M2 (upstream merge):**
- `crates/librefang-storage/src/migrations/sql/030_composite_indexes.surql` — new file
- `crates/librefang-storage/src/migrations/mod.rs` — register migration v30
- `crates/librefang-api/tests/fixtures/kernel_config_schema.golden.json` — regenerate
- `crates/librefang-api/dashboard/src/index.css` — take ours + append upstream spinner CSS
- All other merge-commit file changes are upstream take-as-is

**M3 (.gitattributes):**
- `.gitattributes` — add `merge=ours` entries for `.kbd-orchestrator/`
- `CLAUDE.md` — add `git config merge.ours.driver true` to `just setup` note (or confirm `just setup` handles it)

---

## Verification Checklist (cross-milestone)

- [ ] `cargo check -p librefang-llm-drivers --features uar-driver` — exits 0 (M1)
- [ ] `cargo check --workspace --lib` — exits 0 (M2)
- [ ] `cargo check -p librefang-storage -p librefang-uar-spec -p librefang-memory` — exits 0 (M2)
- [ ] `python3 scripts/enforce-branding.py --check` — exits 0 (M2)
- [ ] `cargo test -p librefang-api --test config_schema_golden` — exits 0 with regenerated fixture (M2)
- [ ] `grep -c "idx_sessions_agent_updated\|idx_audit_agent_timestamp" crates/librefang-storage/src/migrations/sql/030_composite_indexes.surql` — 2 hits (M2)
- [ ] Tauri configs: `productName: "BossFang"`, `identifier: "ai.bossfang.desktop"` — confirmed post-merge (M2)
- [ ] `grep "kreuzberg" Cargo.lock | grep "ssh://"` — 0 hits after M1 (M1)
