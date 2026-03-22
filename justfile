# LibreFang development commands — requires https://github.com/casey/just

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
    cd crates/librefang-api/dashboard && pnpm dev

# Start API daemon with dashboard dev server (hot reload)
api: dashboard-build
    cd crates/librefang-api/dashboard && pnpm dev &
    cargo run -p librefang-cli -- start --foreground

# Remove build artifacts
clean:
    cargo clean

# Synchronize crate versions
sync-versions *ARGS:
    cargo xtask sync-versions {{ARGS}}

# Cut a release
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
dev *ARGS:
    cargo xtask dev {{ARGS}}

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
