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

### `publish-sdks` — Publish SDKs

Publish JavaScript, Python, and Rust SDKs to their respective registries.

```bash
cargo xtask publish-sdks                # publish all SDKs
cargo xtask publish-sdks --js           # npm only
cargo xtask publish-sdks --python       # PyPI only
cargo xtask publish-sdks --rust         # crates.io only
cargo xtask publish-sdks --dry-run      # validate without publishing
```

Requires: `npm`, `twine` (Python), `cargo` credentials configured.

### `dist` — Build Distribution Binaries

Cross-compile release binaries for multiple platforms.

```bash
cargo xtask dist                                          # all default targets
cargo xtask dist --target x86_64-unknown-linux-gnu        # specific target
cargo xtask dist --cross                                  # use cross for cross-compilation
cargo xtask dist --output release-artifacts               # custom output dir
```

Default targets: linux (x86_64, aarch64), macOS (x86_64, aarch64), Windows (x86_64).
Archives: `.tar.gz` for linux/macOS, `.zip` for Windows.

### `docker` — Docker Image

Build and optionally push the Docker image.

```bash
cargo xtask docker                          # build with version tag
cargo xtask docker --push                   # build + push to GHCR
cargo xtask docker --tag 2026.3.2214        # explicit tag
cargo xtask docker --latest --push          # also tag as :latest
cargo xtask docker --platform linux/arm64   # specific platform
```

Image: `ghcr.io/librefang/librefang`. Dockerfile: `deploy/Dockerfile`.

### `setup` — Dev Environment Setup

First-time setup for new contributors.

```bash
cargo xtask setup              # full setup
cargo xtask setup --no-web     # skip frontend dependencies
cargo xtask setup --no-fetch   # skip cargo fetch
```

Checks: cargo, rustup, pnpm, gh, docker, just.
Actions: installs git hooks, fetches Rust deps, runs pnpm install, creates default config.

### `coverage` — Test Coverage

Generate test coverage reports using `cargo-llvm-cov`.

```bash
cargo xtask coverage                   # HTML report
cargo xtask coverage --open            # HTML + open in browser
cargo xtask coverage --lcov            # lcov format (for CI)
cargo xtask coverage --output my-cov   # custom output dir
```

Auto-installs `cargo-llvm-cov` if not present.

### `deps` — Dependency Audit

Audit dependencies for security vulnerabilities and outdated packages.

```bash
cargo xtask deps                   # audit + outdated + web
cargo xtask deps --audit           # cargo audit only
cargo xtask deps --outdated        # cargo outdated only
cargo xtask deps --web             # pnpm audit only
```

Auto-installs `cargo-audit` and `cargo-outdated` if not present.

### `codegen` — Code Generation

Run code generators (OpenAPI spec, etc.).

```bash
cargo xtask codegen                # all generators
cargo xtask codegen --openapi      # OpenAPI spec only
```

Regenerates `openapi.json` from utoipa annotations by running the spec test.

### `check-links` — Link Checker

Check for broken links in documentation.

```bash
cargo xtask check-links                          # full check with lychee
cargo xtask check-links --basic                  # built-in basic checker
cargo xtask check-links --path docs              # specific directory
cargo xtask check-links --exclude "example.com"  # exclude patterns
```

Uses [lychee](https://github.com/lycheeverse/lychee) if installed, otherwise falls back to a basic relative-link checker.

## What This Replaces

| xtask command | Replaced |
|---------------|----------|
| `release` | `scripts/release.sh` (removed) |
| `sync-versions` | `scripts/sync-versions.sh` (removed) |
| `changelog` | `scripts/generate-changelog.sh` (removed) |
| `ci` | manual 3-step workflow |
| `build-web` | manual pnpm commands |
| `integration-test` | manual 8-step curl workflow |
