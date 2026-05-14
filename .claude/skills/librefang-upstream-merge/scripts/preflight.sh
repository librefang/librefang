#!/usr/bin/env bash
# preflight.sh — Phase 1 of the librefang-upstream-merge skill.
#
# Verifies safe operating conditions before any merge action:
#  1. Working dir is a linked git worktree (not the main tree).
#  2. Working tree is clean.
#  3. `upstream` remote is configured and points at librefang/librefang.
#  4. `git fetch upstream --no-tags` succeeds.
#  5. Reports the divergence count and per-commit log.
#
# Exit codes:
#   0 — all checks pass; safe to proceed to Phase 2 (merge).
#   1 — a check failed; the failure is printed on stderr.
#   2 — zero commits behind upstream (nothing to merge); exit early.

set -euo pipefail

fail() { echo "[preflight] FAIL: $*" >&2; exit 1; }
warn() { echo "[preflight] warn: $*" >&2; }
ok()   { echo "[preflight] ok:   $*"; }

# 1. Linked worktree check.
toplevel="$(git rev-parse --show-toplevel 2>/dev/null || true)"
[ -n "$toplevel" ] || fail "not inside a git repository"
if [ -d "$toplevel/.git" ]; then
  fail "running in MAIN worktree at $toplevel — create a linked worktree with 'git worktree add' first"
else
  ok "linked worktree at $toplevel"
fi

# 2. Working tree clean.
dirty="$(git status --porcelain)"
if [ -n "$dirty" ]; then
  echo "$dirty" >&2
  fail "working tree has uncommitted changes — stash or commit before merging"
fi
ok "working tree clean"

# 3. upstream remote configured.
upstream_url="$(git remote get-url upstream 2>/dev/null || true)"
if [ -z "$upstream_url" ]; then
  fail "no 'upstream' remote configured — add it: 'git remote add upstream https://github.com/librefang/librefang.git'"
fi
case "$upstream_url" in
  *librefang/librefang*) ok "upstream remote → $upstream_url" ;;
  *) warn "upstream remote → $upstream_url (does not contain 'librefang/librefang'; double-check before merging)" ;;
esac

# 4. Fetch upstream.
echo "[preflight] fetching upstream/main..."
if ! git fetch upstream --no-tags 2>&1 | tail -3; then
  fail "git fetch upstream failed — check network / SSH auth"
fi
ok "upstream fetched"

# 5. Divergence report.
behind="$(git rev-list --count HEAD..upstream/main 2>/dev/null || echo 0)"
ahead="$(git rev-list --count upstream/main..HEAD 2>/dev/null || echo 0)"
echo
echo "[preflight] divergence vs upstream/main:"
echo "  commits behind upstream: $behind"
echo "  commits ahead  upstream: $ahead"

if [ "$behind" = "0" ]; then
  echo "[preflight] nothing to merge — already up to date with upstream"
  exit 2
fi

echo
echo "[preflight] commits to merge:"
git log HEAD..upstream/main --oneline | head -40
total="$(git log HEAD..upstream/main --oneline | wc -l | tr -d ' ')"
if [ "$total" -gt 40 ]; then
  echo "  ... and $(( total - 40 )) more"
fi

echo
echo "[preflight] all checks passed — proceed to Phase 2 (merge)."
exit 0
