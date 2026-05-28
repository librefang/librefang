# LibreFang development commands — requires https://github.com/casey/just
#
# CANONICAL DEVELOPER ENTRY POINT.
#
# `justfile` is the developer-facing surface; the underlying logic lives in
# `xtask/` (a regular cargo crate that anything in `xtask/src/<name>.rs`
# can grow without taking a `just` dependency). The rule of thumb:
#
#   - Anything non-trivial (multi-step builds, code-gen, release flows,
#     dependency audits, doctor checks, …) lives in `xtask` and is exposed
#     here as a one-line `cargo xtask <subcmd> {{ARGS}}` recipe. Add a new
#     subcommand by editing `xtask/src/main.rs` + a new module; then add a
#     one-line recipe below that forwards `{{ARGS}}`.
#   - Recipes that are pure single-line cargo invocations (`cargo build`,
#     `cargo fmt`, `cargo clippy`, …) may live directly in this file
#     without going through xtask. Anything more than a single command —
#     copying files around, running a tool with non-trivial arguments,
#     branching on platform — belongs in xtask, not as a multi-line `just`
#     recipe. Multi-line recipes here are a smell.
#   - Documentation should reference `just <recipe>` everywhere; mentions
#     of `cargo xtask <subcmd>` in user-facing docs are now a documentation
#     bug — fix the doc to say `just <subcmd>`.
#
# If a recipe and an xtask subcommand drift apart, the xtask side is
# authoritative — update the recipe to forward, don't reimplement.

set windows-shell := ["cmd", "/c"]

# Default: list available recipes
default:
    @just --list

# Build all workspace libraries
build:
    cargo build --workspace --lib

# Run all workspace tests
test:
    cargo test --workspace

# Run clippy with strict warnings
lint:
    cargo clippy --workspace --all-targets -- -D warnings

# Format all code
fmt:
    cargo fmt --all

# Check formatting without modifying files
fmt-check:
    cargo fmt --all -- --check

# Type-check the workspace
check:
    cargo check --workspace

# Local CI simulation: build + test + clippy + web lint
ci:
    cargo xtask ci

# Build and open workspace documentation
doc:
    cargo doc --workspace --no-deps --open

# Build frontend targets (dashboard, web, docs)
build-web *ARGS:
    cargo xtask build-web {{ARGS}}

# Build the React dashboard assets used by librefang-api
dashboard-build:
    cargo xtask build-web --dashboard

# Start React dashboard in dev mode (requires API running on :4545)
dash:
    cd crates/librefang-api/dashboard && pnpm install && pnpm dev

# Build desktop app (Tauri) — builds dashboard assets first (requires: cargo install tauri-cli)
desktop-build: dashboard-build
    cargo tauri build -c crates/librefang-desktop/tauri.conf.json

# Start desktop app in dev mode (requires: cargo install tauri-cli)
desktop-dev: dashboard-build
    cargo tauri dev -c crates/librefang-desktop/tauri.conf.json

# Build release CLI and install to ~/.librefang/bin
[unix]
install: dashboard-build
    cargo build --profile release-local -p librefang-cli
    mkdir -p ~/.librefang/bin
    cp -f target/release-local/librefang ~/.librefang/bin/librefang

# Build release CLI, install binary and fresh dashboard to ~/.librefang
[unix]
install-full: dashboard-build
    cargo build --profile release-local -p librefang-cli
    mkdir -p ~/.librefang/bin
    cp -f target/release-local/librefang ~/.librefang/bin/librefang
    rm -rf ~/.librefang/dashboard
    cp -r crates/librefang-api/static/react ~/.librefang/dashboard
    cargo metadata --format-version 1 --no-deps 2>/dev/null | python3 -c "import sys,json; pkgs=json.load(sys.stdin)['packages']; print(next(p['version'] for p in pkgs if p['name']=='librefang-cli'))" > ~/.librefang/dashboard/.version

