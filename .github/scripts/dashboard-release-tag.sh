#!/bin/sh
set -eu

: "${EVENT_NAME:?EVENT_NAME is required}"
: "${REPOSITORY:?REPOSITORY is required}"

case "$EVENT_NAME" in
  release|workflow_dispatch)
    printf '%s\n' "${EVENT_RELEASE_TAG:-}"
    ;;
  push)
    gh release list \
      --repo "$REPOSITORY" \
      --exclude-drafts \
      --exclude-pre-releases \
      --limit 1 \
      --json tagName \
      --jq '.[0].tagName // empty'
    ;;
  *)
    echo "Unsupported dashboard release event: $EVENT_NAME" >&2
    exit 2
    ;;
esac
