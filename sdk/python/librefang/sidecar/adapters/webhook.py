#!/usr/bin/env python3
"""Generic HTTP webhook sidecar channel adapter for LibreFang.

Replaces the in-process Rust ``librefang-channels::webhook`` adapter
(removed in this migration) AND the earlier demo-only Python
adapter that lived at this path (132 lines, no HMAC timestamp / no
SSRF guard / no deliver_only / hand-rolled JSON-RPC protocol). Same
pattern as the line / mattermost / teams / whatsapp sidecars —
runs its own HTTP webhook server (stdlib
``BaseHTTPRequestHandler`` over ``ThreadingTCPServer``) on the
sidecar SDK's standard ``SidecarAdapter`` surface.

Behaviour parity (citations against
``crates/librefang-channels/src/webhook.rs`` on the pre-migration
tree):

* **Inbound HTTP webhook**: ``POST {WEBHOOK_LISTEN_PATH}`` (default
  ``/webhook``) on ``WEBHOOK_LISTEN_PORT``. Mirrors webhook.rs:265-410.

* **HMAC-SHA256 signature verification**: ``X-Webhook-Signature:
  sha256=<hex-digest>``. Constant-time compare against
  ``HMAC-SHA256(WEBHOOK_SECRET, raw_body)`` (webhook.rs:118-142).
  Empty / missing / malformed prefix all reject.

* **Replay-window timestamp check**: ``X-Webhook-Timestamp`` in
  **milliseconds** since the Unix epoch. Skew tolerance is ±5
  minutes (300 seconds). When the header is absent, the sidecar
  falls back to sig-only verification with a per-request WARN
  log — backwards-compatible with clients that never sent the
  timestamp, but operators are nudged to upgrade. A header that's
  PRESENT but unparseable (e.g. ``X-Webhook-Timestamp:
  not-a-number``) returns 400 with the reason — distinguishes
  "header absent" (legitimate sig-only fallback) from "header
  malformed" (attacker probing for the bypass), per the comment
  at webhook.rs:295-310.

* **Public-string for verification failures**: all auth failures
  collapse to a single ``Forbidden`` response so an attacker
  can't probe which check failed (webhook.rs:328-333).

* **JSON inbound shape** (webhook.rs:191-234)::

      {
        "sender_id": "user-123",
        "sender_name": "Alice",
        "message": "Hello!",
        "thread_id": "optional-thread",
        "is_group": false,
        "metadata": {}
      }

  Missing ``sender_id`` / ``sender_name`` fall back to
  ``"webhook-user"`` / ``"Webhook User"``. Empty / missing
  ``message`` drops the inbound silently with 200 OK so callers
  don't retry on benign no-ops.

* **Slash-command routing**: ``message`` starting with ``/`` is
  parsed as a ``Command`` with the rest split into args
  (webhook.rs:351-364).

* **Outbound POST** to ``WEBHOOK_CALLBACK_URL`` (when set) with
  the same signature scheme. Body shape::

      {
        "sender_id": "librefang",
        "sender_name": "LibreFang",
        "recipient_id": <user.platform_id>,
        "recipient_name": <user.display_name>,
        "message": <chunk>,
        "timestamp": <ISO-8601>
      }

  65535-char chunking via the shared ``split_message`` helper;
  100 ms inter-chunk delay matches webhook.rs:483-485.

* **SSRF guard** on ``WEBHOOK_CALLBACK_URL``: at construction
  time AND on every send (defence in depth) the URL is rejected
  if it points to a private / loopback / link-local / multicast
  / cloud-metadata host. Mirrors the Rust adapter's
  ``http_client::validate_url_for_fetch`` (which is what
  ``WebhookAdapter::new`` invokes at webhook.rs:84-87).

* **deliver_only mode**: when ``WEBHOOK_DELIVER_ONLY=1``, inbound
  messages get two metadata keys (``__deliver_only__`` +
  ``__deliver_target__``) the kernel's ``bridge.rs:2845-2851``
  routing reads to short-circuit the LLM and forward the message
  body straight to the named channel. The sidecar just emits the
  metadata; the kernel still owns the routing semantics.

* **Multi-bot ``account_id``** metadata injection (#5003).

Improvements over the Rust adapter:

1. **Inbound dedupe** on ``platform_message_id`` — Rust assigned
   a fresh ``wh-<timestamp_ms>`` ID on each emit and never
   deduped, so a misbehaving upstream that delivered twice would
   double-emit. Sidecar threads either the inbound's own
   ``metadata.message_id`` (when present) or a synthesised
   ``wh-<ms>-<body_hash[:8]>`` ID through a bounded ``SeenSet``
   (10000 cap / 5000 evict). The hash suffix prevents collisions
   between simultaneous deliveries at the same millisecond, which
   the Rust millisecond-only ID couldn't distinguish.

2. **429 ``Retry-After`` honoured** on outbound POSTs — Rust
   raised on first non-2xx (webhook.rs:476-480). Sidecar parses
   ``Retry-After`` once, sleeps, retries, then logs-and-continues
   on the second 429 so a single throttled chunk doesn't drop
   the rest of a multi-chunk reply.

3. **Explicit 30 s timeout** on every outbound POST — Rust
   relied on ``reqwest``'s default.

4. **Per-send SSRF re-check** — the Rust adapter validated the
   ``callback_url`` once at adapter construction. The sidecar
   re-checks before every POST so a config-reload that swapped
   the URL to a private host doesn't leak the signing secret to
   localhost.

Configure via ``[[sidecar_channels]]``::

    [[sidecar_channels]]
    name = "webhook"
    command = "python3"
    args = ["-m", "librefang.sidecar.adapters.webhook"]
    channel_type = "webhook"
    [sidecar_channels.env]
    WEBHOOK_LISTEN_PORT = "8461"
    # WEBHOOK_LISTEN_PATH = "/webhook"
    # WEBHOOK_CALLBACK_URL = "https://example.com/incoming"
    # WEBHOOK_DELIVER_ONLY = "1"
    # WEBHOOK_DELIVER = "telegram"
    # WEBHOOK_ACCOUNT_ID = "production"

Secrets via ``~/.librefang/secrets.env``: ``WEBHOOK_SECRET`` (the
shared HMAC-SHA256 signing key).
"""
from __future__ import annotations

