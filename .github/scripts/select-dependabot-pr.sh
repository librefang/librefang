#!/usr/bin/env bash
set -euo pipefail

: "${EVENT_NAME:?EVENT_NAME is required}"
: "${REPO:?REPO is required}"
: "${PR_JSON_PATH:?PR_JSON_PATH is required}"

pr_number=""
selected_sha=""

latest_ci_succeeded() {
  local sha=$1 conclusion
  conclusion=$(gh api --method GET \
    "repos/${REPO}/actions/workflows/ci.yml/runs" \
    -f head_sha="$sha" \
    -f event='pull_request' \
    -f per_page='1' \
    --jq '.workflow_runs[0].conclusion // empty')
  [ "$conclusion" = "success" ]
}

if [ "$EVENT_NAME" = "workflow_run" ]; then
  : "${HEAD_SHA:?HEAD_SHA is required for workflow_run}"
  selected_sha="$HEAD_SHA"
  pr_number=$(gh api "/repos/${REPO}/commits/${HEAD_SHA}/pulls" \
    --jq '[.[] | select(.state == "open" and .user.login == "dependabot[bot]")][0].number // empty')
  if [ -n "$pr_number" ] && ! latest_ci_succeeded "$selected_sha"; then
    pr_number=""
  fi
elif [ "$EVENT_NAME" = "schedule" ] || [ "$EVENT_NAME" = "workflow_dispatch" ]; then
  now=$(date -u +%s)
  while IFS=$'\t' read -r candidate sha created_at; do
    [ -n "$candidate" ] || continue
    created_epoch=$(date -u -d "$created_at" +%s)
    [ $((now - created_epoch)) -ge 86400 ] || continue

    if latest_ci_succeeded "$sha"; then
      pr_number="$candidate"
      selected_sha="$sha"
      break
    fi
  done < <(gh pr list --repo "$REPO" --state open --author 'app/dependabot' \
    --limit 1000 --json number,headRefOid,createdAt \
    --jq '.[] | [.number, .headRefOid, .createdAt] | @tsv')
else
  echo "Unsupported event: $EVENT_NAME" >&2
  exit 2
fi

if [ -z "$pr_number" ]; then
  exit 0
fi

gh pr view "$pr_number" --repo "$REPO" \
  --json createdAt,author,title,headRefName,headRefOid,state \
  > "$PR_JSON_PATH"

if ! jq -e '
  .state == "OPEN" and
  .author.is_bot == true and
  .author.login == "app/dependabot" and
  (.headRefName | startswith("dependabot/"))
' "$PR_JSON_PATH" >/dev/null; then
  echo "PR #${pr_number} is not authored from the Dependabot app branch; refusing" >&2
  exit 1
fi

if [ "$(jq -r .headRefOid "$PR_JSON_PATH")" != "$selected_sha" ]; then
  echo "PR #${pr_number} head no longer matches the CI-tested SHA; refusing" >&2
  exit 1
fi

printf '%s\n' "$pr_number"
