#!/usr/bin/env bash
set -euo pipefail

: "${EVENT_NAME:?EVENT_NAME is required}"
: "${REPO:?REPO is required}"
: "${PR_JSON_PATH:?PR_JSON_PATH is required}"

pr_number=""
if [ "$EVENT_NAME" = "workflow_run" ]; then
  : "${HEAD_SHA:?HEAD_SHA is required for workflow_run}"
  pr_number=$(gh api "/repos/${REPO}/commits/${HEAD_SHA}/pulls" --jq '.[0].number // empty')
elif [ "$EVENT_NAME" = "schedule" ]; then
  now=$(date -u +%s)
  while IFS=$'\t' read -r candidate sha created_at; do
    [ -n "$candidate" ] || continue
    created_epoch=$(date -u -d "$created_at" +%s)
    [ $((now - created_epoch)) -ge 86400 ] || continue

    successful_run=$(gh api --method GET \
      "repos/${REPO}/actions/workflows/ci.yml/runs" \
      -f head_sha="$sha" \
      -f event='pull_request' \
      -f status='completed' \
      -f per_page='10' \
      --jq '.workflow_runs | first | select(.conclusion == "success") | .id // empty')
    if [ -n "$successful_run" ]; then
      pr_number="$candidate"
      break
    fi
  done < <(gh pr list --repo "$REPO" --state open --author 'app/dependabot' \
    --limit 100 --json number,headRefOid,createdAt \
    --jq '.[] | [.number, .headRefOid, .createdAt] | @tsv')
else
  echo "Unsupported event: $EVENT_NAME" >&2
  exit 2
fi

if [ -z "$pr_number" ]; then
  exit 0
fi

gh pr view "$pr_number" --repo "$REPO" \
  --json createdAt,labels,body,author,title,headRefName,headRefOid,state \
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

if [ "$EVENT_NAME" = "workflow_run" ] \
  && [ "$(jq -r .headRefOid "$PR_JSON_PATH")" != "$HEAD_SHA" ]; then
  echo "PR #${pr_number} head no longer matches the successful CI SHA; refusing" >&2
  exit 1
fi

printf '%s\n' "$pr_number"
