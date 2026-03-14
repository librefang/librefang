# LibreFang Development Commands
# Install just: https://github.com/casey/just

# Build all workspace crates (library targets)
build:
    cargo build --workspace --lib

# Run all workspace tests
test:
    cargo test --workspace

# Run clippy with warnings as errors
lint:
    cargo clippy --workspace --all-targets -- -D warnings

# Format all code
fmt:
    cargo fmt --all

# Check all workspace crates (fast compile check, no codegen)
check:
    cargo check --workspace

# Run local CI simulation: format check + lint + test
ci:
    cargo fmt --all -- --check
    cargo clippy --workspace --all-targets -- -D warnings
    cargo test --workspace

# Generate workspace documentation
doc:
    cargo doc --workspace --no-deps

# Clean build artifacts
clean:
    cargo clean

# Sync version numbers across all crates
sync-versions:
    ./scripts/sync-versions.sh

# Create a release
release:
    ./scripts/release.sh
