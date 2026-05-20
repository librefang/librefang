#!/usr/bin/env python3
"""QQ Bot API v2 sidecar channel adapter for LibreFang.

Replaces the former in-process Rust ``librefang-channels::qq`` adapter
(removed in this sidecar migration; same pattern as ntfy #5224,
telegram #5241, gotify #5263, mastodon #5264, bluesky #5277, reddit
#5281, twitch #5297, rocketchat #5298, discord #5299, nextcloud
#5301, slack #5302, webex #5309, line #5312, zulip #5310, mattermost
#5315, signal #5317).

Talks to the QQ Bot Open Platform (v2):

* Token fetch — ``POST https://bots.qq.com/app/getAppAccessToken``
  with ``{appId, clientSecret}`` → ``{access_token, expires_in}``.
* Gateway discovery — ``GET https://api.sgroup.qq.com/gateway`` with
  Bearer auth → ``{url: "wss://..."}``.
* WebSocket — connect to the gateway URL, handle the HELLO(op=10) →
  IDENTIFY(op=2) → READY dispatch handshake, heartbeat on
  ``heartbeat_interval`` (op=1), receive DISPATCH events (op=0).
* Outbound — ``POST {api_base}{reply_endpoint}`` with
  ``{content, msg_id, msg_type=0}`` and Bearer auth. The reply
  endpoint and message id are surfaced to the kernel as
  ``channel_id`` and ``thread_id`` on the inbound event so the
  bridge round-trips them on outbound (see "Improvement #4" below).

Behaviour parity with the Rust adapter (every assertion below has a
file/line citation against ``crates/librefang-channels/src/qq.rs``
on the pre-migration tree):

* **Token fetch** — ``POST https://bots.qq.com/app/getAppAccessToken``
  with ``{"appId", "clientSecret"}`` body; pulls ``access_token``
  from the response. Mirrors ``qq.rs:542-557``.
* **Gateway discovery** — ``GET /gateway`` with Bearer auth, pulls
  ``url``. Mirrors ``qq.rs:559-573``.
* **WS HELLO/IDENTIFY/READY handshake** — receive op=10 HELLO,
  derive heartbeat interval from ``d.heartbeat_interval``, send
  op=2 IDENTIFY with ``token = "QQBot <access_token>"``,
  ``intents`` bitmask, and ``shard = [0, 1]``. The first
  dispatch(op=0) with ``t == "READY"`` flips the connected state.
  Mirrors ``qq.rs:413-433``.
* **Heartbeat** — op=1 frame ``{"op":1, "d": <last_seq_or_null>}``
  every ``heartbeat_interval`` ms; ``d`` carries the last seen
  ``s`` field from dispatch frames. Mirrors ``qq.rs:359-368``.
* **Event parsing** — 4 dispatch event types recognised
  (``qq.rs:194-224``):

  * ``MESSAGE_CREATE`` / ``AT_MESSAGE_CREATE`` → guild channel,
    reply via ``/channels/{channel_id}/messages``,
    ``is_group=True``.
  * ``DIRECT_MESSAGE_CREATE`` → guild DM, reply via
    ``/dms/{guild_id}/messages``, ``is_group=False``.
  * ``GROUP_AT_MESSAGE_CREATE`` → C2C group, reply via
    ``/v2/groups/{group_openid}/messages``, ``is_group=True``.
  * ``C2C_MESSAGE_CREATE`` → C2C DM, reply via
    ``/v2/users/{user_openid}/messages``, ``is_group=False``.
* **Bot-mention strip** — a leading ``/`` (the QQ bot-mention
  prefix) is trimmed from the inbound text before slash-command
  detection. Mirrors ``qq.rs:227``.
* **User allowlist** — empty ``QQ_ALLOWED_USERS`` = listen on every
  sender; non-empty restricts to listed ids (``qq.rs:400``).
* **Multi-bot ``account_id``** (``qq.rs:402-405``, #5003). When
  ``QQ_ACCOUNT_ID`` is set, it is injected into the inbound message
  metadata so the bridge can scope ``ApprovalRequested`` delivery
  to the channel bound to the requesting agent.
* **Outbound markdown stripping** — every outbound text passes
  through the same regex pipeline the Rust adapter applied at
  ``qq.rs:137-180`` (code blocks, inline code, bold, italic,
  headings, table separators, links, blockquotes, horizontal
  rules, three-or-more newlines, plus the leading ``<think>...
  </think>`` reasoning block).
* **2000-char chunking** — ``QQ_MSG_LIMIT`` parity with the Rust
  ``QQ_MAX_MESSAGE_LEN`` constant at ``qq.rs:26``.
* **Reconnect** — exponential backoff 2 s → 60 s on every error
  path (token, gateway, WS connect, WS read). Mirrors
  ``qq.rs:282`` (``INITIAL_BACKOFF = 2s``, ``MAX_BACKOFF = 60s``).
* **ChannelType::Custom("qq") preserved** as
  ``channel_type = "qq"`` on the sidecar entry — existing routing
  and ``channel_role_mapping`` keys that reference ``qq`` continue
  to resolve.

Improvements over the Rust adapter
==================================

1. **Reply context actually round-trips**. The Rust
   ``parse_dispatch_event`` (``qq.rs:182-246``) computed
   ``reply_endpoint`` and ``msg_id`` but the dispatch loop bound
   them to ``_endpoint`` / ``_msg_id`` and dropped them on the
   floor (``qq.rs:399``); ``send`` then expected
   ``user.platform_id`` to be encoded as ``"<endpoint>|<msg_id>"``
   (``qq.rs:497-498``) and silently no-op'd when the delimiter
   wasn't there. The Rust adapter therefore failed every real
   outbound — only the synthetic wiremock tests at
   ``qq.rs:686-712`` exercised the working shape. The sidecar
   surfaces the reply endpoint as ``channel_id`` and the QQ
   ``msg_id`` as ``thread_id`` on the inbound event so the bridge
   round-trips them through to ``on_send``, which posts to
   ``{api_base}{channel_id}`` with the correct passive-reply
   ``msg_id``.
2. **Inbound dedupe on ``msg.id``**. The Rust dispatch loop
   (``qq.rs:399-410``) emitted every parsed event unconditionally;
   a WS reconnect that races with the server's last-delivery
   cursor would re-deliver. Bounded local set on QQ's ``id`` with
   ``SEEN_MESSAGES_MAX = 10 000`` / ``SEEN_MESSAGES_EVICT = 5 000``
   (same policy as reddit / rocketchat / nextcloud / webex /
   line / mattermost / signal).
3. **429 ``Retry-After`` honoured on every REST path**. The Rust
   adapter had no 429 handling — a throttled
   ``getAppAccessToken``, ``/gateway``, or outbound send returned
   an Err and either burned the reconnect budget or dropped the
   chunk. Sidecar parses ``Retry-After`` (default 30 s fallback,
   floor 1 s, cap ``MAX_BACKOFF_SECS``), sleeps, retries once,
   then logs-and-continues on the second 429 (matches the
   rocketchat / webex / line / mattermost / signal #5303
   pattern).
4. **Explicit HTTP timeouts**. ``urllib.request.urlopen`` has no
   default timeout; the Rust adapter pre-configured ``reqwest``'s
   30 s default at ``qq.rs:71``. Sidecar passes
   ``timeout=SEND_TIMEOUT_SECS`` (15 s) on every REST call so a
   misbehaving endpoint trips an explicit error instead of
   hanging the worker thread.

Stdlib-only: HTTPS via ``urllib.request``, WebSocket via a
hand-rolled RFC 6455 client over ``socket`` + ``ssl`` (same
pattern as the discord / slack / webex / mattermost sidecars).

Configure via ``[[sidecar_channels]]``::

    [[sidecar_channels]]
    name = "qq"
    command = "python3"
    args = ["-m", "librefang.sidecar.adapters.qq"]
    channel_type = "qq"
    [sidecar_channels.env]
    QQ_APP_ID = "1234567890"
    # QQ_ALLOWED_USERS = "openid-1,openid-2"   # optional
    # QQ_ACCOUNT_ID = "prod-bot"               # optional
    # QQ_INTENTS = "1073746435"                # optional bitmask override

Secret via ``~/.librefang/secrets.env``: ``QQ_APP_SECRET`` (the
bot's ``clientSecret`` from the QQ Open Platform console).
"""
from __future__ import annotations

