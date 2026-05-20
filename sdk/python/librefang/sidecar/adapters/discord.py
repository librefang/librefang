#!/usr/bin/env python3
"""Discord Gateway sidecar channel adapter for LibreFang.

Replaces the former in-process Rust ``librefang-channels::discord``
adapter (removed in this sidecar migration; same pattern as ntfy
#5224, telegram #5241, gotify #5263, mastodon #5264, bluesky #5277,
reddit #5281).

Behaviour parity with the Rust adapter:

* **Gateway**: ``GET https://discord.com/api/v10/gateway/bot`` with
  ``Bot <token>`` returns a WSS URL; connect with
  ``?v=10&encoding=json``. JSON text frames only (no compression).
* **Handshake**: server ``HELLO`` (op 10) carries
  ``heartbeat_interval`` ms. We start a periodic heartbeat and send
  ``IDENTIFY`` (op 2) with ``{token, intents, properties}``, or
  ``RESUME`` (op 6) when ``session_id`` + ``seq`` + ``resume_url``
  survive across reconnects.
* **Inbound dispatch**: ``MESSAGE_CREATE`` and ``MESSAGE_UPDATE``
  produce ``message`` events; ``READY`` populates the bot user id /
  session id / resume url; ``RESUMED``, ``HEARTBEAT_ACK``,
  ``RECONNECT``, ``INVALID_SESSION`` follow Discord's protocol.
* **Filters** (mirroring the Rust ``parse_discord_message``):
  self-skip via ``READY.user.id``; ``ignore_bots`` skips other bots'
  messages; ``allowed_users`` / ``allowed_guilds`` whitelists when
  non-empty.
* **Content extraction**: attachments take priority over slash
  commands — ``/cmd args`` with no attachment is a ``Command``,
  attachment-only or attachment+text yields the matching media
  variant (``Image`` / ``Video`` / ``Voice`` / ``File``).
* **Mention detection**: ``mentions`` array contains the bot's id, or
  the message body contains ``<@bot_id>`` / ``<@!bot_id>``, or any
  ``mention_patterns`` substring (case-insensitive). Sets
  ``was_mentioned = true`` in metadata so the MentionOnly policy can
  enforce it.
* **Display name**: ``username#discriminator`` for legacy users
  (discriminator != "0"), bare ``username`` for new-style users.
* **REST send**: ``POST /channels/{channel_id}/messages`` with the
  ``Bot <token>`` header and a ``{"content": "..."}`` JSON body.
  Discord caps each message at 2000 UTF-16 code units; we chunk on
  that boundary (matches ``split_to_utf16_chunks`` in the Rust crate).

Improvement over the Rust adapter
=================================

The Rust adapter captured ``heartbeat_interval`` from ``HELLO`` but
never spawned a client-side heartbeat task — it only responded when
Discord asked (op 1 ``HEARTBEAT``), which the gateway rarely does.
Connections silently dropped after ~45 s with ``code=4000`` and
re-IDENTIFY'd, losing the session. This sidecar runs a proper
periodic ``HEARTBEAT`` (interval scaled by Discord's mandated jitter
on the first beat) so sessions actually survive long-running idle
periods.

Stdlib-only: HTTPS via ``urllib.request``, WebSocket via a
hand-rolled RFC 6455 client over ``socket`` + ``ssl``.

Configure via ``[[sidecar_channels]]``::

    [[sidecar_channels]]
    name = "discord"
    command = "python3"
    args = ["-m", "librefang.sidecar.adapters.discord"]
    channel_type = "discord"
    [sidecar_channels.env]
    # DISCORD_ALLOWED_GUILDS = "123,456"
    # DISCORD_ALLOWED_USERS = "789"
    # DISCORD_MENTION_PATTERNS = "hey bot, !ask"
    # DISCORD_INTENTS = "37376"
    # DISCORD_IGNORE_BOTS = "true"
    # DISCORD_ACCOUNT_ID = "guild-42"

Secret via ``~/.librefang/secrets.env``: ``DISCORD_BOT_TOKEN``.
"""
from __future__ import annotations

import asyncio
import base64
import hashlib
import json
import os
import random
import select
import socket
import ssl
import struct
import threading
import time
import urllib.error
import urllib.parse
import urllib.request
from typing import Any, Callable, Optional

from librefang.sidecar import Content, Field, Schema, SidecarAdapter, protocol, run_stdio_main
from librefang.sidecar import logging as log

# Discord REST + Gateway constants. ``DISCORD_API_BASE`` is overridable
# via the ``api_base`` instance attribute so unit tests can point at a
# mock without monkey-patching urllib.
DEFAULT_API_BASE = "https://discord.com/api/v10"
GATEWAY_QUERY = "?v=10&encoding=json"

# Discord enforces a 2000-character ceiling per message *measured in
# UTF-16 code units*, not bytes. A single emoji is 1 char in Python but
# 2 UTF-16 units, so we have to count surrogates.
DISCORD_MESSAGE_LIMIT = 2000

# Gateway opcodes — mirror crate::discord::opcode.
OP_DISPATCH = 0
OP_HEARTBEAT = 1
OP_IDENTIFY = 2
OP_RESUME = 6
OP_RECONNECT = 7
OP_INVALID_SESSION = 9
OP_HELLO = 10
OP_HEARTBEAT_ACK = 11

# Default intents: GUILD_MESSAGES | DIRECT_MESSAGES | MESSAGE_CONTENT
# (= 1 << 9 | 1 << 12 | 1 << 15 = 37376). Matches the Rust default.
DEFAULT_INTENTS = 37376

