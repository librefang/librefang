---
name: librefang-upstream-merge
description: Merge upstream librefang/librefang into the GQAdonis/librefang BossFang fork without losing branding, SurrealDB, surreal-memory, or UAR. Use when pulling new commits from upstream, consuming upstream's Tauri desktop code, auditing the fork after a merge, or anytime the user says "pull upstream", "merge upstream", "sync with librefang upstream", "rebase against librefang", or "upstream merge audit". Covers (a) brand preservation (ember palette, boss-libre.png logo, BossFang productName/identifier), (b) SurrealDB-as-default storage (any new upstream SQLite schema must get a matching .surql migration), (c) origin repointing (registry, dashboard tarball, marketplace org, Tauri updater all flow from GQAdonis-owned URLs), (d) Tauri desktop rebrand (BossFang icons, BossFang minisign pubkey, ai.bossfang.* bundle identifier).
license: Proprietary. Internal BossFang tooling.
compatibility: Requires git, python3, jq, gh; designed for Claude Code running inside a librefang git worktree (NOT the main tree).
metadata:
  author: GQAdonis
  version: "1.0"
  fork-of: librefang/librefang
  fork-name: BossFang
---

# librefang-upstream-merge

Merge `upstream/main` into the BossFang fork while preserving the four
BossFang-exclusive surfaces (branding, SurrealDB storage, surreal-memory
substrate, UAR provider) and the origin-repoint plumbing that keeps
fresh installs talking to GQAdonis-owned URLs by default.

## When to invoke

Trigger on any of:

- "pull upstream", "merge upstream", "sync with librefang upstream", "rebase against librefang"
- "upstream merge", "merge from librefang"
- "audit the fork after merging", "is the fork still ours?"
- "consume upstream's Tauri code", "pick up upstream desktop changes"

Skip for:

- A regular feature branch merge (this is upstream-specific)
- Fixing a single brand token (use `scripts/enforce-branding.py` directly)
- A SurrealDB migration unrelated to an upstream merge

## The three-layer rebrand principle (load first)

Every BossFang change classifies into one of three layers. The merge MUST
respect these — getting it wrong is what makes upstream merges painful.

| Layer | Examples | Merge rule |
|---|---|---|
| **Internal** (532+ files) | crate names `librefang-*`, module paths `librefang_runtime::`, struct names `LibreFangKernel`, function names like `librefang_home()`, Python module names `librefang_sdk` | **NEVER touch**. Renaming explodes conflict surface on every merge. Even when upstream renames an internal symbol, take their version. |
| **Boundary** | env vars (`BOSSFANG_HOME` aliasing `LIBREFANG_HOME`), config keys, on-disk paths (`~/.librefang/`), test-fixture seeds | **Additive aliases only**. Never delete the LibreFang version. Default home dir stays `~/.librefang/`. |
| **Surface** | product name "BossFang", binary names (`bossfang` + `librefang`), npm `@bossfang/sdk`, README/docs, CLI help, UA strings, Tauri identifier, content-type vendor prefix | **Full rename OK**. Always-take-ours during conflict resolution. |

For deep detail on each layer's call sites and the full rebrand surface
manifest, see [references/three-layer-rebrand.md](references/three-layer-rebrand.md).

## Workflow

Run the four phases in order. Each is independently reportable — pause
after each and surface findings to the user before continuing.

### Phase 1 — Preflight

Goal: confirm safe operating conditions before touching anything.

```bash
scripts/preflight.sh
```

This script checks (and fails with a clear message if any fail):

1. Working directory is a **linked worktree**, not the main tree
   (`forbid-main-worktree.sh` would block edits otherwise).
2. Working tree is clean (`git status --porcelain` is empty).
3. `upstream` remote is configured and points at `librefang/librefang`.
4. `git fetch upstream --no-tags` succeeds.
5. Reports the commit-count divergence (`git log HEAD..upstream/main --oneline | wc -l`).

If the report shows zero commits behind: nothing to do. Stop here.

### Phase 2 — Merge

Goal: get upstream's commits onto the fork branch with minimal conflict surface.

```bash
git merge upstream/main --no-edit
```

Most merges land clean because BossFang's surface-layer changes don't
overlap upstream's hot edit zones. When conflicts arise, resolve per the
authoritative table at [references/conflict-resolution.md](references/conflict-resolution.md).
The short version, in priority order:

