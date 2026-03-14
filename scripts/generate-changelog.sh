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

# --- Fetch PRs ---

if [ -n "$BASE_TAG" ]; then
    BASE_DATE=$(git -C "$REPO_ROOT" log -1 --format=%aI "$BASE_TAG" 2>/dev/null || echo "")
fi

if ! command -v gh &>/dev/null; then
    echo "Error: gh CLI required" >&2
    exit 1
fi

FETCH_ARGS=(--repo librefang/librefang --state merged --json number,title,author --limit 200)
if [ -n "${BASE_DATE:-}" ]; then
    FETCH_ARGS+=(--search "merged:>=${BASE_DATE}")
fi

PRS=$(gh pr list "${FETCH_ARGS[@]}" 2>/dev/null || echo "[]")

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

categories = {}
for pr in prs:
    title = pr['title'].strip()
    num = pr['number']
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
    FIRST=$(grep -n '^## \[' "$CHANGELOG" | head -1 | cut -d: -f1)
    if [ -n "$FIRST" ]; then
        { head -n $((FIRST - 1)) "$CHANGELOG"; echo "$SECTION"; echo ""; tail -n +"$FIRST" "$CHANGELOG"; } > "$CHANGELOG.tmp"
        mv "$CHANGELOG.tmp" "$CHANGELOG"
    else
        printf '\n%s\n' "$SECTION" >> "$CHANGELOG"
    fi
fi

echo "Updated $CHANGELOG"
