#!/usr/bin/env bash
# Corpus test for scripts/check-pr-commit-attribution.sh.
#
# The checker drives scripts/hooks/commit-msg once per commit, so the rules themselves are already covered by scripts/tests/commit-msg-attribution.sh.
# What is tested here is the part that script cannot reach: that the driver finds the offending commit at all — the right range, the author field, the committer field, and a clean branch passing.
set -euo pipefail

repo_root=$(cd "$(dirname "$0")/../.." && pwd)
CHECKER="$repo_root/scripts/check-pr-commit-attribution.sh"

if [ ! -x "$CHECKER" ]; then
  echo "FAIL: checker not executable at $CHECKER" >&2
  exit 1
fi

tmpdir=$(mktemp -d)
trap 'rm -rf "$tmpdir"' EXIT

# A throwaway repo, so the corpus is built from real commits rather than from strings — the failure this guards against is a range/field mistake, which a string fixture cannot exercise.
work="$tmpdir/repo"
mkdir -p "$work"
cd "$work"
git init -q -b main
git config user.name "Human Person"
git config user.email "human@example.com"
git config commit.gpgsign false
echo seed > seed.txt
git add seed.txt
git commit -q -m "chore: seed"
base_sha=$(git rev-parse HEAD)

pass=0
fail=0

check() {
  local label="$1" expect="$2" head="$3"
  local out rc
  set +e
  out=$("$CHECKER" "$base_sha" "$head" 2>&1)
  rc=$?
  set -e
  if [ "$expect" = "reject" ] && [ "$rc" -ne 0 ]; then
    echo "  ok   (rejected) $label"
    pass=$((pass + 1))
  elif [ "$expect" = "accept" ] && [ "$rc" -eq 0 ]; then
    echo "  ok   (accepted) $label"
    pass=$((pass + 1))
  else
    echo "  FAIL $label — expected $expect, exit=$rc" >&2
    printf '%s\n' "$out" | sed 's/^/       /' >&2
    fail=$((fail + 1))
  fi
}

# ── a clean branch passes ──────────────────────────────────────────────────
git checkout -q -b clean "$base_sha"
echo a > a.txt && git add a.txt && git commit -q -m "feat: add a"
echo b > b.txt && git add b.txt && git commit -q -m "fix: add b"
check "two human commits" accept clean

# ── a Claude AUTHOR is caught, even with a spotless message ───────────────
git checkout -q -b bad-author "$base_sha"
echo c > c.txt && git add c.txt
GIT_AUTHOR_NAME=Claude GIT_AUTHOR_EMAIL=noreply@anthropic.com \
  git commit -q -m "docs: rewrap a comment"
check "Claude author, clean message" reject bad-author

# ── a Claude COMMITTER with a human author is caught ──────────────────────
git checkout -q -b bad-committer "$base_sha"
echo d > d.txt && git add d.txt
GIT_COMMITTER_NAME=Claude GIT_COMMITTER_EMAIL=noreply@anthropic.com \
  git commit -q -m "docs: reword a comment"
check "human author, Claude committer" reject bad-committer

# ── a Co-authored-by trailer is caught ────────────────────────────────────
git checkout -q -b bad-trailer "$base_sha"
echo e > e.txt && git add e.txt
git commit -q -F - <<'MSG'
feat: something

Co-authored-by: Claude <noreply@anthropic.com>
MSG
check "Co-authored-by trailer" reject bad-trailer

# ── the offending commit buried mid-branch is still found ─────────────────
git checkout -q -b buried "$base_sha"
echo f > f.txt && git add f.txt && git commit -q -m "feat: first"
echo g > g.txt && git add g.txt
GIT_AUTHOR_NAME=Claude GIT_AUTHOR_EMAIL=noreply@anthropic.com \
  git commit -q -m "chore: middle"
echo h > h.txt && git add h.txt && git commit -q -m "feat: last"
check "Claude commit in the middle of three" reject buried

# ── a name that merely contains "claude" is NOT collateral ────────────────
# The hook matches whole names on purpose; this pins that the driver does not widen it.
git checkout -q -b claudia "$base_sha"
echo i > i.txt && git add i.txt
GIT_AUTHOR_NAME="Claudia Fernandez" GIT_AUTHOR_EMAIL=claudia@example.com \
  git commit -q -m "feat: from Claudia"
check "contributor named Claudia" accept claudia

# ── a branch merely behind base is not re-judged ──────────────────────────
# merge-base, not a raw range: `main` moving forward must not drag unrelated commits into the check.
git checkout -q main
echo z > z.txt && git add z.txt && git commit -q -m "chore: main moves on"
check "clean branch while base moved ahead" accept clean

echo
echo "pr-commit-attribution: $pass passed, $fail failed"
[ "$fail" -eq 0 ]
