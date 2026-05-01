#!/usr/bin/env bash
# PreToolUse Bash safety guard for librefang.
#
# Blocks classes of AI mistakes documented under "Other AI safety hooks"
# in CLAUDE.md. Rule logic lives in lib/check-bash-rules.py, which uses
# shlex tokenization so quoted argument content (commit message bodies,
# etc.) does not falsely match against rule keywords.
#
# Hook protocol: read JSON from stdin, exit 2 to deny.

set -euo pipefail
input="$(cat)"
script_dir="$(cd "$(dirname "$0")" && pwd -P)"
LIB="$script_dir/lib/check-bash-rules.py"

py() { python3 -c "$1" 2>/dev/null || true; }

tool="$(printf '%s' "$input" | py 'import sys,json; print(json.load(sys.stdin).get("tool_name",""))')"
[ "$tool" = "Bash" ] || exit 0

cmd="$(printf '%s' "$input" | py 'import sys,json; print(json.load(sys.stdin).get("tool_input",{}).get("command",""))')"
[ -n "$cmd" ] || exit 0

rules="force-push-main,no-verify,broad-git-add,sensitive-file-add,claude-attribution,rm-rf-dangerous,librefang-daemon-launch,cargo-add-remove-upgrade,gh-pr-merge"

msg="$(printf '%s' "$cmd" | python3 "$LIB" --rules "$rules" 2>/dev/null || true)"

if [ -n "$msg" ]; then
  cat >&2 <<MSG
[guard-bash-safety] $msg
Command: $cmd
MSG
  exit 2
fi

exit 0
