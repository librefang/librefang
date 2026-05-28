#!/usr/bin/env bash
# Run `cargo xtask <subcmd> [args…]` natively if a Rust toolchain is on PATH, otherwise fall back to the `librefang-rust-dev` container.
# This lets contributors on a host without rustup (typical macOS dev box) invoke xtask recipes like `just release` without first installing Rust.
#
# Why bash, not /bin/sh: we build argv as arrays so that workspace paths containing spaces (e.g. `~/My Workspace/librefang`) survive being passed to `docker run -v`. POSIX sh has no arrays; string concat + unquoted word-splitting would break on the first space. Bash 3.2 is present on macOS by default and on every supported Linux distro.
#
# Mounts (read-only, into the container):
#   ~/.gitconfig                → /home/dev/.gitconfig — so `git commit` inside the container uses your identity.
#   ~/.ssh                      → /home/dev/.ssh       — so `git push` over SSH works.
#   ~/.config/gh                → /home/dev/.config/gh — so `gh` finds its hosts.yml (the token may live in the macOS keychain — see GH_TOKEN passthrough below).
#   <main-repo>                 — when the caller is in a linked worktree, the main repo's checkout is also mounted at its host absolute path.
#                                 Linked worktrees keep their `.git` as a text file pointing at `<main-repo>/.git/worktrees/<name>` — without this extra mount that absolute path doesn't exist inside the container and every `git` call fails.
#
# In-container HOME is fixed at `/home/dev` (set via `-e HOME=/home/dev`) so the same mount layout works whether the container runs as root (macOS Docker Desktop) or as the host uid (Linux, see below). Docker auto-creates the bind-mount targets.
#
# Env-var passthrough:
#   GH_TOKEN                    — if unset on the host, this script tries `gh auth token` (covers macOS where `gh` keeps the token in Keychain instead of `~/.config/gh/hosts.yml`) and forwards the value via `-e GH_TOKEN`.
#                                 The value is read into this process's env only; it is forwarded via the `-e VAR` form (no value on argv) so it does not appear in `ps`.
#
# Caching:
#   librefang-cargo / librefang-target named volumes hold `CARGO_HOME` / `CARGO_TARGET_DIR` so dependencies compile once and survive across runs.
#   The host's shared `target/` is never touched — matches the isolation guarantee documented under "Verifying without a native toolchain (Docker)" in `CLAUDE.md`.
#   Set `LIBREFANG_RUST_IMAGE_REBUILD=1` to force a fresh `docker build` (the wrapper otherwise reuses any locally cached `librefang-rust-dev:latest`, even if `Dockerfile.rust-dev` has changed since).
#
# UID mapping (Linux only):
#   On Linux hosts the container runs as the host uid:gid so generated files (CHANGELOG.md, Cargo.lock, openapi.json, …) end up owned by the contributor instead of root. The named volumes are chown'd once per host-uid the first time they're used (a marker file inside the volume avoids re-traversing on every run). On macOS Docker Desktop handles uid translation transparently, so the container keeps running as root and no chown is needed — same code path as before for that platform.
#
# Forcing the docker path:
#   Set `LIBREFANG_RUST_FORCE_DOCKER=1` to skip the native `cargo` short-circuit even when a host toolchain is available. Useful for reproducing a contributor's container behaviour from a machine that has cargo installed.
#
# Known gaps not solved here:
#   `gh` and `claude` are not in the dev image. `release.rs` guards `gh` with `gh --version` before use, so a missing `gh` skips the PR-create step and prints the command for the user to run on the host. `claude` is best-effort (release continues with the raw changelog).
#   `ssh-agent` does not cross the container boundary. The wrapper mounts `~/.ssh` read-only so on-disk keys work, but `$SSH_AUTH_SOCK` points at a host unix socket the container can't see — passphrase-protected keys that rely on the agent will block `git push` from inside the container. Run with `ssh-add -l` working on the host and an unencrypted key on disk, or push from the host afterwards.
#   Windows: this script is bash-only; the `justfile` exposes `release` as `[unix]` + `[windows]`, where the Windows variant keeps the original `cargo xtask release` call and therefore still requires a native Rust toolchain on Windows. There is no docker fallback for cmd.exe.
set -euo pipefail

if [[ "${LIBREFANG_RUST_FORCE_DOCKER:-}" != "1" ]] && command -v cargo >/dev/null 2>&1; then
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
GUEST_HOME=/home/dev

