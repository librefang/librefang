#!/usr/bin/env python3
"""Signal sidecar channel adapter for LibreFang.

Replaces the former in-process Rust ``librefang-channels::signal``
adapter (removed in this sidecar migration; same pattern as ntfy
#5224, telegram #5241, gotify #5263, mastodon #5264, bluesky #5277,
reddit #5281, twitch #5297, rocketchat #5298, discord #5299,
nextcloud #5301, slack #5302, webex #5309, line #5312, zulip #5310,
mattermost #5315).

Talks to ``signal-cli-rest-api`` (https://github.com/bbernhard/signal-cli-rest-api),
a thin HTTP wrapper around the ``signal-cli`` daemon. The operator must
run that container separately and register a phone number with Signal
first.

Behaviour parity with the Rust adapter (every assertion below has a
file/line citation against ``crates/librefang-channels/src/signal.rs``
on the pre-migration tree):

* **SSRF guard**: ``SIGNAL_API_URL`` is validated at startup. Scheme
  must be ``http`` / ``https``; the resolved host must NOT be a
  private / loopback / link-local / CGNAT / IPv6-ULA address unless
  ``SIGNAL_ALLOW_LOCAL`` is set (signal.rs:27-98). Default-deny
  matches the Rust contract — operators running the REST API on
  ``localhost`` (the typical signal-cli-rest-api deployment) must
  flip the knob explicitly.
* **Auth**: optional Bearer token via ``SIGNAL_API_KEY`` on every
  request (signal.rs:194-201).
* **Polling**: ``GET /v1/receive/{phone_number}`` every
  ``SIGNAL_POLL_INTERVAL_SECS`` (default 2 s, mirrors
  signal.rs:171). signal-cli-rest-api returns an array of envelopes;
  each envelope's ``envelope.dataMessage.message`` is the inbound
  text.
* **Self / allowlist / empty filters** (signal.rs:361-376): drop
  events where ``envelope.source`` matches the bot's own phone
  number, where ``allowed_users`` is non-empty and the source isn't
  listed, or where the extracted ``message`` is empty.
* **Slash-command routing**: ``/cmd args`` → ``Command`` (text
  otherwise; signal.rs:383-396).
* **Sender identity** (signal.rs:404-408): ``platform_id =
  envelope.source`` (the phone number — that's what ``POST /v2/send``
  uses as the recipient on outbound), ``display_name =
  envelope.sourceName`` (falls back to the source phone when
  absent).
* **Outbound send** (signal.rs:204-244): ``POST /v2/send`` with
  ``{message, number, recipients, [base64_attachments]}``. ``number``
  is the registered bot phone (passed by the daemon to signal-cli),
  ``recipients`` is a single-element array of the addressee.
* **Multi-bot ``account_id``** (signal.rs:416-422, #5003). When
  ``SIGNAL_ACCOUNT_ID`` is set, it is injected into the inbound
  message metadata so the bridge can scope ``ApprovalRequested``
  delivery to the channel bound to the requesting agent.
* **ChannelType::Signal preserved** as ``channel_type = "signal"``
  on the sidecar entry — existing routing / ``channel_role_mapping``
  keys that reference ``signal`` continue to resolve.

Improvements over the Rust adapter
==================================

1. **Inbound dedupe on ``envelope.timestamp``**. The Rust adapter at
   signal.rs:398-415 emitted every parsed envelope unconditionally;
   signal-cli-rest-api re-delivers events when the underlying
   ``receive`` call is retried, so a transient network glitch caused
   duplicate agent invocations. Bounded local set on the timestamp
   (the platform-stable message id) with
   ``SEEN_MESSAGES_MAX = 10 000`` / ``SEEN_MESSAGES_EVICT = 5 000``
   (same policy as reddit / rocketchat / nextcloud / webex / line).
2. **429 ``Retry-After`` honoured on both polling and send**. The
   Rust adapter had no 429 handling — a throttled poll either lost
   events or burned the poll cadence; a throttled send returned an
   Err and dropped the chunk. Sidecar parses ``Retry-After``
   (default 30 s fallback, floor 1 s, cap ``MAX_BACKOFF_SECS``),
   sleeps, retries once, then logs-and-continues on the second 429
   (matches webex / line / #5303).
3. **Explicit HTTP timeouts**. ``urllib.request.urlopen`` has no
   default timeout; the Rust adapter pre-configured ``reqwest``'s
   connect / total timeouts at signal.rs:156-162. Sidecar passes
   ``timeout=SEND_TIMEOUT_SECS`` (15 s) on every call so a
   misbehaving REST endpoint trips an explicit error.
4. **Exponential backoff on transport errors**. The Rust polling
   loop at signal.rs:325-356 just ``continue``-d on every error
   without backing off — a wedged signal-cli daemon caused the
   sidecar to spin at ``poll_interval``. Sidecar applies 1 s → 60 s
   exponential backoff on transport / non-2xx errors (mirrors the
   webex / mattermost reconnect ladder).

Stdlib-only: HTTPS via ``urllib.request``, polling on a worker
thread (no asyncio HTTP). The SSRF guard uses ``socket.getaddrinfo``
+ ``ipaddress`` to mirror the Rust adapter's
``std::net::ToSocketAddrs`` + per-octet checks.

Configure via ``[[sidecar_channels]]``::

    [[sidecar_channels]]
    name = "signal"
    command = "python3"
    args = ["-m", "librefang.sidecar.adapters.signal"]
    channel_type = "signal"
    [sidecar_channels.env]
    SIGNAL_API_URL  = "https://signal-cli.example.com"
    SIGNAL_NUMBER   = "+15555550100"
    # SIGNAL_ALLOWED_USERS = "+15555550199,+15555550200"  # optional
    # SIGNAL_ACCOUNT_ID    = "prod-bot"                   # optional
    # SIGNAL_POLL_INTERVAL_SECS = "2"                     # optional
    # SIGNAL_ALLOW_LOCAL   = "1"                          # opt-in SSRF bypass

Secret via ``~/.librefang/secrets.env``: ``SIGNAL_API_KEY`` (optional;
required only when the signal-cli REST API was started with
``--api-key``).
"""
from __future__ import annotations

