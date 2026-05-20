#!/usr/bin/env python3
"""Zulip sidecar channel adapter for LibreFang.

Replaces the former in-process Rust ``librefang-channels::zulip``
adapter (removed in this sidecar migration; same pattern as ntfy
#5224, telegram #5241, gotify #5263, mastodon #5264, bluesky #5277,
reddit #5281, twitch #5297, rocketchat #5298, discord #5299,
nextcloud #5301, slack #5302, webex #5309).

Behaviour parity with the Rust adapter:

* **Auth probe**: ``GET /api/v1/users/me`` with HTTP Basic
  (``<bot_email>:<api_key>``) at startup. Discovers the bot's own
  ``user_id`` + ``email`` + ``full_name`` (used for self-skip).
* **Event queue register**: ``POST /api/v1/register`` with form
  params ``event_types=["message"]`` plus an optional ``narrow``
  list of ``["stream", "<name>"]`` when ``ZULIP_STREAMS`` is set.
  Returns ``queue_id`` + ``last_event_id``.
* **Long-poll events**:
  ``GET /api/v1/events?queue_id=<q>&last_event_id=<n>&dont_block=false``
  with a 70 s HTTP timeout (Zulip's long-poll holds the connection
  up to ~60 s; the extra 10 s is the same buffer the Rust adapter
  used at zulip.rs:244).
* **Queue expiry**: ``400`` with ``code == "BAD_EVENT_QUEUE_ID"``
  re-registers the queue and resumes from the new
  ``last_event_id`` (mirrors zulip.rs:262-307).
* **Stream filter**: empty ``ZULIP_STREAMS`` = subscribe to all
  configured streams. When non-empty, register with a ``narrow``
  AND filter again client-side on
  ``message.display_recipient`` (defence in depth — Zulip's
  ``narrow`` is best-effort).
* **Slash-command routing**: ``/cmd args`` → ``Command``
  (text otherwise).
* **DM vs group**: ``is_group = (message.type == "stream")``.
* **REST send**: ``POST /api/v1/messages`` with form-encoded body
  ``type``, ``to``, ``content`` and (for stream) ``topic``.
  10 000-char chunking matches the Rust adapter's
  ``MAX_MESSAGE_LEN`` (zulip.rs:21).
* **DM detection on outbound**: ``cmd.user.platform_id`` containing
  ``@`` → ``type=direct``, else ``type=stream`` (same heuristic the
  Rust adapter used at zulip.rs:458).
* **Account ID**: optional ``ZULIP_ACCOUNT_ID`` is injected as
  ``account_id`` in inbound message metadata so the bridge's
  multi-bot routing can pin per-realm.
* **Reconnect / backoff**: exponential 1 s → 60 s on any error
  (mirrors zulip.rs:228-313).

Improvements over the Rust adapter
==================================

1. **Outbound topic round-trip via ``thread_id``**. The Rust
   ``send`` (``crates/librefang-channels/src/zulip.rs`` line 463
   on the migrating tree) hard-coded ``topic = "LibreFang"`` for
   every stream reply — losing the inbound topic context, so the
   bot's response landed in a "LibreFang" topic regardless of which
   topic triggered it. (A separate ``send_in_thread`` path at
   line 471 did pass ``thread_id`` through, but the kernel only
   reaches that path when the trigger explicitly carried a thread
   id; the common case dropped the topic.) The sidecar surfaces
   the inbound ``message.subject`` as ``thread_id`` on inbound and
   ``on_send`` routes EVERY stream send through that topic so the
   reply lands in the originating topic. Mirrors reddit /
   rocketchat / nextcloud / webex.

2. **429 ``Retry-After`` honoured on register, events, and send**.
   Zulip documents 429 with ``Retry-After``. The Rust adapter had
   no 429 handling (see the generic exponential-backoff ladder at
   zulip.rs:228-313 — same 1 s → 60 s sleep regardless of
   ``Retry-After``); a server-side rate-limit either burned the
   poll budget or caused the send to return an Err. The sidecar
   parses ``Retry-After`` (with a ``RETRY_AFTER_DEFAULT_SECS =
   30.0`` fallback, floor 1 s, cap ``MAX_BACKOFF_SECS``), sleeps,
   and retries once. Same pattern as
   ``fix(channels): honour Retry-After across sidecar polling
   adapters`` #5303.

3. **Bounded ``message.id`` dedupe**. Zulip's ``last_event_id``
   cursor narrows the *event* range server-side, but on queue
   re-register (``BAD_EVENT_QUEUE_ID``) the bot can re-see a
   message it already emitted because the new queue starts fresh.
   The Rust adapter had no dedupe — its emit at zulip.rs:434 was
   unconditional. The sidecar dedupes locally on
   ``message.id`` with a bounded ``SEEN_MESSAGES_MAX = 10 000`` /
   ``SEEN_MESSAGES_EVICT = 5 000`` cap (same policy as reddit /
   rocketchat / nextcloud / webex).

4. **Self-skip by stable ``sender_id``**. The Rust adapter
   compared ``sender_email == bot_email`` (zulip.rs:357). Email
   is the bot's outward identifier and rarely rotates, but on
   realms that change bot ownership the email moves while the
   integer ``user_id`` stays — the email-only check breaks. The
   sidecar prefers ``sender_id == own_user_id`` (the integer
   ``/users/me`` returns) and falls back to ``sender_email ==
   own_email`` when ``sender_id`` is absent (parallels the
   rocketchat #5298 / nextcloud #5301 fix).

Stdlib-only: HTTPS via ``urllib.request``, polling on a worker
thread (no asyncio HTTP).

Configure via ``[[sidecar_channels]]``::

    [[sidecar_channels]]
    name = "zulip"
    command = "python3"
    args = ["-m", "librefang.sidecar.adapters.zulip"]
    channel_type = "zulip"
    [sidecar_channels.env]
    ZULIP_SERVER_URL = "https://myorg.zulipchat.com"
    ZULIP_BOT_EMAIL  = "bot-bot@myorg.zulipchat.com"
    # ZULIP_STREAMS = "engineering, general"      # optional
    # ZULIP_ACCOUNT_ID = "prod"                   # optional, multi-bot routing key

Secret via ``~/.librefang/secrets.env``: ``ZULIP_API_KEY`` (the
bot's API key from the Zulip "Bot settings" page).
"""
from __future__ import annotations

