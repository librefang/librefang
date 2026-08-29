#!/usr/bin/env bash
# Regression test for #5664 sub-finding 2: the openapi-sha baseline
# sync in pre-commit must still run on Linux dev boxes where only
# `sha256sum` exists (no `shasum`). The previous version hard-required
# `shasum` and skipped silently otherwise — meaning a stale baseline
# could land and the openapi-drift CI gate would reject the commit
# after the fact.
#
# Strategy: build a throwaway git repo that mimics the openapi.json
# + xtask/baselines/openapi.sha256 layout, stage a modified
# openapi.json, run the hook with `shasum` masked out of PATH, and
# assert the baseline file was auto-staged with the new digest.

set -euo pipefail

if ! command -v sha256sum >/dev/null 2>&1; then
    echo "SKIP: sha256sum not installed; can't exercise the fallback" >&2
    exit 0
fi

REPO_ROOT="$(git -C "$(dirname "$0")" rev-parse --show-toplevel)"
HOOK="$REPO_ROOT/scripts/hooks/pre-commit"
test -x "$HOOK" || { echo "FAIL: hook is not executable: $HOOK" >&2; exit 1; }

WORK="$(mktemp -d)"
SHIM_DIR="$(mktemp -d)"
trap 'rm -rf "$WORK" "$SHIM_DIR"' EXIT
LOG="$WORK/hook.log"

cd "$WORK"
git init -q
git config user.email test@librefang.local
git config user.name test
git config commit.gpgsign false

mkdir -p xtask/baselines

# Seed baseline with a wrong digest so the hook MUST rewrite it.
echo "0000000000000000000000000000000000000000000000000000000000000000  openapi.json" \
    > xtask/baselines/openapi.sha256

# Initial commit: empty openapi + wrong baseline. Stage so the next
# step's `git diff --cached` actually has openapi.json in it.
printf '{}' > openapi.json
git add openapi.json xtask/baselines/openapi.sha256
git commit -q -m "init"

# Modify openapi.json and stage it.
printf '{"info":{"version":"0.0.1"}}' > openapi.json
git add openapi.json

EXPECTED=$(sha256sum openapi.json | awk '{print $1}')
# Remove the working-tree post-image. The hook must hash the index blob and
# must not skip the check merely because the staged file is absent on disk.
rm openapi.json

# Mask shasum out of PATH so the hook MUST use sha256sum via the shim.
cat > "$SHIM_DIR/shasum" <<'EOF'
#!/bin/sh
echo "shasum: deliberately disabled by pre-commit-sha-fallback.sh" >&2
exit 127
EOF
cat > "$SHIM_DIR/gitleaks" <<'EOF'
#!/bin/sh
exit 0
EOF
chmod +x "$SHIM_DIR/shasum" "$SHIM_DIR/gitleaks"

# Sanity: confirm masking works (PATH-prefix wins).
PATH="$SHIM_DIR:$PATH"
if shasum -a 256 openapi.json >/dev/null 2>&1; then
    echo "FAIL: shasum mask is not active (got real shasum output)" >&2
    exit 1
fi

set +e
"$HOOK" >"$LOG" 2>&1
rc=$?
set -e

if [ "$rc" -ne 0 ]; then
    echo "FAIL: pre-commit errored under sha256sum-only PATH (rc=$rc)" >&2
    echo "----- hook output -----" >&2
    cat "$LOG" >&2
    exit 1
fi

RECORDED=$(awk '{print $1}' xtask/baselines/openapi.sha256)
if [ "$RECORDED" != "$EXPECTED" ]; then
    echo "FAIL: baseline was not updated under sha256sum-only PATH." >&2
    echo "  expected: $EXPECTED" >&2
    echo "  recorded: $RECORDED" >&2
    echo "----- hook output -----" >&2
    cat "$LOG" >&2
    exit 1
fi

if ! git diff --cached --name-only | grep -qx "xtask/baselines/openapi.sha256"; then
    echo "FAIL: refreshed baseline was not auto-staged." >&2
    exit 1
fi

if [ "$(git show :openapi.json)" != '{"info":{"version":"0.0.1"}}' ]; then
    echo "FAIL: hook changed the staged openapi.json blob." >&2
    exit 1
fi

if [ -e openapi.json ]; then
    echo "FAIL: hook recreated the deliberately absent working-tree post-image." >&2
    exit 1
fi

cat >"$SHIM_DIR/python3" <<'EOF'
#!/bin/sh
exit 0
EOF
chmod +x "$SHIM_DIR/python3"
printf '%s\n' '## [Unreleased]' >CHANGELOG.md
git add CHANGELOG.md
printf '%s\n' '## [Unreleased]' '' '## [Unreleased]' >CHANGELOG.md
if ! "$HOOK" >"$WORK/changelog.log" 2>&1; then
    echo "FAIL: hook inspected the unstaged CHANGELOG.md post-image." >&2
    cat "$WORK/changelog.log" >&2
    exit 1
fi

git commit -q -m sync
git restore openapi.json xtask/baselines/openapi.sha256
printf '{"info":{"version":"0.0.2"}}' >openapi.json
git add openapi.json
printf '%s\n' 'preserve this unstaged baseline edit' \
    >xtask/baselines/openapi.sha256
if "$HOOK" >"$WORK/baseline-dirty.log" 2>&1; then
    echo "FAIL: hook overwrote an unstaged OpenAPI baseline edit." >&2
    exit 1
fi
if ! grep -Fq 'has unstaged changes; refusing to overwrite' \
    "$WORK/baseline-dirty.log"; then
    echo "FAIL: hook rejected dirty baseline for the wrong reason." >&2
    cat "$WORK/baseline-dirty.log" >&2
    exit 1
fi
if ! grep -Fxq 'preserve this unstaged baseline edit' \
    xtask/baselines/openapi.sha256; then
    echo "FAIL: hook changed the unstaged OpenAPI baseline edit." >&2
    exit 1
fi

echo "PASS: pre-commit consumed staged OpenAPI and CHANGELOG content safely"
