#!/usr/bin/env bash
# PreToolUse Bash guard: block five classes of AI mistakes.
#
# Rules:
#   A. Force-push to main / master (incl. `+branch` syntax)
#   B. `--no-verify` / `--no-gpg-sign` on commit/push/rebase/merge/am/cherry-pick/pull
#   C. Staging likely-sensitive files (.env / *.pem / *.key / credentials.json / id_rsa…)
#      and broad `git add -A` / `git add .` (CLAUDE.md global rule: prefer specific paths)
#   D. Commit messages containing Claude attribution
#      (`Co-Authored-By: Claude`, `Generated with Claude`, `🤖 Generated with [Claude Code]`)
#   E. `rm -rf` against dangerous targets (/, ~, $HOME, target, .git, /Users, /usr, /etc, …)
#
# Hook protocol: read tool-call JSON from stdin, exit 2 to deny.

set -euo pipefail
input="$(cat)"
py() { python3 -c "$1" 2>/dev/null || true; }

tool="$(printf '%s' "$input" | py 'import sys,json; print(json.load(sys.stdin).get("tool_name",""))')"
[ "$tool" = "Bash" ] || exit 0

cmd="$(printf '%s' "$input" | py 'import sys,json; print(json.load(sys.stdin).get("tool_input",{}).get("command",""))')"
[ -n "$cmd" ] || exit 0

deny() {
  printf '[guard-bash-safety] %s\nCommand: %s\n' "$1" "$cmd" >&2
  exit 2
}

# ---------------------------------------------------------------------------
# A. Force-push to main / master
# ---------------------------------------------------------------------------
# Match if the command is a `git push` AND uses a force flag AND targets a
# protected branch (either `... main` / `... master` arg, or the `+branch`
# refspec syntax).
if printf '%s' "$cmd" | grep -qE '\bgit\b([[:space:]]+-C[[:space:]]+\S+)?[[:space:]].*\bpush\b'; then
  has_force=0
  if printf '%s' "$cmd" | grep -qE '([[:space:]]|^)(-f|--force|--force-with-lease)([[:space:]]|=|$)'; then
    has_force=1
  fi
  # `git push origin +main` is also a force push.
  if printf '%s' "$cmd" | grep -qE '([[:space:]])\+(main|master|HEAD)([[:space:]]|:|$)'; then
    has_force=1
  fi
  if [ "$has_force" = 1 ]; then
    if printf '%s' "$cmd" | grep -qE '([[:space:]]|/|:|\+)(main|master)([[:space:]]|:|$)'; then
      deny "Refusing force-push to main / master. Force pushes there are near-irreversible — get explicit user confirmation and consider a safer alternative."
    fi
  fi
fi

# ---------------------------------------------------------------------------
# B. --no-verify / --no-gpg-sign
# ---------------------------------------------------------------------------
if printf '%s' "$cmd" | grep -qE '\bgit\b.*(\s|^)(commit|push|rebase|merge|am|cherry-pick|pull)\b' \
   && printf '%s' "$cmd" | grep -qE '(\s|^)(--no-verify|--no-gpg-sign)\b'; then
  deny "Refusing --no-verify / --no-gpg-sign. Bypassing hooks is not allowed — fix the underlying failure instead."
fi

# ---------------------------------------------------------------------------
# D. Claude attribution in commit message
# ---------------------------------------------------------------------------
# Inspect the value of -m / --message arguments. If the user is reading the
# message from a heredoc / file we cannot see it from here; in that case we
# fall through and rely on a git-side commit-msg hook (out of scope here).
if printf '%s' "$cmd" | grep -qE '\bgit\b.*\bcommit\b'; then
  msg="$(printf '%s' "$cmd" | py '
import sys, shlex
text = sys.stdin.read()
try:
    toks = shlex.split(text, posix=True)
except ValueError:
    toks = text.split()
msgs = []
i = 0
while i < len(toks):
    t = toks[i]
    if t in ("-m", "--message", "-F", "--file"):
        if i + 1 < len(toks):
            msgs.append(toks[i+1])
        i += 2
        continue
    if t.startswith("--message="):
        msgs.append(t[len("--message="):])
    elif t.startswith("-m") and len(t) > 2:
        msgs.append(t[2:])
    i += 1
