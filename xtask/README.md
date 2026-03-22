# xtask — LibreFang Build Automation

Cross-platform build automation for the LibreFang workspace, replacing scattered shell scripts with a single Rust CLI.

## Quick Start

```bash
cargo xtask <command> [options]
```

## Commands

### `release` — Full Release Flow

Runs changelog generation, version sync, dashboard build, commit, tag, and creates a PR.

```bash
cargo xtask release                                      # interactive (prompts for stable/beta/rc)
cargo xtask release --version 2026.3.2214                # explicit version
cargo xtask release --version 2026.3.2214-beta1          # pre-release
cargo xtask release --version 2026.3.2214 --no-confirm   # non-interactive (CI)
cargo xtask release --no-push                            # local only, skip push + PR
cargo xtask release --no-article                         # skip Dev.to article
```

Requires: `main` branch, clean worktree, `gh` CLI for PR creation.

### `ci` — Local CI Suite

Runs the same checks as CI, locally.

```bash
cargo xtask ci                  # full suite: build + test + clippy + web lint
cargo xtask ci --no-test        # skip tests
cargo xtask ci --no-web         # skip web lint
cargo xtask ci --release        # use release profile
cargo xtask ci --no-test --no-web  # build + clippy only (fastest)
```

Steps (in order, fail-fast):
1. `cargo build --workspace --lib`
2. `cargo test --workspace`
3. `cargo clippy --workspace --all-targets -- -D warnings`
4. `pnpm run lint` in `web/` (if exists)

### `build-web` — Frontend Builds

Build one or all frontend targets via pnpm.

```bash
cargo xtask build-web               # all: dashboard + web + docs
cargo xtask build-web --dashboard   # React dashboard only
cargo xtask build-web --web         # web/ frontend only
cargo xtask build-web --docs        # docs/ site only
```

Targets:
- `crates/librefang-api/dashboard/` — React dashboard
- `web/` — Vite + React frontend
- `docs/` — Next.js docs site

Skips any target that doesn't have a `package.json`.

### `changelog` — Generate CHANGELOG

Generate a CHANGELOG.md entry from merged PRs since the last tag.

```bash
cargo xtask changelog 2026.3.22                    # since latest stable tag
cargo xtask changelog 2026.3.22 v2026.3.2114       # since specific tag
```

PRs are classified by conventional commit prefix:
- `feat:` → Added
- `fix:` → Fixed
- `refactor:` → Changed
- `perf:` → Performance
- `docs:` → Documentation
- `chore:/ci:/build:/test:` → Maintenance

Requires: `gh` CLI.

### `sync-versions` — Version Sync

Sync CalVer version strings across all packages.

```bash
cargo xtask sync-versions                   # sync all files to current Cargo.toml version
cargo xtask sync-versions 2026.3.2214       # bump everything to new version
cargo xtask sync-versions 2026.3.2214-rc1   # pre-release version
```

Updates:
- `Cargo.toml` workspace version
- `sdk/javascript/package.json`
- `sdk/python/setup.py` (PEP 440: `-beta1` → `b1`)
- `sdk/rust/Cargo.toml` + `README.md`
- `packages/whatsapp-gateway/package.json`
- `crates/librefang-desktop/tauri.conf.json` (MSI-compatible encoding)

### `integration-test` — Live Integration Tests

Start the daemon, hit API endpoints, optionally test LLM, then clean up.

```bash
cargo xtask integration-test --skip-llm                     # basic endpoint tests
cargo xtask integration-test --api-key $GROQ_API_KEY        # full test with LLM
cargo xtask integration-test --port 5000                    # custom port
cargo xtask integration-test --binary target/debug/librefang  # custom binary path
```

Tests:
1. `GET /api/health`
2. `GET /api/agents`
3. `GET /api/budget`
4. `GET /api/network/status`
5. `POST /api/agents/{id}/message` (unless `--skip-llm`)
6. Verify budget updated after LLM call

Default binary: `target/release/librefang`. Build it first with `cargo build --release -p librefang-cli`.

## What This Replaces

| xtask command | Replaced |
|---------------|----------|
| `release` | `scripts/release.sh` (removed) |
| `sync-versions` | `scripts/sync-versions.sh` (removed) |
| `changelog` | `scripts/generate-changelog.sh` (removed) |
| `ci` | manual 3-step workflow |
| `build-web` | manual pnpm commands |
| `integration-test` | manual 8-step curl workflow |
