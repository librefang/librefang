#!/usr/bin/env bash
# PreToolUse guard for the librefang main worktree.
#
# Refuses Edit/Write tool calls whose target file lives under the main worktree,
# and Bash commands that mutate it (git mutations, sed -i, cargo build, etc.).
# The actual Bash rule logic lives in lib/check-bash-rules.py — that file uses
# shlex tokenization so it can tell apart real command invocations from string
# content inside quoted args (a long-running source of false positives in the
# old regex-based version).
#
# Hook protocol: read JSON from stdin, exit 2 to deny.

set -euo pipefail
input="$(cat)"
script_dir="$(cd "$(dirname "$0")" && pwd -P)"
LIB="$script_dir/lib/check-bash-rules.py"

py() { python3 -c "$1" 2>/dev/null || true; }

cwd="$(printf '%s' "$input" | py 'import sys,json; print(json.load(sys.stdin).get("cwd",""))')"
tool="$(printf '%s' "$input" | py 'import sys,json; print(json.load(sys.stdin).get("tool_name",""))')"

# detect_git <path>: prints "<repo_root> <kind>" where kind is main or worktree.
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

# Determine the path the action targets. For Edit/Write we look at the
# file_path; for Bash we look at cwd, plus a leading `cd <path>` and any
# `git -C <path>`.
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
    cmd="$(printf '%s' "$input" | py 'import sys,json; print(json.load(sys.stdin).get("tool_input",{}).get("command",""))')"
    target_dir="$(printf '%s' "$cmd" | python3 -c '
import sys, re, os
text = sys.stdin.read()
cwd = sys.argv[1] if len(sys.argv) > 1 else ""
base = cwd
m = re.match(r"\s*\(?\s*cd\s+(\"([^\"]+)\"|\x27([^\x27]+)\x27|(\S+))", text)
if m:
    p = m.group(2) or m.group(3) or m.group(4) or ""
    if p.startswith("/"):
        base = p
    elif base:
        base = os.path.join(base, p)
m = re.search(r"\bgit\s+-C\s+(\"([^\"]+)\"|\x27([^\x27]+)\x27|(\S+))", text)
if m:
    p = m.group(2) or m.group(3) or m.group(4) or ""
    if p.startswith("/"):
        base = p
    elif base:
        base = os.path.join(base, p)
print(base)
' "$cwd" 2>/dev/null || echo "$cwd")"
    ;;
  *)
    exit 0
    ;;
esac

read -r repo_root kind <<<"$(detect_git "$target_dir" || true)"
[ -n "${repo_root:-}" ] || exit 0

# Compute main_root (the root of the main worktree even when target_dir is in
# a linked one). Used by the cargo / worktree-remove rules so they apply
# anywhere inside the librefang repo.
main_root=""
if [ "$tool" = "Bash" ]; then
  toplevel="$(git -C "$target_dir" rev-parse --show-toplevel 2>/dev/null || true)"
  if [ -n "$toplevel" ]; then
    main_root="$(git -C "$toplevel" worktree list --porcelain 2>/dev/null \
      | awk '/^worktree / {print $2; exit}')"
    [ -n "$main_root" ] || main_root="$toplevel"
  fi
fi

# === Edit/Write tool rules ===
case "$tool" in
  Edit|MultiEdit|Write|NotebookEdit)
    [ "${kind:-}" = "main" ] || exit 0
    case "$repo_root" in
      */librefang) ;;
      *) exit 0 ;;
    esac
    cat >&2 <<EOF
[forbid-main-worktree] Refusing $tool — target lives in the main worktree:
  ${target:-$target_dir}

CLAUDE.md rule: \`git worktree add\` on an external disk (or /tmp/librefang-<feature>)
for any work. Edits in the main worktree collide with the user's other sessions.
EOF
    exit 2
    ;;
esac

# === Bash tool rules — delegated to lib/check-bash-rules.py (shlex-based) ===
# Rules that always apply when somewhere in the librefang repo (cargo bans,
# worktree remove targeting main).
rules="cargo-build-run-install,cargo-test-unscoped,worktree-remove-main"
# Rules that only apply when the effective cwd is the main worktree.
if [ "${kind:-}" = "main" ]; then
  rules="$rules,git-mutation-main,sed-i-perl-pi-main,redirect-into-main"
fi

# Only invoke the rule library when we are inside librefang (otherwise we
# would mis-fire on unrelated repos).
case "$main_root" in
  */librefang) ;;
  *) exit 0 ;;
esac

msg="$(printf '%s' "$cmd" | python3 "$LIB" \
  --rules "$rules" \
  --cwd "$cwd" \
  --main-root "$main_root" \
  --kind "${kind:-}" 2>/dev/null || true)"

if [ -n "$msg" ]; then
  cat >&2 <<EOF
[forbid-main-worktree] $msg
Command: $cmd
EOF
  exit 2
fi

exit 0
