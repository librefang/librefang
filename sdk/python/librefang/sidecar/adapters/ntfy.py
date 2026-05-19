#!/usr/bin/env python3
"""ntfy.sh sidecar channel adapter for LibreFang.

Replaces the former in-process Rust `librefang-channels::ntfy` adapter
(removed in the sidecar-first migration). Behaviour is preserved:

* Inbound: subscribe to ``{server}/{topic}/sse``; ``event:"message"``
  with a non-empty ``message`` becomes a ChannelMessage. A leading
  ``/`` makes it a Command; ``title`` (or ``ntfy-user``) is the sender;
  ``is_group`` is true; ``topic`` goes into metadata.
* Outbound: POST plain-text to ``{server}/{topic}`` with a
  ``Title: LibreFang`` header, chunked at 4096 chars.
* Optional Bearer token for protected topics; optional ``account_id``
  for multi-bot routing. Reconnects with exponential backoff.

Stdlib-only (the SDK has zero runtime deps). Configure via
``[[sidecar_channels]]``:

    [[sidecar_channels]]
    name = "ntfy"
    command = "python3"
    args = ["-m", "librefang.sidecar.adapters.ntfy"]
    channel_type = "ntfy"
    [sidecar_channels.env]
    NTFY_TOPIC = "my-topic"
    # NTFY_SERVER_URL = "https://ntfy.sh"   # default
    # NTFY_TOKEN = "tk_..."                 # omit for public topics
    # NTFY_ACCOUNT_ID = "topic-42"          # multi-bot routing
"""
from __future__ import annotations

import asyncio
import json
import os
import time
import urllib.error
import urllib.request

from librefang.sidecar import Content, Field, Schema, SidecarAdapter, protocol, run_stdio_main
from librefang.sidecar import logging as log

MAX_MESSAGE_LEN = 4096
DEFAULT_SERVER_URL = "https://ntfy.sh"


def _split_message(text: str, max_len: int) -> list[str]:
    """Chunk `text` into <= max_len pieces, preferring newline splits.
    Mirrors the Rust `split_message` shared helper closely enough for
    ntfy (which has no markup to keep intact)."""
    if len(text) <= max_len:
        return [text]
    chunks: list[str] = []
    rest = text
    while len(rest) > max_len:
        window = rest[:max_len]
        cut = window.rfind("\n")
        if cut <= 0:
            cut = max_len
        chunks.append(rest[:cut])
        rest = rest[cut:].lstrip("\n") if cut < max_len else rest[cut:]
    if rest:
        chunks.append(rest)
    return chunks


