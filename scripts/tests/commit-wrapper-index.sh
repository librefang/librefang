#!/usr/bin/env bash
set -euo pipefail

ROOT=$(cd "$(dirname "$0")/../.." && pwd)
SCRIPT="$ROOT/scripts/commit.sh"
WORK=$(mktemp -d "${TMPDIR:-/tmp}/librefang-commit-wrapper-test.XXXXXX")
trap 'rm -rf "$WORK"' EXIT

git -C "$WORK" init -q
git -C "$WORK" config user.email test@librefang.local
git -C "$WORK" config user.name test
git -C "$WORK" config commit.gpgsign false

FILE='path with $meta[1].rs'
mkdir -p "$WORK/$(dirname "$FILE")"
printf '%s\n' 'fn staged(   ) {println!("staged"  );}' >"$WORK/$FILE"
git -C "$WORK" add -- "$FILE"
printf '%s\n' 'fn working(   ) {println!("unstaged"  );}' >"$WORK/$FILE"

(cd "$WORK" && "$SCRIPT" -m 'test: commit staged blob')

committed=$(git -C "$WORK" show "HEAD:$FILE")
working=$(cat "$WORK/$FILE")
case "$committed" in
    *'fn staged()'*'println!("staged");'*) ;;
    *) echo "FAIL: committed index blob was not formatted" >&2; exit 1 ;;
esac
case "$committed" in
    *unstaged*) echo "FAIL: unstaged working content entered the commit" >&2; exit 1 ;;
esac
case "$working" in
    *'fn working(   )'*unstaged*) ;;
    *) echo "FAIL: unstaged working content was modified" >&2; exit 1 ;;
esac

printf '%s\n' 'fn next(   ) {}' >"$WORK/$FILE"
git -C "$WORK" add -- "$FILE"
before_failure=$(git -C "$WORK" rev-parse ":$FILE")
printf '%s\n' 'fn still_working(   ) {}' >"$WORK/$FILE"
mkdir -p "$WORK/bin"
cat >"$WORK/bin/rustfmt" <<'EOF'
#!/bin/sh
exit 1
EOF
chmod +x "$WORK/bin/rustfmt"
set +e
(cd "$WORK" && PATH="$WORK/bin:$PATH" "$SCRIPT" -m 'test: fmt fails') >/dev/null 2>&1
fmt_status=$?
set -e
after_failure=$(git -C "$WORK" rev-parse ":$FILE")
if [[ "$fmt_status" != 3 || "$after_failure" != "$before_failure" ]]; then
    echo "FAIL: rustfmt failure changed the index or returned the wrong status" >&2
    exit 1
fi
grep -Fq 'fn still_working(   )' "$WORK/$FILE" || {
    echo "FAIL: rustfmt failure changed the working file" >&2
    exit 1
}

# A setup failure after lock acquisition must not strand the wrapper lock.
MISSING_TMP="$WORK/missing-tmp-root"
set +e
(cd "$WORK" && TMPDIR="$MISSING_TMP" "$SCRIPT" -m 'test: temp setup fails') \
    >/dev/null 2>&1
tmp_status=$?
set -e
if [[ "$tmp_status" != 5 || -e "$WORK/.git/librefang-commit.lock" ]]; then
    echo "FAIL: temp setup failure returned the wrong status or stranded the lock" >&2
    exit 1
fi

mkdir "$WORK/.git/librefang-commit.lock"
set +e
lock_output=$(cd "$WORK" && "$SCRIPT" -m 'test: blocked' 2>&1)
lock_status=$?
set -e
rmdir "$WORK/.git/librefang-commit.lock"
if [[ "$lock_status" != 2 || "$lock_output" != *'another git operation'* ]]; then
    echo "FAIL: atomic wrapper lock did not return exit 2" >&2
    exit 1
fi

echo "OK: commit-wrapper-index.sh"
