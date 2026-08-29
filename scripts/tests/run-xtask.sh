#!/usr/bin/env bash
set -euo pipefail

ROOT=$(cd "$(dirname "$0")/../.." && pwd)
SCRIPT="$ROOT/scripts/run-xtask.sh"
FIXTURE=$(mktemp -d "${TMPDIR:-/tmp}/librefang-run-xtask-test.XXXXXX")
trap 'rm -rf "$FIXTURE"' EXIT
mkdir -p "$FIXTURE/bin" "$FIXTURE/repo/.git"

cat >"$FIXTURE/bin/docker" <<'SH'
#!/bin/sh
if [ "$1 $2" = "image inspect" ]; then
    exit 0
fi
{
    printf '%s\n' '--- docker call ---'
    printf '%s\n' "$@"
} >>"${DOCKER_ARGS_LOG:?}"
case "$*" in
    *'test -f'*) exit "${DOCKER_MARKER_STATUS:-0}" ;;
esac
SH
chmod +x "$FIXTURE/bin/docker"

set +e
missing_git_output=$(env -u HOME -u PATH LIBREFANG_RUST_FORCE_DOCKER=1 \
    /bin/bash -c 'PATH="$1"; export PATH; exec /bin/bash "$2"' sh \
    "$FIXTURE/bin" "$SCRIPT" 2>&1)
missing_git_status=$?
set -e
if [[ "$missing_git_status" != 127 || "$missing_git_output" != *'`git` is required'* ]]; then
    echo "FAIL: missing git did not produce the exit-127 diagnostic" >&2
    exit 1
fi

cat >"$FIXTURE/bin/git" <<SH
#!/bin/sh
case "\$*" in
  'rev-parse --show-toplevel') printf '%s\n' '$FIXTURE/repo' ;;
  'rev-parse --git-common-dir') printf '%s\n' '$FIXTURE/repo/.git' ;;
  *) exit 2 ;;
esac
SH
cat >"$FIXTURE/bin/uname" <<'SH'
#!/bin/sh
printf '%s\n' "${FAKE_UNAME:-Darwin}"
SH
cat >"$FIXTURE/bin/id" <<'SH'
#!/bin/sh
case "$1" in
    -u) printf '%s\n' 1234 ;;
    -g) printf '%s\n' 5678 ;;
    *) exit 2 ;;
esac
SH
chmod +x "$FIXTURE/bin/git" "$FIXTURE/bin/uname" "$FIXTURE/bin/id"

env -u HOME \
    PATH="$FIXTURE/bin:/usr/bin:/bin" \
    DOCKER_ARGS_LOG="$FIXTURE/docker.args" \
    LIBREFANG_RUST_FORCE_DOCKER=1 \
    /bin/bash "$SCRIPT" release --dry-run 'arg with space' "single'quote" '$dollar'

grep -F 'export PATH="$CARGO_HOME/bin:$PATH"' "$FIXTURE/docker.args" >/dev/null
grep -Fq 'mkdir -p "$HOME" "$HOME/.config"' "$FIXTURE/docker.args"
grep -Fxq 'HOME=/tmp/librefang-home' "$FIXTURE/docker.args"
grep -F "'$FIXTURE/repo:/work'" "$FIXTURE/docker.args" >/dev/null 2>&1 || \
    grep -F "$FIXTURE/repo:/work" "$FIXTURE/docker.args" >/dev/null
grep -Fq "'arg with space'" "$FIXTURE/docker.args"
grep -Fq "'single'\\''quote'" "$FIXTURE/docker.args"
grep -Fq "'\$dollar'" "$FIXTURE/docker.args"

mkdir -p "$FIXTURE/home/.ssh" "$FIXTURE/home/.config/gh"
: >"$FIXTURE/home/.gitconfig"
HOME="$FIXTURE/home" \
    PATH="$FIXTURE/bin:/usr/bin:/bin" \
    DOCKER_ARGS_LOG="$FIXTURE/docker.args" \
    LIBREFANG_RUST_FORCE_DOCKER=1 \
    /bin/bash "$SCRIPT" release
grep -Fxq "$FIXTURE/home/.ssh:/tmp/librefang-host-ssh:ro" \
    "$FIXTURE/docker.args"
grep -Fxq "$FIXTURE/home/.config/gh:/tmp/librefang-host-gh:ro" \
    "$FIXTURE/docker.args"
grep -Fq 'ln -s /tmp/librefang-host-ssh "$HOME/.ssh"' \
    "$FIXTURE/docker.args"
grep -Fq 'ln -s /tmp/librefang-host-gh "$HOME/.config/gh"' \
    "$FIXTURE/docker.args"

rm "$FIXTURE/docker.args"
env -u HOME \
    PATH="$FIXTURE/bin:/usr/bin:/bin" \
    DOCKER_ARGS_LOG="$FIXTURE/docker.args" \
    DOCKER_MARKER_STATUS=1 \
    FAKE_UNAME=Linux \
    LIBREFANG_RUST_FORCE_DOCKER=1 \
    /bin/bash "$SCRIPT" release

grep -Fq 'chown -R "$1:$2" /cargo /target && touch "$3" "$4"' \
    "$FIXTURE/docker.args" || {
    echo "FAIL: Linux ownership command was not passed literally" >&2
    exit 1
}
grep -Fq 'test -f "$1" && test -f "$2"' "$FIXTURE/docker.args"
grep -Fxq '1234' "$FIXTURE/docker.args"
grep -Fxq '5678' "$FIXTURE/docker.args"
grep -Fxq '/cargo/.owned-by-1234-5678' "$FIXTURE/docker.args"
grep -Fxq '/target/.owned-by-1234-5678' "$FIXTURE/docker.args"

echo "OK: run-xtask.sh"