import asyncio
import base64
import json
import os
import threading
import time
import urllib.error
import urllib.parse
import urllib.request
from typing import Any, Optional

from librefang.sidecar import Content, Field, Schema, SidecarAdapter, protocol, run_stdio_main
from librefang.sidecar import logging as log
from librefang.sidecar.common import (
    SeenSet as _SeenSet,
    http_request as _http_request,
    parse_retry_after as _parse_retry_after_impl,
    split_csv as _split_csv,
    split_message as _split_message,
)

# Zulip's official message-text ceiling. Mirrors the Rust adapter's
# ``MAX_MESSAGE_LEN`` (crates/librefang-channels/src/zulip.rs:21).
ZULIP_MSG_LIMIT = 10_000

# Zulip long-poll holds the connection up to ~60 s server-side. The
# Rust adapter used POLL_TIMEOUT + 10 s (zulip.rs:244); we mirror.
POLL_TIMEOUT_SECS = 60
LONG_POLL_HTTP_TIMEOUT_SECS = POLL_TIMEOUT_SECS + 10
SEND_TIMEOUT_SECS = 15.0

INITIAL_BACKOFF_SECS = 1.0
MAX_BACKOFF_SECS = 60.0

# Default fallback when Zulip 429s without a parseable Retry-After.
# 30 s is conservative enough not to re-trigger throttling. Mirrors
# the rocketchat / nextcloud / webex sidecars.
RETRY_AFTER_DEFAULT_SECS = 30.0

# Bounded dedupe cap on Zulip ``message.id``. Same policy as
# reddit / rocketchat / nextcloud / webex.
SEEN_MESSAGES_MAX = 10_000
SEEN_MESSAGES_EVICT = 5_000

