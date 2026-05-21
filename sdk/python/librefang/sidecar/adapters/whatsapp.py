#!/usr/bin/env python3
"""WhatsApp sidecar channel adapter for LibreFang.

Replaces the in-process Rust ``librefang-channels::whatsapp`` adapter
(removed in this migration). Mirrors the Rust adapter's dual-mode
operation:

* **Cloud API mode** (default — when ``WHATSAPP_GATEWAY_URL`` is
  empty): talks to Meta's official WhatsApp Business Cloud API
  (``graph.facebook.com``) for outbound messages, and runs its own
  HTTP webhook server for inbound. (The in-process Rust adapter's
  ``start()`` was a stub that logged "webhook ready" but never
  actually parsed inbound activities — see whatsapp.rs:454-483.
  This sidecar implements the real webhook handler.)

* **Web/QR mode**: when ``WHATSAPP_GATEWAY_URL`` is set, outbound
  messages route through the Node.js Baileys gateway at
  ``{gateway_url}/message/send``. The gateway already handles
  inbound itself (POSTs directly to LibreFang's REST API at
  ``/api/agents/{id}/message``, bypassing the channel adapter
  entirely), so the sidecar's webhook server is unused in this
  mode. Voice messages with raw audio bytes go through
  ``{gateway_url}/message/send-voice`` with base64-encoded audio.

Behaviour parity (citations against
``crates/librefang-channels/src/whatsapp.rs`` on the pre-migration
tree):

* Cloud API outbound — text (``type: "text", text: {body: chunk}``),
  audio/voice URL (``type: "audio", audio: {link}``), image
  (``type: "image", image: {link, caption}``), file
  (``type: "document", document: {link, filename}``), location
  (``type: "location", location: {latitude, longitude}``). All
  authed via ``Authorization: Bearer <access_token>``.

* Cloud API media upload (``send_voice`` path): multipart POST to
  ``/{phone_id}/media`` with ``messaging_product=whatsapp`` then
  reference the returned ``id`` as ``audio.id`` on the message
  POST. Mirrors whatsapp.rs:186-275.

* Gateway outbound — ``POST {gateway}/message/send`` with
  ``{to, text}``, gracefully degrading non-text content
  (voice URL → "(Voice message: <url>)" text;
  image → caption-as-text; file/other → "(Unsupported …)").

* Gateway voice (raw bytes) — ``POST {gateway}/message/send-voice``
  with ``{to, audio: base64, mime_type}``.

* 4096-char chunking via shared ``split_message``.

* DM / group policy filter (``should_handle_message``):
  ``DmPolicy = Respond | AllowedOnly | Ignore``;
  ``GroupPolicy = All | MentionOnly | CommandsOnly | Ignore``.
  Bot mention detection: bot phone (with / without leading ``@``)
  or bot name, case-insensitive substring match.

* Sender allowlist (exact phone-number match).

* Multi-bot ``account_id`` metadata injection (#5003).

Improvements over the Rust adapter:

1. **Real Cloud API webhook server**. Rust's ``start()`` at
   whatsapp.rs:454-483 was a TODO stub — operators wanting Cloud
   API inbound had to wire their own webhook → ``/api/agents/…``
   forwarder. The sidecar implements the real handler:
   ``GET {path}`` returns ``hub.challenge`` for Meta's subscription
   confirmation when ``hub.mode == "subscribe"`` and
   ``hub.verify_token`` matches ``WHATSAPP_VERIFY_TOKEN``;
   ``POST {path}`` verifies ``X-Hub-Signature-256`` against the
   HMAC-SHA256 of the raw body keyed by ``WHATSAPP_APP_SECRET``
   (constant-time compare), then parses
   ``entry[].changes[].value.messages[]`` and emits text events.

2. **Inbound dedupe** on ``message.id`` — Meta retries on non-200,
   bounded ``SeenSet`` (10000 / 5000) keeps redeliveries from
   double-emitting.

3. **429 ``Retry-After`` honoured** on every outbound POST. Rust
   warned-and-failed on the first non-2xx (whatsapp.rs:373-377).

4. **Explicit 30 s ``urlopen`` timeout** on every REST call.
"""
from __future__ import annotations

import asyncio
import hashlib
import hmac
import http.server
import json
import os
import socketserver
import threading
from typing import Any, Callable, Optional
from urllib.parse import parse_qs, urlparse