import asyncio
import base64
import hashlib
import json
import os
import re
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

# QQ Open Platform endpoints. The token URL lives under bots.qq.com;
# every other REST call goes through the sgroup.qq.com API base.
DEFAULT_API_BASE = "https://api.sgroup.qq.com"
DEFAULT_TOKEN_URL = "https://bots.qq.com/app/getAppAccessToken"

# QQ message length limit (qq.rs:26 — QQ_MAX_MESSAGE_LEN = 2000).
QQ_MSG_LIMIT = 2000

SEND_TIMEOUT_SECS = 15.0
HANDSHAKE_TIMEOUT_SECS = 15.0

INITIAL_BACKOFF_SECS = 2.0
MAX_BACKOFF_SECS = 60.0

# Default fallback when QQ 429s without a parseable Retry-After
# header. 30 s is conservative — mirrors the rocketchat / nextcloud /
# webex / line / mattermost / signal sidecars (#5303).
RETRY_AFTER_DEFAULT_SECS = 30.0

# Bounded dedupe cap on QQ ``id``. Same policy as reddit / rocketchat /
# nextcloud / webex / line / mattermost / signal.
SEEN_MESSAGES_MAX = 10_000
SEEN_MESSAGES_EVICT = 5_000

# Intent bit flags for QQ Bot API v2 (qq.rs:29-33).
INTENT_GUILDS = 1 << 0
INTENT_GUILD_MEMBERS = 1 << 1
INTENT_DIRECT_MESSAGE = 1 << 12
INTENT_GROUP_AND_C2C = 1 << 25
INTENT_PUBLIC_GUILD_MESSAGES = 1 << 30

