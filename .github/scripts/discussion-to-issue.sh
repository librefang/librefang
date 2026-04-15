#!/usr/bin/env bash
#
# Create a GitHub Issue from a Discussion.
# Shared by auto-promote, manual-promote, and backfill jobs.
#
# Usage:
#   discussion-to-issue.sh <disc_number> <disc_title> <disc_url> <disc_author> <disc_category>
#
# Env: GH_TOKEN, REPO
#   MANUAL=true — bypass content filters (used by /to-issue command)
#
# Exit 0 on success or skip (duplicate). Exit 1 on error.

set -euo pipefail

DISC_NUMBER="$1"
DISC_TITLE="$2"
DISC_URL="$3"
DISC_AUTHOR="$4"
DISC_CATEGORY="$5"

DISC_JSON=$(gh api "repos/${REPO}/discussions/${DISC_NUMBER}" 2>/dev/null || true)
if [ -z "$DISC_JSON" ]; then
  echo "#${DISC_NUMBER}: could not fetch discussion, skipping"
  exit 0
fi
DISC_BODY=$(printf '%s' "$DISC_JSON" | jq -r '.body // ""')
DISC_LOCKED=$(printf '%s' "$DISC_JSON" | jq -r '.locked // false')
DISC_UPVOTES=$(printf '%s' "$DISC_JSON" | jq -r '.reactions.total_count // 0')
DISC_AUTHOR_ASSOC=$(printf '%s' "$DISC_JSON" | jq -r '.author_association // "NONE"')

# Skip locked discussions (likely spam that was locked by moderators)
if [ "$DISC_LOCKED" = "true" ]; then
  echo "#${DISC_NUMBER}: locked, skipping (likely spam)"
  exit 0
fi

# Skip discussions with very short body from non-members (likely spam)
body_char_count=$(printf '%s' "$DISC_BODY" | wc -c | tr -d ' ')
if [ "$body_char_count" -lt 30 ] && [ "$DISC_AUTHOR_ASSOC" = "NONE" ]; then
  echo "#${DISC_NUMBER}: short body from non-member, skipping"
  exit 0
fi

# ── Content-based filtering ────────────────────────────────────────
# All categories (Ideas, Q&A, General) go through content analysis.
# /to-issue manual command is the only way to bypass filters.
# We check whether the discussion looks like a real issue (bug report,
# error, feature request with problem description) vs informational
# content (FAQ, how-to guide, community showcase thread).
if [ "${MANUAL:-}" != "true" ]; then
  TITLE_LOWER=$(printf '%s' "$DISC_TITLE" | tr '[:upper:]' '[:lower:]')
  BODY_LOWER=$(printf '%s' "$DISC_BODY" | tr '[:upper:]' '[:lower:]')

  # 1) Maintainer-authored Q&A → almost certainly FAQ content they wrote
  #    for the community, not a bug report.
  if [ "$DISC_CATEGORY" = "Q&A" ]; then
    case "$DISC_AUTHOR_ASSOC" in
      OWNER|MEMBER|COLLABORATOR)
        echo "#${DISC_NUMBER}: maintainer Q&A, skipping (FAQ content)"
        exit 0
        ;;
    esac
  fi

  # 2) Title patterns that indicate informational / community content
  is_informational=false
  if printf '%s' "$TITLE_LOWER" | grep -qE \
    '^(how (to|does|do|is|can)|what |where (to|can|do)|can (i|we|you) |which |does )'; then
    is_informational=true
  fi
  if printf '%s' "$TITLE_LOWER" | grep -qE \
    '(guide$|howto[: ]|tutorial|^show your|^share your|^tips (&|and)|^introduce )'; then
    is_informational=true
  fi

  # 3) Problem indicators — suggest a real bug/issue even if phrased as
  #    a question. \b ensures "bug" doesn't match inside "debug".
  has_problem=false
  if printf '%s' "$TITLE_LOWER" | grep -qE \
    "(can'?t |cannot |doesn'?t work|not work|broken|crash|error|\bbug\b|fail|issue with|problem|wrong|missing|unexpected|regression|panic|segfault|timeout|refused|denied|forbidden|ignor|warning|401|403|404|500|502|503)"; then
    has_problem=true
  fi

  # 4) Decision
  if [ "$has_problem" = "true" ]; then
    echo "#${DISC_NUMBER}: problem indicators in title, promoting"
  elif [ "$is_informational" = "true" ]; then
    echo "#${DISC_NUMBER}: informational content in ${DISC_CATEGORY}, skipping"
    exit 0
  else
    # Ambiguous title — check body for problem signals
    if printf '%s' "$BODY_LOWER" | grep -qE \
      "(can'?t |cannot |doesn'?t work|not work|broken|crash|stacktrace|traceback|error|panic|segfault|exception|\bbug\b|fail|regression|timeout|refused|denied|forbidden|ignor|warning|401|403|404|500|502|503)"; then
      echo "#${DISC_NUMBER}: problem indicators in body, promoting"
    else
      echo "#${DISC_NUMBER}: no actionable signals in ${DISC_CATEGORY}, skipping"
      exit 0
    fi
  fi
fi

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
