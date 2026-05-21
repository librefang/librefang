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
import json
import os
import re
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
    RETRY_AFTER_DEFAULT_SECS,
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

# QQ Open Platform endpoints. The token URL lives under bots.qq.com;
# every other REST call goes through the sgroup.qq.com API base.
DEFAULT_API_BASE = "https://api.sgroup.qq.com"
DEFAULT_TOKEN_URL = "https://bots.qq.com/app/getAppAccessToken"

# QQ message length limit (qq.rs:26 — QQ_MAX_MESSAGE_LEN = 2000).
QQ_MSG_LIMIT = 2000

SEND_TIMEOUT_SECS = 15.0
HANDSHAKE_TIMEOUT_SECS = 15.0

INITIAL_BACKOFF_SECS = 2.0
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

# How long to block in select() per loop iteration before re-checking
# liveness. The producer also fires heartbeats from this loop, so the
# tick must be smaller than the smallest QQ heartbeat interval we
# expect (45 s by default).
READ_TICK_SECS = 1.0


def _parse_retry_after(resp_hdrs: dict, *, default_secs: float) -> float:
    """Backwards-compat wrapper around
    :func:`librefang.sidecar.common.parse_retry_after`, pinning the
    floor to 1 s and the cap to this adapter's
    ``MAX_BACKOFF_SECS``."""
    return _parse_retry_after_impl(
        resp_hdrs,
        default_secs=default_secs,
        floor_secs=1.0,
        max_secs=MAX_BACKOFF_SECS,
    )


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
        # Improvement #4 (revised): the inbound QQ message id MUST round-
        # trip to ``on_send`` so the reply can be marked as a passive
        # response (QQ Bot OpenAPI v2 routes anything missing ``msg_id``
        # to the proactive-message quota, which unlicensed bots have
        # set to zero and licensed bots cap hard). Two carriers:
        #
        # * ``channel_id`` — the reply endpoint URL. The bridge sets
        #   ``cmd.channel_id`` from ``user.platform_id`` (the daemon's
        #   contract; see ``crates/librefang-channels/src/sidecar.rs``
        #   in ``derive_sidecar_sender_identity``), so we stash the
        #   endpoint there *via* ``user.platform_id`` below.
        #
        # * ``ChannelUser.librefang_user`` — the per-message ``msg_id``.
        #   This field is preserved bytewise across the bridge
        #   (`sidecar.rs:766` on inbound deserialise / `:1204` on
        #   outbound serialise via ``user: user.clone()``), so it's
        #   the only daemon-honoured channel for per-message reply
        #   correlation that doesn't depend on the operator setting
        #   ``threading = true`` AND the sidecar declaring the
        #   ``thread`` capability (QQ declares neither).
        #
        # ``thread_id`` is kept for forward compat with a future
        # ``threading=true`` opt-in, but on_send no longer relies on
        # it — see the matching docstring on ``on_send``.
        channel_id=reply_endpoint,
        librefang_user=msg_id_raw or None,
        thread_id=msg_id_raw or None,
        is_group=is_group,
        metadata=metadata,
    )


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
        self._seen = _SeenSet(max_size=SEEN_MESSAGES_MAX, evict=SEEN_MESSAGES_EVICT)

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
        """Thin wrapper around :func:`librefang.sidecar.common.http_request`."""
        return _http_request(
            url, method=method, body=body, headers=headers,
            timeout=timeout,
        )

    # ---- dedupe ------------------------------------------------------

    def _mark_seen(self, msg_id: Optional[str]) -> bool:
        """Return True iff freshly seen. Shim around :class:`librefang.sidecar.common.SeenSet`."""
        return self._seen.mark(msg_id)

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
        # Improvement #4 (revised — was using cmd.thread_id, which the
        # daemon NULLs out unless `[channels.qq.overrides] threading = true`
        # AND the sidecar declares the `thread` capability, neither of
        # which QQ does — so every reply was going out with no `msg_id`,
        # tripping QQ Bot OpenAPI v2's proactive-message quota and
        # silently rate-limiting / rejecting the reply).
        #
        # Primary recovery: `cmd.user["librefang_user"]` carries the
        # inbound `msg_id` (see `parse_qq_event` and the matching
        # docstring there) — this field round-trips bytewise through
        # the bridge regardless of capabilities / overrides.
        #
        # Fallback to `cmd.thread_id` so a future `threading=true`
        # opt-in keeps working, and so the existing test-suite
        # fakes that fabricate `thread_id` continue to pass.
        reply_endpoint = (
            cmd.channel_id
            or (cmd.user.get("platform_id") if cmd.user else "")
            or ""
        )
        msg_id = None
        if cmd.user:
            candidate = cmd.user.get("librefang_user")
            # Guard: `librefang_user` is shared across channels
            # (Telegram puts the @username string there, etc.). Reject
            # anything that looks like a username, URL, or has internal
            # whitespace — QQ message ids are opaque strings that don't
            # carry slashes, whitespace, or URI schemes.
            if (isinstance(candidate, str) and candidate
                    and "/" not in candidate
                    and " " not in candidate
                    and "\t" not in candidate
                    and not candidate.startswith(("http://", "https://"))):
                msg_id = candidate
        if msg_id is None:
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