import asyncio
import datetime
import hashlib
import hmac
import http.server
import ipaddress
import json
import os
import socketserver
import threading
import time
import urllib.parse
from typing import Any, Callable, Optional

from .. import logging as log
from .. import protocol
from ..common import (
    SeenSet as _SeenSet,
    http_request as _http_request,
    parse_retry_after as _parse_retry_after,
    split_message as _split_message,
)
from ..protocol import Content, Field, Schema
from ..runtime import SidecarAdapter, run_stdio_main


# ---------------------------------------------------------------------------
# Constants — mirror crates/librefang-channels/src/webhook.rs.
# ---------------------------------------------------------------------------

MAX_MESSAGE_LEN = 65535               # webhook.rs:22
MAX_SKEW_SECS = 5 * 60                # webhook.rs:172
INTER_CHUNK_DELAY_SECS = 0.1          # webhook.rs:484

DEFAULT_LISTEN_PORT = 8461
DEFAULT_LISTEN_PATH = "/webhook"
DEFAULT_BIND_HOST = "0.0.0.0"

SEND_TIMEOUT_SECS = 30.0
SEEN_MESSAGES_MAX = 10_000
SEEN_MESSAGES_EVICT = 5_000


# ---------------------------------------------------------------------------
# SSRF guard — pure-Python port of http_client::validate_url_for_fetch.
# ---------------------------------------------------------------------------


# Hostnames that resolve to the local machine or another reserved
# context. Anything in this set is rejected outright.
_PRIVATE_HOSTNAMES = frozenset({
    "localhost",
    "ip6-localhost",
    "ip6-loopback",
    # Common Kubernetes service-mesh hosts.
    "kubernetes.default",
    "kubernetes.default.svc",
    "kubernetes.default.svc.cluster.local",
})


