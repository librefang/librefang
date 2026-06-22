#!/bin/bash
set -euo pipefail

MAX_RETRIES=300

# Check for jq dependency
if ! command -v jq &> /dev/null; then
  echo "error: please install jq"
  echo "  macOS: brew install jq"
  echo "  Ubuntu: sudo apt install jq"
  exit 1
fi

# Read token from credentials file (never pass token as CLI arg)
KEY_FILE="${POLLYREACH_KEY_FILE:-$HOME/.config/PollyReach/key.json}"
if [ ! -f "$KEY_FILE" ]; then
  echo "error: credentials file not found: $KEY_FILE"
  echo "Please complete PollyReach registration first."
  exit 1
fi
TOKEN=$(jq -r '.token // empty' "$KEY_FILE")
if [ -z "$TOKEN" ]; then
  echo "error: token not found in $KEY_FILE"
  exit 1
fi

for i in $(seq 1 $MAX_RETRIES); do
  RESPONSE=$(curl -s -X POST https://api.pollyreach.ai/platform/v1/chat/openclaw/query \
    -H "Content-Type: application/json" \
    -H "Authorization: Bearer $TOKEN" \
    -d "{}") || { sleep 2; continue; }

  DONE=$(echo "$RESPONSE" | jq -r '.done // false')

  if [ "$DONE" = "true" ]; then
    echo "$RESPONSE"
    exit 0
  fi

  sleep 2
done

echo "query.sh exited with no completion after $MAX_RETRIES retries."
exit 1
