#!/usr/bin/env bash
#
# generate-changelog.sh — Generate a CHANGELOG entry from merged PRs.
#
# Usage:
#   ./scripts/generate-changelog.sh 0.5.0           # from last tag to HEAD
#   ./scripts/generate-changelog.sh 0.5.0 v0.4.0    # from specific base tag
#
# Categorizes PRs by:
#   1. GitHub labels (feat, fix, docs, etc.)
#   2. Conventional commit prefix in title (feat:, fix:, etc.)
#   3. Title keyword analysis (Add → feat, Fix → fix, etc.)
#
# Requires: gh (GitHub CLI), python3
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

echo "Generating changelog: $VERSION (since ${BASE_TAG:-beginning})"

# --- Collect and categorize PRs ---

if command -v gh &>/dev/null; then
    echo "Fetching merged PRs from GitHub..."

    if [ -n "$BASE_TAG" ]; then
        BASE_DATE=$(git -C "$REPO_ROOT" log -1 --format=%aI "$BASE_TAG" 2>/dev/null || echo "")
    else
        BASE_DATE=""
    fi

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

    # Single python script: classify all PRs by label → prefix → keyword
    CLASSIFIED=$(echo "$PRS" | python3 -c "
import json, sys, re

prs = json.load(sys.stdin)

# category → display lines
categories = {
    'Added': [],
    'Fixed': [],
    'Changed': [],
    'Performance': [],
    'Documentation': [],
    'Maintenance': [],
    'Other': [],
}

# Conventional commit prefixes
PREFIX_MAP = {
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
}

# Keywords for titles without conventional prefix
KEYWORD_RULES = [
    (r'^add\b', 'Added'),
    (r'^implement\b', 'Added'),
    (r'^support\b', 'Added'),
    (r'^new\b', 'Added'),
    (r'^introduce\b', 'Added'),
    (r'^enable\b', 'Added'),
    (r'^create\b', 'Added'),
    (r'^fix\b', 'Fixed'),
    (r'^resolve\b', 'Fixed'),
    (r'^patch\b', 'Fixed'),
    (r'^correct\b', 'Fixed'),
    (r'^repair\b', 'Fixed'),
    (r'^update\b', 'Changed'),
    (r'^improve\b', 'Changed'),
    (r'^enhance\b', 'Changed'),
    (r'^refactor\b', 'Changed'),
    (r'^rewrite\b', 'Changed'),
    (r'^replace\b', 'Changed'),
    (r'^migrate\b', 'Changed'),
    (r'^rename\b', 'Changed'),
    (r'^optimize\b', 'Performance'),
    (r'^speed\b', 'Performance'),
    (r'^bump\b', 'Maintenance'),
    (r'^upgrade\b', 'Maintenance'),
    (r'^remove\b', 'Changed'),
    (r'^delete\b', 'Changed'),
    (r'^deprecate\b', 'Changed'),
    (r'^drop\b', 'Changed'),
]

for pr in prs:
    title = pr['title'].strip()
    num = pr['number']
    author = pr.get('author', {}).get('login', '')
    labels = [l['name'].lower() for l in pr.get('labels', [])]

    category = None
    display_title = title

    # 1. Try GitHub labels
    for label in labels:
        for prefix, cat in PREFIX_MAP.items():
            if label == prefix or label == f'type/{prefix}':
                category = cat
                break
        if category:
            break

    # 2. Try conventional commit prefix (feat:, fix(scope):, etc.)
    if not category:
        m = re.match(r'^(\w+)[\(:]', title.lower())
        if m and m.group(1) in PREFIX_MAP:
            category = PREFIX_MAP[m.group(1)]
            # Strip prefix from display title
            display_title = re.sub(r'^\w+(\([^)]*\))?[:\s]+', '', title).strip()

    # 3. Try keyword analysis on raw title
    if not category:
        for pattern, cat in KEYWORD_RULES:
            if re.search(pattern, title, re.IGNORECASE):
                category = cat
                break

    # 4. Fallback
    if not category:
        category = 'Other'

    # Capitalize first letter
    if display_title:
        display_title = display_title[0].upper() + display_title[1:]

    credit = f' (@{author})' if author else ''
    categories[category].append(f'{display_title} (#{num}){credit}')

# Output as sections
for cat in ['Added', 'Fixed', 'Changed', 'Performance', 'Documentation', 'Maintenance', 'Other']:
    items = categories[cat]
    if items:
        print(f'### {cat}')
        print()
        for item in items:
            print(f'- {item}')
        print()
" 2>/dev/null || true)

else
    echo "Warning: gh not available, falling back to git log"

    RANGE="${BASE_TAG:+${BASE_TAG}..}HEAD"
    CLASSIFIED=$(git -C "$REPO_ROOT" log "$RANGE" --pretty=format:"%s" --no-merges 2>/dev/null | python3 -c "
import sys, re

PREFIX_MAP = {
    'feat': 'Added', 'fix': 'Fixed', 'refactor': 'Changed',
    'perf': 'Performance', 'docs': 'Documentation', 'doc': 'Documentation',
    'chore': 'Maintenance', 'ci': 'Maintenance', 'build': 'Maintenance',
    'test': 'Maintenance', 'style': 'Maintenance',
}
KEYWORD_RULES = [
    (r'^add\b', 'Added'), (r'^implement\b', 'Added'), (r'^support\b', 'Added'),
    (r'^new\b', 'Added'), (r'^enable\b', 'Added'), (r'^create\b', 'Added'),
    (r'^fix\b', 'Fixed'), (r'^resolve\b', 'Fixed'),
    (r'^update\b', 'Changed'), (r'^improve\b', 'Changed'), (r'^refactor\b', 'Changed'),
    (r'^rewrite\b', 'Changed'), (r'^replace\b', 'Changed'), (r'^rename\b', 'Changed'),
    (r'^optimize\b', 'Performance'),
    (r'^bump\b', 'Maintenance'), (r'^remove\b', 'Changed'),
]

categories = {k: [] for k in ['Added','Fixed','Changed','Performance','Documentation','Maintenance','Other']}
seen = set()
for line in sys.stdin:
    title = line.strip()
    if not title or title in seen:
        continue
    seen.add(title)
    cat = None
    display = title
    m = re.match(r'^(\w+)[\(:]', title.lower())
    if m and m.group(1) in PREFIX_MAP:
        cat = PREFIX_MAP[m.group(1)]
        display = re.sub(r'^\w+(\([^)]*\))?[:\s]+', '', title).strip()
    if not cat:
        for p, c in KEYWORD_RULES:
            if re.search(p, title, re.IGNORECASE):
                cat = c
                break
    if not cat:
        cat = 'Other'
    if display:
        display = display[0].upper() + display[1:]
    categories[cat].append(f'- {display}')

for cat in ['Added','Fixed','Changed','Performance','Documentation','Maintenance','Other']:
    if categories[cat]:
        print(f'### {cat}')
        print()
        for item in categories[cat]:
            print(item)
        print()
" 2>/dev/null || true)
fi

# --- Build section ---

if [ -z "$CLASSIFIED" ]; then
    SECTION="## [$VERSION] - $DATE

_No notable changes._"
else
    SECTION="## [$VERSION] - $DATE

$CLASSIFIED"
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