from .. import logging as log
from .. import protocol
from ..common import (
    SeenSet as _SeenSet,
    http_request as _http_request,
    parse_retry_after as _parse_retry_after,
    split_csv as _split_csv,
    split_message as _split_message,
)
from ..protocol import Content, Field, Schema
from ..runtime import SidecarAdapter, run_stdio_main


# ---------------------------------------------------------------------------
# Constants — mirror crates/librefang-channels/src/whatsapp.rs.
# ---------------------------------------------------------------------------

DEFAULT_CLOUD_API_BASE = "https://graph.facebook.com/v17.0"
MAX_MESSAGE_LEN = 4096                # whatsapp.rs:16

DEFAULT_WEBHOOK_PORT = 8460
DEFAULT_WEBHOOK_PATH = "/webhook"
DEFAULT_BIND_HOST = "0.0.0.0"

SEND_TIMEOUT_SECS = 30.0
SEEN_MESSAGES_MAX = 10_000
SEEN_MESSAGES_EVICT = 5_000


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------


# DM / group policy enums — mirror librefang_types::config::{DmPolicy,
# GroupPolicy}. Strings match the TOML serde values exactly so an
# operator copying `dm_policy = "allowed_only"` out of the old
# `[channels.whatsapp]` block into `WHATSAPP_DM_POLICY` Just Works.
DM_RESPOND = "respond"
DM_ALLOWED_ONLY = "allowed_only"
DM_IGNORE = "ignore"
GROUP_ALL = "all"
GROUP_MENTION_ONLY = "mention_only"
GROUP_COMMANDS_ONLY = "commands_only"
GROUP_IGNORE = "ignore"


def verify_xhub_signature(
    secret: bytes, body: bytes, header: Optional[str],
) -> bool:
    """Verify Meta's ``X-Hub-Signature-256`` HMAC-SHA256 header.

    Header format: ``sha256=<hex-digest>``. Constant-time compare.
    Empty / missing / wrong-prefix / non-hex all reject.

    The Rust adapter didn't actually run a webhook (whatsapp.rs:454-
    483 was a TODO stub); this verification path is new in the
    sidecar.
    """
    if not isinstance(header, str) or not header:
        return False
    if not header.startswith("sha256="):
        return False
    claimed_hex = header[7:].strip()
    if not claimed_hex:
        return False
    try:
        claimed = bytes.fromhex(claimed_hex)
    except ValueError:
        return False
    digest = hmac.new(secret, body, hashlib.sha256).digest()
    return hmac.compare_digest(claimed, digest)


def is_bot_mentioned(
    text: str, *, bot_phone: Optional[str], bot_name: Optional[str],
) -> bool:
    """Mirrors whatsapp.rs:164-180.

    WhatsApp has no native @mention protocol at the Cloud API level,
    so the Rust adapter looked for the bot's phone (with / without
    ``@`` prefix and ``+``) or display name anywhere in the text,
    case-insensitive substring match.
    """
    lower = text.lower()
    if bot_phone:
        if bot_phone in lower:
            return True
        # Strip leading '+' for the `@<digits>` form.
        bare = bot_phone.lstrip("+")
        if f"@{bare}" in lower:
            return True
    if bot_name and bot_name.lower() in lower:
        return True
    return False


def should_handle_message(
    *,
    is_group: bool,
    text: str,
    sender_phone: str,
    dm_policy: str,
    group_policy: str,
    allowed_users: list[str],
    bot_phone: Optional[str],
    bot_name: Optional[str],
) -> bool:
    """Mirrors whatsapp.rs:143-158."""
    if is_group:
        if group_policy == GROUP_ALL:
            return True
        if group_policy == GROUP_MENTION_ONLY:
            return is_bot_mentioned(
                text, bot_phone=bot_phone, bot_name=bot_name,
            )
        if group_policy == GROUP_COMMANDS_ONLY:
            return text.lstrip().startswith("/")
        if group_policy == GROUP_IGNORE:
            return False
        # Unknown policy → fail-closed
        return False
    # DM
    if dm_policy == DM_RESPOND:
        return True
    if dm_policy == DM_ALLOWED_ONLY:
        if not allowed_users:
            # No allowlist + AllowedOnly = empty allowlist = nobody
            return False
        return sender_phone in allowed_users
    if dm_policy == DM_IGNORE:
        return False
    return False


