#!/usr/bin/env python3
"""Telegram Bot API sidecar channel adapter for LibreFang.

A first-party adapter on the ``librefang.sidecar`` SDK (same shape as
``librefang.sidecar.adapters.ntfy``): the framework owns the
ready/ack handshake, supervised restart, and stdout protocol framing;
this module owns the Telegram transport.

* Inbound: long-poll ``getUpdates``; each text message becomes a
  ChannelMessage (``platform = "telegram"``, ``channel_id`` = chat id,
  sender = Telegram ``first_name`` / ``username`` / ``"unknown"``).
  ``ALLOWED_USERS`` whitelists by numeric user id.
* Outbound: ``sendMessage`` with ``parse_mode = "Markdown"``.

Stdlib-only (the SDK has zero runtime deps — no ``requests``).
Configure via ``[[sidecar_channels]]``:

    [[sidecar_channels]]
    name = "telegram"
    command = "python3"
    args = ["-m", "librefang.sidecar.adapters.telegram"]
    channel_type = "telegram"
    [sidecar_channels.env]
    TELEGRAM_BOT_TOKEN = "123456:ABC-..."   # from @BotFather (required)
    # ALLOWED_USERS = "111,222"             # optional id whitelist
"""
from __future__ import annotations

import asyncio
import json
import os
import socket
import urllib.error
import urllib.parse
import urllib.request

from librefang.sidecar import Content, SidecarAdapter, protocol, run_stdio
from librefang.sidecar import logging as log

# Telegram holds getUpdates open ~30s server-side; give the client a
# little more before treating it as a (normal) long-poll timeout.
LONGPOLL_SERVER_SECS = 30
LONGPOLL_CLIENT_SECS = 35
SEND_TIMEOUT_SECS = 10


def _api_get(url: str, params: dict, timeout: float) -> dict:
    """GET returning parsed JSON. Mirrors the old `requests` behaviour
    of *not* raising on HTTP status (Telegram returns ``{"ok":false}``
    with a 4xx, which the caller surfaces). A client read timeout is
    re-raised as ``TimeoutError`` so the poll loop can treat it as a
    normal empty long-poll rather than an error."""
    full = f"{url}?{urllib.parse.urlencode(params)}"
    try:
        with urllib.request.urlopen(full, timeout=timeout) as resp:  # noqa: S310
            return json.loads(resp.read().decode("utf-8", "replace"))
    except urllib.error.HTTPError as e:
        body = e.read().decode("utf-8", "replace")
        try:
            return json.loads(body)
        except ValueError:
            return {"ok": False, "error": f"HTTP {e.code}: {body}"}
    except urllib.error.URLError as e:
        if isinstance(e.reason, (TimeoutError, socket.timeout)):
            raise TimeoutError(str(e.reason)) from e
        raise


def _api_post_json(url: str, payload: dict, timeout: float) -> None:
    """POST a JSON body. Response is intentionally unused (the old
    adapter ignored it too); failures raise for the SDK to log."""
    req = urllib.request.Request(
        url,
        data=json.dumps(payload).encode("utf-8"),
        headers={"Content-Type": "application/json"},
        method="POST",
    )
    try:
        with urllib.request.urlopen(req, timeout=timeout):  # noqa: S310
            pass
    except urllib.error.HTTPError as e:
        body = e.read().decode("utf-8", "replace")
        raise RuntimeError(f"telegram sendMessage {e.code}: {body}") from e


class TelegramAdapter(SidecarAdapter):
    # Plain text only, matching the former adapter — no typing /
    # reaction / interactive / streaming. Declaring nothing makes
    # LibreFang route plain text and degrade everything else.
    capabilities: list = []

    def __init__(self) -> None:
        self.token = os.environ.get("TELEGRAM_BOT_TOKEN", "").strip()
        raw = os.environ.get("ALLOWED_USERS", "").strip()
        self.allowed_ids = {u.strip() for u in raw.split(",") if u.strip()}
        if not self.token:
            log.error("TELEGRAM_BOT_TOKEN is required; exiting")
            raise SystemExit(2)
        self.api_base = f"https://api.telegram.org/bot{self.token}"

    # ---- inbound: long-poll getUpdates -------------------------------

    def _update_to_event(self, update: dict):
        """One Telegram `update` → a `message` event dict, or None
        (non-text, or filtered by the user whitelist)."""
        msg = update.get("message")
        if not msg or not msg.get("text"):
            return None
        user = update.get("message", {}).get("from", {}) or {}
        uid = str(user.get("id", ""))
        if self.allowed_ids and uid not in self.allowed_ids:
            return None
        uname = (
            user.get("first_name") or user.get("username") or "unknown"
        )
        return protocol.message(
            user_id=uid,
            user_name=uname,
            content=Content.text(msg["text"]),
            channel_id=str(msg.get("chat", {}).get("id", "")),
            platform="telegram",
        )

    def _poll_once(self, emit, state: dict) -> None:
        """One blocking long-poll round (runs in an executor thread).
        Advances `state["offset"]`. Raises TimeoutError on a normal
        empty long-poll; RuntimeError on a Telegram API error."""
        data = _api_get(
            f"{self.api_base}/getUpdates",
            {"offset": state["offset"], "timeout": LONGPOLL_SERVER_SECS},
            LONGPOLL_CLIENT_SECS,
        )
        if not data.get("ok"):
            raise RuntimeError(f"Telegram API error: {data}")
        for update in data.get("result", []):
            state["offset"] = update.get("update_id", state["offset"]) + 1
            ev = self._update_to_event(update)
            if ev:
                emit(ev)

    async def produce(self, emit) -> None:
        loop = asyncio.get_event_loop()
        state = {"offset": 0}
        backoff = 1.0
        while True:
            try:
                await loop.run_in_executor(
                    None, self._poll_once, emit, state
                )
                backoff = 1.0
            except asyncio.CancelledError:
                raise
            except TimeoutError:
                # Normal empty long-poll — re-issue immediately.
                backoff = 1.0
                continue
            except Exception as e:  # noqa: BLE001 - transport errors vary
                log.warn(
                    "telegram poll error; backing off",
                    error=str(e),
                    delay=backoff,
                )
                await asyncio.sleep(backoff)
                backoff = min(backoff * 2, 120.0)

    # ---- outbound: sendMessage ---------------------------------------

    def _send(self, chat_id: str, text: str) -> None:
        _api_post_json(
            f"{self.api_base}/sendMessage",
            {"chat_id": chat_id, "text": text, "parse_mode": "Markdown"},
            SEND_TIMEOUT_SECS,
        )

    async def on_send(self, cmd) -> None:
        # Plain-text only, like the former adapter; structured content
        # the platform can't render falls back to a placeholder.
        if cmd.content and not (
            isinstance(cmd.content, dict) and "Text" in cmd.content
        ):
            text = "(Unsupported content type)"
        else:
            text = cmd.text or ""
        chat_id = cmd.channel_id
        if not chat_id or not text:
            return
        await asyncio.get_event_loop().run_in_executor(
            None, self._send, chat_id, text
        )


if __name__ == "__main__":
    run_stdio(TelegramAdapter())
