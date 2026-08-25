#!/bin/sh
set -eu

ROOT=$(CDPATH= cd -- "$(dirname "$0")/../.." && pwd)
CHECK="$ROOT/scripts/check-agents-claude-pair.sh"
FIXTURE=$(mktemp -d "${TMPDIR:-/tmp}/librefang-agents-pair-test.XXXXXX")
trap 'rm -rf "$FIXTURE"' EXIT HUP INT TERM

git -C "$FIXTURE" init -q
touch "$FIXTURE/AGENTS.md"

mkdir -p "$FIXTURE/path with spaces" "$FIXTURE/glob*[name]" "$FIXTURE/nested/target/ignored"
touch "$FIXTURE/path with spaces/AGENTS.md"
ln -s ./AGENTS.md "$FIXTURE/path with spaces/CLAUDE.md"
touch "$FIXTURE/glob*[name]/AGENTS.md"
ln -s AGENTS.md "$FIXTURE/glob*[name]/CLAUDE.md"
touch "$FIXTURE/nested/target/ignored/AGENTS.md"

output=$(cd "$FIXTURE" && sh "$CHECK")
case "$output" in
    *"path with spaces/AGENTS.md"*) ;;
    *) echo "FAIL: path containing spaces was not checked" >&2; exit 1 ;;
esac
case "$output" in
    *"glob*[name]/AGENTS.md"*) ;;
    *) echo "FAIL: path containing glob characters was not checked" >&2; exit 1 ;;
esac
case "$output" in
    *"nested/target"*) echo "FAIL: nested target directory was scanned" >&2; exit 1 ;;
esac

mkdir -p "$FIXTURE/missing"
touch "$FIXTURE/missing/AGENTS.md"
if (cd "$FIXTURE" && sh "$CHECK" >/dev/null 2>&1); then
    echo "FAIL: missing CLAUDE.md was accepted" >&2
    exit 1
fi

ln -s AGENTS.md "$FIXTURE/missing/CLAUDE.md"
mkdir -p "$FIXTURE/wrong"
touch "$FIXTURE/wrong/AGENTS.md" "$FIXTURE/wrong/other.md"
ln -s other.md "$FIXTURE/wrong/CLAUDE.md"
if (cd "$FIXTURE" && sh "$CHECK" >/dev/null 2>&1); then
    echo "FAIL: link to a different file was accepted" >&2
    exit 1
fi

echo "OK: check-agents-claude-pair.sh"