# Fallback topic when the kernel asks us to send to a stream but no
# ``thread_id`` was carried through (initial outbound from an agent,
# no inbound context). Mirrors the Rust adapter's hard-coded
# ``"LibreFang"`` at zulip.rs:463 for the same edge case.
DEFAULT_STREAM_TOPIC = "LibreFang"
def _parse_retry_after(resp_hdrs: dict, *, default_secs: float) -> float:
    """Zulip's 429 response carries ``Retry-After`` in seconds form.
    Falls back to ``default_secs`` when missing or garbled. Floor
    1 s, capped at ``MAX_BACKOFF_SECS`` so a server bug can't pin
    the loop for hours."""
    raw = resp_hdrs.get("retry-after")
    if not raw:
        return default_secs
    try:
        v = float(raw)
    except (TypeError, ValueError):
        return default_secs
    return min(max(v, 1.0), MAX_BACKOFF_SECS)


def parse_zulip_event(
    event: dict,
    *,
    own_user_id: Optional[int],
    own_email: str,
    allowed_streams: list[str],
    account_id: Optional[str],
) -> Optional[dict]:
    """Pure-function port of the inbound parse path in
    ``crates/librefang-channels/src/zulip.rs`` lines 330-437.

    ``event`` is one element of the ``events`` array returned by
    ``/api/v1/events``. Returns the ``message`` event dict ready to
    ``emit``, or ``None`` when the payload should be skipped (wrong
    event type, system message, filtered stream, self, empty body,
    malformed).

    Improvement over the Rust adapter (see module header for the
    full list): self-skip prefers ``sender_id == own_user_id`` and
    falls back to ``sender_email == own_email`` only when
    ``sender_id`` is absent.
    """
    if not isinstance(event, dict):
        return None
    if event.get("type") != "message":
        return None
    message = event.get("message")
    if not isinstance(message, dict):
        return None

    msg_type = message.get("type")
    if not isinstance(msg_type, str):
        return None

    stream_name = message.get("display_recipient")
    if not isinstance(stream_name, str):
        stream_name = ""

    # Defence-in-depth stream filter — Zulip's `narrow` is best-effort
    # so check again client-side. The Rust adapter did the same at
    # zulip.rs:347-353.
    if msg_type == "stream" and allowed_streams and stream_name not in allowed_streams:
        return None

    # Improvement #4: self-skip on stable user id first.
    sender_id_raw = message.get("sender_id")
    sender_id: Optional[int] = None
    if isinstance(sender_id_raw, int):
        sender_id = sender_id_raw
    elif isinstance(sender_id_raw, str) and sender_id_raw.isdigit():
        sender_id = int(sender_id_raw)
    sender_email = message.get("sender_email")
    if not isinstance(sender_email, str):
        sender_email = ""

    if own_user_id is not None and sender_id is not None:
        if sender_id == own_user_id:
            return None
    elif own_email and sender_email and sender_email == own_email:
        # Fallback when /users/me didn't surface a sender_id (defensive).
        return None

    content_text = message.get("content")
    if not isinstance(content_text, str) or not content_text:
        return None

    sender_name = message.get("sender_full_name")
    if not isinstance(sender_name, str) or not sender_name:
        sender_name = "unknown"

    raw_msg_id = message.get("id")
    if isinstance(raw_msg_id, int):
        msg_id: Optional[str] = str(raw_msg_id)
    elif isinstance(raw_msg_id, str) and raw_msg_id:
        msg_id = raw_msg_id
    else:
        msg_id = None

    topic_raw = message.get("subject")
    topic = topic_raw if isinstance(topic_raw, str) else ""

    is_group = msg_type == "stream"

    # platform_id matches the Rust adapter's choice at zulip.rs:380-384:
    # stream name for stream messages, sender email for DMs.
    if is_group:
        platform_id = stream_name
    else:
        platform_id = sender_email

    # Slash-command routing matches zulip.rs:386-399.
    if content_text.startswith("/"):
        head, _, tail = content_text[1:].partition(" ")
        content = Content.command(head, tail.split() if tail else [])
    else:
        content = Content.text(content_text)

    # Improvement #1: surface the topic as thread_id so `on_send` can
    # round-trip it. The Rust adapter set thread_id = Some(topic) at
    # zulip.rs:413 too, but its send() path ignored the topic for the
    # common case (line 463 hard-coded "LibreFang").
    thread_id = topic if (is_group and topic) else None

    metadata: dict[str, Any] = {
        "sender_id": str(sender_id) if sender_id is not None else "",
        "sender_email": sender_email,
    }
    if account_id is not None:
        metadata["account_id"] = account_id

    return protocol.message(
        # platform_id is the room (stream name) or sender email —
        # matches the Rust adapter's `sender.platform_id` at
        # zulip.rs:404-405.
        user_id=platform_id,
        user_name=sender_name,
        content=content,
        message_id=msg_id,
        is_group=is_group,
        thread_id=thread_id,
        metadata=metadata,
    )


