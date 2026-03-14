#!/usr/bin/env python3
"""Telegram sidecar channel adapter for LibreFang.

A real-world example showing how to bridge Telegram Bot API
to LibreFang via the sidecar JSON-RPC protocol.

Requirements:
    pip install requests

Usage in config.toml:
    [[sidecar_channels]]
    name = "telegram"
    command = "python3"
    args = ["examples/sidecar-channel-python/telegram_adapter.py"]
    env = { TELEGRAM_BOT_TOKEN = "your-bot-token-here" }

Environment variables:
    TELEGRAM_BOT_TOKEN  - Bot token from @BotFather (required)
    ALLOWED_USERS       - Comma-separated user IDs to whitelist (optional)
"""
import json
import os
import sys
import threading
import time

import requests

BOT_TOKEN = os.environ.get("TELEGRAM_BOT_TOKEN", "")
ALLOWED_USERS = os.environ.get("ALLOWED_USERS", "")
API_BASE = f"https://api.telegram.org/bot{BOT_TOKEN}"


def send_event(method, params=None):
    """Send an event to LibreFang via stdout."""
    event = {"method": method}
    if params:
        event["params"] = params
    print(json.dumps(event), flush=True)


def send_telegram(chat_id, text):
    """Send a message via Telegram Bot API."""
    try:
        requests.post(f"{API_BASE}/sendMessage", json={
            "chat_id": chat_id,
            "text": text,
            "parse_mode": "Markdown",
        }, timeout=10)
    except Exception as e:
        send_event("error", {"message": f"Telegram send failed: {e}"})


def poll_updates(allowed_ids):
    """Long-poll Telegram for new messages."""
    offset = 0
    while True:
        try:
            resp = requests.get(f"{API_BASE}/getUpdates", params={
                "offset": offset,
                "timeout": 30,
            }, timeout=35)
            data = resp.json()
            if not data.get("ok"):
                send_event("error", {"message": f"Telegram API error: {data}"})
                time.sleep(5)
                continue

            for update in data.get("result", []):
                offset = update["update_id"] + 1
                msg = update.get("message")
                if not msg or not msg.get("text"):
                    continue

                user = msg.get("from", {})
                user_id = str(user.get("id", ""))

                # Whitelist check
                if allowed_ids and user_id not in allowed_ids:
                    continue

                user_name = user.get("first_name", "") or user.get("username", "unknown")
                chat_id = str(msg["chat"]["id"])

                send_event("message", {
                    "user_id": user_id,
                    "user_name": user_name,
                    "text": msg["text"],
                    "channel_id": chat_id,
                    "platform": "telegram",
                })

        except requests.exceptions.Timeout:
            continue
        except Exception as e:
            send_event("error", {"message": f"Poll error: {e}"})
            time.sleep(5)


def handle_command(cmd):
    """Handle a command from LibreFang."""
    method = cmd.get("method")
    if method == "send":
        params = cmd.get("params", {})
        chat_id = params.get("channel_id", "")
        text = params.get("text", "")
        if chat_id and text:
            send_telegram(chat_id, text)
    elif method == "shutdown":
        sys.exit(0)


def read_stdin():
    """Read commands from LibreFang via stdin."""
    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue
        try:
            cmd = json.loads(line)
            handle_command(cmd)
        except json.JSONDecodeError as e:
            send_event("error", {"message": f"Invalid JSON: {e}"})


def main():
    if not BOT_TOKEN:
        send_event("error", {"message": "TELEGRAM_BOT_TOKEN not set"})
        sys.exit(1)

    # Parse allowed users
    allowed_ids = set()
    if ALLOWED_USERS:
        allowed_ids = {uid.strip() for uid in ALLOWED_USERS.split(",")}

    # Signal readiness
    send_event("ready")

    # Start polling in background thread
    poll_thread = threading.Thread(target=poll_updates, args=(allowed_ids,), daemon=True)
    poll_thread.start()

    # Main thread reads commands from LibreFang
    read_stdin()


if __name__ == "__main__":
    main()
