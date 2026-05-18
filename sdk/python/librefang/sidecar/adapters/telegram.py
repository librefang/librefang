#!/usr/bin/env python3
"""Telegram Bot API sidecar channel adapter for LibreFang.

A first-party adapter on the ``librefang.sidecar`` SDK (same shape as
``librefang.sidecar.adapters.ntfy``). The framework owns the ready/ack
handshake, supervised restart, and stdout protocol framing; this
module owns the Telegram transport.

PARITY STATUS (telegram-sidecar migration, increment 1 of N):
* DONE — outbound rich capabilities, declared via ``capabilities``:
  ``typing`` (sendChatAction), ``reaction`` (setMessageReaction with
  the same emoji-mapping table as the in-process Rust adapter, incl.
  optional clear-on-done), ``interactive`` (inline keyboards),
  ``thread`` (forum-topic ``message_thread_id``), ``streaming``
  (throttled editMessageText, UTF-16-aware 4096 chunking).
* DONE — inbound text long-poll (``getUpdates``), ``ALLOWED_USERS``
  whitelist, sender / channel_id / platform mapping.
* NOT YET (follow-on increments, tracked before in-process
  ``crates/librefang-channels/src/telegram.rs`` can be removed):
  faithful Markdown→Telegram-HTML formatter subsystem (this increment
  sends ``parse_mode = "Markdown"`` like the pre-migration adapter,
  not the Rust ``formatter::TelegramHtml`` pipeline); full inbound
  parsing of non-text updates (media / callback_query / poll / edited)
  into the corresponding ``ChannelContent`` variants.

Stdlib-only (the SDK has zero runtime deps — no ``requests``).
Configure via ``[[sidecar_channels]]``:

    [[sidecar_channels]]
    name = "telegram"
    command = "python3"
    args = ["-m", "librefang.sidecar.adapters.telegram"]
    channel_type = "telegram"
    [sidecar_channels.env]
    TELEGRAM_BOT_TOKEN = "123456:ABC-..."     # from @BotFather (required)
    # ALLOWED_USERS = "111,222"               # optional id whitelist
    # TELEGRAM_CLEAR_DONE_REACTION = "1"      # clear ✅ instead of 🎉
"""
from __future__ import annotations

import asyncio
import json
import os
import socket
import time
import urllib.error
import urllib.parse
import urllib.request

from librefang.sidecar import SidecarAdapter, protocol, run_stdio
from librefang.sidecar import logging as log

LONGPOLL_SERVER_SECS = 30
LONGPOLL_CLIENT_SECS = 35
SEND_TIMEOUT_SECS = 10
# Telegram's message limit is 4096 *UTF-16 code units* (not chars).
TELEGRAM_MSG_LIMIT = 4096
# Throttle streamed editMessageText (mirrors the Rust adapter's 1s).
STREAM_EDIT_INTERVAL = 1.0


def _utf16_len(s: str) -> int:
    """Length of `s` in UTF-16 code units (chars > U+FFFF count as 2)."""
    return sum(2 if ord(c) > 0xFFFF else 1 for c in s)


def _chunks16(text: str, limit: int = TELEGRAM_MSG_LIMIT) -> list[str]:
    """Split `text` so every chunk is <= `limit` UTF-16 code units,
    preferring a newline boundary — the stdlib equivalent of the Rust
    ``split_to_utf16_chunks`` used by the in-process adapter."""
    if _utf16_len(text) <= limit:
        return [text]
    chunks: list[str] = []
    cur: list[str] = []
    cur_len = 0
    last_nl = -1  # index in `cur` just after the last '\n'
    for ch in text:
        w = 2 if ord(ch) > 0xFFFF else 1
        if cur_len + w > limit:
            if last_nl > 0:
                chunks.append("".join(cur[:last_nl]).rstrip("\n"))
                cur = cur[last_nl:]
            else:
                chunks.append("".join(cur))
                cur = []
            cur_len = _utf16_len("".join(cur))
            last_nl = -1
        cur.append(ch)
        cur_len += w
        if ch == "\n":
            last_nl = len(cur)
    if cur:
        chunks.append("".join(cur))
    return [c for c in chunks if c != ""]


