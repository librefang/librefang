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
#   * Comments and doc-comments — match `://` and `://!` after the line
#     number prefix.
#
# Counted by design (intentionally NOT excluded):
#   * The thin re-export modules in `librefang-api/src/{approval,error,
#     mcp_oauth,trajectory,triggers,workflow}.rs`. Those are the
#     centralised facades; they show up in the count once each so the
#     facade boundary itself is auditable from this script's output.
#
# Section 2 (hard gate, #3744): tracks direct `LibreFangKernel` type
# references in production (non-test) source.  Allowlisted sites:
#   - server.rs    — build_router / run_daemon take the concrete type because
#                    channel_bridge::start_channel_bridge still requires it.
#   - channel_bridge.rs — KernelBridgeAdapter + start_channel_bridge/
#                    start_channel_bridge_with_config need ~30 additional
#                    trait methods before they can be widened (tracked 2/N).
#   - routes/mod.rs — AppState.kernel field, same blocker as channel_bridge.
#   - routes/providers.rs — attach_probe_result needs model_catalog_update,
#                    not yet on the trait.
# Any file NOT in the allowlist that introduces a new direct LibreFangKernel
# reference will fail CI.
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

echo "=== Section 1: librefang_kernel::<internal> import surface ==="
echo "Scanning: ${SRC_DIR}"
echo

# Prefer ripgrep when available; fall back to grep -R.
if command -v rg >/dev/null 2>&1; then
    SCAN=(rg -n 'librefang_kernel::' "${SRC_DIR}")
    SCAN_LFK=(rg -n 'LibreFangKernel' "${SRC_DIR}")
else
    SCAN=(grep -RIn 'librefang_kernel::' "${SRC_DIR}")
    SCAN_LFK=(grep -RIn 'LibreFangKernel' "${SRC_DIR}" --include='*.rs')
fi

# Strip comments and the LibreFangKernel root re-export.
"${SCAN[@]}" \
    | grep -v ':[0-9]*://' \
    | sort \
    | tee /tmp/api-kernel-imports.txt

count=$(wc -l < /tmp/api-kernel-imports.txt | tr -d '[:space:]')

echo
echo "Total: ${count} non-comment refs to librefang_kernel::<internal> in librefang-api/src"
echo "(See issue #3744 for the migration plan.)"
echo

# ---------------------------------------------------------------------------
# Section 2 (hard gate): direct LibreFangKernel type references (#3744).
# Allowlisted files may retain the concrete type while widening is in progress.
# Any NEW file with a direct reference fails CI.
# ---------------------------------------------------------------------------
echo "=== Section 2: direct LibreFangKernel type references (hard gate #3744) ==="
echo

# Files explicitly allowlisted while widening is in progress (2/N).
ALLOWLIST=(
    "server.rs"
    "channel_bridge.rs"
    "routes/mod.rs"
    "routes/providers.rs"
)

fail=0

# Collect all non-comment lines referencing LibreFangKernel, skip test modules.
# grep -v ':[0-9]*:.*//.*LibreFangKernel' strips both leading `//` and trailing
# `// LibreFangKernel note` style comments that the older first-char-only
# pattern missed.
"${SCAN_LFK[@]}" \
    | grep -v ':[0-9]*:.*//.*LibreFangKernel' \
    | grep -v '#\[cfg(test' \
    | grep -v 'boot_with_config' \
    > /tmp/api-lfk-refs.txt 2>/dev/null || true

while IFS= read -r line; do
    # Extract filename relative to SRC_DIR.
    filepath="${line%%:*}"
    relpath="${filepath#${SRC_DIR}/}"

    # Check if this file is in the allowlist.
    allowed=0
    for allowed_file in "${ALLOWLIST[@]}"; do
        if [[ "${relpath}" == "${allowed_file}" ]]; then
            allowed=1
            break
        fi
    done

    if [[ "${allowed}" -eq 0 ]]; then
        echo "::error::Unexpected direct LibreFangKernel reference in ${relpath} (#3744 regression):"
        echo "  ${line}"
        fail=1
    fi
done < /tmp/api-lfk-refs.txt

if [[ "${fail}" -eq 1 ]]; then
    echo
    echo "LibreFangKernel concrete-type leak detected outside allowlist." >&2
    echo "Narrow the call site to a trait method or add it to the allowlist" >&2
    echo "in scripts/check-api-kernel-imports.sh with a comment explaining why." >&2
    exit 1
fi

echo "OK — all direct LibreFangKernel references are in the allowlisted files."
echo "(Allowlist: ${ALLOWLIST[*]})"
