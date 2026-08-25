#!/usr/bin/env bash
# scripts/commit.sh — format staged Rust blobs, then commit (#3306).
#
# Why: the in-repo pre-commit hook (scripts/hooks/pre-commit) only *checks*
# rustfmt and rejects the commit when staged Rust files are dirty, leaving
# the contributor to retry by hand. This wrapper formats temporary copies of
# the index blobs, updates the index atomically, and only then invokes git
# commit. Unstaged working-tree edits are never formatted or staged.
#
# It also holds a soft lock against parallel commits in the same worktree
# (the user often has several `librefang-trees/<feature>` checkouts open at
# once; concurrent commits stomp on `.git/index.lock` and produce confusing
# half-aborted states).
#
# Usage:
#   scripts/commit.sh -m "feat: ..."
#   scripts/commit.sh -F path/to/msg.txt
#   scripts/commit.sh -m "fix: ..." --signoff
#
# All arguments are forwarded verbatim to `git commit` after staged formatting.
#
# Exit codes:
#   0   commit succeeded
#   2   another wrapper commit is in progress
#   3   rustfmt failed; staged set is unchanged
#   4   git commit itself failed (hooks, empty diff, signing, …)
#   5   staged blob extraction or atomic index update failed

set -euo pipefail

# ---------------------------------------------------------------------------
# Locate repo root via git itself — works from any subdirectory.
# ---------------------------------------------------------------------------
REPO_ROOT=$(git rev-parse --show-toplevel 2>/dev/null) || {
    echo "scripts/commit.sh: not inside a git working tree" >&2
    exit 1
}
GIT_DIR=$(git rev-parse --git-dir)
# `git rev-parse --git-dir` may return a relative path; resolve to absolute
# so the lock check is unambiguous when the script is invoked from a sub-dir.
case "$GIT_DIR" in
    /*) ;;
    *) GIT_DIR="$REPO_ROOT/$GIT_DIR" ;;
esac

# ---------------------------------------------------------------------------
# 1. Concurrent-commit guard.
#
# Use a wrapper-specific directory lock acquired atomically. Git's own
# index.lock protects each individual index operation; checking for its
# existence before later work would be a check-then-act race.
# ---------------------------------------------------------------------------
WRAPPER_LOCK="$GIT_DIR/librefang-commit.lock"
if ! mkdir "$WRAPPER_LOCK" 2>/dev/null; then
    echo "scripts/commit.sh: another git operation is in progress" >&2
    echo "  ($WRAPPER_LOCK exists)" >&2
    echo "  Wait for it to finish, or remove the lock manually if stale." >&2
    exit 2
fi
WORK_TMP=""
cleanup() {
    rmdir "$WRAPPER_LOCK" 2>/dev/null || true
    if [ -n "$WORK_TMP" ]; then
        rm -rf -- "$WORK_TMP" 2>/dev/null || true
    fi
}
trap cleanup EXIT
trap 'exit 129' HUP
trap 'exit 130' INT
trap 'exit 143' TERM
if ! WORK_TMP=$(mktemp -d "${TMPDIR:-/tmp}/librefang-commit.XXXXXX"); then
    echo "scripts/commit.sh: failed to create private formatting directory" >&2
    exit 5
fi

# ---------------------------------------------------------------------------
# 2. rustfmt on staged Rust blobs (only if rustfmt is on PATH).
#
# Extract index blobs to a private tree, format those copies, then update the
# index in one `git update-index --index-info` transaction. This preserves any
# unstaged edits and leaves the index unchanged if rustfmt fails.
# ---------------------------------------------------------------------------
FILES=()
STAGED_PATHS="$WORK_TMP/staged-rust"
if ! git diff --cached --name-only --diff-filter=ACMR -z -- '*.rs' >"$STAGED_PATHS"; then
    echo "scripts/commit.sh: failed to enumerate staged Rust files" >&2
    exit 5
fi
while IFS= read -r -d '' file; do
    FILES+=("$file")
done <"$STAGED_PATHS"

if [ "${#FILES[@]}" -gt 0 ]; then
    if command -v rustfmt >/dev/null 2>&1; then
        FORMAT_FILES=()
        FORMATTED=()
        MODES=()
        INDEX_INFO="$WORK_TMP/index-info"
        RUSTFMT_CONFIG="$WORK_TMP/rustfmt.toml"
        : >"$INDEX_INFO"
        for file in "${FILES[@]}"; do
            if ! entry=$(git ls-files -s -- "$file") || [ -z "$entry" ] || \
                [[ "$entry" == *$'\n'* ]]; then
                echo "scripts/commit.sh: failed to read index metadata: $file" >&2
                exit 5
            fi
            metadata=${entry%%$'\t'*}
            read -r mode _ stage extra <<<"$metadata"
            if [ -n "${extra:-}" ] || [ "$stage" != 0 ]; then
                echo "scripts/commit.sh: invalid index metadata: $file" >&2
                exit 5
            fi
            case "$mode" in
                100644|100755) ;;
                *) continue ;;
            esac
            staged_copy="$WORK_TMP/tree/$file"
            mkdir -p "$(dirname "$staged_copy")"
            if ! git show ":$file" >"$staged_copy"; then
                echo "scripts/commit.sh: failed to read staged blob: $file" >&2
                exit 5
            fi
            FORMAT_FILES+=("$file")
            FORMATTED+=("$staged_copy")
            MODES+=("$mode")
        done
        if [ "${#FORMATTED[@]}" -gt 0 ] && \
            ! git show :rustfmt.toml >"$RUSTFMT_CONFIG"; then
            echo "scripts/commit.sh: failed to read staged rustfmt.toml" >&2
            exit 5
        fi
        if [ "${#FORMATTED[@]}" -gt 0 ] && \
            ! rustfmt --edition 2021 --config-path "$RUSTFMT_CONFIG" \
                "${FORMATTED[@]}"; then
            echo "scripts/commit.sh: rustfmt failed; staged set unchanged" >&2
            exit 3
        fi
        for index in "${!FORMAT_FILES[@]}"; do
            file=${FORMAT_FILES[$index]}
            staged_copy=${FORMATTED[$index]}
            mode=${MODES[$index]}
            if ! blob=$(git hash-object -w "$staged_copy"); then
                echo "scripts/commit.sh: failed to write formatted blob: $file" >&2
                exit 5
            fi
            printf '%s %s\t%s\0' "$mode" "$blob" "$file" >>"$INDEX_INFO"
        done
        if ! git update-index -z --index-info <"$INDEX_INFO"; then
            echo "scripts/commit.sh: failed to update the staged Rust blobs" >&2
            exit 5
        fi
    else
        echo "scripts/commit.sh: rustfmt not found, skipping staged formatting" >&2
        echo "  (the pre-commit hook will still gate on rustfmt)" >&2
    fi
fi

# ---------------------------------------------------------------------------
# 3. Forward to git commit. All args are passed through unchanged so callers
# can use -m / -F / --signoff / --amend / etc. exactly as with raw git.
# ---------------------------------------------------------------------------
if ! git commit "$@"; then
    exit 4
fi
