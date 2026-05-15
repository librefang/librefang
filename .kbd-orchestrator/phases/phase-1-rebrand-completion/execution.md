# Phase 1 execution log — rebrand completion

## Backend selection

| Backend | Selected | Rationale |
|---|---|---|
| `openspec` | No | No OpenSpec scaffolding in this project. |
| `native-tool` | **Yes** | Single-tool (Claude Code) execution. Surface flips are mechanical; progress.json + commit history sufficient for traceability. |
| `hybrid` | No | Not warranted at this scale. |
| `manual` | No | Mechanical work — automation appropriate. |

## Dispatch contract

- **Tool**: Claude Code (this session)
- **Worktree base**: `/tmp/librefang-m1-m2`
- **Branch**: `feat/m1-m2-rebrand-cli-templates` (from `origin/main`)
- **Target PR base**: `main` (depends on PR #9 landing for the source roadmap doc; the M1+M2 implementation itself does not import from the doc, so this PR can land independently if needed)

## Change m1-m2-cli-banner-install-paths-config-templates

### M1 actions

1. `crates/librefang-cli/src/ui.rs:50` — replace `"LibreFang Agent OS"` → `"BossFang Agent OS"` and the docstring on line 45.
2. `crates/librefang-cli/src/desktop_install.rs` — introduce read-old/write-new shim:
   - `legacy_app_path()`, `bossfang_app_path()` helpers
   - `detect_installed_app()` checks BossFang first, falls back to LibreFang
   - `migrate_legacy_install()` renames `/Applications/LibreFang.app` → `/Applications/BossFang.app` on first BossFang-side install (macOS), equivalent for Linux AppImage and Windows NSIS dirs.
   - All user-facing strings flip `LibreFang Desktop` → `BossFang Desktop`.
3. Lines 117, 191 — `User-Agent: librefang-cli` → `bossfang-cli`.

### M2 actions

1. `crates/librefang-cli/templates/init_default_config.toml`:
   - Lines 2, 226-228, 232 — comment flips `LibreFang` → `BossFang`
   - Lines 3, 12, 236 — repoint upstream-owned doc URLs to `github.com/GQAdonis/librefang`
   - Line 21 — example command `librefang vault set …` → `bossfang vault set …`
   - **KEEP** lines 24-25 (`dashboard_user = "librefang"`, `dashboard_pass = "librefang"`) — default-value migration policy applies.
   - **KEEP** line 216 (`~/.librefang/inbox/`) — default path policy applies.
2. `crates/librefang-cli/templates/init_wizard_config.toml`:
   - Line 1 — comment flip
   - Line 2 — repoint upstream URL to `github.com/GQAdonis/librefang`

## Verification commands

```bash
cd /tmp/librefang-m1-m2

# 1. Compile the changed crate
cargo check -p librefang-cli

# 2. Scoped tests (any test asserting on the changed strings)
cargo test -p librefang-cli --lib --bins

# 3. Brand enforcement audit
python3 scripts/enforce-branding.py --check

# 4. Manual smoke (can't execute in agent context — surface to user):
#    target/release/bossfang --version    # should show banner
#    target/release/bossfang init         # should show BossFang in templates
```

## Artifact-refiner QA

This change has fewer than 3 distinct artifacts (1 .rs source change in
3 places + 2 .toml templates). The `kbd-execute` skill says:

> When to skip QA: Change has fewer than 3 files modified

This change touches 4 files; QA gate applies in principle but no
`.refiner/` infrastructure is set up in this project. Document the
constraint coverage inline:

| Constraint (constraints.md) | Verified |
|---|---|
| Three-layer principle (Surface only) | All changes are Surface-layer strings; no Internal symbol touched |
| Default-value migration policy | `dashboard_user`/`dashboard_pass` defaults preserved; `~/.librefang/` paths preserved |
| Domain ownership | `librefang.ai` / `librefang.com` URLs repointed to `github.com/GQAdonis/librefang`; `bossfang.ai` NOT used |
| File-on-disk migration policy | M1 install path migration uses read-old/write-new shim with auto-rename |
| Layer Internal preservation | `librefang-cli` crate name unchanged; `librefang_*` Rust symbols unchanged; `cargo install --git ... librefang-cli` line unchanged |
