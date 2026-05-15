# Phase Plan: phase-2-rebrand-cleanup

**Project:** LibreFang (BossFang fork) — github.com/GQAdonis/librefang
**Date:** 2026-05-15
**Backend:** native-tool (claude-code)
**Decisions recorded:**
- M9b: YES — change Rust code defaults to BossFang values
- enforce-branding.py: YES — extend prose scan to dashboard `.ts`/`.tsx` files
- shellcheck pre-push: DEFER

---

## Change Inventory (3 changes, 3 PRs)

### Change 1: `m8-m11-dashboard-prose-cleanup`
**Milestones:** M8, M11, + enforce-branding.py extension (decision 2)
**Priority:** HIGH — most visible user-facing strings; enables automated protection
**Estimated PR:** ~1 day, low risk

**Scope:**

1. **`scripts/enforce-branding.py` extension** — add dashboard prose pass
   - Extend `PROSE_SCAN_TARGETS` to include `crates/librefang-api/dashboard/src/`
   - Use the existing `PROSE_REPLACEMENTS` table (`\bLibreFang\b` → `BossFang`)
   - Apply to `.ts` and `.tsx` files (no MDX fence/inline-code logic needed — TS/TSX have no markdown syntax to skip)
   - Add at least 4 unit tests to `scripts/test_enforce_branding.py` covering the new dashboard scope
   - Run the extended script → auto-flips the 5 M8 strings

2. **Dashboard string flips (auto-caught by extended script):**
   - `crates/librefang-api/dashboard/src/lib/skillHubs.ts:45` — `"Official LibreFang registry…"` → `"Official BossFang registry…"`
   - `crates/librefang-api/dashboard/src/pages/UsersPage.tsx:187` — `"…to a LibreFang role…"` → `"…to a BossFang role…"`
   - `crates/librefang-api/dashboard/src/pages/UsersPage.tsx:1183` — `"LibreFang does not yet ping…"` → `"BossFang does not yet ping…"`
   - `crates/librefang-api/dashboard/src/pages/SkillsPage.tsx:1578` — `// FangHub is the LibreFang first-party registry` → `// FangHub is the BossFang first-party registry`
   - `crates/librefang-api/dashboard/src/pages/MobilePairingPage.test.tsx:39` — `"Open the LibreFang mobile app…"` → `"Open the BossFang mobile app…"`

3. **App.tsx TS6133 dead-code removal (M11):**
   - `crates/librefang-api/dashboard/src/App.tsx:57-58` — remove `USER_AVATAR_STYLE` and `BRAND_MARK_STYLE` declarations (unused ember-gradient constants never wired to any JSX element)

**Verification checklist:**
- [ ] `python3 scripts/test_enforce_branding.py` — all tests pass (≥19 total, 4 new for dashboard scope)
- [ ] `python3 scripts/enforce-branding.py --check` — exits 0 (dashboard scope covered)
- [ ] `pnpm test --run` (scoped to dashboard) — 588+ tests pass
- [ ] `pnpm typecheck` — TS6133 errors for USER_AVATAR_STYLE/BRAND_MARK_STYLE gone; no new errors

**Layer constraint check:**
- All changes are Surface layer (user-visible strings, dead code removal)
- No Layer Internal symbols renamed
- Three-layer principle: PASS

---

### Change 2: `m9b-config-defaults-doc-sweep`
**Milestones:** M9b
**Priority:** MEDIUM — correct fork identity in service card and TOTP apps
**Estimated PR:** ~1 day, low risk

**Scope:**

1. **Rust code defaults** in `crates/librefang-types/src/config/types.rs`:
   - `fn default_name() -> String` — `"LibreFang Agent OS"` → `"BossFang Agent OS"`
   - Find and update `totp_issuer` default (either a `fn default_totp_issuer()` function or a `#[serde(default = "...")]` attribute literal) — `"LibreFang"` → `"BossFang"`

2. **English docs** — update all fenced code block examples + inline-code table spans:

   | File | Lines | Type | Change |
   |---|---|---|---|
   | `docs/src/app/configuration/features/page.mdx` | 107, 123, 322, 333 | TOML fence + table backtick span | `LibreFang Agent OS` → `BossFang Agent OS`; `"LibreFang"` totp → `"BossFang"` |
   | `docs/src/app/configuration/page.mdx` | 296, 1666, 1682 | TOML fences + backtick span | same |
   | `docs/src/app/security/approvals/page.mdx` | 36 | TOML fence | `totp_issuer = "LibreFang"` → `"BossFang"` |
   | `docs/src/app/integrations/desktop/page.mdx` | 109, 240 | prose bold + table backtick | `"LibreFang Agent OS"` tray tooltip; `"LibreFang"` window title |
   | `docs/src/app/integrations/cli/commands/page.mdx` | 1038 | prose | Windows registry entry name `LibreFang` → `BossFang` |
   | `docs/src/app/integrations/acp/page.mdx` | 100 | JSON fence | `"label": "LibreFang"` → `"label": "BossFang"` |
   | `docs/src/app/integrations/api/communication/page.mdx` | 210 | JSON fence | `"name": "LibreFang"` → `"name": "BossFang"` |

3. **Chinese locale mirrors** (same changes in `docs/src/app/zh/` subtree):
   - `zh/configuration/features/page.mdx` — lines 107, 123, 322, 333
   - `zh/configuration/page.mdx` — lines 295, 1661, 1677
   - `zh/security/approvals/page.mdx` — line 35
   - `zh/integrations/desktop/page.mdx` — lines 109, 240
   - `zh/integrations/api/communication/page.mdx` — line 210

