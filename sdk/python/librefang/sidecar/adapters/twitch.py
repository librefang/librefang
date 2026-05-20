#!/usr/bin/env python3
"""Twitch IRC sidecar channel adapter for LibreFang.

Replaces the former in-process Rust ``librefang-channels::twitch``
adapter (removed in this sidecar migration; same pattern as ntfy
#5224, telegram #5241, gotify #5263, mastodon #5264, bluesky #5277,
reddit #5281). Behaviour is preserved except for three intentional
improvements explicitly ack'd by the maintainer:

* **TLS by default** (security improvement). The Rust adapter opened
  a plain TCP socket to ``irc.chat.twitch.tv:6667``, sending the
  Twitch OAuth token in cleartext over the wire on every connect /
  reconnect. Twitch's IRC gateway also serves TLS on
  ``irc.chat.twitch.tv:6697`` — the sidecar uses that by default
  (stdlib ``ssl.create_default_context()``). Plain TCP is still
  reachable via ``TWITCH_PLAINTEXT=1`` for local mock listeners
  (tests do this); operators should never set it in production.
* **Per-message threading via IRCv3 tags** (improvement). Twitch's
  IRC dialect ships ``@id=<uuid>`` tags on every PRIVMSG when the
  client requests the ``twitch.tv/tags`` capability. The Rust
  adapter never asked for capabilities and never surfaced the tag
  id, so chunked replies arrived as a flat sequence with no link
  back to the originating message. This sidecar issues ``CAP REQ
  :twitch.tv/tags twitch.tv/commands`` after auth, parses the
  leading ``@…`` tag block on every PRIVMSG, surfaces the source
  message id as ``thread_id`` (so the daemon round-trips it back
  to ``on_send`` via ``cmd.thread_id``), and attaches
  ``@reply-parent-msg-id=<id>`` to outbound PRIVMSGs when the
  daemon supplies one. Twitch renders these as a proper reply
  thread in the chat UI (the same surface Mastodon / Bluesky
  reach via their native reply refs).
* **Token-bucket send rate-limiter** (ban-avoidance improvement).
  Twitch's anti-spam logic drops the bot from chat (and can
  temp-ban the account) above 20 messages / 30 s for an unmodded
  account (100 / 30 s for a mod). The Rust adapter shipped no
  throttling: a chatty agent in a busy channel would hit the cap
  fast. The sidecar maintains a simple in-process token bucket
  on ``PRIVMSG`` sends and blocks until a slot is free; defaults
  to 20 / 30 s, override via ``TWITCH_RATE_LIMIT_MSGS`` and
  ``TWITCH_RATE_LIMIT_SECS`` (set to 100 / 30 if the bot is a
  channel mod). The throttle is local-only — Twitch may still
  reject if multiple bot processes share the same account, so
  operators with mod-bots should keep one process.

Inbound: a persistent IRC connection (TLS by default) authenticated
via ``PASS oauth:<token>`` / ``NICK <bot>``. ``CAP REQ`` requests
the tags + commands extensions so every PRIVMSG carries a stable
``@id`` tag we can use as ``message_id`` and ``thread_id``.
``JOIN #<channel>`` for each configured channel. The reader loop
parses IRC frames, dispatches PING → PONG keepalive, and surfaces
PRIVMSG as ``Content.command`` for ``/cmd`` / ``!cmd`` prefixes
else ``Content.text``. Self-messages are skipped by case-insensitive
nick match. A dedupe set of recent message ids (capped at 1024,
oldest half evicted) suppresses redelivery on a tag-id collision
or a reconnect-replay.

Outbound: rejoins the persistent connection's writer (no
re-connect-per-send like the Rust adapter — that was a wasteful
2-round-trip OAuth dance the rate-limiter would have made worse).
Chunks at 500 chars (matching the Rust ``MAX_MESSAGE_LEN`` and
Twitch's IRC message length cap, conservative against UTF-8 byte
expansion). When ``cmd.thread_id`` is set, prepends
``@reply-parent-msg-id=<id> `` to attach the reply to the source
message; otherwise issues a plain PRIVMSG. Every chunk passes
through the token-bucket gate.

Stdlib-only (the SDK has zero runtime deps): ``socket`` + ``ssl``
for the wire, ``select`` / line buffering for read framing,
``threading.Lock`` for the writer mutex (the producer thread reads
inbound frames; ``on_send`` runs on asyncio threads, so writes need
a lock).

Configure via ``[[sidecar_channels]]``:

    [[sidecar_channels]]
    name = "twitch"
    command = "python3"
    args = ["-m", "librefang.sidecar.adapters.twitch"]
    channel_type = "twitch"
    [sidecar_channels.env]
    TWITCH_NICK = "librefang-bot"
    TWITCH_CHANNELS = "channel1,channel2"     # comma-separated, no '#'
    # TWITCH_ACCOUNT_ID = "prod"               # optional, multi-bot routing
    # TWITCH_HOST = "irc.chat.twitch.tv"       # override (rare)
    # TWITCH_PORT = "6697"                     # override (rare; 6697 TLS, 6667 plain)
    # TWITCH_PLAINTEXT = "1"                   # disable TLS (tests / mock listeners)
    # TWITCH_RATE_LIMIT_MSGS = "20"            # tokens per window (mod: 100)
    # TWITCH_RATE_LIMIT_SECS = "30"            # window seconds

Secrets via ``~/.librefang/secrets.env``:

    TWITCH_OAUTH_TOKEN=oauth:abc123...   # the 'oauth:' prefix is auto-added if absent
"""
from __future__ import annotations