DEFAULT_INTENTS = (
    INTENT_GUILDS
    | INTENT_GUILD_MEMBERS
    | INTENT_DIRECT_MESSAGE
    | INTENT_GROUP_AND_C2C
    | INTENT_PUBLIC_GUILD_MESSAGES
)

# RFC 6455 — same constants as the discord / slack / webex / mattermost
# sidecars.
_WS_GUID = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11"
_OP_CONT = 0x0
_OP_TEXT = 0x1
_OP_BIN = 0x2
_OP_CLOSE = 0x8
_OP_PING = 0x9
_OP_PONG = 0xA

MAX_FRAME_PAYLOAD = 1 << 22  # 4 MiB

# How long to block in select() per loop iteration before re-checking
# liveness. The producer also fires heartbeats from this loop, so the
# tick must be smaller than the smallest QQ heartbeat interval we
# expect (45 s by default).
READ_TICK_SECS = 1.0


def _split_message(text: str, limit: int) -> list[str]:
    """Chunk ``text`` into <= ``limit`` pieces, preferring newline
    splits. Mirrors the shared Rust ``split_message`` helper."""
    if len(text) <= limit:
        return [text]
    chunks: list[str] = []
    rest = text
    while len(rest) > limit:
        window = rest[:limit]
        cut = window.rfind("\n")
        if cut <= 0:
            cut = limit
        chunks.append(rest[:cut])
        rest = rest[cut:].lstrip("\n") if cut < limit else rest[cut:]
    if rest:
        chunks.append(rest)
    return chunks


def _split_csv(raw: str) -> list[str]:
    """Comma-separated env-var → cleaned list of strings."""
    if not raw:
        return []
    return [s.strip() for s in raw.split(",") if s.strip()]


def _parse_retry_after(resp_hdrs: dict, *, default_secs: float) -> float:
    """``Retry-After`` parser, floor 1 s, cap ``MAX_BACKOFF_SECS``."""
    raw = resp_hdrs.get("retry-after")
    if not raw:
        return default_secs
    try:
        v = float(raw)
    except (TypeError, ValueError):
        return default_secs
    return min(max(v, 1.0), MAX_BACKOFF_SECS)


# ---- markdown → plain text (qq.rs:137-180 parity) -------------------

_RE_THINK = re.compile(r"<think>[\s\S]*?</think>", re.IGNORECASE)
_RE_CODEBLOCK = re.compile(r"```\w*\n?([\s\S]*?)```")
_RE_INLINE_CODE = re.compile(r"`([^`]+)`")
_RE_BOLD = re.compile(r"\*\*([^*]+)\*\*")
_RE_ITALIC = re.compile(r"\*([^*]+)\*")
_RE_HEADING = re.compile(r"(?m)^#{1,6}\s+")
_RE_TABLE_SEP = re.compile(r"(?m)^\|[-:| ]+\|$")
_RE_LINK = re.compile(r"\[([^\]]+)\]\([^)]+\)")
_RE_QUOTE = re.compile(r"(?m)^>\s?")
_RE_HR = re.compile(r"(?m)^---+$")
_RE_NEWLINES = re.compile(r"\n{3,}")


def strip_markdown(text: str) -> str:
    """Strip Markdown formatting to plain text for QQ. Mirrors the
    Rust ``strip_markdown`` helper at ``qq.rs:137-180``.

    QQ's bot API renders Markdown literally (asterisks, backticks,
    table pipes show up in the message), so every outbound text is
    flattened first. Order matters: code blocks before inline-code,
    bold before italic (the italic regex would otherwise eat the
    outer ``**`` markers).
    """
    if not text:
        return ""
    # Drop ``<think>...</think>`` reasoning blocks first so they
    # don't survive into later regex passes.
    s = _RE_THINK.sub("", text)
    s = _RE_CODEBLOCK.sub(r"\1", s)
    s = _RE_INLINE_CODE.sub(r"\1", s)
    s = _RE_BOLD.sub(r"\1", s)
    s = _RE_ITALIC.sub(r"\1", s)
    s = _RE_HEADING.sub("", s)
    s = _RE_TABLE_SEP.sub("", s)
    s = _RE_LINK.sub(r"\1", s)
    s = _RE_QUOTE.sub("", s)
    s = _RE_HR.sub("", s)
    s = _RE_NEWLINES.sub("\n\n", s)
    return s.strip()


