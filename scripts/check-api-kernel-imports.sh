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
#   - acp_uds.rs / acp_pipe.rs — the ACP transport adapter still takes the
#                    concrete kernel on Unix and Windows.
#   - routes/agents/config.rs — the inert-tool diagnostic shares the kernel's
#                    concrete self-evolution-tool predicate.
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

TMP_DIR="$(mktemp -d)"
TMP_IMPORTS="${TMP_DIR}/imports"
TMP_LFK="${TMP_DIR}/lfk-refs"
cleanup() {
    rm -f -- "${TMP_IMPORTS}" "${TMP_LFK}"
    rmdir -- "${TMP_DIR}"
}
trap cleanup EXIT
: > "${TMP_IMPORTS}"
: > "${TMP_LFK}"

echo "=== Section 1: librefang_kernel::<internal> import surface ==="
echo "Scanning: ${SRC_DIR}"
echo

# Prefer ripgrep when available; fall back to grep -R.
if command -v rg >/dev/null 2>&1; then
    SCAN=(rg -n --type rust 'librefang_kernel::' "${SRC_DIR}")
    SCAN_LFK=(rg -n --type rust 'LibreFangKernel' "${SRC_DIR}")
else
    SCAN=(grep -RIn 'librefang_kernel::' "${SRC_DIR}" --include='*.rs')
    SCAN_LFK=(grep -RIn 'LibreFangKernel' "${SRC_DIR}" --include='*.rs')
fi

# Strip line-comment tails, then retain only code that still contains the
# internal path. Both the scanner and grep legitimately return 1 when the
# migration reaches zero references; those empty stages are success.
{ "${SCAN[@]}" || true; } \
    | sed 's|//.*$||' \
    | { grep 'librefang_kernel::' || true; } \
    | sort \
    | tee "${TMP_IMPORTS}"

count=$(wc -l < "${TMP_IMPORTS}" | tr -d '[:space:]')

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

# Files explicitly allowlisted while widening is in progress.
ALLOWLIST=(
    "server.rs"
    "channel_bridge.rs"
    "routes/mod.rs"
    "routes/providers.rs"
    "acp_uds.rs"
    "acp_pipe.rs"
    "routes/agents/config.rs"
)

fail=0

# Collect every non-comment line referencing LibreFangKernel. Test-only callers
# use `routes::boot_test_kernel`, so this gate needs no ad-hoc Rust parser or
# test-scope exclusion that could accidentally hide production references.
#
# Stripping comments via `sed 's|//.*$||'` rather than `grep -v '//.*LibreFangKernel'`:
# the latter would also drop legitimate production lines that happen to carry
# a trailing `// ...LibreFangKernel...` comment (e.g.
# `pub fn boot_app(k: LibreFangKernel) -> AppState  // wraps LibreFangKernel`),
# silently letting concrete-type leaks bypass the gate.  Stripping the
# `//` tail first then re-grepping for the bare identifier catches both
# leading and trailing comment forms while keeping production code in the
# scan.  Standard caveat: `//` inside string literals would be stripped
# too, but no such literal currently mentions `LibreFangKernel` and
# adding one would deserve a code review anyway.
"${SCAN_LFK[@]}" \
    | sed 's|//.*$||' \
    | grep 'LibreFangKernel' \
    > "${TMP_LFK}" 2>/dev/null || true

while IFS= read -r line; do
    # Extract filename relative to SRC_DIR.
    filepath="${line%%:*}"
    relpath="${filepath#"${SRC_DIR}"/}"

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
done < "${TMP_LFK}"

if [[ "${fail}" -eq 1 ]]; then
    echo
    echo "LibreFangKernel concrete-type leak detected outside allowlist." >&2
    echo "Narrow the call site to a trait method or add it to the allowlist" >&2
    echo "in scripts/check-api-kernel-imports.sh with a comment explaining why." >&2
    exit 1
fi

echo "OK — all direct LibreFangKernel references are in the allowlisted files."
echo "(Allowlist: ${ALLOWLIST[*]})"
