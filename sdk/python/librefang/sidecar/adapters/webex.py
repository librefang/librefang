#!/usr/bin/env python3
"""Webex Bot sidecar channel adapter for LibreFang.

Replaces the former in-process Rust ``librefang-channels::webex``
adapter (removed in this sidecar migration; same pattern as ntfy
#5224, telegram #5241, gotify #5263, mastodon #5264, bluesky #5277,
reddit #5281, twitch #5297, rocketchat #5298, discord #5299,
nextcloud #5301, slack #5302).

Behaviour parity with the Rust adapter:

* **Auth probe**: ``GET /people/me`` with the bot token at startup
  to discover the bot's own ``id`` + ``displayName`` (used for
  self-skip).
* **Mercury WebSocket**: hard-coded URL
  ``wss://mercury-connection-a.wbx2.com/v1/apps/wx2/registrations``
  with ``Authorization: Bearer <token>`` on the upgrade request.
  No device-registration handshake (the Rust adapter never did one
  either — it relies on Cisco's gateway accepting the bare WSS
  connect with a Bearer header).
* **Event handling**: parse ``data.activity`` envelopes; only
  ``verb == "post"`` produces a message event. Skip when
  ``activity.actor.id == own_bot_id``.
* **Two-step message fetch**: Mercury carries only activity
  metadata (``object.id``, ``target.id``). We follow up with
  ``GET /messages/<id>`` to retrieve the actual ``text`` /
  ``personEmail`` / ``personId`` / ``roomType``. Same shape as
  the Rust adapter's REST follow-up.
* **Room filter**: empty ``WEBEX_ALLOWED_ROOMS`` = listen on all
  rooms the bot is in. When non-empty, only the listed
  ``activity.target.id`` values pass.
* **Slash-command routing**: ``/cmd args`` → ``Command`` (text
  otherwise).
* **DM vs group**: ``is_group = (roomType == "group")``.
* **REST send**: ``POST /messages`` with ``{"roomId", "text"}``,
  optional ``parentId`` for threaded replies (see improvement #1
  below). 7 439-char chunking matches the Rust adapter's
  ``MAX_MESSAGE_LEN``.
* **Account ID**: optional ``WEBEX_ACCOUNT_ID`` is injected as
  ``account_id`` in inbound message metadata so the bridge's
  multi-bot routing can pin per-org.
* **Reconnect**: exponential backoff 1 s → 60 s, mirrors the Rust
  adapter (see ``webex.rs:280-308`` for the exact ladder).

Improvements over the Rust adapter
==================================

1. **``parentId`` outbound threading wired**. The Rust
   ``api_send_message`` (``crates/librefang-channels/src/webex.rs``
   lines 171-201 on the migrating tree) built a body of just
   ``{"roomId", "text"}`` — Webex's ``parentId`` field (which
   threads a reply under a parent message in a Space) was never
   sent. The inbound side dropped the message id entirely
   (``thread_id: None`` at line 438 of the same file), so even
   when we knew the parent we had nothing to round-trip. The
   sidecar surfaces the inbound message id (or the inbound
   ``parentId`` when the user themselves was already inside a
   thread, so the bot threads alongside rather than starting a
   nested child) as ``thread_id``, and ``on_send`` posts
   ``parentId`` populated so threaded replies actually thread.
   Mirrors reddit / rocketchat / nextcloud / mastodon / bluesky.

2. **429 ``Retry-After`` honoured on both fetch and send**.
   Webex documents 429 with ``Retry-After``. The Rust adapter
   had no 429 handling at either ``GET /messages/<id>``
   (``webex.rs:380-398``) or ``POST /messages``
   (``webex.rs:171-201``); a server-side rate-limit either lost
   the inbound fetch or caused ``send()`` to return an Err and
   drop the outbound. The sidecar parses ``Retry-After`` (with a
   ``RETRY_AFTER_DEFAULT_SECS = 30.0`` floor), sleeps, and retries
   once before logging-and-continuing on the second 429. Same
   pattern as the merged
   ``fix(channels): honour Retry-After across sidecar polling
   adapters (#5303)``.

3. **Mercury activity-id dedupe**. Mercury can re-deliver an
   ``activity.object.id`` on reconnect (the Rust adapter had no
   dedupe, see the unconditional emit at ``webex.rs:459`` — the
   only filters were verb / self / empty-id / allowed-rooms).
   Operators with a flaky network saw the bot react twice to the
   same message after a transient drop. The sidecar dedupes
   locally on ``activity.object.id`` with a bounded
   ``SEEN_MESSAGES_MAX = 10 000`` /
   ``SEEN_MESSAGES_EVICT = 5 000`` cap (same policy as reddit /
   rocketchat / nextcloud).

4. **Explicit HTTP timeouts**. ``urllib.request.urlopen`` has no
   default timeout; the Rust adapter relied on ``reqwest``'s
   default (none either, by default). A hung Webex API hangs the
   producer thread forever. The sidecar passes
   ``timeout=SEND_TIMEOUT_SECS`` (15 s) on every ``urlopen``,
   so a misbehaving REST endpoint trips an explicit error and
   loops the reconnect backoff instead of hanging.

Stdlib-only: HTTPS via ``urllib.request``, WebSocket via a
hand-rolled RFC 6455 client over ``socket`` + ``ssl`` (same
pattern as the discord / slack / nextcloud sidecars).

Configure via ``[[sidecar_channels]]``::

    [[sidecar_channels]]
    name = "webex"
    command = "python3"
    args = ["-m", "librefang.sidecar.adapters.webex"]
    channel_type = "webex"
    [sidecar_channels.env]
    # WEBEX_ALLOWED_ROOMS = "Y2lz...A,Y2lz...B"
    # WEBEX_ACCOUNT_ID = "org-prod"

Secret via ``~/.librefang/secrets.env``: ``WEBEX_BOT_TOKEN`` (the
bot Bearer token from developer.webex.com).
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

# Webex constants — mirror crate::webex defaults.
DEFAULT_API_BASE = "https://webexapis.com/v1"
# Hard-coded Mercury endpoint, copied verbatim from
# crates/librefang-channels/src/webex.rs:26. Cisco-internal device
# gateway. There is NO POST /devices registration step — the Rust
# adapter relied on the gateway accepting a bare WSS connect with a
# Bearer header, and we do the same.
DEFAULT_WS_URL = "wss://mercury-connection-a.wbx2.com/v1/apps/wx2/registrations"

# Webex's official message-text ceiling. Mirrors the Rust adapter's
# ``MAX_MESSAGE_LEN`` (see crates/librefang-channels/src/webex.rs:29).
WEBEX_MSG_LIMIT = 7439

SEND_TIMEOUT_SECS = 15.0
HANDSHAKE_TIMEOUT_SECS = 15.0

INITIAL_BACKOFF_SECS = 1.0
MAX_BACKOFF_SECS = 60.0

# Default fallback when Webex 429s without a parseable ``Retry-After``
# header. Mirrors the rocketchat / nextcloud / mastodon sidecars
# (#5303); 30 s is conservative enough that we don't immediately
# re-hit the bruteforce throttle.
RETRY_AFTER_DEFAULT_SECS = 30.0

# Bounded dedupe cap on Mercury ``activity.object.id``. Same policy as
# reddit / rocketchat / nextcloud. ``MAX`` is the high-water mark;
# when reached, evict the oldest ``EVICT`` entries (so the steady-state
# is between EVICT and MAX, not a flap around MAX).
SEEN_MESSAGES_MAX = 10_000
SEEN_MESSAGES_EVICT = 5_000

READ_TICK_SECS = 30.0
def _parse_retry_after(resp_hdrs: dict, *, default_secs: float) -> float:
    """Webex's 429 response includes ``Retry-After`` (seconds).
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


