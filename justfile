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

# Start dev environment.
# Native mode (default):   builds librefang-cli on host, starts daemon + dashboard with cargo-watch hot-reload. Requires host Rust toolchain.
# Docker mode (--docker):  builds daemon + Rust sidecar binaries inside the librefang-rust-dev container, mounts host ~/.librefang/ in, forwards port 4545. Requires NO host Rust toolchain. Pass `--docker --port 4646` to change the port.
dev *ARGS:
    #!/usr/bin/env bash
    set -euo pipefail
    # Explicit --docker → docker mode.
    for arg in {{ARGS}}; do
      if [ "$arg" = "--docker" ]; then
        exec just _dev-docker {{ARGS}}
      fi
    done
    # No --docker but no host cargo either → auto-fall-back to docker mode rather than dying with a confusing `sh: cargo: not found`. Notify so the operator knows what's happening.
    if ! command -v cargo >/dev/null 2>&1; then
      echo "Host has no cargo on PATH; falling back to --docker mode. Run 'mise install rust' to use the native path instead."
      exec just _dev-docker --docker {{ARGS}}
    fi
    exec cargo xtask dev {{ARGS}}

# Pure-shell docker workflow invoked from `just dev --docker`. Not meant to be called directly — use `just dev --docker` so the args parse the same way as the native path.
# Builds daemon + Rust Telegram sidecar inside librefang-rust-dev:latest, bind-mounts host ~/.librefang/ at /root/.librefang/ for config persistence, forwards port 4545 (or the value of `--port <n>`) to the host. Dashboard / cargo-watch are not started — both belong on the host alongside the editor.
_dev-docker *ARGS:
    #!/usr/bin/env bash
    set -euo pipefail
    PORT=4545
    IMAGE_TAG="librefang-rust-dev:latest"
    # Strip --docker and parse --port <n> / --image <tag> if present.
    args=({{ARGS}})
    skip_next=0
    for i in "${!args[@]}"; do
      if [ "$skip_next" = "1" ]; then skip_next=0; continue; fi
      case "${args[$i]}" in
        --docker) ;;
        --port)  PORT="${args[$((i+1))]}"; skip_next=1 ;;
        --image) IMAGE_TAG="${args[$((i+1))]}"; skip_next=1 ;;
        *) ;;
      esac
    done
    HOME_LIBREFANG="${HOME}/.librefang"
    mkdir -p "$HOME_LIBREFANG"
    REPO_ROOT="$(git rev-parse --show-toplevel)"

    # Build the dev image if it isn't on the host yet (one-time, ~5 minutes).
    if ! docker image inspect "$IMAGE_TAG" >/dev/null 2>&1; then
      echo "Building $IMAGE_TAG from Dockerfile.rust-dev (one-time, ~5 minutes)..."
      docker build -t "$IMAGE_TAG" -f "${REPO_ROOT}/Dockerfile.rust-dev" "${REPO_ROOT}"
    fi

    # Compile daemon + Rust Telegram sidecar into the librefang-target named volume.
    echo "Building librefang-cli + librefang-sidecar-telegram inside the container (warm cache: ~30s, cold: ~10 min)..."
    docker run --rm \
      -v "${REPO_ROOT}:/work" \
      -v librefang-cargo:/cargo -v librefang-target:/target \
      -e CARGO_HOME=/cargo -e CARGO_TARGET_DIR=/target \
      -w /work "$IMAGE_TAG" \
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
        -w /work "$IMAGE_TAG" \
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

    Note `command =` is the in-container path; the binary lives in the
    librefang-target named volume mounted at /target inside the daemon.

    Reference: https://docs.librefang.ai/architecture/rust-telegram-sidecar
    NOTE
    fi

    # Pre-clean any stale container (orphan from a previous `--rm` that didn't fire).
    docker rm -f librefang-dev >/dev/null 2>&1 || true

    # Forward provider keys from the host environment if they're set, so the agent inside the container can answer.
    HOST_ENV_ARGS=()
    for k in OPENAI_API_KEY ANTHROPIC_API_KEY GROQ_API_KEY GOOGLE_API_KEY GEMINI_API_KEY DEEPSEEK_API_KEY TELEGRAM_BOT_TOKEN TELEGRAM_LOG; do
      if [ -n "${!k:-}" ]; then
        HOST_ENV_ARGS+=(-e "${k}")
      fi
    done

    echo
    echo "Starting daemon in container on port ${PORT}..."
    echo "  Host repo    ↔ /work"
    echo "  ~/.librefang ↔ /root/.librefang"
    echo "  binaries     ↔ named volume librefang-target (/target)"
    if [ ${#HOST_ENV_ARGS[@]} -gt 0 ]; then
      forwarded=$(printf ' %s' "${HOST_ENV_ARGS[@]}" | tr -s ' ' | sed 's/-e //g')
      echo "  forwarding host env: ${forwarded}"
    fi
    echo
    docker run -it --rm --name librefang-dev \
      -v "${REPO_ROOT}:/work" \
      -v "${HOME_LIBREFANG}:/root/.librefang" \
      -v librefang-cargo:/cargo -v librefang-target:/target \
      -e CARGO_HOME=/cargo -e CARGO_TARGET_DIR=/target \
      -e LIBREFANG_PORT=${PORT} \
      "${HOST_ENV_ARGS[@]}" \
      -p ${PORT}:${PORT} \
      -w /work "$IMAGE_TAG" \
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
