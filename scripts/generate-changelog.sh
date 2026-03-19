#!/usr/bin/env bash
#
# generate-changelog.sh — Generate a CHANGELOG entry from merged PRs.
#
# Usage:
#   ./scripts/generate-changelog.sh 0.5.0           # from last tag to HEAD
#   ./scripts/generate-changelog.sh 0.5.0 v0.4.0    # from specific base tag
#
# PR titles are guaranteed to follow conventional commit format by CI,
# so classification is a simple prefix match.
#
# Requires: gh (GitHub CLI), python3

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
CHANGELOG="$REPO_ROOT/CHANGELOG.md"

VERSION="${1:?Usage: generate-changelog.sh VERSION [BASE_TAG]}"
DATE=$(date +%Y-%m-%d)

if [ $# -ge 2 ]; then
    BASE_TAG="$2"
else
    BASE_TAG=$(git -C "$REPO_ROOT" tag --sort=-creatordate | grep -E '^v[0-9]' | grep -vE '(alpha|beta|rc)' | head -1 || true)
fi

echo "Generating changelog: $VERSION (since ${BASE_TAG:-beginning})"

# --- Extract PR numbers from git commit range ---

if ! command -v gh &>/dev/null; then
    echo "Error: gh CLI required" >&2
    exit 1
fi

if [ -n "$BASE_TAG" ]; then
    GIT_RANGE="${BASE_TAG}..HEAD"
else
    GIT_RANGE="HEAD"
fi

# Extract PR numbers from merge commits and squash-merge messages
PR_NUMBERS=$(git -C "$REPO_ROOT" log --oneline "$GIT_RANGE" | grep -oE '#[0-9]+' | sed 's/#//' | sort -un)

if [ -z "$PR_NUMBERS" ]; then
    echo "No PRs found in range $GIT_RANGE"
    PRS="[]"
else
    # Fetch details for each PR
    PRS="["
    FIRST=true
    for NUM in $PR_NUMBERS; do
        PR_JSON=$(gh pr view "$NUM" --repo librefang/librefang --json number,title,author 2>/dev/null || echo "")
        if [ -n "$PR_JSON" ]; then
            if [ "$FIRST" = true ]; then
                FIRST=false
            else
                PRS+=","
            fi
            PRS+="$PR_JSON"
        fi
    done
    PRS+="]"
fi

# --- Classify by prefix ---

CLASSIFIED=$(echo "$PRS" | python3 -c "
import json, sys, re

prs = json.load(sys.stdin)

MAP = {
    'feat': 'Added',
    'fix': 'Fixed',
    'refactor': 'Changed',
    'perf': 'Performance',
    'docs': 'Documentation',
    'doc': 'Documentation',
    'chore': 'Maintenance',
    'ci': 'Maintenance',
    'build': 'Maintenance',
    'test': 'Maintenance',
    'style': 'Maintenance',
    'revert': 'Reverted',
}

SKIP_PATTERNS = [
    r'update star history',
    r'^v?\d+\.\d+\.\d+',
    r'^release:',
]

categories = {}
for pr in prs:
    title = pr['title'].strip()
    num = pr['number']

    # Skip automated/bot PRs
    if any(re.search(p, title, re.IGNORECASE) for p in SKIP_PATTERNS):
        continue
    author = pr.get('author', {}).get('login', '')
    credit = f' (@{author})' if author else ''

    m = re.match(r'^(\w+)(?:\([^)]*\))?[!]?:\s*(.*)', title)
    if m:
        prefix, desc = m.group(1).lower(), m.group(2).strip()
        cat = MAP.get(prefix, 'Other')
    else:
        desc, cat = title, 'Other'

    desc = desc[0].upper() + desc[1:] if desc else title
    categories.setdefault(cat, []).append(f'{desc} (#{num}){credit}')

order = ['Added','Fixed','Changed','Performance','Documentation','Maintenance','Reverted','Other']
for cat in order:
    items = categories.get(cat, [])
    if items:
        print(f'### {cat}')
        print()
        for item in items:
            print(f'- {item}')
        print()
")

# --- Write to CHANGELOG.md ---

if [ -z "$CLASSIFIED" ]; then
    SECTION="## [$VERSION] - $DATE

_No notable changes._"
else
    SECTION="## [$VERSION] - $DATE

$CLASSIFIED"
fi

if [ ! -f "$CHANGELOG" ]; then
    printf '%s\n\n%s\n' "# Changelog" "$SECTION" > "$CHANGELOG"
else
    # If version already exists, replace it; otherwise insert before first entry
    if grep -q "^## \[$VERSION\]" "$CHANGELOG"; then
        echo "Replacing existing changelog entry for $VERSION"
        OLD_START=$(grep -n "^## \[$VERSION\]" "$CHANGELOG" | head -1 | cut -d: -f1)
        NEXT_HEADING=$(tail -n +"$((OLD_START + 1))" "$CHANGELOG" | grep -n '^## \[' | head -1 | cut -d: -f1)
        if [ -n "$NEXT_HEADING" ]; then
            OLD_END=$((OLD_START + NEXT_HEADING - 1))
            { head -n $((OLD_START - 1)) "$CHANGELOG"; echo "$SECTION"; echo ""; tail -n +"$OLD_END" "$CHANGELOG"; } > "$CHANGELOG.tmp"
        else
            { head -n $((OLD_START - 1)) "$CHANGELOG"; echo "$SECTION"; echo ""; } > "$CHANGELOG.tmp"
        fi
        mv "$CHANGELOG.tmp" "$CHANGELOG"
    else
        FIRST=$(grep -n '^## \[' "$CHANGELOG" | head -1 | cut -d: -f1)
        if [ -n "$FIRST" ]; then
            { head -n $((FIRST - 1)) "$CHANGELOG"; echo "$SECTION"; echo ""; tail -n +"$FIRST" "$CHANGELOG"; } > "$CHANGELOG.tmp"
            mv "$CHANGELOG.tmp" "$CHANGELOG"
        else
            printf '\n%s\n' "$SECTION" >> "$CHANGELOG"
        fi
    fi
fi

echo "Updated $CHANGELOG"