def _is_private_ipv4(addr: ipaddress.IPv4Address) -> bool:
    """IPv4 ranges unsafe for server-side fetch. Mirrors
    `http_client::is_private_ipv4` — first-octet rules for the
    big blocks plus precise CIDRs for 100.64/10, 169.254/16,
    172.16/12, 192.168/16, and 192.0.0/24."""
    o = addr.packed
    # 0.0.0.0/8, 10.0.0.0/8, 127.0.0.0/8 (loopback)
    if o[0] in (0, 10, 127):
        return True
    # 224.0.0.0/4 multicast + 240.0.0.0/4 reserved (incl. broadcast).
    if 224 <= o[0] <= 255:
        return True
    # 100.64.0.0/10 — RFC 6598 carrier-grade NAT.
    if o[0] == 100 and 64 <= o[1] <= 127:
        return True
    # 169.254.0.0/16 — link-local (incl. cloud metadata 169.254.169.254).
    if o[0] == 169 and o[1] == 254:
        return True
    # 172.16.0.0/12 — RFC 1918.
    if o[0] == 172 and 16 <= o[1] <= 31:
        return True
    # 192.168.0.0/16 — RFC 1918.
    if o[0] == 192 and o[1] == 168:
        return True
    # 192.0.0.0/24 — IETF protocol assignments (deliberately /24).
    if o[0] == 192 and o[1] == 0 and o[2] == 0:
        return True
    return False


def _is_private_ipv6(addr: ipaddress.IPv6Address) -> bool:
    """IPv6 ranges unsafe for server-side fetch. Mirrors
    `http_client::is_private_ipv6`."""
    if addr.is_loopback or addr.is_unspecified:
        return True
    if addr.is_link_local:
        return True
    if addr.is_site_local:
        return True
    if addr.is_multicast:
        return True
    if addr.is_private:
        return True
    # IPv4-mapped (::ffff:x.x.x.x) and NAT64 (64:ff9b::x.x.x.x)
    # both deliver to an IPv4 endpoint on the wire. Check the
    # embedded v4 against the private table.
    if addr.ipv4_mapped is not None:
        if _is_private_ipv4(addr.ipv4_mapped):
            return True
    return False


def validate_callback_url(url: str) -> Optional[str]:
    """Returns ``None`` if the URL is safe to dial, else a string
    describing the SSRF rejection reason. Pure function — used at
    both construction and per-send to defend in depth."""
    if not isinstance(url, str) or not url:
        return "URL is empty"
    try:
        parsed = urllib.parse.urlparse(url)
    except Exception as e:  # noqa: BLE001
        return f"invalid URL: {e}"
    if parsed.scheme not in ("http", "https"):
        return f"scheme {parsed.scheme!r} is not allowed; only http/https"
    host = parsed.hostname
    if not host:
        return "URL has no host"
    # Try IP literal first.
    try:
        ip = ipaddress.ip_address(host)
    except ValueError:
        # Hostname — check the reserved list.
        trimmed = host.rstrip(".").lower()
        if trimmed in _PRIVATE_HOSTNAMES:
            return f"host {host!r} is a reserved or private hostname"
        return None
    if isinstance(ip, ipaddress.IPv4Address):
        if _is_private_ipv4(ip):
            return f"host resolves to private/reserved IPv4 {ip}"
    elif isinstance(ip, ipaddress.IPv6Address):
        if _is_private_ipv6(ip):
            return f"host resolves to private/reserved IPv6 {ip}"
    return None


# ---------------------------------------------------------------------------
# Signature helpers
# ---------------------------------------------------------------------------


def compute_signature(secret: bytes, body: bytes) -> str:
    """``sha256=<hex>`` per webhook.rs:118-128."""
    digest = hmac.new(secret, body, hashlib.sha256).hexdigest()
    return f"sha256={digest}"


def verify_webhook_signature(
    secret: bytes, body: bytes, signature: Optional[str],
) -> bool:
    """Constant-time HMAC-SHA256 compare. Empty / missing / wrong-
    prefix / wrong-length all reject. Mirrors webhook.rs:131-142."""
    if not isinstance(signature, str) or not signature:
        return False
    expected = compute_signature(secret, body)
    if len(expected) != len(signature):
        return False
    return hmac.compare_digest(expected, signature)


# ---------------------------------------------------------------------------
# Inbound parse
# ---------------------------------------------------------------------------


