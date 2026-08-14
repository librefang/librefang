#!/bin/sh
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/../../.." && pwd)
SCRIPT="$ROOT/.github/scripts/dashboard-release-tag.sh"
TEST_ROOT=$(mktemp -d)
trap 'rm -rf "$TEST_ROOT"' EXIT HUP INT TERM
FAKE_BIN="$TEST_ROOT/bin"
mkdir -p "$FAKE_BIN"

cat > "$FAKE_BIN/gh" <<'EOF'
#!/bin/sh
printf '%s\n' "$*" > "${GH_LOG:?}"
printf 'v2026.8.1\n'
EOF
chmod +x "$FAKE_BIN/gh"

malicious='v1; touch should-not-exist $(false)'
actual=$(EVENT_NAME=workflow_dispatch REPOSITORY=o/r EVENT_RELEASE_TAG="$malicious" \
  PATH="$FAKE_BIN:$PATH" GH_LOG="$TEST_ROOT/dispatch.log" sh "$SCRIPT")
[ "$actual" = "$malicious" ] || { echo 'FAIL: dispatch tag changed' >&2; exit 1; }
[ ! -e should-not-exist ] || { echo 'FAIL: dispatch tag executed as shell' >&2; exit 1; }
[ ! -e "$TEST_ROOT/dispatch.log" ] || { echo 'FAIL: dispatch should not query releases' >&2; exit 1; }

actual=$(EVENT_NAME=push REPOSITORY=o/r PATH="$FAKE_BIN:$PATH" \
  GH_LOG="$TEST_ROOT/push.log" sh "$SCRIPT")
[ "$actual" = 'v2026.8.1' ] || { echo 'FAIL: push did not return stable release' >&2; exit 1; }
grep -q -- '--repo o/r --exclude-drafts --exclude-pre-releases --limit 1' "$TEST_ROOT/push.log" \
  || { echo 'FAIL: push release query did not exclude drafts and prereleases' >&2; exit 1; }

echo 'dashboard-release-tag tests passed'
