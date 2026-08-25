#!/usr/bin/env bash
# refs #3596 — API → Kernel → Runtime layering, compiler-enforced.
#
# Production `librefang-api` reaches runtime types through kernel-handle
# contracts. Integration tests may use a runtime dev-dependency to exercise
# real backends, but normal/build dependencies and direct references under
# `src/` remain forbidden. This script enforces that boundary from Cargo's
# parsed graph and Rust source.
#
# Failure means the layering invariant has regressed. Fix the diff,
# don't suppress this script.
set -euo pipefail

ROOT="${LIBREFANG_API_DECOUPLING_ROOT:-$(cd "$(dirname "$0")/.." && pwd)}"
API_TOML_REL="crates/librefang-api/Cargo.toml"
API_SRC_REL="crates/librefang-api/src"
API_TESTS_REL="crates/librefang-api/tests"
API_TOML="$ROOT/$API_TOML_REL"
API_SRC="$ROOT/$API_SRC_REL"
API_TESTS="$ROOT/$API_TESTS_REL"

fail=0

# 1. Inspect Cargo's parsed dependency graph. This catches table, dotted-key,
# target-specific, renamed, dev, and build dependency spellings. Dev-only
# runtime edges are permitted for integration tests; normal/build edges are not.
for required in "$API_TOML" "$API_SRC" "$API_TESTS"; do
  if [ ! -e "$required" ]; then
    relative=${required#"$ROOT"/}
    echo "::error file=$relative::required API decoupling scan target is missing."
    fail=1
  fi
done

if [ "$fail" = "0" ]; then
  metadata=$(cargo metadata --format-version 1 --no-deps --manifest-path "$API_TOML")
  runtime_dep=$(printf '%s' "$metadata" | python3 -c '
import json, os, sys
manifest = os.path.realpath(sys.argv[1])
data = json.load(sys.stdin)
package = next((p for p in data["packages"] if os.path.realpath(p["manifest_path"]) == manifest), None)
if package is None:
    raise SystemExit("librefang-api package missing from cargo metadata")
matches = [
    d for d in package["dependencies"]
    if d["name"] == "librefang-runtime" and d.get("kind") != "dev"
]
print("yes" if matches else "no")
' "$API_TOML")
else
  runtime_dep=no
fi

if [ "$runtime_dep" = "yes" ]; then
  echo "::error file=$API_TOML_REL::librefang-runtime dependency reintroduced. API → Kernel → Runtime layering (#3596) requires reaching runtime types through librefang-kernel re-exports."
  fail=1
fi

# 2. Production source imports must not name librefang_runtime directly.
# Use grep (not rg) to avoid a hard dep on ripgrep in CI; filter doc
# comments (lines that are pure /// or // mentions).
hits=$(grep -rEn 'use librefang_runtime|librefang_runtime::' \
        "$API_SRC" --include='*.rs' 2>/dev/null \
        | grep -vE '^[^:]+:[0-9]+:[[:space:]]*//' \
        || true)
if [ -n "$hits" ]; then
  echo "::error::direct librefang_runtime reference in librefang-api source (#3596 regression):"
  printf '%s\n' "$hits" | sed "s|$ROOT/||; s|^|  |"
  fail=1
fi

if [ "$fail" != "0" ]; then
  echo
  echo "API ↔ runtime decoupling regressed. See errors above." >&2
  exit 1
fi

echo "[check-api-runtime-decoupling] OK — librefang-api production code has no direct librefang_runtime edge."
exit 0
