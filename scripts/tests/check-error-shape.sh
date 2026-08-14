#!/usr/bin/env bash
set -euo pipefail

ROOT=$(cd "$(dirname "$0")/../.." && pwd)
CHECK="$ROOT/scripts/check-error-shape.sh"
WORK=$(mktemp -d "${TMPDIR:-/tmp}/librefang-error-shape-test.XXXXXX")
trap 'rm -rf "$WORK"' EXIT

git -C "$WORK" init -q
mkdir -p "$WORK/crates/librefang-api/src/routes"

run_check() {
  local path_value=$1
  (cd "$WORK" && PATH="$path_value" /bin/bash "$CHECK")
}

cat >"$WORK/crates/librefang-api/src/routes/clean.rs" <<'RS'
/// json!({ "detail": "documented legacy shape" })
const DATA: &str = r#"{ "status": "error" }"#;
let row = json!({ "id": 1, "status": "error" });
RS

default_output=$(run_check "$PATH")
grep -Fq 'OK: no new forbidden error shapes' <<<"$default_output"

fallback_output=$(run_check '/usr/bin:/bin')
grep -Fq 'OK: no new forbidden error shapes' <<<"$fallback_output"

mkdir -p "$WORK/bin"
cat >"$WORK/bin/rg" <<'SH'
#!/bin/sh
exit 2
SH
chmod +x "$WORK/bin/rg"
set +e
engine_failure=$(run_check "$WORK/bin:/usr/bin:/bin" 2>&1)
engine_status=$?
set -e
if [[ "$engine_status" != 2 || "$engine_failure" != *'search engine failed'* ]]; then
  echo "FAIL: search engine failure was silently treated as no matches" >&2
  exit 1
fi

cat >"$WORK/crates/librefang-api/src/routes/new.rs" <<'RS'
let response = json!({ "detail": reason });
let other = json!({ "status": "error", "message": reason });
RS
set +e
default_failure=$(run_check "$PATH" 2>&1)
default_status=$?
fallback_failure=$(run_check '/usr/bin:/bin' 2>&1)
fallback_status=$?
set -e
if [[ "$default_status" != 1 || "$fallback_status" != 1 ]]; then
  echo "FAIL: one of the search engines accepted forbidden wrappers" >&2
  exit 1
fi
for output in "$default_failure" "$fallback_failure"; do
  grep -Fq 'new.rs:1:' <<<"$output"
  grep -Fq 'new.rs:2:' <<<"$output"
done

# A legacy filename lookalike must not inherit providers.rs exemption.
mv "$WORK/crates/librefang-api/src/routes/new.rs" \
  "$WORK/crates/librefang-api/src/routes/providersXrs"
if run_check "$PATH" >/dev/null 2>&1; then
  echo "FAIL: regex-like legacy path lookalike was exempted" >&2
  exit 1
fi

echo "OK: check-error-shape.sh"