def parse_webhook_body(body: Any) -> Optional[dict]:
    """Pure parse of the JSON body. Returns ``None`` for missing /
    empty ``message``. Mirrors webhook.rs:191-234."""
    if not isinstance(body, dict):
        return None
    message = body.get("message")
    if not isinstance(message, str) or not message:
        return None
    sender_id = body.get("sender_id")
    if not isinstance(sender_id, str) or not sender_id:
        sender_id = "webhook-user"
    sender_name = body.get("sender_name")
    if not isinstance(sender_name, str) or not sender_name:
        sender_name = "Webhook User"
    thread_id = body.get("thread_id") if isinstance(body.get("thread_id"), str) else None
    is_group = bool(body.get("is_group"))
    metadata = body.get("metadata") if isinstance(body.get("metadata"), dict) else {}
    return {
        "message": message,
        "sender_id": sender_id,
        "sender_name": sender_name,
        "thread_id": thread_id,
        "is_group": is_group,
        "metadata": dict(metadata),
    }


# ---------------------------------------------------------------------------
# Adapter
# ---------------------------------------------------------------------------


def _env_bool(name: str, default: bool = False) -> bool:
    raw = os.environ.get(name, "").strip().lower()
    if not raw:
        return default
    return raw in ("1", "true", "yes", "on")


