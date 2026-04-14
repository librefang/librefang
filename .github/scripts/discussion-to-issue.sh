#!/usr/bin/env bash
#
# Create a GitHub Issue from a Discussion.
# Shared by auto-promote, manual-promote, and backfill jobs.
#
# Usage:
#   discussion-to-issue.sh <disc_number> <disc_title> <disc_url> <disc_author> <disc_category>
#
# Env: GH_TOKEN, REPO
#
# Exit 0 on success or skip (duplicate). Exit 1 on error.

set -euo pipefail

DISC_NUMBER="$1"
DISC_TITLE="$2"
DISC_URL="$3"
DISC_AUTHOR="$4"
DISC_CATEGORY="$5"

DISC_BODY=$(gh api "repos/${REPO}/discussions/${DISC_NUMBER}" --jq '.body // ""')

# Strip emoji prefixes
CLEAN_TITLE=$(printf '%s' "$DISC_TITLE" | sed 's/^[^a-zA-Z0-9[({]*//')
[ -z "$CLEAN_TITLE" ] && CLEAN_TITLE="$DISC_TITLE"

# Map category to label
case "$DISC_CATEGORY" in
  Ideas)   LABELS="enhancement" ;;
  Q\&A)    LABELS="question" ;;
  *)       LABELS="" ;;
esac

# Run auto-label script for area labels
body_file=$(mktemp)
printf '%s' "$DISC_BODY" > "$body_file"
AREA_LABELS=$(bash .github/scripts/auto-label-issue.sh "0" "$CLEAN_TITLE" "$body_file" 2>/dev/null || true)
rm -f "$body_file"

if [ -n "$AREA_LABELS" ] && [ "$AREA_LABELS" != "needs-triage" ]; then
  if [ -n "$LABELS" ]; then
    LABELS="${LABELS},${AREA_LABELS}"
  else
    LABELS="$AREA_LABELS"
  fi
fi

# Duplicate check — use gh search which handles pagination internally
EXISTING=$(gh search issues --repo "$REPO" --match body "discussion #${DISC_NUMBER}" \
  --json number --jq 'length' 2>/dev/null || echo "0")

if [ "${EXISTING:-0}" -gt 0 ]; then
  echo "Issue already exists for discussion #${DISC_NUMBER}, skipping"
  exit 0
fi

# Build issue body (no leading indentation)
ISSUE_BODY=$(cat <<EOF
_Auto-created from discussion #${DISC_NUMBER} by @${DISC_AUTHOR} (${DISC_CATEGORY})_

---

${DISC_BODY}

---
> Source: ${DISC_URL}
EOF
)

# Create the issue
ISSUE_URL=$(gh issue create \
  --repo "$REPO" \
  --title "$CLEAN_TITLE" \
  --body "$ISSUE_BODY" \
  ${LABELS:+--label "$LABELS"})

ISSUE_NUMBER=$(printf '%s' "$ISSUE_URL" | grep -oE '[0-9]+$')
echo "#${DISC_NUMBER} → issue #${ISSUE_NUMBER}"

# Comment on the discussion via GraphQL (REST endpoint doesn't support Discussions)
DISC_NODE_ID=$(gh api "repos/${REPO}/discussions/${DISC_NUMBER}" --jq '.node_id' 2>/dev/null || true)
if [ -n "$DISC_NODE_ID" ]; then
  gh api graphql -f query='
    mutation($discussionId: ID!, $body: String!) {
      addDiscussionComment(input: {discussionId: $discussionId, body: $body}) {
        comment { id }
      }
    }
  ' -f discussionId="$DISC_NODE_ID" -f body="Tracked as issue #${ISSUE_NUMBER}" --silent 2>/dev/null || true
fi
