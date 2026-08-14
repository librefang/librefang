#!/usr/bin/env bash
set -euo pipefail

ROOT=$(cd "$(dirname "$0")/../.." && pwd)
CHECK="$ROOT/scripts/check-api-runtime-decoupling.sh"
FIXTURE=$(mktemp -d "${TMPDIR:-/tmp}/librefang-api-decoupling-test.XXXXXX")
trap 'rm -rf "$FIXTURE"' EXIT

mkdir -p "$FIXTURE/crates/librefang-api/src" "$FIXTURE/crates/librefang-api/tests" "$FIXTURE/bin"
touch "$FIXTURE/crates/librefang-api/Cargo.toml"
cat >"$FIXTURE/bin/cargo" <<'SH'
#!/bin/sh
printf '%s\n' "${FAKE_CARGO_METADATA:?}"
SH
chmod +x "$FIXTURE/bin/cargo"

clean_metadata=$(printf '{"packages":[{"manifest_path":"%s","dependencies":[]}]}' \
  "$FIXTURE/crates/librefang-api/Cargo.toml")
dev_runtime_metadata=$(printf '{"packages":[{"manifest_path":"%s","dependencies":[{"name":"librefang-runtime","rename":"runtime_alias","kind":"dev","target":"cfg(unix)"}]}]}' \
  "$FIXTURE/crates/librefang-api/Cargo.toml")
normal_runtime_metadata=$(printf '{"packages":[{"manifest_path":"%s","dependencies":[{"name":"librefang-runtime","rename":"runtime_alias","kind":null,"target":"cfg(unix)"}]}]}' \
  "$FIXTURE/crates/librefang-api/Cargo.toml")

run_check() {
  PATH="$FIXTURE/bin:$PATH" \
    LIBREFANG_API_DECOUPLING_ROOT="$FIXTURE" \
    FAKE_CARGO_METADATA="$1" \
    bash "$CHECK"
}

printf '%s\n' '// librefang_runtime::DocOnly' '//! use librefang_runtime::Other' \
  >"$FIXTURE/crates/librefang-api/src/lib.rs"
run_check "$clean_metadata" >/dev/null
run_check "$dev_runtime_metadata" >/dev/null

if run_check "$normal_runtime_metadata" >/dev/null 2>&1; then
  echo "FAIL: parsed target-specific renamed normal dependency was accepted" >&2
  exit 1
fi

printf '%s\n' 'use librefang_runtime::AgentLoop;' \
  >"$FIXTURE/crates/librefang-api/src/lib.rs"
if run_check "$clean_metadata" >/dev/null 2>&1; then
  echo "FAIL: direct source import was accepted" >&2
  exit 1
fi

rm -rf "$FIXTURE/crates/librefang-api/tests"
if run_check "$clean_metadata" >/dev/null 2>&1; then
  echo "FAIL: missing scan directory was accepted" >&2
  exit 1
fi

echo "OK: check-api-runtime-decoupling.sh"
