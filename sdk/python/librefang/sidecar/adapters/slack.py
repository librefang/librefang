#!/usr/bin/env python3
"""Slack Socket Mode sidecar channel adapter for LibreFang.

Replaces the former in-process Rust ``librefang-channels::slack``
adapter (removed in this sidecar migration; same pattern as ntfy
#5224, telegram #5241, gotify #5263, mastodon #5264, bluesky #5277,
reddit #5281, discord #5299).

Behaviour parity with the Rust adapter:

* **Auth probe**: ``POST /api/auth.test`` with the bot token at
  startup to discover the bot's own ``user_id`` (used for self-skip).
* **Socket Mode**: ``POST /api/apps.connections.open`` with the
  app-level token (``xapp-…``) returns a WSS URL. We connect and
  read JSON envelopes (``hello`` / ``events_api`` / ``interactive`` /
  ``disconnect``). Each ``events_api`` / ``interactive`` envelope
  must be ACK'd by echoing back ``{"envelope_id": "..."}``.
* **Event handling**: only ``message`` and ``app_mention`` types
  produce ``message`` events. Subtype filter: bare messages pass,
  ``message_changed`` extracts ``event.message`` (edit), every other
  subtype is dropped (joins, leaves, file_share, etc.). Self-skip on
  ``bot_id`` present OR ``user == bot_user_id``.
* **Allowed channels**: empty list = allow all. When non-empty,
  channel must be in the list; DMs (``channel`` starts with ``D``)
  are exempt (the operator's per-user DM allowlist handles those).
* **Display name**: Slack user IDs as display name (the Rust adapter
  surfaces the raw ``Uxxxxxxx`` id, deliberately — DM resolution and
  the kernel user mapping run on the id, not the human name).
* **Slash commands**: ``/cmd args`` → ``Command`` (text otherwise).
* **Thread context**: ``thread_ts`` is surfaced as ``thread_id`` so
  replies thread under the originating message.
* **DM vs group**: ``is_group = not channel.startswith('D')``.
* **Block Kit interactive**: ``block_actions`` payloads → first
  action's ``value`` becomes ``ButtonCallback.action``; ``action_id``,
  ``trigger_id``, and the ``block_action`` flag ride in metadata.
* **REST send**: ``POST /api/chat.postMessage`` with the bot token,
  optional ``thread_ts`` and ``unfurl_links``. 3 000-char chunking
  (matches the Rust ``SLACK_MSG_LIMIT``).
* **Reactions**: ``eyes`` on receive, ``white_check_mark`` on
  completion (opt-out via ``SLACK_REACTIONS=false``).

Stdlib-only: HTTPS via ``urllib.request``, WebSocket via a
hand-rolled RFC 6455 client over ``socket`` + ``ssl`` (same pattern
as the discord sidecar #5299).

Configure via ``[[sidecar_channels]]``::

    [[sidecar_channels]]
    name = "slack"
    command = "python3"
    args = ["-m", "librefang.sidecar.adapters.slack"]
    channel_type = "slack"
    [sidecar_channels.env]
    # SLACK_ALLOWED_CHANNELS = "C0123,C0456"
    # SLACK_UNFURL_LINKS = "false"
    # SLACK_FORCE_FLAT_REPLIES = "false"
    # SLACK_REACTIONS = "true"
    # SLACK_ACCOUNT_ID = "workspace-prod"

Secrets via ``~/.librefang/secrets.env``: ``SLACK_APP_TOKEN`` (the
``xapp-…`` app-level token used to open the Socket Mode connection)
AND ``SLACK_BOT_TOKEN`` (the ``xoxb-…`` bot token used for every Web
API call).
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
    http_request as _http_request,
    MAX_BACKOFF_SECS,
    parse_retry_after as _parse_retry_after_impl,
    SeenSet as _SeenSet,
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

# Slack constants — mirror crate::slack defaults.
DEFAULT_API_BASE = "https://slack.com/api"
# Slack's chat.postMessage caps the `text` field at 4000 chars but
# clients render the first 3000 cleanly; the Rust adapter used 3000
# (`SLACK_MSG_LIMIT`) so we preserve that.
SLACK_MSG_LIMIT = 3000

SEND_TIMEOUT_SECS = 15.0
HANDSHAKE_TIMEOUT_SECS = 15.0

INITIAL_BACKOFF_SECS = 1.0
READ_TICK_SECS = 30.0
def _bool_env(raw: str, *, default: bool) -> bool:
    """Parse a permissive bool env var. ``""`` / unset → ``default``."""
    v = raw.strip().lower()
    if not v:
        return default
    if v in ("false", "0", "no", "off"):
        return False
    if v in ("true", "1", "yes", "on"):
        return True
    return default


def parse_users_info(body: dict) -> tuple[Optional[str], Optional[str]]:
    """Translate a Slack ``users.info`` response into a role token.

    Returns ``(role, error)``. ``role`` is one of ``owner`` /
    ``admin`` / ``guest`` / ``member``; ``None`` when Slack reports
    ``user_not_found`` (the kernel's RBAC then default-denies the
    user, matching the Rust adapter). ``error`` carries the platform
    error string for any other failure.
    """
    if not isinstance(body, dict):
        return None, "non-object response"
    if body.get("ok") is not True:
        err = str(body.get("error") or "unknown error")
        if err == "user_not_found":
            return None, None
        return None, err
    user = body.get("user") or {}
    if user.get("is_owner") is True or user.get("is_primary_owner") is True:
        return "owner", None
    if user.get("is_admin") is True:
        return "admin", None
    if user.get("is_restricted") is True or user.get("is_ultra_restricted") is True:
        return "guest", None
    return "member", None


# ---------------------------------------------------------------------------
# Inbound event parsing — port of crate::slack::parse_slack_event and
# parse_slack_block_action. Pure functions so tests can exercise every
# filter / variant without standing up the Socket Mode WS.
# ---------------------------------------------------------------------------


def parse_slack_event(
    event: dict,
    *,
    bot_user_id: Optional[str],
    allowed_channels: list[str],
    account_id: Optional[str],
) -> Optional[dict]:
    """Mirror of the Rust ``parse_slack_event``.

    Returns the ``message`` event dict ready to ``emit``, or ``None``
    when the payload should be skipped.
    """
    if not isinstance(event, dict):
        return None
    event_type = event.get("type")
    if event_type not in ("message", "app_mention"):
        return None

    subtype = event.get("subtype")
    if subtype == "message_changed":
        inner = event.get("message")
        if not isinstance(inner, dict):
            return None
        msg_data = inner
        is_edit = True
    elif subtype is not None:
        # Other subtypes (joins, leaves, file_share, …) are skipped —
        # matches the Rust adapter precisely.
        return None
    else:
        msg_data = event
        is_edit = False

    # Self-skip: drop messages from any bot id, or any message that
    # came from the bot's own user_id (which may arrive without a
    # bot_id on legacy app routes).
    if msg_data.get("bot_id") is not None:
        return None
    user_id = msg_data.get("user") or event.get("user")
    if not isinstance(user_id, str) or not user_id:
        return None
    if bot_user_id and user_id == bot_user_id:
        return None

    channel = event.get("channel")
    if not isinstance(channel, str) or not channel:
        return None

    # DMs (channel id starts with 'D') are exempt from the allowlist.
    if (
        not channel.startswith("D")
        and allowed_channels
        and channel not in allowed_channels
    ):
        return None

    text = msg_data.get("text")
    if not isinstance(text, str) or not text:
        return None

    ts = (msg_data.get("ts") if is_edit else None) or event.get("ts") or "0"
    if not isinstance(ts, str):
        ts = str(ts)

    if text.startswith("/"):
        head, _, tail = text[1:].partition(" ")
        content = Content.command(head, tail.split() if tail else [])
    else:
        content = Content.text(text)

    is_group = not channel.startswith("D")
    thread_ts = msg_data.get("thread_ts") or event.get("thread_ts")
    # Fall back to the message's own ts when it is not already inside a
    # thread. Two reasons: (1) a reply to a top-level message then threads
    # under it (Slack's default bot UX — the `force_flat_replies` knob
    # opts out), mirroring rocketchat / nextcloud's `thread_id = parent or
    # own_id`; (2) on_send round-trips this id to finalize the :eyes:
    # reaction on the exact triggering message, which is tracked by its own
    # ts. Without the fallback, top-level messages carried `thread_id =
    # None` and reaction finalization fell back to "first pending in the
    # channel", flipping the wrong message under concurrency.
    thread_id = thread_ts if isinstance(thread_ts, str) else ts

    metadata: dict[str, Any] = {
        # SENDER_USER_ID_KEY in the Rust adapter — preserves the
        # actual Slack user id so the kernel's user mapping can find
        # an explicit `[users.<id>]` binding even when the platform_id
        # routes to a DM channel.
        "sender_user_id": user_id,
    }
    if event_type == "app_mention":
        metadata["was_mentioned"] = True
    if account_id is not None:
        metadata["account_id"] = account_id

    return protocol.message(
        # platform_id is the channel id (D… for DMs, C… for channels,
        # G… for private groups). The kernel uses this as the reply
        # target — matching Rust's `sender.platform_id = channel`.
        user_id=channel,
        # Display name is the Slack user id verbatim — the Rust
        # adapter doesn't try to resolve display names (it would
        # need an extra `users.info` call per message). Operators
        # who want human-readable names set them in `[users]`.
        user_name=user_id,
        content=content,
        message_id=ts,
        is_group=is_group,
        thread_id=thread_id,
        metadata=metadata,
    )


def parse_slack_block_action(
    interaction: dict,
    *,
    bot_user_id: Optional[str],
    allowed_channels: list[str],
    account_id: Optional[str],
) -> Optional[dict]:
    """Mirror of the Rust ``parse_slack_block_action``.

    Returns a ``message`` event carrying a ``ButtonCallback`` content
    variant, or ``None`` for the skip cases.
    """
    if not isinstance(interaction, dict):
        return None
    if interaction.get("type") != "block_actions":
        return None

    user = interaction.get("user")
    if not isinstance(user, dict):
        return None
    user_id = user.get("id")
    if not isinstance(user_id, str) or not user_id:
        return None
    if bot_user_id and user_id == bot_user_id:
        return None

    channel_obj = interaction.get("channel") or {}
    channel = channel_obj.get("id") if isinstance(channel_obj, dict) else None
    if not isinstance(channel, str) or not channel:
        return None
    if (
        not channel.startswith("D")
        and allowed_channels
        and channel not in allowed_channels
    ):
        return None

    actions = interaction.get("actions")
    if not isinstance(actions, list) or not actions:
        return None
    action = actions[0]
    if not isinstance(action, dict):
        return None
    action_value = action.get("value")
    if not isinstance(action_value, str) or not action_value:
        return None
    action_id = action.get("action_id") or ""

    message_obj = interaction.get("message") or {}
    message_text = message_obj.get("text") if isinstance(message_obj, dict) else None
    message_ts = (
        message_obj.get("ts") if isinstance(message_obj, dict) else None
    ) or "0"
    if not isinstance(message_ts, str):
        message_ts = str(message_ts)
    trigger_id = interaction.get("trigger_id") or ""

    thread_ts = message_obj.get("thread_ts") if isinstance(message_obj, dict) else None
    thread_id = thread_ts if isinstance(thread_ts, str) else None

    metadata: dict[str, Any] = {
        "sender_user_id": user_id,
        "action_id": action_id,
        "trigger_id": trigger_id,
        "block_action": True,
    }
    if account_id is not None:
        metadata["account_id"] = account_id

    return protocol.message(
        user_id=channel,
        user_name=user_id,
        content=Content.button_callback(
            action_value,
            message_text=message_text if isinstance(message_text, str) else None,
        ),
        message_id=message_ts,
        is_group=not channel.startswith("D"),
        thread_id=thread_id,
        metadata=metadata,
    )




# ---------------------------------------------------------------------------
# Slack adapter
# ---------------------------------------------------------------------------


class SlackAdapter(SidecarAdapter):
    # The in-process adapter declared no capability strings either —
    # routing rich content (interactive, etc.) is determined per-API
    # call. We declare ``interactive`` so the kernel routes button
    # interactions back to ``on_command``/``on_send``.
    capabilities: list = ["interactive", "thread"]

    SCHEMA = Schema(
        name="slack",
        display_name="Slack",
        description="Slack Socket Mode bot adapter (out-of-process sidecar)",
        fields=[
            Field("SLACK_APP_TOKEN", "App Token (xapp-)", "secret",
                  required=True,
                  placeholder="xapp-1-..."),
            Field("SLACK_BOT_TOKEN", "Bot Token (xoxb-)", "secret",
                  required=True,
                  placeholder="xoxb-..."),
            Field("SLACK_ALLOWED_CHANNELS",
                  "Allowed Channel IDs (comma-separated, empty = allow all)",
                  "text",
                  placeholder="C0123, C0456",
                  advanced=True),
            Field("SLACK_UNFURL_LINKS",
                  "Expand link previews in sent messages",
                  "bool",
                  placeholder="true",
                  advanced=True),
            Field("SLACK_FORCE_FLAT_REPLIES",
                  "Post replies as top-level messages instead of threads",
                  "bool",
                  placeholder="false",
                  advanced=True),
            Field("SLACK_REACTIONS",
                  "Add eyes/check reactions to indicate processing state",
                  "bool",
                  placeholder="true",
                  advanced=True),
            Field("SLACK_ACCOUNT_ID",
                  "Account ID (multi-bot routing)",
                  "text",
                  placeholder="workspace-prod",
                  advanced=True),
        ],
    )

    def __init__(self) -> None:
        app_token = os.environ.get("SLACK_APP_TOKEN", "").strip()
        bot_token = os.environ.get("SLACK_BOT_TOKEN", "").strip()
        missing = []
        if not app_token:
            missing.append("SLACK_APP_TOKEN")
        if not bot_token:
            missing.append("SLACK_BOT_TOKEN")
        if missing:
            log.error("slack required env vars missing", missing=missing)
            raise SystemExit(2)
        self.app_token = app_token
        self.bot_token = bot_token
        self.allowed_channels = _split_csv(
            os.environ.get("SLACK_ALLOWED_CHANNELS", "")
        )
        # `SLACK_UNFURL_LINKS` is tri-state in the Rust adapter
        # (``None`` = "use Slack default"); unset env means None, an
        # explicit "false"/"true" overrides.
        unfurl_raw = os.environ.get("SLACK_UNFURL_LINKS", "").strip().lower()
        if not unfurl_raw:
            self.unfurl_links: Optional[bool] = None
        elif unfurl_raw in ("false", "0", "no", "off"):
            self.unfurl_links = False
        else:
            self.unfurl_links = True
        self.force_flat_replies = _bool_env(
            os.environ.get("SLACK_FORCE_FLAT_REPLIES", ""), default=False,
        )
        self.reactions_enabled = _bool_env(
            os.environ.get("SLACK_REACTIONS", ""), default=True,
        )
        acct = os.environ.get("SLACK_ACCOUNT_ID", "").strip()
        self.account_id = acct or None

        self.api_base = DEFAULT_API_BASE
        self.bot_user_id: Optional[str] = None
        # (channel, ts) → emoji name. Cleared when the bot replies.
        # Bounded by `MAX_PENDING_REACTIONS` so a spike of receives
        # without sends can't grow this without bound.
        self._pending_reactions: dict[tuple[str, str], str] = {}
        self._pending_lock = threading.Lock()

    # Capacity cap on the pending-reaction map. The in-process Rust
    # adapter used an unbounded ``RwLock<HashMap>``; we cap to 2k
    # entries here so a flood of inbound messages followed by a hang
    # in the agent loop doesn't grow the map without bound.
    MAX_PENDING_REACTIONS = 2_000

    # ---- HTTP helpers ------------------------------------------------

    def _auth_headers(self, *, content_type: bool = False) -> dict:
        h = {
            "Authorization": f"Bearer {self.bot_token}",
            "User-Agent": "librefang-slack-sidecar/1 (https://librefang.org)",
        }
        if content_type:
            h["Content-Type"] = "application/json; charset=utf-8"
        return h

    def _app_token_headers(self) -> dict:
        return {
            "Authorization": f"Bearer {self.app_token}",
            "Content-Type": "application/x-www-form-urlencoded",
        }

    def _http(
        self,
        url: str,
        *,
        method: str = "GET",
        body: Optional[bytes] = None,
        headers: Optional[dict] = None,
        timeout: float = SEND_TIMEOUT_SECS,
    ) -> tuple[int, Any, bytes]:
        """Thin wrapper around
        :func:`librefang.sidecar.common.http_request`. Slack's
        callers historically unpack the 3-tuple ``(status, parsed,
        raw)`` form — strip the response-headers dict the shared
        helper returns so existing call sites don't break."""
        status, parsed, raw, _resp_hdrs = _http_request(
            url, method=method, body=body, headers=headers,
            timeout=timeout,
        )
        return status, parsed, raw

    # ---- REST: auth, socket-mode URL, send, reactions, role lookup --

    def _validate_bot_token(self) -> str:
        """Return the bot's own ``user_id`` from ``auth.test``. Raises
        ``RuntimeError`` on any non-ok response — the producer loop
        catches and retries with backoff."""
        status, body, raw = self._http(
            f"{self.api_base}/auth.test",
            method="POST",
            headers=self._auth_headers(),
        )
        if status != 200 or not isinstance(body, dict):
            snippet = raw[:200].decode("utf-8", "replace") if raw else ""
            raise RuntimeError(
                f"slack auth.test transport error (status={status}): {snippet}"
            )
        if body.get("ok") is not True:
            err = str(body.get("error") or "unknown error")
            raise RuntimeError(f"slack auth.test rejected: {err}")
        user_id = body.get("user_id")
        if not isinstance(user_id, str) or not user_id:
            raise RuntimeError("slack auth.test missing user_id in 200 OK body")
        return user_id

    def _fetch_socket_mode_url(self) -> str:
        status, body, raw = self._http(
            f"{self.api_base}/apps.connections.open",
            method="POST",
            body=b"",
            headers=self._app_token_headers(),
        )
        if status != 200 or not isinstance(body, dict):
            snippet = raw[:200].decode("utf-8", "replace") if raw else ""
            raise RuntimeError(
                f"slack apps.connections.open failed (status={status}): {snippet}"
            )
        if body.get("ok") is not True:
            raise RuntimeError(
                f"slack apps.connections.open rejected: {body.get('error')!r}"
            )
        url = body.get("url")
        if not isinstance(url, str) or not url.startswith("wss://"):
            raise RuntimeError(
                f"slack apps.connections.open: invalid url {url!r}"
            )
        return url

    def _post_message(
        self,
        channel_id: str,
        text: str,
        *,
        thread_ts: Optional[str] = None,
        blocks: Optional[list] = None,
    ) -> None:
        """POST chat.postMessage with chunking. Slack returns 200
        with ``{"ok": false, "error": "..."}`` on auth/permission
        failures — `_http` reports the 200 status and `_post_message`
        inspects the body for `ok` (matches the Rust adapter)."""
        chunks = (
            _split_message(text, SLACK_MSG_LIMIT) if blocks is None else [text]
        )
        for chunk in chunks:
            payload: dict[str, Any] = {"channel": channel_id, "text": chunk}
            if thread_ts:
                payload["thread_ts"] = thread_ts
            if self.unfurl_links is not None:
                payload["unfurl_links"] = self.unfurl_links
            if blocks is not None:
                payload["blocks"] = blocks
            body = json.dumps(payload).encode("utf-8")
            status, resp, raw = self._http(
                f"{self.api_base}/chat.postMessage",
                method="POST",
                body=body,
                headers=self._auth_headers(content_type=True),
            )
            if status >= 300:
                snippet = raw[:200].decode("utf-8", "replace") if raw else ""
                log.warn(
                    "slack chat.postMessage transport error",
                    status=status, body=snippet,
                )
                continue
            if isinstance(resp, dict) and resp.get("ok") is not True:
                err = resp.get("error") or "unknown"
                # Match Rust fail-open behaviour: log, continue chunking.
                log.warn("slack chat.postMessage rejected", error=err)

    def _add_reaction(self, channel: str, ts: str, name: str) -> None:
        if not self.reactions_enabled:
            return
        payload = json.dumps(
            {"channel": channel, "timestamp": ts, "name": name}
        ).encode("utf-8")
        status, resp, _raw = self._http(
            f"{self.api_base}/reactions.add",
            method="POST",
            body=payload,
            headers=self._auth_headers(content_type=True),
        )
        if status >= 300:
            log.warn("slack reactions.add transport error",
                     status=status, channel=channel, name=name)
            return
        if isinstance(resp, dict) and resp.get("ok") is not True:
            err = resp.get("error") or "unknown"
            # `already_reacted` is the most common benign failure —
            # the agent loop retried a re-emit and we already marked
            # the message. Silently swallow.
            if err != "already_reacted":
                log.warn("slack reactions.add rejected",
                         error=err, channel=channel, name=name)

    def _remove_reaction(self, channel: str, ts: str, name: str) -> None:
        if not self.reactions_enabled:
            return
        payload = json.dumps(
            {"channel": channel, "timestamp": ts, "name": name}
        ).encode("utf-8")
        status, resp, _raw = self._http(
            f"{self.api_base}/reactions.remove",
            method="POST",
            body=payload,
            headers=self._auth_headers(content_type=True),
        )
        if status >= 300:
            log.warn("slack reactions.remove transport error",
                     status=status, channel=channel, name=name)
            return
        if isinstance(resp, dict) and resp.get("ok") is not True:
            err = resp.get("error") or "unknown"
            if err != "no_reaction":
                log.warn("slack reactions.remove rejected",
                         error=err, channel=channel, name=name)

    def _track_pending_reaction(self, channel: str, ts: str, emoji: str) -> None:
        """Record that we added an ``emoji`` reaction on ``channel/ts``
        so :meth:`_finalize_pending_reaction` can flip it to
        white_check_mark after the agent reply lands."""
        key = (channel, ts)
        with self._pending_lock:
            if len(self._pending_reactions) >= self.MAX_PENDING_REACTIONS:
                # Bounded eviction: drop the oldest entry. dict
                # iteration order in CPython 3.7+ is insertion-order,
                # so popitem(last=False) semantically deletes the
                # oldest. We use next(iter(...)) for clarity.
                try:
                    old_key = next(iter(self._pending_reactions))
                    del self._pending_reactions[old_key]
                except StopIteration:
                    pass
            self._pending_reactions[key] = emoji

    def _finalize_pending_reaction(
        self, channel: str, ts: Optional[str],
    ) -> None:
        """Remove the eyes (if present) and add the white_check_mark."""
        if not self.reactions_enabled:
            return
        emoji: Optional[str] = None
        key: Optional[tuple[str, str]] = None
        with self._pending_lock:
            if ts is not None:
                key = (channel, ts)
                emoji = self._pending_reactions.pop(key, None)
            if emoji is None:
                # No explicit ts → pick the first pending entry for
                # this channel (DM context, single-message round-trip).
                for k in list(self._pending_reactions):
                    if k[0] == channel:
                        emoji = self._pending_reactions.pop(k)
                        key = k
                        break
        if emoji is not None and key is not None:
            self._remove_reaction(key[0], key[1], emoji)
            self._add_reaction(key[0], key[1], "white_check_mark")

    # ---- Socket Mode loop -------------------------------------------

    def _make_ws(self, url: str) -> _WebSocketClient:
        """Test seam."""
        return _WebSocketClient(url)

    def _run_session(
        self, ws: _WebSocketClient, emit: Callable[[dict], None],
    ) -> None:
        """Drive one Socket Mode session. Slack sends ``hello`` first,
        then a stream of ``events_api`` / ``interactive`` /
        ``disconnect`` envelopes. We ACK every events/interactive
        envelope by echoing back its ``envelope_id``."""
        ws.settimeout(None)
        while True:
            if not ws.wait_readable(READ_TICK_SECS):
                # Slack server-pings keep the TCP socket alive; if we
                # don't read anything for READ_TICK_SECS we just loop
                # the wait — no client-initiated ping needed (the WS
                # layer answers server pings with pongs automatically).
                continue
            try:
                text, close = ws.recv_frame()
            except (EOFError, OSError) as e:
                log.warn("slack socket mode socket dropped", error=str(e))
                return
            if close is not None:
                code, reason = close
                log.info("slack socket mode closed",
                         code=code,
                         reason=reason.decode("utf-8", "replace"))
                return
            if text is None:
                continue
            try:
                envelope = json.loads(text)
            except (ValueError, TypeError):
                log.warn("slack: malformed envelope JSON")
                continue
            if not isinstance(envelope, dict):
                continue
            self._handle_envelope(envelope, ws, emit)

    def _handle_envelope(
        self,
        envelope: dict,
        ws: _WebSocketClient,
        emit: Callable[[dict], None],
    ) -> None:
        env_type = envelope.get("type")
        envelope_id = envelope.get("envelope_id")

        if env_type == "hello":
            log.info("slack socket mode hello received")
            return
        if env_type == "disconnect":
            reason = envelope.get("reason") or "unknown"
            log.info("slack disconnect request", reason=reason)
            raise RuntimeError("slack-disconnect")
        if env_type == "events_api":
            # ACK first so Slack stops resending.
            if isinstance(envelope_id, str) and envelope_id:
                ws.send_text(json.dumps({"envelope_id": envelope_id}))
            event = (envelope.get("payload") or {}).get("event")
            if not isinstance(event, dict):
                return
            ev = parse_slack_event(
                event,
                bot_user_id=self.bot_user_id,
                allowed_channels=self.allowed_channels,
                account_id=self.account_id,
            )
            if ev is None:
                return
            # Add the eyes reaction so the user sees the bot is
            # working. We track (channel, ts) so the post-send hook
            # can flip eyes → check.
            params = ev["params"]
            channel_id = params["user_id"]
            ts = params.get("message_id")
            if self.reactions_enabled and isinstance(ts, str) and ts:
                self._track_pending_reaction(channel_id, ts, "eyes")
                # Best-effort, fire-and-forget — _add_reaction is
                # synchronous but Slack reactions.add returns in tens
                # of ms; doing it inline is fine and avoids spawning
                # a thread per inbound message.
                self._add_reaction(channel_id, ts, "eyes")
            emit(ev)
            return
        if env_type == "interactive":
            if isinstance(envelope_id, str) and envelope_id:
                ws.send_text(json.dumps({"envelope_id": envelope_id}))
            interaction = envelope.get("payload") or {}
            if not isinstance(interaction, dict):
                return
            ev = parse_slack_block_action(
                interaction,
                bot_user_id=self.bot_user_id,
                allowed_channels=self.allowed_channels,
                account_id=self.account_id,
            )
            if ev is not None:
                emit(ev)
            return
        # Unknown envelope types — slack adds new ones occasionally
        # (slash_commands, etc.). Forward-compat: log and ignore.
        log.debug("slack unknown envelope type", env_type=env_type)

    def _gateway_loop(self, emit: Callable[[dict], None]) -> None:
        """Outer reconnect loop. ``apps.connections.open`` issues a
        fresh WSS URL on every reconnect, so we re-fetch each
        iteration (the URL has a short TTL on Slack's side)."""
        backoff = INITIAL_BACKOFF_SECS
        # Validate the bot token once at startup. If this fails (e.g.
        # token revoked at the developer console), we back off and
        # retry — the supervisor's circuit-breaker will eventually
        # stop us if it keeps failing.
        while self.bot_user_id is None:
            try:
                self.bot_user_id = self._validate_bot_token()
                log.info("slack authenticated", bot_user_id=self.bot_user_id)
            except Exception as e:  # noqa: BLE001
                log.warn("slack auth failed; will retry",
                         error=str(e), delay=backoff)
                time.sleep(backoff)
                backoff = min(backoff * 2.0, MAX_BACKOFF_SECS)

        backoff = INITIAL_BACKOFF_SECS
        while True:
            try:
                ws_url = self._fetch_socket_mode_url()
                log.info("slack socket mode connecting")
                with self._make_ws(ws_url) as ws:
                    self._run_session(ws, emit)
                backoff = INITIAL_BACKOFF_SECS
            except Exception as e:  # noqa: BLE001 — transport varies
                log.warn("slack socket mode error; backing off",
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
            log.warn("slack on_send: empty channel_id, dropping")
            return
        # The inbound thread id (post-#5302 this is the message's own ts
        # for a top-level message, or the thread root for an in-thread
        # reply). Used as the reaction-finalization key below.
        inbound_thread_id = getattr(cmd, "thread_id", None)
        # Decide thread context for *posting*: force-flat-replies mode
        # forces the reply to a top-level post (mirrors the Rust adapter's
        # force_flat_replies knob); otherwise reply in the inbound thread.
        thread_ts = None if self.force_flat_replies else inbound_thread_id

        content = cmd.content
        text = cmd.text or ""
        loop = asyncio.get_event_loop()
        if isinstance(content, dict) and "Text" in content:
            await loop.run_in_executor(
                None,
                lambda: self._post_message(channel_id, text, thread_ts=thread_ts),
            )
        elif isinstance(content, dict) and "Interactive" in content:
            payload = content["Interactive"]
            interactive_text = payload.get("text", "") or text
            buttons = payload.get("buttons", []) or []
            blocks = _build_block_kit(interactive_text, buttons)
            await loop.run_in_executor(
                None,
                lambda: self._post_message(
                    channel_id, interactive_text,
                    thread_ts=thread_ts, blocks=blocks,
                ),
            )
        elif content and not (isinstance(content, dict) and "Text" in content):
            await loop.run_in_executor(
                None,
                lambda: self._post_message(
                    channel_id, "(Unsupported content type)",
                    thread_ts=thread_ts,
                ),
            )
        else:
            await loop.run_in_executor(
                None,
                lambda: self._post_message(channel_id, text, thread_ts=thread_ts),
            )

        # Flip eyes → white_check_mark for the message that triggered
        # this reply. The Rust adapter does this synchronously on the
        # send path; we mirror it. Finalization MUST use the inbound
        # thread id (not the posting `thread_ts`, which is forced to None
        # in force-flat mode) so it targets the message that actually got
        # the :eyes: instead of falling back to "first pending in channel"
        # and flipping the wrong message under concurrency.
        if self.reactions_enabled:
            await loop.run_in_executor(
                None,
                lambda: self._finalize_pending_reaction(
                    channel_id, inbound_thread_id,
                ),
            )


def _build_block_kit(text: str, buttons: list) -> list:
    """Render a ``Content.interactive`` payload into Slack Block Kit
    blocks. Mirrors the Rust adapter's `api_send_interactive_message`
    layout: one ``section`` block with the text (mrkdwn), then one
    ``actions`` block per row of buttons."""
    blocks: list = [{
        "type": "section",
        "text": {"type": "mrkdwn", "text": text},
    }]
    for row_idx, row in enumerate(buttons or []):
        if not isinstance(row, list):
            continue
        elements: list = []
        for btn_idx, btn in enumerate(row):
            if not isinstance(btn, dict):
                continue
            element: dict[str, Any] = {
                "type": "button",
                "text": {
                    "type": "plain_text",
                    "text": btn.get("label", ""),
                    "emoji": True,
                },
                "action_id": f"interactive_{row_idx}_{btn_idx}",
                "value": btn.get("action", ""),
            }
            style = btn.get("style")
            if style in ("primary", "danger"):
                element["style"] = style
            url = btn.get("url")
            if isinstance(url, str) and url:
                element["url"] = url
            elements.append(element)
        if elements:
            blocks.append({
                "type": "actions",
                "block_id": f"interactive_row_{row_idx}",
                "elements": elements,
            })
    return blocks


if __name__ == "__main__":
    run_stdio_main(SlackAdapter)