1. **Always-take-ours** (binary diffs, BossFang identity strings):
   - `crates/librefang-desktop/tauri.conf.json` — `productName: "BossFang"`, `identifier: "ai.bossfang.desktop"`, BossFang descriptions
   - `crates/librefang-desktop/tauri.desktop.conf.json` — `identifier: "ai.bossfang.desktop"`, updater endpoint pinned at `github.com/GQAdonis/librefang/releases/...`
   - `crates/librefang-desktop/tauri.{ios,android}.conf.json` — `identifier: "ai.bossfang.app"`
   - `crates/librefang-desktop/icons/*` — never overwrite from upstream (binary files; regenerate locally from `docs/branding/boss-libre.png` if needed)
   - `crates/librefang-api/dashboard/src/index.css` `:root` / `:root.dark` blocks (ember palette)
   - `crates/librefang-api/dashboard/src/App.tsx` sidebar + mobile header brand blocks (logo must be `boss-libre.png`)
   - `crates/librefang-api/dashboard/public/manifest.json` name / short_name
   - `crates/librefang-api/dashboard/index.html` title / meta tags

2. **Take-upstream-then-perl-rewrite** (new TSX with inline sky-blue):
   - Components with `style={{ background: "linear-gradient(135deg,#38bdf8,...)" }}` or similar — take upstream content, then run `scripts/enforce-branding.py` to flip tokens.

3. **Take-upstream-as-is** (everything else): runtime/kernel/API code, dashboard logic, workflow files that don't touch BossFang surfaces.

After conflict resolution: `git merge --continue` (or commit the resolved
state). Don't `git checkout --` an apparent extra file (like a regenerated
`Cargo.lock`) — per CLAUDE.md, post-merge artefacts count as real work.

### Phase 3 — Audit (parallel checks)

Goal: catch the four classes of regression upstream merges introduce.
Run all four scripts; report their findings together at the end.

```bash
scripts/scan-new-schema.sh          # 3a — new upstream SQLite tables/columns
scripts/scan-hardcoded-urls.sh      # 3b — new librefang.ai / librefang/* URLs
scripts/audit-tauri-desktop.sh      # 3c — Tauri identifier / pubkey / icon drift
scripts/run-branding-enforce.sh     # 3d — ember token enforcement + audit
```

**3a — SurrealDB schema parity** (`scripts/scan-new-schema.sh`):

Greps the merge diff for `CREATE TABLE`, `ALTER TABLE`, `ADD COLUMN`,
`CREATE INDEX`. For each hit, you must add a matching SurrealDB migration
at `crates/librefang-storage/src/migrations/sql/NNN_<name>.surql` and
register it in `crates/librefang-storage/src/migrations/mod.rs`. The
runner enforces SHA256 drift detection — don't edit applied migrations
in place. Detailed playbook: [references/surrealdb-migrations.md](references/surrealdb-migrations.md).

**3b — Origin URL drift** (`scripts/scan-hardcoded-urls.sh`):

Greps the merge diff for any new `librefang.ai` / `github.com/librefang` /
`@librefang/` / `librefang-skills` reference under `crates/**/*.rs` and
the workflow files. Three knobs already exist for this — see
[references/origin-knobs.md](references/origin-knobs.md):

- `[registry] base_url` → default `https://github.com/GQAdonis/librefang-registry` (`crates/librefang-types/src/config/types.rs`, plumbed through `crates/librefang-runtime/src/registry_sync.rs`)
- Dashboard tarball URL → hardcoded at `crates/librefang-api/src/webchat.rs:378` (BossFang fork defaults to embedded-only mode so this is dead-code in the default path)
- `[skills.marketplace] github_org` → default `"GQAdonis"` (`crates/librefang-skills/src/marketplace.rs`)
- Tauri updater endpoint → `crates/librefang-desktop/tauri.desktop.conf.json:8`

Any new hardcoded URL upstream adds must either flow through the
matching knob or be hardcoded to the GQAdonis equivalent.

**3c — Tauri desktop drift** (`scripts/audit-tauri-desktop.sh`):

Checks the four Tauri config files after merge:

