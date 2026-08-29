#!/bin/sh
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/../../.." && pwd)
SCRIPT="$ROOT/.github/scripts/contributor-announcement.sh"
TEST_ROOT=$(mktemp -d)
trap 'rm -rf "$TEST_ROOT"' EXIT
trap 'exit 129' HUP
trap 'exit 130' INT
trap 'exit 143' TERM
FAKE_BIN="$TEST_ROOT/bin"
mkdir -p "$FAKE_BIN"

cat > "$FAKE_BIN/gh" <<'EOF'
#!/bin/sh
case "${ANNOUNCE_MODE:-first}" in
  first|discord_fail) printf '{"total_count":1,"incomplete_results":false,"items":[{"number":42}]}\n' ;;
  returning) printf '{"total_count":2,"incomplete_results":false,"items":[{"number":42},{"number":7}]}\n' ;;
  returning_unindexed) printf '{"total_count":1,"incomplete_results":false,"items":[{"number":7}]}\n' ;;
  incomplete) printf '{"total_count":0,"incomplete_results":true,"items":[]}\n' ;;
  empty_page) printf '{"total_count":1,"incomplete_results":false,"items":[]}\n' ;;
  malformed) printf '{"total_count":1,"incomplete_results":false,"items":[{"number":"42"}]}\n' ;;
  github_fail) exit 1 ;;
  *) exit 2 ;;
esac
printf '%s\n' "$*" > "${GH_ARGS:?}"
EOF
chmod +x "$FAKE_BIN/gh"

cat > "$FAKE_BIN/curl" <<'EOF'
#!/bin/sh
cat > "${CURL_PAYLOAD:?}"
printf '%s\n' "$*" > "${CURL_ARGS:?}"
[ "${ANNOUNCE_MODE:-}" != discord_fail ]
EOF
chmod +x "$FAKE_BIN/curl"

run_announcement() {
  mode=$1
  payload="$TEST_ROOT/$mode.json"
  ANNOUNCE_MODE="$mode" \
    CURL_PAYLOAD="$payload" \
    CURL_ARGS="$TEST_ROOT/$mode.curl-args" \
    GH_ARGS="$TEST_ROOT/$mode.gh-args" \
    DISCORD_WEBHOOK_URL='https://discord.test/webhook' \
    GH_TOKEN='test-token' \
    GITHUB_REPOSITORY='librefang/librefang' \
    PR_AUTHOR='contributor' \
    PR_TITLE="Improve **safety** [click](https://evil.test) @everyone \$(touch $TEST_ROOT/injected)" \
    PR_NUMBER='42' \
    PR_URL='https://github.test/pull/42' \
    PATH="$FAKE_BIN:$PATH" \
    bash "$SCRIPT"
}

run_announcement first
jq -e '
  .content | contains("first PR!\n\n**Improve \\*\\*safety\\*\\* \\[click\\]\\(https://evil.test\\) @everyone")
' "$TEST_ROOT/first.json" >/dev/null
jq -e '.allowed_mentions.parse == []' "$TEST_ROOT/first.json" >/dev/null
[ ! -e "$TEST_ROOT/injected" ] || { echo 'FAIL: PR title executed as shell' >&2; exit 1; }
grep -Fqx 'api --method GET search/issues -f q=repo:librefang/librefang author:contributor is:pr is:merged -F per_page=2' "$TEST_ROOT/first.gh-args"
grep -F -- '--fail-with-body --silent --show-error --max-time 20' "$TEST_ROOT/first.curl-args" >/dev/null
grep -F -- 'https://discord.test/webhook' "$TEST_ROOT/first.curl-args" >/dev/null

run_announcement returning
jq -e '.content | startswith("✅ **PR Merged:**")' "$TEST_ROOT/returning.json" >/dev/null
run_announcement returning_unindexed
jq -e '.content | startswith("✅ **PR Merged:**")' "$TEST_ROOT/returning_unindexed.json" >/dev/null

for mode in incomplete empty_page malformed github_fail discord_fail; do
  if run_announcement "$mode" >/dev/null 2>&1; then
    echo "FAIL: $mode should fail" >&2
    exit 1
  fi
done

if DISCORD_WEBHOOK_URL= PATH="$FAKE_BIN:$PATH" bash "$SCRIPT" >/dev/null 2>&1; then
  :
else
  echo 'FAIL: missing webhook should be a no-op' >&2
  exit 1
fi

echo 'contributor announcement tests passed'