# Send-path timeouts (REST). Discord's gateway has no Send timeout —
# we own the connection lifetime via heartbeats.
SEND_TIMEOUT_SECS = 15.0
HANDSHAKE_TIMEOUT_SECS = 15.0

# Backoff (seconds) for the reconnect loop. Initial 1 s, doubles per
# consecutive failure, capped at MAX_BACKOFF_SECS. Matches Rust
# ``with_backoff`` defaults.
INITIAL_BACKOFF_SECS = 1.0
MAX_BACKOFF_SECS = 60.0

# RFC 6455 — Sec-WebSocket-Accept derivation magic GUID and frame
# opcodes. Same as gotify's hand-rolled reader (#5263).
_WS_GUID = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11"
_OP_CONT = 0x0
_OP_TEXT = 0x1
_OP_BIN = 0x2
_OP_CLOSE = 0x8
_OP_PING = 0x9
_OP_PONG = 0xA

# Cap on a single inbound WS frame's payload length. Discord frames
# are short JSON (the heaviest realistic event is a MESSAGE_CREATE
# with embeds + attachments, well under 100 KiB). 4 MiB guards
# against a hostile server that announces a 64-bit length to make
# us spin reading multi-exabyte payloads.
MAX_FRAME_PAYLOAD = 1 << 22  # 4 MiB

# Gateway close codes that are NOT recoverable by reconnecting — the
# operator must fix the config first. Mapping per Discord docs.
# Closing with one of these and immediately reconnecting just burns
# the gateway budget and produces noisy logs; we surface a single
# clear error and let the supervisor's circuit-breaker stop us.
FATAL_CLOSE_CODES = {
    4004: "authentication failed — DISCORD_BOT_TOKEN is invalid",
    4010: "invalid shard",
    4011: "sharding required",
    4012: "invalid API version",
    4013: "invalid intents — bitmask is not a valid combination",
    4014: "disallowed intents — enable Privileged Gateway Intents in the "
          "Discord Developer Portal (Message Content / Server Members / "
          "Presence) before requesting them",
}


def _split_to_utf16_chunks(text: str, limit: int) -> list[str]:
    """Chunk `text` into pieces whose UTF-16 code-unit length is
    <= `limit`. Mirrors crate::message_truncator::split_to_utf16_chunks.

    Discord measures message length in UTF-16 code units (legacy JS
    string length), not bytes. A single ``"\U0001F600"`` (😀) is 1
    char in Python but 2 UTF-16 units, so we must walk Unicode
    code-points and count surrogate-pair widths.
    """
    if not text:
        return [""]
    chunks: list[str] = []
    current: list[str] = []
    current_len = 0
    for ch in text:
        # In UTF-16, code points above the BMP need a surrogate pair.
        unit = 2 if ord(ch) > 0xFFFF else 1
        if current_len + unit > limit:
            if current:
                chunks.append("".join(current))
            current = [ch]
            current_len = unit
        else:
            current.append(ch)
            current_len += unit
    if current:
        chunks.append("".join(current))
    return chunks or [""]


def _split_command(content: str) -> dict[str, Any]:
    """``/cmd arg1 arg2`` → ``Command{name: "cmd", args: [...]}``.

    First space delimits name from args; args are whitespace-split.
    Mirrors the Rust ``parse_discord_message`` slash branch.
    """
    parts = content.split(" ", 1)
    name = parts[0][1:]  # drop leading '/'
    args = parts[1].split() if len(parts) > 1 else []
    return Content.command(name, args)


def parse_attachment(
    attachments: list[dict], companion_text: str
) -> tuple[dict[str, Any], bool]:
    """Convert the first attachment + companion text into a
    ``ChannelContent`` variant. Returns ``(content, ok)`` — ``ok`` is
    False if the attachment list is empty or the URL is missing (the
    caller falls back to text).

    Mirrors ``crate::discord::parse_discord_attachment``: first
    attachment only (a warning is logged for extras), MIME prefix
    decides the variant, only Image keeps the companion text as
    ``caption`` (audio/file warn on companion text because they have
    no caption channel).
    """
    if len(attachments) > 1:
        log.warn(
            "discord: additional attachment(s) ignored, only first processed",
            extras=len(attachments) - 1,
        )
    att = attachments[0] if attachments else None
    if not isinstance(att, dict):
        return Content.text(companion_text), False
    url = str(att.get("url") or "")
    if not url:
        log.warn("discord: attachment has empty URL, falling back to text")
        return Content.text(companion_text), False
    filename = str(att.get("filename") or "attachment")
    content_type = str(att.get("content_type") or "")
    caption = companion_text or None

    if content_type.startswith("image/"):
        return Content.image(url, caption=caption, mime_type=content_type), True
    if content_type.startswith("video/"):
        return Content.video(url, caption=caption, duration_seconds=0,
                             filename=filename), True
    if content_type.startswith("audio/"):
        if companion_text:
            log.warn(
                "discord: audio attachment has companion text that cannot "
                "be sent as caption",
                companion_text=companion_text,
            )
        return Content.voice(url, caption=None, duration_seconds=0), True
    if companion_text:
        log.warn(
            "discord: file attachment has companion text that cannot be "
            "sent as caption",
            companion_text=companion_text,
        )
    return Content.file(url, filename), True