# ---------------------------------------------------------------------------
# Zulip adapter
# ---------------------------------------------------------------------------


class ZulipAdapter(SidecarAdapter):
    capabilities: list = ["thread"]
    # Zulip stream messages reach every subscriber of the stream;
    # private messages only the addressed users. Matches the
    # chat-room precedent set by twitch / discord / slack / webex —
    # surface errors rather than swallow them. A pure public-broadcast
    # surface (mastodon / bluesky / reddit / nextcloud) sets this True;
    # Zulip's mixed model keeps the chat-room default.
    suppress_error_responses: bool = False

    SCHEMA = Schema(
        name="zulip",
        display_name="Zulip",
        description="Zulip REST + event-queue long-poll adapter (out-of-process sidecar)",
        fields=[
            Field("ZULIP_SERVER_URL", "Server URL", "text",
                  required=True,
                  placeholder="https://myorg.zulipchat.com"),
            Field("ZULIP_BOT_EMAIL", "Bot Email", "text",
                  required=True,
                  placeholder="bot-bot@myorg.zulipchat.com"),
            Field("ZULIP_API_KEY", "Bot API Key", "secret",
                  required=True,
                  placeholder="abcd1234..."),
            Field("ZULIP_STREAMS",
                  "Stream names (comma-separated, empty = all subscribed)",
                  "text",
                  placeholder="engineering, general",
                  advanced=True),
            Field("ZULIP_ACCOUNT_ID",
                  "Account ID (multi-bot routing)",
                  "text",
                  placeholder="prod",
                  advanced=True),
        ],
    )

    def __init__(self) -> None:
        server = os.environ.get("ZULIP_SERVER_URL", "").strip()
        # Strip trailing slashes the same way the Rust adapter did
        # (zulip.rs:60: `trim_end_matches('/')`).
        self.server_url = server.rstrip("/")
        self.bot_email = os.environ.get("ZULIP_BOT_EMAIL", "").strip()
        self.api_key = os.environ.get("ZULIP_API_KEY", "").strip()
        self.allowed_streams = _split_csv(
            os.environ.get("ZULIP_STREAMS", "")
        )
        acct = os.environ.get("ZULIP_ACCOUNT_ID", "").strip()
        self.account_id = acct or None

        missing: list[str] = []
        if not self.server_url:
            missing.append("ZULIP_SERVER_URL")
        if not self.bot_email:
            missing.append("ZULIP_BOT_EMAIL")
        if not self.api_key:
            missing.append("ZULIP_API_KEY")
        if missing:
            log.error("zulip required env vars missing", missing=missing)
            raise SystemExit(2)
        if not (self.server_url.startswith("http://")
                or self.server_url.startswith("https://")):
            log.error(
                "ZULIP_SERVER_URL must start with http:// or https://",
                server_url=self.server_url,
            )
            raise SystemExit(2)

        # Discovered at startup via GET /users/me. Used for self-skip
        # in parse_zulip_event. Both surfaced; user_id is the stable
        # comparison key, email is the fallback when sender_id is
        # absent.
        self.own_user_id: Optional[int] = None
        self.own_full_name: str = ""

        # Improvement #3: bounded dedupe on Zulip message.id.
        self._seen = _SeenSet(max_size=SEEN_MESSAGES_MAX, evict=SEEN_MESSAGES_EVICT)

    # ---- HTTP helpers ------------------------------------------------

    def _auth_headers(
        self,
        *,
        form: bool = False,
    ) -> dict:
        """HTTP Basic with ``<bot_email>:<api_key>``. Mirrors
        reqwest's ``basic_auth`` shape at zulip.rs:100 / 130 / 168."""
        creds = f"{self.bot_email}:{self.api_key}".encode("utf-8")
        h = {
            "Authorization": "Basic " + base64.b64encode(creds).decode("ascii"),
            "User-Agent": "librefang-zulip-sidecar/1 (https://librefang.org)",
        }
        if form:
            h["Content-Type"] = "application/x-www-form-urlencoded"
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

    def _mark_seen(self, msg_id: Optional[str]) -> bool:
        """Return True iff freshly seen. Shim around :class:`librefang.sidecar.common.SeenSet`."""
        return self._seen.mark(msg_id)

    # ---- REST: validate, register, send -----------------------------

    def _validate(self) -> tuple[int, str]:
        """``GET /api/v1/users/me`` → ``(user_id, full_name)``. The
        bot's own ``user_id`` becomes the self-skip key (see
        improvement #4 in the module header). Raises ``RuntimeError``
        on any non-200 so the producer loop backs off."""
        url = f"{self.server_url}/api/v1/users/me"
        status, body, raw, resp_hdrs = self._http(
            url, headers=self._auth_headers(),
        )
        if status == 429:
            wait = _parse_retry_after(
                resp_hdrs, default_secs=RETRY_AFTER_DEFAULT_SECS,
            )
            log.warn(
                "zulip /users/me 429; sleeping then retrying",
                retry_after_secs=wait,
            )
            time.sleep(wait)
            status, body, raw, resp_hdrs = self._http(
                url, headers=self._auth_headers(),
            )
        if status != 200 or not isinstance(body, dict):
            snippet = raw[:200].decode("utf-8", "replace") if raw else ""
            raise RuntimeError(
                f"zulip /users/me failed {status}: {snippet}"
            )
        uid_raw = body.get("user_id")
        if not isinstance(uid_raw, int):
            raise RuntimeError(
                "zulip /users/me: missing user_id in response"
            )
        full_name = body.get("full_name")
        if not isinstance(full_name, str) or not full_name:
            full_name = "unknown"
        self.own_user_id = uid_raw
        self.own_full_name = full_name
        return uid_raw, full_name

    def _register_queue(self) -> tuple[str, int]:
        """``POST /api/v1/register`` with ``event_types=["message"]``
        and optional ``narrow``. Returns ``(queue_id, last_event_id)``.
        Raises on any non-2xx after honouring a single ``Retry-After``
        on 429."""
        url = f"{self.server_url}/api/v1/register"
        params: list[tuple[str, str]] = [
            ("event_types", json.dumps(["message"])),
        ]
        if self.allowed_streams:
            narrow = [["stream", s] for s in self.allowed_streams]
            params.append(("narrow", json.dumps(narrow)))
        body = urllib.parse.urlencode(params).encode("utf-8")
        status, resp, raw, resp_hdrs = self._http(
            url,
            method="POST",
            body=body,
            headers=self._auth_headers(form=True),
        )
        if status == 429:
            wait = _parse_retry_after(
                resp_hdrs, default_secs=RETRY_AFTER_DEFAULT_SECS,
            )
            log.warn(
                "zulip /register 429; sleeping then retrying",
                retry_after_secs=wait,
            )
            time.sleep(wait)
            status, resp, raw, resp_hdrs = self._http(
                url,
                method="POST",
                body=body,
                headers=self._auth_headers(form=True),
            )
        if status >= 300 or not isinstance(resp, dict):
            snippet = raw[:200].decode("utf-8", "replace") if raw else ""
            raise RuntimeError(
                f"zulip /register failed {status}: {snippet}"
            )
        qid = resp.get("queue_id")
        last_event_id = resp.get("last_event_id")
        if not isinstance(qid, str) or not qid:
            raise RuntimeError("zulip /register: missing queue_id")
        if not isinstance(last_event_id, int):
            raise RuntimeError("zulip /register: missing last_event_id")
        return qid, last_event_id

    # ---- inbound: long-poll loop ------------------------------------

    def _poll_once(
        self,
        emit,
        queue_id: str,
        last_event_id: int,
    ) -> tuple[int, Optional[str]]:
        """One ``/events`` long-poll fetch. Emits any new message
        events and returns ``(new_last_event_id, reregister_signal)``.

        ``reregister_signal``:
        * ``None`` — normal completion (may be no-op if no events)
        * ``"reregister"`` — server said ``BAD_EVENT_QUEUE_ID``;
          producer must re-register before next call
        On 429: sleeps ``Retry-After`` then raises so the producer
        loop's outer backoff applies.
        On any other non-2xx: raises (same back-off path)."""
        url = (
            f"{self.server_url}/api/v1/events"
            f"?queue_id={urllib.parse.quote(queue_id, safe='')}"
            f"&last_event_id={last_event_id}"
            f"&dont_block=false"
        )
        status, body, raw, resp_hdrs = self._http(
            url,
            headers=self._auth_headers(),
            timeout=LONG_POLL_HTTP_TIMEOUT_SECS,
        )
        if status == 429:
            wait = _parse_retry_after(
                resp_hdrs, default_secs=RETRY_AFTER_DEFAULT_SECS,
            )
            log.warn(
                "zulip /events 429; sleeping then backing off",
                retry_after_secs=wait,
            )
            time.sleep(wait)
            raise RuntimeError("zulip 429 — rate-limited")
        # Queue expiry — Zulip docs this as 400 + code BAD_EVENT_QUEUE_ID.
        # Mirrors zulip.rs:262-308. Signal the caller to re-register
        # rather than raise; the new queue should start a fresh
        # last_event_id.
        if status == 400 and isinstance(body, dict):
            if body.get("code") == "BAD_EVENT_QUEUE_ID":
                log.info("zulip: event queue expired; will re-register")
                return last_event_id, "reregister"
        if status >= 300:
            snippet = raw[:200].decode("utf-8", "replace") if raw else ""
            raise RuntimeError(
                f"zulip /events failed {status}: {snippet}"
            )
        if not isinstance(body, dict):
            return last_event_id, None
        events = body.get("events")
        if not isinstance(events, list):
            return last_event_id, None

        new_last_id = last_event_id
        for event in events:
            if not isinstance(event, dict):
                continue
            eid = event.get("id")
            if isinstance(eid, int) and eid > new_last_id:
                new_last_id = eid

            ev = parse_zulip_event(
                event,
                own_user_id=self.own_user_id,
                own_email=self.bot_email,
                allowed_streams=self.allowed_streams,
                account_id=self.account_id,
            )
            if ev is None:
                continue

            msg_id = ev["params"].get("message_id")
            if not self._mark_seen(msg_id):
                # Already emitted — likely a boundary repeat after a
                # queue re-register. Drop silently.
                continue

            emit(ev)

        return new_last_id, None

    def _producer_blocking(self, emit) -> None:
        """Verify credentials, register an event queue, long-poll
        forever. Mirrors the recipe in zulip.rs:206-441 but with the
        429 + dedupe improvements layered in."""
        verify_backoff = INITIAL_BACKOFF_SECS
        while True:
            try:
                uid, full_name = self._validate()
                log.info(
                    "zulip authenticated",
                    user_id=uid, full_name=full_name,
                )
                break
            except Exception as e:  # noqa: BLE001
                log.warn(
                    "zulip auth failed; will retry",
                    error=str(e), delay=verify_backoff,
                )
                time.sleep(verify_backoff)
                verify_backoff = min(verify_backoff * 2, MAX_BACKOFF_SECS)

        # Register initial event queue (also with backoff on failure).
        register_backoff = INITIAL_BACKOFF_SECS
        while True:
            try:
                queue_id, last_event_id = self._register_queue()
                log.info(
                    "zulip event queue registered",
                    queue_id=queue_id, last_event_id=last_event_id,
                )
                break
            except Exception as e:  # noqa: BLE001
                log.warn(
                    "zulip /register failed; will retry",
                    error=str(e), delay=register_backoff,
                )
                time.sleep(register_backoff)
                register_backoff = min(
                    register_backoff * 2, MAX_BACKOFF_SECS,
                )

        backoff = INITIAL_BACKOFF_SECS
        while True:
            try:
                last_event_id, signal = self._poll_once(
                    emit, queue_id, last_event_id,
                )
                backoff = INITIAL_BACKOFF_SECS
                if signal == "reregister":
                    # Re-register inline so the next poll uses the new
                    # queue id. If re-register itself fails the outer
                    # loop's backoff kicks in.
                    queue_id, last_event_id = self._register_queue()
                    log.info(
                        "zulip event queue re-registered",
                        queue_id=queue_id,
                        last_event_id=last_event_id,
                    )
            except Exception as e:  # noqa: BLE001
                log.warn(
                    "zulip poll error; backing off",
                    error=str(e), delay=backoff,
                )
                time.sleep(backoff)
                backoff = min(backoff * 2, MAX_BACKOFF_SECS)

    async def produce(self, emit) -> None:
        loop = asyncio.get_event_loop()
        await loop.run_in_executor(None, self._producer_blocking, emit)

    # ---- outbound: /messages POST ------------------------------------

    def _post_message(
        self,
        *,
        msg_type: str,  # "stream" or "direct"
        to: str,
        topic: str,
        text: str,
    ) -> None:
        """POST /api/v1/messages. Long bodies are chunked at
        ``ZULIP_MSG_LIMIT`` and each chunk is sent as a separate
        message (matches the Rust adapter's per-chunk loop at
        zulip.rs:152-178). On 429: sleep Retry-After and retry once;
        a second 429 raises (with the body suppressed by
        ``suppress_error_responses=False`` the kernel logs but
        doesn't echo)."""
        if not to:
            raise RuntimeError(
                "zulip on_send: missing destination "
                "(cmd.user.platform_id was empty)"
            )
        url = f"{self.server_url}/api/v1/messages"
        for chunk in _split_message(text, ZULIP_MSG_LIMIT):
            params: list[tuple[str, str]] = [
                ("type", msg_type),
                ("to", to),
                ("content", chunk),
            ]
            if msg_type == "stream":
                params.append(("topic", topic))
            body = urllib.parse.urlencode(params).encode("utf-8")
            status, _resp, raw, resp_hdrs = self._http(
                url,
                method="POST",
                body=body,
                headers=self._auth_headers(form=True),
            )
            if status == 429:
                wait = _parse_retry_after(
                    resp_hdrs, default_secs=RETRY_AFTER_DEFAULT_SECS,
                )
                log.warn(
                    "zulip /messages 429; sleeping then retrying once",
                    retry_after_secs=wait,
                )
                time.sleep(wait)
                status, _resp, raw, resp_hdrs = self._http(
                    url,
                    method="POST",
                    body=body,
                    headers=self._auth_headers(form=True),
                )
            if status >= 300:
                snippet = raw[:200].decode("utf-8", "replace") if raw else ""
                raise RuntimeError(
                    f"zulip /messages POST {status}: {snippet}"
                )

    async def on_send(self, cmd) -> None:
        # Text-only; structured content falls back to a placeholder so
        # the operator still sees something rather than a silent
        # failure (matches the Rust adapter's `"(Unsupported content
        # type)"` at zulip.rs:453).
        if cmd.content and not (
            isinstance(cmd.content, dict) and "Text" in cmd.content
        ):
            text = "(Unsupported content type)"
        else:
            text = cmd.text or ""

        # cmd.user.platform_id is the destination — stream name for
        # stream messages, sender email for DMs. cmd.channel_id is
        # the same value on the wire; fall back to it if user was
        # stripped.
        user = getattr(cmd, "user", None) or {}
        dest = (
            str(user.get("platform_id") or "")
            if isinstance(user, dict)
            else ""
        )
        if not dest:
            dest = str(getattr(cmd, "channel_id", "") or "")

        # DM detection on outbound mirrors zulip.rs:458: an "@" in the
        # platform id ⇒ direct message; otherwise stream.
        is_direct = "@" in dest

        # Improvement #1: use cmd.thread_id as the topic for stream
        # messages so the bot's reply lands in the originating topic.
        # The Rust send() hard-coded "LibreFang" (zulip.rs:463),
        # losing topic context for every initial reply.
        thread_id_raw = getattr(cmd, "thread_id", None)
        topic = (
            str(thread_id_raw)
            if isinstance(thread_id_raw, str) and thread_id_raw
            else DEFAULT_STREAM_TOPIC
        )

        await asyncio.get_event_loop().run_in_executor(
            None,
            lambda: self._post_message(
                msg_type="direct" if is_direct else "stream",
                to=dest,
                topic=topic,
                text=text,
            ),
        )


if __name__ == "__main__":
    run_stdio_main(ZulipAdapter)