def parse_webex_message(
    full_msg: dict,
    activity: dict,
    *,
    own_bot_id: Optional[str],
    allowed_rooms: list[str],
    account_id: Optional[str],
) -> Optional[dict]:
    """Pure-function port of the inbound parse path in
    ``crates/librefang-channels/src/webex.rs`` lines 352-461.

    ``full_msg`` is the body of ``GET /messages/<id>``; ``activity``
    is the Mercury envelope's ``data.activity`` block. Returns the
    ``message`` event dict ready to ``emit``, or ``None`` when the
    payload should be skipped.

    Improvements over the Rust adapter (see module header for the
    full list with file/line evidence): ``thread_id`` is now
    populated from the inbound id (or ``parentId`` when the user was
    already in a thread) so ``on_send`` can round-trip ``parentId``.
    """
    if not isinstance(activity, dict):
        return None
    verb = activity.get("verb")
    if verb != "post":
        return None

    actor = activity.get("actor") or {}
    actor_id = actor.get("id") if isinstance(actor, dict) else None
    # Self-skip — drop messages from the bot itself.
    if own_bot_id and isinstance(actor_id, str) and actor_id == own_bot_id:
        return None

    obj = activity.get("object") or {}
    message_id = obj.get("id") if isinstance(obj, dict) else None
    if not isinstance(message_id, str) or not message_id:
        return None

    target = activity.get("target") or {}
    activity_room_id = target.get("id") if isinstance(target, dict) else ""
    if not isinstance(activity_room_id, str):
        activity_room_id = ""

    # Filter by room (when configured). The Rust adapter ran this
    # check against the activity's target.id (before the REST
    # fetch); we do the same so the fetch is skipped when filtered
    # out.
    if allowed_rooms and activity_room_id not in allowed_rooms:
        return None

    if not isinstance(full_msg, dict):
        return None

    msg_text = full_msg.get("text")
    if not isinstance(msg_text, str) or not msg_text:
        return None

    sender_email = full_msg.get("personEmail")
    if not isinstance(sender_email, str) or not sender_email:
        sender_email = "unknown"
    # Prefer the personDisplayName for the user-facing label so bot
    # logs / UI surface "Alice" instead of "alice@example.com". Falls
    # back to personEmail when the field is absent (older Webex orgs)
    # or empty (some service-account principals omit it). The Rust
    # adapter at webex.rs:431 used personEmail unconditionally; this
    # is a pure UX win, no behavioural change to routing (which
    # keys on sender_id / sender_email below).
    sender_display = full_msg.get("personDisplayName")
    if not isinstance(sender_display, str) or not sender_display:
        sender_display = sender_email
    sender_id = full_msg.get("personId")
    if not isinstance(sender_id, str):
        sender_id = ""
    full_room_id = full_msg.get("roomId")
    if not isinstance(full_room_id, str) or not full_room_id:
        full_room_id = activity_room_id
    room_type = full_msg.get("roomType")
    if not isinstance(room_type, str):
        room_type = "group"
    is_group = room_type == "group"

    # Slash-command routing.
    if msg_text.startswith("/"):
        head, _, tail = msg_text[1:].partition(" ")
        content = Content.command(head, tail.split() if tail else [])
    else:
        content = Content.text(msg_text)

    # Improvement #1: thread routing. If the user themselves was
    # already inside a thread, thread alongside (their parent =
    # our parent); otherwise our parent is the message id so the
    # bot's reply threads under what triggered it. Mirrors the
    # rocketchat / nextcloud / reddit pattern.
    inbound_parent = full_msg.get("parentId")
    if isinstance(inbound_parent, str) and inbound_parent:
        thread_id: Optional[str] = inbound_parent
    else:
        thread_id = message_id

    metadata: dict[str, Any] = {
        "sender_id": sender_id,
        "sender_email": sender_email,
    }
    if account_id is not None:
        metadata["account_id"] = account_id

    return protocol.message(
        # platform_id is the room id (Webex address is the room,
        # not the person — matches the Rust adapter's
        # `sender.platform_id = full_room_id` at webex.rs:430).
        user_id=full_room_id,
        # personDisplayName when available; personEmail otherwise. The
        # Rust adapter at webex.rs:431 used personEmail unconditionally
        # — bot logs leaked emails into UI surfaces.
        user_name=sender_display,
        content=content,
        message_id=message_id,
        is_group=is_group,
        thread_id=thread_id,
        metadata=metadata,
    )