if [[ "${LIBREFANG_RUST_IMAGE_REBUILD:-}" == "1" ]] || ! docker image inspect "$IMAGE" >/dev/null 2>&1; then
    echo "info: building $IMAGE from Dockerfile.rust-dev (one-time, ~10 min)" >&2
    docker build -t "$IMAGE" -f "$REPO_ROOT/Dockerfile.rust-dev" "$REPO_ROOT"
fi

# Linux: chown named volumes to host uid:gid once, then run --user. macOS: no-op (Docker Desktop handles uid mapping; running as root inside is fine).
user_args=()
if [[ "$(uname -s)" == "Linux" ]]; then
    host_uid=$(id -u)
    host_gid=$(id -g)
    marker="/cargo/.owned-by-${host_uid}"
    if ! docker run --rm \
            -v librefang-cargo:/cargo \
            "$IMAGE" \
            test -f "$marker" >/dev/null 2>&1; then
        echo "info: chowning librefang-cargo + librefang-target volumes to ${host_uid}:${host_gid} (one-time)" >&2
        docker run --rm \
            --user 0:0 \
            -v librefang-cargo:/cargo \
            -v librefang-target:/target \
            "$IMAGE" \
            sh -c "chown -R ${host_uid}:${host_gid} /cargo /target && touch '$marker'"
    fi
    user_args=(--user "${host_uid}:${host_gid}")
fi

# Build the inner command with POSIX-safe single-quote escaping so args containing spaces or quotes survive the `sh -c` wrapper inside the container.
# `gh auth setup-git` (when GH_TOKEN is present) wires gh in as the git credential helper, so `git push` against https://github.com URLs authenticates via the forwarded token instead of dropping to an interactive HTTPS username prompt. Stderr is silenced because gh prints a banner; we only care about its side effect.
inner_cmd='export PATH=/usr/local/cargo/bin:$PATH'
inner_cmd+=' && if [ -n "${GH_TOKEN:-}" ]; then gh auth setup-git 2>/dev/null || true; fi'
inner_cmd+=' && exec cargo xtask'
for arg in "$@"; do
    quoted=${arg//\'/\'\\\'\'}
    inner_cmd+=" '$quoted'"
done

mounts=(-v "$REPO_ROOT:/work")
# Mount the main repo at its host path so the `.git` text-file pointer inside a linked worktree resolves; skip when the worktree IS the main repo (would collide on the same target).
if [[ "$MAIN_REPO" != "$REPO_ROOT" ]]; then
    mounts+=(-v "$MAIN_REPO:$MAIN_REPO")
fi
if [[ -f "$HOME/.gitconfig" ]]; then
    mounts+=(-v "$HOME/.gitconfig:$GUEST_HOME/.gitconfig:ro")
fi
if [[ -d "$HOME/.ssh" ]]; then
    mounts+=(-v "$HOME/.ssh:$GUEST_HOME/.ssh:ro")
fi
if [[ -d "$HOME/.config/gh" ]]; then
    mounts+=(-v "$HOME/.config/gh:$GUEST_HOME/.config/gh:ro")
fi

# Pull the gh token out of the host's keychain (macOS) or wherever `gh auth token` finds it, so the container authenticates even when `~/.config/gh/hosts.yml` carries no token.
env_args=(-e "HOME=$GUEST_HOME" -e CARGO_HOME=/cargo -e CARGO_TARGET_DIR=/target)
if [[ -z "${GH_TOKEN:-}" ]] && command -v gh >/dev/null 2>&1; then
    if token=$(gh auth token 2>/dev/null) && [[ -n "$token" ]]; then
        export GH_TOKEN="$token"
    fi
fi
if [[ -n "${GH_TOKEN:-}" ]]; then
    env_args+=(-e GH_TOKEN)
fi

# `-it` only when stdin AND stdout are ttys — keeps the wrapper usable from CI / hooks where stdin is a pipe.
tty_args=()
if [[ -t 0 && -t 1 ]]; then
    tty_args=(-it)
fi

# `${arr[@]+"${arr[@]}"}` guards the empty-array case — under `set -u` on bash 3.2 (macOS default) a bare `"${arr[@]}"` errors with "unbound variable" when the array has never been assigned a value. `mounts` and `env_args` are always non-empty so the guard is only needed on the optional ones.
exec docker run --rm \
    ${tty_args[@]+"${tty_args[@]}"} \
    ${user_args[@]+"${user_args[@]}"} \
    "${mounts[@]}" \
    "${env_args[@]}" \
    -v librefang-cargo:/cargo \
    -v librefang-target:/target \
    -w /work \
    "$IMAGE" \
    sh -c "$inner_cmd"
