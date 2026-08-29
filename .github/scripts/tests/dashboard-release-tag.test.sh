#!/bin/sh
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/../../.." && pwd)
SCRIPT="$ROOT/.github/scripts/dashboard-release-tag.sh"
TEST_ROOT=$(mktemp -d)
trap 'rm -rf "$TEST_ROOT"' EXIT
trap 'exit 129' HUP
trap 'exit 130' INT
trap 'exit 143' TERM
FAKE_BIN="$TEST_ROOT/bin"
mkdir -p "$FAKE_BIN"

cat > "$FAKE_BIN/gh" <<'EOF'
#!/bin/sh
printf '%s\n' "$@" > "${GH_LOG:?}"
printf '%s\n' "${GH_OUTPUT-v2026.8.1}"
EOF
chmod +x "$FAKE_BIN/gh"

malicious="v1; touch $TEST_ROOT/should-not-exist \$(false)"
actual=$(EVENT_NAME=workflow_dispatch REPOSITORY=o/r EVENT_RELEASE_TAG="$malicious" \
  PATH="$FAKE_BIN:$PATH" GH_LOG="$TEST_ROOT/dispatch.log" sh "$SCRIPT")
[ "$actual" = "$malicious" ] || { echo 'FAIL: dispatch tag changed' >&2; exit 1; }
[ ! -e "$TEST_ROOT/should-not-exist" ] || { echo 'FAIL: dispatch tag executed as shell' >&2; exit 1; }
[ ! -e "$TEST_ROOT/dispatch.log" ] || { echo 'FAIL: dispatch should not query releases' >&2; exit 1; }

actual=$(EVENT_NAME=release REPOSITORY=o/r EVENT_RELEASE_TAG=v2026.8.2 \
  PATH="$FAKE_BIN:$PATH" GH_LOG="$TEST_ROOT/release.log" sh "$SCRIPT")
[ "$actual" = 'v2026.8.2' ] || { echo 'FAIL: release tag changed' >&2; exit 1; }
[ ! -e "$TEST_ROOT/release.log" ] || { echo 'FAIL: release should not query releases' >&2; exit 1; }

actual=$(EVENT_NAME=push REPOSITORY=o/r PATH="$FAKE_BIN:$PATH" \
  GH_LOG="$TEST_ROOT/push.log" sh "$SCRIPT")
[ "$actual" = 'v2026.8.1' ] || { echo 'FAIL: push did not return stable release' >&2; exit 1; }
expected_args=$(printf '%s\n' release list --repo o/r --exclude-drafts \
  --exclude-pre-releases --limit 1 --json tagName --jq '.[0].tagName // empty')
actual_args=$(cat "$TEST_ROOT/push.log")
[ "$actual_args" = "$expected_args" ] \
  || { echo 'FAIL: push release query arguments changed' >&2; exit 1; }

actual=$(EVENT_NAME=push REPOSITORY=o/r PATH="$FAKE_BIN:$PATH" GH_OUTPUT= \
  GH_LOG="$TEST_ROOT/no-release.log" sh "$SCRIPT")
[ -z "$actual" ] || { echo 'FAIL: empty release list produced a tag' >&2; exit 1; }

if EVENT_NAME=pull_request REPOSITORY=o/r PATH="$FAKE_BIN:$PATH" \
  GH_LOG="$TEST_ROOT/unsupported.log" sh "$SCRIPT" >/dev/null 2>&1; then
  echo 'FAIL: unsupported event succeeded' >&2
  exit 1
fi

echo 'dashboard-release-tag tests passed'