import asyncio
import ipaddress
import json
import os
import socket
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

SEND_TIMEOUT_SECS = 15.0
POLL_TIMEOUT_SECS = 30.0

INITIAL_BACKOFF_SECS = 1.0
MAX_BACKOFF_SECS = 60.0

# Default fallback when signal-cli-rest-api 429s without a parseable
# ``Retry-After`` header. 30 s is conservative — mirrors the rocketchat
# / nextcloud / webex / line / mattermost sidecars (#5303).
RETRY_AFTER_DEFAULT_SECS = 30.0

# Bounded dedupe cap on ``envelope.timestamp``. Same policy as
# reddit / rocketchat / nextcloud / webex / line / mattermost.
SEEN_MESSAGES_MAX = 10_000
SEEN_MESSAGES_EVICT = 5_000

DEFAULT_POLL_INTERVAL_SECS = 2.0

def _parse_retry_after(resp_hdrs: dict, *, default_secs: float) -> float:
    """Backwards-compat wrapper around
    :func:`librefang.sidecar.common.parse_retry_after`."""
    return _parse_retry_after_impl(
        resp_hdrs,
        default_secs=default_secs,
        floor_secs=1.0,
        max_secs=MAX_BACKOFF_SECS,
    )

def _is_private_or_loopback(addr: str) -> bool:
    """Mirror the Rust adapter's ``is_private_or_loopback`` at
    signal.rs:27-46.  Accepts a textual IP (v4 or v6) and returns
    True when it falls into any of:

    * loopback (127/8, ::1)
    * link-local (169.254/16, fe80::/10)
    * RFC-1918 private (10/8, 172.16/12, 192.168/16)
    * CGNAT (100.64/10)
    * IPv6 ULA (fc00::/7)
    * IPv4 broadcast / unspecified (0.0.0.0, 255.255.255.255)
    * IPv6 unspecified (::)
    """
    try:
        ip = ipaddress.ip_address(addr)
    except ValueError:
        # Not a literal IP. ``socket.getaddrinfo`` always hands back a
        # well-formed IPv4 / IPv6 literal in production, so this path
        # should be unreachable — but the SSRF guard's whole job is
        # default-deny, so fail-CLOSED here. A future caller that
        # forwards a non-IP string (e.g. an IPv6 scoped literal like
        # ``fe80::1%eth0`` that ``ipaddress.ip_address`` can't parse)
        # must NOT slip through as "public address, allow".
        return True
    if ip.is_loopback or ip.is_link_local or ip.is_unspecified:
        return True
    if isinstance(ip, ipaddress.IPv4Address):
        if ip.is_private:
            return True
        # 255.255.255.255 broadcast
        if int(ip) == 0xFFFFFFFF:
            return True
        # CGNAT 100.64.0.0/10 — `is_private` already covers this on
        # 3.13+, but be explicit for older Pythons.
        if ip in ipaddress.IPv4Network("100.64.0.0/10"):
            return True
        return False
    # IPv6
    if isinstance(ip, ipaddress.IPv6Address):
        if ip.is_private:
            return True
        # is_private on IPv6 covers ULA + link-local already, but
        # check fc00::/7 explicitly to match the Rust per-octet test.
        if ip in ipaddress.IPv6Network("fc00::/7"):
            return True
    return False


