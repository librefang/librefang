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
NEWLINE_FILE=$'path with\nnewline.rs'
SYMLINK_FILE='link.rs'
mkdir -p "$WORK/$(dirname "$FILE")"
cat >"$WORK/rustfmt.toml" <<'EOF'
edition = "2021"
use_field_init_shorthand = true
EOF
printf '%s\n' 'struct Item { value: i32 } fn staged(   ) { let value = 1; let _item = Item { value: value }; println!("staged"  );}' >"$WORK/$FILE"
printf '%s\n' 'fn newline_staged(   ) {}' >"$WORK/$NEWLINE_FILE"
ln -s 'target.rs' "$WORK/$SYMLINK_FILE"
git -C "$WORK" add -- rustfmt.toml "$FILE" "$NEWLINE_FILE" "$SYMLINK_FILE"
printf '%s\n' 'edition = "2021"' 'use_field_init_shorthand = false' >"$WORK/rustfmt.toml"
printf '%s\n' 'fn working(   ) {println!("unstaged"  );}' >"$WORK/$FILE"
printf '%s\n' 'fn newline_working(   ) {}' >"$WORK/$NEWLINE_FILE"

(cd "$WORK" && "$SCRIPT" -m 'test: commit staged blob')

committed=$(git -C "$WORK" show "HEAD:$FILE")
working=$(cat "$WORK/$FILE")
newline_committed=$(git -C "$WORK" show "HEAD:$NEWLINE_FILE")
case "$committed" in
    *'fn staged()'*'println!("staged");'*) ;;
    *) echo "FAIL: committed index blob was not formatted" >&2; exit 1 ;;
esac
case "$committed" in
    *'Item { value }'*) ;;
    *) echo "FAIL: staged rustfmt.toml was not applied" >&2; exit 1 ;;
esac
case "$committed" in
    *unstaged*) echo "FAIL: unstaged working content entered the commit" >&2; exit 1 ;;
esac
case "$working" in
    *'fn working(   )'*unstaged*) ;;
    *) echo "FAIL: unstaged working content was modified" >&2; exit 1 ;;
esac
case "$newline_committed" in
    *'fn newline_staged()'*) ;;
    *) echo "FAIL: newline-delimited staged path was not formatted" >&2; exit 1 ;;
esac
grep -Fq 'fn newline_working(   )' "$WORK/$NEWLINE_FILE" || {
    echo "FAIL: newline-delimited working path was modified" >&2
    exit 1
}
grep -Fq 'use_field_init_shorthand = false' "$WORK/rustfmt.toml" || {
    echo "FAIL: unstaged rustfmt configuration was modified" >&2
    exit 1
}
if [[ "$(git -C "$WORK" show "HEAD:$SYMLINK_FILE")" != target.rs || \
      "$(git -C "$WORK" ls-tree HEAD -- "$SYMLINK_FILE")" != 120000* ]]; then
    echo "FAIL: Rust-named symlink was changed or converted" >&2
    exit 1
fi

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
