#!/bin/sh
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/../../.." && pwd)
SCRIPT="$ROOT/.github/scripts/select-dependabot-pr.sh"
TEST_ROOT=$(mktemp -d)
trap 'rm -rf "$TEST_ROOT"' EXIT HUP INT TERM
FAKE_BIN="$TEST_ROOT/bin"
mkdir -p "$FAKE_BIN"

cat > "$FAKE_BIN/gh" <<'EOF'
#!/bin/sh
case "$1 $2" in
  'api /repos/'*) ;;
esac

if [ "$1" = api ] && printf '%s' "$*" | grep -q '/commits/'; then
  printf '42\n'
  exit 0
fi
if [ "$1 $2" = 'pr list' ]; then
  if [ "${GH_AUTHOR_MODE:-}" = young ]; then
    printf '41\tyoung-green\t2999-01-01T00:00:00Z\n'
  else
    printf '41\told-green\t2000-01-01T00:00:00Z\n'
  fi
  exit 0
fi
if [ "$1" = api ] && printf '%s' "$*" | grep -q '/actions/workflows/ci.yml/runs'; then
  [ "${GH_AUTHOR_MODE:-}" != failed ] || exit 0
  printf '99\n'
  exit 0
fi
if [ "$1 $2" = 'pr view' ]; then
  if [ "${GH_AUTHOR_MODE:-dependabot}" = impostor ]; then
    cat <<'JSON'
{"state":"OPEN","createdAt":"2000-01-01T00:00:00Z","labels":[],"body":"","author":{"is_bot":false,"login":"attacker"},"title":"Bump x from 1.0.0 to 1.0.1","headRefName":"dependabot-lookalike/x","headRefOid":"bad"}
JSON
  else
    cat <<'JSON'
{"state":"OPEN","createdAt":"2000-01-01T00:00:00Z","labels":[],"body":"","author":{"is_bot":true,"login":"app/dependabot"},"title":"Bump x from 1.0.0 to 1.0.1","headRefName":"dependabot/npm/x-1.0.1","headRefOid":"abc123"}
JSON
  fi
  exit 0
fi
exit 2
EOF
chmod +x "$FAKE_BIN/gh"
cat > "$FAKE_BIN/date" <<'EOF'
#!/bin/sh
case "$*" in
  *' -d 2999-'*) printf '32472144000\n' ;;
  *' -d '*) printf '946684800\n' ;;
  *) /bin/date "$@" ;;
esac
EOF
chmod +x "$FAKE_BIN/date"

run_selector() {
  event=$1
  mode=$2
  output="$TEST_ROOT/$event-$mode.json"
  PATH="$FAKE_BIN:$PATH" \
    GH_AUTHOR_MODE="$mode" \
    EVENT_NAME="$event" \
    REPO='librefang/librefang' \
    HEAD_SHA='abc123' \
    PR_JSON_PATH="$output" \
    bash "$SCRIPT"
}

[ "$(run_selector workflow_run dependabot)" = 42 ] \
  || { echo 'FAIL: workflow_run should select PR #42' >&2; exit 1; }
[ "$(run_selector schedule dependabot)" = 41 ] \
  || { echo 'FAIL: schedule should select the aged green PR' >&2; exit 1; }
[ -z "$(run_selector schedule young)" ] \
  || { echo 'FAIL: schedule must not select a PR inside the soak window' >&2; exit 1; }
[ -z "$(run_selector schedule failed)" ] \
  || { echo 'FAIL: schedule must not select a PR without successful latest CI' >&2; exit 1; }
if run_selector workflow_run impostor >/dev/null 2>&1; then
  echo 'FAIL: lookalike PR author must be rejected' >&2
  exit 1
fi

echo 'select-dependabot-pr tests passed'
