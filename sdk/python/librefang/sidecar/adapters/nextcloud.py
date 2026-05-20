#!/usr/bin/env python3
"""Nextcloud Talk sidecar channel adapter for LibreFang.

Replaces the former in-process Rust ``librefang-channels::nextcloud``
adapter (removed in this sidecar migration; same pattern as ntfy
#5224, telegram #5241, gotify #5263, mastodon #5264, bluesky #5277,
reddit #5281, twitch #5297, rocketchat #5298).

Behaviour parity is preserved (REST polling of the Talk
``chat/<token>?lookIntoFuture=1`` endpoint with auth via app-password
Bearer token plus the mandatory ``OCS-APIRequest: true`` header,
slash-command routing, multi-bot ``account_id`` injection,
32000-char chunking), with three intentional improvements explicitly
ack'd by the maintainer:

* **Outbound threading via ``replyTo``** (improvement / bugfix over
  Rust parity). The Rust adapter (``crates/librefang-channels/src/nextcloud.rs``
  ``api_send_message`` lines 130-160 in main) called the chat POST
  with a body of just ``{"message": ...}`` — Talk's ``replyTo``
  parameter that links a reply to its parent message was never sent.
  Chunked replies and threaded responses always landed at the room
  root with no link back to the originating message. The sidecar
  surfaces the inbound ``id`` (or inbound ``parentMessage.id`` when
  the user themselves was already replying inside a thread, so the
  bot threads alongside rather than starting a child) as
  ``thread_id``, and ``on_send`` posts ``replyTo`` populated so the
  reply threads correctly. Mirrors reddit / bluesky / mastodon /
  rocketchat.
* **Self-skip by stable ``actorId`` (always)**. The Rust adapter
  already compared ``msg["actorId"] == own_user`` (nextcloud.rs:338),
  where ``own_user`` was discovered via ``GET /ocs/v2.php/cloud/user``
  — so the existing field is the right one. The sidecar keeps the
  same stable-id comparison, but additionally fans out to handle
  Talk's ``actorType=guests`` / ``federated_users`` shapes where the
  ``actorId`` field is still present but only meaningful when
  qualified with the type; the sidecar matches on
  ``(actorType, actorId) == ("users", own_user)`` so a guest with a
  display string that happens to equal the bot's own user id can't
  spoof self-skip.
* **Dedupe set on top of ``lastKnownMessageId`` watermark**
  (improvement over Rust parity). The Rust adapter advanced
  ``last_known_ids`` with the newest id per room (nextcloud.rs:347-354)
  but the watermark only narrows the server-side query — under
  retry / re-poll boundaries the server can still resend the same
  id (e.g. when the previous fetch's response was lost but the
  newest-id update wasn't persisted). The sidecar keeps the
  watermark for the API query but additionally dedupes locally on
  message ``id`` with bounded eviction, matching reddit / rocketchat.

Inbound: per-room polling of
``GET /ocs/v2.php/apps/spreed/api/v1/chat/<token>?lookIntoFuture=1``
at the configured interval (default 3 s, matching the Rust adapter,
floor 1 s). On startup, ``GET /ocs/v2.php/cloud/user?format=json``
validates the credentials and discovers the bot's own user id
(used as the self-skip key). Empty ``NEXTCLOUD_ROOMS`` discovers
joined rooms via
``GET /ocs/v2.php/apps/spreed/api/v4/room?format=json``.
Slash-command bodies (``/cmd args``) become ``Content.command``;
everything else is plain ``Content.text``. Metadata carries
``actor_id``, ``actor_type``, ``actor_display_name``,
``room_token``, ``reference_id``, ``parent_message_id`` (when
inbound was inside a thread), and ``account_id`` when
``NEXTCLOUD_ACCOUNT_ID`` is set.

Outbound: ``POST /ocs/v2.php/apps/spreed/api/v1/chat/<token>`` with
form-encoded body (``message`` and optional ``replyTo``). Long
bodies are chunked at 32000 chars (matching the Rust
``MAX_MESSAGE_LEN``). When ``cmd.thread_id`` is set, every chunk
carries the same ``replyTo`` so the multi-part reply lives in the
same thread.

Stdlib-only (the SDK has zero runtime deps): HTTP via
``urllib.request``, polling on a worker thread.

Configure via ``[[sidecar_channels]]``:

    [[sidecar_channels]]
    name = "nextcloud"
    command = "python3"
    args = ["-m", "librefang.sidecar.adapters.nextcloud"]
    channel_type = "nextcloud"
    [sidecar_channels.env]
    NEXTCLOUD_SERVER_URL = "https://cloud.example.com"
    # NEXTCLOUD_ROOMS = "abc123,def456"           # optional; empty = all joined
    # NEXTCLOUD_ACCOUNT_ID = "prod"               # optional, multi-bot routing key
    # NEXTCLOUD_POLL_INTERVAL_SECS = "3"          # optional, default 3, floor 1

The Nextcloud app password / OAuth bearer belongs in
``~/.librefang/secrets.env`` as ``NEXTCLOUD_TOKEN`` (the dashboard's
Channels page writes it there when you fill the Nextcloud form).
"""
from __future__ import annotations

