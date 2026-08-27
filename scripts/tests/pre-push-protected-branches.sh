#!/usr/bin/env bash
set -euo pipefail

ROOT=$(cd "$(dirname "$0")/../.." && pwd)
HOOK="$ROOT/scripts/hooks/pre-push"
SHA1=1111111111111111111111111111111111111111
SHA256=1111111111111111111111111111111111111111111111111111111111111111
ZERO40=0000000000000000000000000000000000000000
ZERO64=0000000000000000000000000000000000000000000000000000000000000000
ZERO39=000000000000000000000000000000000000000

run_hook() {
    local branches=$1 input=$2
    printf '%s\n' "$input" | LIBREFANG_PROTECTED_BRANCHES="$branches" "$HOOK"
}

run_config_hook() {
    local input=$1
    printf '%s\n' "$input" | env -u LIBREFANG_PROTECTED_BRANCHES \
        GIT_CONFIG_COUNT=1 \
        GIT_CONFIG_KEY_0=hook.protectedBranches \
        GIT_CONFIG_VALUE_0=develop \
        "$HOOK"
}

for zero in "$ZERO40" "$ZERO64"; do
    run_hook 'main master' "(delete) $zero refs/heads/main $SHA1"
done

if run_hook 'main master' "refs/heads/main $SHA1 refs/heads/main $ZERO40" >/dev/null 2>&1; then
    echo "FAIL: direct main push was accepted" >&2
    exit 1
fi

if run_hook 'main master' "refs/heads/main $SHA256 refs/heads/main $ZERO64" >/dev/null 2>&1; then
    echo "FAIL: direct main push with a SHA-256 object ID was accepted" >&2
    exit 1
fi

if run_hook 'main master' "refs/heads/main $ZERO39 refs/heads/main $ZERO40" >/dev/null 2>&1; then
    echo "FAIL: a noncanonical all-zero object ID bypassed protection" >&2
    exit 1
fi

if run_hook 'release stable' "refs/heads/release $SHA1 refs/heads/release $ZERO40" >/dev/null 2>&1; then
    echo "FAIL: configured protected branch was accepted" >&2
    exit 1
fi
run_hook 'release stable' "refs/heads/main $SHA1 refs/heads/main $ZERO40"

# Protected names are an exact whitespace-delimited list, not shell patterns.
# Cargo.toml exists in the repository and would be produced by expanding `*`.
run_hook '*' "refs/heads/Cargo.toml $SHA1 refs/heads/Cargo.toml $ZERO40"

run_hook 'release/2026' \
    "refs/heads/topic $SHA1 refs/tags/release/2026 $ZERO40"
if run_hook 'release/2026' \
    "refs/heads/topic $SHA1 refs/heads/release/2026 $ZERO40" >/dev/null 2>&1; then
    echo "FAIL: exact protected branch with a slash was accepted" >&2
    exit 1
fi

printf '%s\n' "refs/heads/main $SHA1 refs/heads/main $ZERO40" \
    | LIBREFANG_PREPUSH_SKIP=1 LIBREFANG_PROTECTED_BRANCHES='main' "$HOOK"

if run_config_hook "refs/heads/develop $SHA1 refs/heads/develop $ZERO40" >/dev/null 2>&1; then
    echo "FAIL: git-config protected branch was accepted" >&2
    exit 1
fi

echo "OK: pre-push-protected-branches.sh"