# Build release CLI and install to %USERPROFILE%\.librefang\bin (Windows)
[windows]
install: dashboard-build
    cargo build --profile release-local -p librefang-cli
    if not exist "%USERPROFILE%\.librefang\bin" mkdir "%USERPROFILE%\.librefang\bin"
    copy /Y "target\release-local\librefang.exe" "%USERPROFILE%\.librefang\bin\librefang.exe"

# Remove build artifacts
clean:
    cargo clean

# Synchronize crate versions
sync-versions *ARGS:
    cargo xtask sync-versions {{ARGS}}

# Cut a release (falls back to `librefang-rust-dev` container if cargo is missing)
[unix]
release *ARGS:
    scripts/run-xtask.sh release {{ARGS}}

# Cut a release (Windows: requires a native Rust toolchain — the docker fallback used on Unix relies on bash, which cmd cannot exec)
[windows]
release *ARGS:
    cargo xtask release {{ARGS}}

# Generate CHANGELOG from merged PRs
changelog *ARGS:
    cargo xtask changelog {{ARGS}}

# Run live integration tests
integration-test *ARGS:
    cargo xtask integration-test {{ARGS}}

# Publish SDKs to npm/PyPI/crates.io
publish-sdks *ARGS:
    cargo xtask publish-sdks {{ARGS}}

# Build release binaries for multiple platforms
dist *ARGS:
    cargo xtask dist {{ARGS}}

# Build and optionally push Docker image
docker *ARGS:
    cargo xtask docker {{ARGS}}

# Set up local development environment
setup *ARGS:
    cargo xtask setup {{ARGS}}

# Generate test coverage report
coverage *ARGS:
    cargo xtask coverage {{ARGS}}

# Audit dependencies for vulnerabilities and updates
deps *ARGS:
    cargo xtask deps {{ARGS}}

# Run code generation (OpenAPI spec, etc.)
codegen *ARGS:
    cargo xtask codegen {{ARGS}}

# Check for broken links in documentation
check-links *ARGS:
    cargo xtask check-links {{ARGS}}

# Run criterion benchmarks
bench *ARGS:
    cargo xtask bench {{ARGS}}

# Migrate agents from other frameworks
migrate *ARGS:
    cargo xtask migrate {{ARGS}}

# Check/fix formatting (Rust + web)
fmt-all *ARGS:
    cargo xtask fmt {{ARGS}}

# Clean all build artifacts
clean-all *ARGS:
    cargo xtask clean-all {{ARGS}}

# Diagnose development environment issues
doctor *ARGS:
    cargo xtask doctor {{ARGS}}

# Start dev environment (daemon + dashboard hot reload)
# Pass `--docker` to run daemon + sidecar binaries inside the librefang-rust-dev container with host ~/.librefang mounted in. Requires a host Rust toolchain (cargo + xtask). If the host has no Rust installed, use `just dev-docker` instead — it's a pure-shell wrapper around the same docker workflow.
dev *ARGS:
    cargo xtask dev {{ARGS}}

