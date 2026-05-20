#!/usr/bin/env python3
"""LINE Messaging API sidecar channel adapter for LibreFang.

Replaces the former in-process Rust ``librefang-channels::line``
adapter (removed in this sidecar migration; same pattern as ntfy
#5224, telegram #5241, gotify #5263, mastodon #5264, bluesky #5277,
reddit #5281, twitch #5297, rocketchat #5298, discord #5299,
nextcloud #5301, slack #5302, webex #5309).

Behaviour parity with the Rust adapter (every assertion below has a
file/line citation against ``crates/librefang-channels/src/line.rs``
on the pre-migration tree):

* **Inbound HTTP webhook server**. ``BaseHTTPRequestHandler`` over a
  ``socketserver.ThreadingTCPServer`` accepts POST requests on a
  configurable path (``LINE_WEBHOOK_PATH``, default ``/webhook``) on a
  configurable port (``LINE_WEBHOOK_PORT``, default ``9090``). We
  subclass ``ThreadingTCPServer`` rather than
  ``http.server.ThreadingHTTPServer`` to skip the latter's
  ``socket.getfqdn()`` call in ``server_bind()`` (slow DNS lookup on
  startup); ``BaseHTTPRequestHandler`` works against any TCP-style
  server and our overridden ``log_message`` never reads
  ``server_name`` / ``server_port``. The Rust adapter
  mounted ``/channels/line/webhook`` on LibreFang's shared axum
  server (``line.rs:378-432``); the sidecar runs its own listener, so
  the public URL operators register at the LINE Developers console
  changes — see the migration commit / docs for the upgrade path.
* **X-Line-Signature verification**. ``Base64(HMAC-SHA256(secret,
  raw_body))`` over the *raw wire bytes* (``line.rs:229-250``).
  Mirrors the Rust adapter's two safety properties: constant-time
  compare against the decoded digest, and HMAC over the original
  bytes (not bytes round-tripped through ``serde_json::Value``,
  which would reorder keys and never match).
* **Event parsing**. Only ``type == "message"`` events with
  ``message.type == "text"`` (``line.rs:256-273``); other event
  types (follow, unfollow, postback, beacon, …) and non-text
  message types (sticker, image, video, …) are dropped — same as
  the Rust adapter.
* **Source-type → reply_to mapping** (``line.rs:280-290``):
  ``user`` → ``platform_id = userId, is_group = False``,
  ``group`` → ``platform_id = groupId, is_group = True``,
  ``room`` → ``platform_id = roomId, is_group = True``.
* **Slash-command routing**: ``/cmd args`` → ``Command`` (text
  otherwise; ``line.rs:295-308``).
* **Metadata preservation**: ``user_id``, ``reply_to``,
  ``reply_token`` (when present), ``source_type`` — every field
  the Rust adapter wrote at ``line.rs:310-329`` ships unchanged so
  downstream consumers that key on these names keep working.
* **Multi-bot ``account_id``** (``line.rs:80-84, 416-422``). When
  ``LINE_ACCOUNT_ID`` is set, it is injected into the inbound
  message metadata so the bridge can scope ``ApprovalRequested``
  delivery to the channel bound to the requesting agent (#5003).
* **REST send via** ``POST /v2/bot/message/push`` **with Bearer
  auth** (``line.rs:148-184``). ``MAX_MESSAGE_LEN = 5000``
  character chunking; image variant matches ``line.rs:464-490``
  (``originalContentUrl`` + ``previewImageUrl`` both set to the
  caller-supplied URL; caption sent as a separate text push if
  non-empty).
* **Token probe**: ``GET /v2/bot/info`` at startup with the bot
  access token (``line.rs:102-124``) — fail fast on a misconfigured
  token instead of waiting for the first outbound to fail.
* **ChannelType::Custom("line") preserved** as
  ``channel_type = "line"`` on the sidecar entry — existing routing
  / ``channel_role_mapping`` keys that reference ``line`` continue
  to resolve.

Improvements over the Rust adapter
==================================

1. **429 ``Retry-After`` honoured on outbound** (``line.rs:168-180``
   had no 429 handling — a throttled push returned ``Err`` and the
   chunked reply dropped on the floor). The sidecar parses
   ``Retry-After`` (with a ``RETRY_AFTER_DEFAULT_SECS = 30.0``
   floor), sleeps, and retries once before logging-and-continuing
   on the second 429. Same shape as the merged ``fix(channels):
   honour Retry-After across sidecar polling adapters (#5303)``.

2. **Inbound dedupe on ``message.id``**. LINE redelivers webhook
   events when the operator's endpoint fails (non-2xx or timeout)
   — the Rust handler at ``line.rs:413-427`` emitted every event
   unconditionally, so a transient downstream failure caused
   duplicate agent invocations. The sidecar dedupes locally on
   ``message.id`` with a bounded ``SEEN_MESSAGES_MAX = 10 000`` /
   ``SEEN_MESSAGES_EVICT = 5 000`` cap (same policy as reddit /
   rocketchat / nextcloud / webex).

3. **Explicit HTTP timeouts** on every ``urlopen`` call.
   ``urllib.request.urlopen`` has no default timeout; the Rust
   adapter relied on ``reqwest``'s default (none either). A hung
   LINE API would otherwise hang the worker thread forever. The
   sidecar passes ``timeout=SEND_TIMEOUT_SECS`` (15 s) on every
   call so a misbehaving endpoint trips an explicit error.

Stdlib-only: HTTPS via ``urllib.request``; HTTP webhook server via
``socketserver.ThreadingTCPServer`` with
``http.server.BaseHTTPRequestHandler``. The generic ``webhook``
sidecar uses the single-threaded ``http.server.HTTPServer``; LINE
upgrades to threading because the platform can fan out multiple
webhook deliveries concurrently and a single-threaded loop would
serialise them.

Configure via ``[[sidecar_channels]]``::

    [[sidecar_channels]]
    name = "line"
    command = "python3"
    args = ["-m", "librefang.sidecar.adapters.line"]
    channel_type = "line"
    [sidecar_channels.env]
    LINE_WEBHOOK_PORT = "9090"
    # LINE_WEBHOOK_PATH = "/webhook"
    # LINE_ACCOUNT_ID = "production"

Secrets via ``~/.librefang/secrets.env``: ``LINE_CHANNEL_SECRET``
and ``LINE_CHANNEL_ACCESS_TOKEN`` (both from the LINE Developers
console for your Messaging API channel).
"""
from __future__ import annotations

