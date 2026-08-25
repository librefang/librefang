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

send_error() {
    jq -cn --arg message "$1" '{method:"error",params:{message:$message}}'
}

# Read commands from stdin
while IFS= read -r line; do
    [ -z "$line" ] && continue

    if ! method=$(printf '%s\n' "$line" | jq -er 'select(type == "object" and (.method | type == "string")) | .method'); then
        send_error "Invalid command: expected an object with a string method"
        continue
    fi

    case "$method" in
        send)
            if ! printf '%s\n' "$line" | jq -e '.params | type == "object"' >/dev/null; then
                send_error "Invalid send params: expected an object"
                continue
            fi
            text=$(printf '%s\n' "$line" | jq -r '.params.text // ""')
            channel_id=$(printf '%s\n' "$line" | jq -r '.params.channel_id // "default"')
            jq -cn --arg text "Echo: ${text}" --arg channel_id "$channel_id" \
                '{method:"message",params:{user_id:"echo-user",user_name:"Echo Bot (Bash)",text:$text,channel_id:$channel_id}}'
            ;;
        shutdown)
            exit 0
            ;;
        *)
            send_error "Unsupported command: ${method}"
            ;;
    esac
done
