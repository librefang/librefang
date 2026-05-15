# Phase Assessment: phase-2-rebrand-cleanup

**Project:** LibreFang (BossFang fork) — github.com/GQAdonis/librefang
**Date:** 2026-05-15
**Prior context:** `.kbd-orchestrator/phases/phase-1-rebrand-completion/reflection.md`
**Assessed against:** `origin/main` (post-PR #15 merge + upstream 2026-05-15 sync)

---

## Summary

Phase 1 completed all 7 milestones (M1–M7). Phase 2 addresses the four
categories of technical debt surfaced in the phase-1 reflection, plus one
pre-existing TypeScript issue independently identified. All work is bounded
to Surface and Boundary layers — no Layer Internal renames required.

**Gap count: 4 milestones, ~50 remaining file-level changes.**

---

## Codebase Scan Results

### M8 — Dashboard JSX prose (4 files, 5 hits)

`enforce-branding.py` does not cover `crates/librefang-api/dashboard/src/`
for prose (only color tokens). These PascalCase "LibreFang" strings survive
in JSX user-visible text and code comments:

| File | Line | Current text | Action |
|---|---|---|---|
| `crates/librefang-api/dashboard/src/lib/skillHubs.ts` | 45 | `"Official LibreFang registry — curated hands, agents, MCP, providers, plugins."` | → `"Official BossFang registry …"` |
| `crates/librefang-api/dashboard/src/pages/UsersPage.tsx` | 187 | `"…maps a platform identity … to a LibreFang role."` | → `"…to a BossFang role."` |
| `crates/librefang-api/dashboard/src/pages/UsersPage.tsx` | 1183 | `"LibreFang does not yet ping the platform to confirm…"` | → `"BossFang does not yet ping…"` |
| `crates/librefang-api/dashboard/src/pages/SkillsPage.tsx` | 1578 | `// FangHub is the LibreFang first-party registry` | → `// FangHub is the BossFang first-party registry` |
| `crates/librefang-api/dashboard/src/pages/MobilePairingPage.test.tsx` | 39 | `"Open the LibreFang mobile app and tap…"` | → `"Open the BossFang mobile app…"` |

**Layer classification:** All Surface. No identifier renaming. No conflict
surface (no upstream file touches these strings).

**Recommended approach:** Single PR. Mechanical string flip. No new tests
required (test file string is in an assertion that tests behavior, not the
string itself — verify with `pnpm test --run` scoped to MobilePairingPage).

---

### M9 — Config default values + stale fenced/inline-code docs (2 code changes + ~30 doc changes)

The M4 `enforce-branding.py` prose sweep intentionally preserved:

1. **Fenced code blocks** — TOML config examples containing `LibreFang` values
2. **Inline-code spans** — backtick spans documenting `LibreFang` default values

These are now split into two sub-categories requiring different treatment:

#### M9a — Layer Internal references in backtick spans (NO ACTION — preserve)

These document Rust types and function names that MUST NOT be renamed per the
three-layer principle. Leave as-is in all docs:

- `` `LibreFangKernel` ``, `` `LibreFangError` ``, `` `LibreFangKernel::boot(None)` ``
- `` `Arc<LibreFangKernel>` ``
- All `librefang-*` crate name references (`` `librefang-types` ``, etc.)
- All `librefang_*` function references (`` `librefang_home()` ``, etc.)
- `` `LIBREFANG_AGENT_ID` ``, `` `LIBREFANG_MESSAGE` ``, `` `LIBREFANG_RUNTIME` ``
  env vars (deep-plumbing, deliberately un-aliased per constraints.md)

#### M9b — Stale config-default documentation (REQUIRES DECISION)

The Rust code at `crates/librefang-types/src/config/types.rs` contains:

```rust
fn default_name() -> String {
    "LibreFang Agent OS".to_string()   // ← Surface layer value
}
```

and `totp_issuer` defaults to `"LibreFang"` (exact function TBD — not
found by grep, may be a `#[serde(default = "...")]` inline literal).

The docs accurately document the current code defaults. However, since these
are **Surface layer** values (displayed in the well-known agent card and TOTP
authenticator apps), BossFang should ship with its own defaults.

**Files affected if code default is changed:**

| File | Lines | Type | Change |
|---|---|---|---|
| `crates/librefang-types/src/config/types.rs` | `default_name()` | Rust | `"LibreFang Agent OS"` → `"BossFang Agent OS"` |
| `crates/librefang-types/src/config/types.rs` | totp_issuer default | Rust | `"LibreFang"` → `"BossFang"` |
| `docs/src/app/configuration/features/page.mdx` | 107, 123, 322, 333 | MDX | 2 TOML fences + 2 table backtick spans |
| `docs/src/app/configuration/page.mdx` | 296, 1666, 1682 | MDX | 2 TOML fences + 1 table backtick span |
| `docs/src/app/security/approvals/page.mdx` | 36 | MDX | TOML fence |
| `docs/src/app/integrations/desktop/page.mdx` | 109, 240 | MDX | prose bold + table backtick span |
| `docs/src/app/integrations/cli/commands/page.mdx` | 1038 | MDX | CLI docs Windows registry entry name |
| `docs/src/app/integrations/acp/page.mdx` | 100 | MDX | JSON example label field |
| `docs/src/app/integrations/api/communication/page.mdx` | 210 | MDX | JSON example name field |
| + ZH mirrors of each of the above | — | MDX | same flips in zh/ locale |

**Estimated total:** ~20 doc file edits (EN + ZH), 1 Rust file.

**Decision needed (pre-plan):** Proceed with M9 as described above? Changing
the code default causes any BossFang instance that relies on the compiled
default to show "BossFang Agent OS" instead of "LibreFang Agent OS" in its
well-known agent card and TOTP app — the correct behavior for BossFang.
Existing config.toml files with an explicit `name = "..."` entry are
unaffected (the default is only used when the field is absent).

#### M9c — Prose LibreFang refs in docs that enforce-branding missed (SWEEP)

Post-M4, some prose LibreFang references survive in docs because they appear
in files that weren't covered or in context the script misclassified. Running
`python3 scripts/enforce-branding.py --check` on origin/main (done as part of
the 2026-05-15 upstream merge audit) returned **exit 0** — so none of these
survive the current script. This means:

- Some are Layer Internal symbols in backtick spans (correct, skip)
- Some are in fenced code blocks (correct, skip)
- The rest were caught and fixed by M4 or by the 2026-05-15 post-merge patch

**Conclusion:** M9c requires no additional work unless a future upstream merge
introduces new prose. enforce-branding.py already handles that on each merge.

---

### M10 — Legacy localStorage key removal planning (comments + CHANGELOG)

The M6 read-old/write-new shim at
`crates/librefang-api/dashboard/src/api.ts` contains:

```typescript
// API_KEY_LEGACY is the old "librefang-api-key" accepted as a read-fallback
const API_KEY_LEGACY = "librefang-api-key";  // line 1021
```

Four `API_KEY_LEGACY` removal sites at lines ~1035–1036, ~3363–3364, ~3374–3375
in `api.ts`. The reflection documented these but did not commit a version
target for removal.

**Required actions (docs + comments only, no logic change):**

1. Add `// TODO(drop-legacy-key): remove after v2026.8.x — see CHANGELOG` to
   each of the 4 removal sites in `api.ts`
2. Update `CHANGELOG.md [Unreleased]` section:

   ```markdown
   ### Upcoming Breaking Changes
   - **`librefang-api-key` localStorage key** — the legacy key read-fallback
     (`API_KEY_LEGACY` in `dashboard/src/api.ts`) is scheduled for removal
     after v2026.8.x (approximately 2 release cycles from v2026.6.x baseline).
     Users upgrading from LibreFang to BossFang should complete the upgrade
     before that version. After removal, any residual `librefang-api-key`
     value in sessionStorage/localStorage will be silently ignored.
   ```

3. Add `// TODO(drop-legacy-key)` inline at the `API_KEY_LEGACY` const
   declaration itself to make IDE searches find all removal sites at once

**Risk:** VERY LOW. No logic changes; comments and CHANGELOG only.

---

### M11 — App.tsx pre-existing TS6133 unused variables (pre-existing debt)

`crates/librefang-api/dashboard/src/App.tsx` lines 57–58 declare two constants
that are never read:

```typescript
const USER_AVATAR_STYLE = { background: "linear-gradient(135deg,#FF6A3D,#E04E28)" } as const;
const BRAND_MARK_STYLE  = { background: "linear-gradient(135deg,#FF6A3D,#FF6A3D)" } as const;
```

`pnpm typecheck` fails with `TS6133: 'USER_AVATAR_STYLE' is declared but its
value is never read.` on every branch (pre-existing). A spawn task was filed
in phase 1. Phase 2 should close this.

**Fix options:**
- A: Remove both declarations (if they're truly dead code — check git log for
  when they were added and whether they're referenced anywhere via global search)
- B: Apply them to an element in App.tsx that currently inline-styles (if there's
  a legitimate use case that was just never wired up)

**Recommended:** Option A — remove both. They're ember-gradient constants that
aren't wired to any JSX element; App.tsx uses Tailwind and CSS variables for
its gradient rendering, not inline style objects.

**Risk:** LOW. Small change; verify with `pnpm typecheck` and `pnpm test`.

---

## Layer Integrity Check

| Constraint | Status |
|---|---|
| Three-layer principle | PASS — all M8–M11 changes are Surface or dead-code removal |
| No Layer Internal renames | PASS — `LibreFangKernel`, `librefang_home`, `librefang-*` crates untouched |
| Default-path policy (`~/.librefang/`) | PASS — not touched in any planned change |
| Domain ownership | PASS — no `bossfang.ai` references being claimed |
| `API_KEY_LEGACY` timeline | PLANNED — M10 defines the v2026.8.x drop gate |

---

## Recommended Milestone Sequencing

| Milestone | Scope | PR Strategy | Upstream conflict risk |
|---|---|---|---|
| **M8** — Dashboard JSX prose | 4 files, 5 string flips | Standalone | Near zero |
| **M11** — App.tsx TS6133 | 1 file, 2 line deletions | Bundle with M8 (same PR, same `pnpm` verify) | Near zero |
| **M9b** — Code defaults + doc sweep | 1 Rust file + ~20 MDX files | Standalone | Low (upstream rarely touches `default_name()`) |
| **M10** — Legacy key planning | 1 TS file + CHANGELOG | Standalone (docs-only) | Near zero |

**Phase 2 PRs:** 3 total (#17 = M8+M11, #18 = M9b, #19 = M10)

---

## Architecture Decisions Requiring Human Review Before Plan

1. **M9b decision: change code defaults?**
   - YES (recommended): change `default_name()` to `"BossFang Agent OS"` and
     `default_totp_issuer()` to `"BossFang"`. Surface layer, correct for fork.
   - NO: leave code defaults, add a prose note in docs saying the default differs
     between LibreFang upstream and BossFang.

2. **Should `enforce-branding.py` extend to dashboard JSX prose?**
   - Yes: add `crates/librefang-api/dashboard/src/` to the prose scan scope.
     Would catch M8 automatically on future merges.
   - No: keep dashboard coverage limited to color tokens (current behavior).
     Dashboard prose flips stay manual.

3. **Should `shellcheck` be added to the pre-push hook?**
   - Current: `shellcheck web/public/install.sh` is CI-only (deferred in M3).
   - Add to `scripts/hooks/pre-push` alongside the existing clippy + OpenAPI
     drift checks.

---

## What Is Out of Scope for Phase 2

- `LIBREFANG_REGISTRY_VERIFY`, `LIBREFANG_REGISTRY_INDEX_URL`, and ~57 other
  deep-plumbing env vars — deliberately deferred (see constraints.md)
- Default home dir `~/.librefang/` — no data migration
- Default dashboard user/pass `librefang` — no value migration
- Domain registration of `bossfang.ai`
- Building or signing desktop bundles

---

## Files Requiring Changes (summary)

**M8 + M11 (single PR):**
- `crates/librefang-api/dashboard/src/lib/skillHubs.ts` — 1 string flip
- `crates/librefang-api/dashboard/src/pages/UsersPage.tsx` — 2 string flips
- `crates/librefang-api/dashboard/src/pages/SkillsPage.tsx` — 1 comment flip
- `crates/librefang-api/dashboard/src/pages/MobilePairingPage.test.tsx` — 1 string flip
- `crates/librefang-api/dashboard/src/App.tsx` — 2 line deletions (USER_AVATAR_STYLE, BRAND_MARK_STYLE)

**M9b (standalone PR) — PENDING HUMAN DECISION:**
- `crates/librefang-types/src/config/types.rs` — 2 default function bodies
- `docs/src/app/configuration/features/page.mdx` — lines 107, 123, 322, 333
- `docs/src/app/configuration/page.mdx` — lines 296, 1666, 1682
- `docs/src/app/security/approvals/page.mdx` — line 36
- `docs/src/app/integrations/desktop/page.mdx` — lines 109, 240
- `docs/src/app/integrations/cli/commands/page.mdx` — line 1038
- `docs/src/app/integrations/acp/page.mdx` — line 100
- `docs/src/app/integrations/api/communication/page.mdx` — line 210
- + ZH mirrors (8 files, same line patterns)

**M10 (standalone PR):**
- `crates/librefang-api/dashboard/src/api.ts` — 4–5 TODO comment additions
- `CHANGELOG.md` — [Unreleased] section addition
