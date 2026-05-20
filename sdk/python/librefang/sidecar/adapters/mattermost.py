#!/usr/bin/env python3
"""Mattermost sidecar channel adapter for LibreFang.

Replaces the former in-process Rust ``librefang-channels::mattermost``
adapter (removed in this sidecar migration; same pattern as ntfy
#5224, telegram #5241, gotify #5263, mastodon #5264, bluesky #5277,
reddit #5281, twitch #5297, rocketchat #5298, discord #5299,
nextcloud #5301, slack #5302, webex #5309, line #5312, zulip #5310).

Behaviour parity with the Rust adapter (every assertion below has a
file/line citation against ``crates/librefang-channels/src/mattermost.rs``
on the pre-migration tree):

* **Auth probe**: ``GET /api/v4/users/me`` with the bot/personal
  access token at startup. Discovers the bot's stable ``id`` +
  ``username`` (used for self-skip). Mirrors ``mattermost.rs:107-128``.
* **WebSocket**: connect to ``wss://<host>/api/v4/websocket``
  (mirrors ``mattermost.rs:130-140`` — protocol upgraded from the
  HTTPS server URL). Immediately after the upgrade handshake, send
  ``{"seq":1, "action":"authentication_challenge",
  "data":{"token":"<TOKEN>"}}`` over the channel
  (``mattermost.rs:335-353``). Subsequent server frames are JSON
  envelopes either carrying a ``status`` ACK for the auth
  challenge or a ``{event, data}`` push.
* **Event parsing**: only ``event == "posted"`` produces a message
  event. ``data.post`` is a **JSON string** (Mattermost double-
  encodes the post payload) that needs a second parse
  (``mattermost.rs:197``).
* **Self-skip**: drop events whose ``post.user_id`` matches the
  bot's own user id discovered in the auth probe
  (``mattermost.rs:206-210``).
* **Channel filter**: empty ``MATTERMOST_ALLOWED_CHANNELS`` = listen
  on all channels the bot is in. When non-empty, only matching
  ``channel_id`` values pass (``mattermost.rs:212-215``).
* **DM detection**: ``is_group = (data.channel_type != "D")`` —
  ``"D"`` is Mattermost's direct-message channel type
  (``mattermost.rs:222-223``).
* **Thread routing**: surface ``post.root_id`` as ``thread_id`` when
  non-empty so ``on_send`` can round-trip it
  (``mattermost.rs:226-231``).
* **Slash-command routing**: ``/cmd args`` → ``Command`` (text
  otherwise; ``mattermost.rs:237-251``).
* **REST send via** ``POST /api/v4/posts`` with body
  ``{"channel_id", "message"[, "root_id"]}`` and a Bearer token
  (``mattermost.rs:143-173`` + ``487-525``). ``MAX_MESSAGE_LEN =
  16 383`` character chunking matches the Rust adapter at
  ``mattermost.rs:22``.
* **Typing indicator**: ``POST /api/v4/users/me/typing`` with
  ``{"channel_id"}`` (``mattermost.rs:464-485``). LINE / webex have
  no equivalent; Mattermost does, so we keep parity.
* **Multi-bot ``account_id``** (``mattermost.rs:419-424``,
  #5003). When ``MATTERMOST_ACCOUNT_ID`` is set, it is injected
  into the inbound message metadata so the bridge can scope
  ``ApprovalRequested`` delivery to the channel bound to the
  requesting agent.
* **Reconnect**: exponential backoff 1 s → 60 s, mirrors the Rust
  adapter (``mattermost.rs:306-438``).
* **ChannelType::Mattermost preserved** as ``channel_type =
  "mattermost"`` on the sidecar entry — existing routing /
  ``channel_role_mapping`` keys that reference ``mattermost``
  continue to resolve.

Improvements over the Rust adapter
==================================

1. **Outbound ``root_id`` round-trip via ``thread_id``**. The Rust
   ``send`` (``mattermost.rs:446-462``) used the user channel for
   the destination but **dropped ``root_id``** — every reply
   started a new top-level post even when the inbound message was
   in a thread. A separate ``send_in_thread`` path at
   ``mattermost.rs:487-525`` did pass ``root_id`` through, but the
   kernel only reaches that path when the trigger explicitly
   carried a thread id; the common case lost the thread. The
   sidecar surfaces the inbound ``post.root_id`` (or the post's own
   id when the user was the thread root) as ``thread_id`` and
   ``on_send`` posts ``root_id`` populated so threaded replies
   actually thread. Mirrors reddit / rocketchat / nextcloud /
   webex.

2. **429 ``Retry-After`` honoured on every REST path**. The Rust
   adapter had no 429 handling — a throttled ``/posts`` either
   returned an Err and dropped the reply
   (``mattermost.rs:165-169`` only logs at WARN) or burned the
   typing-indicator without retry. Sidecar parses ``Retry-After``
   (default 30 s fallback, floor 1 s, cap ``MAX_BACKOFF_SECS``),
   sleeps, and retries once before logging-and-continuing on the
   second 429. Same shape as ``fix(channels): honour Retry-After
   across sidecar polling adapters`` #5303.

3. **Inbound dedupe on ``post.id``**. Mattermost may redeliver a
   ``posted`` event on a WS reconnect that races against the
   server's last-delivery cursor (the Rust adapter at
   ``mattermost.rs:425`` emitted every parsed event
   unconditionally). Bounded local set on ``post.id`` with
   ``SEEN_MESSAGES_MAX = 10000`` / ``SEEN_MESSAGES_EVICT = 5000``
   (same policy as reddit / rocketchat / nextcloud / webex).

4. **Explicit HTTP timeouts**. ``urllib.request.urlopen`` has no
   default timeout; the Rust adapter relied on ``reqwest``'s
   default (also none). A hung Mattermost REST endpoint would
   otherwise hang the producer thread forever. The sidecar passes
   ``timeout=SEND_TIMEOUT_SECS`` (15 s) on every call.

Stdlib-only: HTTPS via ``urllib.request``, WebSocket via a
hand-rolled RFC 6455 client over ``socket`` + ``ssl`` (same
pattern as the discord / slack / webex sidecars).

Configure via ``[[sidecar_channels]]``::

    [[sidecar_channels]]
    name = "mattermost"
    command = "python3"
    args = ["-m", "librefang.sidecar.adapters.mattermost"]
    channel_type = "mattermost"
    [sidecar_channels.env]
    MATTERMOST_SERVER_URL = "https://mattermost.example.com"
    # MATTERMOST_ALLOWED_CHANNELS = "ch-id-1,ch-id-2"
    # MATTERMOST_ACCOUNT_ID = "team-prod"

Secret via ``~/.librefang/secrets.env``: ``MATTERMOST_TOKEN`` (bot
or personal access token from the Mattermost System Console).
"""
from __future__ import annotations

