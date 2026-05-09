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

# Use here-strings (`<<<"$input"`) instead of `printf … | py …` pipelines.
# Under `set -o pipefail`, the original pattern propagated a 141 exit
# (SIGPIPE on printf when python's `json.load` finished and exited
# before printf had flushed the input) as the pipeline's status. With
# `set -e` that aborted the hook before any rule even ran, surfacing as
# "Failed with non-blocking status code: No stderr output" noise in the
# Claude Code UI. The here-string form has no active writer process —
# bash buffers `$input` into a temp FD before the reader starts — so
# SIGPIPE cannot occur. This also closes a latent silent-bypass: when
# the old `msg=…|| true` pipeline ate a 141, `msg` came back empty and
# the hook returned 0 (allow), which would have hidden a real rule hit.
tool="$(py 'import sys,json; print(json.load(sys.stdin).get("tool_name",""))' <<<"$input")"
[ "$tool" = "Bash" ] || exit 0

cmd="$(py 'import sys,json; print(json.load(sys.stdin).get("tool_input",{}).get("command",""))' <<<"$input")"
[ -n "$cmd" ] || exit 0

rules="force-push-main,no-verify,broad-git-add,sensitive-file-add,claude-attribution,rm-rf-dangerous,librefang-daemon-launch"

msg="$(python3 "$LIB" --rules "$rules" <<<"$cmd" 2>/dev/null || true)"

if [ -n "$msg" ]; then
  cat >&2 <<MSG
[guard-bash-safety] $msg
Command: $cmd
MSG
  exit 2
fi

exit 0