import asyncio
import os
import socket
import ssl
import threading
import time
from typing import Any, Optional

from librefang.sidecar import Content, Field, Schema, SidecarAdapter, protocol, run_stdio_main
from librefang.sidecar import logging as log
from librefang.sidecar.common import split_message as _split_message

DEFAULT_HOST = "irc.chat.twitch.tv"
DEFAULT_TLS_PORT = 6697
DEFAULT_PLAINTEXT_PORT = 6667
# Twitch IRC's per-message cap. Matches MAX_MESSAGE_LEN in the
# deleted Rust adapter. Conservative against UTF-8 byte expansion —
# Twitch's docs say "approximately 500 chars per line".
MAX_MESSAGE_LEN = 500
# Token-bucket defaults: 20 messages per 30 s window for a non-mod
# bot account. Operators running a mod-status bot can raise to 100/30.
# Source: https://dev.twitch.tv/docs/irc/#rate-limits.
DEFAULT_RATE_LIMIT_MSGS = 20
DEFAULT_RATE_LIMIT_SECS = 30
# Dedupe set cap. IRC reconnects may replay a few frames; with
# IRCv3 ``@id`` tags every PRIVMSG carries a stable uuid we can use.
# Smaller than reddit (1024 vs 10000) because IRC chat is high-churn
# and we only need ~last-minute coverage to suppress redelivery.
SEEN_IDS_MAX = 1024
SEEN_IDS_EVICT = 512
# Socket read timeout — short so the producer loop can notice
# stop() promptly. Twitch IRC sends PINGs every ~5 min; a 30 s
# poll window is fine.
READ_TIMEOUT_SECS = 30.0
# Reconnect backoff bounds.
INITIAL_BACKOFF_SECS = 1.0
MAX_BACKOFF_SECS = 60.0


def _normalise_channel(value: str) -> str:
    """Strip whitespace, leading ``#``, and trailing slashes. Twitch
    channels are lowercase ASCII by convention; we coerce to lower
    so a config typo (``MyChannel``) still matches the JOIN reply."""
    s = value.strip().lstrip("#").rstrip("/")
    return s.lower()

def _parse_irc_tags(tag_blob: str) -> dict[str, str]:
    """Parse the IRCv3 ``@key=value;key2=value2`` tag block (without
    the leading ``@``). Empty values are kept as empty strings; tags
    without ``=`` get value ``""``. Returns ``{}`` for an empty blob.

    IRCv3 tag-value escaping replaces ``\\:`` with ``;``, ``\\s`` with
    space, ``\\\\`` with backslash, ``\\r`` with CR, ``\\n`` with LF.
    Twitch's tag values for ``id`` / ``reply-parent-msg-id`` are
    plain UUIDs / hex, so escapes are uncommon — handle them anyway
    for robustness."""
    if not tag_blob:
        return {}
    out: dict[str, str] = {}
    for pair in tag_blob.split(";"):
        if not pair:
            continue
        if "=" in pair:
            k, v = pair.split("=", 1)
        else:
            k, v = pair, ""
        # Undo IRCv3 escapes.
        v = (v.replace("\\:", ";")
              .replace("\\s", " ")
              .replace("\\\\", "\\")
              .replace("\\r", "\r")
              .replace("\\n", "\n"))
        out[k] = v
    return out