- `productName == "BossFang"` (not `"LibreFang"`)
- `identifier ∈ {ai.bossfang.desktop, ai.bossfang.app}` (not `ai.librefang.*`)
- Updater endpoint host is `github.com/GQAdonis/librefang` (not `github.com/librefang`)
- Tauri minisign pubkey key ID is `E329A6B2863F1707` (the BossFang key, not upstream's `BC91908BD3F1520D`)

For new upstream desktop crates / features, take the upstream code as-is
(it's the runtime / commands / IPC) but keep our branding overlays.
Detailed playbook: [references/tauri-desktop-checklist.md](references/tauri-desktop-checklist.md).

**3d — Brand enforcement** (`scripts/run-branding-enforce.sh`):

Runs `python3 scripts/enforce-branding.py` (idempotent token-replacement)
then `python3 scripts/enforce-branding.py --check` (read-only audit).
Anything `--check` flags after the replacement pass is a manual case
(typically: a new SVG fang glyph inside a gradient box — replace with
`<img src="/boss-libre.png" alt="BossFang" ...>`).

The enforcement script covers `crates/librefang-api/dashboard/src/`,
`crates/librefang-api/static/`, `crates/librefang-desktop/frontend/`,
and `crates/librefang-desktop/src/`. The branding source of truth is
`docs/branding/branding-guide.html`; the ember palette tokens are at
[references/branding-tokens.md](references/branding-tokens.md).

### Phase 4 — Verify and commit

Goal: confirm the fork still builds and the BossFang-exclusive crates
still pass before committing the merge.

```bash
# Compile-check everything (lib only — full builds are forbidden in worktree)
cargo check --workspace --lib

# BossFang exclusives in isolation
cargo check -p librefang-storage -p librefang-uar-spec -p librefang-memory

# Scoped tests for any new upstream feature the merge brought in
# (e.g. if upstream added web_fetch_to_file, run its test)
cargo test -p librefang-runtime --test <new-test>

# Brand enforcement audit (must exit 0)
python3 scripts/enforce-branding.py --check
```

If any step fails:

- `cargo check` failure → fix the breaking change in the merge commit (don't defer)
- `enforce-branding --check` failure → either the `enforce-branding.py` script needs new patterns (commit those in the same merge) or there's a new SVG/raster fang glyph to swap manually
- A BossFang-exclusive crate failure → likely upstream changed a trait signature; update our impl in the same commit, never bypass

Commit message convention (from CLAUDE.md):

```
chore(merge): upstream YYYY-MM-DD (N commits) + BossFang preservation
```

Body must enumerate:

- Number of upstream commits merged
- Any `.surql` migration added (file path + summary)
- Any branding fix needed
- Any deferred follow-ups (with file:line evidence)

## Reference index

- [references/three-layer-rebrand.md](references/three-layer-rebrand.md) — Internal/Boundary/Surface principle, full rebrand surface manifest
- [references/conflict-resolution.md](references/conflict-resolution.md) — always-take-ours table
- [references/surrealdb-migrations.md](references/surrealdb-migrations.md) — how to add a `.surql` migration when upstream adds SQLite schema
- [references/origin-knobs.md](references/origin-knobs.md) — the four origin-repoint knobs and where they live
- [references/tauri-desktop-checklist.md](references/tauri-desktop-checklist.md) — Tauri identifier / pubkey / icon audit
- [references/branding-tokens.md](references/branding-tokens.md) — the ember palette token table

## Scripts

- [scripts/preflight.sh](scripts/preflight.sh) — Phase 1
- [scripts/scan-new-schema.sh](scripts/scan-new-schema.sh) — Phase 3a
- [scripts/scan-hardcoded-urls.sh](scripts/scan-hardcoded-urls.sh) — Phase 3b
- [scripts/audit-tauri-desktop.sh](scripts/audit-tauri-desktop.sh) — Phase 3c
- [scripts/run-branding-enforce.sh](scripts/run-branding-enforce.sh) — Phase 3d

## Non-goals

This skill does NOT:

- Rebase the fork onto a single upstream tag (this is `git merge`, not `git rebase`)
- Decide whether to upstream a BossFang change back to `librefang/librefang` (separate workflow)
- Generate signing keypairs (those are Phase-2 PRs, done once)
- Build, run, or sign desktop bundles (those need real `cargo build` and `tauri build` runs outside the worktree-edit context)

## Common gotchas

- The `forbid-main-worktree.sh` hook hard-blocks `cargo build` from anywhere in the repo. Phase 4 uses `cargo check` instead; CI does the full build.
- `librefang start` / `target/*/librefang start` are hook-blocked too — never auto-launch the daemon for verification. Surface the curl smoke commands for the human to run.
- BossFang's repo has `core.hooksPath` typically unset on fresh clones — version-controlled hooks in `scripts/hooks/` only activate after `just setup`. If a commit-message reject seems wrong, run `git config core.hooksPath` and confirm.
- The kernel-config golden fixture at `crates/librefang-api/tests/fixtures/kernel_config_schema.golden.json` regenerates from `KernelConfig`'s schema. When `RegistryConfig` (or any other config struct) changes shape in an upstream merge, regenerate with `cargo test -p librefang-api --test config_schema_golden -- --ignored regenerate_golden --nocapture` and commit the new bytes.
- UAR's `build.rs` runs `npm ci` on a vendored frontend. The BossFang fork uses `GQAdonis/universal-agent-runtime` (forked from `Prometheus-AGS/universal-agent-runtime`) with `frontend/package-lock.json` + `frontend/.npmrc` committed. If upstream switches UAR to a newer ref, verify the fork still has the lockfile or repoint to a refreshed fork.
- pnpm version is pinned at 10.33.0 via Corepack. Fresh clones: `corepack enable && corepack prepare pnpm@10.33.0 --activate`. Without this, `librefang-api/build.rs` fails on a version-mismatch error.