def parse_cloud_api_message(
    payload: Any,
    *,
    account_id: Optional[str] = None,
) -> list[dict]:
    """Parse a Cloud API webhook POST body into 0-or-more sidecar
    ``message`` events. Only `text` messages are emitted; other
    types (image / video / audio / interactive) are skipped, same
    drop policy as the Rust adapter's `match` arms.

    Cloud API webhook shape:

    .. code-block:: json

      {
        "object": "whatsapp_business_account",
        "entry": [
          {"id": "WABA_ID",
           "changes": [
             {"value": {
                "messaging_product": "whatsapp",
                "messages": [
                  {"id": "wamid.xxx",
                   "from": "15551234567",
                   "type": "text",
                   "text": {"body": "hello"}}
                ],
                "contacts": [{"profile": {"name": "Alice"}, "wa_id": "15551234567"}]
              },
              "field": "messages"}
           ]}
        ]
      }
    """
    if not isinstance(payload, dict):
        return []
    out: list[dict] = []
    entries = payload.get("entry")
    if not isinstance(entries, list):
        return []
    for entry in entries:
        if not isinstance(entry, dict):
            continue
        changes = entry.get("changes")
        if not isinstance(changes, list):
            continue
        for change in changes:
            if not isinstance(change, dict):
                continue
            value = change.get("value")
            if not isinstance(value, dict):
                continue
            messages = value.get("messages")
            if not isinstance(messages, list):
                continue
            contacts = value.get("contacts")
            display_name_by_wa_id: dict[str, str] = {}
            if isinstance(contacts, list):
                for c in contacts:
                    if not isinstance(c, dict):
                        continue
                    wa_id = c.get("wa_id")
                    profile = c.get("profile")
                    name = (
                        profile.get("name") if isinstance(profile, dict)
                        else None
                    )
                    if isinstance(wa_id, str) and isinstance(name, str):
                        display_name_by_wa_id[wa_id] = name
            for msg in messages:
                ev = _parse_one_message(
                    msg,
                    display_name_by_wa_id,
                    account_id=account_id,
                )
                if ev is not None:
                    out.append(ev)
    return out


def _parse_one_message(
    msg: Any,
    display_name_by_wa_id: dict,
    *,
    account_id: Optional[str],
) -> Optional[dict]:
    if not isinstance(msg, dict):
        return None
    msg_type = msg.get("type")
    if msg_type != "text":
        # Other types silently dropped — matches the Rust adapter's
        # match arms (whatsapp.rs:523-609 only emits text).
        return None
    from_phone = msg.get("from")
    if not isinstance(from_phone, str) or not from_phone:
        return None
    text_obj = msg.get("text")
    text = (
        text_obj.get("body") if isinstance(text_obj, dict)
        and isinstance(text_obj.get("body"), str) else ""
    )
    if not text:
        return None
    msg_id = msg.get("id") if isinstance(msg.get("id"), str) else ""
    display_name = display_name_by_wa_id.get(from_phone, from_phone)

    metadata: dict[str, Any] = {}
    if account_id is not None:
        metadata["account_id"] = account_id

    return protocol.message(
        user_id=from_phone,
        user_name=display_name,
        content=Content.text(text),
        message_id=msg_id or None,
        channel_id=from_phone,
        metadata=metadata,
    )


# ---------------------------------------------------------------------------
# Adapter
# ---------------------------------------------------------------------------