def parse_message_create(
    d: dict,
    *,
    bot_user_id: Optional[str],
    allowed_guilds: list[str],
    allowed_users: list[str],
    ignore_bots: bool,
    mention_patterns: list[str],
    account_id: Optional[str],
) -> Optional[dict]:
    """Mirror of the Rust ``parse_discord_message``.

    Returns the ``message`` event dict (ready to ``emit``) or ``None``
    when the payload should be skipped. Pure function so tests can
    exercise every filter / variant without standing up a gateway.
    """
    author = d.get("author")
    if not isinstance(author, dict):
        return None
    author_id = author.get("id")
    if not isinstance(author_id, str):
        return None
    # Self-skip (always, even when ignore_bots is False).
    if bot_user_id and author_id == bot_user_id:
        return None
    # Filter other bots.
    if ignore_bots and author.get("bot") is True:
        return None
    # allowed_users whitelist (when non-empty).
    if allowed_users and author_id not in allowed_users:
        return None
    # allowed_guilds whitelist (when non-empty and the message is in a guild).
    guild_id = d.get("guild_id")
    if allowed_guilds and isinstance(guild_id, str):
        if guild_id not in allowed_guilds:
            return None

    content_text = str(d.get("content") or "")
    attachments = d.get("attachments")
    has_attachments = isinstance(attachments, list) and len(attachments) > 0
    if not content_text and not has_attachments:
        return None

    channel_id = d.get("channel_id")
    if not isinstance(channel_id, str):
        return None
    message_id = str(d.get("id") or "0")
    username = str(author.get("username") or "Unknown")
    discriminator = str(author.get("discriminator") or "0000")
    display_name = (
        username if discriminator == "0" else f"{username}#{discriminator}"
    )

    if has_attachments:
        content, _ok = parse_attachment(attachments, content_text)
    elif content_text.startswith("/"):
        content = _split_command(content_text)
    else:
        content = Content.text(content_text)

    is_group = isinstance(guild_id, str)

    # Mention detection: mentions array contains bot_id OR body has
    # <@bot_id> / <@!bot_id> OR any mention_pattern substring (case-insensitive).
    was_mentioned = False
    if bot_user_id:
        mentions_arr = d.get("mentions")
        if isinstance(mentions_arr, list):
            for m in mentions_arr:
                if isinstance(m, dict) and m.get("id") == bot_user_id:
                    was_mentioned = True
                    break
        if not was_mentioned:
            if (f"<@{bot_user_id}>" in content_text
                    or f"<@!{bot_user_id}>" in content_text):
                was_mentioned = True
    if not was_mentioned and mention_patterns:
        lower = content_text.lower()
        for pat in mention_patterns:
            if pat and pat.lower() in lower:
                was_mentioned = True
                break

    metadata: dict[str, Any] = {}
    if was_mentioned:
        metadata["was_mentioned"] = True
    if account_id is not None:
        metadata["account_id"] = account_id

    return protocol.message(
        user_id=channel_id,  # Rust adapter uses channel_id as platform_id
        user_name=display_name,
        content=content,
        message_id=message_id,
        is_group=is_group,
        metadata=metadata or None,
    )


# ---------------------------------------------------------------------------
# Stdlib WebSocket client. Adapted from gotify (#5263); same RFC 6455
# logic with two Discord-specific additions:
#   * the iterator yields raw text frames (Discord's gateway frames
#     carry JSON one event per frame, so a frame-level iterator is
#     enough — no continuation handling needed in practice but we
#     support it for correctness);
#   * ``send_text(s)`` is exposed so callers (the heartbeat sender
#     and the IDENTIFY/RESUME emitters) can push frames back.
# ---------------------------------------------------------------------------


