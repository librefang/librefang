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

# For Bash, recompute target_dir by inspecting the command itself:
#   1. A leading `cd <path>` (or `(cd <path>` subshell) changes the shell's
#      working directory for everything that follows. Common case:
#      `cd /tmp/librefang-feat && git add ... && git commit ...`
#   2. `git -C <path>` overrides cwd for that single git invocation. Common
#      case: operating on a worktree without `cd`-ing into it.
# Resolution: cd first (sets the new shell base), then -C (overrides for git).
if [ "$tool" = "Bash" ]; then
  cmd="$(printf '%s' "$input" | py 'import sys,json; print(json.load(sys.stdin).get("tool_input",{}).get("command",""))')"

  # Compute the effective working dir for this Bash command.
  target_dir="$(printf '%s' "$cmd" | python3 -c '
import sys, re, os
text = sys.stdin.read()
cwd = sys.argv[1] if len(sys.argv) > 1 else ""
base = cwd

# Leading `cd <path>` (or `(cd <path>` subshell) at the very start of the
# command updates the shell base for everything that follows. We only
# handle the leading occurrence — chasing nested cds is too ambiguous.
m = re.match(r"\s*\(?\s*cd\s+(\"([^\"]+)\"|\x27([^\x27]+)\x27|(\S+))", text)
if m:
    p = m.group(2) or m.group(3) or m.group(4) or ""
    if p.startswith("/"):
        base = p
    elif base:
        base = os.path.join(base, p)

# `git -C <path>` overrides for that one git invocation. If both are present
# the -C wins (it is closer to the call site). Relative -C resolves against
# the post-cd base.
m = re.search(r"\bgit\s+-C\s+(\"([^\"]+)\"|\x27([^\x27]+)\x27|(\S+))", text)
if m:
    p = m.group(2) or m.group(3) or m.group(4) or ""
    if p.startswith("/"):
        base = p
    elif base:
        base = os.path.join(base, p)

print(base)
' "$cwd" 2>/dev/null || echo "$cwd")"
fi

read -r repo_root kind <<<"$(detect_git "$target_dir" || true)"
[ -n "${repo_root:-}" ] || exit 0

# For Bash, also resolve the *toplevel* of the main repo (so cargo bans apply
# whether we are in the main tree or a linked worktree — they share target/).
toplevel=""; main_root=""
if [ "$tool" = "Bash" ]; then
  toplevel="$(git -C "$target_dir" rev-parse --show-toplevel 2>/dev/null || true)"
  if [ -n "$toplevel" ]; then
    main_root="$(git -C "$toplevel" worktree list --porcelain 2>/dev/null \
      | awk '/^worktree / {print $2; exit}')"
    [ -n "$main_root" ] || main_root="$toplevel"
  fi

  # Cargo build/test/run/install: banned anywhere in the librefang repo.
  if [ -n "$main_root" ]; then
    case "$main_root" in
      */librefang)
        # Always-banned cargo subcommands (long-running and shared-target).
        if printf '%s' "$cmd" | grep -qE '(^|[;&|`(]|&&|\|\|)[[:space:]]*cargo[[:space:]]+(build|run|install)\b'; then
          cat >&2 <<EOF
