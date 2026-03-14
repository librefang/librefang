#!/usr/bin/env bash
#
# generate-changelog.sh — Generate a CHANGELOG entry from merged PRs.
#
# Usage:
#   ./scripts/generate-changelog.sh 0.5.0           # from last tag to HEAD
#   ./scripts/generate-changelog.sh 0.5.0 v0.4.0    # from specific base tag
#
# Requires: gh (GitHub CLI)
#
# Output: prepends a new version section to CHANGELOG.md

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
CHANGELOG="$REPO_ROOT/CHANGELOG.md"

VERSION="${1:?Usage: generate-changelog.sh VERSION [BASE_TAG]}"
DATE=$(date +%Y-%m-%d)

# Find base tag
if [ $# -ge 2 ]; then
    BASE_TAG="$2"
else
    BASE_TAG=$(git -C "$REPO_ROOT" tag --sort=-creatordate | grep -E '^v[0-9]' | head -1 || true)
fi

# Get base date for PR query
if [ -n "$BASE_TAG" ]; then
    BASE_DATE=$(git -C "$REPO_ROOT" log -1 --format=%aI "$BASE_TAG" 2>/dev/null || echo "")
else
    BASE_DATE=""
fi

echo "Generating changelog: $VERSION (since ${BASE_TAG:-beginning})"

# --- Collect PRs ---

if command -v gh &>/dev/null; then
    echo "Fetching merged PRs from GitHub..."

    # Get merged PRs since base tag
    if [ -n "$BASE_DATE" ]; then
        PRS=$(gh pr list --repo librefang/librefang \
            --state merged \
            --search "merged:>=${BASE_DATE}" \
            --json number,title,labels,author \
            --limit 200 2>/dev/null || echo "[]")
    else
        PRS=$(gh pr list --repo librefang/librefang \
            --state merged \
            --json number,title,labels,author \
            --limit 200 2>/dev/null || echo "[]")
    fi

    # Parse PRs by label/title prefix into categories
    extract_category() {
        local label_or_prefix="$1"
        echo "$PRS" | python3 -c "
import json, sys
prs = json.load(sys.stdin)
for pr in prs:
    labels = [l['name'] for l in pr.get('labels', [])]
    title = pr['title']
    num = pr['number']
    author = pr.get('author', {}).get('login', '')

    # Match by label or title prefix
    prefix = '$label_or_prefix'
    matched = False
    if prefix in labels:
        matched = True
    elif title.lower().startswith(prefix + ':') or title.lower().startswith(prefix + '('):
        matched = True
        title = title.split(':', 1)[-1].strip() if ':' in title else title

    if matched:
        credit = f' (@{author})' if author else ''
        print(f'{title} (#{num}){credit}')
" 2>/dev/null || true
    }

    FEATS=$(extract_category "feat")
    FIXES=$(extract_category "fix")
    REFACTORS=$(extract_category "refactor")
    DOCS=$(extract_category "docs")
    CHORES=$(extract_category "chore")
    PERFS=$(extract_category "perf")

    # Catch unlabeled/unmatched PRs
    ALL_MATCHED=$(echo "$FEATS"; echo "$FIXES"; echo "$REFACTORS"; echo "$DOCS"; echo "$CHORES"; echo "$PERFS")
    OTHER=$(echo "$PRS" | python3 -c "
import json, sys
prs = json.load(sys.stdin)
matched_nums = set()
for line in sys.stdin.read().strip().split('\n'):
    import re
    m = re.search(r'#(\d+)', line)
    if m:
        matched_nums.add(int(m.group(1)))
" 2>/dev/null <<< "$ALL_MATCHED" || true)

else
    echo "Warning: gh not available, falling back to git log commits"

    # Fallback: use merge commits (squash-merged PRs show as regular commits)
    collect_commits() {
        local prefix="$1"
        local range="${BASE_TAG:+${BASE_TAG}..}HEAD"
        git -C "$REPO_ROOT" log "$range" --pretty=format:"%s" --no-merges 2>/dev/null \
            | grep -iE "^${prefix}" \
            | sed -E "s/^${prefix}[:(! ]*[ )]*//" \
            | awk '{ print toupper(substr($0,1,1)) substr($0,2) }' \
            | sort -u || true
    }

    FEATS=$(collect_commits "feat")
    FIXES=$(collect_commits "fix")
    REFACTORS=$(collect_commits "refactor")
    DOCS=$(collect_commits "docs?")
    CHORES=$(collect_commits "chore")
    PERFS=$(collect_commits "perf")
fi

# --- Build the new section ---

SECTION="## [$VERSION] - $DATE"

add_category() {
    local title="$1"
    local items="$2"
    if [ -n "$items" ]; then
        SECTION="$SECTION

### $title
"
        while IFS= read -r line; do
            [ -n "$line" ] && SECTION="$SECTION
- $line"
        done <<< "$items"
    fi
}

add_category "Added" "$FEATS"
add_category "Fixed" "$FIXES"
add_category "Changed" "$REFACTORS"
add_category "Performance" "$PERFS"
add_category "Documentation" "$DOCS"
add_category "Maintenance" "$CHORES"

# If nothing found, add placeholder
if [ -z "$FEATS" ] && [ -z "$FIXES" ] && [ -z "$REFACTORS" ] && [ -z "$CHORES" ] && [ -z "$DOCS" ] && [ -z "$PERFS" ]; then
    SECTION="$SECTION

_No notable changes._"
fi

# --- Prepend to CHANGELOG.md ---

if [ ! -f "$CHANGELOG" ]; then
    echo "# Changelog" > "$CHANGELOG"
    echo "" >> "$CHANGELOG"
    echo "$SECTION" >> "$CHANGELOG"
else
    FIRST_VERSION_LINE=$(grep -n '^## \[' "$CHANGELOG" | head -1 | cut -d: -f1)
    if [ -n "$FIRST_VERSION_LINE" ]; then
        {
            head -n $((FIRST_VERSION_LINE - 1)) "$CHANGELOG"
            echo "$SECTION"
            echo ""
            tail -n +$FIRST_VERSION_LINE "$CHANGELOG"
        } > "$CHANGELOG.tmp"
        mv "$CHANGELOG.tmp" "$CHANGELOG"
    else
        echo "" >> "$CHANGELOG"
        echo "$SECTION" >> "$CHANGELOG"
    fi
fi

echo "Updated $CHANGELOG"