class _WebSocketClient:
    """Minimal RFC 6455 client (text-only). Use as a context manager.

    Server→client frames are never masked; client→server frames MUST
    be masked with a fresh 4-byte key per frame.
    """

    def __init__(
        self,
        url: str,
        *,
        headers: Optional[dict] = None,
        handshake_timeout: float = HANDSHAKE_TIMEOUT_SECS,
    ) -> None:
        self.url = url
        self.headers = dict(headers or {})
        self._sock: Optional[socket.socket] = None
        self._leftover = b""
        self._handshake_timeout = handshake_timeout
        self._send_lock = threading.Lock()
        self.closed = False

    @staticmethod
    def _parse_url(url: str) -> tuple[str, int, str, bool]:
        u = urllib.parse.urlparse(url)
        scheme = u.scheme.lower()
        if scheme not in ("ws", "wss"):
            raise ValueError(f"not a websocket url: {url!r}")
        if not u.hostname:
            raise ValueError(f"websocket url missing host: {url!r}")
        is_tls = scheme == "wss"
        port = u.port or (443 if is_tls else 80)
        path = u.path or "/"
        if u.query:
            path += "?" + u.query
        return u.hostname, port, path, is_tls

    def __enter__(self) -> "_WebSocketClient":
        host, port, path, is_tls = self._parse_url(self.url)
        sock = socket.create_connection((host, port),
                                        timeout=self._handshake_timeout)
        if is_tls:
            ctx = ssl.create_default_context()
            sock = ctx.wrap_socket(sock, server_hostname=host)
        key = base64.b64encode(os.urandom(16)).decode("ascii")
        lines = [
            f"GET {path} HTTP/1.1",
            f"Host: {host}:{port}",
            "Upgrade: websocket",
            "Connection: Upgrade",
            f"Sec-WebSocket-Key: {key}",
            "Sec-WebSocket-Version: 13",
        ]
        for k, v in self.headers.items():
            lines.append(f"{k}: {v}")
        req = ("\r\n".join(lines) + "\r\n\r\n").encode("ascii")
        sock.sendall(req)
        buf = b""
        while b"\r\n\r\n" not in buf:
            chunk = sock.recv(4096)
            if not chunk:
                sock.close()
                raise RuntimeError("connection closed during ws handshake")
            buf += chunk
            if len(buf) > 65536:
                sock.close()
                raise RuntimeError("ws handshake response too large")
        head, _, leftover = buf.partition(b"\r\n\r\n")
        head_lines = head.split(b"\r\n")
        status = head_lines[0]
        if not status.startswith(b"HTTP/1.1 101 "):
            sock.close()
            raise RuntimeError(
                f"ws handshake failed: {status.decode('ascii', 'replace')}"
            )
        expected = base64.b64encode(
            hashlib.sha1((key + _WS_GUID).encode("ascii")).digest()
        ).decode("ascii")
        got = None
        for line in head_lines[1:]:
            name, _, val = line.partition(b":")
            if name.strip().lower() == b"sec-websocket-accept":
                got = val.strip().decode("ascii", "replace")
                break
        if got != expected:
            sock.close()
            raise RuntimeError("ws handshake Sec-WebSocket-Accept mismatch")
        self._sock = sock
        self._leftover = leftover
        return self

    def __exit__(self, *_exc) -> None:
        self.closed = True
        if self._sock is not None:
            try:
                self._sock.close()
            except OSError:
                pass
            self._sock = None

    def settimeout(self, timeout: Optional[float]) -> None:
        if self._sock is not None:
            self._sock.settimeout(timeout)

    def wait_readable(self, timeout: float) -> bool:
        """Return True when bytes are ready (or already buffered),
        False on a clean timeout. Used to gate the heartbeat tick
        BEFORE we start consuming a frame, so a mid-frame stall
        becomes a hard transport error instead of corrupting state.

        TLS-on-Python is the awkward bit: ``ssl.SSLSocket.pending()``
        exposes already-decrypted bytes that aren't visible to
        ``select`` (TLS records can come back from the OS but stay
        in the SSL layer's read buffer). We check leftover-from-
        handshake first, then ``ssl.pending()``, then ``select``.
        """
        if self._leftover:
            return True
        sock = self._sock
        if sock is None:
            return False
        # SSLSocket exposes already-decrypted bytes via `.pending()` —
        # those don't show up in select() because the OS has already
        # given them to the SSL layer.
        pending = getattr(sock, "pending", None)
        if callable(pending):
            try:
                if pending() > 0:
                    return True
            except Exception:  # noqa: BLE001 — closed socket etc.
                pass
        try:
            r, _, _ = select.select([sock], [], [], max(0.0, timeout))
        except (OSError, ValueError):
            # ``select`` rejects closed/negative-fd sockets — treat as
            # "no data, will fail on next recv".
            return False
        return bool(r)

    def _recv_exact(self, n: int) -> bytes:
        if n <= 0:
            return b""
        buf = bytearray()
        while len(buf) < n:
            if self._leftover:
                take = min(n - len(buf), len(self._leftover))
                buf.extend(self._leftover[:take])
                self._leftover = self._leftover[take:]
                continue
            assert self._sock is not None
            chunk = self._sock.recv(n - len(buf))
            if not chunk:
                raise EOFError("websocket closed mid-frame")
            buf.extend(chunk)
        return bytes(buf)

    def _send_frame(self, opcode: int, payload: bytes) -> None:
        assert self._sock is not None
        header = bytearray([0x80 | (opcode & 0x0F)])
        ln = len(payload)
        if ln < 126:
            header.append(0x80 | ln)
        elif ln < 65536:
            header.append(0x80 | 126)
            header.extend(struct.pack(">H", ln))
        else:
            header.append(0x80 | 127)
            header.extend(struct.pack(">Q", ln))
        mask = os.urandom(4)
        header.extend(mask)
        masked = bytes(b ^ mask[i % 4] for i, b in enumerate(payload))
        with self._send_lock:
            self._sock.sendall(bytes(header) + masked)

    def send_text(self, s: str) -> None:
        self._send_frame(_OP_TEXT, s.encode("utf-8"))

    def send_close(self) -> None:
        try:
            self._send_frame(_OP_CLOSE, b"")
        except OSError:
            pass

    def recv_frame(self) -> tuple[Optional[str], Optional[tuple[int, bytes]]]:
        """Read one frame and return either ``(text, None)`` for a
        completed text message, or ``(None, (close_code, reason))``
        for a close frame the server sent. Pings are answered inline
        and skipped. Returns ``(None, None)`` for non-text frames we
        ignore (binary, pong)."""
        h2 = self._recv_exact(2)
        fin = (h2[0] & 0x80) != 0
        opcode = h2[0] & 0x0F
        masked = (h2[1] & 0x80) != 0
        ln = h2[1] & 0x7F
        if ln == 126:
            ln = struct.unpack(">H", self._recv_exact(2))[0]
        elif ln == 127:
            ln = struct.unpack(">Q", self._recv_exact(8))[0]
        if ln > MAX_FRAME_PAYLOAD:
            raise RuntimeError(
                f"websocket frame payload {ln} exceeds cap "
                f"{MAX_FRAME_PAYLOAD}; failing the stream"
            )
        mask_key = self._recv_exact(4) if masked else None
        payload = self._recv_exact(ln)
        if mask_key is not None:
            payload = bytes(
                b ^ mask_key[i % 4] for i, b in enumerate(payload)
            )
        if opcode == _OP_PING:
            self._send_frame(_OP_PONG, payload)
            return None, None
        if opcode == _OP_PONG:
            return None, None
        if opcode == _OP_CLOSE:
            code = 1005  # "no status received" if payload < 2 bytes
            reason = b""
            if len(payload) >= 2:
                code = struct.unpack(">H", payload[:2])[0]
                reason = payload[2:]
            return None, (code, reason)
        if opcode == _OP_TEXT:
            # Discord frames are always one event per frame (no
            # multi-frame fragmentation), but support continuation
            # for spec correctness.
            buf = bytearray(payload)
            while not fin:
                h2 = self._recv_exact(2)
                fin = (h2[0] & 0x80) != 0
                opcode2 = h2[0] & 0x0F
                masked2 = (h2[1] & 0x80) != 0
                ln2 = h2[1] & 0x7F
                if ln2 == 126:
                    ln2 = struct.unpack(">H", self._recv_exact(2))[0]
                elif ln2 == 127:
                    ln2 = struct.unpack(">Q", self._recv_exact(8))[0]
                if ln2 > MAX_FRAME_PAYLOAD:
                    raise RuntimeError("ws continuation payload too large")
                mk = self._recv_exact(4) if masked2 else None
                payload2 = self._recv_exact(ln2)
                if mk is not None:
                    payload2 = bytes(
                        b ^ mk[i % 4] for i, b in enumerate(payload2)
                    )
                if opcode2 != _OP_CONT:
                    # Unexpected interleaved frame — bail.
                    raise RuntimeError(
                        f"ws unexpected interleaved opcode {opcode2}"
                    )
                buf.extend(payload2)
            return buf.decode("utf-8", "replace"), None
        # Binary / unknown — ignore.
        return None, None


