#!/usr/bin/env bash
# scripts/commit.sh — wrap `cargo fmt → git add → git commit` (#3306).
#
# Why: the in-repo pre-commit hook (scripts/hooks/pre-commit) only *checks*
# rustfmt and rejects the commit when staged Rust files are dirty, leaving
# the contributor to retry by hand. This wrapper formats first, re-stages
# the same files, and only then invokes git commit — saving a manual round
# trip.
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
# All arguments are forwarded verbatim to `git commit` after fmt + re-stage.
#
# Exit codes:
#   0   commit succeeded
#   2   another commit is in progress (index.lock held)
#   3   cargo fmt failed (rustfmt errors); staged set is unchanged
#   4   git commit itself failed (hooks, empty diff, signing, …)

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
# `.git/index.lock` is git's own atomicity primitive — it always exists for
# the duration of any index-mutating operation. If we see it, another commit
# (CLI session, IDE, parallel agent) is already running and our `git add`
# would either block or race. Bail out loud rather than silently waiting.
# ---------------------------------------------------------------------------
if [ -e "$GIT_DIR/index.lock" ]; then
    echo "scripts/commit.sh: another git operation is in progress" >&2
    echo "  ($GIT_DIR/index.lock exists)" >&2
    echo "  Wait for it to finish, or remove the lock manually if stale." >&2
    exit 2
fi

# ---------------------------------------------------------------------------
# 2. cargo fmt on staged Rust files (only if cargo is on PATH).
#
# We do not run a full-workspace fmt — that would touch unstaged files and
# blow the contributor's working tree open. We mirror the pre-commit hook
# scope: ACMR-staged *.rs files only. If cargo is missing (e.g. running
# inside a CI container without rust), warn once and skip.
# ---------------------------------------------------------------------------
STAGED_RS=$(git diff --cached --name-only --diff-filter=ACMR -- '*.rs' || true)

if [ -n "$STAGED_RS" ]; then
    if command -v cargo >/dev/null 2>&1; then
        # Read the staged files list into an array for safe quoting.
        # shellcheck disable=SC2206 # word-splitting on \n is intentional
        FILES=($STAGED_RS)
        if ! cargo fmt -- "${FILES[@]}"; then
            echo "scripts/commit.sh: cargo fmt failed; staged set unchanged" >&2
            exit 3
        fi
        # Re-add the same files so any reformatting lands in the commit.
        git add -- "${FILES[@]}"
    else
        echo "scripts/commit.sh: cargo not found, skipping rustfmt" >&2
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
