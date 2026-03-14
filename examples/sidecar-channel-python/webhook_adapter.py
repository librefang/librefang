#!/usr/bin/env python3
"""HTTP Webhook sidecar channel adapter for LibreFang.

Starts a simple HTTP server that receives POST requests and forwards
them as messages to LibreFang. Useful for integrating any service
that supports webhooks (GitHub, Stripe, custom apps, etc.).

Requirements:
    No external dependencies (stdlib only)

Usage in config.toml:
    [[sidecar_channels]]
    name = "webhook"
    command = "python3"
    args = ["examples/sidecar-channel-python/webhook_adapter.py"]
    env = { WEBHOOK_PORT = "9090", WEBHOOK_SECRET = "my-secret" }

Environment variables:
    WEBHOOK_PORT    - Port to listen on (default: 9090)
    WEBHOOK_SECRET  - Shared secret for HMAC validation (optional)

Send messages to this adapter:
    curl -X POST http://localhost:9090/webhook \
      -H "Content-Type: application/json" \
      -d '{"user": "ci-bot", "text": "Build #123 passed", "channel": "builds"}'
"""
import hashlib
import hmac
import json
import os
import sys
import threading
from http.server import HTTPServer, BaseHTTPRequestHandler

WEBHOOK_PORT = int(os.environ.get("WEBHOOK_PORT", "9090"))
WEBHOOK_SECRET = os.environ.get("WEBHOOK_SECRET", "")


def send_event(method, params=None):
    """Send an event to LibreFang via stdout."""
    event = {"method": method}
    if params:
        event["params"] = params
    print(json.dumps(event), flush=True)


class WebhookHandler(BaseHTTPRequestHandler):
    """Handle incoming webhook POST requests."""

    def do_POST(self):
        content_length = int(self.headers.get("Content-Length", 0))
        body = self.rfile.read(content_length)

        # HMAC validation if secret is configured
        if WEBHOOK_SECRET:
            sig = self.headers.get("X-Signature", "")
            expected = hmac.new(
                WEBHOOK_SECRET.encode(), body, hashlib.sha256
            ).hexdigest()
            if not hmac.compare_digest(sig, expected):
                self.send_response(401)
                self.end_headers()
                self.wfile.write(b"Invalid signature")
                return

        try:
            data = json.loads(body)
        except json.JSONDecodeError:
            self.send_response(400)
            self.end_headers()
            self.wfile.write(b"Invalid JSON")
            return

        # Forward as message event
        send_event("message", {
            "user_id": data.get("user", "webhook"),
            "user_name": data.get("user", "Webhook"),
            "text": data.get("text", json.dumps(data)),
            "channel_id": data.get("channel", "webhook"),
            "platform": "webhook",
        })

        self.send_response(200)
        self.end_headers()
        self.wfile.write(b"OK")

    def log_message(self, format, *args):
        """Suppress default access logs — use stderr for errors only."""
        pass


def handle_command(cmd):
    """Handle a command from LibreFang."""
    method = cmd.get("method")
    if method == "send":
        # Webhook is receive-only; log outbound for debugging
        params = cmd.get("params", {})
        sys.stderr.write(f"[webhook] outbound (no-op): {params.get('text', '')}\n")
        sys.stderr.flush()
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
    send_event("ready")

    # Start HTTP server in background
    server = HTTPServer(("0.0.0.0", WEBHOOK_PORT), WebhookHandler)
    server_thread = threading.Thread(target=server.serve_forever, daemon=True)
    server_thread.start()
    sys.stderr.write(f"[webhook] listening on port {WEBHOOK_PORT}\n")
    sys.stderr.flush()

    # Main thread reads commands from LibreFang
    read_stdin()


if __name__ == "__main__":
    main()
