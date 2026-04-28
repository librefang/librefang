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

Image: `ghcr.io/librefang/librefang`. Dockerfile: `./Dockerfile`.

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

### `bench` — Benchmarks

Run criterion benchmarks with optional baseline comparison.

```bash
cargo xtask bench                              # run all benchmarks
cargo xtask bench --name dispatch              # specific benchmark
cargo xtask bench --save-baseline main         # save baseline
cargo xtask bench --baseline main              # compare against baseline
cargo xtask bench --open                       # open HTML report
```

### `migrate` — Framework Migration

Import agents from other agent frameworks into LibreFang.

```bash
cargo xtask migrate --source openclaw --source-dir ~/openclaw-data
cargo xtask migrate --source openfang --source-dir ./import --dry-run
cargo xtask migrate --source openclaw --source-dir ./data --target-dir ~/.librefang
```

Supported sources: `openclaw`, `openfang`.

### `fmt` — Format Check

Unified formatting check across Rust and frontend code.

```bash
cargo xtask fmt                        # check all
cargo xtask fmt --fix                  # auto-fix formatting
cargo xtask fmt --no-web               # Rust only
cargo xtask fmt --no-rust              # web only (prettier)
```

### `clean-all` — Deep Clean

Remove all build artifacts across Rust and frontend.

```bash
cargo xtask clean-all                  # remove everything
cargo xtask clean-all --rust           # target/ + dist/ only
cargo xtask clean-all --web            # node_modules/ + .next/ + dist/ only
cargo xtask clean-all --dry-run        # show what would be deleted
```

### `doctor` — Environment Diagnostics

Deep health check of the development environment.

```bash
cargo xtask doctor                     # full diagnostics
cargo xtask doctor --port 5000         # check custom port
```

Checks: toolchain, port availability, daemon health, config validity, API keys, workspace state.

### `dev` — Development Environment

Start the daemon and dashboard dev server together.

```bash
cargo xtask dev                        # build + start daemon + dashboard
cargo xtask dev --no-dashboard         # daemon only
cargo xtask dev --release              # release build
cargo xtask dev --port 5000            # custom port
```

Press Ctrl+C to stop both processes.

### `db` — Database Management

Inspect, backup, or reset LibreFang databases.

```bash
cargo xtask db                         # show database info
cargo xtask db --info                  # same as above
cargo xtask db --backup ./backup       # backup db files
cargo xtask db --reset                 # delete databases (daemon must be stopped)
cargo xtask db --data-dir /custom/path # custom data directory
```

### `license-check` — License Compliance

Check dependencies for license compliance.

```bash
cargo xtask license-check              # check all (Rust + web)
cargo xtask license-check --rust       # Rust only
cargo xtask license-check --web        # web only
cargo xtask license-check --deny "GPL-3.0,AGPL-3.0"  # custom denied licenses
```

Uses `cargo-deny` if installed, falls back to `cargo metadata`.

### `loc` — Code Statistics

Lines of code and workspace structure.

```bash
cargo xtask loc                        # Rust code summary
cargo xtask loc --crates               # per-crate breakdown
cargo xtask loc --web                  # include web/frontend
cargo xtask loc --deps                 # show crate dependency graph
```

### `update-deps` — Update Dependencies

Batch update Rust and web dependencies.

```bash
cargo xtask update-deps                # update all
cargo xtask update-deps --rust         # Rust only (cargo update)
cargo xtask update-deps --web          # web only (pnpm update)
cargo xtask update-deps --dry-run      # show outdated without updating
cargo xtask update-deps --test         # update + run tests
```

### `validate-config` — Config Validation

Validate `~/.librefang/config.toml` syntax and known fields.

```bash
cargo xtask validate-config            # validate default config
cargo xtask validate-config --config ./my-config.toml  # custom path
cargo xtask validate-config --show     # show parsed config contents
```

### `pre-commit` — Pre-Commit Checks

Run format + clippy + tests as a pre-commit check.

```bash
cargo xtask pre-commit                 # full check (fmt + clippy + test)
cargo xtask pre-commit --no-test       # skip tests (faster)
cargo xtask pre-commit --no-clippy     # skip clippy
cargo xtask pre-commit --fix           # auto-fix formatting
```

### `api-docs` — API Documentation

Generate API documentation site from OpenAPI spec.

```bash
cargo xtask api-docs                   # generate Swagger UI site
cargo xtask api-docs --open            # generate + open in browser
cargo xtask api-docs --refresh         # regenerate openapi.json first
cargo xtask api-docs --output my-docs  # custom output directory
```

Outputs a standalone Swagger UI HTML page with the OpenAPI spec.

## What This Replaces

| xtask command | Replaced |
|---------------|----------|
| `release` | `scripts/release.sh` (removed) |
| `sync-versions` | `scripts/sync-versions.sh` (removed) |
| `changelog` | `scripts/generate-changelog.sh` (removed) |
| `ci` | manual 3-step workflow |
| `build-web` | manual pnpm commands |
| `integration-test` | manual 8-step curl workflow |