# ---------------------------------------------------------------------------
# Discord adapter
# ---------------------------------------------------------------------------


class DiscordAdapter(SidecarAdapter):
    # Discord supports typing indicators (POST /channels/{id}/typing)
    # but no reactions / interactive / streaming in this initial
    # migration — matches the Rust adapter's surface (it only
    # implements send + send_typing).
    capabilities: list = ["typing"]

    SCHEMA = Schema(
        name="discord",
        display_name="Discord",
        description="Discord Gateway bot adapter (out-of-process sidecar)",
        fields=[
            Field("DISCORD_BOT_TOKEN", "Bot Token", "secret",
                  required=True,
                  placeholder="MTIz..."),
            Field("DISCORD_ALLOWED_GUILDS",
                  "Allowed Guild IDs (comma-separated, empty = allow all)",
                  "text",
                  placeholder="123456789, 987654321",
                  advanced=True),
            Field("DISCORD_ALLOWED_USERS",
                  "Allowed User IDs (comma-separated, empty = allow all)",
                  "text",
                  placeholder="123456789, 987654321",
                  advanced=True),
            Field("DISCORD_INTENTS",
                  f"Gateway intents bitmask (default {DEFAULT_INTENTS} = "
                  "GUILD_MESSAGES | DIRECT_MESSAGES | MESSAGE_CONTENT)",
                  "number",
                  placeholder=str(DEFAULT_INTENTS),
                  advanced=True),
            Field("DISCORD_IGNORE_BOTS",
                  "Ignore messages from other bots (default true)",
                  "bool",
                  placeholder="true",
                  advanced=True),
            Field("DISCORD_MENTION_PATTERNS",
                  "Custom mention patterns (comma-separated; "
                  "case-insensitive contains match)",
                  "text",
                  placeholder="hey bot, !ask",
                  advanced=True),
            Field("DISCORD_ACCOUNT_ID",
                  "Account ID (multi-bot routing)",
                  "text",
                  placeholder="guild-42",
                  advanced=True),
        ],
    )

    def __init__(self) -> None:
        token = os.environ.get("DISCORD_BOT_TOKEN", "").strip()
        if not token:
            log.error("DISCORD_BOT_TOKEN is required")
            raise SystemExit(2)
        self.token = token
        self.allowed_guilds = _split_csv(
            os.environ.get("DISCORD_ALLOWED_GUILDS", "")
        )
        self.allowed_users = _split_csv(
            os.environ.get("DISCORD_ALLOWED_USERS", "")
        )
        intents_raw = os.environ.get("DISCORD_INTENTS", "").strip()
        try:
            self.intents = (
                int(intents_raw) if intents_raw else DEFAULT_INTENTS
            )
        except (TypeError, ValueError):
            log.error("DISCORD_INTENTS must be an integer",
                      value=intents_raw)
            raise SystemExit(2) from None
        ignore_raw = os.environ.get("DISCORD_IGNORE_BOTS", "").strip().lower()
        # Default true; only explicit "false"/"0"/"no" disables.
        self.ignore_bots = ignore_raw not in ("false", "0", "no", "off")
        self.mention_patterns = [
            p.strip() for p in
            os.environ.get("DISCORD_MENTION_PATTERNS", "").split(",")
            if p.strip()
        ]
        acct = os.environ.get("DISCORD_ACCOUNT_ID", "").strip()
        self.account_id = acct or None

        # REST API base (overridable for tests).
        self.api_base = DEFAULT_API_BASE

        # Gateway session state — survives reconnects.
        self.bot_user_id: Optional[str] = None
        self.session_id: Optional[str] = None
        self.resume_gateway_url: Optional[str] = None
        self.last_seq: Optional[int] = None

        # NOTE: the Rust adapter cached a `channel_id → guild_id` map
        # for `ChannelRoleQuery::lookup_role` (live Discord guild role
        # → LibreFang role translation via the kernel's RBAC layer).
        # Sidecar adapters cannot implement that Rust trait, so the
        # cache and the live-lookup path are deliberately omitted
        # here; per-message Discord-role RBAC is unavailable in the
        # sidecar (matches the telegram-sidecar precedent #5241).
        # Operators who relied on it should configure explicit
        # [users.<id>] entries instead.

    # ---- HTTP helpers ------------------------------------------------

    def _auth_headers(self, *, content_type: bool = False) -> dict:
        h = {"Authorization": f"Bot {self.token}",
             "User-Agent": "librefang-discord-sidecar/1 (https://librefang.org)"}
        if content_type:
            h["Content-Type"] = "application/json"
        return h

    def _http(
        self,
        url: str,
        *,
        method: str = "GET",
        body: Optional[bytes] = None,
        headers: Optional[dict] = None,
        timeout: float = SEND_TIMEOUT_SECS,
    ) -> tuple[int, Any, bytes, dict]:
        """Return ``(status, parsed_json_or_None, raw_body, response_headers)``.

        Response header keys are normalised to lowercase. HTTPError is
        captured so callers can inspect 4xx/5xx without try/except.
        """
        req = urllib.request.Request(
            url, data=body, headers=headers or {}, method=method,
        )
        resp_headers: dict = {}
        try:
            with urllib.request.urlopen(  # noqa: S310 — configured URL
                req, timeout=timeout,
            ) as resp:
                status = getattr(resp, "status", 200)
                raw = resp.read()
                if resp.headers is not None:
                    resp_headers = {
                        k.lower(): v for k, v in resp.headers.items()
                    }
        except urllib.error.HTTPError as e:
            status = e.code
            try:
                raw = e.read()
            except Exception:  # noqa: BLE001
                raw = b""
            if e.headers is not None:
                resp_headers = {k.lower(): v for k, v in e.headers.items()}
        if not raw:
            return status, None, b"", resp_headers
        try:
            return status, json.loads(raw.decode("utf-8")), raw, resp_headers
        except (ValueError, TypeError, UnicodeDecodeError):
            return status, None, raw, resp_headers

    # ---- REST: gateway URL / send / typing / role lookup -----------

    def _fetch_gateway_url(self) -> str:
        """``GET /gateway/bot`` returns the WSS URL. Sub-bots that
        try to log in past their session-start limit get a 429 here;
        we surface the response as a RuntimeError so the supervisor
        backs off."""
        url = f"{self.api_base}/gateway/bot"
        status, body, raw, _hdrs = self._http(
            url, headers=self._auth_headers(),
        )
        if status != 200 or not isinstance(body, dict):
            snippet = raw[:200].decode("utf-8", "replace") if raw else ""
            raise RuntimeError(
                f"discord /gateway/bot failed (status={status}): {snippet}"
            )
        ws_url = body.get("url")
        if not isinstance(ws_url, str) or not ws_url:
            raise RuntimeError("discord /gateway/bot missing 'url'")
        return f"{ws_url}{GATEWAY_QUERY}"

    def _send_message(self, channel_id: str, text: str) -> None:
        """Chunk-aware ``POST /channels/{channel_id}/messages``. Discord
        rejects messages > 2000 UTF-16 code units; we split first."""
        chunks = _split_to_utf16_chunks(text, DISCORD_MESSAGE_LIMIT)
        url = f"{self.api_base}/channels/{channel_id}/messages"
        for chunk in chunks:
            payload = json.dumps({"content": chunk}).encode("utf-8")
            status, _resp, raw, resp_hdrs = self._http(
                url, method="POST", body=payload,
                headers=self._auth_headers(content_type=True),
            )
            if status == 429:
                # Discord-side rate-limit. Honour Retry-After and
                # retry once. The Rust adapter warned and dropped on
                # 429 today (see discord_send_does_not_retry_on_429_today);
                # this sidecar pulls that improvement forward since
                # operators were already complaining about lost messages.
                wait = _parse_retry_after(resp_hdrs, default_secs=2.0)
                log.warn("discord 429; sleeping then retrying once",
                         retry_after_secs=wait)
                time.sleep(wait)
                status, _resp, raw, resp_hdrs = self._http(
                    url, method="POST", body=payload,
                    headers=self._auth_headers(content_type=True),
                )
            if status >= 300:
                snippet = raw[:200].decode("utf-8", "replace") if raw else ""
                # Match the Rust adapter's fail-open behaviour: log
                # and continue (operators saw the existing warn-then-
                # carry-on shape, and chunks downstream still send).
                log.warn("discord send failed",
                         status=status, body=snippet)

    def _send_typing(self, channel_id: str) -> None:
        url = f"{self.api_base}/channels/{channel_id}/typing"
        try:
            self._http(
                url, method="POST",
                headers=self._auth_headers(content_type=True),
            )
        except Exception as e:  # noqa: BLE001 — typing is best-effort
            log.warn("discord typing indicator failed", error=str(e))

    # ---- Gateway loop -------------------------------------------------

    def _make_ws(self, url: str) -> _WebSocketClient:
        """Test seam — unit tests substitute a fake that yields a
        canned frame sequence so we can exercise the dispatch loop
        without a real network."""
        return _WebSocketClient(url)

    def _gateway_loop(self, emit: Callable[[dict], None]) -> None:
        """Connect → IDENTIFY/RESUME → dispatch loop → reconnect on
        clean drop. Runs in a worker thread. The asyncio side wraps
        this in ``loop.run_in_executor`` (see ``produce``)."""
        connect_url: Optional[str] = None
        backoff = INITIAL_BACKOFF_SECS
        # Outer reconnect loop.
        while True:
            try:
                if not connect_url:
                    connect_url = self._fetch_gateway_url()
                target = (
                    f"{self.resume_gateway_url}{GATEWAY_QUERY}"
                    if (self.session_id and self.last_seq is not None
                        and self.resume_gateway_url)
                    else connect_url
                )
                log.info("discord gateway connecting", url=target)
                with self._make_ws(target) as ws:
                    self._run_session(ws, emit)
                # Clean exit from the inner loop — try to reconnect
                # straight away with whatever session state survives.
                backoff = INITIAL_BACKOFF_SECS
            except _FatalGatewayError as e:
                # Don't reconnect — config-level problem. Surface
                # once and exit; the supervisor's circuit-breaker
                # stops us after restart_max_retries.
                log.error("discord gateway fatal", reason=str(e))
                raise
            except Exception as e:  # noqa: BLE001 — transport varies
                log.warn("discord gateway error; backing off",
                         error=str(e), delay=backoff)
                time.sleep(backoff)
                backoff = min(backoff * 2.0, MAX_BACKOFF_SECS)

    def _run_session(
        self, ws: _WebSocketClient, emit: Callable[[dict], None]
    ) -> None:
        """Drive one gateway session: HELLO → IDENTIFY/RESUME → loop.

        Heartbeat scheduling: ``select`` waits for the socket to
        become readable before we touch the frame reader. A clean
        wait-timeout means it's safe to send a heartbeat (no half-
        read frame on the wire); a readable socket means a real
        frame is incoming and we consume it without a deadline
        (frames are short — milliseconds — so blocking briefly
        on the rest of the bytes is fine).
        """
        # Disable any inherited timeout: we drive blocking on the
        # frame body, gating only on select() before each frame.
        ws.settimeout(None)
        # First step: read HELLO synchronously.
        text, close = ws.recv_frame()
        if close is not None:
            self._raise_close(close)
        if text is None:
            return
        hello = self._parse_payload(text)
        if hello.get("op") != OP_HELLO:
            log.warn("discord: expected HELLO; got op",
                     op=hello.get("op"))
            return
        heartbeat_interval_ms = int(
            ((hello.get("d") or {}).get("heartbeat_interval")) or 41250
        )
        heartbeat_interval_secs = heartbeat_interval_ms / 1000.0

        # IDENTIFY or RESUME.
        if (self.session_id and self.last_seq is not None
                and self.resume_gateway_url):
            log.info("discord: sending RESUME",
                     session=self.session_id, seq=self.last_seq)
            ws.send_text(json.dumps({
                "op": OP_RESUME,
                "d": {
                    "token": self.token,
                    "session_id": self.session_id,
                    "seq": self.last_seq,
                },
            }))
        else:
            log.info("discord: sending IDENTIFY", intents=self.intents)
            ws.send_text(json.dumps({
                "op": OP_IDENTIFY,
                "d": {
                    "token": self.token,
                    "intents": self.intents,
                    "properties": {
                        "os": "linux",
                        "browser": "librefang",
                        "device": "librefang",
                    },
                },
            }))

        # First heartbeat is delayed by a random fraction of interval
        # (Discord docs: "Your client should ... begin sending Opcode 1
        # Heartbeat payloads ... after waiting a random number between
        # 0 and heartbeat_interval ms"). Use a deadline so we send
        # exactly one heartbeat per interval thereafter.
        next_heartbeat_at = (
            time.monotonic() + heartbeat_interval_secs * random.random()
        )

        while True:
            now = time.monotonic()
            wait = max(0.0, next_heartbeat_at - now)
            if not ws.wait_readable(wait):
                # No frame waiting — time to heartbeat.
                ws.send_text(json.dumps({
                    "op": OP_HEARTBEAT,
                    "d": self.last_seq,
                }))
                next_heartbeat_at = (
                    time.monotonic() + heartbeat_interval_secs
                )
                continue
            try:
                text, close = ws.recv_frame()
            except (EOFError, OSError) as e:
                log.warn("discord gateway socket dropped", error=str(e))
                return
            if close is not None:
                self._raise_close(close)
                return
            if text is None:
                continue
            try:
                payload = self._parse_payload(text)
            except ValueError as e:
                log.warn("discord: bad gateway frame", error=str(e))
                continue
            sent_hb = self._handle_payload(payload, ws, emit)
            now = time.monotonic()
            if sent_hb:
                # Server-requested HEARTBEAT already went out from the
                # handler — slide our own deadline forward so we don't
                # double-beat back-to-back.
                next_heartbeat_at = now + heartbeat_interval_secs
            elif now >= next_heartbeat_at:
                # We read a long burst of frames and overran the
                # deadline mid-burst; catch up with one beat now.
                ws.send_text(json.dumps({
                    "op": OP_HEARTBEAT,
                    "d": self.last_seq,
                }))
                next_heartbeat_at = now + heartbeat_interval_secs

    def _handle_payload(
        self,
        payload: dict,
        ws: _WebSocketClient,
        emit: Callable[[dict], None],
    ) -> bool:
        """Returns ``True`` when the handler itself sent a HEARTBEAT
        (in response to a server-initiated opcode 1), so the outer
        loop can reset its ``next_heartbeat_at`` deadline and not
        immediately fire another beat."""
        op = payload.get("op")
        seq = payload.get("s")
        if isinstance(seq, int):
            self.last_seq = seq
        if op == OP_DISPATCH:
            event = payload.get("t") or ""
            d = payload.get("d") or {}
            if event == "READY":
                user = d.get("user") or {}
                self.bot_user_id = user.get("id")
                self.session_id = d.get("session_id")
                resume_url = d.get("resume_gateway_url")
                if isinstance(resume_url, str) and resume_url:
                    self.resume_gateway_url = resume_url
                log.info("discord READY",
                         bot=self.bot_user_id,
                         username=user.get("username"),
                         session=self.session_id)
            elif event in ("MESSAGE_CREATE", "MESSAGE_UPDATE"):
                ev = parse_message_create(
                    d,
                    bot_user_id=self.bot_user_id,
                    allowed_guilds=self.allowed_guilds,
                    allowed_users=self.allowed_users,
                    ignore_bots=self.ignore_bots,
                    mention_patterns=self.mention_patterns,
                    account_id=self.account_id,
                )
                if ev is not None:
                    emit(ev)
            elif event == "RESUMED":
                log.info("discord session RESUMED",
                         session=self.session_id)
            # Other dispatch events ignored (PRESENCE_UPDATE,
            # TYPING_START etc. — none of which the Rust adapter
            # surfaced either).
        elif op == OP_HEARTBEAT:
            # Server-requested heartbeat — respond immediately and
            # signal the outer loop to reset its scheduling deadline.
            ws.send_text(json.dumps({
                "op": OP_HEARTBEAT, "d": self.last_seq,
            }))
            return True
        elif op == OP_HEARTBEAT_ACK:
            pass
        elif op == OP_RECONNECT:
            log.info("discord: server requested reconnect")
            ws.send_close()
            raise RuntimeError("reconnect")
        elif op == OP_INVALID_SESSION:
            resumable = bool(payload.get("d") is True)
            if not resumable:
                log.info("discord: session invalid and not resumable; "
                         "clearing")
                self.session_id = None
                self.last_seq = None
                self.resume_gateway_url = None
            else:
                log.info("discord: session invalid (resumable)")
            ws.send_close()
            raise RuntimeError("invalid_session")
        else:
            log.warn("discord: unknown op", op=op)
        return False

    @staticmethod
    def _parse_payload(text: str) -> dict:
        v = json.loads(text)
        if not isinstance(v, dict):
            raise ValueError("gateway frame is not a JSON object")
        return v

    @staticmethod
    def _raise_close(close: tuple[int, bytes]) -> None:
        code, reason = close
        reason_text = reason.decode("utf-8", "replace")
        if code in FATAL_CLOSE_CODES:
            raise _FatalGatewayError(
                f"gateway close code {code}: {FATAL_CLOSE_CODES[code]} "
                f"(server reason: {reason_text!r})"
            )
        log.info("discord gateway closed",
                 code=code, reason=reason_text)

    # ---- public sidecar surface --------------------------------------

    async def produce(self, emit: Callable[[dict], None]) -> None:
        loop = asyncio.get_event_loop()
        await loop.run_in_executor(None, self._gateway_loop, emit)

    async def on_send(self, cmd) -> None:
        channel_id = (
            cmd.channel_id
            or (cmd.user.get("platform_id") if cmd.user else "")
            or ""
        )
        if not channel_id:
            log.warn("discord on_send: empty channel_id, dropping")
            return
        # Plain text only — non-text content surfaces a placeholder so
        # the agent loop sees a delivery rather than silent failure.
        if cmd.content and not (
            isinstance(cmd.content, dict) and "Text" in cmd.content
        ):
            text = "(Unsupported content type)"
        else:
            text = cmd.text or ""
        await asyncio.get_event_loop().run_in_executor(
            None, self._send_message, str(channel_id), text,
        )

    async def on_command(self, cmd) -> None:
        # The default routes ``send`` to ``on_send``; we also support
        # ``typing``.
        from librefang.sidecar.protocol import Send, TypingCmd
        if isinstance(cmd, Send):
            await self.on_send(cmd)
        elif isinstance(cmd, TypingCmd):
            await asyncio.get_event_loop().run_in_executor(
                None, self._send_typing, cmd.channel_id,
            )