import asyncio
import json
import os
import time
import urllib.error
import urllib.parse
import urllib.request
from typing import Any

from librefang.sidecar import Content, Field, Schema, SidecarAdapter, protocol, run_stdio_main
from librefang.sidecar import logging as log
from librefang.sidecar.common import (
    SeenSet as _SeenSet,
    http_request as _http_request,
    split_message as _split_message,
)

# Matches the Rust adapter's MAX_MESSAGE_LEN (nextcloud.rs:23).
MAX_MESSAGE_LEN = 32000
# Default poll interval matches the Rust adapter's POLL_INTERVAL_SECS
# (nextcloud.rs:26).
DEFAULT_POLL_INTERVAL_SECS = 3
MIN_POLL_INTERVAL_SECS = 1
SEND_TIMEOUT_SECS = 30
MAX_BACKOFF_SECS = 60.0
# Default fallback when Talk 429s without a `Retry-After` header. The
# OCS bruteforce throttler typically sends one (delay-in-seconds form),
# but the protocol does not require it, so we need a sane fallback.
# 30 s matches Talk's documented default throttle window.
RETRY_AFTER_DEFAULT_SECS = 30.0
# How many messages per `chat/<token>?lookIntoFuture=1` poll. Matches
# the Rust adapter's `limit=100` query param.
CHAT_FETCH_LIMIT = 100
# Bounded dedupe set: capped + oldest half evicted on overflow, same
# policy as reddit / rocketchat. The lastKnownMessageId watermark
# already excludes older ids server-side; this only needs to hold
# enough to cover boundary repeats and overlapping fetches between
# polls.
SEEN_MESSAGES_MAX = 10000
SEEN_MESSAGES_EVICT = 5000

def _parse_rooms(raw: str) -> list[str]:
    """Comma-separated room-token list. Strips whitespace and empty
    entries; preserves order. Empty input → empty list (means 'discover
    joined rooms at startup')."""
    return [s.strip() for s in raw.split(",") if s.strip()]


