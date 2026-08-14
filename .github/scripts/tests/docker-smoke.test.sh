#!/bin/sh
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/../../.." && pwd)
SCRIPT="$ROOT/.github/scripts/docker-smoke.sh"
TEST_ROOT=$(mktemp -d)
trap 'rm -rf "$TEST_ROOT"' EXIT HUP INT TERM
FAKE_BIN="$TEST_ROOT/bin"
mkdir -p "$FAKE_BIN"

cat > "$FAKE_BIN/docker" <<'EOF'
#!/bin/sh
printf 'docker %s\n' "$*" >> "${SMOKE_LOG:?}"
case "$1" in
  run) printf 'container-id\n' ;;
  inspect) [ "$SMOKE_MODE" = healthy ] && printf 'true\n' || printf 'false\n' ;;
  logs) printf 'fixture container log\n' ;;
  *) exit 2 ;;
esac
EOF
cat > "$FAKE_BIN/curl" <<'EOF'
#!/bin/sh
printf 'curl %s\n' "$*" >> "${SMOKE_LOG:?}"
[ "$SMOKE_MODE" = healthy ]
EOF
chmod +x "$FAKE_BIN/docker" "$FAKE_BIN/curl"

run_smoke() {
  mode=$1
  log="$TEST_ROOT/$mode.log"
  : > "$log"
  PATH="$FAKE_BIN:$PATH" \
    SMOKE_MODE="$mode" \
    SMOKE_LOG="$log" \
    DOCKER_SMOKE_ATTEMPTS=2 \
    DOCKER_SMOKE_INTERVAL_SECONDS=0 \
    sh "$SCRIPT"
}

run_smoke healthy
grep -q 'curl .*\/api/health' "$TEST_ROOT/healthy.log" \
  || { echo 'FAIL: health endpoint was not probed' >&2; exit 1; }
grep -q 'curl .*\/api/ready' "$TEST_ROOT/healthy.log" \
  || { echo 'FAIL: readiness endpoint was not probed' >&2; exit 1; }

if run_smoke crashed >/dev/null 2>&1; then
  echo 'FAIL: exited container should fail the smoke test' >&2
  exit 1
fi
[ "$(grep -c '^docker inspect ' "$TEST_ROOT/crashed.log")" -eq 1 ] \
  || { echo 'FAIL: exited container should be detected on the first poll' >&2; exit 1; }
grep -q '^docker logs ' "$TEST_ROOT/crashed.log" \
  || { echo 'FAIL: exited container logs were not collected' >&2; exit 1; }

echo 'docker-smoke tests passed'