# ---- inbound dispatch parsing (qq.rs:182-246 parity) ----------------


def parse_qq_event(
    event_type: str,
    data: Any,
    *,
    allowed_users: list[str],
    account_id: Optional[str],
) -> Optional[dict]:
    """Pure-function port of ``parse_dispatch_event`` at
    ``crates/librefang-channels/src/qq.rs`` lines 182-246.

    Returns a ``message`` event dict ready to ``emit``, or ``None``
    when the payload should be skipped (unknown event type, empty
    content, malformed envelope, blocked sender).
    """
    if not isinstance(data, dict):
        return None

    msg_id_raw = data.get("id")
    if not isinstance(msg_id_raw, str):
        msg_id_raw = ""

    content_raw = data.get("content")
    if not isinstance(content_raw, str):
        return None
    content = content_raw.strip()
    if not content:
        return None

    # Resolve (sender_id, sender_name, is_group, reply_endpoint) by
    # event type. Mirrors qq.rs:194-224.
    if event_type in ("MESSAGE_CREATE", "AT_MESSAGE_CREATE"):
        channel_id = data.get("channel_id")
        if not isinstance(channel_id, str):
            channel_id = ""
        author = data.get("author")
        if not isinstance(author, dict):
            author = {}
        user_id = author.get("id")
        if not isinstance(user_id, str):
            user_id = ""
        username = author.get("username")
        if not isinstance(username, str) or not username:
            username = "User"
        is_group = True
        reply_endpoint = f"/channels/{channel_id}/messages"
    elif event_type == "DIRECT_MESSAGE_CREATE":
        guild_id = data.get("guild_id")
        if not isinstance(guild_id, str):
            guild_id = ""
        author = data.get("author")
        if not isinstance(author, dict):
            author = {}
        user_id = author.get("id")
        if not isinstance(user_id, str):
            user_id = ""
        username = author.get("username")
        if not isinstance(username, str) or not username:
            username = "User"
        is_group = False
        reply_endpoint = f"/dms/{guild_id}/messages"
    elif event_type == "GROUP_AT_MESSAGE_CREATE":
        group_openid = data.get("group_openid")
        if not isinstance(group_openid, str):
            group_openid = ""
        author = data.get("author")
        if not isinstance(author, dict):
            author = {}
        user_id = author.get("member_openid")
        if not isinstance(user_id, str):
            user_id = ""
        username = "GroupUser"
        is_group = True
        reply_endpoint = f"/v2/groups/{group_openid}/messages"
    elif event_type == "C2C_MESSAGE_CREATE":
        author = data.get("author")
        if not isinstance(author, dict):
            author = {}
        user_openid = author.get("user_openid")
        if not isinstance(user_openid, str):
            user_openid = ""
        user_id = user_openid
        username = "User"
        is_group = False
        reply_endpoint = f"/v2/users/{user_openid}/messages"
    else:
        return None

    # User allowlist filter. Empty list = accept everyone.
    # Falsy `user_id` (missing sender) is never allowed when an
    # explicit allowlist is configured.
    if allowed_users and user_id not in allowed_users:
        return None

    # Strip leading bot-mention prefix ('/' or '<@!...>' — the Rust
    # adapter only handled the bare ``/`` form at qq.rs:227).
    clean = content.lstrip("/").strip()
    if not clean:
        return None

    if clean.startswith("/"):
        # Slash-command form. The Rust adapter routed every inbound
        # through ChannelContent::Text — slash routing was bridge-side.
        # The sidecar surfaces the structured form so the kernel sees
        # the same shape as other adapters; bridge-side routing is
        # unaffected.
        head, _, tail = clean[1:].partition(" ")
        msg_content = Content.command(head, tail.split() if tail else [])
    else:
        msg_content = Content.text(clean)

    metadata: dict[str, Any] = {}
    if account_id is not None:
        metadata["account_id"] = account_id

    return protocol.message(
        user_id=user_id,
        user_name=username,
        content=msg_content,
        message_id=msg_id_raw or None,
        # Improvement #4: surface the reply endpoint and msg_id on
        # standard protocol fields so the bridge round-trips them on
        # outbound without any QQ-specific encoding.
        channel_id=reply_endpoint,
        thread_id=msg_id_raw or None,
        is_group=is_group,
        metadata=metadata,
    )


# ---------------------------------------------------------------------------
# Stdlib WebSocket client — same RFC 6455 reader as the discord / slack /
# webex / mattermost sidecars. Lifted verbatim from mattermost.py.
# ---------------------------------------------------------------------------


