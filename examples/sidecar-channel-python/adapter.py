#!/usr/bin/env python3
"""Example sidecar channel adapter for LibreFang.

Reads commands from stdin (JSON, one per line) and writes events to stdout.
This example creates a simple echo adapter for testing.

Protocol:
  - stdout: sidecar -> LibreFang (events)
  - stdin:  LibreFang -> sidecar (commands)

Events (stdout):
  {"method": "ready"}
  {"method": "message", "params": {"user_id": "...", "user_name": "...", "text": "...", "channel_id": "..."}}
  {"method": "error", "params": {"message": "..."}}

Commands (stdin):
  {"method": "send", "params": {"channel_id": "...", "text": "..."}}
  {"method": "shutdown"}
"""
import json
import sys


def send_event(method, params=None):
    """Send an event to LibreFang via stdout."""
    event = {"method": method}
    if params:
        event["params"] = params
    print(json.dumps(event), flush=True)


def handle_command(cmd):
    """Handle a command from LibreFang."""
    method = cmd.get("method")
    if method == "send":
        params = cmd.get("params", {})
        # Echo: pretend we sent it and got a reply
        send_event("message", {
            "user_id": "echo-user",
            "user_name": "Echo Bot",
            "text": f"Echo: {params.get('text', '')}",
            "channel_id": params.get("channel_id", "default"),
        })
    elif method == "shutdown":
        sys.exit(0)


def main():
    # Signal readiness
    send_event("ready")

    # Read commands from stdin
    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue
        try:
            cmd = json.loads(line)
            handle_command(cmd)
        except json.JSONDecodeError as e:
            send_event("error", {"message": f"Invalid JSON: {e}"})


if __name__ == "__main__":
    main()