# Pure-shell dev-in-docker entry point. Identical workflow to `just dev --docker` but requires NO host Rust toolchain — daemon and sidecar binaries are built and run entirely inside the librefang-rust-dev container, with host ~/.librefang/ bind-mounted in so config edits on the host are immediately visible. Use this on a fresh macOS / Linux host where `mise install rust` hasn't been run yet.
#   PORT       daemon port (default 4545; also forwarded to host)
#   IMAGE_TAG  override the dev image tag (default librefang-rust-dev:latest)
dev-docker PORT="4545" IMAGE_TAG="librefang-rust-dev:latest":
    #!/usr/bin/env bash
    set -euo pipefail
    HOME_LIBREFANG="${HOME}/.librefang"
    mkdir -p "$HOME_LIBREFANG"
    REPO_ROOT="$(git rev-parse --show-toplevel)"

    # Build the dev image if it isn't on the host yet (one-time, ~5 minutes).
    if ! docker image inspect {{IMAGE_TAG}} >/dev/null 2>&1; then
      echo "Building {{IMAGE_TAG}} from Dockerfile.rust-dev (one-time, ~5 minutes)..."
      docker build -t {{IMAGE_TAG}} -f "${REPO_ROOT}/Dockerfile.rust-dev" "${REPO_ROOT}"
    fi

    # Compile daemon + Rust Telegram sidecar into the librefang-target named volume.
    echo "Building librefang-cli + librefang-sidecar-telegram inside the container..."
    docker run --rm \
      -v "${REPO_ROOT}:/work" \
      -v librefang-cargo:/cargo -v librefang-target:/target \
      -e CARGO_HOME=/cargo -e CARGO_TARGET_DIR=/target \
      -w /work {{IMAGE_TAG}} \
      sh -c 'export PATH=/usr/local/cargo/bin:$PATH && \
             cargo build --release -p librefang-cli && \
             cargo build --release --manifest-path sdk/rust/librefang-sidecar-telegram/Cargo.toml'

    # Bootstrap ~/.librefang/config.toml if missing.
    if [ ! -f "${HOME_LIBREFANG}/config.toml" ]; then
      echo "Bootstrapping ${HOME_LIBREFANG}/config.toml via 'librefang init --quick'..."
      docker run --rm \
        -v "${REPO_ROOT}:/work" \
        -v "${HOME_LIBREFANG}:/root/.librefang" \
        -v librefang-target:/target \
        -w /work {{IMAGE_TAG}} \
        /target/release/librefang init --quick || \
        echo "warn: 'librefang init --quick' exited non-zero, continuing"
      cat <<'NOTE'

      Edit ~/.librefang/config.toml to add the Rust Telegram sidecar:

        [[sidecar_channels]]
        name = "telegram"
        command = "/target/release/librefang-sidecar-telegram"
        channel_type = "telegram"
        [sidecar_channels.secrets]
        TELEGRAM_BOT_TOKEN = "<your-token>"

      Note `command` is the in-container path; the binary lives in the
      librefang-target named volume mounted at /target inside the daemon.

      Reference: https://docs.librefang.ai/architecture/rust-telegram-sidecar
    NOTE
    fi

    # Pre-clean any stale container (orphan from a previous `--rm` that didn't fire).
    docker rm -f librefang-dev >/dev/null 2>&1 || true

    echo
    echo "Starting daemon in container on port {{PORT}}..."
    echo "  Host repo    ↔ /work"
    echo "  ~/.librefang ↔ /root/.librefang"
    echo "  binaries     ↔ named volume librefang-target (/target)"
    echo
    docker run -it --rm --name librefang-dev \
      -v "${REPO_ROOT}:/work" \
      -v "${HOME_LIBREFANG}:/root/.librefang" \
      -v librefang-cargo:/cargo -v librefang-target:/target \
      -e CARGO_HOME=/cargo -e CARGO_TARGET_DIR=/target \
      -e LIBREFANG_PORT={{PORT}} \
      -p {{PORT}}:{{PORT}} \
      -w /work {{IMAGE_TAG}} \
      /target/release/librefang start --foreground

# Database management (info, backup, reset)
db *ARGS:
    cargo xtask db {{ARGS}}

# Check dependency licenses
license-check *ARGS:
    cargo xtask license-check {{ARGS}}

# Code statistics (lines of code, dependency graph)
loc *ARGS:
    cargo xtask loc {{ARGS}}

# Update dependencies (Rust + web)
update-deps *ARGS:
    cargo xtask update-deps {{ARGS}}

# Validate config.toml
validate-config *ARGS:
    cargo xtask validate-config {{ARGS}}

# Run pre-commit checks (fmt + clippy + test)
pre-commit *ARGS:
    cargo xtask pre-commit {{ARGS}}

# Generate API docs from OpenAPI spec
api-docs *ARGS:
    cargo xtask api-docs {{ARGS}}

# Generate contributors + star history SVGs
contributors *ARGS:
    cargo xtask contributors {{ARGS}}

# Publish CLI binaries to npm
publish-npm-binaries *ARGS:
    cargo xtask publish-npm-binaries {{ARGS}}

# Publish CLI wheels to PyPI
publish-pypi-binaries *ARGS:
    cargo xtask publish-pypi-binaries {{ARGS}}
