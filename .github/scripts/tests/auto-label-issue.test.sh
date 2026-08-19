#!/bin/sh
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/../../.." && pwd)
SCRIPT="$ROOT/.github/scripts/auto-label-issue.sh"
TEST_ROOT=$(mktemp -d)
trap 'rm -rf "$TEST_ROOT"' EXIT HUP INT TERM

fail() {
  echo "FAIL: $*" >&2
  exit 1
}

assert_labels() {
  expected=$1
  title=$2
  body=$3
  actual=$(bash "$SCRIPT" 1 "$title" "$body") \
    || fail "labeler failed for title: $title"
  [ "$actual" = "$expected" ] \
    || fail "$title: expected '$expected', got '$actual'"
}

printf 'too short' > "$TEST_ROOT/short.txt"
cat > "$TEST_ROOT/non-english.txt" <<'EOF'
程序启动后立即退出。这里包含完整的环境、复现过程、预期行为和实际行为，足以让维护者开始调查问题。
EOF

assert_labels 'needs-info,needs-triage' 'error' "$TEST_ROOT/short.txt"
assert_labels 'enhancement' 'perf!: optimize the hot loop' "$TEST_ROOT/short.txt"
assert_labels 'needs-triage' 'debugger improvements' "$TEST_ROOT/short.txt"
assert_labels 'needs-triage' 'improve error handling' "$TEST_ROOT/short.txt"
assert_labels 'needs-triage' 'feature: retry on fail' "$TEST_ROOT/short.txt"
assert_labels 'needs-triage' 'error: 无法启动' "$TEST_ROOT/non-english.txt"
assert_labels 'needs-info,needs-triage' 'service crashes on startup' "$TEST_ROOT/short.txt"
assert_labels 'needs-info,needs-triage' 'request failed after retry' "$TEST_ROOT/short.txt"
assert_labels 'bug,needs-info' 'fix!: correct startup crash' "$TEST_ROOT/short.txt"
assert_labels 'enhancement' 'feat: improve error handling' "$TEST_ROOT/short.txt"
assert_labels 'enhancement' 'perf: avoid failed probes' "$TEST_ROOT/short.txt"
assert_labels 'area/docs' 'docs: explain error handling' "$TEST_ROOT/short.txt"
assert_labels '' 'chore: refresh generated metadata' "$TEST_ROOT/short.txt"
assert_labels '' 'refactor: rename internal helper' "$TEST_ROOT/short.txt"

echo 'auto-label-issue tests passed'
