#!/usr/bin/env bash
set -euo pipefail

ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)
RUNNER="$ROOT/packaging/aur/librefang-docker/librefang-docker"
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT

mkdir -p "$TMP/bin"
cat >"$TMP/bin/docker" <<'MOCK'
#!/usr/bin/env bash
set -eu
printf '%s\n' "$*" >>"$MOCK_LOG"
if [[ "$*" == "container inspect"* ]]; then
  [[ "${MOCK_CONTAINER_STATE:-missing}" != "missing" ]] || exit 1
  if [[ "$*" == *"--format"* ]]; then
    [[ "$MOCK_CONTAINER_STATE" == "managed" ]] && printf 'true\n' || printf 'false\n'
  fi
fi
if [[ "${1:-}" == "run" ]]; then
  printf '%s\n' "${OPENAI_API_KEY:-}" >"$MOCK_SECRET_LOG"
fi
MOCK
chmod +x "$TMP/bin/docker"

cat >"$TMP/docker.env" <<'ENV'
OPENAI_API_KEY=secret-that-must-not-enter-argv
ENV

export PATH="$TMP/bin:$PATH"
export LIBREFANG_DOCKER_ENV="$TMP/docker.env"
export MOCK_LOG="$TMP/docker.log"
export MOCK_SECRET_LOG="$TMP/secret.log"

: >"$MOCK_LOG"
MOCK_CONTAINER_STATE=missing "$RUNNER" start >/dev/null
grep -q -- '--env OPENAI_API_KEY' "$MOCK_LOG"
! grep -q 'secret-that-must-not-enter-argv' "$MOCK_LOG"
grep -q -- '--label ai.librefang.managed=true' "$MOCK_LOG"
grep -qx 'secret-that-must-not-enter-argv' "$MOCK_SECRET_LOG"

: >"$MOCK_LOG"
if MOCK_CONTAINER_STATE=unmanaged "$RUNNER" start >"$TMP/unmanaged.out" 2>&1; then
  echo "unmanaged container was replaced" >&2
  exit 1
fi
grep -q 'Refusing to modify container' "$TMP/unmanaged.out"
! grep -q '^rm -f ' "$MOCK_LOG"

: >"$MOCK_LOG"
MOCK_CONTAINER_STATE=managed "$RUNNER" start >/dev/null
grep -q '^rm -f librefang$' "$MOCK_LOG"

MOCK_CONTAINER_STATE=missing "$RUNNER" stop >"$TMP/stop.out"
grep -q 'already stopped' "$TMP/stop.out"

if MOCK_CONTAINER_STATE=missing "$RUNNER" logs >"$TMP/logs.out" 2>&1; then
  echo "logs unexpectedly succeeded without a container" >&2
  exit 1
fi
grep -q 'does not exist' "$TMP/logs.out"