# Telegram only supports a limited reaction-emoji set; map the
# lifecycle emoji exactly like the in-process Rust adapter
# (`map_reaction_emoji`).
_REACTION_MAP = {
    "⏳": "\U0001f440",            # ⏳ → 👀
    "⚙️": "⚡",          # ⚙️ → ⚡
    "✅": "\U0001f389",            # ✅ → 🎉
    "❌": "\U0001f44e",            # ❌ → 👎
}
_DONE_EMOJI = "✅"  # ✅


def _map_reaction(emoji: str) -> str:
    return _REACTION_MAP.get(emoji, emoji)


def _api_get(url: str, params: dict, timeout: float) -> dict:
    """GET returning parsed JSON. Does not raise on HTTP status
    (Telegram returns ``{"ok":false}`` with a 4xx); a client read
    timeout is re-raised as ``TimeoutError`` for the long-poll loop."""
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


def _api_post(url: str, payload: dict, timeout: float) -> dict:
    """POST a JSON body, returning the parsed Telegram response (used
    for the ``message_id`` of a streamed message). Raises on transport
    / HTTP error so the SDK supervisor logs + backs off."""
    req = urllib.request.Request(
        url,
        data=json.dumps(payload).encode("utf-8"),
        headers={"Content-Type": "application/json"},
        method="POST",
    )
    try:
        with urllib.request.urlopen(req, timeout=timeout) as resp:  # noqa: S310
            return json.loads(resp.read().decode("utf-8", "replace"))
    except urllib.error.HTTPError as e:
        body = e.read().decode("utf-8", "replace")
        raise RuntimeError(f"telegram API {e.code}: {body}") from e


