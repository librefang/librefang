#!/usr/bin/env bash
set -euo pipefail

ROOT=$(cd "$(dirname "$0")/../.." && pwd)
HOOK="$ROOT/scripts/hooks/pre-push"
SHA1=1111111111111111111111111111111111111111
ZERO40=0000000000000000000000000000000000000000
ZERO64=0000000000000000000000000000000000000000000000000000000000000000

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

if run_hook 'release stable' "refs/heads/release $SHA1 refs/heads/release $ZERO40" >/dev/null 2>&1; then
    echo "FAIL: configured protected branch was accepted" >&2
    exit 1
fi
run_hook 'release stable' "refs/heads/main $SHA1 refs/heads/main $ZERO40"

if run_config_hook "refs/heads/develop $SHA1 refs/heads/develop $ZERO40" >/dev/null 2>&1; then
    echo "FAIL: git-config protected branch was accepted" >&2
    exit 1
fi

echo "OK: pre-push-protected-branches.sh"
