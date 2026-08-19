#!/usr/bin/env bash
set -euo pipefail

ROOT=$(cd "$(dirname "$0")/../.." && pwd)
CHECK="$ROOT/scripts/check-error-shape.sh"
WORK=$(mktemp -d "${TMPDIR:-/tmp}/librefang-error-shape-test.XXXXXX")
trap 'rm -rf "$WORK"' EXIT

git -C "$WORK" init -q
mkdir -p "$WORK/crates/librefang-api/src/routes"

# Run once with the normal toolchain and once with only the declared runtime
# dependencies available.
NO_RG_BIN="$WORK/no-rg-bin"
mkdir -p "$NO_RG_BIN"
for utility in git grep python3; do
  ln -s "$(command -v "$utility")" "$NO_RG_BIN/$utility"
done

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

fallback_output=$(run_check "$NO_RG_BIN")
grep -Fq 'OK: no new forbidden error shapes' <<<"$fallback_output"

NO_PYTHON_BIN="$WORK/no-python-bin"
mkdir -p "$NO_PYTHON_BIN"
for utility in git grep; do
  ln -s "$(command -v "$utility")" "$NO_PYTHON_BIN/$utility"
done
set +e
missing_python=$(run_check "$NO_PYTHON_BIN" 2>&1)
missing_python_status=$?
set -e
if [[ "$missing_python_status" != 2 || "$missing_python" != *'python3 is required'* ]]; then
  echo "FAIL: a missing scanner dependency did not produce exit code 2" >&2
  exit 1
fi

cat >"$WORK/crates/librefang-api/src/routes/new.rs" <<'RS'
let response = json!({
    "detail": reason,
});
let other = json ! ({
    "status": "error",
    "message": reason,
});
RS
mkdir -p "$WORK/crates/librefang-api/src/routes/.hidden" \
  "$WORK/crates/librefang-api/src/routes/ignored" "$WORK/outside"
printf '%s\n' 'let hidden = json!({ "detail": reason });' \
  >"$WORK/crates/librefang-api/src/routes/.hidden/hidden.rs"
printf '%s\n' 'let ignored = json!({ "status": "error" });' \
  >"$WORK/crates/librefang-api/src/routes/ignored/ignored.rs"
printf '%s\n' 'let escaped = json!({ "detail": reason });' \
  >"$WORK/outside/symlink-target.rs"
printf '%s\n' 'crates/librefang-api/src/routes/ignored/' >"$WORK/.gitignore"
ln -s "$WORK/outside" "$WORK/crates/librefang-api/src/routes/outside-link"
set +e
default_failure=$(run_check "$PATH" 2>&1)
default_status=$?
fallback_failure=$(run_check "$NO_RG_BIN" 2>&1)
fallback_status=$?
set -e
if [[ "$default_status" != 1 || "$fallback_status" != 1 ]]; then
  echo "FAIL: one of the search engines accepted forbidden wrappers" >&2
  exit 1
fi
for output in "$default_failure" "$fallback_failure"; do
  grep -Fq 'new.rs:1:' <<<"$output"
  grep -Fq 'new.rs:4:' <<<"$output"
  grep -Fq '.hidden/hidden.rs:1:' <<<"$output"
  grep -Fq 'ignored/ignored.rs:1:' <<<"$output"
  if grep -Fq 'symlink-target.rs' <<<"$output"; then
    echo "FAIL: scanner followed a recursive symlink" >&2
    exit 1
  fi
done

# A same-named Rust file outside the exact legacy path must not inherit the
# providers.rs exemption.
rm -f "$WORK/crates/librefang-api/src/routes/.hidden/hidden.rs" \
  "$WORK/crates/librefang-api/src/routes/ignored/ignored.rs" \
  "$WORK/crates/librefang-api/src/routes/outside-link"
mv "$WORK/crates/librefang-api/src/routes/new.rs" \
  "$WORK/crates/librefang-api/src/routes/.hidden/providers.rs"
if run_check "$PATH" >/dev/null 2>&1; then
  echo "FAIL: same-named non-legacy path was exempted" >&2
  exit 1
fi

echo "OK: check-error-shape.sh"