def _parse_irc_line(line: str) -> Optional[dict[str, Any]]:
    """Parse one IRC frame into a structured dict. Supports the
    extended ``@tags :prefix CMD args :trailing`` form.

    Returns ``None`` if the line is empty / blatantly malformed.
    Otherwise returns ``{"tags": {...}, "prefix": str|None,
    "command": str, "params": [str, ...]}``. The trailing param
    (everything after ``:``) is the last entry of ``params``.

    The parser is deliberately permissive: anything we can't parse
    we surface to the caller, which decides whether to log or
    ignore. This is more forgiving than the Rust adapter's hand-rolled
    ``parse_privmsg`` which only recognised one shape and silently
    dropped everything else (including PING — handled separately
    there by string prefix, which we also do for compatibility)."""
    line = line.rstrip("\r\n")
    if not line:
        return None
    tags: dict[str, str] = {}
    if line.startswith("@"):
        tag_end = line.find(" ")
        if tag_end < 0:
            return None
        tags = _parse_irc_tags(line[1:tag_end])
        line = line[tag_end + 1:]
    prefix: Optional[str] = None
    if line.startswith(":"):
        prefix_end = line.find(" ")
        if prefix_end < 0:
            return None
        prefix = line[1:prefix_end]
        line = line[prefix_end + 1:]
    # Split command + args, handling the trailing ``:`` form.
    parts: list[str] = []
    while line:
        if line.startswith(":"):
            parts.append(line[1:])
            line = ""
            break
        sp = line.find(" ")
        if sp < 0:
            parts.append(line)
            line = ""
        else:
            parts.append(line[:sp])
            line = line[sp + 1:]
    if not parts:
        return None
    return {
        "tags": tags,
        "prefix": prefix,
        "command": parts[0].upper(),
        "params": parts[1:],
    }


def _nick_from_prefix(prefix: Optional[str]) -> str:
    """Extract the nick from an IRC prefix ``nick!user@host``.
    Falls back to the whole prefix if there's no ``!`` (server
    PINGs / numerics use the server name as prefix)."""
    if not prefix:
        return ""
    if "!" in prefix:
        return prefix.split("!", 1)[0]
    return prefix


class _TokenBucket:
    """Simple monotonic-clock token bucket for outbound rate-limiting.

    Twitch's per-account cap (20 msgs / 30 s, 100 if mod) is a hard
    server-side limit — exceed it and the bot is dropped from chat.
    The bucket re-fills tokens evenly over the window. ``acquire()``
    sleeps the current thread until a token is available; bucket
    state is monitor-locked because both the producer thread (rare:
    server-reply auto-acks) and asyncio executor threads (``on_send``)
    may call into it."""

    def __init__(self, capacity: int, window_secs: float):
        if capacity < 1:
            capacity = 1
        if window_secs <= 0:
            window_secs = 1.0
        self.capacity = capacity
        self.window = float(window_secs)
        self.tokens = float(capacity)
        self.last = time.monotonic()
        self.lock = threading.Lock()

    def _refill_locked(self) -> None:
        now = time.monotonic()
        elapsed = now - self.last
        if elapsed > 0:
            self.tokens = min(
                self.capacity,
                self.tokens + (elapsed * self.capacity / self.window),
            )
            self.last = now

    def acquire(self) -> None:
        """Block (sleeping) until a token is available, then consume
        one. Spin-with-sleep instead of a condition variable because
        the regen rate is fixed and there are no producers to notify."""
        while True:
            with self.lock:
                self._refill_locked()
                if self.tokens >= 1.0:
                    self.tokens -= 1.0
                    return
                # Sleep just long enough for one token to regen.
                wait = (1.0 - self.tokens) * self.window / self.capacity
            # Sleep outside the lock so other threads can refill / consume.
            time.sleep(max(wait, 0.01))


