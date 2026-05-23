#!/usr/bin/env bash
# Corpus test for scripts/hooks/commit-msg.
#
# Refs docs/issues/commit-msg-attribution-regex.md sub-finding "this":
# the original regex used `Claude[[:space:]]+Code` which required at least
# one space, so the zero-space variant `ClaudeCode` slipped through. After
# the fix the space class is `*` (zero or more) and all spacing variants
# are blocked equally.
#
# The hook is case-insensitive (`grep -iE`), so casings vary in the corpus
# below to lock that behavior in too.
set -euo pipefail

# Resolve hook path relative to repo root so the test can be run from
# anywhere (e.g. `bash scripts/tests/commit-msg-attribution.sh` from root,
# or invoked by CI from a different cwd).
repo_root=$(cd "$(dirname "$0")/../.." && pwd)
HOOK="$repo_root/scripts/hooks/commit-msg"

if [ ! -x "$HOOK" ]; then
  echo "FAIL: hook not executable at $HOOK" >&2
  exit 1
fi

tmpdir=$(mktemp -d)
trap 'rm -rf "$tmpdir"' EXIT

run() {
  local msg="$1"
  local file="$tmpdir/msg"
  printf '%s\n' "$msg" > "$file"
  "$HOOK" "$file" >/dev/null 2>&1
}

# Should-reject corpus.
#
# - The 🤖 ... Code variants exercise the `[[:space:]]*` fix directly:
#   zero spaces (the bug), single space (status quo), and double space.
# - Co-Authored-By and "Generated with Claude" exercise the other two
#   alternations of the same regex.
# - Mixed case locks in the `-i` flag.
REJECT_CORPUS=(
  "🤖 ClaudeCode"
  "🤖 Claude Code"
  "🤖 Claude  Code"
  "🤖 claudecode"
  "Co-Authored-By: Claude <noreply@anthropic.com>"
  "Co-Authored-By: Someone <noreply@anthropic.com>"
  "Generated with Claude Code"
  "generated with claude"
)

for bad in "${REJECT_CORPUS[@]}"; do
  if run "$bad"; then
    echo "FAIL: hook accepted attribution: $bad" >&2
    exit 1
  fi
done

# Should-accept corpus. Plain conventional commits and unrelated mentions
# of the word "code" must not trigger the guard.
ACCEPT_CORPUS=(
  "fix(api): something"
  "feat: add foo"
  "refactor(kernel): tidy code paths"
  "docs: mention Anthropic API as one option"
)

for ok in "${ACCEPT_CORPUS[@]}"; do
  if ! run "$ok"; then
    echo "FAIL: hook rejected legit message: $ok" >&2
    exit 1
  fi
done

echo "OK: $(basename "$0")"
