#!/bin/sh
# Verify that every AGENTS.md outside the repo root has a sibling
# `CLAUDE.md` that is a symlink to `AGENTS.md` (#3297).
#
# Why: AI tooling that doesn't recognise AGENTS.md (older Claude Code
# builds, Codex CLI variants) walks up looking for CLAUDE.md instead.
# A symlink keeps the two files in lockstep without bit-rotting copies.
#
# Exempt: the repo root itself, where AGENTS.md and CLAUDE.md are
# *separate* files by design (the latter carries Claude-Code-specific
# rules that don't belong in a portable AGENTS.md).

set -eu

REPO_ROOT="$(git rev-parse --show-toplevel 2>/dev/null || pwd)"
cd "$REPO_ROOT"

fail=0
agents_list=$(mktemp "${TMPDIR:-/tmp}/librefang-agents-pair.XXXXXX")
trap 'rm -f "$agents_list"' EXIT HUP INT TERM

# Keep the loop in the parent shell so `fail=1` remains visible. Prune
# generated/VCS directories by name at any depth. Repository paths cannot
# contain newlines under this collaboration contract, but may contain spaces.
find . \
    \( -type d \( -name .git -o -name target -o -name node_modules \) -prune \) -o \
    \( -type f -name AGENTS.md ! -path './AGENTS.md' -print \) \
    >"$agents_list" 2>/dev/null

while IFS= read -r agents; do
    dir=$(dirname "$agents")
    claude="$dir/CLAUDE.md"

    if [ ! -e "$claude" ] && [ ! -L "$claude" ]; then
        echo "::error file=$agents::AGENTS.md present without sibling CLAUDE.md (#3297). Run: ln -s AGENTS.md $claude"
        fail=1
        continue
    fi

    if [ ! -L "$claude" ]; then
        echo "::error file=$claude::CLAUDE.md exists but is not a symlink. Replace with: rm $claude && ln -s AGENTS.md $claude"
        fail=1
        continue
    fi

    target=$(readlink "$claude")
    if ! (cd "$dir" && [ CLAUDE.md -ef AGENTS.md ]); then
        echo "::error file=$claude::CLAUDE.md symlink points to '$target', expected the sibling AGENTS.md."
        fail=1
        continue
    fi

    echo "ok: $agents <-> $claude"
done <"$agents_list"

if [ "$fail" != "0" ]; then
    echo
    echo "Pair check failed. See errors above." >&2
    exit 1
fi
