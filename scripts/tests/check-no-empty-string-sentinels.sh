#!/usr/bin/env bash
set -euo pipefail

ROOT=$(cd "$(dirname "$0")/../.." && pwd)
CHECK="$ROOT/scripts/check-no-empty-string-sentinels.sh"
WORK=$(mktemp -d "${TMPDIR:-/tmp}/librefang-sentinel-lint-test.XXXXXX")
trap 'rm -rf "$WORK"' EXIT
mkdir -p "$WORK/crates/librefang-api/src/routes/.hidden" \
    "$WORK/crates/librefang-channels/src" "$WORK/outside" "$WORK/target"

# Build a deterministic fallback PATH containing the tools the scanner needs,
# but deliberately no `rg`. Runner images may install ripgrep in /usr/bin, so
# merely narrowing PATH to /usr/bin:/bin does not guarantee grep coverage.
NO_RG_BIN="$WORK/no-rg-bin"
mkdir -p "$NO_RG_BIN"
for utility in grep tr wc; do
    ln -s "$(command -v "$utility")" "$NO_RG_BIN/$utility"
done

printf '%s\n' 'if value.is_empty() {}' \
    >"$WORK/crates/librefang-api/src/routes/soft.rs"
printf '%s\n' 'const TEXT: &str = "<NONE>";' \
    >"$WORK/crates/librefang-api/src/routes/.hidden/hard.rs"
printf '%s\n' 'const TEXT: &str = "<none>";' \
    >"$WORK/crates/librefang-api/src/routes/ignored.txt"
printf '%s\n' 'const TEXT: &str = "<none>";' >"$WORK/outside/symlink-target.rs"
ln -s "$WORK/outside" "$WORK/crates/librefang-channels/src/outside-link"

run_check() {
    local path_value=$1; shift
    PATH="$path_value" LIBREFANG_SENTINEL_ROOT="$WORK" /bin/bash "$CHECK" "$@"
}

for path_value in "$PATH" "$NO_RG_BIN"; do
    with_hard=$(run_check "$path_value")
    grep -Fq 'Hard sentinel hits: 1' <<<"$with_hard"
    grep -Fq 'Soft review-only hits: 1' <<<"$with_hard"
    if run_check "$path_value" --strict >/dev/null 2>&1; then
        echo "FAIL: strict mode accepted a hard hidden Rust sentinel" >&2
        exit 1
    fi
done

printf '%s\n' 'const TEXT: &str = "valid";' \
    >"$WORK/crates/librefang-api/src/routes/.hidden/hard.rs"
for path_value in "$PATH" "$NO_RG_BIN"; do
    soft_only=$(run_check "$path_value" --strict)
    grep -Fq 'Hard sentinel hits: 0' <<<"$soft_only"
    grep -Fq 'Soft review-only hits: 1' <<<"$soft_only"
done

help=$(run_check "$PATH" --help)
grep -Fq 'Strict mode fails' <<<"$help"
grep -Fq 'only on hard textual/default sentinels' <<<"$help"

echo "OK: check-no-empty-string-sentinels.sh"
