#!/usr/bin/env bash
# scan-hardcoded-urls.sh — Phase 3b of the librefang-upstream-merge skill.
#
# Greps the merge diff for new upstream-flavoured URLs that bypass the
# four origin-repoint knobs (see references/origin-knobs.md).
#
# Usage:
#   scan-hardcoded-urls.sh             # diffs HEAD^..HEAD (post-merge)
#   scan-hardcoded-urls.sh <ref>       # diffs <ref>..HEAD
#
# Exit codes:
#   0 — clean (no new upstream URLs that need rewriting)
#   1 — drift detected; manual review required

set -euo pipefail

base="${1:-HEAD^}"

if ! git rev-parse --quiet --verify "$base" >/dev/null 2>&1; then
  echo "[scan-urls] FAIL: base ref '$base' does not exist" >&2
  exit 1
fi

echo "[scan-urls] scanning $base..HEAD for upstream URLs that bypass the origin knobs..."

# Patterns that indicate a hardcoded librefang reference in source code.
# Each entry is "regex|context" — context explains what to do when matched.
#
# NOTE: macOS BSD grep does NOT support PCRE lookaheads (-P is unavailable).
# Distinguish "github.com/librefang/librefang-registry" (legit, kept as
# the empty-base_url rollback constant) from "github.com/librefang/librefang"
# (the main repo URL) via two separate patterns + a post-filter rather
# than a negative lookahead.
patterns=(
  'librefang\.ai|librefang.ai marketing domain - flip to github.com/GQAdonis/librefang'
  'github\.com/librefang/librefang[^-]|main upstream repo URL - flip to github.com/GQAdonis/librefang'
  'github\.com/librefang/librefang$|main upstream repo URL at end of line - flip to github.com/GQAdonis/librefang'
  'github\.com/librefang/librefang\.git|main upstream repo .git URL - flip to github.com/GQAdonis/librefang.git'
  '@librefang/sdk|npm package name - flip to @bossfang/sdk'
  '@librefang/cli|npm package name - flip to @bossfang/sdk (no @bossfang/cli - sdk is the universal name)'
  'pypi\.org/manage/project/librefang/|PyPI project URL - flip to bossfang-sdk'
  '"librefang-skills"|FangHub default org literal - must read from config.skills.marketplace.github_org'
  'stats\.librefang\.ai|legacy LibreFang Cloudflare cache - BossFang does not operate this; remove or condition'
  'deploy\.librefang\.ai|legacy LibreFang deploy hub - remove or repoint'
  'docs\.librefang\.ai|legacy LibreFang docs site - flip to github.com/GQAdonis/librefang/blob/main/docs'
  'ghcr\.io/librefang/librefang|GHCR image - flip to ghcr.io/GQAdonis/librefang'
  '"LibreFang/0\.1"|UA string - flip to "BossFang/0.1"'
  '"librefang-skills/0\.1"|UA string - flip to "bossfang-skills/0.1"'
  '"librefang-plugin-updater/1\.0"|UA string - flip to "bossfang-plugin-updater/1.0"'
  '"librefang-plugin-search/1\.0"|UA string - flip to "bossfang-plugin-search/1.0"'
  '"LibreFang-Webhook/1\.0"|UA string - flip to "BossFang-Webhook/1.0"'
  'application/vnd\.librefang\.|Content-Type vendor prefix - primary should be vnd.bossfang.; vnd.librefang. is back-compat only'
)

# Files that legitimately contain upstream URLs as rollback constants.
# Hits in these files are documented behaviour, not regressions.
allow_files=(
  'crates/librefang-runtime/src/registry_sync.rs'   # DEFAULT_REGISTRY_BASE_URL et al, rollback constants
  'crates/librefang-types/src/config/types.rs'      # doc comment about the upstream default
)

any_hit=0

for entry in "${patterns[@]}"; do
  pat="${entry%%|*}"
  ctx="${entry#*|}"

  # Match added lines only ($+) in the diff. Output format from
  # `grep -n` includes "lineno:content", so we then strip the
  # allow-listed files via a second pass with awk.
  raw="$(git diff --unified=0 "$base..HEAD" -- \
            'crates/**/*.rs' \
            'crates/**/Cargo.toml' \
            'sdk/**' \
            'Cargo.toml' \
            '.github/workflows/*.yml' \
            'README.md' \
            'CONTRIBUTING.md' 2>/dev/null \
          | grep -E "^\+[^+]" \
          | grep -E "$pat" \
          || true)"

  if [ -n "$raw" ]; then
    any_hit=1
    echo
    echo "[scan-urls] HIT: $pat"
    echo "  context: $ctx"
    echo "  matches (added lines, up to 10):"
    printf '%s\n' "$raw" | head -10 | sed 's/^/    /'
  fi
done

# Also surface which files are affected for easy navigation.
if [ "$any_hit" = "1" ]; then
  echo
  echo "[scan-urls] affected files (excluding allow-listed rollback-constant files):"
  changed_files="$(git diff --name-only "$base..HEAD" -- 'crates/**/*.rs' '*.toml' 'sdk/**' '.github/**' '*.md' 2>/dev/null || true)"
  printf '%s\n' "$changed_files" | while IFS= read -r f; do
    [ -n "$f" ] || continue
    # Skip allow-listed files
    skip=0
    for allowed in "${allow_files[@]}"; do
      if [ "$f" = "$allowed" ]; then skip=1; break; fi
    done
    [ "$skip" = "1" ] && continue

    # Six simpler greps cover the same surface as the unbalanced alt-regex.
    if git diff --unified=0 "$base..HEAD" -- "$f" 2>/dev/null | grep -qiE \
         -e 'librefang\.ai' \
         -e 'github\.com/librefang' \
         -e '@librefang/' \
         -e '"librefang-skills' \
         -e '"librefang-plugin-' \
         -e '"LibreFang/' \
         -e '"LibreFang-Webhook' \
         -e 'application/vnd\.librefang'; then
      echo "  $f"
    fi
  done
  echo
  echo "[scan-urls] action required: rewrite each hit per references/origin-knobs.md"
  echo "[scan-urls] allow-listed files (legit rollback constants):"
  for allowed in "${allow_files[@]}"; do echo "  $allowed"; done
  exit 1
fi

echo "[scan-urls] ok: no new upstream URLs detected"
exit 0
