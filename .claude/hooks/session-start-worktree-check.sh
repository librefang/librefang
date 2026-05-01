#!/usr/bin/env bash
# SessionStart hook: emit a banner into the model's context noting whether the
# session is starting in the librefang main worktree (where edits will be
# blocked by forbid-main-worktree.sh) or in a linked worktree.
#
# Hook protocol: read JSON from stdin, write `additionalContext` text to
# stdout via {"hookSpecificOutput":{"hookEventName":"SessionStart",...}}.

set -euo pipefail

input="$(cat)"
cwd="$(printf '%s' "$input" | python3 -c 'import sys,json; print(json.load(sys.stdin).get("cwd",""))' 2>/dev/null || true)"
[ -n "$cwd" ] || { echo '{}'; exit 0; }

real_cwd="$(cd "$cwd" 2>/dev/null && pwd -P || true)"
[ -n "$real_cwd" ] || { echo '{}'; exit 0; }

dir="$real_cwd"
git_kind=""; repo_root=""
while [ "$dir" != "/" ] && [ -n "$dir" ]; do
  if [ -e "$dir/.git" ]; then
    repo_root="$dir"
    if [ -d "$dir/.git" ]; then git_kind="main"; else git_kind="worktree"; fi
    break
  fi
  dir="$(dirname "$dir")"
done
[ -n "$repo_root" ] || { echo '{}'; exit 0; }

# Find the *main* worktree path (first entry of `git worktree list`) so we can
# tell whether this session is inside the librefang repo regardless of whether
# we are in the main tree or a linked one.
main_root="$(git -C "$repo_root" worktree list --porcelain 2>/dev/null \
  | awk '/^worktree / {print $2; exit}')"
[ -n "$main_root" ] || main_root="$repo_root"

case "$main_root" in
  */librefang) ;;
  *) echo '{}'; exit 0 ;;
esac

if [ "$git_kind" = "main" ]; then
  msg="⚠️  Session starting in the librefang MAIN WORKTREE ($repo_root). Edits and mutating git commands here are blocked by .claude/hooks/forbid-main-worktree.sh. For any task that will modify files, FIRST run: git worktree add /tmp/librefang-<feature> -b <branch> origin/main, then continue from that path."
else
  msg="✅ Session starting in a librefang LINKED WORKTREE ($repo_root). Edits permitted; cargo build/test still forbidden — only cargo check / cargo clippy."
fi

python3 - "$msg" <<'PY'
import json, sys
print(json.dumps({
    "hookSpecificOutput": {
        "hookEventName": "SessionStart",
        "additionalContext": sys.argv[1],
    }
}))
PY
