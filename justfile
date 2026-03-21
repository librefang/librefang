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

# Local CI simulation: fmt-check + lint + test
ci: fmt-check lint test

# Build and open workspace documentation
doc:
    cargo doc --workspace --no-deps --open

# Build the React dashboard assets used by librefang-api
dashboard-build:
    cd crates/librefang-api/dashboard && pnpm install && pnpm run build

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
sync-versions:
    ./scripts/sync-versions.sh

# Cut a release
release:
    ./scripts/release.sh
