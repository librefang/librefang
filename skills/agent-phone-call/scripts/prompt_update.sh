#!/bin/bash
set -euo pipefail

if [ $# -lt 1 ]; then
  echo "use: ./prompt_update.sh <message>"
  exit 1
fi

MESSAGE="$1"
MAX_RETRIES=20

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
  # Use jq to safely construct JSON (prevents shell injection)
  BODY=$(jq -n --arg prompt "$MESSAGE" '{"prompt": $prompt}')

  HTTP_RESPONSE=$(curl -s -w "\n%{http_code}" -X POST https://api.pollyreach.ai/platform/v1/users/phone/pollyreach/update \
    -H "Content-Type: application/json" \
    -H "Authorization: Bearer $TOKEN" \
    -d "$BODY") || { sleep 2; continue; }

  HTTP_CODE=$(echo "$HTTP_RESPONSE" | tail -1)
  RESPONSE=$(echo "$HTTP_RESPONSE" | sed '$d')

  if [ "$HTTP_CODE" -eq 401 ]; then
    echo "$RESPONSE"
    exit 1
  fi

  if [ "$HTTP_CODE" -lt 200 ] || [ "$HTTP_CODE" -ge 300 ]; then
    sleep 2
    continue
  fi

  echo "$RESPONSE"
  exit 0
done

echo "prompt_update.sh exited with no completion after $MAX_RETRIES retries."
exit 1