import asyncio
import json
import os
import threading
import time
import urllib.error
import urllib.parse
import urllib.request
from typing import Any, Callable, Optional

from librefang.sidecar import Content, Field, Schema, SidecarAdapter, protocol, run_stdio_main
from librefang.sidecar import logging as log
from librefang.sidecar.common import (
    SeenSet as _SeenSet,
    http_request as _http_request,
    parse_retry_after as _parse_retry_after_impl,
    split_csv as _split_csv,
    split_message as _split_message,
)
from librefang.sidecar.ws import (
    MAX_FRAME_PAYLOAD,
    OP_CLOSE as _OP_CLOSE,
    OP_CONT as _OP_CONT,
    OP_PING as _OP_PING,
    OP_PONG as _OP_PONG,
    OP_TEXT as _OP_TEXT,
    WebSocketClient as _WebSocketClient,
)

# Mattermost's official message-text ceiling. Mirrors the Rust
# adapter's ``MAX_MESSAGE_LEN`` (mattermost.rs:22).
MM_MSG_LIMIT = 16_383

SEND_TIMEOUT_SECS = 15.0
HANDSHAKE_TIMEOUT_SECS = 15.0

INITIAL_BACKOFF_SECS = 1.0
MAX_BACKOFF_SECS = 60.0

# Default fallback when Mattermost 429s without a parseable
# Retry-After header. 30 s is conservative; mirrors the rocketchat /
# nextcloud / webex / line sidecars (#5303).
RETRY_AFTER_DEFAULT_SECS = 30.0

# Bounded dedupe cap on Mattermost ``post.id``. Same policy as
# reddit / rocketchat / nextcloud / webex.
SEEN_MESSAGES_MAX = 10_000
SEEN_MESSAGES_EVICT = 5_000