# ---------------------------------------------------------------------------
# Webex adapter
# ---------------------------------------------------------------------------


class WebexAdapter(SidecarAdapter):
    capabilities: list = ["thread"]
    # Webex spaces support direct (1:1) and group rooms; the chat-room
    # precedent set by twitch / discord / slack is to surface errors
    # so the user gets a visible failure instead of silent swallow. A
    # public-broadcast surface like mastodon / bluesky / reddit /
    # nextcloud would set this True to avoid echoing internal errors to
    # every member; Webex spaces are typically smaller, identifiable
    # groups, so the chat-room default applies here.
    suppress_error_responses: bool = False

    SCHEMA = Schema(
        name="webex",
        display_name="Webex",
        description="Cisco Webex bot adapter (out-of-process sidecar)",
        fields=[
            Field("WEBEX_BOT_TOKEN", "Bot Token", "secret",
                  required=True,
                  placeholder="NjIzOTkz..."),
            Field("WEBEX_ALLOWED_ROOMS",
                  "Allowed Room IDs (comma-separated, empty = allow all)",
                  "text",
                  placeholder="Y2lzY29zcGFyazov...",
                  advanced=True),
            Field("WEBEX_ACCOUNT_ID",
                  "Account ID (multi-bot routing)",
                  "text",
                  placeholder="org-prod",
                  advanced=True),
        ],
    )

    def __init__(self) -> None:
        bot_token = os.environ.get("WEBEX_BOT_TOKEN", "").strip()
        if not bot_token:
            log.error("webex required env var missing", missing=["WEBEX_BOT_TOKEN"])
            raise SystemExit(2)
        self.bot_token = bot_token
        self.allowed_rooms = _split_csv(
            os.environ.get("WEBEX_ALLOWED_ROOMS", "")
        )
        acct = os.environ.get("WEBEX_ACCOUNT_ID", "").strip()
        self.account_id = acct or None

        # Test seam — overridable via env var so tests can point us
        # at a local mock without monkey-patching urllib globally.
        self.api_base = os.environ.get("WEBEX_API_BASE", "").strip() or DEFAULT_API_BASE
        self.ws_url = os.environ.get("WEBEX_WS_URL", "").strip() or DEFAULT_WS_URL
        # Discovered at startup via GET /people/me. Used for self-skip
        # in parse_webex_message.
        self.bot_user_id: Optional[str] = None
        self.bot_display_name: Optional[str] = None

        # Improvement #3: bounded dedupe on activity.object.id.
        self._seen = _SeenSet(max_size=SEEN_MESSAGES_MAX, evict=SEEN_MESSAGES_EVICT)

    # ---- HTTP helpers ------------------------------------------------

    def _auth_headers(self, *, content_type: bool = False) -> dict:
        h = {
            "Authorization": f"Bearer {self.bot_token}",
            "User-Agent": "librefang-webex-sidecar/1 (https://librefang.org)",
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

    def _mark_seen(self, message_id: str) -> bool:
        """Return True iff freshly seen. Shim around
        :class:`librefang.sidecar.common.SeenSet` with webex's
        historical empty-id → False override (drop the event
        instead of treating an empty id as fresh — different from
        the other adapters)."""
        if not message_id:
            return False
        return self._seen.mark(message_id)

    # ---- REST: auth, message fetch, send ----------------------------

    def _validate_bot_token(self) -> tuple[str, str]:
        """``GET /people/me`` → ``(id, displayName)``. Raises
        ``RuntimeError`` on any non-200 response so the producer loop
        can back off and retry."""
        status, body, raw, resp_hdrs = self._http(
            f"{self.api_base}/people/me",
            headers=self._auth_headers(),
        )
        if status == 429:
            wait = _parse_retry_after(
                resp_hdrs, default_secs=RETRY_AFTER_DEFAULT_SECS,
            )
            log.warn("webex /people/me 429; will retry after",
                     retry_after_secs=wait)
            time.sleep(wait)
            status, body, raw, resp_hdrs = self._http(
                f"{self.api_base}/people/me",
                headers=self._auth_headers(),
            )
        if status != 200 or not isinstance(body, dict):
            snippet = raw[:200].decode("utf-8", "replace") if raw else ""
            raise RuntimeError(
                f"webex /people/me failed (status={status}): {snippet}"
            )
        bot_id = body.get("id")
        if not isinstance(bot_id, str) or not bot_id:
            raise RuntimeError("webex /people/me: missing 'id' in 200 body")
        display = body.get("displayName")
        if not isinstance(display, str) or not display:
            display = "LibreFang Bot"
        return bot_id, display

    def _fetch_message(self, message_id: str) -> Optional[dict]:
        """``GET /messages/<id>`` → full message body. Returns ``None``
        on a non-recoverable error (logged); on 429 we honour
        ``Retry-After`` and retry once before giving up. Improvement
        #2."""
        url = f"{self.api_base}/messages/{urllib.parse.quote(message_id, safe='')}"
        status, body, raw, resp_hdrs = self._http(
            url, headers=self._auth_headers(),
        )
        if status == 429:
            wait = _parse_retry_after(
                resp_hdrs, default_secs=RETRY_AFTER_DEFAULT_SECS,
            )
            log.warn("webex /messages 429; sleeping then retrying once",
                     message_id=message_id, retry_after_secs=wait)
            time.sleep(wait)
            status, body, raw, resp_hdrs = self._http(
                url, headers=self._auth_headers(),
            )
        if status >= 300 or not isinstance(body, dict):
            snippet = raw[:200].decode("utf-8", "replace") if raw else ""
            log.warn("webex /messages fetch failed",
                     message_id=message_id, status=status, body=snippet)
            return None
        return body

    def _post_message(
        self,
        room_id: str,
        text: str,
        *,
        parent_id: Optional[str] = None,
    ) -> None:
        """POST /messages with chunking + optional ``parentId``
        (improvement #1). Honours 429 ``Retry-After`` and retries once
        per chunk (improvement #2). On the second 429 we log and
        continue chunking — matches the discord / slack fail-open
        behaviour so a single throttled chunk doesn't drop the rest of
        the reply."""
        url = f"{self.api_base}/messages"
        chunks = _split_message(text, WEBEX_MSG_LIMIT)
        for chunk in chunks:
            payload: dict[str, Any] = {"roomId": room_id, "text": chunk}
            if parent_id:
                payload["parentId"] = parent_id
            body = json.dumps(payload).encode("utf-8")
            status, resp, raw, resp_hdrs = self._http(
                url, method="POST", body=body,
                headers=self._auth_headers(content_type=True),
            )
            if status == 429:
                wait = _parse_retry_after(
                    resp_hdrs, default_secs=RETRY_AFTER_DEFAULT_SECS,
                )
                log.warn("webex POST /messages 429; sleeping then retrying once",
                         room_id=room_id, retry_after_secs=wait)
                time.sleep(wait)
                status, resp, raw, resp_hdrs = self._http(
                    url, method="POST", body=body,
                    headers=self._auth_headers(content_type=True),
                )
            if status >= 300:
                snippet = raw[:200].decode("utf-8", "replace") if raw else ""
                log.warn("webex POST /messages failed",
                         room_id=room_id, status=status, body=snippet)
                # fail-open — continue with remaining chunks
                continue

    # ---- Mercury WS loop --------------------------------------------

    def _make_ws(self, url: str, *, headers: dict) -> _WebSocketClient:
        """Test seam."""
        return _WebSocketClient(url, headers=headers)

    def _handle_envelope(
        self,
        envelope: dict,
        emit: Callable[[dict], None],
    ) -> None:
        """Parse one Mercury envelope. Mercury wraps the activity
        under ``data.activity`` (see webex.rs:352-377). We extract
        that, run the parse path, optionally fetch the full message
        via REST, and emit."""
        if not isinstance(envelope, dict):
            return
        data = envelope.get("data")
        if not isinstance(data, dict):
            return
        activity = data.get("activity")
        if not isinstance(activity, dict):
            return
        verb = activity.get("verb")
        if verb != "post":
            # Same shape as webex.rs:357 — short-circuit non-post
            # verbs before touching the REST API.
            return

        actor = activity.get("actor") or {}
        actor_id = actor.get("id") if isinstance(actor, dict) else None
        if (
            self.bot_user_id
            and isinstance(actor_id, str)
            and actor_id == self.bot_user_id
        ):
            return

        obj = activity.get("object") or {}
        message_id = obj.get("id") if isinstance(obj, dict) else None
        if not isinstance(message_id, str) or not message_id:
            return

        target = activity.get("target") or {}
        room_id = target.get("id") if isinstance(target, dict) else None
        if not isinstance(room_id, str):
            room_id = ""
        if self.allowed_rooms and room_id not in self.allowed_rooms:
            return

        # Improvement #3: dedupe on the activity id before paying
        # for the REST follow-up.
        if not self._mark_seen(message_id):
            return

        full_msg = self._fetch_message(message_id)
        if full_msg is None:
            return

        ev = parse_webex_message(
            full_msg,
            activity,
            own_bot_id=self.bot_user_id,
            # _handle_envelope already filtered against
            # self.allowed_rooms above (line 855), so pass an empty
            # list to skip the redundant per-message check inside the
            # parser. The parser keeps the filter for direct callers
            # (tests, future code paths).
            allowed_rooms=[],
            account_id=self.account_id,
        )
        if ev is not None:
            emit(ev)

    def _run_session(
        self, ws: _WebSocketClient, emit: Callable[[dict], None],
    ) -> None:
        """Drive one Mercury session. Read frames forever; the
        outer reconnect loop catches socket drops and reconnects."""
        ws.settimeout(None)
        while True:
            if not ws.wait_readable(READ_TICK_SECS):
                continue
            try:
                text, close = ws.recv_frame()
            except (EOFError, OSError) as e:
                log.warn("webex mercury socket dropped", error=str(e))
                return
            if close is not None:
                code, reason = close
                log.info("webex mercury closed",
                         code=code,
                         reason=reason.decode("utf-8", "replace"))
                return
            if text is None:
                continue
            try:
                envelope = json.loads(text)
            except (ValueError, TypeError):
                log.warn("webex: malformed envelope JSON")
                continue
            self._handle_envelope(envelope, emit)

    def _gateway_loop(self, emit: Callable[[dict], None]) -> None:
        """Outer reconnect loop. Mercury's URL is static so we
        don't re-fetch per reconnect. The Authorization header rides
        on the upgrade request."""
        backoff = INITIAL_BACKOFF_SECS
        while self.bot_user_id is None:
            try:
                bot_id, display = self._validate_bot_token()
                self.bot_user_id = bot_id
                self.bot_display_name = display
                log.info("webex authenticated",
                         bot_user_id=bot_id, display_name=display)
            except Exception as e:  # noqa: BLE001
                log.warn("webex auth failed; will retry",
                         error=str(e), delay=backoff)
                time.sleep(backoff)
                backoff = min(backoff * 2.0, MAX_BACKOFF_SECS)

        backoff = INITIAL_BACKOFF_SECS
        while True:
            try:
                ws_headers = {"Authorization": f"Bearer {self.bot_token}"}
                log.info("webex mercury connecting")
                with self._make_ws(self.ws_url, headers=ws_headers) as ws:
                    self._run_session(ws, emit)
                backoff = INITIAL_BACKOFF_SECS
            except Exception as e:  # noqa: BLE001 — transport varies
                log.warn("webex mercury error; backing off",
                         error=str(e), delay=backoff)
                time.sleep(backoff)
                backoff = min(backoff * 2.0, MAX_BACKOFF_SECS)

    # ---- public sidecar surface --------------------------------------

    async def produce(self, emit: Callable[[dict], None]) -> None:
        loop = asyncio.get_event_loop()
        await loop.run_in_executor(None, self._gateway_loop, emit)

    async def on_send(self, cmd) -> None:
        room_id = (
            cmd.channel_id
            or (cmd.user.get("platform_id") if cmd.user else "")
            or ""
        )
        if not room_id:
            log.warn("webex on_send: empty room_id, dropping")
            return

        # Improvement #1: round-trip the inbound thread_id to
        # parentId so the bot's reply threads under the originating
        # message.
        parent_id = getattr(cmd, "thread_id", None) or None

        content = cmd.content
        text = cmd.text or ""
        loop = asyncio.get_event_loop()
        if isinstance(content, dict) and "Text" in content:
            await loop.run_in_executor(
                None,
                lambda: self._post_message(
                    room_id, text, parent_id=parent_id,
                ),
            )
        elif content and not (isinstance(content, dict) and "Text" in content):
            await loop.run_in_executor(
                None,
                lambda: self._post_message(
                    room_id, "(Unsupported content type)",
                    parent_id=parent_id,
                ),
            )
        else:
            await loop.run_in_executor(
                None,
                lambda: self._post_message(
                    room_id, text, parent_id=parent_id,
                ),
            )


if __name__ == "__main__":
    run_stdio_main(WebexAdapter)
