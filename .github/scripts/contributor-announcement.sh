#!/usr/bin/env bash
set -euo pipefail

if [[ -z "${DISCORD_WEBHOOK_URL:-}" ]]; then
  exit 0
fi

: "${GH_TOKEN:?GH_TOKEN is required}"
: "${GITHUB_REPOSITORY:?GITHUB_REPOSITORY is required}"
: "${PR_AUTHOR:?PR_AUTHOR is required}"
: "${PR_TITLE:?PR_TITLE is required}"
: "${PR_NUMBER:?PR_NUMBER is required}"
: "${PR_URL:?PR_URL is required}"

if [[ ! "$PR_NUMBER" =~ ^[1-9][0-9]*$ ]]; then
  echo "Invalid PR number: $PR_NUMBER" >&2
  exit 2
fi

# Exclude this PR explicitly. Search indexing may not include a merge
# immediately, but any older merged PR remains enough to classify the
# author as a returning contributor.
search_response=$(gh api --method GET search/issues \
  -f q="repo:${GITHUB_REPOSITORY} author:${PR_AUTHOR} is:pr is:merged" \
  -F per_page=2)

prior_count=$(jq -er --argjson current "$PR_NUMBER" '
  if .incomplete_results != false or
     (.total_count | type) != "number" or .total_count < 0 or (.total_count | floor) != .total_count or
     (.items | type) != "array" or .total_count < (.items | length) or
     (.total_count > 0 and (.items | length) == 0) or
     any(.items[]; (.number | type) != "number" or .number < 1 or (.number | floor) != .number)
  then
    error("GitHub returned an incomplete contributor search")
  else
    [.items[] | select(.number != $current)] | length
  end
' <<< "$search_response")

escape_discord_markdown() {
  local input=$1 escaped='' character index
  for ((index = 0; index < ${#input}; index++)); do
    character=${input:index:1}
    case "$character" in
      "\\"|"*"|"_"|"~"|'`'|"|"|">"|"["|"]"|"("|")"|"#"|"-")
        escaped+="\\$character"
        ;;
      *)
        escaped+="$character"
        ;;
    esac
  done
  printf '%s' "$escaped"
}

safe_title=$(escape_discord_markdown "$PR_TITLE")

if (( prior_count == 0 )); then
  printf -v content \
    '🎉 **Welcome our newest contributor!** @%s just merged their first PR!\n\n**%s** ([#%s](%s))\n\nThank you for contributing to LibreFang! 🦊' \
    "$PR_AUTHOR" "$safe_title" "$PR_NUMBER" "$PR_URL"
else
  printf -v content '✅ **PR Merged:** %s ([#%s](%s)) by @%s' \
    "$safe_title" "$PR_NUMBER" "$PR_URL" "$PR_AUTHOR"
fi

jq -n --arg content "$content" \
  '{content: $content, allowed_mentions: {parse: []}}' | \
  curl --fail-with-body --silent --show-error --max-time 20 \
    -X POST -H 'Content-Type: application/json' \
    --data-binary @- "$DISCORD_WEBHOOK_URL"
