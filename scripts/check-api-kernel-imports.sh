#!/usr/bin/env bash
# check-api-kernel-imports.sh — informational baseline for issue #3744.
#
# Reports how many `librefang_kernel::<internal>::*` references still live
# in `crates/librefang-api/src/` so progress on narrowing the API → kernel
# import surface is visible in PR diffs. Not a hard gate (yet) — once the
# count is driven to zero (or to the small set of approved facade modules),
# this will graduate to a cargo-deny `[bans]` rule. See the follow-up
# tracked under #3744.
#
# Excluded from the count:
#   * The `LibreFangKernel` root re-export — that's the kernel's public
#     entry-point used to construct `AppState` and is by design.
#   * Comments and doc-comments — match `://` and `://!` after the line
#     number prefix.
#
# Counted by design (intentionally NOT excluded):
#   * The thin re-export modules in `librefang-api/src/{approval,error,
#     mcp_oauth,trajectory,triggers,workflow}.rs`. Those are the
#     centralised facades; they show up in the count once each so the
#     facade boundary itself is auditable from this script's output.
#
# Usage:
#   scripts/check-api-kernel-imports.sh

set -euo pipefail

# Resolve repo root regardless of where the script is invoked from.
REPO_ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
SRC_DIR="${REPO_ROOT}/crates/librefang-api/src"

if [[ ! -d "${SRC_DIR}" ]]; then
    echo "error: ${SRC_DIR} not found — run from within the repo" >&2
    exit 2
fi

echo "Scanning: ${SRC_DIR}"
echo

# Prefer ripgrep when available; fall back to grep -R.
if command -v rg >/dev/null 2>&1; then
    SCAN=(rg -n 'librefang_kernel::' "${SRC_DIR}")
else
    SCAN=(grep -RIn 'librefang_kernel::' "${SRC_DIR}")
fi

# Strip comments and the LibreFangKernel root re-export.
"${SCAN[@]}" \
    | grep -v ':[0-9]*://' \
    | grep -v 'librefang_kernel::LibreFangKernel' \
    | sort \
    | tee /tmp/api-kernel-imports.txt

count=$(wc -l < /tmp/api-kernel-imports.txt | tr -d '[:space:]')

echo
echo "Total: ${count} non-comment refs to librefang_kernel::<internal> in librefang-api/src"
echo "(See issue #3744 for the migration plan.)"