def _split_csv(raw: str) -> list[str]:
    """Split a comma-separated env-var into a clean list of strings.

    Empty input → empty list. Each item is whitespace-stripped.
    Matches the Rust deserialize_string_or_int_vec shape (we accept
    both ``"123,456"`` and ``"123, 456"``)."""
    if not raw:
        return []
    return [s.strip() for s in raw.split(",") if s.strip()]


def _parse_retry_after(resp_hdrs: dict, *, default_secs: float) -> float:
    """Discord's 429 response includes ``Retry-After`` (seconds, may be
    a decimal). Fall back to ``default_secs`` when missing/garbled.
    Capped at MAX_BACKOFF_SECS so a server bug can't pin the send
    loop for hours."""
    raw = resp_hdrs.get("retry-after")
    if not raw:
        return default_secs
    try:
        v = float(raw)
    except (TypeError, ValueError):
        return default_secs
    return min(max(v, 0.1), MAX_BACKOFF_SECS)


class _FatalGatewayError(Exception):
    """Raised on a Discord gateway close code that no amount of
    retrying will fix (bad token, disallowed intents, …). The
    supervisor's circuit-breaker will stop us after a few attempts;
    surfacing the close-code reason as a single ERROR log gives the
    operator enough information to fix the config."""


if __name__ == "__main__":
    run_stdio_main(DiscordAdapter)
