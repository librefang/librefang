#!/usr/bin/env python3
"""Microsoft Teams Bot Framework v3 sidecar channel adapter for LibreFang.

Replaces the in-process Rust ``librefang-channels::teams`` adapter
(removed in this migration). Same pattern as the line / mattermost
/ webex / qq sidecars — runs its own HTTP webhook server (stdlib
``BaseHTTPRequestHandler`` over ``ThreadingTCPServer``) rather than
mounting onto LibreFang's shared axum server.

Behaviour parity with the Rust adapter (every assertion has a
file/line citation against ``crates/librefang-channels/src/teams.rs``
on the pre-migration tree):

* **Inbound HTTP webhook**: ``POST {TEAMS_WEBHOOK_PATH}`` (default
  ``/webhook``) on ``TEAMS_WEBHOOK_PORT``. The Rust adapter mounted
  ``/channels/teams/webhook`` on the shared server (teams.rs:401-446);
  the sidecar runs its own listener so the public URL operators
  register in the Teams Developer Portal changes — see the migration
  commit for the upgrade path.

* **HMAC-SHA256 signature verification**: ``Authorization: HMAC
  <base64-digest>``, key = base64-decoded ``TEAMS_SECURITY_TOKEN``.
  Expected digest is ``Base64(HMAC-SHA256(key, raw_body))``.
  Constant-time compare. Empty / non-base64 / missing header all
  reject (teams.rs:29-53). When ``TEAMS_SECURITY_TOKEN`` itself is
  empty or non-base64, verification is DISABLED with a WARN at
  startup (teams.rs:119-137) — matches the Rust behaviour for
  smoke-test environments.

* **OAuth2 client credentials flow** for outbound bearer tokens
  (teams.rs:174-219). ``POST {TEAMS_OAUTH_TOKEN_URL}`` (default
  ``https://login.microsoftonline.com/botframework.com/oauth2/v2.0/token``)
  with ``grant_type=client_credentials`` + ``client_id`` +
  ``client_secret`` + ``scope=https://api.botframework.com/.default``.
  Response ``access_token`` + ``expires_in`` cached with a 300 s
  refresh buffer (``TOKEN_REFRESH_BUFFER_SECS``).

* **Outbound REST**: ``POST {service_url}/v3/conversations/{id}/activities``
  with ``{type: "message", text: <chunk>}`` (teams.rs:226-262).
  4096-char chunking via the shared ``split_message`` helper.

* **Self-skip** by ``from.id == app_id`` (teams.rs:290-293).

* **Tenant allowlist** via ``channelData.tenant.id``
  (teams.rs:295-303). Empty list = allow all.

* **Slash-command parsing** ``/cmd args`` → ``Command`` content
  (teams.rs:323-337).

* **Group detection** via ``conversation.isGroup`` (teams.rs:317-320).

* **Multi-bot ``account_id``** metadata injection (#5003).

* **Typing indicator**: ``POST {service_url}/v3/conversations/{id}/activities``
  with ``{type: "typing"}`` (teams.rs:493-517) — best-effort, ignored
  on failure.

Improvements over the Rust adapter:

1. **Per-conversation ``service_url`` reuse**. The Rust adapter
   stored the inbound ``serviceUrl`` in ``metadata.serviceUrl``
   but never used it on outbound — every send hit
   ``DEFAULT_SERVICE_URL`` regardless. For tenant- /
   region-routed deployments where Microsoft assigns different
   service URLs per conversation, this silently lands replies on
   the wrong endpoint. The sidecar caches the most recent
   ``serviceUrl`` per ``conversation_id`` and uses it on outbound
   in preference to the default.

2. **Inbound dedupe on Activity ID**. The Rust adapter at
   teams.rs:434-441 emitted every activity unconditionally;
   Bot Framework retries deliveries (non-2xx response or timeout
   within ~10 s) which could cause double-emit. Bounded SeenSet
   (10000 / evict 5000), same envelope as the other sidecars.

3. **429 ``Retry-After`` honoured** on every outbound POST. Rust
   warned and dropped (teams.rs:254-258); the sidecar parses
   ``Retry-After``, sleeps once, retries, then logs-and-continues.

4. **Explicit 30 s ``urlopen`` timeout** on every REST call —
   Rust relied on ``reqwest``'s default (none).

Configure via ``[[sidecar_channels]]``::

    [[sidecar_channels]]
    name = "teams"
    command = "python3"
    args = ["-m", "librefang.sidecar.adapters.teams"]
    channel_type = "teams"
    [sidecar_channels.env]
    TEAMS_APP_ID = "00000000-0000-0000-0000-000000000000"
    TEAMS_WEBHOOK_PORT = "8459"
    # TEAMS_ALLOWED_TENANTS = "tenant-a,tenant-b"
    # TEAMS_ACCOUNT_ID = "production"

Secrets via ``~/.librefang/secrets.env``: ``TEAMS_APP_PASSWORD``
(the Bot Framework app password / client secret) and
``TEAMS_SECURITY_TOKEN`` (base64 outgoing-webhook security token
from the Teams portal).
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
import urllib.parse
from typing import Any, Callable, Optional

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
# Constants — mirror crates/librefang-channels/src/teams.rs.
# ---------------------------------------------------------------------------

DEFAULT_OAUTH_TOKEN_URL = (
    "https://login.microsoftonline.com"
    "/botframework.com/oauth2/v2.0/token"
)
DEFAULT_SERVICE_URL = "https://smba.trafficmanager.net/teams/"
MAX_MESSAGE_LEN = 4096                    # teams.rs:62
TOKEN_REFRESH_BUFFER_SECS = 300.0         # teams.rs:65

DEFAULT_WEBHOOK_PORT = 8459
DEFAULT_WEBHOOK_PATH = "/webhook"
DEFAULT_BIND_HOST = "0.0.0.0"

SEND_TIMEOUT_SECS = 30.0
SEEN_MESSAGES_MAX = 10_000
SEEN_MESSAGES_EVICT = 5_000


# ---------------------------------------------------------------------------
# HMAC verification
# ---------------------------------------------------------------------------


def verify_teams_signature(
    key_bytes: bytes, body: bytes, auth_header: Optional[str],
) -> bool:
    """Verify a Microsoft Teams outgoing-webhook HMAC-SHA256 signature.

    ``Authorization: HMAC <base64-digest>``. Mirrors the Rust
    adapter's ``verify_teams_signature`` (teams.rs:29-53):

    * empty / missing / wrong-prefix header → reject
    * malformed base64 in the claimed digest → reject (warn)
    * constant-time comparison against the expected digest
    """
    if not isinstance(auth_header, str) or not auth_header:
        return False
    if not auth_header.startswith("HMAC "):
        return False
    claimed_b64 = auth_header[5:].strip()
    if not claimed_b64:
        return False
    try:
        claimed = base64.b64decode(claimed_b64, validate=True)
    except (binascii.Error, ValueError):
        return False
    digest = hmac.new(key_bytes, body, hashlib.sha256).digest()
    return hmac.compare_digest(claimed, digest)


# ---------------------------------------------------------------------------
# Activity parsing
# ---------------------------------------------------------------------------


def parse_teams_activity(
    activity: Any,
    *,
    app_id: str,
    allowed_tenants: list[str],
    account_id: Optional[str] = None,
) -> Optional[dict]:
    """Translate a Bot Framework activity JSON into a sidecar
    ``message`` event. Returns ``None`` for activities that should
    be ignored (non-message types, bot-originated, disallowed
    tenants, empty text). Pure function — no dedupe state mutation
    so unit tests stay simple. Mirrors teams.rs:275-363."""
    if not isinstance(activity, dict):
        return None
    if activity.get("type") != "message":
        return None

    sender = activity.get("from")
    if not isinstance(sender, dict):
        return None
    from_id = sender.get("id") if isinstance(sender.get("id"), str) else ""
    from_name = sender.get("name") if isinstance(sender.get("name"), str) else "Unknown"

    # Self-skip — teams.rs:290-293
    if from_id == app_id:
        return None

    # Tenant filter — teams.rs:295-303
    if allowed_tenants:
        channel_data = activity.get("channelData") or {}
        tenant = (
            channel_data.get("tenant") if isinstance(channel_data, dict) else None
        )
        tenant_id = (
            tenant.get("id") if isinstance(tenant, dict)
            and isinstance(tenant.get("id"), str) else ""
        )
        if tenant_id not in allowed_tenants:
            return None

    text = activity.get("text") if isinstance(activity.get("text"), str) else ""
    if not text:
        return None

    conversation = activity.get("conversation") or {}
    conversation_id = (
        conversation.get("id") if isinstance(conversation, dict)
        and isinstance(conversation.get("id"), str) else ""
    )
    is_group = (
        conversation.get("isGroup") is True if isinstance(conversation, dict)
        else False
    )

    activity_id = activity.get("id") if isinstance(activity.get("id"), str) else ""
    service_url = (
        activity.get("serviceUrl") if isinstance(activity.get("serviceUrl"), str)
        else ""
    )

    # Slash-command routing — teams.rs:323-337
    if text.startswith("/"):
        parts = text.split(" ", 1)
        cmd_name = parts[0][1:]
        args = parts[1].split() if len(parts) > 1 else []
        content = {"Command": {"name": cmd_name, "args": args}}
    else:
        content = Content.text(text)

    metadata: dict[str, Any] = {}
    if service_url:
        metadata["serviceUrl"] = service_url
    if is_group:
        metadata["is_group"] = True
    if account_id is not None:
        metadata["account_id"] = account_id

    return protocol.message(
        user_id=conversation_id,
        user_name=from_name,
        content=content,
        message_id=activity_id or None,
        channel_id=conversation_id,
        metadata=metadata,
    )


# ---------------------------------------------------------------------------
# Adapter
# ---------------------------------------------------------------------------


class TeamsAdapter(SidecarAdapter):
    """Microsoft Teams Bot Framework v3 sidecar."""

    # ``typing`` — POST /v3/conversations/{id}/activities with
    # ``{type: "typing"}`` handled by ``_on_typing`` via TypingCmd.
    # Same surface the Rust adapter offered at teams.rs:493-517.
    capabilities: list = ["typing"]
    suppress_error_responses: bool = False

    SCHEMA = Schema(
        name="teams",
        display_name="Microsoft Teams",
        description=(
            "Microsoft Teams adapter via Bot Framework v3. "
            "Out-of-process sidecar (Python stdlib only)."
        ),
        fields=[
            Field("TEAMS_APP_ID", "Bot Framework App ID", "text",
                  required=True,
                  placeholder="00000000-0000-0000-0000-000000000000"),
            Field("TEAMS_APP_PASSWORD", "App Password / Client Secret", "secret",
                  required=True),
            Field("TEAMS_SECURITY_TOKEN",
                  "Outgoing Webhook Security Token (base64)",
                  "secret",
                  placeholder="(from Teams portal — disables HMAC verification when blank)"),
            Field("TEAMS_WEBHOOK_PORT",
                  "Webhook Port", "number",
                  placeholder=str(DEFAULT_WEBHOOK_PORT)),
            Field("TEAMS_WEBHOOK_PATH",
                  "Webhook Path", "text",
                  placeholder=DEFAULT_WEBHOOK_PATH,
                  advanced=True),
            Field("TEAMS_ALLOWED_TENANTS",
                  "Allowed Azure AD Tenant IDs (csv, empty = all)", "text",
                  advanced=True),
            Field("TEAMS_ACCOUNT_ID",
                  "Account ID (multi-bot routing)", "text",
                  advanced=True),
        ],
    )

    def __init__(self) -> None:
        app_id = os.environ.get("TEAMS_APP_ID", "").strip()
        app_password = os.environ.get("TEAMS_APP_PASSWORD", "").strip()
        missing = []
        if not app_id:
            missing.append("TEAMS_APP_ID")
        if not app_password:
            missing.append("TEAMS_APP_PASSWORD")
        if missing:
            log.error("teams required env vars missing", missing=missing)
            raise SystemExit(2)

        self.app_id = app_id
        self.app_password = app_password

        # Decode security_token ONCE at construction so the verify
        # hot-path stays cheap. Empty / non-base64 → verification
        # disabled with a startup WARN, mirroring teams.rs:119-137.
        sec_b64 = os.environ.get("TEAMS_SECURITY_TOKEN", "").strip()
        self.security_token_key: Optional[bytes]
        if not sec_b64:
            log.warn(
                "teams: no TEAMS_SECURITY_TOKEN configured — webhook "
                "signature verification is DISABLED. Set it to harden "
                "this endpoint.",
            )
            self.security_token_key = None
        else:
            try:
                self.security_token_key = base64.b64decode(sec_b64, validate=True)
            except (binascii.Error, ValueError) as e:
                log.warn(
                    "teams: TEAMS_SECURITY_TOKEN is not valid base64 — "
                    "webhook signature verification is DISABLED",
                    error=str(e),
                )
                self.security_token_key = None

        self.allowed_tenants = _split_csv(
            os.environ.get("TEAMS_ALLOWED_TENANTS", ""),
        )
        acct = os.environ.get("TEAMS_ACCOUNT_ID", "").strip()
        self.account_id: Optional[str] = acct or None

        port_raw = os.environ.get("TEAMS_WEBHOOK_PORT", "").strip()
        try:
            self.webhook_port = int(port_raw) if port_raw else DEFAULT_WEBHOOK_PORT
        except ValueError:
            log.warn(
                "teams TEAMS_WEBHOOK_PORT not an integer; using default",
                value=port_raw, default=DEFAULT_WEBHOOK_PORT,
            )
            self.webhook_port = DEFAULT_WEBHOOK_PORT
        path = os.environ.get("TEAMS_WEBHOOK_PATH", "").strip() or DEFAULT_WEBHOOK_PATH
        if not path.startswith("/"):
            path = "/" + path
        self.webhook_path = path
        self.bind_host = (
            os.environ.get("TEAMS_BIND_HOST", "").strip() or DEFAULT_BIND_HOST
        )

        # Test seams — overridable via env so tests can point us at
        # a local mock without monkey-patching urllib globally.
        self.oauth_token_url = (
            os.environ.get("TEAMS_OAUTH_TOKEN_URL", "").strip()
            or DEFAULT_OAUTH_TOKEN_URL
        )
        self.default_service_url = (
            os.environ.get("TEAMS_SERVICE_URL", "").strip()
            or DEFAULT_SERVICE_URL
        )

        self._token_lock = threading.Lock()
        self._cached_token: Optional[tuple[str, float]] = None  # (token, expiry_monotonic)

        # Per-conversation serviceUrl cache (Improvement #1).
        self._service_url_lock = threading.Lock()
        self._service_urls: dict[str, str] = {}

        self._seen = _SeenSet(
            max_size=SEEN_MESSAGES_MAX, evict=SEEN_MESSAGES_EVICT,
        )

        self._httpd: Optional[socketserver.ThreadingTCPServer] = None
        self._shutdown = threading.Event()

    # ---- OAuth ------------------------------------------------------

    def _get_token(self) -> str:
        """Return a cached bearer token, refreshing if expired.
        Mirrors teams.rs:174-219."""
        with self._token_lock:
            if self._cached_token is not None:
                token, expiry = self._cached_token
                if time.monotonic() < expiry:
                    return token
        body = urllib.parse.urlencode({
            "grant_type": "client_credentials",
            "client_id": self.app_id,
            "client_secret": self.app_password,
            "scope": "https://api.botframework.com/.default",
        }).encode("ascii")
        headers = {
            "Content-Type": "application/x-www-form-urlencoded",
            "User-Agent": "librefang-teams-sidecar/1 (https://librefang.org)",
        }
        status, resp, raw, _hdrs = _http_request(
            self.oauth_token_url, method="POST", body=body, headers=headers,
            timeout=SEND_TIMEOUT_SECS,
        )
        if status < 200 or status >= 300 or not isinstance(resp, dict):
            snippet = raw[:200].decode("utf-8", "replace") if raw else ""
            raise RuntimeError(
                f"teams OAuth2 token error (status={status}): {snippet}",
            )
        token = resp.get("access_token")
        if not isinstance(token, str) or not token:
            raise RuntimeError("teams OAuth2 response missing access_token")
        try:
            expires_in = int(resp.get("expires_in") or 3600)
        except (TypeError, ValueError):
            expires_in = 3600
        # Refresh 5 minutes before actual expiry.
        ttl = max(60, expires_in - int(TOKEN_REFRESH_BUFFER_SECS))
        with self._token_lock:
            self._cached_token = (token, time.monotonic() + ttl)
        return token

    # ---- service_url cache ------------------------------------------

    def _service_url_for(self, conversation_id: str) -> str:
        with self._service_url_lock:
            url = self._service_urls.get(conversation_id)
        return url or self.default_service_url

    def _stash_service_url(self, conversation_id: str, url: str) -> None:
        if not conversation_id or not url:
            return
        with self._service_url_lock:
            self._service_urls[conversation_id] = url

    # ---- outbound REST ----------------------------------------------

    def _post_activity(self, conversation_id: str, body: dict) -> tuple[int, bytes, dict]:
        """POST one activity to /v3/conversations/{id}/activities."""
        token = self._get_token()
        service = self._service_url_for(conversation_id).rstrip("/")
        url = f"{service}/v3/conversations/{conversation_id}/activities"
        headers = {
            "Authorization": f"Bearer {token}",
            "Content-Type": "application/json",
            "User-Agent": "librefang-teams-sidecar/1 (https://librefang.org)",
        }
        payload = json.dumps(body).encode("utf-8")
        status, _resp, raw, resp_hdrs = _http_request(
            url, method="POST", body=payload, headers=headers,
            timeout=SEND_TIMEOUT_SECS,
        )
        if status == 429:
            wait = _parse_retry_after(
                resp_hdrs, default_secs=30.0,
                floor_secs=1.0, max_secs=60.0,
            )
            log.warn("teams activities 429; sleeping then retrying once",
                     retry_after=wait)
            if self._shutdown.wait(wait):
                return status, raw, resp_hdrs
            status, _resp, raw, resp_hdrs = _http_request(
                url, method="POST", body=payload, headers=headers,
                timeout=SEND_TIMEOUT_SECS,
            )
        return status, raw, resp_hdrs

    def _send_text(self, conversation_id: str, text: str) -> None:
        if not conversation_id or not text:
            return
        for chunk in _split_message(text, MAX_MESSAGE_LEN):
            status, raw, _hdrs = self._post_activity(
                conversation_id, {"type": "message", "text": chunk},
            )
            if status < 200 or status >= 300:
                snippet = raw[:200].decode("utf-8", "replace") if raw else ""
                # Match the Rust adapter's WARN-and-continue behaviour
                # (teams.rs:254-258) — a single throttled chunk must
                # not drop the rest of a multi-chunk reply.
                log.warn("teams send chunk failed",
                         status=status, body=snippet)

    def _send_typing(self, conversation_id: str) -> None:
        """Best-effort typing indicator (teams.rs:493-517) — any
        failure is logged at DEBUG and swallowed."""
        if not conversation_id:
            return
        try:
            status, _raw, _hdrs = self._post_activity(
                conversation_id, {"type": "typing"},
            )
        except Exception as e:  # noqa: BLE001
            log.debug("teams typing error", error=str(e))
            return
        if status < 200 or status >= 300:
            log.debug("teams typing non-2xx", status=status)

    # ---- inbound webhook --------------------------------------------

    def _handle_webhook_body(
        self,
        body: bytes,
        auth_header: Optional[str],
        emit: Callable[[dict], None],
    ) -> int:
        """Verify + parse one webhook POST body. Returns the HTTP
        status code to send back. Extracted so tests can drive it
        without spinning up a real ``ThreadingTCPServer``."""
        if self.security_token_key is not None:
            if not verify_teams_signature(
                self.security_token_key, body, auth_header,
            ):
                if not auth_header:
                    log.warn("teams: missing Authorization header")
                    return 400
                log.warn("teams: invalid HMAC-SHA256 signature")
                return 401
        try:
            activity = json.loads(body.decode("utf-8"))
        except (ValueError, UnicodeDecodeError):
            return 400
        if not isinstance(activity, dict):
            return 400

        # Stash the per-conversation serviceUrl before parse so
        # outbound replies for this conversation use the correct
        # endpoint (Improvement #1 over the Rust adapter).
        conversation = activity.get("conversation")
        conv_id = (
            conversation.get("id") if isinstance(conversation, dict)
            and isinstance(conversation.get("id"), str) else ""
        )
        service_url = (
            activity.get("serviceUrl") if isinstance(activity.get("serviceUrl"), str)
            else ""
        )
        if conv_id and service_url:
            self._stash_service_url(conv_id, service_url)

        # Parse first, then dedupe only successfully-parsed messages.
        # Marking the Activity ID seen on a *dropped* activity (no
        # text / wrong type / disallowed tenant) would block any
        # subsequent legitimate retry that shares the ID — Bot
        # Framework can redeliver after a non-2xx, and a previously
        # rejected payload may arrive with the fields the parse path
        # needs the second time around (e.g. activity 'updated' to
        # carry text).
        ev = parse_teams_activity(
            activity,
            app_id=self.app_id,
            allowed_tenants=self.allowed_tenants,
            account_id=self.account_id,
        )
        if ev is None:
            return 200

        # Improvement #2: bounded SeenSet on Activity ID. Bot
        # Framework retries on non-2xx / timeout within ~10 s and
        # the Rust adapter at teams.rs:434-441 emitted every
        # activity unconditionally.
        params = ev.get("params", {})
        activity_id = params.get("message_id")
        if isinstance(activity_id, str) and activity_id:
            if not self._seen.mark(activity_id):
                log.debug("teams duplicate activity id, dropping",
                          activity_id=activity_id)
                return 200
        emit(ev)
        return 200

    def _make_handler_class(
        self, emit: Callable[[dict], None],
    ) -> type:
        adapter = self

        class _TeamsWebhookHandler(http.server.BaseHTTPRequestHandler):
            _MAX_BODY_BYTES = 4 * 1024 * 1024

            def do_POST(self) -> None:  # noqa: N802
                if self.path.split("?", 1)[0] != adapter.webhook_path:
                    self.send_response(404)
                    self.end_headers()
                    return
                try:
                    cl = int(self.headers.get("Content-Length", "0") or 0)
                except ValueError:
                    cl = 0
                if cl < 0:
                    # `Content-Length: -1` (or any negative integer)
                    # would make `rfile.read(-1)` consume to EOF —
                    # an unbounded read. Treat as malformed.
                    self.send_response(400)
                    self.end_headers()
                    return
                if cl > self._MAX_BODY_BYTES:
                    self.send_response(413)
                    self.end_headers()
                    return
                body = self.rfile.read(cl) if cl > 0 else b""
                auth = self.headers.get("Authorization")
                status = adapter._handle_webhook_body(body, auth, emit)
                self.send_response(status)
                self.end_headers()
                if status == 200:
                    self.wfile.write(b"OK")

            def log_message(self, fmt: str, *args: Any) -> None:  # noqa: A003
                return

        return _TeamsWebhookHandler

    def _serve_forever(
        self,
        emit: Callable[[dict], None],
        ready: threading.Event,
    ) -> None:
        # Validate credentials by acquiring a token — teams.rs:381-386
        # does the same fail-fast at adapter start so a misconfigured
        # app password surfaces immediately instead of on first send.
        try:
            self._get_token()
            log.info("teams authenticated", app_id=self.app_id)
        except Exception as e:  # noqa: BLE001
            log.error("teams OAuth2 token acquisition failed", error=str(e))
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
            log.error("teams webhook bind failed",
                      host=self.bind_host, port=self.webhook_port,
                      error=str(e))
            ready.set()
            return

        self._httpd = httpd
        ready.set()
        log.info("teams webhook listening",
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
        ready = threading.Event()
        t = threading.Thread(
            target=self._serve_forever,
            args=(emit, ready),
            name="teams-webhook",
            daemon=True,
        )
        t.start()
        while not ready.is_set():
            await asyncio.sleep(0.05)
        if self._httpd is None:
            raise RuntimeError(
                "teams sidecar failed to start its webhook server; "
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
                name="teams-shutdown", daemon=True,
            ).start()
        except Exception:  # noqa: BLE001
            pass

    async def on_shutdown(self) -> None:
        self._shutdown_server()

    async def on_command(self, cmd) -> None:
        """Dispatch inbound daemon commands. `Send` falls through to
        `on_send`; `TypingCmd` triggers a best-effort typing post."""
        from librefang.sidecar.protocol import Send, TypingCmd
        if isinstance(cmd, TypingCmd):
            conv = cmd.channel_id or ""
            if not conv:
                return
            loop = asyncio.get_event_loop()
            await loop.run_in_executor(None, self._send_typing, conv)
            return
        if isinstance(cmd, Send):
            await self.on_send(cmd)
            return

    async def on_send(self, cmd) -> None:
        conv = (
            cmd.channel_id
            or (cmd.user.get("platform_id") if cmd.user else "")
            or ""
        )
        if not conv:
            log.warn("teams on_send: empty conversation id, dropping")
            return

        content = cmd.content
        text = cmd.text or ""
        if isinstance(content, dict) and "Text" in content:
            inner = content["Text"]
            if isinstance(inner, str):
                text = inner
        elif content and not (isinstance(content, dict) and "Text" in content):
            text = "(Unsupported content type)"

        if not text:
            return

        loop = asyncio.get_event_loop()
        try:
            await loop.run_in_executor(
                None, lambda: self._send_text(conv, text),
            )
        except Exception as e:  # noqa: BLE001
            log.error("teams send failed", to=conv, error=str(e))
            raise


if __name__ == "__main__":
    run_stdio_main(TeamsAdapter)
