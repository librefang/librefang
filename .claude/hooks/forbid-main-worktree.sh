#!/usr/bin/env bash
# PreToolUse guard: forbid Claude from doing modifying work in the librefang
# main worktree. CLAUDE.md requires `git worktree add` for any feature work.
# Hook protocol: read tool-call JSON from stdin, exit 2 to deny.

set -euo pipefail

input="$(cat)"
py() { python3 -c "$1" 2>/dev/null || true; }

cwd="$(printf '%s' "$input" | py 'import sys,json; print(json.load(sys.stdin).get("cwd",""))')"
tool="$(printf '%s' "$input" | py 'import sys,json; print(json.load(sys.stdin).get("tool_name",""))')"

# detect_git <path>: prints "<repo_root> <kind>" where kind is "main" if the
# repo's .git is a directory or "worktree" if it is a gitlink file.
detect_git() {
  local start="$1"
  [ -n "$start" ] || return 0
  local dir
  dir="$(cd "$start" 2>/dev/null && pwd -P || true)"
  [ -n "$dir" ] || return 0
  while [ "$dir" != "/" ] && [ -n "$dir" ]; do
    if [ -e "$dir/.git" ]; then
      if [ -d "$dir/.git" ]; then echo "$dir main"; else echo "$dir worktree"; fi
      return 0
    fi
    dir="$(dirname "$dir")"
  done
}

target_dir=""
case "$tool" in
  Edit|MultiEdit|Write|NotebookEdit)
    fp="$(printf '%s' "$input" | py 'import sys,json; t=json.load(sys.stdin).get("tool_input",{}); print(t.get("file_path") or t.get("notebook_path") or "")')"
    if [ -n "$fp" ]; then
      case "$fp" in
        /*) target="$fp" ;;
        *)  target="$cwd/$fp" ;;
      esac
      target_dir="$(dirname "$target")"
    else
      target_dir="$cwd"
    fi
    ;;
  Bash)
    target_dir="$cwd"  # default; may be overridden per-command below
    ;;
  *)
    exit 0
    ;;
esac

# For Bash, prefer the path passed via `git -C <path>` if present, since the
# user often operates on a worktree from a different cwd.
if [ "$tool" = "Bash" ]; then
  cmd="$(printf '%s' "$input" | py 'import sys,json; print(json.load(sys.stdin).get("tool_input",{}).get("command",""))')"
  c_path="$(printf '%s' "$cmd" | py 'import sys,re
cmd=sys.stdin.read()
m=re.search(r"\bgit\s+-C\s+(\"([^\"]+)\"|'\''([^'\'']+)'\''|(\S+))", cmd)
if m:
    print(m.group(2) or m.group(3) or m.group(4) or "")')"
  if [ -n "$c_path" ]; then
    case "$c_path" in
      /*) target_dir="$c_path" ;;
      *)  target_dir="$cwd/$c_path" ;;
    esac
  fi
fi

read -r repo_root kind <<<"$(detect_git "$target_dir" || true)"
[ -n "${repo_root:-}" ] || exit 0
[ "${kind:-}" = "main" ] || exit 0

case "$repo_root" in
  */librefang) ;;
  *) exit 0 ;;
esac

case "$tool" in
  Edit|MultiEdit|Write|NotebookEdit)
    cat >&2 <<EOF
[forbid-main-worktree] Refusing $tool — target lives in the main worktree:
  ${target:-$target_dir}

CLAUDE.md rule: \`git worktree add\` on an external disk (or /tmp/librefang-<feature>)
for any work. Edits in the main worktree collide with the user's other sessions.
EOF
    exit 2
    ;;
  Bash)
    trimmed="$(printf '%s' "$cmd" | sed -E 's/^[[:space:]]+//')"
    block=0; reason=""
    if printf '%s' "$trimmed" | grep -qE '(^|[;&|`(]|&&|\|\|)[[:space:]]*git([[:space:]]+-C[[:space:]]+\S+)?[[:space:]]+(checkout|switch|merge|rebase|reset|commit|push|pull|cherry-pick|revert|am|apply|branch[[:space:]]+(-D|-d|-m|--delete|--force)|stash[[:space:]]+(pop|apply|drop|clear)|worktree[[:space:]]+(remove|prune)|clean|tag[[:space:]]+(-d|--delete))\b'; then
      block=1; reason="git mutation in main worktree"
    fi
    if printf '%s' "$trimmed" | grep -qE '(^|[[:space:]])(cat|tee|printf|echo)[[:space:]].*[[:space:]]>[>]?[[:space:]]'; then
      block=1; reason="${reason:+$reason; }shell write redirect in main worktree"
    fi
    if printf '%s' "$trimmed" | grep -qE '(^|[[:space:]])(sed[[:space:]]+(-[a-zA-Z]*i[a-zA-Z]*|-i)|perl[[:space:]]+-[a-zA-Z]*pi[a-zA-Z]*)\b'; then
      block=1; reason="${reason:+$reason; }in-place edit in main worktree"
    fi
    if [ "$block" -eq 1 ]; then
      cat >&2 <<EOF
[forbid-main-worktree] Refusing Bash — target is the main worktree:
  $repo_root
Reason: $reason
Command: $cmd

Move to a worktree first (or pass git -C <worktree-path>).
EOF
      exit 2
    fi
    ;;
esac
exit 0
