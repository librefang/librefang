#!/usr/bin/env python3
"""Rocket.Chat sidecar channel adapter for LibreFang.

Replaces the former in-process Rust ``librefang-channels::rocketchat``
adapter (removed in this sidecar migration; same pattern as ntfy
#5224, telegram #5241, gotify #5263, mastodon #5264, bluesky #5277,
reddit #5281).

Behaviour parity is preserved (REST polling of
``channels.history`` with auth via personal access token plus
``X-Auth-Token`` / ``X-User-Id`` headers, slash-command routing,
multi-bot ``account_id`` injection), with three intentional
improvements explicitly ack'd by the maintainer:

* **Outbound threading** (improvement / bugfix over Rust parity).
  The Rust adapter captured ``tmid`` (parent message id) on inbound
  but ``send()`` always called ``POST /api/v1/chat.sendMessage``
  without forwarding it, so threaded replies broke and the bot's
  responses landed at the room root regardless of the inbound
  context. The sidecar surfaces the inbound ``_id`` as
  ``thread_id`` (or the inbound ``tmid`` when the user themselves
  is already inside a thread, so the bot replies in the same thread
  rather than starting a child), and ``on_send`` calls
  ``POST /api/v1/chat.postMessage`` with ``tmid`` populated so the
  reply threads correctly. Mirrors reddit / bluesky / mastodon.
* **Dedupe by message ``_id``** (improvement over Rust parity).
  The Rust adapter advanced ``last_timestamps`` by RFC3339 string
  and re-fetched ``oldest=<watermark>``. With second-granularity
  ``ts`` and ``count=50``, two messages sharing the same timestamp
  caused either a re-emission on the next poll (same-ts duplicates
  refetched) or a silent drop (if the watermark advances past one of
  them). The sidecar keeps the RFC3339 watermark for the API query
  but additionally dedupes locally on ``_id`` with bounded eviction,
  matching reddit / bluesky.
* **Self-skip by user id, not username** (improvement over Rust
  parity). The Rust adapter compared ``u.username == own_username``;
  for instances that let users change their displayed username
  without invalidating the bot's token this can silently break
  self-skip. The sidecar compares ``u._id == ROCKETCHAT_USER_ID``
  (which is the stable internal id the adapter was already
  configured with) and falls back to username for instances where
  the inbound shape omits ``u._id``.

Inbound: per-room polling of ``GET /api/v1/channels.history`` at the
configured interval (default 2 s, matching the Rust adapter, with a
floor of 1 s). On startup, ``GET /api/v1/me`` validates the token and
discovers the bot's own username (also kept as a fallback self-skip
key). Empty ``ROCKETCHAT_CHANNELS`` discovers joined channels via
``GET /api/v1/channels.list.joined``. Slash-command bodies (``/cmd
args``) become ``Content.command``; everything else is plain
``Content.text``. Metadata carries ``sender_id``, ``sender_username``,
``room_id``, ``ts``, ``tmid`` (when inbound was inside a thread), and
``account_id`` when ``ROCKETCHAT_ACCOUNT_ID`` is set.

Outbound: ``POST /api/v1/chat.postMessage`` with ``rid`` (the room
id, carried in ``user.platform_id``), ``text`` (chunked at 4096
chars, matching the Rust ``MAX_MESSAGE_LEN``), and ``tmid``
(``cmd.thread_id`` from the round-trip). Non-text content falls
back to a placeholder string so the operator still sees something
rather than a silent failure (matches the Rust adapter).

Stdlib-only (the SDK has zero runtime deps): HTTP via
``urllib.request``, polling on a worker thread.

Configure via ``[[sidecar_channels]]``:

    [[sidecar_channels]]
    name = "rocketchat"
    command = "python3"
    args = ["-m", "librefang.sidecar.adapters.rocketchat"]
    channel_type = "rocketchat"
    [sidecar_channels.env]
    ROCKETCHAT_SERVER_URL = "https://chat.example.com"
    ROCKETCHAT_USER_ID = "abc123"
    # ROCKETCHAT_CHANNELS = "GENERAL,room2"        # optional; empty = all joined
    # ROCKETCHAT_ACCOUNT_ID = "prod"                # optional, multi-bot routing
    # ROCKETCHAT_POLL_INTERVAL_SECS = "2"           # optional, default 2, floor 1

The Rocket.Chat personal access token belongs in
``~/.librefang/secrets.env`` as ``ROCKETCHAT_TOKEN`` (the dashboard's
Channels page writes it there when you fill the Rocket.Chat form).
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

# Matches the Rust adapter's MAX_MESSAGE_LEN. Rocket.Chat's hard cap is
# typically configured at 5000 by default and operator-tunable; 4096 is
# the conservative shared default the Rust adapter shipped and we keep
# it for parity.
MAX_MESSAGE_LEN = 4096
# Default poll interval matches the Rust adapter (POLL_INTERVAL_SECS).
DEFAULT_POLL_INTERVAL_SECS = 2
MIN_POLL_INTERVAL_SECS = 1
SEND_TIMEOUT_SECS = 15
MAX_BACKOFF_SECS = 60.0
# How many channels to fetch in `channels.list.joined`. Matches Rust.
LIST_JOINED_COUNT = 100
# How many messages per `channels.history` poll. Matches Rust.
HISTORY_COUNT = 50
# Bounded dedupe set: capped + oldest half evicted on overflow, same
# policy as reddit. Messages older than the watermark would already be
# excluded by the `oldest=` query param, so this only has to hold
# enough to cover ts-collisions and overlapping fetches between polls.
SEEN_MESSAGES_MAX = 10000
SEEN_MESSAGES_EVICT = 5000


def _split_message(text: str, max_len: int) -> list[str]:
    """Chunk `text` into <= max_len pieces, preferring newline splits.
    Matches the Rust ``split_message`` helper used across channels."""
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


def _parse_channels(raw: str) -> list[str]:
    """Comma-separated channel id list. Strips whitespace and empty
    entries; preserves order. Empty input → empty list (means 'discover
    joined channels at startup')."""
    return [s.strip() for s in raw.split(",") if s.strip()]


class RocketChatAdapter(SidecarAdapter):
    # Rocket.Chat messages can be public to a whole room — mirroring
    # mastodon / bluesky / reddit we suppress the error-echo path so an
    # internal exception doesn't surface as a chat message to every
    # member of the room.
    suppress_error_responses = True
    # No typing / reaction / interactive / streaming concept on the
    # Rocket.Chat REST adapter (typing is realtime/DDP-only; mirrors the
    # Rust adapter's no-op `send_typing`).
    capabilities: list = []

    SCHEMA = Schema(
        name="rocketchat",
        display_name="Rocket.Chat",
        description="Rocket.Chat REST API (out-of-process sidecar)",
        fields=[
            Field("ROCKETCHAT_SERVER_URL", "Server URL", "text",
                  required=True,
                  placeholder="https://chat.example.com"),
            Field("ROCKETCHAT_TOKEN", "Personal Access Token",
                  "secret", required=True,
                  placeholder="abc123..."),
            Field("ROCKETCHAT_USER_ID", "Bot User ID", "text",
                  required=True,
                  placeholder="abc123"),
            Field("ROCKETCHAT_CHANNELS",
                  "Channel IDs (comma-separated; empty = all joined)",
                  "text",
                  placeholder="GENERAL,room2"),
            Field("ROCKETCHAT_ACCOUNT_ID",
                  "Account ID (multi-bot routing)", "text",
                  placeholder="prod", advanced=True),
            Field("ROCKETCHAT_POLL_INTERVAL_SECS",
                  f"Poll interval seconds (default {DEFAULT_POLL_INTERVAL_SECS}, "
                  f"floor {MIN_POLL_INTERVAL_SECS})",
                  "text",
                  placeholder=str(DEFAULT_POLL_INTERVAL_SECS),
                  advanced=True),
        ],
    )

    def __init__(self) -> None:
        server = os.environ.get("ROCKETCHAT_SERVER_URL", "").strip()
        # Strip trailing slashes the same way the Rust adapter did
        # (`trim_end_matches('/')`) so callers get the same URL shape
        # whether they configured `https://chat.example.com` or
        # `https://chat.example.com/`.
        self.server_url = server.rstrip("/")
        self.token = os.environ.get("ROCKETCHAT_TOKEN", "").strip()
        self.user_id = os.environ.get("ROCKETCHAT_USER_ID", "").strip()
        channels_raw = os.environ.get("ROCKETCHAT_CHANNELS", "").strip()
        self.allowed_channels = _parse_channels(channels_raw)
        acct = os.environ.get("ROCKETCHAT_ACCOUNT_ID", "").strip()
        self.account_id = acct or None

        interval_raw = os.environ.get(
            "ROCKETCHAT_POLL_INTERVAL_SECS", "",
        ).strip()
        try:
            interval = (
                int(interval_raw) if interval_raw
                else DEFAULT_POLL_INTERVAL_SECS
            )
        except (TypeError, ValueError):
            log.error(
                "ROCKETCHAT_POLL_INTERVAL_SECS invalid (must be integer)",
                value=interval_raw,
            )
            raise SystemExit(2) from None
        if interval < MIN_POLL_INTERVAL_SECS:
            log.warn(
                "ROCKETCHAT_POLL_INTERVAL_SECS below floor; clamping",
                requested=interval, floor=MIN_POLL_INTERVAL_SECS,
            )
            interval = MIN_POLL_INTERVAL_SECS
        self.poll_interval = interval

        missing: list[str] = []
        if not self.server_url:
            missing.append("ROCKETCHAT_SERVER_URL")
        if not self.token:
            missing.append("ROCKETCHAT_TOKEN")
        if not self.user_id:
            missing.append("ROCKETCHAT_USER_ID")
        if missing:
            log.error("rocketchat required env vars missing", missing=missing)
            raise SystemExit(2)
        if not (self.server_url.startswith("http://")
                or self.server_url.startswith("https://")):
            log.error(
                "ROCKETCHAT_SERVER_URL must start with http:// or https://",
                server_url=self.server_url,
            )
            raise SystemExit(2)

        # Discovered at startup via `_verify_credentials()` — used as a
        # FALLBACK self-skip key when an inbound message's `u._id` is
        # missing. The primary self-skip key is `self.user_id` (env var
        # `ROCKETCHAT_USER_ID`), which is the stable internal id.
        self.own_username: str = ""

        # Per-room watermark (the RFC3339 `oldest=` cursor the API
        # accepts). Initialised to "now" on first poll so the bot only
        # sees messages posted after it started.
        self._room_watermarks: dict[str, str] = {}
        # Dedupe set on Rocket.Chat message `_id` (cheap, bounded). Same
        # cap / eviction policy as reddit.
        self._seen_messages: list[str] = []
        self._seen_messages_set: set[str] = set()

    # ---- HTTP helpers ------------------------------------------------

    def _auth_headers(self, extra: dict | None = None) -> dict:
        h = {
            "X-Auth-Token": self.token,
            "X-User-Id": self.user_id,
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
    ) -> tuple[int, dict | None, bytes]:
        """Issue an HTTP request and return
        ``(status, parsed_json_or_None, raw_body)``. Captures HTTPError
        and surfaces it via the status code so callers can branch on
        401 / 4xx / 5xx without try/except."""
        req = urllib.request.Request(
            url, data=body, headers=headers or {}, method=method,
        )
        try:
            with urllib.request.urlopen(  # noqa: S310 — configured URL
                req, timeout=timeout,
            ) as resp:
                status = getattr(resp, "status", 200)
                raw = resp.read()
        except urllib.error.HTTPError as e:
            status = e.code
            try:
                raw = e.read()
            except Exception:  # noqa: BLE001
                raw = b""
        if not raw:
            return status, None, b""
        try:
            return status, json.loads(raw.decode("utf-8")), raw
        except (ValueError, TypeError, UnicodeDecodeError):
            return status, None, raw

    # ---- startup: validate credentials -------------------------------

    def _verify_credentials(self) -> str:
        """Call ``GET /api/v1/me`` to validate the token and discover
        the bot's own username (used as the FALLBACK self-skip key when
        an inbound message omits ``u._id``). Returns the username for
        logging."""
        url = f"{self.server_url}/api/v1/me"
        status, resp, raw = self._http(url, headers=self._auth_headers())
        if status != 200 or not isinstance(resp, dict):
            snippet = raw[:200].decode("utf-8", "replace")
            raise RuntimeError(
                f"rocketchat authentication failed {status}: {snippet}"
            )
        name = str(resp.get("username") or "")
        # The Rust adapter accepted "unknown" silently — surface it as a
        # warning so an operator who's mis-scoped the token sees it,
        # without failing the boot.
        if not name:
            log.warn(
                "rocketchat /api/v1/me returned no username; self-skip "
                "will rely on ROCKETCHAT_USER_ID only",
            )
        self.own_username = name
        return name or "unknown"

    # ---- channel discovery -------------------------------------------

    def _list_joined_channels(self) -> list[str]:
        """Discover joined channel ids via ``channels.list.joined``.
        Returns an empty list (and logs a warning) on failure — the
        producer loop then has nothing to poll and bails."""
        url = (
            f"{self.server_url}/api/v1/channels.list.joined"
            f"?count={LIST_JOINED_COUNT}"
        )
        try:
            status, body, _raw = self._http(
                url, headers=self._auth_headers(),
            )
        except urllib.error.URLError as e:
            log.warn(
                "rocketchat channels.list.joined transport error",
                error=str(e),
            )
            return []
        if status != 200 or not isinstance(body, dict):
            log.warn(
                "rocketchat channels.list.joined failed",
                status=status,
            )
            return []
        arr = body.get("channels")
        if not isinstance(arr, list):
            return []
        out: list[str] = []
        for ch in arr:
            if isinstance(ch, dict):
                cid = ch.get("_id")
                if isinstance(cid, str) and cid:
                    out.append(cid)
        return out

    # ---- dedupe ------------------------------------------------------

    def _mark_seen(self, msg_id: str) -> None:
        """Track a message id with bounded eviction. Mirrors reddit's
        list+set policy so eviction is deterministic across runs."""
        if not msg_id or msg_id in self._seen_messages_set:
            return
        self._seen_messages.append(msg_id)
        self._seen_messages_set.add(msg_id)
        if len(self._seen_messages) > SEEN_MESSAGES_MAX:
            to_drop = self._seen_messages[:SEEN_MESSAGES_EVICT]
            self._seen_messages = self._seen_messages[SEEN_MESSAGES_EVICT:]
            self._seen_messages_set.difference_update(to_drop)

    # ---- inbound parsing ---------------------------------------------

    def _is_self(self, msg: dict) -> bool:
        """Self-skip. Prefer `u._id == self.user_id` (the stable
        internal id the operator configured); fall back to username
        when the inbound shape omits `u._id` (older Rocket.Chat
        versions, custom message routes)."""
        u = msg.get("u")
        if isinstance(u, dict):
            uid = u.get("_id")
            if isinstance(uid, str) and uid:
                return uid == self.user_id
            uname = u.get("username")
            if isinstance(uname, str) and self.own_username:
                return uname == self.own_username
        return False

    def _parse_message(self, msg: dict, room_id: str) -> dict | None:
        """Parse a Rocket.Chat ``channels.history`` element into a
        ``message`` event. Returns ``None`` if the message should be
        skipped (self, empty body, malformed)."""
        if not isinstance(msg, dict):
            return None
        if self._is_self(msg):
            return None
        text = str(msg.get("msg") or "")
        if not text:
            return None

        msg_id = str(msg.get("_id") or "")
        ts = str(msg.get("ts") or "")
        tmid = msg.get("tmid")
        tmid_str = str(tmid) if isinstance(tmid, str) and tmid else None

        u = msg.get("u") if isinstance(msg.get("u"), dict) else {}
        sender_id = str(u.get("_id") or "")
        sender_username = str(u.get("username") or "")

        if text.startswith("/"):
            head, _, tail = text[1:].partition(" ")
            content = Content.command(head, tail.split() if tail else [])
        else:
            content = Content.text(text)

        metadata: dict[str, Any] = {
            "sender_id": sender_id,
            "sender_username": sender_username,
            "room_id": room_id,
            "ts": ts,
        }
        if tmid_str is not None:
            metadata["tmid"] = tmid_str

        # Surface a thread_id so `on_send` can round-trip it to `tmid`
        # on the outbound `chat.postMessage`. If the inbound was already
        # inside a thread, prefer the existing `tmid` so the bot's
        # reply lands in the same thread (rather than starting a child
        # under the inbound message). Otherwise use the inbound `_id`,
        # which Rocket.Chat accepts as the new thread's parent.
        thread_id = tmid_str or (msg_id or None)

        return protocol.message(
            user_id=room_id,
            user_name=sender_username or "unknown",
            content=content,
            # Carry the room id as the canonical reply target —
            # `on_send` reads `cmd.user.platform_id` for the `rid`. This
            # matches the Rust `ChannelUser{platform_id: room_id}`
            # shape.
            channel_id=room_id,
            message_id=msg_id or None,
            is_group=True,
            thread_id=thread_id,
            metadata=metadata,
        )

    # ---- inbound: poll loop ------------------------------------------

    def _poll_once(self, emit, channels: list[str]) -> None:
        """One pass over every configured room. Per-room transport
        errors are logged and skipped — one bad room doesn't kill the
        whole adapter. Raises only on auth (401) so the caller backs
        off and re-validates next loop."""
        for room_id in channels:
            oldest = self._room_watermarks.get(room_id, "")
            params = {
                "roomId": room_id,
                "oldest": oldest,
                "count": str(HISTORY_COUNT),
            }
            url = (
                f"{self.server_url}/api/v1/channels.history"
                f"?{urllib.parse.urlencode(params)}"
            )
            try:
                status, body, _raw = self._http(
                    url, headers=self._auth_headers(),
                )
            except urllib.error.URLError as e:
                log.warn(
                    "rocketchat history fetch transport error",
                    room_id=room_id, error=str(e),
                )
                continue
            if status == 401:
                # Mirror reddit / bluesky: surface so the producer loop
                # backs off and the next pass re-validates the token.
                raise RuntimeError("rocketchat 401 — token rejected")
            if status != 200 or not isinstance(body, dict):
                log.warn(
                    "rocketchat history fetch failed",
                    room_id=room_id, status=status,
                )
                continue
            messages = body.get("messages")
            if not isinstance(messages, list):
                continue

            newest_ts = oldest
            for msg in messages:
                if not isinstance(msg, dict):
                    continue
                msg_id = str(msg.get("_id") or "")
                msg_ts = str(msg.get("ts") or "")
                if msg_id and msg_id in self._seen_messages_set:
                    # Already emitted — could be a same-ts duplicate
                    # from the previous poll. Still bump the watermark
                    # so the API query keeps advancing.
                    if msg_ts > newest_ts:
                        newest_ts = msg_ts
                    continue

                ev = self._parse_message(msg, room_id)
                if msg_id:
                    self._mark_seen(msg_id)
                if msg_ts > newest_ts:
                    newest_ts = msg_ts
                if ev is None:
                    continue
                if self.account_id is not None:
                    meta = ev["params"].setdefault("metadata", {})
                    meta["account_id"] = self.account_id
                emit(ev)

            if newest_ts and newest_ts != oldest:
                self._room_watermarks[room_id] = newest_ts

    def _producer_blocking(self, emit) -> None:
        """Verify credentials, resolve the channel list, then poll
        forever in this worker thread. Mirrors reddit / bluesky:
        verify-once with backoff, then steady-state loop with
        exponential backoff on errors."""
        verify_backoff = 1.0
        while True:
            try:
                username = self._verify_credentials()
                log.info(
                    "rocketchat authenticated",
                    username=username, user_id=self.user_id,
                )
                break
            except Exception as e:  # noqa: BLE001
                log.warn(
                    "rocketchat auth failed; will retry",
                    error=str(e), delay=verify_backoff,
                )
                time.sleep(verify_backoff)
                verify_backoff = min(verify_backoff * 2, MAX_BACKOFF_SECS)

        # Resolve channels to poll. Empty allowed_channels → discover.
        if self.allowed_channels:
            channels = list(self.allowed_channels)
        else:
            channels = self._list_joined_channels()
            if not channels:
                log.warn(
                    "rocketchat: no channels to poll "
                    "(no ROCKETCHAT_CHANNELS configured and "
                    "channels.list.joined returned empty); adapter "
                    "will idle"
                )
                # Idle indefinitely — there's nothing useful to poll.
                # The reader/shutdown path still terminates cleanly via
                # the outer asyncio runtime.
                while True:
                    time.sleep(MAX_BACKOFF_SECS)

        # Initialise watermarks to "now" so we only see messages posted
        # after startup (mirrors the Rust adapter's
        # `Utc::now().to_rfc3339()` initialisation).
        import datetime
        now_iso = datetime.datetime.now(datetime.timezone.utc).isoformat()
        # Python's isoformat uses `+00:00` rather than the `Z` suffix
        # the Rust `to_rfc3339` emits; Rocket.Chat accepts both, and
        # we'd only ever compare the watermark string-wise against ts
        # values returned by the API (which use the same `+00:00`
        # convention back). Either way the bot starts seeing new
        # messages after this point.
        for room in channels:
            self._room_watermarks.setdefault(room, now_iso)

        log.info(
            "rocketchat polling started",
            channels=len(channels), poll_interval=self.poll_interval,
        )

        backoff = 1.0
        while True:
            try:
                self._poll_once(emit, channels)
                backoff = 1.0
            except Exception as e:  # noqa: BLE001
                log.warn(
                    "rocketchat poll error; backing off",
                    error=str(e), delay=backoff,
                )
                time.sleep(backoff)
                backoff = min(backoff * 2, MAX_BACKOFF_SECS)
            time.sleep(self.poll_interval)

    async def produce(self, emit) -> None:
        loop = asyncio.get_event_loop()
        await loop.run_in_executor(None, self._producer_blocking, emit)

    # ---- outbound: chat.postMessage ----------------------------------

    def _post_message(
        self, room_id: str, text: str, tmid: str | None,
    ) -> None:
        """Post a message to a Rocket.Chat room. Long bodies are
        chunked at MAX_MESSAGE_LEN and each chunk is sent as a separate
        message (matches the Rust adapter's per-chunk loop). When
        ``tmid`` is set, every chunk is posted into the same thread."""
        if not room_id:
            raise RuntimeError(
                "rocketchat on_send: missing room id "
                "(cmd.user.platform_id was empty)"
            )
        url = f"{self.server_url}/api/v1/chat.postMessage"
        for chunk in _split_message(text, MAX_MESSAGE_LEN):
            payload: dict[str, Any] = {"roomId": room_id, "text": chunk}
            if tmid:
                payload["tmid"] = tmid
            body = json.dumps(payload).encode("utf-8")
            headers = self._auth_headers(
                {"Content-Type": "application/json"},
            )
            status, resp, raw = self._http(
                url, method="POST", body=body, headers=headers,
            )
            if status >= 300:
                snippet = raw[:200].decode("utf-8", "replace")
                raise RuntimeError(
                    f"rocketchat chat.postMessage error {status}: {snippet}"
                )
            # The Rocket.Chat REST API returns `success: true` on the
            # happy path; a 200 with `success: false` is a soft error
            # (e.g. permission denied) — surface it so the operator
            # sees the message in the logs even when the HTTP layer
            # said OK.
            if isinstance(resp, dict) and resp.get("success") is False:
                err = resp.get("error") or resp.get("message") or "unknown"
                log.warn(
                    "rocketchat chat.postMessage soft-error",
                    error=str(err),
                )

    async def on_send(self, cmd) -> None:
        # Text-only; structured content falls back to a placeholder so
        # the operator still sees something rather than a silent failure
        # (matches the Rust adapter's `"(Unsupported content type)"`).
        if cmd.content and not (
            isinstance(cmd.content, dict) and "Text" in cmd.content
        ):
            text = "(Unsupported content type)"
        else:
            text = cmd.text or ""

        # `cmd.user.platform_id` carries the room id (the inbound
        # adapter set it explicitly to `room_id`). `cmd.channel_id` is
        # the same value on the wire — fall back to it if a
        # pre-#5219 daemon ever stripped `user`.
        user = getattr(cmd, "user", None) or {}
        room_id = str(user.get("platform_id") or "") if isinstance(user, dict) else ""
        if not room_id:
            room_id = str(getattr(cmd, "channel_id", "") or "")

        thread_id = getattr(cmd, "thread_id", None)
        if thread_id is not None and not isinstance(thread_id, str):
            thread_id = str(thread_id) if thread_id else None
        tmid = thread_id or None

        await asyncio.get_event_loop().run_in_executor(
            None, self._post_message, room_id, text, tmid,
        )


if __name__ == "__main__":
    run_stdio_main(RocketChatAdapter)