**Do NOT change:**
- `` `LibreFangKernel` ``, `` `LibreFangError` ``, `` `Arc<LibreFangKernel>` `` — Layer Internal Rust types, MUST stay
- `librefang-types`, `librefang-kernel`, etc. crate name references — Layer Internal
- `LIBREFANG_AGENT_ID`, `LIBREFANG_MESSAGE`, `LIBREFANG_RUNTIME` env vars — Layer Internal
- `librefang uninstall` CLI command name — Layer Internal binary name
- `LibreFang Kernel` in architecture diagram box — describes the internal module
- `~/.librefang/` paths — default-path policy, must stay

**Verification checklist:**
- [ ] `cargo check -p librefang-types` — no compile errors
- [ ] `python3 scripts/enforce-branding.py --check` — exits 0 (should already be 0; verify no regressions)
- [ ] Manual spot-check: 3 sampled fenced blocks confirm BossFang values
- [ ] Manual spot-check: 2 sampled inline-code spans in tables confirm BossFang values

**Layer constraint check:**
- `default_name()` return value is a Surface-layer display string — rename allowed
- `totp_issuer` default is a Surface-layer display string — rename allowed
- No Rust identifiers (fn names, struct names, crate names) renamed

---

### Change 3: `m10-legacy-key-removal-planning`
**Milestones:** M10
**Priority:** LOW — bookkeeping; no behavior change
**Estimated PR:** <1 hour, near-zero risk

**Scope:**

1. **TODO comments** in `crates/librefang-api/dashboard/src/api.ts`:
   - `API_KEY_LEGACY` const declaration (line ~1021): add inline comment
     `// TODO(drop-legacy-key): remove after v2026.8.x — see CHANGELOG`
   - `getStoredApiKey()` legacy fallback block (~line 1035–1040): add comment
     `// Legacy read fallback — TODO(drop-legacy-key): remove after v2026.8.x`
   - `setApiKey()` legacy key removal (~line 3363–3364): add comment
     `// TODO(drop-legacy-key): remove both lines after v2026.8.x`
   - `clearApiKey()` legacy key removal (~line 3374–3375): add comment
     `// TODO(drop-legacy-key): remove both lines after v2026.8.x`

2. **CHANGELOG.md** — add to `[Unreleased]` section:
   ```markdown
   ### Upcoming Breaking Changes
   - **`librefang-api-key` localStorage key** — the legacy key read-fallback
     in `dashboard/src/api.ts` (`API_KEY_LEGACY`) is scheduled for removal
     after v2026.8.x (approximately 2 release cycles from the v2026.6.x
     baseline). After removal, any residual `librefang-api-key` values in
     sessionStorage/localStorage will be silently ignored. Users upgrading
     from LibreFang upstream to BossFang should complete the key migration
     before v2026.8.x.
   ```

**Verification checklist:**
- [ ] `pnpm typecheck` — no new errors (comment-only change)
- [ ] `pnpm test --run` — all tests still pass (no logic change)
- [ ] `grep -n "TODO(drop-legacy-key)" crates/librefang-api/dashboard/src/api.ts` — 4 hits

**Layer constraint check:** Comments + CHANGELOG only. No logic change. PASS.

---

## PR Sequencing

```
main
 └─ PR-A: feat/m8-m11-dashboard-prose-cleanup
     (no upstream conflict surface — upstream never touches skillHubs.ts / UsersPage.tsx / enforce-branding.py)
 └─ PR-B: feat/m9b-config-defaults-doc-sweep
     (based on main, not chained to PR-A)
 └─ PR-C: feat/m10-legacy-key-removal-planning
     (based on main, not chained to either above — comment-only, no ordering dep)
```

All three PRs target `main` independently. No ordering constraint — they can be
reviewed and merged in any order. This avoids the chained-PR sequencing overhead
that slowed Phase 1.

---

## Execution Notes

- M8 enforce-branding.py extension: apply to `.ts`/`.tsx` files **without** the
  MDX fence-skipping logic — TypeScript files contain no markdown syntax to skip.
  The existing `apply_replacements()` or equivalent function works directly.
- M9b doc sweep: `enforce-branding.py` still CANNOT help with fenced code blocks
  or inline-code spans (intentionally skipped). Manual edits required. The count
  is bounded (~20 files, ~25 individual substitutions) and all follow the same
  pattern: `LibreFang Agent OS` → `BossFang Agent OS`, `"LibreFang"` → `"BossFang"`.
- M9b Rust side: confirm `totp_issuer` default location before editing. Likely
  a `fn default_totp_issuer() -> String` or `#[serde(default = "default_totp_issuer")]`
  attribute. Grep: `grep -n "totp_issuer\|LibreFang" crates/librefang-types/src/config/types.rs`
- M10 is read-only from a logic standpoint — no test changes needed beyond
  verifying the existing test suite still passes.

---

## Constraints Carried Forward

- Three-layer principle (Internal never renamed, Boundary additive, Surface OK)
- Default-path `~/.librefang/` unchanged
- Domain ownership: `bossfang.ai` not claimed
- `API_KEY_LEGACY` removal version: v2026.8.x (locked in M10)
- shellcheck pre-push: DEFERRED (explicit human decision 2026-05-15)