class NextcloudAdapter(SidecarAdapter):
    # Nextcloud Talk conversations can be public to many participants —
    # mirroring mastodon / bluesky / reddit / rocketchat we suppress the
    # error-echo path so an internal exception doesn't surface as a chat
    # message to every member of the room.
    suppress_error_responses = True
    # No typing / reaction / interactive / streaming concept on the
    # Talk REST adapter (Talk's typing indicator is a separate WebSocket
    # /signaling endpoint and not part of the OCS REST surface; mirrors
    # the Rust adapter's no-op `send_typing`).
    capabilities: list = []

    SCHEMA = Schema(
        name="nextcloud",
        display_name="Nextcloud Talk",
        description="Nextcloud Talk OCS REST adapter (out-of-process sidecar)",
        fields=[
            Field("NEXTCLOUD_SERVER_URL", "Server URL", "text",
                  required=True,
                  placeholder="https://cloud.example.com"),
            Field("NEXTCLOUD_TOKEN", "App Password / OAuth Token",
                  "secret", required=True,
                  placeholder="abc123..."),
            Field("NEXTCLOUD_ROOMS",
                  "Room Tokens (comma-separated; empty = all joined)",
                  "text",
                  placeholder="abc123,def456"),
            Field("NEXTCLOUD_ACCOUNT_ID",
                  "Account ID (multi-bot routing)", "text",
                  placeholder="prod", advanced=True),
            Field("NEXTCLOUD_POLL_INTERVAL_SECS",
                  f"Poll interval seconds (default {DEFAULT_POLL_INTERVAL_SECS}, "
                  f"floor {MIN_POLL_INTERVAL_SECS})",
                  "text",
                  placeholder=str(DEFAULT_POLL_INTERVAL_SECS),
                  advanced=True),
        ],
    )

    def __init__(self) -> None:
        server = os.environ.get("NEXTCLOUD_SERVER_URL", "").strip()
        # Strip trailing slashes the same way the Rust adapter did
        # (`trim_end_matches('/')`) so callers get the same URL shape
        # whether they configured `https://cloud.example.com` or
        # `https://cloud.example.com/`.
        self.server_url = server.rstrip("/")
        self.token = os.environ.get("NEXTCLOUD_TOKEN", "").strip()
        rooms_raw = os.environ.get("NEXTCLOUD_ROOMS", "").strip()
        self.allowed_rooms = _parse_rooms(rooms_raw)
        acct = os.environ.get("NEXTCLOUD_ACCOUNT_ID", "").strip()
        self.account_id = acct or None

        interval_raw = os.environ.get(
            "NEXTCLOUD_POLL_INTERVAL_SECS", "",
        ).strip()
        try:
            interval = (
                int(interval_raw) if interval_raw
                else DEFAULT_POLL_INTERVAL_SECS
            )
        except (TypeError, ValueError):
            log.error(
                "NEXTCLOUD_POLL_INTERVAL_SECS invalid (must be integer)",
                value=interval_raw,
            )
            raise SystemExit(2) from None
        if interval < MIN_POLL_INTERVAL_SECS:
            log.warn(
                "NEXTCLOUD_POLL_INTERVAL_SECS below floor; clamping",
                requested=interval, floor=MIN_POLL_INTERVAL_SECS,
            )
            interval = MIN_POLL_INTERVAL_SECS
        self.poll_interval = interval

        missing: list[str] = []
        if not self.server_url:
            missing.append("NEXTCLOUD_SERVER_URL")
        if not self.token:
            missing.append("NEXTCLOUD_TOKEN")
        if missing:
            log.error("nextcloud required env vars missing", missing=missing)
            raise SystemExit(2)
        if not (self.server_url.startswith("http://")
                or self.server_url.startswith("https://")):
            log.error(
                "NEXTCLOUD_SERVER_URL must start with http:// or https://",
                server_url=self.server_url,
            )
            raise SystemExit(2)

        # Discovered at startup via `_verify_credentials()` — the stable
        # user id Talk uses in `actorId` for messages the bot itself
        # sends. The Rust adapter discovered the same value via
        # `GET /cloud/user` and used it for self-skip.
        self.own_user_id: str = ""

        # Per-room watermark — the `lastKnownMessageId` cursor Talk's
        # `chat/<token>?lookIntoFuture=1` endpoint accepts. Initialised
        # to 0 the first time we see a room; bumped to the newest seen
        # `id` after each successful poll.
        self._room_watermarks: dict[str, int] = {}
        # Dedupe set on Talk message `id` (cheap, bounded). Same
        # cap / eviction policy as reddit / rocketchat.
        self._seen = _SeenSet(
            max_size=SEEN_MESSAGES_MAX, evict=SEEN_MESSAGES_EVICT,
        )

    # ---- HTTP helpers ------------------------------------------------

    def _auth_headers(self, extra: dict | None = None) -> dict:
        """Bearer + the mandatory OCS API marker. Matches the Rust
        adapter's `ocs_headers` (nextcloud.rs:79-84) — without
        `OCS-APIRequest: true` Talk returns 401 even for a valid
        token."""
        h = {
            "Authorization": f"Bearer {self.token}",
            "OCS-APIRequest": "true",
            "Accept": "application/json",
        }
        if extra:
            h.update(extra)
        return h

    def _http(
        self,
        url: str,
        *,
        method: str = "GET",
        body: bytes | None = None,
        headers: dict | None = None,
        timeout: float = SEND_TIMEOUT_SECS,
    ) -> tuple[int, dict | None, bytes, dict]:
        """Issue an HTTP request and return
        ``(status, parsed_json_or_None, raw_body, response_headers)``.
        Response header keys are normalised to lowercase so callers can
        do case-insensitive lookups (notably for ``Retry-After`` on
        429). Captures HTTPError and surfaces it via the status code so
        callers can branch on 401 / 4xx / 5xx without try/except
        (response headers are still returned on HTTPError, which is
        what makes ``Retry-After`` reachable after a 429)."""
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

    @staticmethod
    def _retry_after_secs(resp_headers: dict) -> float:
        """Parse ``Retry-After`` (seconds form). Falls back to
        ``RETRY_AFTER_DEFAULT_SECS`` if absent / unparseable, floored at
        1 s and capped at ``MAX_BACKOFF_SECS`` so a misreported value
        can't block the poller for more than a minute. We don't support
        the HTTP-date form — Talk's bruteforce throttler always sends
        seconds for rate-limit replies."""
        raw = resp_headers.get("retry-after")
        if not raw:
            return RETRY_AFTER_DEFAULT_SECS
        try:
            return min(max(float(raw), 1.0), MAX_BACKOFF_SECS)
        except (TypeError, ValueError):
            return RETRY_AFTER_DEFAULT_SECS

    # ---- startup: validate credentials -------------------------------

    def _verify_credentials(self) -> str:
        """Call ``GET /ocs/v2.php/cloud/user?format=json`` to validate
        the token and discover the bot's own user id (used as the
        self-skip key). Returns the user id for logging.

        Mirrors the Rust adapter's `validate()` (nextcloud.rs:87-101);
        a non-2xx response raises so the caller backs off."""
        url = f"{self.server_url}/ocs/v2.php/cloud/user?format=json"
        status, resp, raw, resp_hdrs = self._http(
            url, headers=self._auth_headers(),
        )
        if status == 429:
            # OCS bruteforce protection — usually triggered by a string
            # of failed auths from the same IP. Honour Retry-After and
            # raise; the producer loop's exponential backoff would
            # otherwise spam more probes inside the throttling window.
            wait = self._retry_after_secs(resp_hdrs)
            log.warn(
                "nextcloud 429 on /cloud/user "
                "(OCS bruteforce throttling); sleeping",
                retry_after_secs=wait,
            )
            time.sleep(wait)
            raise RuntimeError("nextcloud 429 — bruteforce throttled")
        if status != 200 or not isinstance(resp, dict):
            snippet = raw[:200].decode("utf-8", "replace")
            raise RuntimeError(
                f"nextcloud authentication failed {status}: {snippet}"
            )
        # OCS responses nest the payload under `ocs.data`.
        data = resp.get("ocs", {}).get("data") if isinstance(resp, dict) else None
        uid = ""
        if isinstance(data, dict):
            v = data.get("id")
            if isinstance(v, str):
                uid = v
        # The Rust adapter accepted "unknown" silently — surface it as
        # a warning so an operator who's mis-scoped the token sees it,
        # without failing the boot.
        if not uid:
            log.warn(
                "nextcloud /cloud/user returned no id; self-skip will "
                "be disabled (every message routes to the bot)",
            )
        self.own_user_id = uid
        return uid or "unknown"

    # ---- room discovery ----------------------------------------------

    def _list_joined_rooms(self) -> list[str]:
        """Discover joined room tokens via
        ``GET /ocs/v2.php/apps/spreed/api/v4/room?format=json``.
        Returns an empty list (and logs a warning) on failure — the
        producer loop then has nothing to poll and bails. Mirrors the
        Rust adapter's `fetch_rooms` (nextcloud.rs:105-127)."""
        url = (
            f"{self.server_url}"
            f"/ocs/v2.php/apps/spreed/api/v4/room?format=json"
        )
        try:
            status, body, _raw, resp_hdrs = self._http(
                url, headers=self._auth_headers(),
            )
        except urllib.error.URLError as e:
            log.warn(
                "nextcloud room list transport error",
                error=str(e),
            )
            return []
        if status == 429:
            # Same OCS bruteforce throttling can surface here. Sleeping
            # is enough — discovery is one-shot, so we just yield no
            # rooms and let the next producer-loop iteration retry.
            wait = self._retry_after_secs(resp_hdrs)
            log.warn(
                "nextcloud 429 on room discovery; sleeping",
                retry_after_secs=wait,
            )
            time.sleep(wait)
            return []
        if status != 200 or not isinstance(body, dict):
            log.warn(
                "nextcloud room list failed",
                status=status,
            )
            return []
        data = body.get("ocs", {}).get("data")
        if not isinstance(data, list):
            return []
        out: list[str] = []
        for room in data:
            if isinstance(room, dict):
                tok = room.get("token")
                if isinstance(tok, str) and tok:
                    out.append(tok)
        return out

    # ---- dedupe ------------------------------------------------------

    def _mark_seen(self, msg_id: int) -> None:
        """Track a message id with bounded eviction. Thin shim
        around :class:`librefang.sidecar.common.SeenSet` (the shared
        helper returns True/False; nextcloud historically returned
        None so we discard the return)."""
        self._seen.mark(msg_id)

    # ---- inbound parsing ---------------------------------------------

    def _is_self(self, msg: dict) -> bool:
        """Self-skip: match on (actorType, actorId) == ("users",
        self.own_user_id). The Rust adapter compared on `actorId`
        alone, which is correct in practice but ambiguous when Talk
        returns a guest / federated_users actor whose id happens to
        equal the bot's user id. Requiring `actorType == "users"`
        eliminates that ambiguity. When `own_user_id` is empty
        (credential discovery returned no id), self-skip is disabled
        — the operator already got a startup warning."""
        if not self.own_user_id:
            return False
        actor_id = msg.get("actorId")
        actor_type = msg.get("actorType") or "users"
        if not isinstance(actor_id, str):
            return False
        return actor_type == "users" and actor_id == self.own_user_id

    def _parse_message(self, msg: dict, room_token: str) -> dict | None:
        """Parse a Talk chat element into a ``message`` event. Returns
        ``None`` if the message should be skipped (system message,
        self, empty body, malformed)."""
        if not isinstance(msg, dict):
            return None
        # Talk emits `messageType="system"` for join/leave/etc. — the
        # Rust adapter filters these out (nextcloud.rs:331-334).
        msg_type = msg.get("messageType")
        if isinstance(msg_type, str) and msg_type == "system":
            return None
        if self._is_self(msg):
            return None
        text = msg.get("message")
        if not isinstance(text, str) or not text:
            return None

        # Talk's message id is a positive integer (i64 in the Rust
        # adapter at nextcloud.rs:347). Coerce defensively.
        raw_id = msg.get("id")
        try:
            msg_id = int(raw_id) if raw_id is not None else 0
        except (TypeError, ValueError):
            msg_id = 0
        actor_id = msg.get("actorId") if isinstance(msg.get("actorId"), str) else ""
        actor_type = (
            msg.get("actorType")
            if isinstance(msg.get("actorType"), str)
            else "users"
        )
        actor_display = (
            msg.get("actorDisplayName")
            if isinstance(msg.get("actorDisplayName"), str)
            else "unknown"
        )
        reference_id = msg.get("referenceId")
        if not isinstance(reference_id, str) or not reference_id:
            reference_id = None
        # Talk surfaces the parent message id via `parentMessage.id`
        # when this message is a threaded reply.
        parent_msg = msg.get("parentMessage")
        parent_msg_id: str | None = None
        if isinstance(parent_msg, dict):
            pv = parent_msg.get("id")
            if isinstance(pv, int):
                parent_msg_id = str(pv)
            elif isinstance(pv, str) and pv:
                parent_msg_id = pv

        if text.startswith("/"):
            head, _, tail = text[1:].partition(" ")
            content = Content.command(head, tail.split() if tail else [])
        else:
            content = Content.text(text)

        metadata: dict[str, Any] = {
            "actor_id": actor_id,
            "actor_type": actor_type,
            "actor_display_name": actor_display,
            "room_token": room_token,
        }
        if reference_id is not None:
            metadata["reference_id"] = reference_id
        if parent_msg_id is not None:
            metadata["parent_message_id"] = parent_msg_id

        # Surface a thread_id so `on_send` can round-trip it as Talk's
        # `replyTo` form parameter on the outbound POST. If the
        # inbound was already inside a thread, prefer the existing
        # parent message id so the bot's reply lands in the same
        # thread (rather than starting a child under the user's
        # reply). Otherwise use the inbound `id`, which Talk accepts
        # as the new thread's parent.
        thread_id = parent_msg_id or (str(msg_id) if msg_id else None)

        return protocol.message(
            user_id=room_token,
            user_name=actor_display or "unknown",
            content=content,
            # Carry the room token as the canonical reply target —
            # `on_send` reads `cmd.user.platform_id` for the URL path.
            # This matches the Rust `ChannelUser{platform_id:
            # room_token}` shape (nextcloud.rs:374-378).
            channel_id=room_token,
            message_id=str(msg_id) if msg_id else None,
            is_group=True,
            thread_id=thread_id,
            metadata=metadata,
        )

    # ---- inbound: poll loop ------------------------------------------

    def _poll_once(self, emit, rooms: list[str]) -> None:
        """One pass over every configured room. Per-room transport
        errors are logged and skipped — one bad room doesn't kill the
        whole adapter. Raises only on auth (401) so the caller backs
        off and re-validates next loop."""
        for room_token in rooms:
            last_id = self._room_watermarks.get(room_token, 0)
            params = {
                "lookIntoFuture": "1",
                "limit": str(CHAT_FETCH_LIMIT),
                "lastKnownMessageId": str(last_id),
                "format": "json",
            }
            url = (
                f"{self.server_url}"
                f"/ocs/v2.php/apps/spreed/api/v1/chat/{room_token}"
                f"?{urllib.parse.urlencode(params)}"
            )
            try:
                status, body, _raw, resp_hdrs = self._http(
                    url, headers=self._auth_headers(),
                )
            except urllib.error.URLError as e:
                log.warn(
                    "nextcloud chat poll transport error",
                    room_token=room_token, error=str(e),
                )
                continue
            if status == 429:
                # OCS bruteforce protection. Honour Retry-After then
                # raise so the producer loop sleeps before its next
                # pass — without this the per-room loop would keep
                # hammering the same endpoint inside the throttling
                # window, extending the ban.
                wait = self._retry_after_secs(resp_hdrs)
                log.warn(
                    "nextcloud 429 on chat poll; sleeping then backing off",
                    room_token=room_token, retry_after_secs=wait,
                )
                time.sleep(wait)
                raise RuntimeError("nextcloud 429 — rate-limited")
            if status == 401:
                # Mirror reddit / bluesky / rocketchat: surface so the
                # producer loop backs off and the next pass
                # re-validates the token.
                raise RuntimeError("nextcloud 401 — token rejected")
            # 304 Not Modified — Talk returns this when the long-poll
            # window expires with no new messages. The Rust adapter
            # treated this as a no-op (nextcloud.rs:297-300); we do
            # the same.
            if status == 304:
                continue
            if status != 200 or not isinstance(body, dict):
                log.warn(
                    "nextcloud chat poll failed",
                    room_token=room_token, status=status,
                )
                continue
            data = body.get("ocs", {}).get("data")
            if not isinstance(data, list):
                continue

            newest_id = last_id
            for msg in data:
                if not isinstance(msg, dict):
                    continue
                raw_id = msg.get("id")
                try:
                    msg_id = int(raw_id) if raw_id is not None else 0
                except (TypeError, ValueError):
                    msg_id = 0
                if msg_id and msg_id in self._seen.ids:
                    # Already emitted — could be a boundary repeat
                    # from a previous poll. Still bump the watermark
                    # so the API query keeps advancing.
                    if msg_id > newest_id:
                        newest_id = msg_id
                    continue

                ev = self._parse_message(msg, room_token)
                if msg_id:
                    self._mark_seen(msg_id)
                if msg_id > newest_id:
                    newest_id = msg_id
                if ev is None:
                    continue
                if self.account_id is not None:
                    meta = ev["params"].setdefault("metadata", {})
                    meta["account_id"] = self.account_id
                emit(ev)

            if newest_id > last_id:
                self._room_watermarks[room_token] = newest_id

    def _producer_blocking(self, emit) -> None:
        """Verify credentials, resolve the room list, then poll forever
        in this worker thread. Mirrors reddit / bluesky / rocketchat:
        verify-once with backoff, then steady-state loop with
        exponential backoff on errors."""
        verify_backoff = 1.0
        while True:
            try:
                uid = self._verify_credentials()
                log.info(
                    "nextcloud authenticated",
                    user_id=uid,
                )
                break
            except Exception as e:  # noqa: BLE001
                log.warn(
                    "nextcloud auth failed; will retry",
                    error=str(e), delay=verify_backoff,
                )
                time.sleep(verify_backoff)
                verify_backoff = min(verify_backoff * 2, MAX_BACKOFF_SECS)

        # Resolve rooms to poll. Empty allowed_rooms → discover.
        if self.allowed_rooms:
            rooms = list(self.allowed_rooms)
        else:
            rooms = self._list_joined_rooms()
            if not rooms:
                log.warn(
                    "nextcloud: no rooms to poll "
                    "(no NEXTCLOUD_ROOMS configured and the room "
                    "list endpoint returned empty); adapter will idle"
                )
                # Idle indefinitely — there's nothing useful to poll.
                # The reader/shutdown path still terminates cleanly
                # via the outer asyncio runtime.
                while True:
                    time.sleep(MAX_BACKOFF_SECS)

        # Initialise watermarks to 0 so the first poll uses
        # `lastKnownMessageId=0` (matching the Rust adapter's
        # initialisation at nextcloud.rs:243-248). Talk + lookIntoFuture
        # still only returns messages newer than the watermark; the
        # first poll therefore catches up to current.
        for room in rooms:
            self._room_watermarks.setdefault(room, 0)

        log.info(
            "nextcloud polling started",
            rooms=len(rooms), poll_interval=self.poll_interval,
        )

        backoff = 1.0
        while True:
            try:
                self._poll_once(emit, rooms)
                backoff = 1.0
            except Exception as e:  # noqa: BLE001
                log.warn(
                    "nextcloud poll error; backing off",
                    error=str(e), delay=backoff,
                )
                time.sleep(backoff)
                backoff = min(backoff * 2, MAX_BACKOFF_SECS)
            time.sleep(self.poll_interval)

    async def produce(self, emit) -> None:
        loop = asyncio.get_event_loop()
        await loop.run_in_executor(None, self._producer_blocking, emit)

    # ---- outbound: chat POST ------------------------------------------

    def _post_message(
        self, room_token: str, text: str, reply_to: str | None,
    ) -> None:
        """Post a message to a Talk room. Long bodies are chunked at
        MAX_MESSAGE_LEN and each chunk is sent as a separate message
        (matches the Rust adapter's per-chunk loop at
        nextcloud.rs:139-157). When ``reply_to`` is set, every chunk
        carries the same `replyTo` so the whole multi-part reply
        threads under the same parent."""
        if not room_token:
            raise RuntimeError(
                "nextcloud on_send: missing room token "
                "(cmd.user.platform_id was empty)"
            )
        url = (
            f"{self.server_url}"
            f"/ocs/v2.php/apps/spreed/api/v1/chat/{room_token}"
        )
        for chunk in _split_message(text, MAX_MESSAGE_LEN):
            payload: dict[str, Any] = {"message": chunk}
            if reply_to:
                # Talk's `replyTo` parameter is an integer message id;
                # the form encoder will stringify it either way.
                payload["replyTo"] = reply_to
            body = urllib.parse.urlencode(payload).encode("utf-8")
            headers = self._auth_headers({
                "Content-Type": "application/x-www-form-urlencoded",
            })
            status, _resp, raw, resp_hdrs = self._http(
                url, method="POST", body=body, headers=headers,
            )
            if status == 429:
                # POST to /chat can also be throttled (Talk rate-limits
                # chat posting separately from auth). Honour Retry-After
                # and raise once — `suppress_error_responses=True`
                # already prevents the raise from echoing back as a
                # chat reply.
                wait = self._retry_after_secs(resp_hdrs)
                log.warn(
                    "nextcloud 429 on chat POST; sleeping",
                    room_token=room_token, retry_after_secs=wait,
                )
                time.sleep(wait)
                raise RuntimeError("nextcloud 429 — rate-limited")
            if status >= 300:
                snippet = raw[:200].decode("utf-8", "replace")
                raise RuntimeError(
                    f"nextcloud chat POST error {status}: {snippet}"
                )

    async def on_send(self, cmd) -> None:
        # Text-only; structured content falls back to a placeholder so
        # the operator still sees something rather than a silent
        # failure (matches the Rust adapter's
        # `"(Unsupported content type)"` at nextcloud.rs:435-436).
        if cmd.content and not (
            isinstance(cmd.content, dict) and "Text" in cmd.content
        ):
            text = "(Unsupported content type)"
        else:
            text = cmd.text or ""

        # `cmd.user.platform_id` carries the room token (the inbound
        # adapter set it explicitly to the room token). `cmd.channel_id`
        # is the same value on the wire — fall back to it if a
        # pre-#5219 daemon ever stripped `user`.
        user = getattr(cmd, "user", None) or {}
        room_token = (
            str(user.get("platform_id") or "")
            if isinstance(user, dict)
            else ""
        )
        if not room_token:
            room_token = str(getattr(cmd, "channel_id", "") or "")

        thread_id = getattr(cmd, "thread_id", None)
        if thread_id is not None and not isinstance(thread_id, str):
            thread_id = str(thread_id) if thread_id else None
        reply_to = thread_id or None

        await asyncio.get_event_loop().run_in_executor(
            None, self._post_message, room_token, text, reply_to,
        )


if __name__ == "__main__":
    run_stdio_main(NextcloudAdapter)