class TelegramAdapter(SidecarAdapter):
    # Outbound rich features the in-process adapter also exposed. Each
    # is wired below; LibreFang routes the matching command here
    # instead of degrading to plain text.
    capabilities = ["typing", "reaction", "interactive", "thread", "streaming"]

    def __init__(self) -> None:
        self.token = os.environ.get("TELEGRAM_BOT_TOKEN", "").strip()
        raw = os.environ.get("ALLOWED_USERS", "").strip()
        self.allowed_ids = {u.strip() for u in raw.split(",") if u.strip()}
        self.clear_done = os.environ.get(
            "TELEGRAM_CLEAR_DONE_REACTION", ""
        ).strip().lower() in ("1", "true", "yes")
        if not self.token:
            log.error("TELEGRAM_BOT_TOKEN is required; exiting")
            raise SystemExit(2)
        self.api_base = f"https://api.telegram.org/bot{self.token}"
        # stream_id -> {chat_id, thread_id, text, msg_id, last_edit}
        self._streams: dict = {}

    # ---- low-level API (blocking; called via executor) ---------------

    def _call(self, method: str, payload: dict) -> dict:
        return _api_post(
            f"{self.api_base}/{method}", payload, SEND_TIMEOUT_SECS
        )

    def _send_text(self, chat_id, text: str, thread_id=None) -> dict:
        """sendMessage with UTF-16-aware chunking + optional forum
        thread. Returns the last Telegram response."""
        last: dict = {}
        for chunk in _chunks16(text):
            payload = {
                "chat_id": chat_id,
                "text": chunk,
                "parse_mode": "Markdown",
            }
            if thread_id:
                payload["message_thread_id"] = thread_id
            last = self._call("sendMessage", payload)
        return last

    def _edit_text(self, chat_id, message_id, text: str) -> None:
        self._call("editMessageText", {
            "chat_id": chat_id,
            "message_id": message_id,
            "text": text,
            "parse_mode": "Markdown",
        })

    # ---- inbound: long-poll getUpdates -------------------------------

    def _update_to_event(self, update: dict):
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
            content=protocol.Content.text(msg["text"]),
            channel_id=str(msg.get("chat", {}).get("id", "")),
            platform="telegram",
        )

    def _poll_once(self, emit, state: dict) -> None:
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
                backoff = 1.0
                continue
            except Exception as e:  # noqa: BLE001 - transport errors vary
                log.warn("telegram poll error; backing off",
                         error=str(e), delay=backoff)
                await asyncio.sleep(backoff)
                backoff = min(backoff * 2, 120.0)

    # ---- outbound: rich command dispatch -----------------------------

    @staticmethod
    def _flatten(cmd) -> str:
        if cmd.content and not (
            isinstance(cmd.content, dict) and "Text" in cmd.content
        ):
            return "(Unsupported content type)"
        return cmd.text or ""

    def _inline_keyboard(self, message: dict) -> dict:
        rows = []
        for row in message.get("buttons", []) or []:
            out = []
            for b in row:
                btn = {"text": b.get("label", "")}
                if b.get("url"):
                    btn["url"] = b["url"]
                else:
                    btn["callback_data"] = b.get("action", "")
                out.append(btn)
            rows.append(out)
        return {"inline_keyboard": rows}

    def _do_reaction(self, cmd) -> None:
        clear = cmd.reaction == _DONE_EMOJI and self.clear_done
        reaction = [] if clear else [
            {"type": "emoji", "emoji": _map_reaction(cmd.reaction)}
        ]
        self._call("setMessageReaction", {
            "chat_id": cmd.channel_id,
            "message_id": int(cmd.message_id),
            "reaction": reaction,
        })

    def _do_interactive(self, cmd) -> None:
        msg = cmd.message or {}
        self._call("sendMessage", {
            "chat_id": cmd.channel_id,
            "text": msg.get("text", ""),
            "parse_mode": "Markdown",
            "reply_markup": self._inline_keyboard(msg),
        })

    def _stream_delta(self, sid: str, chunk: str) -> None:
        st = self._streams.get(sid)
        if st is None:
            return
        st["text"] += chunk
        if st["msg_id"] is None:
            resp = self._send_text(st["chat_id"], st["text"], st["thread_id"])
            st["msg_id"] = (resp.get("result") or {}).get("message_id")
            st["last_edit"] = time.monotonic()
        elif time.monotonic() - st["last_edit"] >= STREAM_EDIT_INTERVAL:
            self._edit_text(st["chat_id"], st["msg_id"], st["text"][
                :TELEGRAM_MSG_LIMIT * 2])
            st["last_edit"] = time.monotonic()

    def _stream_end(self, sid: str) -> None:
        st = self._streams.pop(sid, None)
        if st is None or not st["text"]:
            return
        chunks = _chunks16(st["text"])
        if st["msg_id"] is not None:
            self._edit_text(st["chat_id"], st["msg_id"], chunks[0])
            for extra in chunks[1:]:
                self._send_text(st["chat_id"], extra, st["thread_id"])
        else:
            self._send_text(st["chat_id"], st["text"], st["thread_id"])

    async def on_command(self, cmd) -> None:
        loop = asyncio.get_event_loop()
        if isinstance(cmd, protocol.Send):
            text = self._flatten(cmd)
            chat_id = cmd.channel_id
            if not chat_id or not text:
                return
            await loop.run_in_executor(
                None, self._send_text, chat_id, text, cmd.thread_id
            )
        elif isinstance(cmd, protocol.TypingCmd):
            await loop.run_in_executor(None, self._call, "sendChatAction",
                                       {"chat_id": cmd.channel_id,
                                        "action": "typing"})
        elif isinstance(cmd, protocol.Reaction):
            await loop.run_in_executor(None, self._do_reaction, cmd)
        elif isinstance(cmd, protocol.Interactive):
            await loop.run_in_executor(None, self._do_interactive, cmd)
        elif isinstance(cmd, protocol.StreamStart):
            self._streams[cmd.stream_id] = {
                "chat_id": cmd.channel_id, "thread_id": None,
                "text": "", "msg_id": None, "last_edit": 0.0,
            }
        elif isinstance(cmd, protocol.StreamDelta):
            await loop.run_in_executor(
                None, self._stream_delta, cmd.stream_id, cmd.text
            )
        elif isinstance(cmd, protocol.StreamEnd):
            await loop.run_in_executor(
                None, self._stream_end, cmd.stream_id
            )
        else:
            await super().on_command(cmd)

    # `on_send` kept so the SDK base's default Send routing still works
    # if a subclass/caller bypasses on_command.
    async def on_send(self, cmd) -> None:
        text = self._flatten(cmd)
        if not cmd.channel_id or not text:
            return
        await asyncio.get_event_loop().run_in_executor(
            None, self._send_text, cmd.channel_id, text, cmd.thread_id
        )


if __name__ == "__main__":
    run_stdio(TelegramAdapter())
