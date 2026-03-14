#!/bin/bash
# Example sidecar channel adapter for LibreFang (Bash)
#
# The simplest possible adapter — just bash + jq.
#
# Requirements: jq
#
# Usage in config.toml:
#   [[sidecar_channels]]
#   name = "bash-echo"
#   command = "bash"
#   args = ["examples/sidecar-channel-bash/adapter.sh"]

# Signal readiness
echo '{"method":"ready"}'

# Read commands from stdin
while IFS= read -r line; do
    [ -z "$line" ] && continue

    method=$(echo "$line" | jq -r '.method // empty')

    case "$method" in
        send)
            text=$(echo "$line" | jq -r '.params.text // ""')
            channel_id=$(echo "$line" | jq -r '.params.channel_id // "default"')
            echo "{\"method\":\"message\",\"params\":{\"user_id\":\"echo-user\",\"user_name\":\"Echo Bot (Bash)\",\"text\":\"Echo: ${text}\",\"channel_id\":\"${channel_id}\"}}"
            ;;
        shutdown)
            exit 0
            ;;
    esac
done
