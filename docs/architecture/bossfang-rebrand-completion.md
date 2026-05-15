# BossFang Rebrand Completion Roadmap

> **Status**: planning doc. No code changes here. This is the structured
> output of `/kbd-assess` against the goal *"complete the user-facing
> BossFang rebrand while preserving cheap upstream merges"*.
>
> Conversion work happens in follow-up PRs sequenced per the schedule
> at the bottom.

## 1. The three-layer principle (load-bearing)

Every BossFang fork change classifies into one of three layers. The
roadmap below respects them; future PRs must too.

| Layer | Rule | Examples |
|---|---|---|
| **Internal** | NEVER rename — explodes merge conflicts | Cargo crate names (`librefang-types`, `librefang-kernel`, …), Rust module/symbol names (`LibreFangKernel`, `librefang_runtime::tool_runner`), Python module names (`librefang_sdk`, `librefang_client`), function names like `librefang_home()`, workspace member paths in `Cargo.toml`, test fixture identifiers |
| **Boundary** | Additive aliases only — never delete the legacy form | Env vars: `BOSSFANG_HOME` primary + `LIBREFANG_HOME` fallback. Config keys, on-disk paths (`~/.librefang/`), test-fixture seeds. Default values stay LibreFang-flavoured to preserve in-place upgrade. |
| **Surface** | Full rename OK — always-take-ours during conflict resolution | CLI banners, product name, install script echoes, README/docs body text, content-type vendor prefix, release artifact names, dashboard chrome |

Decision rule when in doubt:

> *If I rename this, how many merge conflicts does the next upstream pull cause?*
> - Zero or one (config / metadata) → **Surface**, rename freely
> - A handful → **Boundary**, alias additively
> - Dozens (hot edit zone) → **Internal**, leave alone

Reference: `.claude/skills/librefang-upstream-merge/references/three-layer-rebrand.md`.

## 2. Done inventory (Phase 1 + Phase 2 PRs)

The following surface flips have landed:

- **PR #1** — Initial branding scaffolding (ember palette in
  `dashboard/src/index.css`, `boss-libre.png` logo in `dashboard/src/App.tsx`
  sidebar + mobile header, "BossFang" wordmark)
- **PR #2** — UAR fork repoint (`Cargo.toml` dep on
  `Prometheus-AGS/universal-agent-runtime` fork)
- **PR #3** — Registry base_url default to `GQAdonis/librefang-registry`
- **PR #4** — Tauri signing keys (BossFang minisign keypair, key ID
  `E329A6B2863F1707`), updater endpoint pinned at
  `github.com/GQAdonis/librefang`
- **PR #5** — User-facing rebrand (dual `[[bin]] bossfang|librefang` in
  `librefang-cli/Cargo.toml`, npm `@bossfang/sdk`, PyPI `bossfang-sdk`,
  Cargo `bossfang-sdk`, Tauri `productName: "BossFang"` + `identifier:
  ai.bossfang.{desktop,app}`, dashboard chrome — title / `manifest.json`
  / `index.html`, content-type vendor prefix
  `application/vnd.bossfang.v1+json` with `application/vnd.librefang.*`
  back-compat, release artifact names `bossfang-<target>.{tar.gz,zip}`,
  README/CONTRIBUTING product-name flips, UA strings:
  `BossFang-Webhook/1.0`, `BossFang/0.1` for skillhub/clawhub,
  `bossfang-skills/0.1` for marketplace, `bossfang-plugin-{updater,search}/1.0`)
- **PR #6** — `BOSSFANG_HOME` + `BOSSFANG_VAULT_KEY` env-var aliases
  (Boundary layer, additive)
- **PR #7** — `librefang-upstream-merge` agentskills.io skill (this
  roadmap's tooling foundation)

## 3. Remaining surface inventory (post-merge audit)

Grouped into milestones. Each milestone is a single follow-up PR.

