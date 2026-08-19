#!/usr/bin/env bash
# Regression test for #5664 sub-finding 1: the pre-commit rustfmt
# pipeline must handle staged Rust files whose paths contain spaces and shell
# metacharacters without word-splitting.
#
# Strategy: build a throwaway git repo, stage a deliberately mis-formatted
# `with space.rs`, then invoke scripts/hooks/pre-commit and assert it
# rejects the commit. The old `$STAGED_RS` (unquoted) form would either
# treat the path as two separate files ("with" + "space.rs") and silently
# pass, or `rustfmt` would crash on a non-existent path — both are bugs.
#
set -euo pipefail

REPO_ROOT="$(git -C "$(dirname "$0")" rev-parse --show-toplevel)"
HOOK="$REPO_ROOT/scripts/hooks/pre-commit"
test -x "$HOOK" || { echo "FAIL: hook is not executable: $HOOK" >&2; exit 1; }

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
LOG="$WORK/hook.log"
RUSTFMT_ARGS="$WORK/rustfmt.args"
RUSTFMT_INPUT="$WORK/rustfmt.input"

cd "$WORK"
git init -q
git config user.email test@librefang.local
git config user.name test
git config commit.gpgsign false
printf '%s\n' 'edition = "2021"' >rustfmt.toml
git add rustfmt.toml
git commit -q -m config

# A fake rustfmt makes the test independent of the host toolchain and records
# exact argv so rejection cannot be confused with another hook stage.
mkdir -p "$WORK/bin"
cat >"$WORK/bin/rustfmt" <<'EOF'
#!/bin/sh
printf '%s\n' "$@" >"${RUSTFMT_ARGS:?}"
for arg; do input=$arg; done
cat "$input" >"${RUSTFMT_INPUT:?}"
exit 1
EOF
chmod +x "$WORK/bin/rustfmt"

FILE='with space $dollar[1].rs'
cat > "$FILE" <<'EOF'
fn main(   ) {println!("hi"  );}
EOF

git add -- "$FILE"
ln -s 'target.rs' 'zz staged link.rs'
git add -- 'zz staged link.rs'
cat > "$FILE" <<'EOF'
fn main() { println!("unstaged"); }
EOF

# Run the hook from inside the throwaway repo. We do NOT chdir into the
# real repo because the hook only inspects `git diff --cached`, which is
# bound to whichever index we're currently in.
set +e
PATH="$WORK/bin:$PATH" RUSTFMT_ARGS="$RUSTFMT_ARGS" \
    RUSTFMT_INPUT="$RUSTFMT_INPUT" "$HOOK" >"$LOG" 2>&1
rc=$?
set -e

if [ "$rc" -eq 0 ]; then
    echo "FAIL: pre-commit accepted a staged metacharacter path." >&2
    echo "----- hook output -----" >&2
    cat "$LOG" >&2
    exit 1
fi

RUSTFMT_PATH=$(tail -n 1 "$RUSTFMT_ARGS")
case "$RUSTFMT_PATH" in
    */"$FILE") ;;
    *)
    echo "FAIL: rustfmt did not receive the staged path as one exact argument." >&2
    printf 'recorded argv:\n' >&2
    cat "$RUSTFMT_ARGS" >&2
    exit 1
    ;;
esac

if ! grep -Fq 'fn main(   )' "$RUSTFMT_INPUT" \
    || grep -Fq 'unstaged' "$RUSTFMT_INPUT"; then
    echo "FAIL: rustfmt did not receive the exact staged Rust blob." >&2
    cat "$RUSTFMT_INPUT" >&2
    exit 1
fi

if grep -Fq 'zz staged link.rs' "$RUSTFMT_ARGS" \
    || grep -Fq 'target.rs' "$RUSTFMT_INPUT"; then
    echo "FAIL: rustfmt consumed a staged Rust-named symlink." >&2
    exit 1
fi

echo "PASS: pre-commit preserved the staged metacharacter path and blob"
