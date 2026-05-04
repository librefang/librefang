#!/usr/bin/env bash
# refs #3596 — API → Kernel → Runtime layering progress check.
#
# `librefang-api` historically imported runtime types directly, bypassing
# kernel encapsulation. The intended layering is `API → Kernel → Runtime`;
# API code should reach runtime types through the kernel boundary.
#
# Migration is incremental: this script counts remaining direct
# `use librefang_runtime` occurrences in `crates/librefang-api/src/` so PR
# diffs make progress visible. It is informational only (does not fail) —
# a follow-up PR will delete the runtime dep line in
# `crates/librefang-api/Cargo.toml`, at which point the compiler enforces
# the boundary and this script becomes redundant.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
API_SRC="$ROOT/crates/librefang-api/src"

if ! command -v rg >/dev/null 2>&1; then
  echo "ripgrep (rg) not found; skipping API↔runtime decoupling check" >&2
  exit 0
fi

remaining=$(rg -c 'use librefang_runtime' "$API_SRC" 2>/dev/null | awk -F: '{s+=$2} END {print s+0}')

echo "[check-api-runtime-decoupling] direct \`use librefang_runtime\` in librefang-api/src: $remaining"
echo "[check-api-runtime-decoupling] target: 0 (then drop runtime dep from crates/librefang-api/Cargo.toml — refs #3596)"

if [ "$remaining" -gt 0 ]; then
  echo
  echo "Remaining import sites:"
  rg -n 'use librefang_runtime' "$API_SRC" | sed 's|^|  |'
fi

exit 0