def validate_api_url(api_url: str, *, allow_local: bool) -> Optional[str]:
    """Validate ``api_url`` for SSRF safety. Returns ``None`` on
    success, ``str`` error message on violation. Mirrors the Rust
    helper at signal.rs:57-98:

    * scheme must be ``http`` or ``https``
    * host must resolve to a non-private / non-loopback address
      unless ``allow_local`` is set
    """
    try:
        parsed = urllib.parse.urlparse(api_url)
    except (TypeError, ValueError) as e:
        return f"Signal api_url is not a valid URL: {e}"
    if parsed.scheme not in ("http", "https"):
        return (
            f"Signal api_url scheme {parsed.scheme!r} is not allowed; "
            "use http or https"
        )
    if allow_local:
        return None
    host = parsed.hostname
    if not host:
        return "Signal api_url has no host"
    port = parsed.port or (443 if parsed.scheme == "https" else 80)
    try:
        addrs = socket.getaddrinfo(host, port, type=socket.SOCK_STREAM)
    except socket.gaierror as e:
        return f"Signal api_url DNS resolution failed for {host!r}: {e}"
    for entry in addrs:
        # entry shape: (family, type, proto, canonname, sockaddr)
        sockaddr = entry[4]
        if not sockaddr:
            continue
        addr = sockaddr[0]
        if _is_private_or_loopback(addr):
            return (
                f"Signal api_url {api_url!r} resolves to a private/loopback "
                f"address ({addr}). Set SIGNAL_ALLOW_LOCAL=1 in "
                "[sidecar_channels.env] if this is intentional."
            )
    return None


