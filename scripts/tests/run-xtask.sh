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
printf '%s\n' "$@" >"${DOCKER_ARGS_LOG:?}"
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
printf '%s\n' Darwin
SH
chmod +x "$FIXTURE/bin/git" "$FIXTURE/bin/uname"

env -u HOME \
    PATH="$FIXTURE/bin:/usr/bin:/bin" \
    DOCKER_ARGS_LOG="$FIXTURE/docker.args" \
    LIBREFANG_RUST_FORCE_DOCKER=1 \
    /bin/bash "$SCRIPT" release --dry-run

grep -F 'export PATH="$CARGO_HOME/bin:$PATH"' "$FIXTURE/docker.args" >/dev/null
grep -F "'$FIXTURE/repo:/work'" "$FIXTURE/docker.args" >/dev/null 2>&1 || \
    grep -F "$FIXTURE/repo:/work" "$FIXTURE/docker.args" >/dev/null

grep -Fq 'sh -c "chown -R ${host_uid}' "$SCRIPT" && {
    echo "FAIL: chown values are still interpolated into the command string" >&2
    exit 1
}
grep -Fq 'sh "$host_uid" "$host_gid" "$marker"' "$SCRIPT"

echo "OK: run-xtask.sh"