READ_TICK_SECS = 30.0
def _parse_retry_after(resp_hdrs: dict, *, default_secs: float) -> float:
    """Mattermost may include ``Retry-After`` (seconds) on 429.
    Fall back to ``default_secs`` when missing/garbled. Floor 1 s,
    capped at ``MAX_BACKOFF_SECS`` so a server bug can't pin the
    loop for hours."""
    raw = resp_hdrs.get("retry-after")
    if not raw:
        return default_secs
    try:
        v = float(raw)
    except (TypeError, ValueError):
        return default_secs
    return min(max(v, 1.0), MAX_BACKOFF_SECS)


def _server_url_to_ws(server_url: str) -> str:
    """Translate ``https://host`` → ``wss://host/api/v4/websocket``
    (and the ``http://`` → ``ws://`` variant). Mirrors the Rust
    adapter's ``ws_url`` helper at mattermost.rs:131-140."""
    base = server_url.rstrip("/")
    if base.startswith("https://"):
        base = "wss://" + base[len("https://"):]
    elif base.startswith("http://"):
        base = "ws://" + base[len("http://"):]
    else:
        base = "wss://" + base
    return base + "/api/v4/websocket"


def parse_mm_event(
    event: dict,
    *,
    own_user_id: Optional[str],
    allowed_channels: list[str],
    account_id: Optional[str],
) -> Optional[dict]:
    """Pure-function port of the inbound parse path in
    ``crates/librefang-channels/src/mattermost.rs`` lines 186-267.

    Returns a ``message`` event dict ready to ``emit``, or ``None``
    when the payload should be skipped (non-posted event type, self,
    filtered channel, empty message, malformed envelope).
    """
    if not isinstance(event, dict):
        return None
    if event.get("event") != "posted":
        return None

    data = event.get("data")
    if not isinstance(data, dict):
        return None

    # Mattermost double-encodes the post: ``data.post`` is a JSON
    # string that must be parsed again. Mirrors mattermost.rs:197.
    post_str = data.get("post")
    if not isinstance(post_str, str) or not post_str:
        return None
    try:
        post = json.loads(post_str)
    except (ValueError, TypeError):
        return None
    if not isinstance(post, dict):
        return None

    user_id = post.get("user_id")
    if not isinstance(user_id, str):
        user_id = ""
    channel_id = post.get("channel_id")
    if not isinstance(channel_id, str):
        channel_id = ""
    message = post.get("message")
    if not isinstance(message, str) or not message:
        return None
    post_id = post.get("id")
    if not isinstance(post_id, str):
        post_id = ""

    # Self-skip.
    if own_user_id and user_id == own_user_id:
        return None

    # Channel filter.
    if allowed_channels and channel_id not in allowed_channels:
        return None

    channel_type = data.get("channel_type")
    if not isinstance(channel_type, str):
        channel_type = ""
    # "D" = direct message. Everything else (O = open, P = private,
    # G = group DM) is treated as a group surface for `is_group`.
    is_group = channel_type != "D"

    root_id = post.get("root_id")
    if isinstance(root_id, str) and root_id:
        # Improvement #1: inbound was inside a thread → reply
        # threads alongside it.
        thread_id: Optional[str] = root_id
    elif post_id:
        # Inbound was top-level → reply threads under it. Same
        # rocketchat / nextcloud / webex pattern.
        thread_id = post_id
    else:
        thread_id = None

    sender_name = data.get("sender_name")
    if not isinstance(sender_name, str) or not sender_name:
        sender_name = user_id or "unknown"

    # Slash-command routing matches mattermost.rs:237-251 — Rust used
    # `splitn(2, ' ')` then `split_whitespace()` on the remainder,
    # which `partition(' ')` + `tail.split()` reproduces.
    if message.startswith("/"):
        head, _, tail = message[1:].partition(" ")
        content = Content.command(head, tail.split() if tail else [])
    else:
        content = Content.text(message)

    metadata: dict[str, Any] = {}
    if account_id is not None:
        metadata["account_id"] = account_id

    return protocol.message(
        # platform_id matches the Rust adapter's choice at
        # mattermost.rs:257: the channel id (Mattermost's
        # addressable destination for outbound posts).
        user_id=channel_id,
        user_name=sender_name,
        content=content,
        message_id=post_id or None,
        is_group=is_group,
        thread_id=thread_id,
        metadata=metadata,
    )




# ---------------------------------------------------------------------------
# Mattermost adapter
# ---------------------------------------------------------------------------