import asyncio
import base64
import binascii
import hashlib
import hmac
import http.server
import json
import os
import socketserver
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

# LINE constants — mirror crate::line defaults.
DEFAULT_API_BASE = "https://api.line.me"

# LINE's official text-message ceiling. Mirrors the Rust adapter's
# ``MAX_MESSAGE_LEN`` (see crates/librefang-channels/src/line.rs:39).
LINE_MSG_LIMIT = 5000

DEFAULT_WEBHOOK_PORT = 9090
DEFAULT_WEBHOOK_PATH = "/webhook"
DEFAULT_BIND_HOST = "0.0.0.0"

SEND_TIMEOUT_SECS = 15.0
MAX_BACKOFF_SECS = 60.0

# Default fallback when LINE 429s without a parseable ``Retry-After``
# header. Mirrors the rocketchat / nextcloud / mastodon / webex
# sidecars (#5303); 30 s is conservative enough that we don't
# immediately re-hit the bruteforce throttle.
RETRY_AFTER_DEFAULT_SECS = 30.0

# Bounded dedupe cap on ``message.id`` (Improvement #2). Same policy
# as reddit / rocketchat / nextcloud / webex. ``MAX`` is the
# high-water mark; when reached, evict the oldest ``EVICT`` entries
# (so the steady-state is between EVICT and MAX, not a flap around
# MAX).
SEEN_MESSAGES_MAX = 10_000
SEEN_MESSAGES_EVICT = 5_000

def _parse_retry_after(resp_hdrs: dict, *, default_secs: float) -> float:
    """Backwards-compat wrapper around
    :func:`librefang.sidecar.common.parse_retry_after`."""
    return _parse_retry_after_impl(
        resp_hdrs,
        default_secs=default_secs,
        floor_secs=1.0,
        max_secs=MAX_BACKOFF_SECS,
    )

