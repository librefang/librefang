#!/usr/bin/env bash
# Regression test for #5664 sub-finding 1: the pre-commit rustfmt
# pipeline must handle staged Rust files whose paths contain spaces
# (or other shell metacharacters) without word-splitting.
#
# Strategy: build a throwaway git repo, stage a deliberately mis-formatted
# `with space.rs`, then invoke scripts/hooks/pre-commit and assert it
# rejects the commit. The old `$STAGED_RS` (unquoted) form would either
# treat the path as two separate files ("with" + "space.rs") and silently
# pass, or `rustfmt` would crash on a non-existent path — both are bugs.
#
# Skips cleanly if `rustfmt` is not on PATH.

set -euo pipefail

if ! command -v rustfmt >/dev/null 2>&1; then
    echo "SKIP: rustfmt not installed" >&2
    exit 0
fi

REPO_ROOT="$(git -C "$(dirname "$0")" rev-parse --show-toplevel)"
HOOK="$REPO_ROOT/scripts/hooks/pre-commit"
test -x "$HOOK" || chmod +x "$HOOK"

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

cd "$WORK"
git init -q
git config user.email test@librefang.local
git config user.name test
git config commit.gpgsign false

# Deliberately mis-formatted Rust: extra spaces inside the fn signature
# and around the brace so `rustfmt --check` will refuse.
cat > "with space.rs" <<'EOF'
fn main(   ) {println!("hi"  );}
EOF

git add -- "with space.rs"

# Run the hook from inside the throwaway repo. We do NOT chdir into the
# real repo because the hook only inspects `git diff --cached`, which is
# bound to whichever index we're currently in.
set +e
"$HOOK" >/tmp/precommit-spaces-out.log 2>&1
rc=$?
set -e

if [ "$rc" -eq 0 ]; then
    echo "FAIL: pre-commit accepted a mis-formatted 'with space.rs'." >&2
    echo "----- hook output -----" >&2
    cat /tmp/precommit-spaces-out.log >&2
    exit 1
fi

if ! grep -q "not rustfmt-clean" /tmp/precommit-spaces-out.log; then
    echo "FAIL: pre-commit rejected but not via the rustfmt path." >&2
    echo "----- hook output -----" >&2
    cat /tmp/precommit-spaces-out.log >&2
    exit 1
fi

echo "PASS: pre-commit correctly rejected mis-formatted 'with space.rs'"