class NtfyAdapter(SidecarAdapter):
    # ntfy has no typing/reaction/interactive/thread/streaming concept
    # — declare nothing, so LibreFang routes plain text only.
    capabilities: list = []

    SCHEMA = Schema(
        name="ntfy",
        display_name="ntfy",
        description="ntfy.sh pub/sub notifications (out-of-process sidecar)",
        fields=[
            Field("NTFY_TOPIC", "Topic", "text",
                  required=True, placeholder="my-topic"),
            Field("NTFY_SERVER_URL", "Server URL", "text",
                  placeholder="https://ntfy.sh", advanced=True),
            Field("NTFY_TOKEN", "Auth Token", "secret",
                  placeholder="tk_...", advanced=True),
            Field("NTFY_ACCOUNT_ID", "Account ID (multi-bot)", "text",
                  placeholder="topic-42", advanced=True),
        ],
    )

    def __init__(self) -> None:
        server = os.environ.get("NTFY_SERVER_URL", "").strip()
        self.server_url = (
            server.rstrip("/") if server else DEFAULT_SERVER_URL
        )
        self.topic = os.environ.get("NTFY_TOPIC", "").strip()
        self.token = os.environ.get("NTFY_TOKEN", "").strip()
        acct = os.environ.get("NTFY_ACCOUNT_ID", "").strip()
        # Surfaced to LibreFang via the `ready` event (multi-bot key).
        self.account_id = acct or None
        if not self.topic:
            log.error("NTFY_TOPIC is required; exiting")
            raise SystemExit(2)

    # ---- inbound: SSE subscription -----------------------------------

    def _auth_headers(self) -> dict:
        return {"Authorization": f"Bearer {self.token}"} if self.token else {}

    def _parse_event(self, raw: str):
        """ntfy SSE `data:` JSON → (id, message, topic, title) or None
        (skips open/keepalive/empty, matches the Rust parser)."""
        try:
            val = json.loads(raw)
        except (ValueError, TypeError):
            return None
        if val.get("event") != "message":
            return None
        msg = val.get("message")
        mid = val.get("id")
        if not msg or not mid:
            return None
        return (
            str(mid),
            str(msg),
            str(val.get("topic", "")),
            val.get("title"),
        )

    def _to_event(self, mid, message, topic, title) -> dict:
        sender = title if title else "ntfy-user"
        if message.startswith("/"):
            head, _, tail = message[1:].partition(" ")
            content = Content.command(head, tail.split() if tail else [])
        else:
            content = Content.text(message)
        return protocol.message(
            user_id=sender,
            user_name=sender,
            content=content,
            is_group=True,
            metadata={"topic": topic or self.topic},
        )

    def _sse_loop(self, emit) -> None:
        """Blocking SSE read (runs in an executor thread). One pass; the
        caller wraps it in reconnect backoff."""
        url = f"{self.server_url}/{self.topic}/sse"
        req = urllib.request.Request(url, headers=self._auth_headers())
        # No read timeout: SSE is a long-lived stream.
        with urllib.request.urlopen(req) as resp:  # noqa: S310 - configured URL
            if getattr(resp, "status", 200) != 200:
                raise RuntimeError(f"ntfy SSE HTTP {resp.status}")
            log.info("ntfy SSE connected", topic=self.topic)
            data = ""
            for rawline in resp:
                line = rawline.decode("utf-8", "replace").rstrip("\r\n")
                if line.startswith("data: "):
                    data = line[6:]
                elif line == "" and data:
                    parsed = self._parse_event(data)
                    data = ""
                    if parsed:
                        emit(self._to_event(*parsed))

    async def produce(self, emit) -> None:
        loop = asyncio.get_event_loop()
        backoff = 1.0
        while True:
            try:
                await loop.run_in_executor(None, self._sse_loop, emit)
                backoff = 1.0  # clean stream end → reconnect promptly
            except asyncio.CancelledError:
                raise
            except Exception as e:  # noqa: BLE001 - transport errors vary
                log.warn(
                    "ntfy SSE error; backing off",
                    error=str(e),
                    delay=backoff,
                )
                await asyncio.sleep(backoff)
                backoff = min(backoff * 2, 120.0)

    # ---- outbound: publish -------------------------------------------

    def _publish(self, text: str) -> None:
        url = f"{self.server_url}/{self.topic}"
        for chunk in _split_message(text, MAX_MESSAGE_LEN):
            headers = {
                "Content-Type": "text/plain",
                "Title": "LibreFang",
            }
            headers.update(self._auth_headers())
            req = urllib.request.Request(
                url,
                data=chunk.encode("utf-8"),
                headers=headers,
                method="POST",
            )
            try:
                with urllib.request.urlopen(req) as resp:  # noqa: S310
                    if getattr(resp, "status", 200) >= 300:
                        raise RuntimeError(f"ntfy publish HTTP {resp.status}")
            except urllib.error.HTTPError as e:
                body = e.read().decode("utf-8", "replace")
                raise RuntimeError(f"ntfy publish {e.code}: {body}") from e

    async def on_send(self, cmd) -> None:
        # Plain-text only, like the Rust adapter; structured content
        # the platform can't render falls back to a placeholder.
        if cmd.content and not (
            isinstance(cmd.content, dict) and "Text" in cmd.content
        ):
            text = "(Unsupported content type)"
        else:
            text = cmd.text or ""
        await asyncio.get_event_loop().run_in_executor(
            None, self._publish, text
        )


if __name__ == "__main__":
    run_stdio_main(NtfyAdapter)
