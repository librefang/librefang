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
* **Event handling**: only ``message`` and ``app_mention`` types produce ``message`` events.
  Subtype filter: bare messages pass, ``message_changed`` extracts ``event.message`` (edit), ``file_share`` passes as an ordinary message carrying ``files`` (#7087), every other subtype is dropped (joins, leaves, topic changes, etc.).
  Self-skip on ``bot_id`` present OR ``user == bot_user_id``.
* **Inbound attachments** (#7087): a message's ``files`` array becomes an ``Image`` / ``Video`` / ``Audio`` / ``File`` content variant carrying ``url_private_download``, and the adapter declares ``header_rules`` so the daemon fetches that URL with the bot token — Slack's private file URLs 302 to a login page without it.
  The URL is forwarded rather than the bytes because the daemon's media pipeline is what produces a vision image block, an audio transcription or a saved document path; inbound ``FileData`` is rendered as a text placeholder and its payload discarded, so inlining bytes would deliver nothing.
  One attachment per message (the wire carries one ``ChannelContent``), the attachment outranks the message text including a slash command, and the message text rides along as the caption for every variant that has one.
  Policy knobs: ``SLACK_FILE_DOWNLOADS``, ``SLACK_FILE_MAX_BYTES``, ``SLACK_FILE_ALLOWED_EXTENSIONS``, ``SLACK_FILE_DOWNLOAD_CHANNELS`` and ``SLACK_FILE_DOWNLOAD_EXCLUDE_CHANNELS``.
  Link-unfurl ``attachments[].image_url`` is deliberately **not** followed: it is preview metadata for a URL somebody pasted, not an upload, and fetching it would point the daemon at an arbitrary host.
* **Allowed channels**: empty list = allow all. When non-empty,
  channel must be in the list; DMs (``channel`` starts with ``D``)
  are exempt (the operator's per-user DM allowlist handles those).
* **Display name**: the raw ``Uxxxxxxx`` id by default (DM resolution and the kernel user mapping run on the id, not the human name, and the in-process Rust adapter never spent a call to improve on it).
  Set ``SLACK_RESOLVE_DISPLAY_NAMES=true`` (#7086) to resolve it through ``users.info`` instead, cached per user id for ``SLACK_DISPLAY_NAME_TTL`` seconds so a busy channel costs a handful of calls a day rather than one per message.
  Requires the ``users:read`` bot scope; without it every lookup fails and the adapter keeps reporting the id.
  Off by default because turning it on changes what the daemon *stores*, not only what it shows: the roster row the bridge persists for each group sender carries whatever ``user_name`` this adapter reports.
  Operators who prefer explicit mappings keep using ``[users]``, which stays authoritative.
* **Slash commands**: ``/cmd args`` → ``Command`` (text otherwise).
* **Thread context**: ``thread_ts`` is surfaced as ``thread_id`` so
  replies thread under the originating message.
* **DM vs group**: ``is_group = not channel.startswith('D')``.
* **Bulk member enumeration** (#7086): with ``SLACK_ENUMERATE_MEMBERS=true`` every inbound group message carries the channel's full ``conversations.members`` list as ``group_members`` metadata, so an agent can answer "who is in this channel?" for people who have never spoken rather than only for those who have.
  Cached per channel for ``SLACK_MEMBER_LIST_TTL`` seconds and capped at ``SLACK_MEMBER_LIST_MAX`` members; needs ``channels:read`` / ``groups:read``.
  Names on this path come from the display-name cache only — a sweep never issues ``users.info``, because resolving hundreds of people who have not spoken would spend the whole per-method budget.
  Off by default, and for a larger reason than the display-name knob: this changes *how many people* the daemon stores, from those who addressed the agent to everyone the workspace lists in the channel.
  The daemon keeps those rows classified apart from the observational ones, so ``channel_dm`` still refuses anyone who has never addressed the agent — enumeration widens what can be *reported*, never what can be *messaged*.
* **Block Kit interactive**: ``block_actions`` payloads → first
  action's ``value`` becomes ``ButtonCallback.action``; ``action_id``,
  ``trigger_id``, and the ``block_action`` flag ride in metadata.
* **REST send**: text and Block Kit responses use ``chat.postMessage`` with optional ``thread_ts`` / ``unfurl_links`` and 3 000-char chunking. ``File`` / ``FileData`` attachments use Slack's external upload flow, preserve ``thread_ts``, and require the ``files:write`` bot scope.
* **Reactions** (#6731): the receipt is driven by the daemon's AgentPhase lifecycle, not by the receive hook — ``eyes`` on ``queued``, flipped to ``white_check_mark`` on ``done`` and ``x`` on ``error``.
  A message the daemon declines to answer (group mention-only gating, a rate-limit rejection, a slash command handled in-bridge) never reaches ``queued``, so it never gets a reaction at all instead of being left with a permanent ``eyes``.
  Opt out via ``SLACK_REACTIONS=false``.
* **Task progress** (#6451): the same AgentPhase lifecycle (Thinking → ToolUse{name} → Done/Error) is rendered as an updated-in-place Block Kit step list (``chat.update``) for multi-step turns.
  Single-step turns post no card and keep just the receipt reactions.
  Toggled independently of the receipt via ``SLACK_PROGRESS_CARD`` (#6730), which defaults to whatever ``SLACK_REACTIONS`` is set to so neither knob silently turns the other's output on.

Stdlib-only: Slack Web API calls use the shared urllib transport, URL-backed file downloads use DNS-pinned ``http.client`` connections, and WebSocket uses a hand-rolled RFC 6455 client over ``socket`` + ``ssl`` (same pattern as the discord sidecar #5299).

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
    # SLACK_PROGRESS_CARD = "true"
    # SLACK_FILE_DOWNLOADS = "true"
    # SLACK_FILE_MAX_BYTES = "10485760"
    # SLACK_FILE_ALLOWED_EXTENSIONS = "png,jpg,pdf,mp4"
    # SLACK_FILE_DOWNLOAD_CHANNELS = "C0123"
    # SLACK_FILE_DOWNLOAD_EXCLUDE_CHANNELS = "C0789"
    # SLACK_RESOLVE_DISPLAY_NAMES = "false"
    # SLACK_DISPLAY_NAME_TTL = "21600"
    # SLACK_ENUMERATE_MEMBERS = "false"
    # SLACK_MEMBER_LIST_TTL = "3600"
    # SLACK_MEMBER_LIST_MAX = "500"
    # SLACK_ACCOUNT_ID = "workspace-prod"

Secrets via ``~/.librefang/secrets.env``: ``SLACK_APP_TOKEN`` (the
``xapp-…`` app-level token used to open the Socket Mode connection)
AND ``SLACK_BOT_TOKEN`` (the ``xoxb-…`` bot token used for every Web
API call).
"""
from __future__ import annotations

import asyncio
import http.client
import ipaddress
import json
import os
import re
import socket
import ssl
import threading
import time
import urllib.parse
import urllib.request
from dataclasses import dataclass
from typing import Any, Callable, Optional

from librefang.sidecar import Content, Field, Schema, SidecarAdapter, protocol, run_stdio_main
from librefang.sidecar import logging as log
from librefang.sidecar.common import (
    http_request as _http_request,
    MAX_BACKOFF_SECS,
    parse_retry_after as _parse_retry_after,
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

# Slack constants — mirror crate::slack defaults.
DEFAULT_API_BASE = "https://slack.com/api"
# Slack's chat.postMessage caps the `text` field at 4000 chars but
# clients render the first 3000 cleanly; the Rust adapter used 3000
# (`SLACK_MSG_LIMIT`) so we preserve that.
SLACK_MSG_LIMIT = 3000
# Slack rejects a `chat.postMessage` carrying more than 50 blocks.
# Only the Block Kit paths (interactive replies, the task-progress card) can approach this; the plain-text path chunks into separate messages instead.
MAX_BLOCKS_PER_MESSAGE = 50

SEND_TIMEOUT_SECS = 15.0
HANDSHAKE_TIMEOUT_SECS = 15.0
MAX_FILE_UPLOAD_BYTES = 10 * 1024 * 1024

# Hosts that serve Slack's own `url_private` / `url_private_download` file URLs.
# Inbound attachments are pinned to this set (#7087) and it is the exact set declared in `header_rules`, so the bot token is only ever attached to a fetch of a Slack-hosted file.
#
# `files.remote.add` lets any workspace member register a "file" whose `url_private` points at a host of their choosing, and a link unfurl can put an arbitrary `image_url` in `attachments`.
# Both reach this adapter through an authentic Socket Mode envelope, so "the event came from Slack" says nothing about who chose the URL — the host pin is what keeps a member-chosen address from being fetched with the bot's credentials.
SLACK_FILE_HOSTS = ("files.slack.com", "slack-files.com")

# Slack `file.mode` values whose `url_private` no longer serves bytes.
SLACK_UNFETCHABLE_FILE_MODES = frozenset({"tombstone", "hidden_by_limit"})

# Default ceiling on an inbound attachment.
# Same 10 MiB as the outbound upload cap so a round trip (user uploads, agent edits, agent posts back) does not fail one direction with a size the other accepted.
DEFAULT_INBOUND_FILE_MAX_BYTES = MAX_FILE_UPLOAD_BYTES

INITIAL_BACKOFF_SECS = 1.0
READ_TICK_SECS = 30.0

# How long a resolved (or definitively absent) display name is trusted (#7086).
# Six hours: long enough that a busy channel costs a handful of `users.info` calls a day, short enough that somebody who changes their display name is not misnamed for a week.
DEFAULT_DISPLAY_NAME_TTL_SECS = 6 * 60 * 60

# How long a *transient* lookup failure (429, transport error) suppresses a retry.
# Deliberately short — a rate limit that has passed should not keep the whole workspace anonymous for hours.
NEGATIVE_TTL_SECS = 60.0

# Ceiling on the identity cache. A workspace larger than this degrades into extra lookups, never into unbounded memory.
MAX_CACHED_IDENTITIES = 5_000

# Bulk member enumeration (#7086).
#
# How long a channel's member list is trusted. An hour rather than the display-name TTL's six: joins and leaves are the thing this list is *about*, so a stale one is wrong in a way a stale display name is not.
DEFAULT_MEMBER_LIST_TTL_SECS = 60 * 60

# Members requested per `conversations.members` page. 200 is Slack's comfortable page size; the method is rate-limited per call, not per member, so larger pages mean fewer calls.
MEMBER_LIST_PAGE_SIZE = 200

# Ceiling on how many members one channel contributes, across all pages.
# A general channel in a large workspace lists everyone, and every one of them becomes a stored identity row in the daemon's roster — so the default answers "who is in this channel?" for team-sized channels and declines to for company-sized ones, rather than quietly persisting ten thousand people.
DEFAULT_MEMBER_LIST_MAX = 500

# Ceiling on the number of channels whose member lists are cached at once.
MAX_CACHED_MEMBER_LISTS = 256


def _resolve_public_url(
    url: str,
    *,
    require_https: bool = False,
) -> tuple[urllib.parse.SplitResult, str, list[tuple[int, tuple]]]:
    """Parse a URL, resolve every address, and reject the whole answer set if any target is non-public."""
    try:
        parsed = urllib.parse.urlsplit(url)
        host = parsed.hostname
        port = parsed.port
    except (TypeError, ValueError) as e:
        raise RuntimeError(f"invalid URL: {e}") from e
    if parsed.scheme not in ("http", "https"):
        raise RuntimeError("only http/https URLs are allowed")
    if require_https and parsed.scheme != "https":
        raise RuntimeError("HTTPS is required")
    if not host:
        raise RuntimeError("URL has no host")
    if parsed.username is not None or parsed.password is not None:
        raise RuntimeError("URL credentials are not allowed")
    if port is not None and not (1 <= port <= 65535):
        raise RuntimeError("URL port is out of range")

    normalized = host.rstrip(".").lower()
    if (
        normalized in {"localhost", "ip6-localhost", "metadata", "metadata.google.internal"}
        or normalized.endswith(".localhost")
        or normalized.endswith(".local")
    ):
        raise RuntimeError(f"host '{host}' is reserved or private")
    try:
        hostname = normalized.encode("idna").decode("ascii")
    except UnicodeError as e:
        raise RuntimeError(f"invalid internationalized hostname: {e}") from e
    resolved_port = port or (443 if parsed.scheme == "https" else 80)
    try:
        answers = socket.getaddrinfo(
            hostname,
            resolved_port,
            type=socket.SOCK_STREAM,
        )
    except OSError as e:
        raise RuntimeError(f"DNS resolution failed for '{host}': {e}") from e

    targets: list[tuple[int, tuple]] = []
    seen: set[tuple[int, str, int]] = set()
    for family, _socktype, _proto, _canonname, sockaddr in answers:
        if family not in (socket.AF_INET, socket.AF_INET6):
            continue
        address = sockaddr[0].split("%", 1)[0]
        try:
            ip = ipaddress.ip_address(address)
        except ValueError as e:
            raise RuntimeError(f"DNS returned invalid address '{address}'") from e
        if not ip.is_global:
            raise RuntimeError(f"host '{host}' resolves to non-public IP {ip}")
        key = (family, str(ip), sockaddr[1])
        if key not in seen:
            seen.add(key)
            targets.append((family, sockaddr))
    if not targets:
        raise RuntimeError(f"DNS resolution returned no usable addresses for '{host}'")
    return parsed, hostname, targets


def _validate_file_url(url: str) -> Optional[str]:
    """Return an SSRF rejection reason, or ``None`` after validating every resolved address."""
    try:
        _resolve_public_url(url)
    except RuntimeError as e:
        return str(e)
    return None


def _read_bounded_response(response, max_bytes: int) -> bytes:
    content_length = response.headers.get("content-length") if response.headers is not None else None
    if content_length:
        try:
            declared = int(content_length)
        except (TypeError, ValueError):
            declared = None
        if declared is not None and declared > max_bytes:
            raise RuntimeError(f"file exceeds {max_bytes} byte upload cap")
    data = response.read(max_bytes + 1)
    if len(data) > max_bytes:
        raise RuntimeError(f"file exceeds {max_bytes} byte upload cap")
    return data


class _PreSendConnectionError(RuntimeError):
    """A connection failure that occurred before any request bytes could be sent."""


def _request_pinned_once(
    parsed: urllib.parse.SplitResult,
    hostname: str,
    target: tuple[int, tuple],
    *,
    method: str,
    body: Optional[bytes],
    headers: Optional[dict[str, str]],
    max_bytes: int,
) -> tuple[int, bytes, Optional[str]]:
    """Issue one request through an already-validated address while retaining the original Host header and TLS SNI."""
    family, sockaddr = target
    port = parsed.port or (443 if parsed.scheme == "https" else 80)
    if parsed.scheme == "https":
        connection: http.client.HTTPConnection = http.client.HTTPSConnection(
            hostname,
            port,
            timeout=SEND_TIMEOUT_SECS,
            context=ssl.create_default_context(),
        )
    else:
        connection = http.client.HTTPConnection(
            hostname,
            port,
            timeout=SEND_TIMEOUT_SECS,
        )

    def _create_connection(
        _address,
        timeout=SEND_TIMEOUT_SECS,
        source_address=None,
        **_kwargs,
    ):
        sock = socket.socket(family, socket.SOCK_STREAM)
        try:
            if timeout is not socket._GLOBAL_DEFAULT_TIMEOUT:
                sock.settimeout(timeout)
            if source_address:
                sock.bind(source_address)
            sock.connect(sockaddr)
            return sock
        except BaseException:
            sock.close()
            raise

    connection._create_connection = _create_connection
    path = urllib.parse.urlunsplit(("", "", parsed.path or "/", parsed.query, ""))
    try:
        try:
            connection.connect()
        except (OSError, http.client.HTTPException) as e:
            raise _PreSendConnectionError(str(e)) from e
        connection.request(method, path, body=body, headers=headers or {})
        response = connection.getresponse()
        data = _read_bounded_response(response, max_bytes)
        return response.status, data, response.getheader("location")
    finally:
        connection.close()


def _public_http_request(
    url: str,
    *,
    method: str,
    body: Optional[bytes] = None,
    headers: Optional[dict[str, str]] = None,
    max_bytes: int,
    require_https: bool = False,
) -> tuple[int, bytes]:
    """Resolve and pin every request hop so validation and connection cannot diverge through DNS rebinding."""
    current_url = url
    method = method.upper()
    for redirect_count in range(6):
        parsed, hostname, targets = _resolve_public_url(
            current_url,
            require_https=require_https,
        )
        result = None
        last_error: Optional[BaseException] = None
        for target in targets:
            try:
                result = _request_pinned_once(
                    parsed,
                    hostname,
                    target,
                    method=method,
                    body=body,
                    headers=headers,
                    max_bytes=max_bytes,
                )
                break
            except _PreSendConnectionError as e:
                last_error = e
            except (OSError, http.client.HTTPException) as e:
                if method != "GET":
                    raise RuntimeError(f"request outcome is uncertain; refusing to retry {method} on another address") from e
                last_error = e
        if result is None:
            raise RuntimeError(f"request failed for every validated address: {last_error}") from last_error

        status, data, location = result
        if status not in (301, 302, 303, 307, 308) or not location:
            return status, data
        if redirect_count == 5:
            raise RuntimeError("too many redirects (cap: 5)")
        if method != "GET" and status not in (307, 308):
            raise RuntimeError(f"redirect status {status} would not preserve {method}")
        try:
            next_url = urllib.parse.urljoin(current_url, location)
        except (TypeError, ValueError) as e:
            raise RuntimeError(f"invalid URL in redirect: {e}") from e
        current_url = next_url
    raise RuntimeError("too many redirects (cap: 5)")


def _safe_filename(raw: Any) -> str:
    if not isinstance(raw, str):
        return "file"
    name = raw.replace("\\", "/").rsplit("/", 1)[-1].replace("\x00", "").strip()
    if name in ("", ".", ".."):
        return "file"
    return name[:255]


def _coerce_file_data(raw: Any) -> Optional[bytes]:
    if isinstance(raw, bytes):
        return raw
    if isinstance(raw, bytearray):
        return bytes(raw)
    if not isinstance(raw, list):
        return None
    if any(isinstance(item, bool) or not isinstance(item, int) or not 0 <= item <= 255 for item in raw):
        return None
    return bytes(raw)


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
# Display-name resolution (#7086)
# ---------------------------------------------------------------------------


@dataclass(frozen=True)
class SlackIdentity:
    """The human-readable half of a Slack user, as ``users.info`` reports it.

    ``display_name`` is what a person recognises; ``username`` is the ``@handle``.
    Either may be ``None`` — a workspace can leave both blank, and a bot without the ``users:read`` scope gets neither.
    """

    display_name: Optional[str] = None
    username: Optional[str] = None


def _first_nonempty(*candidates: Any) -> Optional[str]:
    for candidate in candidates:
        if isinstance(candidate, str) and candidate.strip():
            return candidate.strip()
    return None


def parse_users_identity(body: dict) -> tuple[Optional[SlackIdentity], Optional[str]]:
    """Translate a Slack ``users.info`` response into the name a human would recognise.

    Returns ``(identity, error)``.
    ``identity`` is ``None`` when the response carries no usable name — a deleted or unknown user, or a workspace where every name field is blank.
    ``error`` carries the platform error string for a failure the caller should treat as transient (rate limits, transport hiccups); a *definitive* "no such user" answer returns ``(None, None)`` so the caller can cache the absence instead of asking again on every message.

    Precedence for ``display_name`` follows what the person chose to be called, then falls back through what the workspace knows: ``profile.display_name`` → ``profile.real_name`` → ``user.real_name`` → ``user.name``.
    ``profile.display_name`` is empty for a large share of real accounts, which is why the ladder exists at all — resolving to an empty string would be a regression on the raw id it replaces.
    """
    if not isinstance(body, dict):
        return None, "non-object response"
    if body.get("ok") is not True:
        err = str(body.get("error") or "unknown error")
        if err in ("user_not_found", "users_not_found"):
            return None, None
        return None, err
    user = body.get("user")
    if not isinstance(user, dict):
        return None, "response has no user object"
    profile = user.get("profile")
    if not isinstance(profile, dict):
        profile = {}
    display_name = _first_nonempty(
        profile.get("display_name"),
        profile.get("real_name"),
        user.get("real_name"),
        user.get("name"),
    )
    username = _first_nonempty(user.get("name"))
    if display_name is None and username is None:
        return None, None
    return SlackIdentity(display_name=display_name, username=username), None


class _IdentityCache:
    """Bounded, TTL'd ``user_id`` → :class:`SlackIdentity` cache.

    The cache is the whole point of the feature, not an optimisation on top of it: Slack's ``users.info`` sits in a tiered per-method rate limit, and a busy channel produces one message per member per minute, so an uncached per-message lookup would spend the workspace's budget on re-resolving the same handful of people.

    Absences are cached too, and for the same reason: a deleted user, or a bot without the ``users:read`` scope, otherwise costs one doomed request per message forever.
    A *transient* failure (a 429, a transport error) is cached only for :data:`NEGATIVE_TTL_SECS`, long enough to stop a burst from hammering the API and short enough that a recovered workspace resolves names again within the minute.

    Eviction is oldest-first on insertion order once ``max_entries`` is reached — the same bound-then-evict shape as ``SlackAdapter._pending_reactions``, so a workspace with more members than the cap degrades into extra lookups rather than unbounded memory.
    """

    def __init__(self, *, ttl_secs: float, max_entries: int) -> None:
        self.ttl_secs = ttl_secs
        self.max_entries = max_entries
        # user_id -> (expires_at, identity_or_None)
        self._entries: dict[str, tuple[float, Optional[SlackIdentity]]] = {}
        self._lock = threading.Lock()

    def get(self, user_id: str) -> tuple[bool, Optional[SlackIdentity]]:
        """``(hit, identity)``. A hit with ``None`` is a cached absence, not a miss."""
        now = time.monotonic()
        with self._lock:
            entry = self._entries.get(user_id)
            if entry is None:
                return False, None
            expires_at, identity = entry
            if expires_at <= now:
                del self._entries[user_id]
                return False, None
            return True, identity

    def put(
        self,
        user_id: str,
        identity: Optional[SlackIdentity],
        *,
        ttl_secs: Optional[float] = None,
    ) -> None:
        ttl = self.ttl_secs if ttl_secs is None else ttl_secs
        with self._lock:
            # Re-inserting an existing key must not keep its old insertion position, or a hot entry would be evicted ahead of colder ones.
            self._entries.pop(user_id, None)
            while len(self._entries) >= self.max_entries:
                self._entries.pop(next(iter(self._entries)))
            self._entries[user_id] = (time.monotonic() + ttl, identity)


# ---------------------------------------------------------------------------
# Bulk member enumeration (#7086)
# ---------------------------------------------------------------------------


def parse_conversations_members(
    body: dict,
) -> tuple[list[str], Optional[str], Optional[str]]:
    """Translate one ``conversations.members`` page into ``(user_ids, next_cursor, error)``.

    ``next_cursor`` is ``None`` when the page is the last one — Slack signals that with an empty string, which is easy to mistake for "keep going with an empty cursor" and would loop forever.
    ``error`` carries the platform error string; the common ones are ``missing_scope`` (no ``channels:read`` / ``groups:read``), ``channel_not_found`` (the bot is not in the channel) and ``ratelimited``.
    """
    if not isinstance(body, dict):
        return [], None, "non-object response"
    if body.get("ok") is not True:
        return [], None, str(body.get("error") or "unknown error")
    raw = body.get("members")
    members = [m for m in raw if isinstance(m, str) and m] if isinstance(raw, list) else []
    metadata = body.get("response_metadata")
    cursor = metadata.get("next_cursor") if isinstance(metadata, dict) else None
    if not isinstance(cursor, str) or not cursor.strip():
        cursor = None
    return members, cursor, None


class _MemberListCache:
    """Bounded, TTL'd ``channel_id`` → member-id tuple cache.

    Same shape as :class:`_IdentityCache`, and load-bearing for the same reason: ``conversations.members`` is rate-limited per call, and a busy channel produces many messages per minute while its membership changes a few times a week.
    Without the cache, enumeration would re-walk every page of a 500-person channel on every inbound message.

    An empty tuple is a legitimate cached value (a channel the bot cannot read, or one it is alone in); ``hit`` distinguishes it from a miss, so a failure costs one sweep per TTL rather than one per message.
    """

    def __init__(self, *, ttl_secs: float, max_entries: int) -> None:
        self.ttl_secs = ttl_secs
        self.max_entries = max_entries
        self._entries: dict[str, tuple[float, tuple[str, ...]]] = {}
        self._lock = threading.Lock()

    def get(self, channel_id: str) -> tuple[bool, tuple[str, ...]]:
        now = time.monotonic()
        with self._lock:
            entry = self._entries.get(channel_id)
            if entry is None:
                return False, ()
            expires_at, members = entry
            if expires_at <= now:
                del self._entries[channel_id]
                return False, ()
            return True, members

    def put(
        self,
        channel_id: str,
        members: tuple[str, ...],
        *,
        ttl_secs: Optional[float] = None,
    ) -> None:
        ttl = self.ttl_secs if ttl_secs is None else ttl_secs
        with self._lock:
            self._entries.pop(channel_id, None)
            while len(self._entries) >= self.max_entries:
                self._entries.pop(next(iter(self._entries)))
            self._entries[channel_id] = (time.monotonic() + ttl, members)


# ---------------------------------------------------------------------------
# Inbound attachments (#7087)
# ---------------------------------------------------------------------------


@dataclass(frozen=True)
class SlackFilePolicy:
    """Resolved policy for inbound attachments.

    ``enabled=False`` — the default when no policy is supplied — reproduces the pre-#7087 behaviour of dropping every file-bearing message, so an operator can turn the feature off without changing anything else.

    ``channels`` is an allow-list (empty = every channel) and ``excluded_channels`` a deny-list applied on top of it, which is the ergonomic shape for the two things operators actually ask for: "only this one channel accepts uploads" and "every channel except this busy one".
    """

    enabled: bool = False
    max_bytes: int = DEFAULT_INBOUND_FILE_MAX_BYTES
    allowed_extensions: frozenset = frozenset()
    channels: tuple = ()
    excluded_channels: tuple = ()

    def enabled_for(self, channel: str) -> bool:
        """Whether attachments in ``channel`` should be forwarded to the agent."""
        if not self.enabled:
            return False
        if channel in self.excluded_channels:
            return False
        return not self.channels or channel in self.channels

    def extension_allowed(self, name: Any, filetype: Any) -> bool:
        """Whether this file's extension passes the allow-list. An empty allow-list accepts everything."""
        if not self.allowed_extensions:
            return True
        return _file_extension(name, filetype) in self.allowed_extensions


def _file_extension(name: Any, filetype: Any) -> str:
    """Lowercased extension for an inbound Slack file object.

    The filename wins because it is what the agent's tools will see; Slack's own ``filetype`` token is the fallback for uploads that arrive without a usable name.
    Returns ``""`` when neither yields one, which a non-empty allow-list then rejects.
    """
    head, dot, tail = _safe_filename(name).rpartition(".")
    if dot and head and tail:
        return tail.lower()
    if isinstance(filetype, str):
        return filetype.strip().lstrip(".").lower()
    return ""


def _is_slack_file_url(url: Any) -> bool:
    """Whether ``url`` is an HTTPS URL served by one of Slack's own file hosts.

    Userinfo is refused outright rather than ignored: ``https://files.slack.com@evil.example/x``
    reads as a Slack URL to a human and resolves to ``evil.example``.
    """
    if not isinstance(url, str) or not url:
        return False
    try:
        parsed = urllib.parse.urlsplit(url)
        host = parsed.hostname
    except (TypeError, ValueError):
        return False
    if parsed.scheme != "https" or not host:
        return False
    if parsed.username is not None or parsed.password is not None:
        return False
    return host.rstrip(".").lower() in SLACK_FILE_HOSTS


def _file_rejection(entry: Any, policy: SlackFilePolicy) -> Optional[str]:
    """Return why this Slack file object must not be forwarded, or ``None`` when it passes policy."""
    if not isinstance(entry, dict):
        return "file entry is not an object"
    mode = entry.get("mode")
    if mode in SLACK_UNFETCHABLE_FILE_MODES:
        # A deleted file, or one Slack has hidden behind a free-plan storage limit, still arrives with a `url_private` that no longer resolves.
        # Refusing it here keeps a `[File download failed]` line out of the agent's prompt.
        return f"file mode is {mode}"
    url = entry.get("url_private_download") or entry.get("url_private")
    if not isinstance(url, str) or not url:
        return "file has neither url_private_download nor url_private"
    if not _is_slack_file_url(url):
        return "file URL is not served by a Slack file host"
    if not policy.extension_allowed(
        entry.get("name") or entry.get("title"), entry.get("filetype"),
    ):
        return "file extension is not in the allow-list"
    size = entry.get("size")
    if isinstance(size, int) and not isinstance(size, bool) and size > policy.max_bytes:
        return f"file is {size} bytes, over the {policy.max_bytes} byte cap"
    return None


def _file_content(entry: dict, companion_text: str) -> dict[str, Any]:
    """Map one policy-approved Slack file object onto a ``ChannelContent`` variant.

    The URL is handed to the daemon rather than the bytes: the daemon's media pipeline is what turns a URL into an image block for vision, a transcription for audio, or a saved path for a document, and it attaches the bot token for exactly the hosts this adapter declared in ``header_rules``.
    Inlining bytes as ``FileData`` would not reach any of that — the inbound side of the bridge renders ``FileData`` as a text placeholder and discards the payload.
    """
    url = entry.get("url_private_download") or entry.get("url_private")
    filename = _safe_filename(entry.get("name") or entry.get("title"))
    raw_mime = entry.get("mimetype")
    mimetype = raw_mime.strip() if isinstance(raw_mime, str) else ""
    caption = companion_text or None
    duration_seconds = 0
    raw_ms = entry.get("duration_ms")
    if isinstance(raw_ms, int) and not isinstance(raw_ms, bool) and raw_ms > 0:
        duration_seconds = raw_ms // 1000

    if mimetype.startswith("image/"):
        return Content.image(url, caption=caption, mime_type=mimetype)
    if mimetype.startswith("video/"):
        return Content.video(url, caption=caption,
                             duration_seconds=duration_seconds,
                             filename=filename)
    if mimetype.startswith("audio/"):
        title = entry.get("title")
        return Content.audio(url, caption=caption,
                             duration_seconds=duration_seconds,
                             title=title if isinstance(title, str) and title else None)
    if companion_text:
        # `ChannelContent::File` has no caption field, so the accompanying message text has nowhere to ride.
        # Same limitation (and the same warning) as the discord sidecar's file attachments.
        log.warn(
            "slack file attachment has companion text that cannot be sent as a caption",
            filename=filename,
        )
    return Content.file(url, filename)


def parse_slack_files(
    files: Any,
    *,
    channel: str,
    companion_text: str,
    policy: Optional[SlackFilePolicy],
) -> Optional[dict[str, Any]]:
    """Pick the first policy-approved attachment out of a message's ``files`` array.

    Returns the ``ChannelContent`` for it, or ``None`` when downloads are off for this channel, the array is absent, or nothing in it passes policy.

    One attachment per message, matching the discord sidecar: the wire protocol carries a single ``ChannelContent`` per message, so a multi-file upload has to pick one.
    Extras are counted in a warning rather than silently dropped.
    """
    if policy is None or not policy.enabled_for(channel):
        return None
    if not isinstance(files, list) or not files:
        return None

    chosen: Optional[dict] = None
    rejected = 0
    extra = 0
    for entry in files:
        reason = _file_rejection(entry, policy)
        if reason is not None:
            log.warn("slack inbound attachment rejected",
                     channel=channel, reason=reason)
            rejected += 1
            continue
        if chosen is None:
            chosen = entry
        else:
            extra += 1
    if chosen is None:
        return None
    if extra:
        log.warn("slack forwarded only the first eligible attachment",
                 channel=channel, ignored=extra, rejected=rejected)
    return _file_content(chosen, companion_text)


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
    file_policy: Optional[SlackFilePolicy] = None,
) -> Optional[dict]:
    """Mirror of the Rust ``parse_slack_event``, extended with inbound attachments.

    Returns the ``message`` event dict ready to ``emit``, or ``None``
    when the payload should be skipped.

    ``file_policy`` defaults to ``None``, which drops every attachment and leaves the pre-#7087 text-only behaviour untouched.
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
    elif subtype is not None and subtype != "file_share":
        # Other subtypes (joins, leaves, topic changes, …) are skipped — matches the Rust adapter precisely.
        return None
    else:
        # `file_share` shares this arm: it is an ordinary message that also carries `files`, and dropping the whole subtype (#7087) discarded the user's upload before any content parsing ran.
        # The attachment itself is still gated by `file_policy` below.
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

    raw_text = msg_data.get("text")
    text = raw_text if isinstance(raw_text, str) else ""

    file_content = parse_slack_files(
        msg_data.get("files"),
        channel=channel,
        companion_text=text,
        policy=file_policy,
    )
    # An upload with no comment is a complete message; a text-only message with no text is not.
    if file_content is None and not text:
        return None

    ts = (msg_data.get("ts") if is_edit else None) or event.get("ts") or "0"
    if not isinstance(ts, str):
        ts = str(ts)

    if file_content is not None:
        # The attachment outranks the text, slash commands included: one message carries one `ChannelContent`, and the upload is the part the agent cannot reconstruct from the transcript.
        # Same precedence as the discord sidecar.
        content = file_content
    elif text.startswith("/"):
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
    # Named mentions so the bridge can route to a specific non-default agent
    # in a multi-agent channel (#5323). Slack encodes mentions as
    # `<@USERID>` or `<@USERID|handle>`; surface both the id and the optional
    # handle and let the bridge match them against agent names/handles.
    mention_names: list[str] = []
    idx = 0
    while True:
        start = text.find("<@", idx)
        if start == -1:
            break
        end = text.find(">", start)
        if end == -1:
            break
        for part in text[start + 2:end].split("|", 1):
            handle = part.strip()
            if handle and handle not in mention_names:
                mention_names.append(handle)
        idx = end + 1
    if mention_names:
        metadata["mention_names"] = mention_names

    return protocol.message(
        # platform_id is the channel id (D… for DMs, C… for channels,
        # G… for private groups). The kernel uses this as the reply
        # target — matching Rust's `sender.platform_id = channel`.
        user_id=channel,
        # Placeholder only. This is a pure function with no workspace access, so it stamps the raw Slack user id and `SlackAdapter._apply_identity` overwrites it with a resolved display name when `SLACK_RESOLVE_DISPLAY_NAMES` is on (#7086).
        # With resolution off — the default — the raw id is what the agent sees, and operators who want names without the `users:read` scope set them in `[users]`.
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
    # interactions back to ``on_command``/``on_send``, ``thread`` for
    # threaded replies, and ``reaction`` so the generic AgentPhase
    # lifecycle (Queued → Thinking → ToolUse{name} → Streaming →
    # Done/Error) reaches ``on_command`` as ``reaction`` commands.
    # That one phase stream drives BOTH adapter-side processing indicators: the eyes/white_check_mark receipt (Queued to Done/Error) and the updated-in-place Block Kit task-progress card for multi-step turns (#6451).
    #
    # The receipt used to be driven by the receive/send hooks instead, which is what #6731 fixed: ``_handle_envelope`` added the eyes to every message it emitted, including the ones the daemon then declined to answer (mention-only group gating and the other pre-lifecycle early returns in ``dispatch_message``), leaving a permanent eyes with no way for the adapter to learn the turn was never run.
    # Keying off the lifecycle closes all of those paths structurally: the first adapter-visible signal of a dispatched turn is Queued, so nothing is added unless a turn actually starts, and every started turn reaches Done or Error.
    #
    # Why ``reaction`` and not a new ``task_update`` capability: the
    # generic phase lifecycle is dispatched to adapters through
    # ``ChannelAdapter::send_reaction`` (bridge.rs), which the sidecar
    # trampoline gates on exactly this capability. ``interactive`` gates
    # ``send_interactive`` only and never carries phase events, so
    # reusing it would not deliver the lifecycle. ``reaction`` is already
    # in the negotiation table and already carries every phase, so no new
    # capability is required (see docs/architecture/sidecar-channels.md).
    capabilities: list = ["interactive", "thread", "reaction"]

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
                  placeholder="C0123, C0456"),
            Field("SLACK_UNFURL_LINKS",
                  "Expand link previews in sent messages",
                  "bool",
                  placeholder="true",
                  advanced=True),
            Field("SLACK_FORCE_FLAT_REPLIES",
                  "Post replies as top-level messages instead of threads",
                  "bool",
                  placeholder="false"),
            Field("SLACK_REACTIONS",
                  "Add an eyes reaction while a turn runs and flip it to "
                  "a check / cross when it finishes",
                  "bool",
                  placeholder="true"),
            Field("SLACK_PROGRESS_CARD",
                  "Show the multi-step task-progress card (defaults to "
                  "following SLACK_REACTIONS)",
                  "bool",
                  placeholder="true",
                  advanced=True),
            Field("SLACK_FILE_DOWNLOADS",
                  "Forward user-uploaded files and images to the agent",
                  "bool",
                  placeholder="true"),
            Field("SLACK_FILE_MAX_BYTES",
                  "Maximum inbound attachment size in bytes",
                  "number",
                  placeholder=str(DEFAULT_INBOUND_FILE_MAX_BYTES),
                  advanced=True),
            Field("SLACK_FILE_ALLOWED_EXTENSIONS",
                  "Allowed attachment extensions (comma-separated, empty = "
                  "allow all)",
                  "list",
                  placeholder="png, jpg, pdf, mp4",
                  advanced=True),
            Field("SLACK_FILE_DOWNLOAD_CHANNELS",
                  "Channel IDs that accept attachments (comma-separated, "
                  "empty = every channel)",
                  "list",
                  placeholder="C0123, C0456",
                  advanced=True),
            Field("SLACK_FILE_DOWNLOAD_EXCLUDE_CHANNELS",
                  "Channel IDs that never accept attachments "
                  "(comma-separated)",
                  "list",
                  placeholder="C0789",
                  advanced=True),
            Field("SLACK_RESOLVE_DISPLAY_NAMES",
                  "Resolve sender display names via users.info (needs the "
                  "users:read scope; off by default)",
                  "bool",
                  placeholder="false",
                  advanced=True),
            Field("SLACK_DISPLAY_NAME_TTL",
                  "How long a resolved display name is cached, in seconds",
                  "number",
                  placeholder=str(DEFAULT_DISPLAY_NAME_TTL_SECS),
                  advanced=True),
            Field("SLACK_ENUMERATE_MEMBERS",
                  "List every channel member via conversations.members, not "
                  "only those who have spoken (needs channels:read / "
                  "groups:read; off by default)",
                  "bool",
                  placeholder="false",
                  advanced=True),
            Field("SLACK_MEMBER_LIST_TTL",
                  "How long an enumerated channel member list is cached, in "
                  "seconds",
                  "number",
                  placeholder=str(DEFAULT_MEMBER_LIST_TTL_SECS),
                  advanced=True),
            Field("SLACK_MEMBER_LIST_MAX",
                  "Maximum members enumerated per channel",
                  "number",
                  placeholder=str(DEFAULT_MEMBER_LIST_MAX),
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
        # The task-progress card is a separate indicator from the receipt reaction (#6730): silencing the emoji noise used to silence the card too, which was the only workaround for either.
        # It defaults to whatever `SLACK_REACTIONS` resolved to, so an operator who already runs `SLACK_REACTIONS=false` for total silence keeps it and does not suddenly start receiving cards.
        self.progress_card_enabled = _bool_env(
            os.environ.get("SLACK_PROGRESS_CARD", ""),
            default=self.reactions_enabled,
        )
        acct = os.environ.get("SLACK_ACCOUNT_ID", "").strip()
        self.account_id = acct or None

        # Display-name resolution (#7086).
        #
        # Default OFF, and deliberately so on two counts.
        # It needs the `users:read` scope, which a bot installed before this existed does not have — enabling it by default would turn every inbound message into a logged `missing_scope` failure on upgrade.
        # And it is the point at which the daemon starts learning and persisting real people's names: the roster row the bridge writes carries whatever `user_name` this adapter reports, so turning this on changes what is stored, not just what is displayed.
        # An operator opts in; nobody has personal data resolved on their behalf by an upgrade.
        self.resolve_display_names = _bool_env(
            os.environ.get("SLACK_RESOLVE_DISPLAY_NAMES", ""), default=False,
        )
        ttl_raw = os.environ.get("SLACK_DISPLAY_NAME_TTL", "").strip()
        try:
            display_name_ttl = float(ttl_raw or DEFAULT_DISPLAY_NAME_TTL_SECS)
        except (TypeError, ValueError):
            log.error("SLACK_DISPLAY_NAME_TTL invalid (must be a number of seconds)",
                      value=ttl_raw)
            raise SystemExit(2) from None
        if display_name_ttl <= 0:
            log.warn("SLACK_DISPLAY_NAME_TTL <= 0; using the default instead",
                     requested=display_name_ttl,
                     default=DEFAULT_DISPLAY_NAME_TTL_SECS)
            display_name_ttl = float(DEFAULT_DISPLAY_NAME_TTL_SECS)
        self.display_name_ttl = display_name_ttl
        self._identity_cache = _IdentityCache(
            ttl_secs=display_name_ttl,
            max_entries=MAX_CACHED_IDENTITIES,
        )

        # Bulk member enumeration (#7086).
        #
        # Default OFF, and for a strictly larger version of the reason `SLACK_RESOLVE_DISPLAY_NAMES` is.
        # That knob changes the *quality* of what is stored about the handful of people who have spoken to the agent; this one changes *how many people* are stored at all, from "those who addressed the bot" to "everyone the workspace lists in this channel", most of whom have never interacted with it and did not choose to.
        # It also needs `channels:read` / `groups:read`, which a bot installed before this existed does not have, so enabling it by default would turn every group message into a logged `missing_scope`.
        #
        # The two knobs are independent: enumeration on with resolution off records opaque ids, which is the least-data configuration that still answers "who is in this channel?".
        self.enumerate_members = _bool_env(
            os.environ.get("SLACK_ENUMERATE_MEMBERS", ""), default=False,
        )
        member_ttl_raw = os.environ.get("SLACK_MEMBER_LIST_TTL", "").strip()
        try:
            member_list_ttl = float(member_ttl_raw or DEFAULT_MEMBER_LIST_TTL_SECS)
        except (TypeError, ValueError):
            log.error("SLACK_MEMBER_LIST_TTL invalid (must be a number of seconds)",
                      value=member_ttl_raw)
            raise SystemExit(2) from None
        if member_list_ttl <= 0:
            log.warn("SLACK_MEMBER_LIST_TTL <= 0; using the default instead",
                     requested=member_list_ttl,
                     default=DEFAULT_MEMBER_LIST_TTL_SECS)
            member_list_ttl = float(DEFAULT_MEMBER_LIST_TTL_SECS)
        self.member_list_ttl = member_list_ttl
        member_max_raw = os.environ.get("SLACK_MEMBER_LIST_MAX", "").strip()
        try:
            member_list_max = int(member_max_raw or DEFAULT_MEMBER_LIST_MAX)
        except (TypeError, ValueError):
            log.error("SLACK_MEMBER_LIST_MAX invalid (must be an integer)",
                      value=member_max_raw)
            raise SystemExit(2) from None
        if member_list_max < 1:
            log.warn("SLACK_MEMBER_LIST_MAX < 1; using the default instead",
                     requested=member_list_max,
                     default=DEFAULT_MEMBER_LIST_MAX)
            member_list_max = DEFAULT_MEMBER_LIST_MAX
        self.member_list_max = member_list_max
        self._member_list_cache = _MemberListCache(
            ttl_secs=member_list_ttl,
            max_entries=MAX_CACHED_MEMBER_LISTS,
        )

        # Inbound attachment policy (#7087).
        max_bytes_raw = os.environ.get("SLACK_FILE_MAX_BYTES", "").strip()
        try:
            file_max_bytes = int(max_bytes_raw or DEFAULT_INBOUND_FILE_MAX_BYTES)
        except (TypeError, ValueError):
            log.error("SLACK_FILE_MAX_BYTES invalid (must be an integer)",
                      value=max_bytes_raw)
            raise SystemExit(2) from None
        if file_max_bytes < 1:
            log.warn("SLACK_FILE_MAX_BYTES < 1; using the default instead",
                     requested=file_max_bytes,
                     default=DEFAULT_INBOUND_FILE_MAX_BYTES)
            file_max_bytes = DEFAULT_INBOUND_FILE_MAX_BYTES
        self.file_policy = SlackFilePolicy(
            enabled=_bool_env(
                os.environ.get("SLACK_FILE_DOWNLOADS", ""), default=True,
            ),
            max_bytes=file_max_bytes,
            allowed_extensions=frozenset(
                ext.lstrip(".").lower()
                for ext in _split_csv(
                    os.environ.get("SLACK_FILE_ALLOWED_EXTENSIONS", ""),
                )
                if ext.strip(". ")
            ),
            channels=tuple(_split_csv(
                os.environ.get("SLACK_FILE_DOWNLOAD_CHANNELS", ""),
            )),
            excluded_channels=tuple(_split_csv(
                os.environ.get("SLACK_FILE_DOWNLOAD_EXCLUDE_CHANNELS", ""),
            )),
        )
        # `url_private_download` 302s to a login page without the bot token, so the daemon needs it to fetch what we forward.
        # `header_rules` is the mechanism for that (matrix uses it for MSC3916 media): the daemon exact-matches the request host against these rules and attaches nothing for anything else, so the token cannot follow a member-chosen URL out of the workspace — see `fetch_headers_for` in `crates/librefang-channels/src/sidecar.rs`.
        # The rules are only declared when attachment forwarding is on, so an operator who turns it off does not ship the token at all.
        #
        # Sorted so the ready event is byte-identical across runs.
        if self.file_policy.enabled:
            self.header_rules = [
                (host, [["Authorization", f"Bearer {self.bot_token}"]])
                for host in sorted(SLACK_FILE_HOSTS)
            ]

        self.api_base = DEFAULT_API_BASE
        self.bot_user_id: Optional[str] = None
        # (channel, ts) → emoji name. Cleared when the triggering turn
        # reaches its terminal (done/error) lifecycle phase (#6731).
        # Bounded by `MAX_PENDING_REACTIONS` so a spike of receives
        # without terminations can't grow this without bound.
        self._pending_reactions: dict[tuple[str, str], str] = {}
        self._pending_lock = threading.Lock()
        # Live task-progress cards keyed by (channel_id, triggering ts).
        # Populated from the AgentPhase lifecycle `reaction` commands and
        # rendered as an updated-in-place Block Kit message for
        # multi-step turns (#6451). Touched only from the serialized
        # `on_command` reader path, so no lock is required.
        self._task_progress: dict[tuple[str, str], "_TaskProgress"] = {}

    # Capacity cap on the pending-reaction map. The in-process Rust
    # adapter used an unbounded ``RwLock<HashMap>``; we cap to 2k
    # entries here so a flood of inbound messages followed by a hang
    # in the agent loop doesn't grow the map without bound.
    MAX_PENDING_REACTIONS = 2_000

    # Capacity cap on the task-progress map (same rationale as
    # MAX_PENDING_REACTIONS): a turn that never reaches a terminal phase
    # must not grow the map without bound.
    MAX_TASK_PROGRESS = 1_000

    # Match the channel_send local-file boundary so the sidecar never holds or forwards a larger inline attachment than the producing tool permits.
    MAX_UPLOAD_BYTES = MAX_FILE_UPLOAD_BYTES

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
        retry_429: bool = True,
    ) -> tuple[int, Any, bytes]:
        """Thin wrapper around
        :func:`librefang.sidecar.common.http_request`. Slack's
        callers historically unpack the 3-tuple ``(status, parsed,
        raw)`` form — strip the response-headers dict the shared
        helper returns so existing call sites don't break.

        Honours ``Retry-After`` on 429 and retries the request once
        before surfacing the rate limit to the caller. Slack's tiered
        per-method limits (Tier 1 = 1/min, Tier 4 = 100/min) make
        429s a routine occurrence on burst replies; without an
        in-helper retry the existing ``status >= 300`` arm in
        ``_post_message`` silently drops the chunk."""
        status, parsed, raw, resp_hdrs = _http_request(
            url, method=method, body=body, headers=headers,
            timeout=timeout,
        )
        if status == 429 and retry_429:
            wait = _parse_retry_after(
                resp_hdrs, default_secs=RETRY_AFTER_DEFAULT_SECS,
            )
            log.warn(
                "slack 429; sleeping then retrying once",
                url=url,
                retry_after_secs=wait,
            )
            time.sleep(wait)
            return self._http(
                url, method=method, body=body, headers=headers,
                timeout=timeout, retry_429=False,
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

    def _lookup_identity(self, user_id: str) -> Optional[SlackIdentity]:
        """Resolve one Slack user id to a human-readable identity, at most once per TTL.

        Every exit path writes the cache, including the failures — an unresolvable user must cost one request, not one per message.
        A transient failure gets the short :data:`NEGATIVE_TTL_SECS` cooldown; a definitive answer (resolved, or "no such user") gets the full TTL.
        """
        hit, cached = self._identity_cache.get(user_id)
        if hit:
            return cached

        try:
            status, body, raw = self._http(
                f"{self.api_base}/users.info",
                method="POST",
                body=urllib.parse.urlencode({"user": user_id}).encode("utf-8"),
                headers={
                    **self._auth_headers(),
                    "Content-Type": "application/x-www-form-urlencoded",
                },
            )
        except Exception as e:  # transport failure — cool down briefly, then try again
            log.warn("slack users.info transport error", user=user_id, error=str(e))
            self._identity_cache.put(user_id, None, ttl_secs=NEGATIVE_TTL_SECS)
            return None

        if status != 200 or not isinstance(body, dict):
            snippet = raw[:200].decode("utf-8", "replace") if raw else ""
            log.warn("slack users.info non-200", user=user_id, status=status, body=snippet)
            self._identity_cache.put(user_id, None, ttl_secs=NEGATIVE_TTL_SECS)
            return None

        identity, err = parse_users_identity(body)
        if err is not None:
            # `missing_scope` is the one an operator most needs to see: it means the feature is switched on but the bot was never granted `users:read`, and every message will otherwise silently keep the raw id.
            log.warn("slack users.info rejected", user=user_id, error=err)
            self._identity_cache.put(user_id, None, ttl_secs=NEGATIVE_TTL_SECS)
            return None

        self._identity_cache.put(user_id, identity)
        return identity

    def _apply_identity(self, ev: Optional[dict]) -> Optional[dict]:
        """Replace an inbound event's placeholder ``user_name`` (the raw ``U…`` id) with a resolved display name.

        A no-op unless the operator opted in, and a no-op again whenever the lookup yields nothing — the raw id is a worse label than a real name but a better one than an empty string, and it is what every pre-#7086 deployment already shows.

        The resolved handle is stamped into ``sender_username`` as well, because the same request already answered for it and the roster has a column waiting for it.
        """
        if not self.resolve_display_names or not isinstance(ev, dict):
            return ev
        params = ev.get("params")
        if not isinstance(params, dict):
            return ev
        metadata = params.get("metadata")
        user_id = metadata.get("sender_user_id") if isinstance(metadata, dict) else None
        if not isinstance(user_id, str) or not user_id:
            return ev
        identity = self._lookup_identity(user_id)
        if identity is None:
            return ev
        if identity.display_name:
            params["user_name"] = identity.display_name
        if identity.username:
            metadata["sender_username"] = identity.username
        return ev

    def _lookup_channel_members(self, channel_id: str) -> tuple[str, ...]:
        """Walk ``conversations.members`` for one channel, at most once per TTL.

        Paginates until Slack stops handing back a cursor or :attr:`member_list_max` is reached, whichever comes first.
        Every exit path writes the cache, failures included: a channel the bot cannot read must cost one sweep per TTL, not one per message.
        A partial result (the cap, or a page that failed mid-walk) is cached and used — an incomplete answer to "who is in this channel?" is still an answer, and re-walking on the next message would only spend more of the same rate limit.
        """
        hit, cached = self._member_list_cache.get(channel_id)
        if hit:
            return cached

        collected: list[str] = []
        cursor: Optional[str] = None
        while True:
            params = {
                "channel": channel_id,
                "limit": str(min(MEMBER_LIST_PAGE_SIZE, self.member_list_max)),
            }
            if cursor:
                params["cursor"] = cursor
            try:
                status, body, raw = self._http(
                    f"{self.api_base}/conversations.members",
                    method="POST",
                    body=urllib.parse.urlencode(params).encode("utf-8"),
                    headers={
                        **self._auth_headers(),
                        "Content-Type": "application/x-www-form-urlencoded",
                    },
                )
            except Exception as e:  # transport failure — keep what we have, retry after the cooldown
                log.warn("slack conversations.members transport error",
                         channel=channel_id, error=str(e))
                break

            if status != 200 or not isinstance(body, dict):
                snippet = raw[:200].decode("utf-8", "replace") if raw else ""
                log.warn("slack conversations.members non-200",
                         channel=channel_id, status=status, body=snippet)
                break

            page, cursor, err = parse_conversations_members(body)
            if err is not None:
                # `missing_scope` is the one an operator most needs to see: enumeration is switched on but the bot was never granted `channels:read` / `groups:read`, and every channel will otherwise silently report only the people who have spoken.
                log.warn("slack conversations.members rejected",
                         channel=channel_id, error=err)
                break
            collected.extend(page)
            if len(collected) >= self.member_list_max:
                log.warn("slack channel member list truncated at the configured cap",
                         channel=channel_id, cap=self.member_list_max)
                collected = collected[:self.member_list_max]
                break
            if cursor is None:
                break

        # Sorted so the metadata the daemon receives is byte-identical across sweeps that return the same people in a different page order.
        # The list reaches an LLM prompt through `channel_members`, where an unstable order invalidates the provider prompt cache on unchanged content (#3298).
        members = tuple(sorted(set(collected)))
        ttl = None if members else NEGATIVE_TTL_SECS
        self._member_list_cache.put(channel_id, members, ttl_secs=ttl)
        return members

    def _apply_enumerated_members(self, ev: Optional[dict]) -> Optional[dict]:
        """Stamp the channel's full member list onto an inbound group event as ``group_members`` metadata (#7086).

        This is the bulk half of the roster the reporter asked for: ``channel_members`` could only ever report people who had spoken, because the per-sender upsert was the only thing writing rows.

        The daemon stores what lands here as ``source = 'enumerated'``, deliberately apart from the observational rows, so ``channel_dm`` still refuses anyone who has never addressed the agent.
        Widening the DM authorization set is the failure mode this whole shape exists to avoid, and it would be invisible from here — the adapter cannot tell what the daemon does with the key, which is exactly why the split lives on the daemon side and not in this file.

        Display names come from the identity cache only; **no ``users.info`` call is made on this path**.
        A sweep is bulk by nature, and resolving 500 members would spend the workspace's entire per-method budget to name people who have never spoken.
        Anyone who has spoken is already cached by :meth:`_apply_identity`, so in practice the people an agent can act on carry names and the rest carry ids until they say something.
        """
        if not self.enumerate_members or not isinstance(ev, dict):
            return ev
        params = ev.get("params")
        if not isinstance(params, dict) or params.get("is_group") is not True:
            return ev
        channel_id = params.get("user_id")
        if not isinstance(channel_id, str) or not channel_id:
            return ev
        member_ids = self._lookup_channel_members(channel_id)
        if not member_ids:
            return ev
        metadata = params.get("metadata")
        if not isinstance(metadata, dict):
            metadata = {}
            params["metadata"] = metadata

        members: list[dict] = []
        for member_id in member_ids:
            entry: dict[str, Any] = {"user_id": member_id, "display_name": member_id}
            _, identity = self._identity_cache.get(member_id)
            if identity is not None:
                if identity.display_name:
                    entry["display_name"] = identity.display_name
                if identity.username:
                    entry["username"] = identity.username
            members.append(entry)
        metadata["group_members"] = members
        return ev

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

    def _fetch_file_url(self, url: str) -> bytes:
        """Download one public file URL with DNS-pinned redirect and size guards."""
        status, data = _public_http_request(
            url,
            method="GET",
            headers={"User-Agent": "librefang-slack-sidecar/1 (https://librefang.org)"},
            max_bytes=self.MAX_UPLOAD_BYTES,
        )
        if status >= 300:
            raise RuntimeError(f"file download failed (status={status})")
        return data

    def _upload_file_bytes(
        self,
        channel_id: str,
        data: bytes,
        filename: str,
        *,
        thread_ts: Optional[str] = None,
    ) -> bool:
        """Upload bytes with Slack's external-upload flow and share them once."""
        filename = _safe_filename(filename)
        if not data:
            log.warn("slack file upload refused empty payload", filename=filename)
            return False
        if len(data) > self.MAX_UPLOAD_BYTES:
            log.warn(
                "slack file upload exceeds size cap",
                filename=filename,
                size=len(data),
                max_bytes=self.MAX_UPLOAD_BYTES,
            )
            return False

        ticket_body = json.dumps({"filename": filename, "length": len(data)}).encode("utf-8")
        status, ticket, raw = self._http(
            f"{self.api_base}/files.getUploadURLExternal",
            method="POST",
            body=ticket_body,
            headers=self._auth_headers(content_type=True),
        )
        if status >= 300 or not isinstance(ticket, dict) or ticket.get("ok") is not True:
            error = ticket.get("error") if isinstance(ticket, dict) else raw[:200].decode("utf-8", "replace")
            log.warn("slack files.getUploadURLExternal failed", status=status, error=error or "unknown")
            return False
        upload_url = ticket.get("upload_url")
        file_id = ticket.get("file_id")
        if not isinstance(upload_url, str) or not isinstance(file_id, str) or not upload_url or not file_id:
            log.warn("slack upload ticket missing URL or file id")
            return False
        try:
            upload_status, upload_response = _public_http_request(
                upload_url,
                method="POST",
                body=data,
                headers={"Content-Type": "application/octet-stream"},
                max_bytes=200,
                require_https=True,
            )
        except RuntimeError as e:
            log.warn("slack file byte upload failed", error=str(e))
            return False
        if upload_status >= 300:
            log.warn(
                "slack file byte upload failed",
                status=upload_status,
                body=upload_response.decode("utf-8", "replace"),
            )
            return False

        complete_payload: dict[str, Any] = {
            "files": [{"id": file_id, "title": filename}],
            "channel_id": channel_id,
        }
        if thread_ts:
            complete_payload["thread_ts"] = thread_ts
        complete_body = json.dumps(complete_payload).encode("utf-8")
        status, complete, raw = self._http(
            f"{self.api_base}/files.completeUploadExternal",
            method="POST",
            body=complete_body,
            headers=self._auth_headers(content_type=True),
        )
        if status >= 300 or not isinstance(complete, dict) or complete.get("ok") is not True:
            error = complete.get("error") if isinstance(complete, dict) else raw[:200].decode("utf-8", "replace")
            log.warn("slack files.completeUploadExternal failed", status=status, error=error or "unknown")
            return False
        return True

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
            _split_message(text, SLACK_MSG_LIMIT)
            if blocks is None
            # With blocks, `text` is only the notification-preview fallback (the blocks carry the rendered content, already split to fit per-section), so chunking it into several messages would post the same blocks repeatedly.
            # Bound it instead — an unbounded notification string buys nothing and is one more length the API can reject.
            else [text[:SLACK_MSG_LIMIT]]
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

    def _post_blocks(
        self,
        channel_id: str,
        text: str,
        blocks: list,
        *,
        thread_ts: Optional[str] = None,
    ) -> Optional[str]:
        """POST a single Block Kit message (no chunking) and return its
        ``ts`` so the caller can later ``chat.update`` it in place.
        Returns ``None`` on any transport/API failure — the task-progress
        card is best-effort UX and must never break the reply path."""
        payload: dict[str, Any] = {
            "channel": channel_id,
            "text": text,
            "blocks": blocks,
        }
        if thread_ts:
            payload["thread_ts"] = thread_ts
        if self.unfurl_links is not None:
            payload["unfurl_links"] = self.unfurl_links
        body = json.dumps(payload).encode("utf-8")
        status, resp, raw = self._http(
            f"{self.api_base}/chat.postMessage",
            method="POST",
            body=body,
            headers=self._auth_headers(content_type=True),
        )
        if status >= 300:
            snippet = raw[:200].decode("utf-8", "replace") if raw else ""
            log.warn("slack task-progress post transport error",
                     status=status, body=snippet)
            return None
        if isinstance(resp, dict):
            if resp.get("ok") is not True:
                log.warn("slack task-progress post rejected",
                         error=resp.get("error") or "unknown")
                return None
            ts = resp.get("ts")
            return ts if isinstance(ts, str) and ts else None
        return None

    def _update_blocks(
        self, channel_id: str, ts: str, text: str, blocks: list,
    ) -> None:
        """``chat.update`` an existing Block Kit message in place.
        Best-effort: a failure just leaves the card at its prior state."""
        payload = {
            "channel": channel_id,
            "ts": ts,
            "text": text,
            "blocks": blocks,
        }
        body = json.dumps(payload).encode("utf-8")
        status, resp, raw = self._http(
            f"{self.api_base}/chat.update",
            method="POST",
            body=body,
            headers=self._auth_headers(content_type=True),
        )
        if status >= 300:
            snippet = raw[:200].decode("utf-8", "replace") if raw else ""
            log.warn("slack task-progress update transport error",
                     status=status, body=snippet)
            return
        if isinstance(resp, dict) and resp.get("ok") is not True:
            log.warn("slack task-progress update rejected",
                     error=resp.get("error") or "unknown")

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
        """Record that we added an ``emoji`` reaction on ``channel/ts`` so :meth:`_finalize_pending_reaction` can flip it to the terminal emoji once the turn reaches its Done / Error phase."""
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
        self, channel: str, ts: Optional[str], emoji: Optional[str],
    ) -> None:
        """Remove the in-progress receipt on ``channel``/``ts`` and put ``emoji`` in its place.
        ``emoji=None`` removes only — that is the daemon's ``clear_done_reaction`` signal, which arrives as an empty emoji on the terminal phase.

        Keyed strictly by ``(channel, ts)``.
        There is deliberately no "first pending entry in this channel" fallback: the lifecycle always carries the exact triggering ``message_id``, and the old fallback flipped an unrelated sibling message's receipt whenever the exact key missed (which it did for every in-thread reply, because the send hook keyed off the thread root instead of the message's own ts).

        A miss is therefore a no-op, which also makes a repeated terminal phase idempotent: the first one pops the entry, the second finds nothing and adds nothing.
        """
        if not self.reactions_enabled or not ts:
            return
        with self._pending_lock:
            pending = self._pending_reactions.pop((channel, ts), None)
        if pending is None:
            return
        self._remove_reaction(channel, ts, pending)
        if emoji:
            self._add_reaction(channel, ts, emoji)

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
                file_policy=self.file_policy,
            )
            if ev is None:
                return
            ev = self._apply_enumerated_members(self._apply_identity(ev))
            # No reaction here (#6731). Receiving a message is not the same as answering it: the daemon may decline the turn for any of ~two dozen reasons (mention-only group gating, an `[allowed_channels]`-adjacent RBAC denial, a per-user rate limit, a slash command it handles itself), all of which return before any adapter-visible lifecycle signal.
            # The receipt is added from the `queued` phase in `_on_phase` instead, which fires only for a turn that is actually run.
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
                emit(self._apply_enumerated_members(self._apply_identity(ev)))
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
        # The inbound thread id (post-#5302 this is the message's own ts for a top-level message, or the thread root for an in-thread reply).
        # Used to decide where to post, nothing else — reaction finalization moved onto the lifecycle stream in #6731, keyed by the triggering message's own ts rather than this.
        inbound_thread_id = getattr(cmd, "thread_id", None)
        # Decide thread context for *posting*: force-flat-replies mode
        # forces the reply to a top-level post (mirrors the Rust adapter's
        # force_flat_replies knob); otherwise reply in the inbound thread.
        thread_ts = None if self.force_flat_replies else inbound_thread_id

        content = cmd.content
        # Sidecars own outbound formatting (owns_formatting()=true), so the kernel hands us raw Markdown — convert to Slack mrkdwn here.
        text = _markdown_to_mrkdwn(cmd.text or "")
        loop = asyncio.get_event_loop()
        if isinstance(content, dict) and "Text" in content:
            await loop.run_in_executor(
                None,
                lambda: self._post_message(channel_id, text, thread_ts=thread_ts),
            )
        elif isinstance(content, dict) and "Interactive" in content:
            payload = content["Interactive"]
            interactive_text = _markdown_to_mrkdwn(payload.get("text", "")) or text
            buttons = payload.get("buttons", []) or []
            blocks = _build_block_kit(interactive_text, buttons)
            await loop.run_in_executor(
                None,
                lambda: self._post_message(
                    channel_id, interactive_text,
                    thread_ts=thread_ts, blocks=blocks,
                ),
            )
        elif isinstance(content, dict) and "FileData" in content:
            payload = content["FileData"]

            def _send_file_data() -> None:
                if not isinstance(payload, dict):
                    log.warn("slack FileData payload is not an object")
                    return
                data = _coerce_file_data(payload.get("data"))
                if data is None:
                    log.warn("slack FileData bytes are malformed")
                    return
                self._upload_file_bytes(
                    channel_id,
                    data,
                    _safe_filename(payload.get("filename")),
                    thread_ts=thread_ts,
                )

            await loop.run_in_executor(None, _send_file_data)
        elif isinstance(content, dict) and "File" in content:
            payload = content["File"]

            def _send_file_url() -> None:
                if not isinstance(payload, dict):
                    log.warn("slack File payload is not an object")
                    return
                url = payload.get("url")
                if not isinstance(url, str) or not url:
                    log.warn("slack File URL is missing")
                    return
                filename = _safe_filename(payload.get("filename"))
                try:
                    data = self._fetch_file_url(url)
                except RuntimeError as e:
                    log.warn("slack File URL download failed", error=str(e))
                    return
                self._upload_file_bytes(
                    channel_id,
                    data,
                    filename,
                    thread_ts=thread_ts,
                )

            await loop.run_in_executor(None, _send_file_url)
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

    async def on_command(self, cmd) -> None:
        """Route lifecycle ``reaction`` commands to the receipt reaction and task-progress display; defer everything else (``send`` → ``on_send``) to the base dispatcher."""
        if isinstance(cmd, protocol.Reaction):
            await self._on_phase(cmd)
            return
        await super().on_command(cmd)

    async def _on_phase(self, cmd) -> None:
        """Fold one AgentPhase lifecycle ``reaction`` into the receipt reaction and, for multi-step turns, a live Block Kit task-progress card.

        Two independent indicators, one stream:

        * **Receipt** (``SLACK_REACTIONS``): ``queued`` adds the eyes to the triggering message; ``done`` / ``error`` flip it to white_check_mark / x.
          Handled BEFORE the card bookkeeping below, because a single-step turn has no card state and would otherwise hit the early-out and never finalize — leaving exactly the stuck eyes #6731 is about.
        * **Card** (``SLACK_PROGRESS_CARD``): materialized lazily, only once a turn actually runs a tool (a genuinely multi-step turn).
          Single-step turns (Queued → Thinking → Done with no ToolUse) post nothing and keep just the receipt.

        Called only from the serialized ``on_command`` reader path, so the state map needs no lock; the Slack Web API calls are offloaded to the executor.
        """
        if not self.reactions_enabled and not self.progress_card_enabled:
            return
        phase = (getattr(cmd, "phase", "") or "").strip()
        if not phase:
            # Legacy emoji-only reaction (pre-#6451 daemon): no structured
            # phase, so neither indicator has anything to key off. Ignore.
            return
        channel_id = cmd.channel_id
        message_id = cmd.message_id
        if not channel_id or not message_id:
            return
        key = (channel_id, message_id)
        terminal = phase in ("done", "error")
        loop = asyncio.get_event_loop()

        # ---- receipt reaction (#6731) --------------------------------
        if self.reactions_enabled and (phase == "queued" or terminal):
            if phase == "queued":
                self._track_pending_reaction(channel_id, message_id, "eyes")
                await loop.run_in_executor(
                    None,
                    lambda: self._add_reaction(channel_id, message_id, "eyes"),
                )
            else:
                # An empty emoji on the wire is the daemon's `clear_done_reaction` signal ("remove, add nothing").
                # Every other terminal frame carries one, so only Done can be empty — see `lifecycle_reaction_emoji` in bridge.rs.
                wire_emoji = getattr(cmd, "reaction", "") or ""
                if phase == "error":
                    terminal_emoji: Optional[str] = "x"
                elif wire_emoji:
                    terminal_emoji = "white_check_mark"
                else:
                    terminal_emoji = None
                await loop.run_in_executor(
                    None,
                    lambda: self._finalize_pending_reaction(
                        channel_id, message_id, terminal_emoji,
                    ),
                )
        if phase == "queued":
            # `queued` is never rendered in the card, so there is nothing left to do — and materializing card state for it would make every turn look multi-step.
            return
        if not self.progress_card_enabled:
            return

        # ---- task-progress card (#6451) ------------------------------
        prog = self._task_progress.get(key)
        if prog is None:
            if terminal:
                # Terminal phase with no tracked task = a single-step turn
                # (no tool ran, so no card was ever posted). Nothing to
                # finalize.
                return
            prog = _TaskProgress()
            # Bounded insert: evict the oldest entry (insertion-ordered
            # dict) so a turn that never terminates can't grow the map.
            if len(self._task_progress) >= self.MAX_TASK_PROGRESS:
                try:
                    del self._task_progress[next(iter(self._task_progress))]
                except StopIteration:
                    pass
            self._task_progress[key] = prog

        # Record the step. `queued` returned above and is never shown.
        if phase == "tool_use":
            prog.steps.append(("tool_use", getattr(cmd, "tool_name", None)))
        elif phase in ("thinking", "streaming"):
            # Collapse consecutive duplicates so repeated thinking /
            # streaming phases don't spam the list.
            if not prog.steps or prog.steps[-1][0] != phase:
                prog.steps.append((phase, None))

        # Materialize the card only for multi-step turns (a tool ran), or
        # keep updating one already posted.
        has_tool = any(s[0] == "tool_use" for s in prog.steps)
        if not has_tool and prog.card_ts is None:
            if terminal:
                self._task_progress.pop(key, None)
            return

        text, blocks = _build_task_progress_blocks(prog.steps, phase)
        if prog.card_ts is None:
            thread_ts = None if self.force_flat_replies else message_id
            ts = await loop.run_in_executor(
                None,
                lambda: self._post_blocks(
                    channel_id, text, blocks, thread_ts=thread_ts,
                ),
            )
            prog.card_ts = ts
        else:
            card_ts = prog.card_ts
            await loop.run_in_executor(
                None,
                lambda: self._update_blocks(channel_id, card_ts, text, blocks),
            )

        if terminal:
            self._task_progress.pop(key, None)


class _TaskProgress:
    """In-flight task-progress state for one agent turn."""

    __slots__ = ("steps", "card_ts")

    def __init__(self) -> None:
        # Ordered list of (phase, tool_name) steps. `tool_name` is set
        # only for `tool_use` steps.
        self.steps: list[tuple[str, Optional[str]]] = []
        # Slack `ts` of the posted progress card; None until first post.
        self.card_ts: Optional[str] = None


# Per-phase step icon for the live (in-progress) step in the card.
_PHASE_STEP_ICON = {
    "thinking": "\U0001f914",   # 🤔
    "tool_use": "⚙️",  # ⚙️
    "streaming": "✍️",  # ✍️
}


def _phase_step_label(phase: str, tool_name: Optional[str]) -> str:
    if phase == "tool_use":
        return f"`{tool_name}`" if tool_name else "Running a tool"
    if phase == "thinking":
        return "Thinking"
    if phase == "streaming":
        return "Writing response"
    return phase


def _build_task_progress_blocks(
    steps: list, current_phase: str,
) -> tuple[str, list]:
    """Render the ordered step list into a single-section Block Kit card.

    Returns ``(text, blocks)`` — ``text`` is the mrkdwn fallback Slack
    shows in notifications, ``blocks`` the rich rendering. Completed
    steps carry a ``✓``; the active (last, non-terminal) step carries its
    phase icon so the user sees which step is running right now.
    """
    if current_phase == "error":
        header = "*❌ Task failed*"        # ❌
    elif current_phase == "done":
        header = "*✅ Task complete*"      # ✅
    else:
        header = "*⏳ Working…*"       # ⏳ Working…
    terminal = current_phase in ("done", "error")
    lines = [header]
    n = len(steps)
    for i, (phase, tool_name) in enumerate(steps):
        active = (not terminal) and (i == n - 1)
        icon = _PHASE_STEP_ICON.get(phase, "•") if active else "✓"  # ✓
        lines.append(f"{icon} {_phase_step_label(phase, tool_name)}")
    text = "\n".join(lines)
    # Slack rejects a section whose text exceeds SLACK_MSG_LIMIT (3000) chars,
    # which would freeze the progress card on a long or looping turn. Keep the
    # header plus the most recent steps under the limit, collapsing older steps
    # into a single summary line (#6451 review).
    if len(text) > SLACK_MSG_LIMIT:
        header_line = lines[0]
        step_lines = lines[1:]
        budget = SLACK_MSG_LIMIT - len(header_line) - 40
        kept: list[str] = []
        for line in reversed(step_lines):
            if sum(len(x) + 1 for x in kept) + len(line) + 1 > budget:
                break
            kept.append(line)
        kept.reverse()
        dropped = len(step_lines) - len(kept)
        summary = [header_line]
        if dropped > 0:
            summary.append(f"… {dropped} earlier step{'s' if dropped != 1 else ''}")
        summary.extend(kept)
        text = "\n".join(summary)
    blocks = [{
        "type": "section",
        "text": {"type": "mrkdwn", "text": text},
    }]
    return text, blocks


# Matches one code span — fenced ```...```, an UNCLOSED ``` running to the end of the message, or inline `...`.
# The converter masks every match with an index token before any other rule runs, then restores the spans at the end, so a bold/link span or header/bullet line that wraps around a code span is still seen whole by the regexes (the previous top-level split hid everything past the code delimiter from them).
# The unclosed alternative is second so a closed fence always wins; it matters because a truncated model response ends mid-block, and Slack renders an unterminated ``` as code all the way to the end of the message. Without it the blank-line collapse would rewrite the interior of something the user sees as a code block.
# Deliberately NOT covered: `~~~` fences and 4-space-indented blocks. Both are GitHub-flavoured Markdown that Slack's mrkdwn does not render as code, so their blank lines are ordinary prose whitespace and collapsing them is exactly what #6730 asks for.
_CODE_SPAN_RE = re.compile(r"```.*?```|```.*|`[^`]*`", re.DOTALL)
# An index token standing in for a masked code span: \x00<index>\x01.
# Control-character delimiters guarantee no Markdown rule can match into or across a token.
_CODE_TOKEN_RE = re.compile("\x00(\\d+)\x01")
_ATX_HEADER_RE = re.compile(r"^(\s{0,3})#{1,6}\s+(.*?)\s*#*\s*$")
_BULLET_RE = re.compile(r"^(\s*)[-*+]\s+(.*)$")
_MD_LINK_RE = re.compile(r"\[([^\]]+)\]\(([^)\s]+)\)")
_BOLD_STAR_RE = re.compile(r"\*\*([^*\n]+)\*\*")
_BOLD_USCORE_RE = re.compile(r"__([^_\n]+)__")
_STRIKE_RE = re.compile(r"~~([^~\n]+)~~")
# A run of three or more newlines, i.e. two or more consecutive blank lines.
# Collapsed to a single blank line (Slack's paragraph separator) so a model that pads its answer with blank lines does not turn a short reply into a wall of whitespace (#6730).
# Applied to the code-MASKED string, so a fenced block's interior — already a single token by then — is untouchable.
_BLANK_LINE_RUN_RE = re.compile(r"\n{3,}")
# A GitHub-flavoured-Markdown table divider row: |---|:--:|---:| etc.
# Requires ≥2 columns (≥1 internal pipe) so a lone `---` horizontal rule is not mistaken for a table.
_TABLE_DIVIDER_RE = re.compile(r"^\s*\|?\s*:?-{1,}:?\s*(\|\s*:?-{1,}:?\s*)+\|?\s*$")


def _restore_code_spans(s: str, codes: list[str], strip_backticks: bool = False) -> str:
    """Replace \\x00<i>\\x01 tokens with the masked code span at index i.

    Every token is one the converter minted itself (`_markdown_to_mrkdwn` drops any pre-existing delimiter bytes), so the index is always in range.
    ``strip_backticks`` is for table cells, which land inside a monospace code block where literal backtick delimiters would just be noise.
    """

    def repl(m: "re.Match[str]") -> str:
        code = codes[int(m.group(1))]
        return code.replace("`", "") if strip_backticks else code

    return _CODE_TOKEN_RE.sub(repl, s)


def _inline_md(s: str) -> str:
    """Inline Markdown → Slack mrkdwn (links, bold, strikethrough)."""
    s = _MD_LINK_RE.sub(r"<\2|\1>", s)      # [text](url) → <url|text>
    s = _BOLD_STAR_RE.sub(r"*\1*", s)        # **bold** → *bold*
    s = _BOLD_USCORE_RE.sub(r"*\1*", s)      # __bold__ → *bold*
    s = _STRIKE_RE.sub(r"~\1~", s)           # ~~strike~~ → ~strike~
    return s


def _split_table_row(row: str) -> list[str]:
    r = row.strip()
    if r.startswith("|"):
        r = r[1:]
    if r.endswith("|"):
        r = r[:-1]
    return [c.strip() for c in r.split("|")]


def _plain_cell(c: str, codes: list[str]) -> str:
    """Strip inline markers from a cell so the monospace grid stays aligned."""
    c = _restore_code_spans(c, codes, strip_backticks=True)
    c = _MD_LINK_RE.sub(r"\1", c)  # [text](url) → text
    return c.replace("**", "").replace("__", "").replace("`", "").strip()


def _render_table(header: str, rows: list[str], codes: list[str]) -> str:
    """Render a Markdown table as a fixed-width Slack code block (Slack mrkdwn has no table element; monospace preserves the columns).

    Cells may contain code-span tokens; they are restored (backticks stripped) before widths are computed so the grid aligns on the real cell text.
    """
    hcells = [_plain_cell(c, codes) for c in _split_table_row(header)]
    rcells = [[_plain_cell(c, codes) for c in _split_table_row(r)] for r in rows]
    ncol = max([len(hcells)] + [len(r) for r in rcells])

    def pad(cells: list[str]) -> list[str]:
        return cells + [""] * (ncol - len(cells))

    hcells = pad(hcells)
    rcells = [pad(r) for r in rcells]
    widths = []
    for k in range(ncol):
        w = len(hcells[k])
        for r in rcells:
            w = max(w, len(r[k]))
        widths.append(w)

    def fmt(cells: list[str]) -> str:
        return " | ".join(cells[k].ljust(widths[k]) for k in range(ncol))

    sep = "-+-".join("-" * widths[k] for k in range(ncol))
    body = "\n".join([fmt(hcells), sep] + [fmt(r) for r in rcells])
    return f"```\n{body}\n```"


def _convert_md_lines(masked: str, codes: list[str]) -> str:
    """Convert code-masked Markdown to Slack mrkdwn (tokens still in place)."""
    lines = masked.split("\n")
    out: list[str] = []
    i = 0
    n = len(lines)
    while i < n:
        line = lines[i]
        # Tables: a header row followed by a |---|---| divider.
        # Rendered as a fixed-width code block since Slack has no table support.
        if "|" in line and i + 1 < n and _TABLE_DIVIDER_RE.match(lines[i + 1]):
            j = i + 2
            body_rows: list[str] = []
            while j < n and lines[j].strip() and "|" in lines[j]:
                body_rows.append(lines[j])
                j += 1
            out.append(_render_table(line, body_rows, codes))
            i = j
            continue
        # ATX headers (# .. ######) → bold (Slack has no headings).
        m = _ATX_HEADER_RE.match(line)
        if m:
            content = m.group(2).strip()
            out.append(f"{m.group(1)}*{_inline_md(content)}*" if content else "")
            i += 1
            continue
        # Bullets (-, *, +) → •. Also stops a "* item" bullet from being mistaken for italic.
        b = _BULLET_RE.match(line)
        if b:
            out.append(f"{b.group(1)}•   {_inline_md(b.group(2))}")
            i += 1
            continue
        out.append(_inline_md(line))
        i += 1
    return "\n".join(out)


def _markdown_to_mrkdwn(text: str) -> str:
    """Convert common Markdown to Slack mrkdwn, leaving code spans intact.

    The Rust `formatter::markdown_to_slack_mrkdwn` used to run kernel-side, but sidecar adapters report `owns_formatting() = true`, so the kernel now sends raw text and the adapter owns the conversion.
    Handles bold, headers, bullets, links, and strikethrough — the cases that otherwise leak literal `**` / `##` into Slack.
    Italic and blockquotes already share syntax with Slack (`_x_`, `> q`) and pass through untouched; fenced/inline code is preserved verbatim (Slack renders backticks).

    Code spans are masked with index tokens up front and restored after all other rules have run, so an inline construct wrapping a code span ("**the `foo()` method**") converts as one whole span instead of leaking its markers.
    """
    if not text:
        return text
    # The delimiter bytes cannot legitimately occur in chat text; dropping them up front means a pathological input can never alias a real token.
    text = text.replace("\x00", "").replace("\x01", "")
    codes: list[str] = []

    def mask(m: "re.Match[str]") -> str:
        codes.append(m.group(0))
        return f"\x00{len(codes) - 1}\x01"

    masked = _CODE_SPAN_RE.sub(mask, text)
    converted = _convert_md_lines(masked, codes)
    # Collapse blank-line runs while the code spans are still masked — a fenced block is one token at this point, so its interior blank lines cannot be touched.
    # `_convert_md_lines` is a 1:1 line mapper (a content-less header even emits an extra empty line), so without this an `\n\n\n\n` run reached Slack verbatim.
    converted = _BLANK_LINE_RUN_RE.sub("\n\n", converted)
    return _restore_code_spans(converted, codes)


def _build_block_kit(text: str, buttons: list) -> list:
    """Render a ``Content.interactive`` payload into Slack Block Kit
    blocks. Mirrors the Rust adapter's `api_send_interactive_message`
    layout: ``section`` block(s) with the text (mrkdwn), then one
    ``actions`` block per row of buttons.

    The text is split across as many sections as it needs (#6730).
    Slack rejects a ``section`` whose text exceeds ``SLACK_MSG_LIMIT``, and ``_post_message`` deliberately skips its chunking when ``blocks`` is present, so a long interactive reply used to be rejected wholesale and dropped with nothing but a log line — the same limit ``_build_task_progress_blocks`` already guards for.

    Buttons are built first and budgeted against Slack's 50-blocks-per-message cap, because they are the functional payload: truncating text degrades a reply, dropping buttons breaks the interaction.
    """
    action_blocks: list = []
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
            action_blocks.append({
                "type": "actions",
                "block_id": f"interactive_row_{row_idx}",
                "elements": elements,
            })

    # `_split_message` cuts on newlines where it can and provably yields chunks <= the limit; an empty string comes back as [""], preserving the historical single-empty-section shape.
    chunks = _split_message(text, SLACK_MSG_LIMIT)

    # The marker costs a block, so its slot is reserved only once truncation is actually happening.
    # Reserving it up front spends a slot that the content may not need: with 49 button rows and one chunk, one section plus 49 actions is exactly 50 blocks and fits, but an unconditional reservation left zero sections and replaced the text with the marker.
    # `/agents` and `/models` build one row per agent / per provider with no cap, so that is reachable rather than theoretical.
    #
    # When the rows alone reach the cap the message cannot carry text at all, and the rows themselves have to give — Slack rejects an over-cap message outright, so "buttons are never dropped" cannot mean emitting 51 blocks; that delivers nothing at all, buttons included.
    truncated = False
    if len(action_blocks) >= MAX_BLOCKS_PER_MESSAGE:
        action_blocks = action_blocks[: MAX_BLOCKS_PER_MESSAGE - 1]
        chunks = []
        truncated = True
    else:
        section_budget = MAX_BLOCKS_PER_MESSAGE - len(action_blocks)
        if len(chunks) > section_budget:
            truncated = True
            # Now — and only now — the marker takes one of the slots.
            chunks = chunks[: max(0, section_budget - 1)]
    blocks: list = [
        {"type": "section", "text": {"type": "mrkdwn", "text": chunk}}
        for chunk in chunks
    ]
    if truncated:
        # Its own section, so the marker can never push a chunk over the
        # per-section character limit.
        blocks.append({
            "type": "section",
            "text": {"type": "mrkdwn", "text": "_(message truncated)_"},
        })
    blocks.extend(action_blocks)
    return blocks


if __name__ == "__main__":
    run_stdio_main(SlackAdapter)
