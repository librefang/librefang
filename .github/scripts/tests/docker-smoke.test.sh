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
  inspect)
    [ "$SMOKE_MODE" = crashed ] && printf 'false\n' || printf 'true\n'
    ;;
  logs) printf 'fixture container log\n' ;;
  *) exit 2 ;;
esac
EOF
cat > "$FAKE_BIN/curl" <<'EOF'
#!/bin/sh
printf 'curl %s\n' "$*" >> "${SMOKE_LOG:?}"
case "$SMOKE_MODE:$*" in
  healthy:*) exit 0 ;;
  not_ready:*\/api\/health) exit 0 ;;
  *) exit 1 ;;
esac
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
    DOCKER_SMOKE_CURL_TIMEOUT_SECONDS=3 \
    sh "$SCRIPT"
}

run_smoke healthy
grep -q 'curl .*\/api/health' "$TEST_ROOT/healthy.log" \
  || { echo 'FAIL: health endpoint was not probed' >&2; exit 1; }
grep -q 'curl .*\/api/ready' "$TEST_ROOT/healthy.log" \
  || { echo 'FAIL: readiness endpoint was not probed' >&2; exit 1; }
[ "$(grep -c '^curl --max-time 3 ' "$TEST_ROOT/healthy.log")" -eq 2 ] \
  || { echo 'FAIL: both HTTP probes should use the configured timeout' >&2; exit 1; }

if run_smoke crashed >/dev/null 2>&1; then
  echo 'FAIL: exited container should fail the smoke test' >&2
  exit 1
fi
[ "$(grep -c '^docker inspect ' "$TEST_ROOT/crashed.log")" -eq 1 ] \
  || { echo 'FAIL: exited container should be detected on the first poll' >&2; exit 1; }
grep -q '^docker logs ' "$TEST_ROOT/crashed.log" \
  || { echo 'FAIL: exited container logs were not collected' >&2; exit 1; }

if run_smoke not_ready >/dev/null 2>&1; then
  echo 'FAIL: a container that never becomes ready should fail' >&2
  exit 1
fi
[ "$(grep -c '^docker inspect ' "$TEST_ROOT/not_ready.log")" -eq 2 ] \
  || { echo 'FAIL: readiness timeout should exhaust every poll' >&2; exit 1; }
[ "$(grep -c 'curl .*\/api/ready' "$TEST_ROOT/not_ready.log")" -eq 2 ] \
  || { echo 'FAIL: readiness endpoint should be checked on every poll' >&2; exit 1; }
grep -q '^docker logs ' "$TEST_ROOT/not_ready.log" \
  || { echo 'FAIL: readiness timeout logs were not collected' >&2; exit 1; }

echo 'docker-smoke tests passed'