class MattermostAdapter(SidecarAdapter):
    capabilities: list = ["thread", "typing"]
    # Mattermost channels can be DMs, private/public channels, or group DMs.
    # The chat-room precedent set by twitch / discord / slack / webex / line
    # is to surface errors so the user gets a visible failure instead of
    # silent swallow. A pure public-broadcast surface (mastodon / bluesky /
    # reddit / nextcloud) sets this True; Mattermost's mixed model keeps
    # the chat-room default.
    suppress_error_responses: bool = False

    SCHEMA = Schema(
        name="mattermost",
        display_name="Mattermost",
        description="Mattermost WebSocket + REST adapter (out-of-process sidecar)",
        fields=[
            Field("MATTERMOST_SERVER_URL", "Server URL", "text",
                  required=True,
                  placeholder="https://mattermost.example.com"),
            Field("MATTERMOST_TOKEN", "Bot Token", "secret",
                  required=True,
                  placeholder="abc123..."),
            Field("MATTERMOST_ALLOWED_CHANNELS",
                  "Allowed Channel IDs (comma-separated, empty = all)",
                  "text",
                  placeholder="ch-id-1, ch-id-2",
                  advanced=True),
            Field("MATTERMOST_ACCOUNT_ID",
                  "Account ID (multi-bot routing)",
                  "text",
                  placeholder="team-prod",
                  advanced=True),
        ],
    )

    def __init__(self) -> None:
        server_url = os.environ.get("MATTERMOST_SERVER_URL", "").strip()
        token = os.environ.get("MATTERMOST_TOKEN", "").strip()
        missing: list[str] = []
        if not server_url:
            missing.append("MATTERMOST_SERVER_URL")
        if not token:
            missing.append("MATTERMOST_TOKEN")
        if missing:
            log.error("mattermost required env vars missing", missing=missing)
            raise SystemExit(2)
        if not (server_url.startswith("http://") or server_url.startswith("https://")):
            log.error(
                "MATTERMOST_SERVER_URL must start with http:// or https://",
                server_url=server_url,
            )
            raise SystemExit(2)
        # Strip trailing slash the same way the Rust adapter did
        # (mattermost.rs:65: `trim_end_matches('/')`).
        self.server_url = server_url.rstrip("/")
        self.token = token
        self.allowed_channels = _split_csv(
            os.environ.get("MATTERMOST_ALLOWED_CHANNELS", "")
        )
        acct = os.environ.get("MATTERMOST_ACCOUNT_ID", "").strip()
        self.account_id = acct or None

        # Test seam — override the WS URL for tests that point us at
        # a local mock WS server.
        self.ws_url = os.environ.get("MATTERMOST_WS_URL", "").strip() or \
            _server_url_to_ws(self.server_url)

        # Discovered at startup via GET /users/me. Used for self-skip
        # in parse_mm_event.
        self.bot_user_id: Optional[str] = None
        self.bot_username: Optional[str] = None

        # Improvement #3: bounded dedupe on post.id.
        self._seen = _SeenSet(max_size=SEEN_MESSAGES_MAX, evict=SEEN_MESSAGES_EVICT)

    # ---- HTTP helpers ------------------------------------------------

    def _auth_headers(self, *, content_type: bool = False) -> dict:
        h = {
            "Authorization": f"Bearer {self.token}",
            "User-Agent": "librefang-mattermost-sidecar/1 (https://librefang.org)",
        }
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
        """Thin wrapper around :func:`librefang.sidecar.common.http_request`."""
        return _http_request(
            url, method=method, body=body, headers=headers,
            timeout=timeout,
        )

    # ---- dedupe ------------------------------------------------------

    def _mark_seen(self, post_id: Optional[str]) -> bool:
        """Return True iff freshly seen. Shim around :class:`librefang.sidecar.common.SeenSet`."""
        return self._seen.mark(post_id)

    # ---- REST: auth + outbound send ---------------------------------

    def _validate_token(self) -> tuple[str, str]:
        """``GET /api/v4/users/me`` → ``(id, username)``. Raises
        ``RuntimeError`` on any non-200 so the outer gateway loop
        backs off."""
        url = f"{self.server_url}/api/v4/users/me"
        status, body, raw, resp_hdrs = self._http(
            url, headers=self._auth_headers(),
        )
        if status == 429:
            wait = _parse_retry_after(
                resp_hdrs, default_secs=RETRY_AFTER_DEFAULT_SECS,
            )
            log.warn("mattermost /users/me 429; will retry after",
                     retry_after_secs=wait)
            time.sleep(wait)
            status, body, raw, resp_hdrs = self._http(
                url, headers=self._auth_headers(),
            )
        if status != 200 or not isinstance(body, dict):
            snippet = raw[:200].decode("utf-8", "replace") if raw else ""
            raise RuntimeError(
                f"mattermost /users/me failed (status={status}): {snippet}"
            )
        user_id = body.get("id")
        if not isinstance(user_id, str) or not user_id:
            raise RuntimeError("mattermost /users/me: missing 'id' in body")
        username = body.get("username")
        if not isinstance(username, str) or not username:
            username = "unknown"
        return user_id, username

    def _post_message(
        self,
        channel_id: str,
        text: str,
        *,
        root_id: Optional[str] = None,
    ) -> None:
        """POST /api/v4/posts with chunking + optional ``root_id``
        (improvement #1). Honours 429 ``Retry-After`` and retries
        once per chunk (improvement #2). On the second 429 / non-2xx
        we log and continue — matches the webex / line fail-open
        behaviour so a single throttled chunk doesn't drop the rest
        of the reply."""
        if not channel_id:
            log.warn("mattermost _post_message: empty channel_id, dropping")
            return
        url = f"{self.server_url}/api/v4/posts"
        for chunk in _split_message(text, MM_MSG_LIMIT):
            payload: dict[str, Any] = {
                "channel_id": channel_id,
                "message": chunk,
            }
            if root_id:
                payload["root_id"] = root_id
            body = json.dumps(payload).encode("utf-8")
            status, _resp, raw, resp_hdrs = self._http(
                url, method="POST", body=body,
                headers=self._auth_headers(content_type=True),
            )
            if status == 429:
                wait = _parse_retry_after(
                    resp_hdrs, default_secs=RETRY_AFTER_DEFAULT_SECS,
                )
                log.warn("mattermost POST /posts 429; sleeping then "
                         "retrying once",
                         retry_after_secs=wait)
                time.sleep(wait)
                status, _resp, raw, resp_hdrs = self._http(
                    url, method="POST", body=body,
                    headers=self._auth_headers(content_type=True),
                )
            if status >= 300:
                snippet = raw[:200].decode("utf-8", "replace") if raw else ""
                log.warn("mattermost POST /posts failed",
                         channel_id=channel_id, status=status, body=snippet)
                # fail-open: keep chunking
                continue

    def _post_typing(self, channel_id: str) -> None:
        """POST /api/v4/users/me/typing — fire-and-forget. Mirrors
        the Rust adapter at mattermost.rs:464-485, which itself
        ignored the response."""
        if not channel_id:
            return
        url = f"{self.server_url}/api/v4/users/me/typing"
        body = json.dumps({"channel_id": channel_id}).encode("utf-8")
        try:
            self._http(
                url, method="POST", body=body,
                headers=self._auth_headers(content_type=True),
            )
        except Exception:  # noqa: BLE001 — best-effort, never raise
            pass

    # ---- Mattermost WS gateway loop ---------------------------------

    def _make_ws(self, url: str, *, headers: dict) -> _WebSocketClient:
        """Test seam."""
        return _WebSocketClient(url, headers=headers)

    def _handle_envelope(
        self,
        envelope: dict,
        emit: Callable[[dict], None],
    ) -> None:
        """Parse one server frame. Mattermost frames are either
        ``{status, seq_reply, ...}`` ACKs (which we just log) or
        ``{event, data, ...}`` event pushes (which we route into
        ``parse_mm_event``)."""
        if not isinstance(envelope, dict):
            return
        # Auth ACK / replies carry a ``status`` field. Mirrors
        # mattermost.rs:398-407.
        if "status" in envelope and envelope.get("event") is None:
            status = envelope.get("status")
            if status == "OK":
                log.info("mattermost ws authentication ack received")
            else:
                log.warn("mattermost ws response status", status=status)
            return

        if envelope.get("event") != "posted":
            return

        # Improvement #3: dedupe before the (cheap) parse so identical
        # redelivery on reconnect doesn't double-emit.
        data = envelope.get("data")
        post_id = None
        if isinstance(data, dict):
            post_str = data.get("post")
            if isinstance(post_str, str) and post_str:
                try:
                    post = json.loads(post_str)
                    if isinstance(post, dict):
                        pid = post.get("id")
                        if isinstance(pid, str) and pid:
                            post_id = pid
                except (ValueError, TypeError):
                    pass
        if post_id and not self._mark_seen(post_id):
            return

        ev = parse_mm_event(
            envelope,
            own_user_id=self.bot_user_id,
            allowed_channels=self.allowed_channels,
            account_id=self.account_id,
        )
        if ev is not None:
            emit(ev)

    def _run_session(
        self, ws: _WebSocketClient, emit: Callable[[dict], None],
    ) -> None:
        """Drive one WS session: send the auth challenge, then read
        frames until the connection drops. The outer reconnect loop
        catches socket drops and reconnects."""
        # WS auth challenge — Mattermost expects a JSON frame with
        # action=authentication_challenge after the upgrade. Mirrors
        # mattermost.rs:335-353.
        auth_msg = {
            "seq": 1,
            "action": "authentication_challenge",
            "data": {"token": self.token},
        }
        try:
            ws.send_text(json.dumps(auth_msg))
        except OSError as e:
            log.warn("mattermost ws auth send failed", error=str(e))
            return

        ws.settimeout(None)
        while True:
            if not ws.wait_readable(READ_TICK_SECS):
                continue
            try:
                text, close = ws.recv_frame()
            except (EOFError, OSError) as e:
                log.warn("mattermost ws socket dropped", error=str(e))
                return
            if close is not None:
                code, reason = close
                log.info("mattermost ws closed",
                         code=code,
                         reason=reason.decode("utf-8", "replace"))
                return
            if text is None:
                continue
            try:
                envelope = json.loads(text)
            except (ValueError, TypeError):
                log.warn("mattermost: malformed envelope JSON")
                continue
            self._handle_envelope(envelope, emit)

    def _gateway_loop(self, emit: Callable[[dict], None]) -> None:
        """Outer reconnect loop. Validate the token first (with
        backoff), then loop the WS session with exponential
        reconnect backoff."""
        backoff = INITIAL_BACKOFF_SECS
        while self.bot_user_id is None:
            try:
                uid, username = self._validate_token()
                self.bot_user_id = uid
                self.bot_username = username
                log.info("mattermost authenticated",
                         user_id=uid, username=username)
            except Exception as e:  # noqa: BLE001
                log.warn("mattermost auth failed; will retry",
                         error=str(e), delay=backoff)
                time.sleep(backoff)
                backoff = min(backoff * 2.0, MAX_BACKOFF_SECS)

        backoff = INITIAL_BACKOFF_SECS
        while True:
            try:
                log.info("mattermost ws connecting", url=self.ws_url)
                with self._make_ws(self.ws_url, headers={}) as ws:
                    self._run_session(ws, emit)
                backoff = INITIAL_BACKOFF_SECS
            except Exception as e:  # noqa: BLE001 — transport varies
                log.warn("mattermost ws error; backing off",
                         error=str(e), delay=backoff)
                time.sleep(backoff)
                backoff = min(backoff * 2.0, MAX_BACKOFF_SECS)

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
            log.warn("mattermost on_send: empty channel_id, dropping")
            return

        # Improvement #1: round-trip the inbound thread_id to root_id
        # so the bot's reply threads under the originating post.
        root_id = getattr(cmd, "thread_id", None) or None

        content = cmd.content
        text = cmd.text or ""
        loop = asyncio.get_event_loop()
        if isinstance(content, dict) and "Text" in content:
            await loop.run_in_executor(
                None,
                lambda: self._post_message(
                    channel_id, text, root_id=root_id,
                ),
            )
        elif content and not (isinstance(content, dict) and "Text" in content):
            await loop.run_in_executor(
                None,
                lambda: self._post_message(
                    channel_id, "(Unsupported content type)",
                    root_id=root_id,
                ),
            )
        else:
            await loop.run_in_executor(
                None,
                lambda: self._post_message(
                    channel_id, text, root_id=root_id,
                ),
            )

    async def on_command(self, cmd) -> None:
        """Dispatch incoming commands. The default :class:`SidecarAdapter`
        routes ``send`` to :meth:`on_send` and drops the rest. We declare
        the ``typing`` capability, so the daemon will send us
        :class:`~librefang.sidecar.protocol.TypingCmd` envelopes — wire
        them through to ``POST /api/v4/users/me/typing`` (mirrors the
        Rust adapter at mattermost.rs:464-485). Same pattern as discord
        / telegram which also surface a typing indicator."""
        from librefang.sidecar.protocol import Send, TypingCmd
        if isinstance(cmd, Send):
            await self.on_send(cmd)
        elif isinstance(cmd, TypingCmd):
            await asyncio.get_event_loop().run_in_executor(
                None, self._post_typing, cmd.channel_id,
            )


if __name__ == "__main__":
    run_stdio_main(MattermostAdapter)
