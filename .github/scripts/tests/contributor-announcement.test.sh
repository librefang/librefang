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
  first|discord_fail) printf '{"incomplete_results":false,"items":[{"number":42}]}\n' ;;
  returning) printf '{"incomplete_results":false,"items":[{"number":42},{"number":7}]}\n' ;;
  returning_unindexed) printf '{"incomplete_results":false,"items":[{"number":7}]}\n' ;;
  incomplete) printf '{"incomplete_results":true,"items":[]}\n' ;;
  github_fail) exit 1 ;;
  *) exit 2 ;;
esac
EOF
chmod +x "$FAKE_BIN/gh"

cat > "$FAKE_BIN/curl" <<'EOF'
#!/bin/sh
cat > "${CURL_PAYLOAD:?}"
[ "${ANNOUNCE_MODE:-}" != discord_fail ]
EOF
chmod +x "$FAKE_BIN/curl"

run_announcement() {
  mode=$1
  payload="$TEST_ROOT/$mode.json"
  ANNOUNCE_MODE="$mode" \
    CURL_PAYLOAD="$payload" \
    DISCORD_WEBHOOK_URL='https://discord.test/webhook' \
    GH_TOKEN='test-token' \
    GITHUB_REPOSITORY='librefang/librefang' \
    PR_AUTHOR='contributor' \
    PR_TITLE="Improve safety @everyone \$(touch $TEST_ROOT/injected)" \
    PR_NUMBER='42' \
    PR_URL='https://github.test/pull/42' \
    PATH="$FAKE_BIN:$PATH" \
    bash "$SCRIPT"
}

run_announcement first
jq -e '
  .content | contains("first PR!\n\n**Improve safety @everyone")
' "$TEST_ROOT/first.json" >/dev/null
jq -e '.allowed_mentions.parse == []' "$TEST_ROOT/first.json" >/dev/null
[ ! -e "$TEST_ROOT/injected" ] || { echo 'FAIL: PR title executed as shell' >&2; exit 1; }

run_announcement returning
jq -e '.content | startswith("✅ **PR Merged:**")' "$TEST_ROOT/returning.json" >/dev/null
run_announcement returning_unindexed
jq -e '.content | startswith("✅ **PR Merged:**")' "$TEST_ROOT/returning_unindexed.json" >/dev/null

for mode in incomplete github_fail discord_fail; do
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
