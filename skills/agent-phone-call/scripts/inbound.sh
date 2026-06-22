#!/bin/bash
set -euo pipefail

MAX_RETRIES=10

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
    HTTP_RESPONSE=$(curl -s -w "\n%{http_code}" -X GET \
        "https://api.pollyreach.ai/platform/v1/sms_messages/unread" \
        -H "Authorization: Bearer $TOKEN") || { sleep 2; continue; }

    HTTP_CODE=$(echo "$HTTP_RESPONSE" | tail -1)
    RESPONSE_BODY=$(echo "$HTTP_RESPONSE" | sed '$d')

    if [ "$HTTP_CODE" -lt 200 ] || [ "$HTTP_CODE" -ge 300 ]; then
        sleep 2
        continue
    fi

    SUCCESS=$(echo "$RESPONSE_BODY" | jq -r '.success' 2>/dev/null)
    if [ "$SUCCESS" != "true" ]; then
        sleep 2
        continue
    fi

    MSG_COUNT=$(echo "$RESPONSE_BODY" | jq '.messages | length' 2>/dev/null)
    if [ -z "$MSG_COUNT" ] || [ "$MSG_COUNT" -eq 0 ]; then
        sleep 2
        continue
    fi

    COMBINED_CONTENT=""
    for ((j=0; j<MSG_COUNT; j++)); do
        CONTENT=$(echo "$RESPONSE_BODY" | jq -r ".messages[$j].content")
        FROM_PHONE=$(echo "$RESPONSE_BODY" | jq -r ".messages[$j].from_phone")
        COMBINED_CONTENT+="$FROM_PHONE: $CONTENT | "
    done

    COMBINED_CONTENT=${COMBINED_CONTENT% | }
    echo "========================================"
    echo "$COMBINED_CONTENT"

    exit 0
done

exit 1
