#!/usr/bin/env sh
# Run `cargo xtask <subcmd> [args…]` natively if a Rust toolchain is on PATH,
# otherwise fall back to the `librefang-rust-dev` container so contributors
# on a host without rustup (typical macOS dev box) can still invoke xtask
# recipes (e.g. `just release`) without first installing Rust.
#
# Forwarded credentials (mounted read-only into the container):
#   ~/.gitconfig     — so `git commit` inside the container uses your identity
#   ~/.ssh           — so `git push` via SSH works
#   ~/.config/gh     — so `gh pr create` reuses your auth
#
# Cargo / build state lives in named volumes (librefang-cargo,
# librefang-target) so dependencies compile once and survive across runs,
# and the host's shared `target/` directory is NOT touched (the
# Dockerfile.rust-dev comment + CLAUDE.md "Verifying without a native
# toolchain (Docker)" section explain why that isolation matters).
#
# Tools the dev image does NOT ship that some xtask flows expect:
#   gh          — release.rs uses it to open the version-bump PR
#   claude      — release.rs uses it to polish the Dev.to article (optional;
#                 release flow continues with raw changelog if missing)
# If the flow you're running needs one of these, install it inside the
# container by extending Dockerfile.rust-dev, or run that step natively
# on the host after the build artifacts are produced.
set -eu

if command -v cargo >/dev/null 2>&1; then
    exec cargo xtask "$@"
fi

if ! command -v docker >/dev/null 2>&1; then
    cat >&2 <<'EOF'
error: neither `cargo` nor `docker` is on PATH.

Install one of:
  - Rust toolchain (recommended): https://rustup.rs
  - Docker:                       https://docs.docker.com/get-docker/
EOF
    exit 127
fi

REPO_ROOT="$(git rev-parse --show-toplevel)"
IMAGE="${LIBREFANG_RUST_IMAGE:-librefang-rust-dev:latest}"

if ! docker image inspect "$IMAGE" >/dev/null 2>&1; then
    echo "info: building $IMAGE from Dockerfile.rust-dev (one-time, ~10 min)" >&2
    docker build -t "$IMAGE" -f "$REPO_ROOT/Dockerfile.rust-dev" "$REPO_ROOT"
fi

set -- "$@"
# Build the inner command string with proper quoting so args with spaces
# survive the sh -c wrapper inside the container.
inner_cmd='export PATH=/usr/local/cargo/bin:$PATH && exec cargo xtask'
for arg in "$@"; do
    # POSIX-safe single-quote escape: ' -> '\''
    quoted=$(printf '%s' "$arg" | sed "s/'/'\\\\''/g")
    inner_cmd="$inner_cmd '$quoted'"
done

mounts="-v $REPO_ROOT:/work"
if [ -f "$HOME/.gitconfig" ]; then
    mounts="$mounts -v $HOME/.gitconfig:/root/.gitconfig:ro"
fi
if [ -d "$HOME/.ssh" ]; then
    mounts="$mounts -v $HOME/.ssh:/root/.ssh:ro"
fi
if [ -d "$HOME/.config/gh" ]; then
    mounts="$mounts -v $HOME/.config/gh:/root/.config/gh:ro"
fi

# `-it` only when stdin is a tty — keeps the wrapper usable from CI / hooks.
tty_flag=""
if [ -t 0 ] && [ -t 1 ]; then
    tty_flag="-it"
fi

# shellcheck disable=SC2086  # word-splitting on $mounts is intentional
exec docker run --rm $tty_flag \
    $mounts \
    -v librefang-cargo:/cargo \
    -v librefang-target:/target \
    -e CARGO_HOME=/cargo \
    -e CARGO_TARGET_DIR=/target \
    -w /work \
    "$IMAGE" \
    sh -c "$inner_cmd"
