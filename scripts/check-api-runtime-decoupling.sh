#!/usr/bin/env bash
# refs #3596 — API → Kernel → Runtime layering, compiler-enforced.
#
# As of 2/N (#3596), `librefang-api` no longer declares a direct
# `librefang-runtime` dependency in `Cargo.toml` — `cargo check`
# rejects any new `use librefang_runtime::*` outright. This script
# is now a defence-in-depth guard against re-introducing the dep:
#   1. asserts the `librefang-runtime = { path = ... }` line is gone
#      from `crates/librefang-api/Cargo.toml`,
#   2. asserts no `use librefang_runtime::*` slipped back into
#      `crates/librefang-api/{src,tests}/` via a path-style import.
#
# Failure means the layering invariant has regressed. Fix the diff,
# don't suppress this script.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
API_TOML="$ROOT/crates/librefang-api/Cargo.toml"
API_SRC="$ROOT/crates/librefang-api/src"
API_TESTS="$ROOT/crates/librefang-api/tests"

fail=0

# 1. Cargo.toml must not declare a direct runtime dep. Anchor the
# regex on a real dep-line (key = value), so doc / comment mentions
# of the crate name don't false-trip the check.
if grep -E '^[[:space:]]*librefang-runtime[[:space:]]*=' "$API_TOML" >/dev/null 2>&1; then
  echo "::error file=$API_TOML::librefang-runtime dependency reintroduced. API → Kernel → Runtime layering (#3596) requires reaching runtime types through librefang-kernel re-exports."
  fail=1
fi

# 2. Source / test imports must not name librefang_runtime directly.
# Use grep (not rg) to avoid a hard dep on ripgrep in CI; filter doc
# comments (lines that are pure /// or // mentions).
hits=$(grep -rEn 'use librefang_runtime|librefang_runtime::' \
        "$API_SRC" "$API_TESTS" --include='*.rs' 2>/dev/null \
        | grep -vE '^[^:]+:[0-9]+:[[:space:]]*///' \
        || true)
if [ -n "$hits" ]; then
  echo "::error::direct librefang_runtime reference in librefang-api source (#3596 regression):"
  printf '%s\n' "$hits" | sed 's|^|  |'
  fail=1
fi

if [ "$fail" != "0" ]; then
  echo
  echo "API ↔ runtime decoupling regressed. See errors above." >&2
  exit 1
fi

echo "[check-api-runtime-decoupling] OK — librefang-api has no direct librefang_runtime imports or Cargo dep."
exit 0