class _WebSocketClient:
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
        sock = socket.create_connection((host, port), timeout=self._handshake_timeout)
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
        if self._leftover:
            return True
        sock = self._sock
        if sock is None:
            return False
        pending = getattr(sock, "pending", None)
        if callable(pending):
            try:
                if pending() > 0:
                    return True
            except Exception:  # noqa: BLE001
                pass
        try:
            r, _, _ = select.select([sock], [], [], max(0.0, timeout))
        except (OSError, ValueError):
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
                f"websocket frame payload {ln} exceeds cap {MAX_FRAME_PAYLOAD}"
            )
        mask_key = self._recv_exact(4) if masked else None
        payload = self._recv_exact(ln)
        if mask_key is not None:
            payload = bytes(b ^ mask_key[i % 4] for i, b in enumerate(payload))
        if opcode == _OP_PING:
            self._send_frame(_OP_PONG, payload)
            return None, None
        if opcode == _OP_PONG:
            return None, None
        if opcode == _OP_CLOSE:
            code = 1005
            reason = b""
            if len(payload) >= 2:
                code = struct.unpack(">H", payload[:2])[0]
                reason = payload[2:]
            return None, (code, reason)
        if opcode == _OP_TEXT:
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
                    payload2 = bytes(b ^ mk[i % 4] for i, b in enumerate(payload2))
                if opcode2 != _OP_CONT:
                    raise RuntimeError(f"ws unexpected interleaved opcode {opcode2}")
                buf.extend(payload2)
            return buf.decode("utf-8", "replace"), None
        return None, None


# ---------------------------------------------------------------------------
# QQ adapter
# ---------------------------------------------------------------------------