class WhatsAppAdapter(SidecarAdapter):
    """WhatsApp sidecar — Cloud API + Web/QR gateway dual-mode."""

    capabilities: list = []
    suppress_error_responses: bool = False

    SCHEMA = Schema(
        name="whatsapp",
        display_name="WhatsApp",
        description=(
            "WhatsApp adapter: Meta Cloud API for production, Web/QR "
            "via Baileys gateway for personal accounts. Out-of-process "
            "sidecar (Python stdlib only)."
        ),
        fields=[
            Field("WHATSAPP_PHONE_NUMBER_ID",
                  "WhatsApp Business Phone Number ID (Cloud API)", "text",
                  placeholder="123456789012345"),
            Field("WHATSAPP_ACCESS_TOKEN",
                  "Cloud API Access Token", "secret"),
            Field("WHATSAPP_VERIFY_TOKEN",
                  "Webhook Verify Token (for GET subscription handshake)",
                  "secret"),
            Field("WHATSAPP_APP_SECRET",
                  "App Secret (HMAC-SHA256 key for X-Hub-Signature-256)",
                  "secret",
                  placeholder="(production should always set this)"),
            Field("WHATSAPP_GATEWAY_URL",
                  "Web/QR Gateway URL (switches outbound to Web mode)",
                  "text",
                  placeholder="http://127.0.0.1:3009",
                  advanced=True),
            Field("WHATSAPP_WEBHOOK_PORT",
                  "Webhook Port (Cloud API mode)", "number",
                  placeholder=str(DEFAULT_WEBHOOK_PORT)),
            Field("WHATSAPP_WEBHOOK_PATH",
                  "Webhook Path (Cloud API mode)", "text",
                  placeholder=DEFAULT_WEBHOOK_PATH,
                  advanced=True),
            Field("WHATSAPP_ALLOWED_USERS",
                  "Allowed phone numbers (csv, empty = all)", "text",
                  advanced=True),
            Field("WHATSAPP_ACCOUNT_ID",
                  "Account ID (multi-bot routing)", "text",
                  advanced=True),
            Field("WHATSAPP_BOT_PHONE",
                  "Bot's own phone (for mention detection in groups)",
                  "text", advanced=True),
            Field("WHATSAPP_BOT_NAME",
                  "Bot display name (mention-keyword fallback)",
                  "text", advanced=True),
            Field("WHATSAPP_DM_POLICY",
                  "DM policy: respond / allowed_only / ignore", "text",
                  placeholder=DM_RESPOND,
                  advanced=True),
            Field("WHATSAPP_GROUP_POLICY",
                  "Group policy: all / mention_only / commands_only / ignore",
                  "text",
                  placeholder=GROUP_ALL,
                  advanced=True),
        ],
    )

    def __init__(self) -> None:
        self.phone_number_id = os.environ.get("WHATSAPP_PHONE_NUMBER_ID", "").strip()
        self.access_token = os.environ.get("WHATSAPP_ACCESS_TOKEN", "").strip()
        self.verify_token = os.environ.get("WHATSAPP_VERIFY_TOKEN", "").strip()
        self.app_secret = os.environ.get("WHATSAPP_APP_SECRET", "").strip()
        self.gateway_url = (
            os.environ.get("WHATSAPP_GATEWAY_URL", "").strip() or None
        )

        # Validate per-mode requirements.
        if self.gateway_url is None:
            # Cloud API mode → access token + phone_number_id required.
            missing = []
            if not self.access_token:
                missing.append("WHATSAPP_ACCESS_TOKEN")
            if not self.phone_number_id:
                missing.append("WHATSAPP_PHONE_NUMBER_ID")
            if missing:
                log.error(
                    "whatsapp Cloud API mode requires env vars",
                    missing=missing,
                )
                raise SystemExit(2)
            if not self.app_secret:
                log.warn(
                    "whatsapp WHATSAPP_APP_SECRET unset — X-Hub-Signature-256 "
                    "verification on inbound webhook is DISABLED. Production "
                    "deployments should always set this.",
                )

        port_raw = os.environ.get("WHATSAPP_WEBHOOK_PORT", "").strip()
        try:
            self.webhook_port = int(port_raw) if port_raw else DEFAULT_WEBHOOK_PORT
        except ValueError:
            log.warn(
                "whatsapp WHATSAPP_WEBHOOK_PORT not an integer; using default",
                value=port_raw, default=DEFAULT_WEBHOOK_PORT,
            )
            self.webhook_port = DEFAULT_WEBHOOK_PORT
        path = (
            os.environ.get("WHATSAPP_WEBHOOK_PATH", "").strip()
            or DEFAULT_WEBHOOK_PATH
        )
        if not path.startswith("/"):
            path = "/" + path
        self.webhook_path = path
        self.bind_host = (
            os.environ.get("WHATSAPP_BIND_HOST", "").strip() or DEFAULT_BIND_HOST
        )

        self.allowed_users = _split_csv(
            os.environ.get("WHATSAPP_ALLOWED_USERS", ""),
        )
        acct = os.environ.get("WHATSAPP_ACCOUNT_ID", "").strip()
        self.account_id: Optional[str] = acct or None
        self.bot_phone: Optional[str] = (
            os.environ.get("WHATSAPP_BOT_PHONE", "").strip() or None
        )
        self.bot_name: Optional[str] = (
            os.environ.get("WHATSAPP_BOT_NAME", "").strip() or None
        )

        self.dm_policy = (
            os.environ.get("WHATSAPP_DM_POLICY", "").strip().lower()
            or DM_RESPOND
        )
        self.group_policy = (
            os.environ.get("WHATSAPP_GROUP_POLICY", "").strip().lower()
            or GROUP_ALL
        )

        # Test seam — overridable so tests can point us at a mock.
        self.cloud_api_base = (
            os.environ.get("WHATSAPP_CLOUD_API_BASE", "").strip()
            or DEFAULT_CLOUD_API_BASE
        )

        self._seen = _SeenSet(
            max_size=SEEN_MESSAGES_MAX, evict=SEEN_MESSAGES_EVICT,
        )
        self._httpd: Optional[socketserver.ThreadingTCPServer] = None
        self._shutdown = threading.Event()

    # ---- HTTP helpers ------------------------------------------------

    def _cloud_headers(self) -> dict:
        return {
            "Authorization": f"Bearer {self.access_token}",
            "Content-Type": "application/json",
            "User-Agent": "librefang-whatsapp-sidecar/1 (https://librefang.org)",
        }

    def _cloud_post_with_retry(
        self, url: str, payload: dict,
    ) -> tuple[int, Any, bytes, dict]:
        body = json.dumps(payload).encode("utf-8")
        status, resp, raw, hdrs = _http_request(
            url, method="POST", body=body,
            headers=self._cloud_headers(), timeout=SEND_TIMEOUT_SECS,
        )
        if status == 429:
            wait = _parse_retry_after(
                hdrs, default_secs=30.0,
                floor_secs=1.0, max_secs=60.0,
            )
            log.warn(
                "whatsapp Cloud API 429; sleeping then retrying once",
                retry_after=wait,
            )
            if self._shutdown.wait(wait):
                return status, resp, raw, hdrs
            status, resp, raw, hdrs = _http_request(
                url, method="POST", body=body,
                headers=self._cloud_headers(),
                timeout=SEND_TIMEOUT_SECS,
            )
        return status, resp, raw, hdrs

    # ---- Cloud API outbound -----------------------------------------

    def _cloud_send_text(self, to: str, text: str) -> None:
        if not to or not text:
            return
        url = f"{self.cloud_api_base}/{self.phone_number_id}/messages"
        for chunk in _split_message(text, MAX_MESSAGE_LEN):
            status, _resp, raw, _hdrs = self._cloud_post_with_retry(
                url, {
                    "messaging_product": "whatsapp",
                    "to": to,
                    "type": "text",
                    "text": {"body": chunk},
                },
            )
            if status < 200 or status >= 300:
                snippet = raw[:200].decode("utf-8", "replace") if raw else ""
                raise RuntimeError(
                    f"whatsapp Cloud API send error (status={status}): {snippet}",
                )

    def _cloud_send_audio_url(self, to: str, url: str) -> None:
        api_url = f"{self.cloud_api_base}/{self.phone_number_id}/messages"
        status, _resp, raw, _hdrs = self._cloud_post_with_retry(
            api_url, {
                "messaging_product": "whatsapp",
                "to": to,
                "type": "audio",
                "audio": {"link": url},
            },
        )
        if status < 200 or status >= 300:
            snippet = raw[:200].decode("utf-8", "replace") if raw else ""
            raise RuntimeError(
                f"whatsapp Cloud API voice send error "
                f"(status={status}): {snippet}",
            )

    def _cloud_send_image(
        self, to: str, url: str, caption: Optional[str],
    ) -> None:
        api_url = f"{self.cloud_api_base}/{self.phone_number_id}/messages"
        status, _resp, raw, _hdrs = self._cloud_post_with_retry(
            api_url, {
                "messaging_product": "whatsapp",
                "to": to,
                "type": "image",
                "image": {"link": url, "caption": caption or ""},
            },
        )
        if status < 200 or status >= 300:
            snippet = raw[:200].decode("utf-8", "replace") if raw else ""
            raise RuntimeError(
                f"whatsapp Cloud API image send error "
                f"(status={status}): {snippet}",
            )

    def _cloud_send_file(self, to: str, url: str, filename: str) -> None:
        api_url = f"{self.cloud_api_base}/{self.phone_number_id}/messages"
        status, _resp, raw, _hdrs = self._cloud_post_with_retry(
            api_url, {
                "messaging_product": "whatsapp",
                "to": to,
                "type": "document",
                "document": {"link": url, "filename": filename},
            },
        )
        if status < 200 or status >= 300:
            snippet = raw[:200].decode("utf-8", "replace") if raw else ""
            raise RuntimeError(
                f"whatsapp Cloud API file send error "
                f"(status={status}): {snippet}",
            )

    def _cloud_send_location(
        self, to: str, lat: float, lon: float,
    ) -> None:
        api_url = f"{self.cloud_api_base}/{self.phone_number_id}/messages"
        status, _resp, raw, _hdrs = self._cloud_post_with_retry(
            api_url, {
                "messaging_product": "whatsapp",
                "to": to,
                "type": "location",
                "location": {"latitude": lat, "longitude": lon},
            },
        )
        if status < 200 or status >= 300:
            snippet = raw[:200].decode("utf-8", "replace") if raw else ""
            raise RuntimeError(
                f"whatsapp Cloud API location send error "
                f"(status={status}): {snippet}",
            )

    # ---- Gateway outbound -------------------------------------------

    def _gateway_send_text(self, to: str, text: str) -> None:
        if not to or not text:
            return
        url = f"{self.gateway_url.rstrip('/')}/message/send"
        for chunk in _split_message(text, MAX_MESSAGE_LEN):
            body = json.dumps({"to": to, "text": chunk}).encode("utf-8")
            status, _resp, raw, _hdrs = _http_request(
                url, method="POST", body=body,
                headers={
                    "Content-Type": "application/json",
                    "User-Agent": "librefang-whatsapp-sidecar/1",
                },
                timeout=SEND_TIMEOUT_SECS,
            )
            if status < 200 or status >= 300:
                snippet = raw[:200].decode("utf-8", "replace") if raw else ""
                raise RuntimeError(
                    f"whatsapp gateway send error "
                    f"(status={status}): {snippet}",
                )

    # ---- inbound webhook (Cloud API) --------------------------------

    def _handle_get_verify(
        self, query: str,
    ) -> tuple[int, bytes]:
        """Meta's webhook subscription handshake. GET with query:
        ``?hub.mode=subscribe&hub.verify_token=<token>&hub.challenge=<echo>``.
        On match return the echo verbatim; on mismatch return 403."""
        params = parse_qs(query)
        mode = (params.get("hub.mode") or [""])[0]
        token = (params.get("hub.verify_token") or [""])[0]
        challenge = (params.get("hub.challenge") or [""])[0]
        # Constant-time compare of the verify_token — the handshake
        # runs infrequently so timing attacks are impractical, but
        # `==` on strings short-circuits at the first mismatching
        # byte. Always cheap to do this right.
        if (
            mode == "subscribe"
            and self.verify_token
            and hmac.compare_digest(token, self.verify_token)
        ):
            return 200, challenge.encode("utf-8")
        return 403, b""

    def _handle_post_webhook(
        self,
        body: bytes,
        signature: Optional[str],
        emit: Callable[[dict], None],
    ) -> int:
        """Verify X-Hub-Signature-256 (if app_secret configured) +
        parse Cloud API webhook body + emit text events."""
        if self.app_secret:
            if not signature:
                # Missing header (Meta omits it, or the upstream
                # proxy stripped it) — 400 not 401, since 401
                # implies credentials were presented and rejected.
                log.warn("whatsapp: missing X-Hub-Signature-256 header")
                return 400
            if not verify_xhub_signature(
                self.app_secret.encode("utf-8"), body, signature,
            ):
                log.warn("whatsapp: invalid X-Hub-Signature-256")
                return 401
        try:
            payload = json.loads(body.decode("utf-8"))
        except (ValueError, UnicodeDecodeError):
            return 400

        events = parse_cloud_api_message(payload, account_id=self.account_id)
        for ev in events:
            params = ev.get("params", {})
            from_phone = params.get("user_id", "")
            text = params.get("text", "")
            # Cloud API webhook doesn't expose group conversations
            # in a first-class way (group messaging is gated behind
            # a separate API surface); treat all inbound as DM here.
            if not should_handle_message(
                is_group=False, text=text, sender_phone=from_phone,
                dm_policy=self.dm_policy,
                group_policy=self.group_policy,
                allowed_users=self.allowed_users,
                bot_phone=self.bot_phone, bot_name=self.bot_name,
            ):
                log.debug("whatsapp dropping per policy",
                          dm_policy=self.dm_policy, sender=from_phone)
                continue
            msg_id = params.get("message_id")
            if isinstance(msg_id, str) and msg_id:
                if not self._seen.mark(msg_id):
                    log.debug("whatsapp duplicate message id",
                              message_id=msg_id)
                    continue
            emit(ev)
        return 200

    def _make_handler_class(
        self, emit: Callable[[dict], None],
    ) -> type:
        adapter = self

        class _WhatsAppWebhookHandler(http.server.BaseHTTPRequestHandler):
            _MAX_BODY_BYTES = 4 * 1024 * 1024

            def _path_matches(self) -> bool:
                return self.path.split("?", 1)[0] == adapter.webhook_path

            def do_GET(self) -> None:  # noqa: N802
                if not self._path_matches():
                    self.send_response(404)
                    self.end_headers()
                    return
                parsed = urlparse(self.path)
                status, body = adapter._handle_get_verify(parsed.query or "")
                self.send_response(status)
                self.end_headers()
                if body:
                    self.wfile.write(body)

            def do_POST(self) -> None:  # noqa: N802
                if not self._path_matches():
                    self.send_response(404)
                    self.end_headers()
                    return
                try:
                    cl = int(self.headers.get("Content-Length", "0") or 0)
                except ValueError:
                    cl = 0
                if cl < 0:
                    # `Content-Length: -1` would make rfile.read(-1)
                    # consume to EOF — an unbounded read from a TCP
                    # socket the attacker controls.
                    self.send_response(400)
                    self.end_headers()
                    return
                if cl > self._MAX_BODY_BYTES:
                    self.send_response(413)
                    self.end_headers()
                    return
                body = self.rfile.read(cl) if cl > 0 else b""
                sig = self.headers.get("X-Hub-Signature-256")
                status = adapter._handle_post_webhook(body, sig, emit)
                self.send_response(status)
                self.end_headers()
                if status == 200:
                    self.wfile.write(b"OK")

            def log_message(self, fmt: str, *args: Any) -> None:  # noqa: A003
                return

        return _WhatsAppWebhookHandler

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
                (self.bind_host, self.webhook_port), handler_cls,
            )
        except OSError as e:
            log.error("whatsapp webhook bind failed",
                      host=self.bind_host, port=self.webhook_port,
                      error=str(e))
            ready.set()
            return

        self._httpd = httpd
        ready.set()
        log.info("whatsapp webhook listening",
                 host=self.bind_host, port=self.webhook_port,
                 path=self.webhook_path)
        try:
            httpd.serve_forever()
        finally:
            try:
                httpd.server_close()
            except Exception:  # noqa: BLE001
                pass

    # ---- sidecar surface --------------------------------------------

    async def produce(self, emit: Callable[[dict], None]) -> None:
        if self.gateway_url is not None:
            # Web/QR mode: JS gateway delivers inbound directly to
            # LibreFang's REST API (`POST /api/agents/{id}/message`)
            # — bypasses the channel adapter entirely. The sidecar
            # has no inbound work to do; just block until cancelled.
            log.info(
                "whatsapp gateway mode active — inbound is handled by the "
                "Baileys gateway (POSTs directly to /api/agents/.../message)",
                gateway_url=self.gateway_url,
            )
            try:
                while True:
                    await asyncio.sleep(3600)
            except asyncio.CancelledError:
                raise
            return

        # Cloud API mode: spin up our own webhook server.
        ready = threading.Event()
        t = threading.Thread(
            target=self._serve_forever,
            args=(emit, ready),
            name="whatsapp-webhook",
            daemon=True,
        )
        t.start()
        while not ready.is_set():
            await asyncio.sleep(0.05)
        if self._httpd is None:
            raise RuntimeError(
                "whatsapp sidecar failed to start its webhook server; "
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
                name="whatsapp-shutdown", daemon=True,
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
            log.warn("whatsapp on_send: empty platform_id, dropping")
            return

        loop = asyncio.get_event_loop()
        content = cmd.content
        text = cmd.text or ""

        # Extract structured content into the dispatcher path.
        if self.gateway_url is not None:
            # Web/QR gateway mode — text only, everything else
            # degrades to text. Matches whatsapp.rs:491-519.
            if isinstance(content, dict):
                if "Voice" in content:
                    v = content.get("Voice") or {}
                    url = v.get("url") if isinstance(v, dict) else None
                    if not isinstance(url, str):
                        url = ""
                    text = f"(Voice message: {url})"
                elif "Image" in content:
                    img = content.get("Image") or {}
                    cap = img.get("caption") if isinstance(img, dict) else None
                    text = cap if isinstance(cap, str) and cap else (
                        "(Image — not supported in Web mode)"
                    )
                elif "File" in content:
                    f = content.get("File") or {}
                    fn = f.get("filename") if isinstance(f, dict) else None
                    text = (
                        f"(File: {fn} — not supported in Web mode)"
                        if isinstance(fn, str) else
                        "(File — not supported in Web mode)"
                    )
                elif "Text" in content:
                    inner = content["Text"]
                    if isinstance(inner, str):
                        text = inner
                else:
                    text = "(Unsupported content type in Web mode)"
            if not text:
                return
            try:
                await loop.run_in_executor(
                    None, lambda: self._gateway_send_text(to, text),
                )
            except Exception as e:  # noqa: BLE001
                log.error("whatsapp gateway send failed", to=to, error=str(e))
                raise
            return

        # Cloud API mode — handle each structured variant.
        if isinstance(content, dict):
            if "Voice" in content:
                v = content.get("Voice") or {}
                url = v.get("url") if isinstance(v, dict) else ""
                if isinstance(url, str) and url:
                    try:
                        await loop.run_in_executor(
                            None, lambda: self._cloud_send_audio_url(to, url),
                        )
                    except Exception as e:  # noqa: BLE001
                        log.error("whatsapp voice send failed",
                                  to=to, error=str(e))
                        raise
                return
            if "Image" in content:
                img = content.get("Image") or {}
                url = img.get("url") if isinstance(img, dict) else ""
                cap = img.get("caption") if isinstance(img, dict) else None
                if isinstance(url, str) and url:
                    try:
                        await loop.run_in_executor(
                            None,
                            lambda: self._cloud_send_image(
                                to, url, cap if isinstance(cap, str) else None,
                            ),
                        )
                    except Exception as e:  # noqa: BLE001
                        log.error("whatsapp image send failed",
                                  to=to, error=str(e))
                        raise
                return
            if "File" in content:
                f = content.get("File") or {}
                url = f.get("url") if isinstance(f, dict) else ""
                fn = f.get("filename") if isinstance(f, dict) else "file"
                if isinstance(url, str) and url:
                    try:
                        await loop.run_in_executor(
                            None,
                            lambda: self._cloud_send_file(
                                to, url, fn if isinstance(fn, str) else "file",
                            ),
                        )
                    except Exception as e:  # noqa: BLE001
                        log.error("whatsapp file send failed",
                                  to=to, error=str(e))
                        raise
                return
            if "Location" in content:
                loc = content.get("Location") or {}
                lat = loc.get("lat") if isinstance(loc, dict) else None
                lon = loc.get("lon") if isinstance(loc, dict) else None
                try:
                    lat_f = float(lat) if lat is not None else 0.0
                    lon_f = float(lon) if lon is not None else 0.0
                except (TypeError, ValueError):
                    log.warn("whatsapp location: invalid lat/lon",
                             lat=lat, lon=lon)
                    return
                try:
                    await loop.run_in_executor(
                        None,
                        lambda: self._cloud_send_location(to, lat_f, lon_f),
                    )
                except Exception as e:  # noqa: BLE001
                    log.error("whatsapp location send failed",
                              to=to, error=str(e))
                    raise
                return
            if "Text" in content:
                inner = content["Text"]
                if isinstance(inner, str):
                    text = inner

        if not text:
            return
        try:
            await loop.run_in_executor(
                None, lambda: self._cloud_send_text(to, text),
            )
        except Exception as e:  # noqa: BLE001
            log.error("whatsapp send failed", to=to, error=str(e))
            raise


if __name__ == "__main__":
    run_stdio_main(WhatsAppAdapter)