def parse_signal_envelope(
    payload: dict,
    *,
    own_phone: str,
    allowed_users: list[str],
    account_id: Optional[str],
) -> Optional[dict]:
    """Pure-function port of the inbound parse path in
    ``crates/librefang-channels/src/signal.rs`` lines 358-426.

    ``payload`` is one element of the array returned by
    ``GET /v1/receive/{phone}``. signal-cli-rest-api returns each item
    either bare or wrapped under an ``envelope`` key — we unwrap when
    the key is present (signal.rs:359).

    Returns a ``message`` event dict ready to ``emit``, or ``None``
    when the payload should be skipped (self / allowlist / empty
    text / malformed).
    """
    if not isinstance(payload, dict):
        return None
    envelope = payload.get("envelope")
    if not isinstance(envelope, dict):
        envelope = payload

    source = envelope.get("source")
    if not isinstance(source, str) or not source:
        return None
    # Self-skip.
    if own_phone and source == own_phone:
        return None
    if allowed_users and source not in allowed_users:
        return None

    data_message = envelope.get("dataMessage")
    if not isinstance(data_message, dict):
        return None
    text = data_message.get("message")
    if not isinstance(text, str) or not text:
        return None

    source_name = envelope.get("sourceName")
    if not isinstance(source_name, str) or not source_name:
        source_name = source

    ts = envelope.get("timestamp")
    if isinstance(ts, int):
        message_id: Optional[str] = str(ts)
    elif isinstance(ts, str) and ts:
        message_id = ts
    else:
        message_id = None

    if text.startswith("/"):
        head, _, tail = text[1:].partition(" ")
        content = Content.command(head, tail.split() if tail else [])
    else:
        content = Content.text(text)

    metadata: dict[str, Any] = {}
    if account_id is not None:
        metadata["account_id"] = account_id

    return protocol.message(
        # platform_id is the sender's phone (what POST /v2/send takes
        # as the recipient on outbound). Mirrors signal.rs:405.
        user_id=source,
        user_name=source_name,
        content=content,
        message_id=message_id,
        is_group=False,
        metadata=metadata,
    )