def verify_line_signature(secret: bytes, body: bytes, signature: str) -> bool:
    """Verify ``X-Line-Signature`` using HMAC-SHA256 with
    constant-time comparison. Mirrors the Rust adapter's
    ``verify_line_signature`` (line.rs:229-250):

    * the digest is computed over the *raw wire bytes* (not
      bytes round-tripped through a JSON parser, which would
      reorder keys and never match);
    * the comparison is constant-time
      (``hmac.compare_digest`` mirrors the Rust adapter's
      ``ct_eq`` helper);
    * empty / non-base64 / whitespace-only signatures reject
      (regression for #3439 caught by the Rust test
      ``test_verify_line_signature_rejects_empty_signature``).
    """
    if not isinstance(signature, str):
        return False
    trimmed = signature.strip()
    if not trimmed:
        return False
    try:
        expected = base64.b64decode(trimmed, validate=True)
    except (binascii.Error, ValueError):
        return False
    digest = hmac.new(secret, body, hashlib.sha256).digest()
    return hmac.compare_digest(digest, expected)


def parse_line_event(
    event: dict,
    *,
    account_id: Optional[str] = None,
) -> Optional[dict]:
    """Pure-function port of the inbound parse path in
    ``crates/librefang-channels/src/line.rs`` lines 252-345.

    Returns a ``message`` event dict ready to ``emit``, or ``None``
    when the payload should be skipped (non-message event type,
    non-text message type, empty text, missing source).
    """
    if not isinstance(event, dict):
        return None
    if event.get("type") != "message":
        return None

    message = event.get("message")
    if not isinstance(message, dict):
        return None
    if message.get("type") != "text":
        return None

    text = message.get("text")
    if not isinstance(text, str) or not text:
        return None

    source = event.get("source")
    if not isinstance(source, dict):
        return None
    source_type = source.get("type") if isinstance(source.get("type"), str) else "user"
    user_id = source.get("userId")
    if not isinstance(user_id, str):
        user_id = ""

    # Determine the target (user, group, or room) for replies.
    # Mirrors the Rust adapter's source-type match at line.rs:280-290.
    if source_type == "group":
        reply_to = source.get("groupId")
        is_group = True
    elif source_type == "room":
        reply_to = source.get("roomId")
        is_group = True
    else:
        reply_to = user_id
        is_group = False
    if not isinstance(reply_to, str):
        reply_to = ""

    msg_id = message.get("id")
    if not isinstance(msg_id, str):
        msg_id = ""
    reply_token = event.get("replyToken")
    if not isinstance(reply_token, str):
        reply_token = ""

    # Slash-command routing.
    if text.startswith("/"):
        head, _, tail = text[1:].partition(" ")
        content = Content.command(head, tail.split() if tail else [])
    else:
        content = Content.text(text)

    metadata: dict[str, Any] = {
        "user_id": user_id,
        "reply_to": reply_to,
        "source_type": source_type,
    }
    if reply_token:
        metadata["reply_token"] = reply_token
    if account_id is not None:
        metadata["account_id"] = account_id

    # The Rust adapter at line.rs:333-336 used the group/room/user id
    # for ``sender.platform_id`` and the *user_id* string for the
    # display name (because the REST profile fetch was dead code) —
    # keep that exact mapping so existing routing keys round-trip
    # unchanged.
    return protocol.message(
        user_id=reply_to,
        user_name=user_id or "Unknown",
        content=content,
        message_id=msg_id or None,
        is_group=is_group,
        metadata=metadata,
    )