### M1 — Operator banners and desktop install paths (HIGH)

User-facing strings printed at CLI startup and during desktop-install
flows. Pure Surface — full rename, no back-compat needed.

**Files:**

- [crates/librefang-cli/src/ui.rs:50](../../crates/librefang-cli/src/ui.rs) — `"LibreFang Agent OS"` brand banner → `"BossFang Agent OS"`
- [crates/librefang-cli/src/desktop_install.rs](../../crates/librefang-cli/src/desktop_install.rs) — **15 user-facing references**:
  - Line 81: `ui::hint("LibreFang Desktop is not installed.")` → `"BossFang Desktop is not installed."`
  - Line 176: `ui::success("LibreFang Desktop installed successfully.")` → `"BossFang Desktop installed successfully."`
  - Lines 246, 255, 281, 329, 335, 339, 367, 371, 391, 394, 577 — installer file-path strings (`LibreFang.app`, `LibreFang.exe`, `LibreFang.AppImage`)

  **Important nuance**: install-path strings represent **real artifact
  filenames** in the user's filesystem (`/Applications/LibreFang.app`,
  `%LOCALAPPDATA%\LibreFang\LibreFang.exe`). Renaming the *string* in
  the installer without **also** renaming the artifact that
  `tauri build` produces orphans existing installs and breaks the next
  install (the installer would no longer detect/clean the old version).

  **Recommended approach**: read both names on detect (prefer
  BossFang.app, fall back to LibreFang.app), write the new name on
  install — same shim pattern as the localStorage migration in M6. Add
  one-time auto-migration step that renames an existing
  `/Applications/LibreFang.app` → `/Applications/BossFang.app` so users
  upgrade transparently. Coordinate with the Tauri `productName` field
  (already `"BossFang"`) so freshly-built bundles already carry the new
  name.

- [crates/librefang-cli/src/desktop_install.rs:117](../../crates/librefang-cli/src/desktop_install.rs:117), [:191](../../crates/librefang-cli/src/desktop_install.rs:191) — UA header `librefang-cli` → `bossfang-cli`

**Estimated PR size**: ~30 lines changed, 1 new helper function.

### M2 — Init wizard + config template (HIGH)

The init flow drops these onto disk on first run. New users see the
LibreFang wording immediately.

**Files:**