class QqAdapter(SidecarAdapter):
    # QQ Bot API v2 exposes no public typing/reaction surface we can
    # wire through cleanly — keep capabilities empty rather than
    # over-claim. Mirrors line / zulip / signal.
    capabilities: list = []
    # QQ surfaces are mixed (guild channels, group chats, DMs). The
    # chat-room precedent (twitch / discord / slack / webex / line /
    # mattermost / signal) is to surface errors so the user sees a
    # visible failure instead of silent swallow.
    suppress_error_responses: bool = False

    SCHEMA = Schema(
        name="qq",
        display_name="QQ Bot",
        description="QQ Bot API v2 WebSocket + REST adapter (out-of-process sidecar)",
        fields=[
            Field("QQ_APP_ID", "App ID", "text",
                  required=True,
                  placeholder="1234567890"),
            Field("QQ_APP_SECRET", "App Secret", "secret",
                  required=True,
                  placeholder="abc123..."),
            Field("QQ_ALLOWED_USERS",
                  "Allowed sender IDs (comma-separated, empty = all)",
                  "text",
                  placeholder="openid-1,openid-2",
                  advanced=True),
            Field("QQ_ACCOUNT_ID",
                  "Account ID (multi-bot routing)",
                  "text",
                  placeholder="prod-bot",
                  advanced=True),
            Field("QQ_INTENTS",
                  "Intents bitmask (decimal, leave empty for the default)",
                  "text",
                  placeholder=str(DEFAULT_INTENTS),
                  advanced=True),
        ],
    )

    def __init__(self) -> None:
        app_id = os.environ.get("QQ_APP_ID", "").strip()
        app_secret = os.environ.get("QQ_APP_SECRET", "").strip()
        missing: list[str] = []
        if not app_id:
            missing.append("QQ_APP_ID")
        if not app_secret:
            missing.append("QQ_APP_SECRET")
        if missing:
            log.error("qq required env vars missing", missing=missing)
            raise SystemExit(2)

        self.app_id = app_id
        self.app_secret = app_secret
        self.allowed_users = _split_csv(
            os.environ.get("QQ_ALLOWED_USERS", "")
        )
        acct = os.environ.get("QQ_ACCOUNT_ID", "").strip()
        self.account_id = acct or None

        # Test seams. Real deployments leave these unset.
        self.api_base = (
            os.environ.get("QQ_API_BASE", "").strip() or DEFAULT_API_BASE
        ).rstrip("/")
        self.token_url = (
            os.environ.get("QQ_TOKEN_URL", "").strip() or DEFAULT_TOKEN_URL
        )
        # When set, skip /gateway discovery and connect to this URL
        # directly. Used by tests that point us at a local mock WS.
        self.ws_url_override = os.environ.get("QQ_WS_URL", "").strip() or None

        intents_raw = os.environ.get("QQ_INTENTS", "").strip()
        if intents_raw:
            try:
                self.intents = int(intents_raw, 0)
            except ValueError:
                log.warn(
                    "qq QQ_INTENTS not an integer; using default",
                    value=intents_raw,
                    default=DEFAULT_INTENTS,
                )
                self.intents = DEFAULT_INTENTS
        else:
            self.intents = DEFAULT_INTENTS

        # Current access token. Refreshed at each reconnect (matches
        # the Rust adapter's behaviour at qq.rs:289-302).
        self._token: Optional[str] = None
        self._token_lock = threading.Lock()

        # Improvement #2: bounded dedupe on QQ ``id``.
        self._seen_ids: set[str] = set()
        self._seen_order: list[str] = []
        self._seen_lock = threading.Lock()

    # ---- HTTP helpers ------------------------------------------------

    def _bearer_headers(self, *, content_type: bool = False) -> dict:
        token = ""
        with self._token_lock:
            if self._token is not None:
                token = self._token
        h: dict = {
            "User-Agent": "librefang-qq-sidecar/1 (https://librefang.org)",
        }
        if token:
            h["Authorization"] = f"Bearer {token}"
        if content_type:
            h["Content-Type"] = "application/json; charset=utf-8"
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
        """One-shot HTTP request. Returns
        ``(status, parsed_json_or_None, raw_bytes, response_headers)``.
        Response headers are lower-cased so 429 ``Retry-After`` lookups
        are case-insensitive."""
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

    # ---- dedupe ------------------------------------------------------

    def _mark_seen(self, msg_id: Optional[str]) -> bool:
        """Return True iff ``msg_id`` is freshly seen (i.e. emit it).
        ``None`` / empty ids are always treated as fresh — they don't
        participate in dedupe. Mirrors reddit / rocketchat / nextcloud /
        webex / line / mattermost / signal."""
        if not msg_id:
            return True
        with self._seen_lock:
            if msg_id in self._seen_ids:
                return False
            self._seen_ids.add(msg_id)
            self._seen_order.append(msg_id)
            if len(self._seen_order) > SEEN_MESSAGES_MAX:
                drop = self._seen_order[:SEEN_MESSAGES_EVICT]
                self._seen_order = self._seen_order[SEEN_MESSAGES_EVICT:]
                for k in drop:
                    self._seen_ids.discard(k)
            return True

    # ---- REST: token + gateway + outbound send ----------------------

    def _fetch_token(self) -> str:
        """``POST bots.qq.com/app/getAppAccessToken`` →
        ``access_token``. Raises ``RuntimeError`` on any non-200 so
        the outer gateway loop backs off. Honours 429 ``Retry-After``
        once (improvement #3)."""
        body = json.dumps({
            "appId": self.app_id,
            "clientSecret": self.app_secret,
        }).encode("utf-8")
        headers = {
            "Content-Type": "application/json; charset=utf-8",
            "User-Agent": "librefang-qq-sidecar/1 (https://librefang.org)",
        }
        status, parsed, raw, resp_hdrs = self._http(
            self.token_url, method="POST", body=body, headers=headers,
        )
        if status == 429:
            wait = _parse_retry_after(
                resp_hdrs, default_secs=RETRY_AFTER_DEFAULT_SECS,
            )
            log.warn("qq getAppAccessToken 429; will retry once",
                     retry_after_secs=wait)
            time.sleep(wait)
            status, parsed, raw, resp_hdrs = self._http(
                self.token_url, method="POST", body=body, headers=headers,
            )
        if status != 200 or not isinstance(parsed, dict):
            snippet = raw[:200].decode("utf-8", "replace") if raw else ""
            raise RuntimeError(
                f"qq getAppAccessToken failed (status={status}): {snippet}"
            )
        token = parsed.get("access_token")
        if not isinstance(token, str) or not token:
            raise RuntimeError("qq getAppAccessToken: missing access_token")
        return token

    def _fetch_gateway(self) -> str:
        """``GET {api_base}/gateway`` → WS URL. Bearer auth via the
        cached token. Honours 429 once."""
        url = f"{self.api_base}/gateway"
        status, parsed, raw, resp_hdrs = self._http(
            url, headers=self._bearer_headers(),
        )
        if status == 429:
            wait = _parse_retry_after(
                resp_hdrs, default_secs=RETRY_AFTER_DEFAULT_SECS,
            )
            log.warn("qq /gateway 429; will retry once",
                     retry_after_secs=wait)
            time.sleep(wait)
            status, parsed, raw, resp_hdrs = self._http(
                url, headers=self._bearer_headers(),
            )
        if status != 200 or not isinstance(parsed, dict):
            snippet = raw[:200].decode("utf-8", "replace") if raw else ""
            raise RuntimeError(
                f"qq /gateway failed (status={status}): {snippet}"
            )
        ws_url = parsed.get("url")
        if not isinstance(ws_url, str) or not ws_url:
            raise RuntimeError("qq /gateway: missing url")
        return ws_url

    def _post_message(
        self,
        reply_endpoint: str,
        msg_id: Optional[str],
        text: str,
    ) -> None:
        """``POST {api_base}{reply_endpoint}`` with chunking. Honours
        429 ``Retry-After`` and retries once per chunk (improvement
        #3). On the second 429 / non-2xx we log and continue — matches
        the webex / line / mattermost / signal fail-open behaviour so
        a single throttled chunk doesn't drop the rest of the reply."""
        if not reply_endpoint:
            log.warn("qq _post_message: empty reply_endpoint, dropping")
            return
        url = f"{self.api_base}{reply_endpoint}"
        for chunk in _split_message(text, QQ_MSG_LIMIT):
            payload: dict[str, Any] = {
                "content": chunk,
                "msg_type": 0,
            }
            if msg_id:
                payload["msg_id"] = msg_id
            body = json.dumps(payload).encode("utf-8")
            status, _resp, raw, resp_hdrs = self._http(
                url, method="POST", body=body,
                headers=self._bearer_headers(content_type=True),
            )
            if status == 429:
                wait = _parse_retry_after(
                    resp_hdrs, default_secs=RETRY_AFTER_DEFAULT_SECS,
                )
                log.warn("qq POST send 429; sleeping then retrying once",
                         endpoint=reply_endpoint, retry_after_secs=wait)
                time.sleep(wait)
                status, _resp, raw, resp_hdrs = self._http(
                    url, method="POST", body=body,
                    headers=self._bearer_headers(content_type=True),
                )
            if status >= 300:
                snippet = raw[:200].decode("utf-8", "replace") if raw else ""
                log.warn("qq POST send failed",
                         endpoint=reply_endpoint, status=status,
                         body=snippet)
                # fail-open: keep chunking
                continue

    # ---- WS gateway loop --------------------------------------------

    def _make_ws(self, url: str, *, headers: dict) -> _WebSocketClient:
        """Test seam — overridden by tests to inject a mock socket."""
        return _WebSocketClient(url, headers=headers)

    def _handle_dispatch(
        self,
        event_type: str,
        data: Any,
        emit: Callable[[dict], None],
    ) -> None:
        """Route a DISPATCH(op=0) frame's ``t`` / ``d`` into
        ``parse_qq_event`` after the inbound dedupe check."""
        msg_id = None
        if isinstance(data, dict):
            raw = data.get("id")
            if isinstance(raw, str) and raw:
                msg_id = raw
        # Improvement #2: dedupe before the parse so identical
        # redelivery on reconnect doesn't double-emit.
        if msg_id and not self._mark_seen(msg_id):
            return

        ev = parse_qq_event(
            event_type, data,
            allowed_users=self.allowed_users,
            account_id=self.account_id,
        )
        if ev is not None:
            emit(ev)

    def _run_session(
        self,
        ws: _WebSocketClient,
        token: str,
        emit: Callable[[dict], None],
    ) -> None:
        """Drive one WS session: handle HELLO/IDENTIFY/READY/HEARTBEAT
        on top of the raw RFC 6455 reader. Returns when the connection
        drops (the outer reconnect loop will reconnect)."""
        last_seq: Optional[int] = None
        heartbeat_interval: Optional[float] = None  # seconds
        next_heartbeat: Optional[float] = None      # monotonic deadline
        identified = False

        ws.settimeout(None)
        while True:
            now = time.monotonic()
            # Fire heartbeat if it's time.
            if (
                heartbeat_interval is not None
                and next_heartbeat is not None
                and now >= next_heartbeat
            ):
                try:
                    ws.send_text(json.dumps({"op": 1, "d": last_seq}))
                except OSError as e:
                    log.warn("qq heartbeat send failed", error=str(e))
                    return
                next_heartbeat = now + heartbeat_interval

            # Decide how long to block — until the next heartbeat
            # deadline (or READ_TICK_SECS, whichever sooner).
            if next_heartbeat is not None:
                wait_for = max(0.0, min(READ_TICK_SECS, next_heartbeat - now))
            else:
                wait_for = READ_TICK_SECS

            if not ws.wait_readable(wait_for):
                continue
            try:
                text, close = ws.recv_frame()
            except (EOFError, OSError) as e:
                log.warn("qq ws socket dropped", error=str(e))
                return
            if close is not None:
                code, reason = close
                log.info("qq ws closed",
                         code=code,
                         reason=reason.decode("utf-8", "replace"))
                return
            if text is None:
                continue
            try:
                payload = json.loads(text)
            except (ValueError, TypeError):
                log.warn("qq ws: malformed envelope JSON")
                continue
            if not isinstance(payload, dict):
                continue

            op = payload.get("op")
            if op == 10:  # HELLO
                d = payload.get("d") if isinstance(payload.get("d"), dict) else {}
                interval_ms = d.get("heartbeat_interval")
                if not isinstance(interval_ms, (int, float)) or interval_ms <= 0:
                    interval_ms = 45_000
                heartbeat_interval = float(interval_ms) / 1000.0
                next_heartbeat = time.monotonic() + heartbeat_interval
                log.info("qq HELLO received",
                         heartbeat_interval_secs=heartbeat_interval)
                identify = {
                    "op": 2,
                    "d": {
                        "token": f"QQBot {token}",
                        "intents": self.intents,
                        "shard": [0, 1],
                    },
                }
                try:
                    ws.send_text(json.dumps(identify))
                except OSError as e:
                    log.warn("qq IDENTIFY send failed", error=str(e))
                    return
                log.info("qq IDENTIFY sent", intents=self.intents)
            elif op == 0:  # DISPATCH
                s = payload.get("s")
                if isinstance(s, int):
                    last_seq = s
                event_type = payload.get("t")
                if not isinstance(event_type, str):
                    event_type = ""
                data = payload.get("d")
                if event_type == "READY" and not identified:
                    user = (data or {}).get("user") if isinstance(data, dict) else {}
                    bot_name = (user or {}).get("username", "QQBot") if isinstance(user, dict) else "QQBot"
                    log.info("qq READY", bot_name=bot_name)
                    identified = True
                    continue
                self._handle_dispatch(event_type, data, emit)
            elif op == 11:  # HEARTBEAT_ACK
                # No-op; the producer notes that the server is alive
                # via the read tick.
                pass
            elif op == 7:  # RECONNECT
                log.info("qq RECONNECT requested by server")
                return
            elif op == 9:  # INVALID_SESSION
                log.warn("qq INVALID_SESSION; will reconnect")
                # QQ recommends a small jitter before reconnect.
                time.sleep(3.0)
                return
            else:
                log.debug("qq unhandled opcode", op=op)

    def _gateway_loop(self, emit: Callable[[dict], None]) -> None:
        """Outer reconnect loop. Fetch token + gateway, open WS, run
        session, back off on every error path."""
        backoff = INITIAL_BACKOFF_SECS
        while True:
            try:
                token = self._fetch_token()
                with self._token_lock:
                    self._token = token
                log.info("qq access token acquired")
            except Exception as e:  # noqa: BLE001 — transport varies
                log.warn("qq token fetch failed; backing off",
                         error=str(e), delay=backoff)
                time.sleep(backoff)
                backoff = min(backoff * 2.0, MAX_BACKOFF_SECS)
                continue

            if self.ws_url_override:
                gw_url = self.ws_url_override
            else:
                try:
                    gw_url = self._fetch_gateway()
                except Exception as e:  # noqa: BLE001
                    log.warn("qq gateway fetch failed; backing off",
                             error=str(e), delay=backoff)
                    time.sleep(backoff)
                    backoff = min(backoff * 2.0, MAX_BACKOFF_SECS)
                    continue

            try:
                log.info("qq ws connecting", url=gw_url)
                with self._make_ws(gw_url, headers={}) as ws:
                    self._run_session(ws, token, emit)
                # Clean session end → reset backoff for next reconnect.
                backoff = INITIAL_BACKOFF_SECS
            except Exception as e:  # noqa: BLE001 — transport varies
                log.warn("qq ws error; backing off",
                         error=str(e), delay=backoff)
                time.sleep(backoff)
                backoff = min(backoff * 2.0, MAX_BACKOFF_SECS)

    # ---- public sidecar surface --------------------------------------

    async def produce(self, emit: Callable[[dict], None]) -> None:
        loop = asyncio.get_event_loop()
        await loop.run_in_executor(None, self._gateway_loop, emit)

    async def on_send(self, cmd) -> None:
        # Improvement #4: the inbound event sets ``channel_id`` to the
        # QQ reply endpoint and ``thread_id`` to the source ``msg_id``;
        # the bridge round-trips both back to us on outbound.
        reply_endpoint = (
            cmd.channel_id
            or (cmd.user.get("platform_id") if cmd.user else "")
            or ""
        )
        msg_id = getattr(cmd, "thread_id", None) or None
        if not reply_endpoint:
            log.warn("qq on_send: empty reply_endpoint, dropping")
            return

        content = cmd.content
        raw_text = cmd.text or ""
        loop = asyncio.get_event_loop()
        if isinstance(content, dict) and "Text" in content:
            text = strip_markdown(raw_text)
            await loop.run_in_executor(
                None,
                lambda: self._post_message(reply_endpoint, msg_id, text),
            )
            return
        if content and not (isinstance(content, dict) and "Text" in content):
            # Non-text content. The Rust adapter at qq.rs:491 silently
            # returned Ok on any non-text content; the sidecar surfaces
            # a clear placeholder so the operator sees the failure
            # mode (same shape as line / mattermost / signal).
            await loop.run_in_executor(
                None,
                lambda: self._post_message(
                    reply_endpoint, msg_id, "(Unsupported content type)",
                ),
            )
            return

        text = strip_markdown(raw_text)
        await loop.run_in_executor(
            None,
            lambda: self._post_message(reply_endpoint, msg_id, text),
        )


if __name__ == "__main__":
    run_stdio_main(QqAdapter)