class SignalAdapter(SidecarAdapter):
    # signal-cli-rest-api has no native typing indicator or reactions
    # exposed on its standard endpoints — surface a clean capability
    # set rather than over-claim.
    capabilities: list = []
    # Signal is 1:1 or group chat (the Rust adapter only handled
    # 1:1 dataMessages, so we match). Surface errors so the user
    # sees the failure mode — same chat-room precedent as
    # mattermost / line / discord.
    suppress_error_responses: bool = False

    SCHEMA = Schema(
        name="signal",
        display_name="Signal",
        description="signal-cli REST API adapter (out-of-process sidecar)",
        fields=[
            Field("SIGNAL_API_URL", "signal-cli REST API URL", "text",
                  required=True,
                  placeholder="https://signal-cli.example.com"),
            Field("SIGNAL_NUMBER", "Registered Phone Number", "text",
                  required=True,
                  placeholder="+15555550100"),
            Field("SIGNAL_API_KEY",
                  "API Key (only when signal-cli REST API was started with --api-key)",
                  "secret",
                  placeholder="api-key-...",
                  advanced=True),
            Field("SIGNAL_ALLOWED_USERS",
                  "Allowed Phone Numbers (comma-separated, empty = all)",
                  "text",
                  placeholder="+15555550199,+15555550200",
                  advanced=True),
            Field("SIGNAL_POLL_INTERVAL_SECS",
                  "Poll Interval (seconds)",
                  "number",
                  placeholder=str(int(DEFAULT_POLL_INTERVAL_SECS)),
                  advanced=True),
            Field("SIGNAL_ALLOW_LOCAL",
                  "Allow loopback / private address (1 to opt in)",
                  "text",
                  placeholder="0",
                  advanced=True),
            Field("SIGNAL_ACCOUNT_ID",
                  "Account ID (multi-bot routing)",
                  "text",
                  placeholder="prod-bot",
                  advanced=True),
        ],
    )

    def __init__(self) -> None:
        api_url = os.environ.get("SIGNAL_API_URL", "").strip()
        phone = os.environ.get("SIGNAL_NUMBER", "").strip()
        missing: list[str] = []
        if not api_url:
            missing.append("SIGNAL_API_URL")
        if not phone:
            missing.append("SIGNAL_NUMBER")
        if missing:
            log.error("signal required env vars missing", missing=missing)
            raise SystemExit(2)

        allow_local_raw = os.environ.get("SIGNAL_ALLOW_LOCAL", "").strip().lower()
        # Truthy values match the convention used by other sidecars
        # (`1`, `true`, `yes`, `on`); anything else is treated as off.
        allow_local = allow_local_raw in ("1", "true", "yes", "on")
        err = validate_api_url(api_url, allow_local=allow_local)
        if err is not None:
            log.error("signal SIGNAL_API_URL rejected", error=err)
            raise SystemExit(2)
        self.api_url = api_url.rstrip("/")
        self.phone_number = phone
        self.api_key = os.environ.get("SIGNAL_API_KEY", "").strip() or None
        self.allowed_users = _split_csv(
            os.environ.get("SIGNAL_ALLOWED_USERS", "")
        )
        acct = os.environ.get("SIGNAL_ACCOUNT_ID", "").strip()
        self.account_id = acct or None

        poll_raw = os.environ.get("SIGNAL_POLL_INTERVAL_SECS", "").strip()
        if poll_raw:
            try:
                self.poll_interval = max(0.5, float(poll_raw))
            except ValueError:
                log.warn(
                    "signal SIGNAL_POLL_INTERVAL_SECS not numeric; using default",
                    value=poll_raw, default=DEFAULT_POLL_INTERVAL_SECS,
                )
                self.poll_interval = DEFAULT_POLL_INTERVAL_SECS
        else:
            self.poll_interval = DEFAULT_POLL_INTERVAL_SECS

        # Improvement #1: bounded dedupe on envelope.timestamp.
        self._seen = _SeenSet(max_size=SEEN_MESSAGES_MAX, evict=SEEN_MESSAGES_EVICT)

        self._shutdown = threading.Event()

    # ---- HTTP helpers ------------------------------------------------

    def _auth_headers(self, *, content_type: bool = False) -> dict:
        h = {
            "User-Agent": "librefang-signal-sidecar/1 (https://librefang.org)",
        }
        if self.api_key:
            h["Authorization"] = f"Bearer {self.api_key}"
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

    def _mark_seen(self, message_id: Optional[str]) -> bool:
        """Return True iff freshly seen. Shim around :class:`librefang.sidecar.common.SeenSet`."""
        return self._seen.mark(message_id)

    # ---- REST: poll + send ------------------------------------------

    def _poll_once(self) -> tuple[Optional[list], Optional[float]]:
        """Single ``GET /v1/receive/{phone}`` fetch. Returns
        ``(envelopes, retry_after_secs)``:

        * ``envelopes`` is a list of payloads when the call returned
          200 + a JSON array; ``None`` for any error path (so the
          producer loop can apply backoff).
        * ``retry_after_secs`` is set when the server returned 429
          with a parseable ``Retry-After`` (or the default fallback);
          the producer sleeps that long before its next poll.
        """
        url = f"{self.api_url}/v1/receive/{urllib.parse.quote(self.phone_number, safe='+')}"
        status, body, raw, resp_hdrs = self._http(
            url, headers=self._auth_headers(),
            timeout=POLL_TIMEOUT_SECS,
        )
        if status == 429:
            wait = _parse_retry_after(
                resp_hdrs, default_secs=RETRY_AFTER_DEFAULT_SECS,
            )
            log.warn(
                "signal /v1/receive 429; honouring Retry-After",
                retry_after_secs=wait,
            )
            return None, wait
        if status >= 300:
            snippet = raw[:200].decode("utf-8", "replace") if raw else ""
            log.warn("signal /v1/receive failed",
                     status=status, body=snippet)
            return None, None
        if not isinstance(body, list):
            return None, None
        return body, None

    def _post_send(
        self,
        recipient: str,
        text: str,
    ) -> None:
        """POST /v2/send. Honours 429 ``Retry-After`` and retries once.
        Mirrors signal.rs:204-244. On the second 429 / non-2xx we log
        and continue — matches the webex / line / mattermost fail-open
        behaviour so a single throttled outbound doesn't crash the
        adapter."""
        if not recipient:
            log.warn("signal _post_send: empty recipient, dropping")
            return
        url = f"{self.api_url}/v2/send"
        payload = {
            "message": text,
            "number": self.phone_number,
            "recipients": [recipient],
        }
        body = json.dumps(payload).encode("utf-8")
        status, _resp, raw, resp_hdrs = self._http(
            url, method="POST", body=body,
            headers=self._auth_headers(content_type=True),
        )
        if status == 429:
            wait = _parse_retry_after(
                resp_hdrs, default_secs=RETRY_AFTER_DEFAULT_SECS,
            )
            log.warn("signal POST /v2/send 429; sleeping then retrying once",
                     retry_after_secs=wait)
            time.sleep(wait)
            status, _resp, raw, resp_hdrs = self._http(
                url, method="POST", body=body,
                headers=self._auth_headers(content_type=True),
            )
        if status >= 300:
            snippet = raw[:200].decode("utf-8", "replace") if raw else ""
            log.warn("signal POST /v2/send failed",
                     recipient=recipient, status=status, body=snippet)

    # ---- polling producer loop --------------------------------------

    def _producer_blocking(self, emit: Callable[[dict], None]) -> None:
        """Polling worker. Sleeps ``poll_interval`` between successful
        fetches; on transport / non-2xx errors applies 1 s → 60 s
        exponential backoff (improvement #4)."""
        backoff = INITIAL_BACKOFF_SECS
        log.info("signal polling started",
                 api_url=self.api_url,
                 phone_number=self.phone_number,
                 poll_interval_secs=self.poll_interval)
        while not self._shutdown.is_set():
            try:
                envelopes, retry_after = self._poll_once()
            except Exception as e:  # noqa: BLE001 — transport varies
                log.warn("signal poll transport error",
                         error=str(e), delay=backoff)
                if self._shutdown.wait(backoff):
                    return
                backoff = min(backoff * 2.0, MAX_BACKOFF_SECS)
                continue

            if retry_after is not None:
                if self._shutdown.wait(retry_after):
                    return
                continue
            if envelopes is None:
                if self._shutdown.wait(backoff):
                    return
                backoff = min(backoff * 2.0, MAX_BACKOFF_SECS)
                continue

            backoff = INITIAL_BACKOFF_SECS

            for payload in envelopes:
                if not isinstance(payload, dict):
                    continue
                inner = payload.get("envelope") if isinstance(payload.get("envelope"), dict) else payload
                ts = inner.get("timestamp") if isinstance(inner, dict) else None
                msg_id = str(ts) if isinstance(ts, (int, str)) and ts else None
                if msg_id and not self._mark_seen(msg_id):
                    continue
                ev = parse_signal_envelope(
                    payload,
                    own_phone=self.phone_number,
                    allowed_users=self.allowed_users,
                    account_id=self.account_id,
                )
                if ev is not None:
                    emit(ev)

            if self._shutdown.wait(self.poll_interval):
                return

    # ---- public sidecar surface --------------------------------------

    async def produce(self, emit: Callable[[dict], None]) -> None:
        loop = asyncio.get_event_loop()
        try:
            await loop.run_in_executor(None, self._producer_blocking, emit)
        except asyncio.CancelledError:
            self._shutdown.set()
            raise

    async def on_shutdown(self) -> None:
        self._shutdown.set()

    async def on_send(self, cmd) -> None:
        recipient = (
            cmd.channel_id
            or (cmd.user.get("platform_id") if cmd.user else "")
            or ""
        )
        if not recipient:
            log.warn("signal on_send: empty recipient, dropping")
            return

        content = cmd.content
        text = cmd.text or ""
        loop = asyncio.get_event_loop()
        if isinstance(content, dict) and "Text" in content:
            await loop.run_in_executor(
                None, lambda: self._post_send(recipient, text),
            )
            return
        if content and not (isinstance(content, dict) and "Text" in content):
            # The Rust adapter implemented attachment support
            # (signal.rs:444-716); the sidecar keeps the surface
            # simple — non-text content becomes a "(Unsupported
            # content type)" placeholder so the user sees a clear
            # signal that the send was dropped. A future follow-up
            # can wire up image/file attachments by fetching the URL
            # and POSTing base64 to ``/v2/send`` via
            # ``base64_attachments``.
            await loop.run_in_executor(
                None,
                lambda: self._post_send(
                    recipient, "(Unsupported content type)",
                ),
            )
            return

        await loop.run_in_executor(
            None, lambda: self._post_send(recipient, text),
        )


if __name__ == "__main__":
    run_stdio_main(SignalAdapter)