- [crates/librefang-cli/templates/init_default_config.toml](../../crates/librefang-cli/templates/init_default_config.toml) — **9 references**:
  - Line 2: `# LibreFang Agent OS — Configuration` → `# BossFang Agent OS — Configuration`
  - Line 3: `# Docs: https://docs.librefang.ai/configuration` → `# Docs: https://github.com/GQAdonis/librefang` (no `docs.bossfang.ai` domain owned yet)
  - Line 12: `https://docs.librefang.ai/security/network` → likewise repoint to GitHub
  - Line 21: `#   1. Vault:   librefang vault set dashboard_password` → `#   1. Vault:   bossfang vault set dashboard_password`
  - Line 216: `# directory = "~/.librefang/inbox/"` — **keep `~/.librefang/`** (Boundary, default path migration deferred)
  - Line 226-232: `LibreFang exposes an Agent Client Protocol …` — replace product references in comments
  - Line 236: `https://librefang.com/integrations/acp` → upstream-owned domain; either delete the link or repoint to a GitHub equivalent

  **Decisions:**
  - **Default `dashboard_user = "librefang"` and `dashboard_pass = "librefang"`** (lines 24-25) **STAY**. Renaming them is a data migration (existing operators' configs would break). The username/password are arbitrary defaults the user should change anyway.

- [crates/librefang-cli/templates/init_wizard_config.toml](../../crates/librefang-cli/templates/init_wizard_config.toml) — 2 references in header comments

**Estimated PR size**: ~15 lines changed in 2 files.

### M3 — Install script (HIGH)

Top-of-funnel surface — every new user runs this. Convert echo output
and add a `BOSSFANG_INSTALL_DIR` boundary alias.

**File:** [web/public/install.sh](../../web/public/install.sh) — **11 references**:

- Line 2: `# LibreFang installer` → `# BossFang installer`
- Line 3: `# Usage: curl -fsSL https://librefang.ai/install.sh | sh` → repoint to a GQAdonis-owned URL (e.g.
  `https://github.com/GQAdonis/librefang/raw/main/install.sh`). Domain
  `librefang.ai` is **upstream-owned** — never claim it.
- Line 6: `LIBREFANG_INSTALL_DIR custom install directory` — **add `BOSSFANG_INSTALL_DIR` primary, keep `LIBREFANG_INSTALL_DIR` fallback** (Boundary, additive)
- Lines 7-10: `LIBREFANG_VERSION`, `LIBREFANG_AUTO_START`, `LIBREFANG_INSTALLER_SOURCE_ONLY` — these are user-typed env vars in install instructions; alias additively per the M7 decision
- Line 15: `REPO="librefang/librefang"` — flip to `"GQAdonis/librefang"` (the install script is BossFang-owned; pulls binaries from our releases)
- Line 16: `INSTALL_DIR="${LIBREFANG_INSTALL_DIR:-$HOME/.librefang/bin}"` — change to `${BOSSFANG_INSTALL_DIR:-${LIBREFANG_INSTALL_DIR:-$HOME/.librefang/bin}}` (additive alias, default path stays)
- Line 62: `irm https://librefang.ai/install.ps1 | iex` → repoint
- Line 68: `cargo install --git https://github.com/$REPO librefang-cli` — keep `librefang-cli` (Layer Internal — crate name)
- Line 156: `"$INSTALL_DIR/librefang" start` → use `bossfang` (now the primary binary name); keep `librefang` symlink for back-compat
- Line 178: `${C_BOLD}LibreFang Installer${C_RESET}` → `BossFang Installer`
- Line 207: `Installing LibreFang $VERSION for $PLATFORM…` → `Installing BossFang $VERSION…`

**Estimated PR size**: ~20 lines changed in 1 file.

### M4 — Docs site MDX (HIGH, biggest churn)

**120 files** in `docs/src/app/**` reference "LibreFang" in body text.
This is the largest surface remaining but also the most automatable.

**Strategy:**

- **Do not** flip code blocks or inline-code references to crate names
  / module paths / function names — those are Layer Internal.
- **Do** flip prose: product name "LibreFang" → "BossFang", URLs
  pointing at upstream-owned domains (`librefang.ai`, `docs.librefang.ai`)
  to GQAdonis-owned equivalents.

**Implementation options:**

1. **Extend `scripts/enforce-branding.py`** to scan `docs/src/app/**`
   for `.mdx` files. Add a "prose-only" mode that skips fenced code
   blocks (`` ``` ``…`` ``` ``) and inline-code spans (`` `…` ``). The
   script already has the file-discovery + idempotent-replacement
   infrastructure; this is a pure addition.
2. **Dedicated one-shot script** `scripts/rebrand-docs.py` that runs
   once, lands a PR, then is deleted. Lower long-term maintenance
   cost; less safety against future drift.

**Recommendation**: Option 1. The enforce-branding script becomes the
single source of truth for "what `.mdx` content is allowed to say".
Once extended, it runs on every merge audit. Add an MDX-aware ignore
list for legitimate code references (the "what's intentionally
LibreFang-named" list from `three-layer-rebrand.md`).

**Estimated PR size**: ~120 file modifications (mostly single-word
replacements), plus ~80 lines of Python in `enforce-branding.py`.

### M5 — Outbound user-agent strings + registry URLs (MED)

Outbound HTTP traffic carries "LibreFang" branding to third-party
servers (search engines, GitHub, Copilot). These are user-visible in
server-side logs of operators we integrate with.

**Files:**

- [crates/librefang-llm-drivers/src/drivers/copilot.rs:90](../../crates/librefang-llm-drivers/src/drivers/copilot.rs:90) — `User-Agent: LibreFang/1.0` → `BossFang/1.0`
- [crates/librefang-runtime/src/web_search.rs:488](../../crates/librefang-runtime/src/web_search.rs:488), [:577](../../crates/librefang-runtime/src/web_search.rs:577), [:674](../../crates/librefang-runtime/src/web_search.rs:674) — `User-Agent: Mozilla/5.0 (compatible; LibreFangAgent/0.1)` → `BossFangAgent/0.1`

**Plugin registry URLs (deferred)**:

- [crates/librefang-runtime/src/plugin_manager.rs:78](../../crates/librefang-runtime/src/plugin_manager.rs:78), [:95](../../crates/librefang-runtime/src/plugin_manager.rs:95), [:99](../../crates/librefang-runtime/src/plugin_manager.rs:99) — `https://stats.librefang.ai/api/registry/{pubkey,index.json,index.json.sig}`

  These are the **upstream plugin registry trust root**. BossFang does
  not yet host a plugin registry. Three override mechanisms already
  exist for operators who want to flip locally (per
  `.claude/skills/librefang-upstream-merge/references/origin-knobs.md`):
  - `LIBREFANG_REGISTRY_PUBKEY` env var (base64, takes priority)
  - `~/.librefang/registry.pub` TOFU cache
  - `LIBREFANG_REGISTRY_PUBKEY_URL` env var (custom fetch endpoint)

  **Decision**: keep the constants as-is. Re-pointing them without a
  hosted BossFang trust root would break plugin installs for every
  user. Document the deferred status in `origin-knobs.md` (already
  done). When a BossFang registry exists, add `BOSSFANG_REGISTRY_*`
  aliases as part of M7.

**Estimated PR size**: 4 lines changed in 2 files.

### M6 — Dashboard localStorage key migration (MED, user-state-touching)

**File:** [crates/librefang-api/dashboard/src/api.ts](../../crates/librefang-api/dashboard/src/api.ts) — **6 references** to `"librefang-api-key"`:

- Lines 1019-1020: get from sessionStorage / localStorage
- Lines 3339, 3340: set sessionStorage, remove old localStorage (key rotation)
- Lines 3347-3348: clear both on logout

**Strategy (locked pre-decision): read-old-write-new shim.**

Add a helper module `dashboard/src/lib/apiKey.ts`:

```typescript
const PRIMARY = "bossfang-api-key";
const LEGACY = "librefang-api-key";

export function getApiKey(): string | null {
  const fromPrimary =
    sessionStorage.getItem(PRIMARY) ?? localStorage.getItem(PRIMARY);
  if (fromPrimary) return fromPrimary;

  const fromLegacy =
    sessionStorage.getItem(LEGACY) ?? localStorage.getItem(LEGACY);
  if (fromLegacy) {
    // Auto-migrate: copy legacy → primary, then drop legacy.
    sessionStorage.setItem(PRIMARY, fromLegacy);
    sessionStorage.removeItem(LEGACY);
    localStorage.removeItem(LEGACY);
    return fromLegacy;
  }
  return null;
}

export function setApiKey(key: string): void {
  sessionStorage.setItem(PRIMARY, key);
  localStorage.removeItem(PRIMARY);
  // Clear any stale legacy entries the user might still have:
  sessionStorage.removeItem(LEGACY);
  localStorage.removeItem(LEGACY);
}

export function clearApiKey(): void {
  sessionStorage.removeItem(PRIMARY);
  localStorage.removeItem(PRIMARY);
  sessionStorage.removeItem(LEGACY);
  localStorage.removeItem(LEGACY);
}
```

Replace the 6 inline call sites in `api.ts` with calls into the helper.
Drop the LEGACY fallback in ~2 release cycles after metrics confirm
near-zero legacy hits.

**Estimated PR size**: 1 new file (~40 lines), 6 single-line
replacements in `api.ts`.

### M7 — Additional operator env-var aliases (LOW)

Boundary layer. Aliases follow the same pattern as `BOSSFANG_HOME` /
`BOSSFANG_VAULT_KEY` already landed in PR #6: `BOSSFANG_*` reads first,
falls back to `LIBREFANG_*`, both kept working forever.

**Scope (locked pre-decision)**: only operator-facing variables that
appear in user-typed contexts (docs, error messages, install scripts).
The ~60 deep plumbing vars (`LIBREFANG_COHERE_INPUT_TYPE`,
`LIBREFANG_PLUGIN_CONFIG`, `LIBREFANG_TEST_*`, etc.) stay unaliased —
no operator ever types them.

**New aliases:**

| Primary (new) | Fallback (legacy) | Resolution helper | File |
|---|---|---|---|
| `BOSSFANG_INSTALL_DIR` | `LIBREFANG_INSTALL_DIR` | install.sh inline | `web/public/install.sh` (M3 covers this) |
| `BOSSFANG_REGISTRY_PUBKEY` | `LIBREFANG_REGISTRY_PUBKEY` | `pubkey_from_env()` | `crates/librefang-runtime/src/plugin_manager.rs` |
| `BOSSFANG_REGISTRY_PUBKEY_URL` | `LIBREFANG_REGISTRY_PUBKEY_URL` | same | same |
| `BOSSFANG_DASHBOARD_EMBEDDED_ONLY` | `LIBREFANG_DASHBOARD_EMBEDDED_ONLY` | `dashboard_mode_from_env()` | `crates/librefang-api/src/webchat.rs` |

Each is a 3-5 line additive change in the matching helper. Pattern:

```rust
const BOSSFANG_KEY: &str = "BOSSFANG_REGISTRY_PUBKEY";
const LEGACY_KEY: &str = "LIBREFANG_REGISTRY_PUBKEY";

fn pubkey_from_env() -> Option<String> {
    std::env::var(BOSSFANG_KEY).ok()
        .or_else(|| std::env::var(LEGACY_KEY).ok())
}
```

**Estimated PR size**: ~40 lines across 3-4 files. Bundle with M5 since
both touch the same `plugin_manager.rs` neighbourhood.

## 4. Layer Internal — MUST NOT rename

The following symbols are Layer Internal. Renaming any of them
guarantees conflicts on every future upstream merge. Future PRs MUST
NOT touch these even when the surrounding file is converted.

**Cargo crate names** (workspace members):
- `librefang-types`, `librefang-http`, `librefang-wire`, `librefang-telemetry`,
  `librefang-testing`, `librefang-migrate`
- `librefang-kernel`, `librefang-kernel-handle`, `librefang-kernel-router`,
  `librefang-kernel-metering`
- `librefang-runtime`, `librefang-runtime-mcp`, `librefang-runtime-oauth`,
  `librefang-runtime-wasm`
- `librefang-llm-driver`, `librefang-llm-drivers`
- `librefang-memory`, `librefang-memory-wiki`
- `librefang-storage` (BossFang-exclusive)
- `librefang-api`, `librefang-cli`, `librefang-desktop`, `librefang-acp`
- `librefang-skills`, `librefang-hands`, `librefang-extensions`, `librefang-channels`
- `librefang-uar-spec` (BossFang-exclusive)

**Rust module paths**:
- `use librefang_runtime::...`, `use librefang_kernel::...`, etc.

**Function names**:
- `librefang_home()` in `librefang-kernel/src/config.rs`
- `librefang_dirs()`, any `librefang_*` helper

**Type names**:
- `LibreFangKernel`, `LibreFangError`, `LibreFangConfig`, `LibreFangRuntime`

**Python SDK module names** (PyPI package is `bossfang-sdk`, import path stays):
- `librefang_sdk`, `librefang_client`

**Cargo workspace member paths**:
- `[workspace] members = [...]` entries in `Cargo.toml`

**Test fixture identifiers**:
- Anything in `crates/librefang-api/tests/fixtures/` that matches
  upstream — fixture names diverge only at the schema level, not the
  filename level (e.g., `kernel_config_schema.golden.json` keeps its
  name even though the contents diverged).

**Internal binary `[[bin]]` `path`**:
- The `path = "src/main.rs"` in `librefang-cli/Cargo.toml` is fixed;
  only the `name` is overridable. We use this to alias `bossfang` and
  `librefang` to the same binary source.

## 5. Out-of-scope decisions with rationale

These items came up in the audit but are **explicitly deferred** with
the reasons documented so future agents don't re-litigate.

| Item | Decision | Why |
|---|---|---|
| Default home dir `~/.librefang/` | **Stays** | Renaming orphans every existing user's config / vault / registry cache. Power users on fresh install can `export BOSSFANG_HOME=~/.bossfang/`. |
| Default `dashboard_user = "librefang"` / `dashboard_pass = "librefang"` in `init_default_config.toml` | **Stays** | They're arbitrary defaults the user is expected to change. Renaming breaks existing operators' configs. |
| `noreply@librefang.ai` git committer in `workspace_setup.rs:228` | **Stays** | We don't own `bossfang.ai`. Using it creates dangling-email refs in workspace git histories that can't be verified. Better an honest upstream-owned address than a fake BossFang one. |
| `stats.librefang.ai` plugin registry URLs in `plugin_manager.rs` | **Stays** | BossFang has no plugin trust root yet. Re-pointing without a hosted endpoint breaks plugin installs. Three operator overrides exist for self-hosting users. Aliases land when a BossFang registry ships. |
| `github.com/librefang/librefang-registry` fallback constants in `registry_sync.rs` | **Stays** | They're the empty-`base_url` rollback path. The default `[registry] base_url` already routes to `GQAdonis/librefang-registry`; the constants only fire when an operator explicitly empties the field. |
| ~60 deep plumbing `LIBREFANG_*` env vars | **No aliases** | Devs-only vars that no operator types. Aliasing each adds merge surface for marginal benefit. |
| Internal crate-name mentions in README "crate map" section | **Stays** | The crate IS `librefang-cli`. Renaming explodes Layer Internal. README "crate map" is technical-reference content. |
| Historical attribution `"BossFang is a community fork of librefang/librefang"` in README | **Stays** | This is the *intended* meaning — we are a fork. |
| Cargo workspace `[workspace.package] repository / homepage` | **Already done** — points to `github.com/GQAdonis/librefang` | — |

## 6. Recommended PR sequencing

Each milestone lands as a separate PR. Sequencing balances reviewability
against merge cost.

1. **M1 + M2 bundle** — CLI banner + desktop install path strings + init
   config templates. Tightly coupled (both Surface, both pure rename
   except the install-path migration shim). ~50 lines across 4 files.

2. **M3 standalone** — Install script. Top-of-funnel surface; deserves
   its own focused review (security/network sensitivity, additive env
   aliases). ~20 lines, 1 file.

3. **M4 standalone** — Docs MDX sweep with extended
   `enforce-branding.py`. The largest diff (~120 files) but mostly
   single-word replacements. Standalone because the diff dominates the
   PR view and reviewers can't usefully diff it line-by-line.

4. **M5 + M7 bundle** — UA strings (4 lines) + env-var aliases (~40
   lines). Both touch the runtime crate's `plugin_manager.rs` area.

5. **M6 standalone** — Dashboard localStorage migration. User-state-
   touching with a back-compat shim; deserves explicit review of the
   read-old-write-new semantics and metrics for the legacy-fallback
   sunset.

Total: **5 PRs**, ~250 lines of Rust/TS/Python changes plus ~120
single-word `.mdx` edits.

## 7. Merge-cost forecast

How each milestone affects the per-upstream-merge conflict surface:

| Milestone | New conflict surface | Notes |
|---|---|---|
| M1 (CLI + install paths) | **Low** | Surface strings rarely overlap with upstream features. `ui.rs:50` is a constant; `desktop_install.rs` filenames could conflict if upstream adds new platforms, but the shim pattern means we always-take-ours and merge logic by hand. |
| M2 (config templates) | **Low** | Templates are leaf files; upstream usually adds new comments rather than rewriting old ones. Take-ours on conflict. |
| M3 (install script) | **Low** | Upstream rarely edits `web/public/install.sh`; when it does, the changes are mostly to `REPO=` and version pins (which we already overlay). |
| M4 (docs MDX) | **Medium** | Upstream actively writes docs. Take-ours-then-rerun-enforce-branding pattern; the script automation keeps the conflict surface bounded. Expect ~5-10 minutes per merge to re-run the prose flip on new upstream MDX content. |
| M5 (UA strings) | **Negligible** | Three call sites total. |
| M6 (localStorage shim) | **Negligible** | Helper module is BossFang-only; api.ts call sites are stable. |
| M7 (env aliases) | **Low** | Same pattern as M5; helpers are local and rarely conflict. |

**Net forecast**: completing M1-M7 adds **~10 minutes** to a typical
upstream merge audit (mostly the MDX re-run from M4). Without the
script automation, M4 alone would add 30+ minutes. The
enforce-branding extension in M4 is the load-bearing investment.

## 8. Verification for each PR

Every M-PR ships with:

1. `cargo check --workspace --lib` (or `pnpm tsc` / `mdx-validate` for
   non-Rust changes) — must pass
2. `python3 scripts/enforce-branding.py --check` — must exit 0
3. Scoped tests for the changed crate (e.g., `cargo test -p
   librefang-cli` for M1/M2/M3, `cargo test -p librefang-api` for M6)
4. Manual smoke test: run the affected user flow (init wizard for M2,
   install script in a fresh VM for M3, dashboard login for M6) and
   screenshot the user-visible output

The merge-PR's audit pipeline already runs (1) and (2); the rest is
per-milestone reviewer due diligence.

## 9. Out-of-band: the upstream PR closure

A separate housekeeping item discovered during the assessment: when
the upstream-merge PR was opened, `gh pr create` defaulted to the
fork-parent base, creating PR `librefang/librefang#5050` on **upstream**
rather than on `GQAdonis/librefang`. The correct PR
(`GQAdonis/librefang#8`) was reopened immediately, but the upstream PR
remains open and visible to upstream maintainers. **User action needed**:
close `librefang/librefang#5050` with a brief comment that the change
is fork-internal sync and not intended for upstream merge. (The auto-mode
classifier correctly blocked the agent from closing a PR on a repo
outside the user's fork.)

For future merges, use:

```bash
gh pr create --repo GQAdonis/librefang --base main --head <branch> ...
```

The `--repo` flag is the load-bearing fix.

## 10. References

- Three-layer principle: `.claude/skills/librefang-upstream-merge/references/three-layer-rebrand.md`
- Conflict-resolution table: `.claude/skills/librefang-upstream-merge/references/conflict-resolution.md`
- Origin-repoint knobs: `.claude/skills/librefang-upstream-merge/references/origin-knobs.md`
- Tauri desktop checklist: `.claude/skills/librefang-upstream-merge/references/tauri-desktop-checklist.md`
- Branding tokens: `.claude/skills/librefang-upstream-merge/references/branding-tokens.md`
- Brand enforcement script: `scripts/enforce-branding.py`
- Project agent instructions: [CLAUDE.md](../../CLAUDE.md) (BossFang Branding section)