[forbid-main-worktree] Refusing Bash — \`cargo build/run/install\` is
banned in this repo (target/ is shared across worktrees and contends
with the user's other sessions). Use \`cargo check\` for compile
verification; CI handles full build.
Command: $cmd
EOF
          exit 2
        fi
        # \`git worktree remove\`/\`worktree move\` against the MAIN tree —
        # blocked from any worktree. The earlier git-mutation regex only
        # catches it when cwd resolves to main; using \`git -C <linked>\` to
        # remove main was a hole. Here we parse the target path and refuse
        # if it resolves to the main worktree itself.
        wt_target_hits_main="$(printf '%s' "$cmd" | python3 -c '
import sys, shlex, os
text = sys.stdin.read()
cwd = sys.argv[1] if len(sys.argv) > 1 else ""
main_root = sys.argv[2] if len(sys.argv) > 2 else ""
real_main = os.path.realpath(main_root) if main_root else ""
try:
    toks = shlex.split(text, posix=True)
except ValueError:
    toks = text.split()
# Track a -C base for relative-path resolution.
c_base = cwd
i = 0
hit = False
subcmd = None
while i < len(toks):
    t = toks[i]
    if t == "git" and i + 1 < len(toks) and toks[i+1] == "-C":
        c_base = toks[i+2] if i + 2 < len(toks) else c_base
        i += 3
        continue
    if t == "worktree" and i + 1 < len(toks) and toks[i+1] in ("remove", "move"):
        subcmd = toks[i+1]
        rest = toks[i+2:]
        positionals = [x for x in rest if not x.startswith("-")]
        if positionals:
            target = positionals[0]
            if not target.startswith("/") and c_base:
                target = os.path.join(c_base, target)
            target = os.path.realpath(target) if target else ""
            if real_main and (target == real_main):
                hit = True
        break
    i += 1
print(str(1 if hit else 0) + "|" + (subcmd if subcmd else ""))
' "$cwd" "$main_root" 2>/dev/null || echo "0|")"
        if [ "${wt_target_hits_main%%|*}" = "1" ]; then
          subcmd="${wt_target_hits_main#*|}"
          cat >&2 <<EOF
[forbid-main-worktree] Refusing \`git worktree $subcmd\` targeting the MAIN
worktree itself ($main_root). That would destroy the user's main checkout.
If this is really what you want, ask the user to do it manually.
Command: $cmd
EOF
          exit 2
        fi
        # Conditional: \`cargo test\` without --package / -p compiles & runs the
        # whole workspace, which is the slow case we want to keep out of the AI's
        # hands. Allow scoped \`cargo test -p <crate>\`.
        if printf '%s' "$cmd" | grep -qE '(^|[;&|`(]|&&|\|\|)[[:space:]]*cargo[[:space:]]+test\b'; then
          if ! printf '%s' "$cmd" | grep -qE '(^|[[:space:]])(-p|--package)([[:space:]]+|=)[[:alnum:]_-]+'; then
            cat >&2 <<EOF
[forbid-main-worktree] Refusing Bash — unscoped \`cargo test\` builds and
runs the whole workspace, which is too slow / target-contending for the
AI to invoke. Re-run with \`-p <crate>\` (or \`--package <crate>\`) so it's
scoped to one crate. CI runs the full suite.
Command: $cmd
EOF
            exit 2
          fi
        fi
        ;;
    esac
  fi
fi

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
    # Shell write redirect: only block if the redirect target path resolves
    # *into the main worktree*. Writes to /tmp, /var/log, etc. are fine.
    redirect_into_main="$(printf '%s' "$cmd" | python3 -c '
import sys, re, os
text = sys.stdin.read()
repo = sys.argv[1] if len(sys.argv) > 1 else ""
real_repo = os.path.realpath(repo) if repo else ""
hit = False
# Match >  or  >>  (NOT preceded by a digit or & — those are file-descriptor
# operators like 2>&1, &>file, 2>file). We only care about plain stdout
# redirects, since those are what an AI would use to write into the repo.
for m in re.finditer(r"(?<![\d&])>>?\s*(?:\"([^\"]+)\"|\x27([^\x27]+)\x27|(\S+))", text):
    p = m.group(1) or m.group(2) or m.group(3)
    if not p:
        continue
    # Skip fd duplications (>&1, >&2) and known void targets.
    if p.startswith("&") or p in ("/dev/null", "/dev/stderr", "/dev/stdout"):
        continue
    if p.startswith("/"):
        ap = os.path.realpath(p)
    elif real_repo:
        ap = os.path.realpath(os.path.join(real_repo, p))
    else:
        continue
    if real_repo and (ap == real_repo or ap.startswith(real_repo + "/")):
        hit = True
        break
print("1" if hit else "0")
' "$repo_root" 2>/dev/null || echo 0)"
    if [ "$redirect_into_main" = "1" ]; then
      block=1; reason="${reason:+$reason; }shell write redirect into main worktree"
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