print("\n----\n".join(msgs))
')"
  if [ -n "$msg" ] && printf '%s' "$msg" | grep -qiE '(Co-Authored-By:.*(Claude|Anthropic|noreply@anthropic\.com)|Generated with .{0,40}Claude|🤖.*Claude[[:space:]]+Code)'; then
    deny "Refusing commit — message contains Claude attribution (Co-Authored-By / Generated with Claude). CLAUDE.md forbids these. Remove the line and retry."
  fi
fi

# ---------------------------------------------------------------------------
# C. Sensitive-file staging + broad `git add`
# ---------------------------------------------------------------------------
if printf '%s' "$cmd" | grep -qE '\bgit\b([[:space:]]+-C[[:space:]]+\S+)?[[:space:]].*\b(add|commit)\b'; then
  # C1. Block broad add (`git add -A`, `git add --all`, `git add .`, `git add :/`).
  if printf '%s' "$cmd" | grep -qE '\bgit\b([[:space:]]+-C[[:space:]]+\S+)?[[:space:]]+add\b[^|;&]*([[:space:]](-A|-a|--all|--update|-u)\b|[[:space:]]\.($|[[:space:]])|[[:space:]]:/(\s|$))'; then
    deny "Refusing broad \`git add\`. CLAUDE.md (global) requires staging specific files by name to avoid sweeping in secrets / large binaries. Add files explicitly: \`git add path/to/file ...\`."
  fi
  # C2. Block known sensitive filenames anywhere in the command.
  if printf '%s' "$cmd" | grep -qiE '(^|[[:space:]/=":'\''])((\.env)(\.[a-z0-9._-]+)?|id_(rsa|ed25519|ecdsa|dsa)(\.pub)?|credentials(\.[a-z]+)?|secrets?(\.[a-z]+)?|vault[_-][a-z0-9_-]+\.(key|json)|[a-z0-9_/.-]+\.(pem|p12|pfx|jks|keystore))($|[[:space:]"'\''])' \
     && ! printf '%s' "$cmd" | grep -qiE '\.(example|template|sample)(\s|$|"|'\'')'; then
    deny "Refusing — command references a likely-sensitive file (.env / id_rsa / *.pem / credentials.json / vault_*.key …). If you really need to track it, ask the user first."
  fi
fi

# ---------------------------------------------------------------------------
# E. rm -rf against dangerous paths
# ---------------------------------------------------------------------------
# Catch any rm with a recursive+force combo (-rf / -fr / -Rf / -fR / -r -f / etc.)
if printf '%s' "$cmd" | grep -qE '(^|[[:space:]/;&|`(])rm[[:space:]]+([^|;&]*[[:space:]])?(-[a-zA-Z]*r[a-zA-Z]*f[a-zA-Z]*|-[a-zA-Z]*f[a-zA-Z]*r[a-zA-Z]*|-rf|-fr|-Rf|-fR)([[:space:]]|$)' \
   || printf '%s' "$cmd" | grep -qE '(^|[[:space:]/;&|`(])rm[[:space:]]+(-[rR]\b[^|;&]*-f\b|-f\b[^|;&]*-[rR]\b)'; then
  # Dangerous targets: filesystem roots, $HOME, repo-shared dirs, system trees.
  # Match either as a standalone token after rm or with simple suffixes.
  if printf '%s' "$cmd" | grep -qE '(^|[[:space:]"'\''])(/|/\*|~|~/|\$HOME|\$HOME/|\$\{HOME\}|/Users|/Users/|/home|/home/|/usr|/var|/etc|/opt|/private|/System|/Library|target|target/|\./target|\./target/|\.git|\.git/|\./\.git|\./\.git/)([[:space:]"'\'']|$)'; then
    deny "Refusing rm -rf against a dangerous path (/, ~, \$HOME, target, .git, /Users, /usr, /etc …). Be specific or ask the user."
  fi
fi

exit 0