class LineAdapter(SidecarAdapter):
    # No native typing-indicator or reaction equivalent on LINE's
    # Messaging API — the Rust adapter's ``send_typing`` was a no-op
    # (line.rs:507-513) so we don't claim typing capability either.
    capabilities: list = []
    # 1:1 and group chats — same chat-room precedent as
    # discord / slack / webex. A public-broadcast surface like
    # mastodon / bluesky / reddit / nextcloud would set this True to
    # avoid echoing internal errors to every member; LINE chats are
    # identifiable user/group/room conversations, so the chat-room
    # default applies.
    suppress_error_responses: bool = False

    SCHEMA = Schema(
        name="line",
        display_name="LINE",
        description="LINE Messaging API adapter (out-of-process sidecar)",
        fields=[
            Field("LINE_CHANNEL_SECRET", "Channel Secret", "secret",
                  required=True,
                  placeholder="abc123..."),
            Field("LINE_CHANNEL_ACCESS_TOKEN", "Channel Access Token", "secret",
                  required=True,
                  placeholder="xyz789..."),
            Field("LINE_WEBHOOK_PORT", "Webhook Port",
                  "number",
                  placeholder=str(DEFAULT_WEBHOOK_PORT),
                  advanced=True),
            Field("LINE_WEBHOOK_PATH", "Webhook Path",
                  "text",
                  placeholder=DEFAULT_WEBHOOK_PATH,
                  advanced=True),
            Field("LINE_ACCOUNT_ID",
                  "Account ID (multi-bot routing)",
                  "text",
                  placeholder="production",
                  advanced=True),
        ],
    )

    def __init__(self) -> None:
        secret = os.environ.get("LINE_CHANNEL_SECRET", "").strip()
        token = os.environ.get("LINE_CHANNEL_ACCESS_TOKEN", "").strip()
        missing = []
        if not secret:
            missing.append("LINE_CHANNEL_SECRET")
        if not token:
            missing.append("LINE_CHANNEL_ACCESS_TOKEN")
        if missing:
            log.error("line required env vars missing", missing=missing)
            raise SystemExit(2)
        self.channel_secret = secret
        self.access_token = token
        self._secret_bytes = secret.encode("utf-8")

        port_raw = os.environ.get("LINE_WEBHOOK_PORT", "").strip()
        try:
            self.webhook_port = (
                int(port_raw) if port_raw else DEFAULT_WEBHOOK_PORT
            )
        except ValueError:
            log.warn("line LINE_WEBHOOK_PORT not an integer; using default",
                     value=port_raw, default=DEFAULT_WEBHOOK_PORT)
            self.webhook_port = DEFAULT_WEBHOOK_PORT
        path = os.environ.get("LINE_WEBHOOK_PATH", "").strip()
        self.webhook_path = path or DEFAULT_WEBHOOK_PATH
        if not self.webhook_path.startswith("/"):
            self.webhook_path = "/" + self.webhook_path
        self.bind_host = os.environ.get("LINE_BIND_HOST", "").strip() or DEFAULT_BIND_HOST

        acct = os.environ.get("LINE_ACCOUNT_ID", "").strip()
        self.account_id = acct or None

        # Test seam — overridable via env var so tests can point us
        # at a local mock without monkey-patching urllib globally.
        self.api_base = os.environ.get("LINE_API_BASE", "").strip() or DEFAULT_API_BASE

        # Improvement #2: bounded dedupe on inbound message.id.
        self._seen = _SeenSet(max_size=SEEN_MESSAGES_MAX, evict=SEEN_MESSAGES_EVICT)

        # Set by ``produce`` so ``on_shutdown`` can release the
        # listening socket cleanly.
        self._httpd: Optional[socketserver.ThreadingTCPServer] = None

    # ---- HTTP helpers ------------------------------------------------

    def _auth_headers(self, *, content_type: bool = False) -> dict:
        h = {
            "Authorization": f"Bearer {self.access_token}",
            "User-Agent": "librefang-line-sidecar/1 (https://librefang.org)",
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
        """Return True iff freshly seen. Shim around :class:`librefang.sidecar.common.SeenSet`."""
        return self._seen.mark(message_id)

    # ---- REST: auth + outbound send ---------------------------------

    def _validate_token(self) -> str:
        """``GET /v2/bot/info`` → bot display name. Raises
        ``RuntimeError`` on any non-200 response so the producer loop
        can log and exit (the daemon supervisor restarts us)."""
        status, body, raw, resp_hdrs = self._http(
            f"{self.api_base}/v2/bot/info",
            headers=self._auth_headers(),
        )
        if status == 429:
            wait = _parse_retry_after(
                resp_hdrs, default_secs=RETRY_AFTER_DEFAULT_SECS,
            )
            log.warn("line /v2/bot/info 429; will retry after",
                     retry_after_secs=wait)
            time.sleep(wait)
            status, body, raw, resp_hdrs = self._http(
                f"{self.api_base}/v2/bot/info",
                headers=self._auth_headers(),
            )
        if status != 200 or not isinstance(body, dict):
            snippet = raw[:200].decode("utf-8", "replace") if raw else ""
            raise RuntimeError(
                f"line /v2/bot/info failed (status={status}): {snippet}"
            )
        display = body.get("displayName")
        if not isinstance(display, str) or not display:
            display = "LINE Bot"
        return display

    def _post_push(self, payload: dict) -> None:
        """POST /v2/bot/message/push with one chunk. Honours 429
        ``Retry-After`` and retries once (Improvement #1). On the
        second 429 / non-2xx we log and continue — matches the
        webex / slack fail-open behaviour so a single throttled
        chunk doesn't drop the rest of the reply."""
        url = f"{self.api_base}/v2/bot/message/push"
        body = json.dumps(payload).encode("utf-8")
        status, resp, raw, resp_hdrs = self._http(
            url, method="POST", body=body,
            headers=self._auth_headers(content_type=True),
        )
        if status == 429:
            wait = _parse_retry_after(
                resp_hdrs, default_secs=RETRY_AFTER_DEFAULT_SECS,
            )
            log.warn("line POST /v2/bot/message/push 429; sleeping then "
                     "retrying once",
                     retry_after_secs=wait)
            time.sleep(wait)
            status, resp, raw, resp_hdrs = self._http(
                url, method="POST", body=body,
                headers=self._auth_headers(content_type=True),
            )
        if status >= 300:
            snippet = raw[:200].decode("utf-8", "replace") if raw else ""
            log.warn("line POST /v2/bot/message/push failed",
                     status=status, body=snippet)

    def _push_text(self, to: str, text: str) -> None:
        for chunk in _split_message(text, LINE_MSG_LIMIT):
            self._post_push({
                "to": to,
                "messages": [{"type": "text", "text": chunk}],
            })

    def _push_image(self, to: str, url: str,
                    caption: Optional[str]) -> None:
        """LINE image messages take an ``originalContentUrl`` plus a
        ``previewImageUrl``. Mirrors the Rust adapter's image branch
        (line.rs:464-490): both URLs are populated with the caller-
        supplied URL, and any non-empty caption is sent as a
        follow-up text push."""
        if url:
            self._post_push({
                "to": to,
                "messages": [{
                    "type": "image",
                    "originalContentUrl": url,
                    "previewImageUrl": url,
                }],
            })
        if caption:
            self._push_text(to, caption)

    # ---- inbound webhook server ------------------------------------

    def _handle_webhook_body(
        self,
        body: bytes,
        signature: str,
        emit: Callable[[dict], None],
    ) -> int:
        """Verify + parse one webhook POST body. Returns the HTTP
        status code to send back. Extracted so tests can drive it
        without spinning up a real ``ThreadingTCPServer``."""
        if not verify_line_signature(self._secret_bytes, body, signature):
            log.warn("line: invalid webhook signature")
            return 401
        try:
            body_json = json.loads(body.decode("utf-8"))
        except (ValueError, UnicodeDecodeError):
            return 400
        if not isinstance(body_json, dict):
            return 400
        events = body_json.get("events")
        if not isinstance(events, list):
            # LINE sends ``{"destination": "...", "events": []}`` on
            # webhook URL verification — empty/missing events is OK.
            return 200
        for event in events:
            if not isinstance(event, dict):
                continue
            message = event.get("message")
            message_id = (
                message.get("id")
                if isinstance(message, dict)
                else None
            )
            if (isinstance(message_id, str) and message_id
                    and not self._mark_seen(message_id)):
                # Improvement #2: drop the duplicate before parsing.
                continue
            parsed = parse_line_event(event, account_id=self.account_id)
            if parsed is not None:
                emit(parsed)
        return 200

    def _make_handler_class(
        self,
        emit: Callable[[dict], None],
    ) -> type:
        """Build a ``BaseHTTPRequestHandler`` subclass closed over
        ``self`` + ``emit``. ``BaseHTTPRequestHandler.__init__``
        takes no user args, so a closure is the cleanest way to
        thread state into the handler."""
        adapter = self

        class _LineWebhookHandler(http.server.BaseHTTPRequestHandler):
            # Cap request size at 4 MiB — way over any realistic
            # LINE webhook payload, and keeps a stray giant POST
            # from exhausting memory.
            _MAX_BODY_BYTES = 4 * 1024 * 1024

            def do_POST(self) -> None:  # noqa: N802 — stdlib API
                if self.path.split("?", 1)[0] != adapter.webhook_path:
                    self.send_response(404)
                    self.end_headers()
                    return
                try:
                    cl = int(self.headers.get("Content-Length", "0") or 0)
                except ValueError:
                    cl = 0
                if cl > self._MAX_BODY_BYTES:
                    self.send_response(413)
                    self.end_headers()
                    return
                body = self.rfile.read(cl) if cl > 0 else b""
                signature = self.headers.get("X-Line-Signature", "")
                status = adapter._handle_webhook_body(body, signature, emit)
                self.send_response(status)
                self.end_headers()
                if status == 200:
                    self.wfile.write(b"OK")

            def log_message(self, fmt: str, *args: Any) -> None:  # noqa: A003
                # Silence stdlib's default per-request stderr access
                # log; failures are logged via ``log.warn`` from
                # ``_handle_webhook_body``.
                return

        return _LineWebhookHandler

    def _serve_forever(
        self,
        emit: Callable[[dict], None],
        ready: threading.Event,
    ) -> None:
        """Worker-thread entry point. Validates the token, binds the
        listening socket, signals ``ready`` (so ``produce`` knows
        whether we successfully started), and runs the HTTP server
        until ``self._httpd.shutdown()`` is called."""
        try:
            display = self._validate_token()
            log.info("line authenticated", display_name=display)
        except Exception as e:  # noqa: BLE001
            log.error("line token validation failed", error=str(e))
            ready.set()
            return
        handler_cls = self._make_handler_class(emit)

        class _ReusingServer(socketserver.ThreadingTCPServer):
            allow_reuse_address = True
            daemon_threads = True

        try:
            httpd = _ReusingServer(
                (self.bind_host, self.webhook_port), handler_cls,
            )
        except OSError as e:
            log.error("line webhook bind failed",
                      host=self.bind_host,
                      port=self.webhook_port,
                      error=str(e))
            ready.set()
            return

        self._httpd = httpd
        ready.set()
        log.info("line webhook listening",
                 host=self.bind_host,
                 port=self.webhook_port,
                 path=self.webhook_path)
        try:
            httpd.serve_forever()
        finally:
            try:
                httpd.server_close()
            except Exception:  # noqa: BLE001
                pass

    # ---- public sidecar surface --------------------------------------

    async def produce(self, emit: Callable[[dict], None]) -> None:
        loop = asyncio.get_event_loop()
        ready = threading.Event()
        t = threading.Thread(
            target=self._serve_forever,
            args=(emit, ready),
            name="line-webhook",
            daemon=True,
        )
        t.start()
        # Wait for the server to bind (or fail). Polled in the loop
        # rather than blocked on the threading.Event so the asyncio
        # side stays responsive to cancellation.
        while not ready.is_set():
            await asyncio.sleep(0.05)
        if self._httpd is None:
            # Bind / token-validation failed — surface as a producer
            # crash so the daemon supervisor restarts us. Without
            # this the produce coroutine would silently return and
            # the daemon would treat the adapter as "running but
            # silent", which is worse than a restart loop.
            raise RuntimeError(
                "line sidecar failed to start its webhook server; "
                "see prior log lines for the underlying error"
            )
        # The HTTP server runs in its background thread; this
        # coroutine just blocks until the framework cancels it on
        # shutdown.
        try:
            while True:
                await asyncio.sleep(3600)
        except asyncio.CancelledError:
            self._shutdown_server()
            raise

    def _shutdown_server(self) -> None:
        httpd = self._httpd
        if httpd is None:
            return
        try:
            # ``shutdown`` blocks until ``serve_forever`` returns;
            # call it in a thread to avoid deadlocking the asyncio
            # loop (``shutdown`` from inside the serving thread is
            # the documented deadlock case).
            threading.Thread(
                target=httpd.shutdown, name="line-shutdown", daemon=True,
            ).start()
        except Exception:  # noqa: BLE001
            pass

    async def on_shutdown(self) -> None:
        self._shutdown_server()

    async def on_send(self, cmd) -> None:
        to = (
            cmd.channel_id
            or (cmd.user.get("platform_id") if cmd.user else "")
            or ""
        )
        if not to:
            log.warn("line on_send: empty platform_id, dropping")
            return

        content = cmd.content
        text = cmd.text or ""
        loop = asyncio.get_event_loop()

        if isinstance(content, dict):
            if "Image" in content:
                img = content.get("Image") or {}
                url = img.get("url") if isinstance(img, dict) else None
                caption = img.get("caption") if isinstance(img, dict) else None
                if not isinstance(url, str):
                    url = ""
                if not isinstance(caption, str):
                    caption = None
                await loop.run_in_executor(
                    None,
                    lambda: self._push_image(to, url, caption),
                )
                return
            if "Text" in content:
                # Fall through to the text push below.
                pass
            else:
                # Mirrors the Rust adapter's fallback at
                # line.rs:499-502: anything other than text/image
                # becomes a placeholder push so the sender at least
                # sees that something happened.
                await loop.run_in_executor(
                    None,
                    lambda: self._push_text(
                        to, "(Unsupported content type)",
                    ),
                )
                return

        await loop.run_in_executor(
            None, lambda: self._push_text(to, text),
        )


if __name__ == "__main__":
    run_stdio_main(LineAdapter)
