#!/bin/bash
set -euo pipefail

MAX_RETRIES=10
SLEEP=2

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
  HTTP_RESPONSE=$(curl -s -w "\n%{http_code}" -X GET https://api.pollyreach.ai/platform/v1/credits/balance \
    -H "Authorization: Bearer $TOKEN") || { sleep "$SLEEP"; SLEEP=$((SLEEP * 2 > 30 ? 30 : SLEEP * 2)); continue; }

  HTTP_CODE=$(echo "$HTTP_RESPONSE" | tail -1)
  RESPONSE=$(echo "$HTTP_RESPONSE" | sed '$d')

  if [ "$HTTP_CODE" -lt 200 ] || [ "$HTTP_CODE" -ge 300 ]; then
    sleep "$SLEEP"
    SLEEP=$((SLEEP * 2 > 30 ? 30 : SLEEP * 2))
    continue
  fi

  BALANCE=$(echo "$RESPONSE" | jq -r '.balance // 0')

  if [ "$(echo "$BALANCE > 0" | bc)" -eq 1 ]; then
    echo "token is active, balance: $BALANCE"
  else
    echo "token is inactive"
  fi
  exit 0
done

echo "balance.sh exited with no valid balance after $MAX_RETRIES retries."
exit 1
