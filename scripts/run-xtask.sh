#!/usr/bin/env sh
# Run `cargo xtask <subcmd> [args…]` natively if a Rust toolchain is on PATH, otherwise fall back to the `librefang-rust-dev` container.
# This lets contributors on a host without rustup (typical macOS dev box) invoke xtask recipes like `just release` without first installing Rust.
#
# Mounts (read-only, into the container):
#   ~/.gitconfig                — so `git commit` inside the container uses your identity.
#   ~/.ssh                      — so `git push` over SSH works.
#   ~/.config/gh                — so `gh` finds its hosts.yml (the token may live in the macOS keychain — see GH_TOKEN passthrough below).
#   <main-repo>                 — when the caller is in a linked worktree, the main repo's checkout is mounted at the same absolute path it has on the host.
#                                 Linked worktrees keep their `.git` as a text file pointing at `<main-repo>/.git/worktrees/<name>` — without this extra mount that absolute path doesn't exist inside the container and every `git` call fails.
#
# Env-var passthrough:
#   GH_TOKEN                    — if unset, this script tries `gh auth token` on the host (covers macOS where `gh` keeps the token in Keychain instead of `~/.config/gh/hosts.yml`) and forwards the value via `-e GH_TOKEN`.
#                                 The value is read into this process's env only; it is never printed and is forwarded via the `-e VAR` form (no value on argv) so it does not appear in `ps`.
#
# Caching:
#   librefang-cargo / librefang-target named volumes hold `CARGO_HOME` / `CARGO_TARGET_DIR` so dependencies compile once and survive across runs.
#   The host's shared `target/` is never touched — matches the isolation guarantee documented under "Verifying without a native toolchain (Docker)" in `CLAUDE.md`.
#   Set `LIBREFANG_RUST_IMAGE_REBUILD=1` to force a fresh `docker build` (the wrapper otherwise reuses any locally cached `librefang-rust-dev:latest`, even if `Dockerfile.rust-dev` has changed since).
#
# Known gaps not worth solving in this script:
#   `gh` and `claude` are not in the dev image — `release.rs` calls both. `claude` is best-effort (release continues without it). `gh` is checked with `gh --version` before use, so missing-`gh` skips the PR-create step and prints the command for the user to run on the host.
#   Files written through the bind mount are root-owned on Linux hosts (Docker Desktop on macOS hides this via UID mapping). Add `--user "$(id -u):$(id -g)"` if that bites you; left out for now because the cargo named volume would also need world-writable perms.
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
GIT_COMMON_DIR_ABS="$(cd "$(git rev-parse --git-common-dir)" && pwd)"
MAIN_REPO="$(dirname "$GIT_COMMON_DIR_ABS")"
IMAGE="${LIBREFANG_RUST_IMAGE:-librefang-rust-dev:latest}"

if [ "${LIBREFANG_RUST_IMAGE_REBUILD:-}" = "1" ] || ! docker image inspect "$IMAGE" >/dev/null 2>&1; then
    echo "info: building $IMAGE from Dockerfile.rust-dev (one-time, ~10 min)" >&2
    docker build -t "$IMAGE" -f "$REPO_ROOT/Dockerfile.rust-dev" "$REPO_ROOT"
fi

# Build the inner command with POSIX-safe single-quote escaping so args containing spaces or quotes survive the `sh -c` wrapper inside the container.
inner_cmd='export PATH=/usr/local/cargo/bin:$PATH && exec cargo xtask'
for arg in "$@"; do
    quoted=$(printf '%s' "$arg" | sed "s/'/'\\\\''/g")
    inner_cmd="$inner_cmd '$quoted'"
done

mounts="-v $REPO_ROOT:/work"
# Mount the main repo at its host path so the `.git` text-file pointer inside a linked worktree still resolves; skip when the worktree IS the main repo (would collide on the same target).
if [ "$MAIN_REPO" != "$REPO_ROOT" ]; then
    mounts="$mounts -v $MAIN_REPO:$MAIN_REPO"
fi
if [ -f "$HOME/.gitconfig" ]; then
    mounts="$mounts -v $HOME/.gitconfig:/root/.gitconfig:ro"
fi
if [ -d "$HOME/.ssh" ]; then
    mounts="$mounts -v $HOME/.ssh:/root/.ssh:ro"
fi
if [ -d "$HOME/.config/gh" ]; then
    mounts="$mounts -v $HOME/.config/gh:/root/.config/gh:ro"
fi

# Pull the gh token out of the host's keychain (macOS) or wherever `gh auth token` finds it, so the container can authenticate even when `~/.config/gh/hosts.yml` carries no token.
env_args=""
if [ -z "${GH_TOKEN:-}" ] && command -v gh >/dev/null 2>&1; then
    GH_TOKEN=$(gh auth token 2>/dev/null || true)
    if [ -n "$GH_TOKEN" ]; then
        export GH_TOKEN
    else
        unset GH_TOKEN
    fi
fi
if [ -n "${GH_TOKEN:-}" ]; then
    env_args="$env_args -e GH_TOKEN"
fi

# `-it` only when stdin AND stdout are ttys — keeps the wrapper usable from CI / hooks where stdin is a pipe.
tty_flag=""
if [ -t 0 ] && [ -t 1 ]; then
    tty_flag="-it"
fi

# shellcheck disable=SC2086  # word-splitting on $mounts / $env_args / $tty_flag is intentional.
exec docker run --rm $tty_flag \
    $mounts \
    $env_args \
    -v librefang-cargo:/cargo \
    -v librefang-target:/target \
    -e CARGO_HOME=/cargo \
    -e CARGO_TARGET_DIR=/target \
    -w /work \
    "$IMAGE" \
    sh -c "$inner_cmd"
