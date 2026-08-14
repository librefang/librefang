#!/bin/sh
set -eu

input='[]
{"method":"unknown"}
{"method":"send","params":{"text":"quote \" and slash \\","channel_id":"chat"}}'

check_adapter() {
    output=$(printf '%s\n' "$input" | "$@")
    printf '%s\n' "$output" | jq -s -e '
        length == 4 and
        .[0].method == "ready" and
        .[1].method == "error" and
        .[2].method == "error" and
        .[3].method == "message" and
        .[3].params.channel_id == "chat" and
        .[3].params.text == "Echo: quote \" and slash \\"
    ' >/dev/null
}

check_adapter node examples/sidecar-channel-node/adapter.js
check_adapter python3 examples/sidecar-channel-python/adapter.py
check_adapter bash examples/sidecar-channel-bash/adapter.sh
check_adapter go run examples/sidecar-channel-go/adapter.go