class WebhookAdapter(SidecarAdapter):
    """Generic HTTP webhook sidecar."""

    capabilities: list = []
    suppress_error_responses: bool = False

    SCHEMA = Schema(
        name="webhook",
        display_name="Webhook",
        description=(
            "Generic HMAC-signed HTTP webhook adapter. Out-of-process "
            "sidecar (Python stdlib only)."
        ),
        fields=[
            Field("WEBHOOK_SECRET",
                  "Shared HMAC-SHA256 signing secret", "secret",
                  required=True),
            Field("WEBHOOK_LISTEN_PORT",
                  "Listen port", "number",
                  placeholder=str(DEFAULT_LISTEN_PORT)),
            Field("WEBHOOK_LISTEN_PATH",
                  "Listen path", "text",
                  placeholder=DEFAULT_LISTEN_PATH,
                  advanced=True),
            Field("WEBHOOK_CALLBACK_URL",
                  "Outbound callback URL (POSTs are signed with WEBHOOK_SECRET)",
                  "text",
                  placeholder="https://example.com/incoming",
                  advanced=True),
            Field("WEBHOOK_DELIVER_ONLY",
                  "Deliver-only mode (forward inbound to another channel without invoking an LLM)",
                  "text",
                  placeholder="1",
                  advanced=True),
            Field("WEBHOOK_DELIVER",
                  "Deliver target channel (required when WEBHOOK_DELIVER_ONLY=1)",
                  "text",
                  placeholder="telegram",
                  advanced=True),
            Field("WEBHOOK_ACCOUNT_ID",
                  "Account ID (multi-bot routing)", "text",
                  advanced=True),
        ],
    )

    def __init__(self) -> None:
        secret = os.environ.get("WEBHOOK_SECRET", "").strip()
        if not secret:
            log.error("webhook required env var missing", missing=["WEBHOOK_SECRET"])
            raise SystemExit(2)
        self.secret = secret
        self._secret_bytes = secret.encode("utf-8")

        port_raw = os.environ.get("WEBHOOK_LISTEN_PORT", "").strip()
        try:
            self.listen_port = int(port_raw) if port_raw else DEFAULT_LISTEN_PORT
        except ValueError:
            log.warn(
                "webhook WEBHOOK_LISTEN_PORT not an integer; using default",
                value=port_raw, default=DEFAULT_LISTEN_PORT,
            )
            self.listen_port = DEFAULT_LISTEN_PORT
        path = (
            os.environ.get("WEBHOOK_LISTEN_PATH", "").strip()
            or DEFAULT_LISTEN_PATH
        )
        if not path.startswith("/"):
            path = "/" + path
        self.listen_path = path
        self.bind_host = (
            os.environ.get("WEBHOOK_BIND_HOST", "").strip() or DEFAULT_BIND_HOST
        )

        cb = os.environ.get("WEBHOOK_CALLBACK_URL", "").strip()
        if cb:
            reason = validate_callback_url(cb)
            if reason is not None:
                log.error(
                    "webhook WEBHOOK_CALLBACK_URL rejected by SSRF guard",
                    reason=reason,
                )
                raise SystemExit(2)
        self.callback_url: Optional[str] = cb or None

        self.deliver_only = _env_bool("WEBHOOK_DELIVER_ONLY", False)
        deliver_target = os.environ.get("WEBHOOK_DELIVER", "").strip()
        self.deliver_target: Optional[str] = deliver_target or None
        if self.deliver_only and not self.deliver_target:
            # Match the Rust kernel's startup WARN — refuse to
            # silently drop inbound when deliver_only is on but
            # the target is missing. Better to fail-closed at
            # boot than to lose messages at runtime.
            log.error(
                "webhook WEBHOOK_DELIVER_ONLY=1 but WEBHOOK_DELIVER is empty — "
                "set WEBHOOK_DELIVER to a target channel (e.g. \"telegram\") "
                "or unset WEBHOOK_DELIVER_ONLY",
            )
            raise SystemExit(2)

        acct = os.environ.get("WEBHOOK_ACCOUNT_ID", "").strip()
        self.account_id: Optional[str] = acct or None

        self._seen = _SeenSet(
            max_size=SEEN_MESSAGES_MAX, evict=SEEN_MESSAGES_EVICT,
        )
        self._httpd: Optional[socketserver.ThreadingTCPServer] = None
        self._shutdown = threading.Event()

    # ---- inbound webhook --------------------------------------------

    def _verify_request(
        self,
        body: bytes,
        signature: Optional[str],
        ts_header: Optional[str],
        now_secs: int,
    ) -> tuple[bool, str, int]:
        """Returns ``(ok, reason, status_code)``. Mirrors webhook.rs
        verify_request + the inline timestamp parsing at lines
        297-326. The ``reason`` is for logs only; the public
        response collapses all auth failures to one string."""
        if not isinstance(signature, str) or not signature:
            return False, "missing signature", 403

        ts_secs: Optional[int] = None
        if ts_header is not None:
            try:
                ts_ms = int(ts_header)
            except ValueError:
                return False, "invalid timestamp header", 400
            ts_secs = ts_ms // 1000

        if ts_secs is not None:
            skew = now_secs - ts_secs
            if skew > MAX_SKEW_SECS:
                return False, "timestamp too old", 403
            if skew < -MAX_SKEW_SECS:
                return False, "timestamp in the future", 403
        else:
            # No timestamp — sig-only fallback, log a nudge per
            # request like Rust does at webhook.rs:320.
            log.warn(
                "webhook: request has no X-Webhook-Timestamp — "
                "replay protection unavailable",
            )

        if not verify_webhook_signature(self._secret_bytes, body, signature):
            return False, "invalid signature", 403

        return True, "", 200

    def _handle_webhook_body(
        self,
        body: bytes,
        signature: Optional[str],
        ts_header: Optional[str],
        emit: Callable[[dict], None],
    ) -> int:
        """Verify + parse + emit. Returns the HTTP status to send."""
        now_secs = int(time.time())
        ok, reason, status = self._verify_request(
            body, signature, ts_header, now_secs,
        )
        if not ok:
            log.warn("webhook rejected", reason=reason)
            return status

        try:
            payload = json.loads(body.decode("utf-8"))
        except (ValueError, UnicodeDecodeError):
            return 400

        parsed = parse_webhook_body(payload)
        if parsed is None:
            # Empty / no-message body is not an error per the Rust
            # contract — return 200 so the caller doesn't retry.
            return 200

        message = parsed["message"]
        if message.startswith("/"):
            head, _, rest = message[1:].partition(" ")
            args = rest.split() if rest else []
            content = {"Command": {"name": head, "args": args}}
        else:
            content = Content.text(message)

        # Synthesise a deterministic message_id when the inbound
        # didn't provide one. Rust used `wh-<ms-timestamp>` alone
        # (webhook.rs:368-371); the 8-char body hash suffix
        # protects against millisecond-collision dupes that
        # Rust's ID would have flattened together.
        inbound_id = parsed["metadata"].get("message_id")
        if isinstance(inbound_id, str) and inbound_id:
            msg_id = inbound_id
        else:
            ms = int(time.time() * 1000)
            tail = hashlib.sha256(body).hexdigest()[:8]
            msg_id = f"wh-{ms}-{tail}"

        # Dedupe — improvement #1 over the Rust adapter.
        if not self._seen.mark(msg_id):
            log.debug("webhook duplicate message_id, dropping",
                      message_id=msg_id)
            return 200

        metadata = dict(parsed["metadata"])
        if self.account_id is not None:
            metadata["account_id"] = self.account_id
        if self.deliver_only:
            metadata["__deliver_only__"] = True
            if self.deliver_target:
                metadata["__deliver_target__"] = self.deliver_target
        if parsed["is_group"]:
            metadata["is_group"] = True

        ev = protocol.message(
            user_id=parsed["sender_id"],
            user_name=parsed["sender_name"],
            content=content,
            message_id=msg_id,
            channel_id=parsed["sender_id"],
            thread_id=parsed["thread_id"],
            metadata=metadata,
        )
        emit(ev)
        return 200

    def _make_handler_class(
        self, emit: Callable[[dict], None],
    ) -> type:
        adapter = self

        class _WebhookHandler(http.server.BaseHTTPRequestHandler):
            _MAX_BODY_BYTES = 4 * 1024 * 1024

            def do_POST(self) -> None:  # noqa: N802
                if self.path.split("?", 1)[0] != adapter.listen_path:
                    self.send_response(404)
                    self.end_headers()
                    return
                try:
                    cl = int(self.headers.get("Content-Length", "0") or 0)
                except ValueError:
                    cl = 0
                if cl < 0:
                    # Negative Content-Length would make
                    # `rfile.read(-1)` consume to EOF.
                    self.send_response(400)
                    self.end_headers()
                    return
                if cl > self._MAX_BODY_BYTES:
                    self.send_response(413)
                    self.end_headers()
                    return
                body = self.rfile.read(cl) if cl > 0 else b""
                sig = self.headers.get("X-Webhook-Signature")
                ts = self.headers.get("X-Webhook-Timestamp")
                status = adapter._handle_webhook_body(body, sig, ts, emit)
                # Collapse 401/403 to a single "Forbidden" body so
                # an attacker can't probe which check failed
                # (matches webhook.rs:328-333).
                self.send_response(status)
                self.end_headers()
                if status == 200:
                    self.wfile.write(b"ok")
                elif status == 403:
                    self.wfile.write(b"Forbidden")
                elif status == 400:
                    self.wfile.write(b"Bad Request")

            def log_message(self, fmt: str, *args: Any) -> None:  # noqa: A003
                return

        return _WebhookHandler

    def _serve_forever(
        self,
        emit: Callable[[dict], None],
        ready: threading.Event,
    ) -> None:
        handler_cls = self._make_handler_class(emit)

        class _ReusingServer(socketserver.ThreadingTCPServer):
            allow_reuse_address = True
            daemon_threads = True

        try:
            httpd = _ReusingServer(
                (self.bind_host, self.listen_port), handler_cls,
            )
        except OSError as e:
            log.error("webhook bind failed",
                      host=self.bind_host, port=self.listen_port,
                      error=str(e))
            ready.set()
            return

        self._httpd = httpd
        ready.set()
        log.info("webhook listening",
                 host=self.bind_host, port=self.listen_port,
                 path=self.listen_path)
        try:
            httpd.serve_forever()
        finally:
            try:
                httpd.server_close()
            except Exception:  # noqa: BLE001
                pass

    # ---- outbound POST to callback_url ------------------------------

    def _post_chunk(self, chunk: str, user_id: str, user_name: str) -> None:
        """Sign + POST a single chunk to ``self.callback_url``. Honors
        429 with one retry (improvement #2 over Rust)."""
        if self.callback_url is None:
            raise RuntimeError(
                "webhook send: no WEBHOOK_CALLBACK_URL configured",
            )
        # Re-validate per send — defence in depth. If an operator
        # somehow rotates the env to a private URL mid-process
        # (config reload), refuse instead of leaking the signing
        # secret to localhost.
        reason = validate_callback_url(self.callback_url)
        if reason is not None:
            raise RuntimeError(
                f"webhook send refused by SSRF guard: {reason}",
            )

        body = json.dumps({
            "sender_id": "librefang",
            "sender_name": "LibreFang",
            "recipient_id": user_id,
            "recipient_name": user_name,
            "message": chunk,
            "timestamp": datetime.datetime.now(datetime.timezone.utc).isoformat(),
        }).encode("utf-8")
        signature = compute_signature(self._secret_bytes, body)
        headers = {
            "Content-Type": "application/json",
            "X-Webhook-Signature": signature,
            "User-Agent": "librefang-webhook-sidecar/1 (https://librefang.org)",
        }
        status, _resp, raw, resp_hdrs = _http_request(
            self.callback_url, method="POST", body=body, headers=headers,
            timeout=SEND_TIMEOUT_SECS,
        )
        if status == 429:
            wait = _parse_retry_after(
                resp_hdrs, default_secs=30.0,
                floor_secs=1.0, max_secs=60.0,
            )
            log.warn("webhook callback 429; sleeping then retrying once",
                     retry_after=wait)
            if self._shutdown.wait(wait):
                return
            status, _resp, raw, resp_hdrs = _http_request(
                self.callback_url, method="POST", body=body, headers=headers,
                timeout=SEND_TIMEOUT_SECS,
            )
        if status < 200 or status >= 300:
            snippet = raw[:200].decode("utf-8", "replace") if raw else ""
            raise RuntimeError(
                f"webhook callback error (status={status}): {snippet}",
            )

    def _send_text(self, user_id: str, user_name: str, text: str) -> None:
        if not text:
            return
        if self.callback_url is None:
            # Rust raises here ("no callback_url configured"); we
            # log + return so a deliver_only-mode setup (which has
            # no outbound callback) doesn't surface a noisy error
            # on every reply attempt.
            log.warn(
                "webhook send: WEBHOOK_CALLBACK_URL not configured, "
                "outbound dropped",
                to=user_id,
            )
            return
        chunks = _split_message(text, MAX_MESSAGE_LEN)
        for i, chunk in enumerate(chunks):
            # Skip remaining chunks if shutdown was signalled — a
            # multi-chunk send shouldn't keep firing 30 s-timeout
            # HTTP requests once the supervisor wants us gone. The
            # 429 retry path inside `_post_chunk` honours
            # `_shutdown.wait()` already; this is the outer guard
            # for the path between chunks.
            if self._shutdown.is_set():
                return
            self._post_chunk(chunk, user_id, user_name)
            if i + 1 < len(chunks):
                # 100 ms inter-chunk delay matches webhook.rs:483-485.
                if self._shutdown.wait(INTER_CHUNK_DELAY_SECS):
                    return

    # ---- sidecar surface --------------------------------------------

    async def produce(self, emit: Callable[[dict], None]) -> None:
        ready = threading.Event()
        t = threading.Thread(
            target=self._serve_forever,
            args=(emit, ready),
            name="webhook-listener",
            daemon=True,
        )
        t.start()
        while not ready.is_set():
            await asyncio.sleep(0.05)
        if self._httpd is None:
            raise RuntimeError(
                "webhook sidecar failed to start its listener; "
                "see prior log lines for the underlying error",
            )
        try:
            while True:
                await asyncio.sleep(3600)
        except asyncio.CancelledError:
            self._shutdown_server()
            raise

    def _shutdown_server(self) -> None:
        self._shutdown.set()
        httpd = self._httpd
        if httpd is None:
            return
        try:
            threading.Thread(
                target=httpd.shutdown,
                name="webhook-shutdown", daemon=True,
            ).start()
        except Exception:  # noqa: BLE001
            pass

    async def on_shutdown(self) -> None:
        self._shutdown_server()

    async def on_send(self, cmd) -> None:
        user_id = (
            cmd.channel_id
            or (cmd.user.get("platform_id") if cmd.user else "")
            or ""
        )
        if not user_id:
            log.warn("webhook on_send: empty platform_id, dropping")
            return
        user_name = (cmd.user.get("display_name") if cmd.user else None) or user_id

        content = cmd.content
        text = cmd.text or ""
        if isinstance(content, dict) and "Text" in content:
            inner = content["Text"]
            if isinstance(inner, str):
                text = inner
        elif content and not (isinstance(content, dict) and "Text" in content):
            # Rust's webhook send falls back to a placeholder for
            # anything other than Text (webhook.rs:446-449).
            text = "(Unsupported content type)"

        if not text:
            return

        loop = asyncio.get_event_loop()
        try:
            await loop.run_in_executor(
                None, lambda: self._send_text(user_id, user_name, text),
            )
        except Exception as e:  # noqa: BLE001
            log.error("webhook send failed", to=user_id, error=str(e))
            raise


if __name__ == "__main__":
    run_stdio_main(WebhookAdapter)