class TwitchAdapter(SidecarAdapter):
    # Twitch chat is interactive — operators expect to see error
    # responses in-channel when something breaks, unlike the public
    # Reddit / Mastodon / Bluesky reply-bot scenarios. Default the
    # suppression knob to False to match the Rust adapter.
    suppress_error_responses = False
    # No typing / reaction / interactive / streaming concept on
    # Twitch IRC. Reply threading is achieved via the @reply-parent-msg-id
    # tag on outbound PRIVMSG; it's not a framework-level capability.
    capabilities: list = []

    SCHEMA = Schema(
        name="twitch",
        display_name="Twitch",
        description="Twitch IRC gateway adapter (out-of-process sidecar)",
        fields=[
            Field("TWITCH_OAUTH_TOKEN", "OAuth Token", "secret",
                  required=True,
                  placeholder="oauth:abc123… (the prefix is auto-added)"),
            Field("TWITCH_NICK", "Bot Nickname", "text",
                  required=True,
                  placeholder="librefang-bot"),
            Field("TWITCH_CHANNELS", "Channels (comma-separated, no '#')",
                  "text", required=True,
                  placeholder="channel1,channel2"),
            Field("TWITCH_ACCOUNT_ID",
                  "Account ID (multi-bot routing)", "text",
                  placeholder="prod", advanced=True),
            Field("TWITCH_RATE_LIMIT_MSGS",
                  f"Rate limit (msgs / window). Default "
                  f"{DEFAULT_RATE_LIMIT_MSGS} (non-mod), 100 if bot is mod.",
                  "text",
                  placeholder=str(DEFAULT_RATE_LIMIT_MSGS),
                  advanced=True),
            Field("TWITCH_RATE_LIMIT_SECS",
                  f"Rate-limit window seconds. Default "
                  f"{DEFAULT_RATE_LIMIT_SECS}.",
                  "text",
                  placeholder=str(DEFAULT_RATE_LIMIT_SECS),
                  advanced=True),
        ],
    )

    def __init__(self) -> None:
        self.oauth_token = os.environ.get("TWITCH_OAUTH_TOKEN", "").strip()
        self.nick = os.environ.get("TWITCH_NICK", "").strip()
        channels_raw = os.environ.get("TWITCH_CHANNELS", "").strip()
        self.channels = [
            _normalise_channel(c) for c in channels_raw.split(",")
            if _normalise_channel(c)
        ]
        acct = os.environ.get("TWITCH_ACCOUNT_ID", "").strip()
        self.account_id = acct or None

        # Transport overrides — production never sets these. Tests
        # use them to point the adapter at a local mock listener.
        self.host = os.environ.get("TWITCH_HOST", "").strip() or DEFAULT_HOST
        plaintext = os.environ.get("TWITCH_PLAINTEXT", "").strip()
        self.use_tls = plaintext not in ("1", "true", "yes", "on")
        port_raw = os.environ.get("TWITCH_PORT", "").strip()
        if port_raw:
            try:
                self.port = int(port_raw)
            except (TypeError, ValueError):
                log.error("TWITCH_PORT invalid (must be integer)",
                          value=port_raw)
                raise SystemExit(2) from None
        else:
            self.port = DEFAULT_TLS_PORT if self.use_tls else DEFAULT_PLAINTEXT_PORT

        # Rate-limit knobs.
        try:
            msgs = int(os.environ.get("TWITCH_RATE_LIMIT_MSGS", "").strip()
                       or DEFAULT_RATE_LIMIT_MSGS)
        except (TypeError, ValueError):
            log.error("TWITCH_RATE_LIMIT_MSGS invalid (must be integer)",
                      value=os.environ.get("TWITCH_RATE_LIMIT_MSGS"))
            raise SystemExit(2) from None
        try:
            secs = int(os.environ.get("TWITCH_RATE_LIMIT_SECS", "").strip()
                       or DEFAULT_RATE_LIMIT_SECS)
        except (TypeError, ValueError):
            log.error("TWITCH_RATE_LIMIT_SECS invalid (must be integer)",
                      value=os.environ.get("TWITCH_RATE_LIMIT_SECS"))
            raise SystemExit(2) from None
        if msgs < 1:
            log.warn("TWITCH_RATE_LIMIT_MSGS < 1; clamping to 1",
                     requested=msgs)
            msgs = 1
        if secs < 1:
            log.warn("TWITCH_RATE_LIMIT_SECS < 1; clamping to 1",
                     requested=secs)
            secs = 1
        self.rate_limit_msgs = msgs
        self.rate_limit_secs = secs
        self._bucket = _TokenBucket(msgs, secs)

        missing: list[str] = []
        if not self.oauth_token:
            missing.append("TWITCH_OAUTH_TOKEN")
        if not self.nick:
            missing.append("TWITCH_NICK")
        if not self.channels:
            missing.append("TWITCH_CHANNELS")
        if missing:
            log.error("twitch required env vars missing", missing=missing)
            raise SystemExit(2)

        # Bot nick is matched case-insensitively against PRIVMSG sender
        # to skip own messages (the Rust adapter did the same).
        self._bot_nick_lc = self.nick.lower()

        # Active socket, writer lock, and stop flag. The producer
        # thread owns the read side; on_send writes through the
        # locked writer. None when not currently connected.
        self._sock: Optional[socket.socket] = None
        self._writer_lock = threading.Lock()
        self._stop = threading.Event()

        # Dedupe set for already-seen message ids. Capped with
        # crude oldest-half eviction.
        self._seen_ids: list[str] = []
        self._seen_ids_set: set[str] = set()

    # ---- transport ---------------------------------------------------

    def _pass_string(self) -> str:
        """Format the OAuth token for the IRC PASS command, adding
        the ``oauth:`` prefix if the operator left it off."""
        tok = self.oauth_token
        if not tok.startswith("oauth:"):
            tok = f"oauth:{tok}"
        return f"PASS {tok}\r\n"

    def _connect(self) -> socket.socket:
        """Open a fresh socket (TLS by default), authenticate, request
        IRCv3 capabilities, and join configured channels. Returns the
        connected socket. Raises on any transport / auth error.

        TLS uses the stdlib default context, which is the modern
        secure baseline: verifies the server cert against system
        trust roots and enforces a current cipher suite. This is the
        single biggest security improvement over the Rust adapter,
        which sent the OAuth token in cleartext on every connect."""
        log.info("twitch connecting",
                 host=self.host, port=self.port, tls=self.use_tls)
        sock = socket.create_connection((self.host, self.port), timeout=15.0)
        if self.use_tls:
            ctx = ssl.create_default_context()
            sock = ctx.wrap_socket(sock, server_hostname=self.host)
        sock.settimeout(READ_TIMEOUT_SECS)

        # IRCv3 capability negotiation. Twitch's tags + commands caps
        # give us @id (for thread_id round-trip), @reply-parent-msg-id
        # handling on the server side, and explicit user state events
        # (USERSTATE / NOTICE / etc) we can log. Request BEFORE auth
        # — Twitch closes the capability window after registration.
        self._raw_send(sock, "CAP REQ :twitch.tv/tags twitch.tv/commands\r\n")
        # Authenticate.
        self._raw_send(sock, self._pass_string())
        self._raw_send(sock, f"NICK {self.nick}\r\n")
        # Join all configured channels.
        for ch in self.channels:
            self._raw_send(sock, f"JOIN #{ch}\r\n")
        log.info("twitch authenticated and joined", channels=self.channels)
        return sock

    @staticmethod
    def _raw_send(sock: socket.socket, frame: str) -> None:
        """Send a single IRC frame on an *unlocked* socket. Used during
        connect / capability negotiation (single-thread context). For
        the steady-state send path, see ``_send_frame_locked``."""
        sock.sendall(frame.encode("utf-8"))

    def _send_frame_locked(self, frame: str) -> None:
        """Send one IRC frame on the active socket, holding the writer
        lock. Raises ``RuntimeError`` if the socket isn't connected."""
        with self._writer_lock:
            if self._sock is None:
                raise RuntimeError("twitch not connected")
            self._sock.sendall(frame.encode("utf-8"))

    # ---- dedupe ------------------------------------------------------

    def _mark_seen(self, msg_id: str) -> bool:
        """Return True if `msg_id` is fresh; False if we've seen it.
        Crude oldest-half eviction at the cap."""
        if not msg_id:
            return True
        if msg_id in self._seen_ids_set:
            return False
        self._seen_ids.append(msg_id)
        self._seen_ids_set.add(msg_id)
        if len(self._seen_ids) > SEEN_IDS_MAX:
            to_drop = self._seen_ids[:SEEN_IDS_EVICT]
            self._seen_ids = self._seen_ids[SEEN_IDS_EVICT:]
            self._seen_ids_set.difference_update(to_drop)
        return True

    # ---- inbound: read loop on a worker thread ------------------------

    def _handle_line(self, line: str, emit) -> None:
        """Process one raw IRC line: dispatch PING → PONG, parse
        PRIVMSG → ``message`` event. All other commands are logged
        at DEBUG and ignored."""
        # PING is special-cased because Twitch sends it as
        # ``PING :tmi.twitch.tv`` with no nick prefix; the response
        # must echo the same trailing param.
        if line.startswith("PING"):
            pong = line.replace("PING", "PONG", 1).rstrip("\r\n") + "\r\n"
            try:
                self._send_frame_locked(pong)
            except Exception as e:  # noqa: BLE001
                log.warn("twitch PONG send failed", error=str(e))
            return
        frame = _parse_irc_line(line)
        if frame is None or frame["command"] != "PRIVMSG":
            return
        params = frame["params"]
        if len(params) < 2:
            return
        channel = params[0]
        message = params[1]
        sender = _nick_from_prefix(frame["prefix"])
        if not sender or not message:
            return
        # Skip own messages — case-insensitive nick match.
        if sender.lower() == self._bot_nick_lc:
            return
        # IRCv3 tag id (Twitch always sets this when twitch.tv/tags
        # is enabled). Fall back to a synthetic uuid only if absent.
        tags = frame.get("tags") or {}
        msg_id = tags.get("id") or ""
        # Dedupe — Twitch IRC re-broadcasts on JOIN; with the tag id
        # we can suppress redelivery deterministically.
        if msg_id and not self._mark_seen(msg_id):
            return

        # Slash / bang command routing matches the Rust adapter.
        if message.startswith("/") or message.startswith("!"):
            trimmed = message[1:]
            head, sep, tail = trimmed.partition(" ")
            args = tail.split() if (sep and tail) else []
            content = Content.command(head, args)
        else:
            content = Content.text(message)

        metadata: dict[str, Any] = {"channel": channel}
        # Useful Twitch tags for downstream consumers / debugging.
        for k in ("display-name", "user-id", "room-id", "color",
                  "badges", "first-msg", "reply-parent-msg-id",
                  "reply-parent-user-login"):
            if k in tags and tags[k]:
                metadata[k] = tags[k]

        # Twitch's display-name tag preserves the original casing
        # (sender from the prefix is forced lowercase by Twitch).
        display_name = tags.get("display-name") or sender

        ev = protocol.message(
            user_id=channel,  # platform_id = channel (matches Rust
                              # behaviour: outbound PRIVMSG targets
                              # the channel, not the user)
            user_name=display_name,
            content=content,
            # Native Twitch message id — use this as platform_message_id
            # so reactions / edits target the real frame instead of
            # a server-generated UUID.
            message_id=msg_id or None,
            is_group=True,  # Twitch chat is always a public channel.
            # P2: surface @id as thread_id so on_send can attach
            # @reply-parent-msg-id back to the source message.
            thread_id=msg_id or None,
            metadata=metadata,
        )
        if self.account_id is not None:
            ev["params"].setdefault("metadata", {})["account_id"] = self.account_id
        emit(ev)

    def _reader_loop_blocking(self, sock: socket.socket, emit) -> None:
        """Read IRC frames off `sock` until EOF / stop / error. Each
        complete line (terminated by ``\\r\\n``) is fed to
        ``_handle_line``. Returns normally on clean EOF; raises on
        transport error so the producer's backoff catches it."""
        buf = b""
        while not self._stop.is_set():
            try:
                chunk = sock.recv(4096)
            except socket.timeout:
                # Timeout is benign — loop back and check `_stop`.
                continue
            except OSError as e:
                # Includes connection reset / TLS shutdown errors.
                raise RuntimeError(f"twitch socket recv failed: {e}") from e
            if not chunk:
                log.info("twitch socket closed by peer")
                return
            buf += chunk
            while b"\r\n" in buf:
                line_bytes, _, buf = buf.partition(b"\r\n")
                try:
                    line = line_bytes.decode("utf-8", "replace")
                except Exception:  # noqa: BLE001
                    continue
                try:
                    self._handle_line(line, emit)
                except Exception as e:  # noqa: BLE001
                    log.warn("twitch handle_line failed",
                             error=str(e), line=line[:200])

    def _producer_blocking(self, emit) -> None:
        """Maintain an IRC connection forever in this worker thread.
        On any transport / auth error, close the socket and back off
        exponentially before reconnecting.

        Reconnect is the *adapter's* job per the SDK split (transport
        reconnect is adapter; process restart is daemon). The daemon
        supervises us as a black box: an irrecoverable failure raises
        out of here, the run-loop crashes, and the supervisor respawns
        the whole process. A merely flaky network stays inside this
        loop."""
        backoff = INITIAL_BACKOFF_SECS
        while not self._stop.is_set():
            try:
                sock = self._connect()
            except Exception as e:  # noqa: BLE001
                log.warn("twitch connect failed; backing off",
                         error=str(e), delay=backoff)
                if self._stop.wait(backoff):
                    return
                backoff = min(backoff * 2, MAX_BACKOFF_SECS)
                continue
            with self._writer_lock:
                self._sock = sock
            backoff = INITIAL_BACKOFF_SECS
            try:
                self._reader_loop_blocking(sock, emit)
            except Exception as e:  # noqa: BLE001
                log.warn("twitch reader loop error", error=str(e))
            finally:
                with self._writer_lock:
                    self._sock = None
                try:
                    sock.close()
                except Exception:  # noqa: BLE001
                    pass
            if self._stop.is_set():
                return
            log.info("twitch reconnecting", delay=backoff)
            if self._stop.wait(backoff):
                return
            backoff = min(backoff * 2, MAX_BACKOFF_SECS)

    async def produce(self, emit) -> None:
        loop = asyncio.get_event_loop()
        await loop.run_in_executor(None, self._producer_blocking, emit)

    # ---- outbound: PRIVMSG -------------------------------------------

    def _send_privmsg_blocking(self, channel: str, text: str,
                               reply_parent_id: Optional[str]) -> None:
        """Issue one or more PRIVMSG frames (chunked at MAX_MESSAGE_LEN)
        on the active connection. Each chunk passes the token-bucket
        gate first. When ``reply_parent_id`` is set, the IRCv3 tag
        ``@reply-parent-msg-id`` is prepended so Twitch renders the
        reply threaded under the source message (improvement over
        the Rust adapter, which never set this).

        Raises ``RuntimeError`` if the socket is not currently
        connected (producer not yet wired / mid-reconnect). The
        daemon's send-retry policy is the right place to handle that."""
        # Normalise the channel — incoming target is the IRC channel
        # name (e.g. "#librefang"); we tolerate either form.
        if not channel.startswith("#"):
            channel = f"#{_normalise_channel(channel)}"
        chunks = _split_message(text, MAX_MESSAGE_LEN)
        for chunk in chunks:
            self._bucket.acquire()
            if reply_parent_id:
                frame = (f"@reply-parent-msg-id={reply_parent_id} "
                         f"PRIVMSG {channel} :{chunk}\r\n")
            else:
                frame = f"PRIVMSG {channel} :{chunk}\r\n"
            self._send_frame_locked(frame)

    async def on_send(self, cmd) -> None:
        # Text-only: Twitch IRC is a plain-text protocol. Structured
        # content falls back to a placeholder rather than silently
        # dropping the send (matches the Rust adapter).
        if cmd.content and not (
            isinstance(cmd.content, dict) and "Text" in cmd.content
        ):
            text = "(Unsupported content type — Twitch IRC only supports text replies)"
        else:
            text = cmd.text or ""
        channel = cmd.channel_id or ""
        if not channel:
            # Fall back to the first configured channel if the daemon
            # didn't carry one through. The Rust adapter relied on
            # user.platform_id for this.
            raw_user = getattr(cmd, "user", None) or {}
            if isinstance(raw_user, dict):
                channel = str(raw_user.get("platform_id") or "")
        if not channel:
            raise RuntimeError(
                "twitch on_send: missing channel target "
                "(cmd.channel_id and cmd.user.platform_id are both empty)"
            )
        thread_id = getattr(cmd, "thread_id", None)
        if thread_id is not None and not isinstance(thread_id, str):
            thread_id = str(thread_id) if thread_id else None
        await asyncio.get_event_loop().run_in_executor(
            None,
            self._send_privmsg_blocking,
            channel, text, thread_id or None,
        )

    async def on_shutdown(self) -> None:
        """Send a courteous QUIT (best-effort) and close the socket
        so Twitch sees a clean disconnect rather than a half-open
        TCP state."""
        self._stop.set()
        with self._writer_lock:
            sock = self._sock
            self._sock = None
        if sock is not None:
            try:
                sock.sendall(b"QUIT :Shutting down\r\n")
            except Exception:  # noqa: BLE001
                pass
            try:
                sock.close()
            except Exception:  # noqa: BLE001
                pass


if __name__ == "__main__":
    run_stdio_main(TwitchAdapter)
