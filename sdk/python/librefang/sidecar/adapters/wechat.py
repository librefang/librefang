#!/usr/bin/env python3
"""WeChat (personal account via iLink) sidecar channel adapter for LibreFang.

Replaces the former in-process Rust ``librefang-channels::wechat``
adapter, removed in this migration. Connects to Tencent's official
iLink API (``ilinkai.weixin.qq.com``) used by the WeChat ClawBot
plugin. Supports QR code login and long-polling for real-time
message delivery.

Behaviour parity with the Rust adapter — every assertion has a
file/line citation against ``crates/librefang-channels/src/wechat.rs``
on the pre-migration tree.

* **Auth**: ``WECHAT_BOT_TOKEN`` (persisted from a previous QR
  session) skips the login flow. When unset, the sidecar runs
  ``GET /ilink/bot/get_bot_qrcode?bot_type=3`` and polls
  ``/ilink/bot/get_qrcode_status?qrcode=…`` until ``confirmed``
  (5 min timeout). The QR code string is logged at INFO so the
  operator can scan from the WeChat app. Mirrors wechat.rs:174-247.

* **Common headers**: ``Content-Type: application/json``,
  ``AuthorizationType: ilink_bot_token``,
  ``X-WECHAT-UIN: <random>``, ``Authorization: Bearer <token>``.
  ``X-WECHAT-UIN`` is base64(decimal-stringified random u32) and
  pinned per adapter instance (wechat.rs:90-97).

* **Long-poll**: ``POST /ilink/bot/getupdates`` with
  ``{get_updates_buf: <cursor>, base_info: {channel_version:
  "1.0.2"}}``. Response carries the next ``get_updates_buf``,
  ``msgs[]``, an optional ``typing_ticket``, and a
  ``longpolling_timeout_ms`` hint the server already waited
  (wechat.rs:290-315).

* **Inbound parse**: only the first ``item_list[0]`` is read.
  Types: 1=text, 2=image, 3=voice, 4=file, 5=video. Anything
  else is silently dropped (wechat.rs:391-477).

* **Self-skip**: ``from_user_id.endswith("@im.bot")`` filters bot-
  originated messages out (wechat.rs:401-403).

* **Per-user reply context**: ``context_token`` from each inbound
  is stored under ``from_user_id`` and re-used as the
  ``context_token`` on the next outbound ``sendmessage``. Without
  it, replies don't thread back. Mirrors wechat.rs:667-677.

* **Outbound**: ``POST /ilink/bot/sendmessage`` with a per-chunk
  ``client_id = uuid4`` for idempotency, ``message_type: 2``,
  ``message_state: 2``, and ``item_list[0].type = 1`` (text). 4096-
  char chunking via the shared ``split_message`` helper. Media
  variants (image/file/video/voice) degrade to a placeholder
  text — Rust's adapter ships the same fallback (wechat.rs:749-768).

* **Allowlist**: empty list = allow all. Match by exact user_id
  string (no domain match — iLink user IDs are opaque hashes).

* **Multi-bot ``account_id``** metadata injection (#5003).

Improvements over the Rust adapter:

* **inbound dedupe on msg_id / svr_msg_id** — Rust emitted every
  parsed message unconditionally; a long-poll retry could
  re-deliver. Bounded ``SeenSet`` (10000 cap / 5000 evict).
* **429 Retry-After honoured on every REST call** — Rust had no
  429 handling at all.
* **explicit 30 s timeouts on every REST call** — Rust used
  ``reqwest``'s default (90 s); the sidecar tightens it so a
  wedged iLink endpoint doesn't pin the worker thread for a
  minute and a half.
"""
from __future__ import annotations

import asyncio
import base64
import json
import os
import random
import threading
import time
import urllib.parse
import uuid
from datetime import datetime, timedelta, timezone
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
# Constants — mirror crates/librefang-channels/src/wechat.rs.
# ---------------------------------------------------------------------------

ILINK_BASE = "https://ilinkai.weixin.qq.com"
CHANNEL_VERSION = "1.0.2"      # wechat.rs:29
MAX_MESSAGE_LEN = 4096         # wechat.rs:27
QR_LOGIN_TIMEOUT_SECS = 300.0  # wechat.rs:43

ITEM_TYPE_TEXT = 1
ITEM_TYPE_IMAGE = 2
ITEM_TYPE_VOICE = 3
ITEM_TYPE_FILE = 4
ITEM_TYPE_VIDEO = 5

SEND_TIMEOUT_SECS = 30.0
INITIAL_BACKOFF_SECS = 2.0
MAX_BACKOFF_SECS_DEFAULT = 60.0

# Dedupe envelope — same shape as recent sidecars.
SEEN_MESSAGES_MAX = 10_000
SEEN_MESSAGES_EVICT = 5_000


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------


def generate_wechat_uin() -> str:
    """Base64-encoded decimal string of a random u32. Pinned per
    adapter instance and threaded into every iLink request as
    ``X-WECHAT-UIN``. Mirrors wechat.rs:90-97."""
    n = random.randint(0, 2**32 - 1)
    return base64.b64encode(str(n).encode("ascii")).decode("ascii")


def parse_wechat_msg(
    msg: Any,
    *,
    account_id: Optional[str] = None,
) -> Optional[dict]:
    """Translate an iLink message blob into a sidecar ``message``
    event. Returns ``None`` for bot-originated messages, empty
    payloads, or unsupported item types. Pure function — does NOT
    mark dedupe state; callers do that themselves so this helper
    stays testable without a SeenSet."""
    if not isinstance(msg, dict):
        return None
    from_user_id = msg.get("from_user_id")
    if not isinstance(from_user_id, str) or not from_user_id:
        return None
    if from_user_id.endswith("@im.bot"):
        return None  # self-skip — wechat.rs:401-403

    to_user_id = msg.get("to_user_id") if isinstance(msg.get("to_user_id"), str) else ""
    context_token = (
        msg.get("context_token") if isinstance(msg.get("context_token"), str) else ""
    )
    message_type_raw = msg.get("message_type")
    try:
        message_type = int(message_type_raw) if message_type_raw is not None else 0
    except (TypeError, ValueError):
        message_type = 0

    items = msg.get("item_list")
    if not isinstance(items, list) or not items:
        return None
    item = items[0]
    if not isinstance(item, dict):
        return None
    item_type_raw = item.get("type")
    try:
        item_type = int(item_type_raw) if item_type_raw is not None else 0
    except (TypeError, ValueError):
        item_type = 0

    content: Optional[dict] = None
    if item_type == ITEM_TYPE_TEXT:
        text_obj = item.get("text_item")
        text = ""
        if isinstance(text_obj, dict):
            v = text_obj.get("text")
            if isinstance(v, str):
                text = v
        if not text:
            return None
        content = Content.text(text)
    elif item_type == ITEM_TYPE_IMAGE:
        img = item.get("image_item")
        url = ""
        if isinstance(img, dict):
            v = img.get("url") or img.get("cdn_url")
            if isinstance(v, str):
                url = v
        content = {"Image": {"url": url, "caption": None,
                              "mime_type": "image/jpeg"}}
    elif item_type == ITEM_TYPE_VOICE:
        vo = item.get("voice_item")
        url = ""
        duration = 0
        if isinstance(vo, dict):
            v = vo.get("url") or vo.get("cdn_url")
            if isinstance(v, str):
                url = v
            d = vo.get("duration")
            try:
                duration = int(d) if d is not None else 0
            except (TypeError, ValueError):
                duration = 0
        content = {"Voice": {"url": url, "caption": None,
                              "duration_seconds": duration}}
    elif item_type == ITEM_TYPE_FILE:
        f = item.get("file_item")
        url = ""
        filename = "file"
        if isinstance(f, dict):
            v = f.get("url") or f.get("cdn_url")
            if isinstance(v, str):
                url = v
            n = f.get("file_name")
            if isinstance(n, str) and n:
                filename = n
        content = {"File": {"url": url, "filename": filename}}
    elif item_type == ITEM_TYPE_VIDEO:
        v = item.get("video_item")
        url = ""
        duration = 0
        if isinstance(v, dict):
            u = v.get("url") or v.get("cdn_url")
            if isinstance(u, str):
                url = u
            d = v.get("duration")
            try:
                duration = int(d) if d is not None else 0
            except (TypeError, ValueError):
                duration = 0
        content = {"Video": {"url": url, "caption": None,
                              "duration_seconds": duration,
                              "filename": None}}
    else:
        return None  # unsupported item type

    msg_id = msg.get("msg_id") or msg.get("svr_msg_id")
    if not isinstance(msg_id, str):
        msg_id = ""

    display_name = msg.get("from_user_name") or msg.get("from_user_nick")
    if not isinstance(display_name, str) or not display_name:
        display_name = from_user_id

    metadata: dict[str, Any] = {
        "context_token": context_token,
        "to_user_id": to_user_id,
        "message_type": message_type,
    }
    if account_id is not None:
        metadata["account_id"] = account_id

    # `librefang_user` is the always-round-tripped carrier for the
    # per-user iLink `context_token`. The previous routing relied on
    # `self._user_context_tokens[user_id]` only — an in-memory dict
    # that vanished on sidecar restart, leaving the bot's first reply
    # after restart with an empty `context_token` (iLink may reject
    # or post out-of-thread). librefang_user round-trips bytewise
    # through the bridge via `ChannelUser.librefang_user`
    # (`crates/librefang-channels/src/sidecar.rs:766` inbound,
    # `:1204` outbound) so it survives serde + restart cleanly. The
    # process-local cache is kept as a freshness signal — the
    # context_token Lark/iLink last issued is more current than the
    # one stamped on whichever message the daemon happens to round-
    # trip back. See `on_send` for the precedence: cache first, then
    # `librefang_user`, then empty.
    return protocol.message(
        user_id=from_user_id,
        user_name=display_name,
        content=content,
        message_id=msg_id or None,
        channel_id=from_user_id,
        librefang_user=context_token or None,
        metadata=metadata,
    )


# ---------------------------------------------------------------------------
# Adapter
# ---------------------------------------------------------------------------


class WeChatAdapter(SidecarAdapter):
    """WeChat personal-account sidecar via the iLink protocol."""

    # `typing` — POST /ilink/bot/sendtyping handled by `_on_typing`
    # via TypingCmd. Same surface the Rust adapter offered at
    # wechat.rs:773-817. (No reaction / thread / streaming — iLink
    # has no analogue.)
    capabilities: list = ["typing"]
    suppress_error_responses: bool = False

    SCHEMA = Schema(
        name="wechat",
        display_name="WeChat",
        description=(
            "WeChat personal-account adapter via Tencent's iLink "
            "(open-source ClawBot) gateway. Out-of-process sidecar."
        ),
        fields=[
            Field("WECHAT_BOT_TOKEN",
                  "Bot token (leave blank to trigger QR login)",
                  "secret",
                  placeholder="(optional, populated by QR login)"),
            Field("WECHAT_ALLOWED_USERS",
                  "Allowed user IDs (comma-separated, empty = all)",
                  "text",
                  placeholder="hash1@im.wechat,hash2@im.wechat",
                  advanced=True),
            Field("WECHAT_ACCOUNT_ID",
                  "Account ID (multi-bot routing)", "text",
                  advanced=True),
            Field("WECHAT_INITIAL_BACKOFF_SECS",
                  "Initial backoff (seconds)", "text",
                  placeholder="2", advanced=True),
            Field("WECHAT_MAX_BACKOFF_SECS",
                  "Maximum backoff (seconds)", "text",
                  placeholder="60", advanced=True),
        ],
    )

    def __init__(self) -> None:
        # Bot token can be empty — sidecar triggers QR login on start.
        self.bot_token: Optional[str] = (
            os.environ.get("WECHAT_BOT_TOKEN", "").strip() or None
        )
        self.allowed_users = _split_csv(
            os.environ.get("WECHAT_ALLOWED_USERS", "")
        )
        acct = os.environ.get("WECHAT_ACCOUNT_ID", "").strip()
        self.account_id: Optional[str] = acct or None

        self.initial_backoff_secs = _env_float(
            "WECHAT_INITIAL_BACKOFF_SECS", INITIAL_BACKOFF_SECS,
        )
        self.max_backoff_secs = _env_float(
            "WECHAT_MAX_BACKOFF_SECS", MAX_BACKOFF_SECS_DEFAULT,
        )

        # Test seam — production deployments leave unset.
        self.api_base = (
            os.environ.get("WECHAT_API_BASE_OVERRIDE", "").strip()
            or ILINK_BASE
        )

        self.wechat_uin = generate_wechat_uin()

        self._token_lock = threading.Lock()
        self._cursor = ""
        self._typing_ticket: Optional[str] = None
        self._context_lock = threading.Lock()
        self._user_context_tokens: dict[str, str] = {}
        self._seen = _SeenSet(
            max_size=SEEN_MESSAGES_MAX, evict=SEEN_MESSAGES_EVICT,
        )
        self._shutdown = threading.Event()

    # ---- token mgmt --------------------------------------------------

    def _get_token(self) -> Optional[str]:
        with self._token_lock:
            return self.bot_token

    def _set_token(self, token: str) -> None:
        with self._token_lock:
            self.bot_token = token

    # ---- HTTP --------------------------------------------------------

    def _auth_headers(self, token: str) -> dict:
        return {
            "Content-Type": "application/json",
            "AuthorizationType": "ilink_bot_token",
            "X-WECHAT-UIN": self.wechat_uin,
            "Authorization": f"Bearer {token}",
            "User-Agent": "librefang-wechat-sidecar/1 (https://librefang.org)",
        }

    def _post_json(
        self,
        path: str,
        payload: Optional[dict],
        *,
        token: Optional[str] = None,
        method: str = "POST",
    ) -> tuple[int, Any, bytes, dict]:
        url = f"{self.api_base}{path}"
        headers = self._auth_headers(token) if token else {
            "Content-Type": "application/json",
            "X-WECHAT-UIN": self.wechat_uin,
            "User-Agent": "librefang-wechat-sidecar/1 (https://librefang.org)",
        }
        body_bytes: Optional[bytes] = None
        if payload is not None:
            body_bytes = json.dumps(payload).encode("utf-8")
        return _http_request(
            url, method=method, body=body_bytes, headers=headers,
            timeout=SEND_TIMEOUT_SECS,
        )

    def _get(self, path: str) -> tuple[int, Any, bytes, dict]:
        url = f"{self.api_base}{path}"
        headers = {
            "X-WECHAT-UIN": self.wechat_uin,
            "User-Agent": "librefang-wechat-sidecar/1 (https://librefang.org)",
        }
        return _http_request(
            url, method="GET", headers=headers, timeout=SEND_TIMEOUT_SECS,
        )

    # ---- QR login ----------------------------------------------------

    def _qr_login(
        self,
        emit: Optional[Callable[[dict], None]] = None,
    ) -> str:
        """Run the QR login flow. Returns the bot_token on success.
        Raises RuntimeError on timeout / unrecoverable failure.
        Mirrors wechat.rs:174-247.

        When ``emit`` is provided, also surface the QR lifecycle to the
        daemon via the ``qr_ready`` / ``qr_status`` sidecar events so
        the dashboard's ``GET /api/channels/{name}/qr`` projection can
        show the operator the scannable code without them having to
        read sidecar logs. The parameter is optional so the existing
        tests that drive ``_qr_login`` directly with no emit callback
        continue to work."""
        log.info("wechat starting QR code login flow")
        status, body, raw, _ = self._get(
            "/ilink/bot/get_bot_qrcode?bot_type=3",
        )
        if status != 200 or not isinstance(body, dict):
            snippet = raw[:200].decode("utf-8", "replace") if raw else ""
            msg = (
                f"wechat QR code request failed "
                f"(status={status}): {snippet}"
            )
            if emit is not None:
                emit(protocol.qr_status("failed", message=msg))
            raise RuntimeError(msg)
        qrcode = body.get("qrcode")
        if not isinstance(qrcode, str) or not qrcode:
            msg = "wechat QR response missing 'qrcode'"
            if emit is not None:
                emit(protocol.qr_status("failed", message=msg))
            raise RuntimeError(msg)

        log.info(
            "wechat QR code ready — scan with the WeChat app to log in",
            qrcode=qrcode,
        )

        encoded_qr = urllib.parse.quote(qrcode, safe="")
        deadline_monotonic = time.monotonic() + QR_LOGIN_TIMEOUT_SECS
        # ISO 8601 expiry derived from wall clock for the daemon /
        # dashboard projection — monotonic is for the local timeout
        # check only and is meaningless across process boundaries.
        expires_at = datetime.now(timezone.utc) + timedelta(
            seconds=QR_LOGIN_TIMEOUT_SECS,
        )
        if emit is not None:
            emit(protocol.qr_ready(
                qr_code=qrcode,
                # iLink does not surface a pre-formed deep-link URL —
                # the dashboard renders the raw qrcode string itself.
                qr_url=None,
                message=(
                    "Scan with the WeChat app — 5-minute window"
                ),
                expires_at=expires_at.isoformat(),
            ))

        backoff = self.initial_backoff_secs

        while time.monotonic() < deadline_monotonic:
            if self._shutdown.is_set():
                if emit is not None:
                    emit(protocol.qr_status(
                        "failed",
                        message="QR login cancelled by shutdown",
                    ))
                raise RuntimeError("wechat QR login cancelled by shutdown")
            status, body, _raw, _hdrs = self._get(
                f"/ilink/bot/get_qrcode_status?qrcode={encoded_qr}",
            )
            if status == 200 and isinstance(body, dict):
                qr_status_str = body.get("status")
                if qr_status_str == "confirmed":
                    token = body.get("bot_token")
                    if not isinstance(token, str) or not token:
                        msg = "wechat QR confirmed but bot_token missing"
                        if emit is not None:
                            emit(protocol.qr_status(
                                "failed", message=msg,
                            ))
                        raise RuntimeError(msg)
                    # The Rust adapter relied on the dashboard's
                    # /channels/wechat/qr/start + /qr/status endpoints
                    # to capture the token and write it to secrets.env.
                    # Those routes are gone in the sidecar — surface
                    # the token at DEBUG so an operator running the
                    # sidecar at -v can copy it into
                    # ~/.librefang/secrets.env as WECHAT_BOT_TOKEN to
                    # skip QR login on subsequent restarts. INFO-level
                    # message intentionally omits the token to keep
                    # production logs free of secrets.
                    log.info(
                        "wechat QR login successful — set WECHAT_BOT_TOKEN "
                        "in ~/.librefang/secrets.env to skip QR on next "
                        "restart (token logged at DEBUG)",
                    )
                    log.debug("wechat captured bot_token", bot_token=token)
                    if emit is not None:
                        # Token is NOT in the protocol event — see
                        # `protocol.qr_status` docstring for why. The
                        # dashboard surfaces this message verbatim;
                        # the operator copies the token (logged at
                        # DEBUG above) into secrets.env by hand.
                        emit(protocol.qr_status(
                            "confirmed",
                            message=(
                                "Login successful. To skip QR on next "
                                "restart, set WECHAT_BOT_TOKEN in "
                                "~/.librefang/secrets.env (the token "
                                "is logged at DEBUG by this sidecar)"
                            ),
                        ))
                    return token
                if qr_status_str == "expired":
                    msg = "wechat QR code expired — restart to try again"
                    if emit is not None:
                        emit(protocol.qr_status(
                            "expired", message=msg,
                        ))
                    raise RuntimeError(msg)
                log.debug("wechat QR status pending", status=qr_status_str)
            else:
                log.warn("wechat QR status poll non-200", status=status)

            if self._shutdown.wait(backoff):
                if emit is not None:
                    emit(protocol.qr_status(
                        "failed",
                        message="QR login cancelled by shutdown",
                    ))
                raise RuntimeError(
                    "wechat QR login cancelled by shutdown",
                )
            backoff = min(backoff * 2.0, 5.0)
        msg = "wechat QR login timed out (5 minutes) — restart to try again"
        if emit is not None:
            emit(protocol.qr_status("expired", message=msg))
        raise RuntimeError(msg)

    # ---- typing ticket -----------------------------------------------

    def _refresh_typing_ticket(self, token: str) -> None:
        try:
            status, body, _raw, _hdrs = self._post_json(
                "/ilink/bot/getconfig", {}, token=token,
            )
        except Exception as e:  # noqa: BLE001 — best effort
            log.warn("wechat getconfig error", error=str(e))
            return
        if status != 200 or not isinstance(body, dict):
            log.warn("wechat getconfig non-200", status=status)
            return
        v = body.get("typing_ticket")
        if isinstance(v, str) and v:
            self._typing_ticket = v

    # ---- long-poll ---------------------------------------------------

    def _poll_updates(self, token: str) -> Optional[dict]:
        body = {
            "get_updates_buf": self._cursor,
            "base_info": {"channel_version": CHANNEL_VERSION},
        }
        status, resp, raw, hdrs = self._post_json(
            "/ilink/bot/getupdates", body, token=token,
        )
        if status == 429:
            wait = _parse_retry_after(
                hdrs, default_secs=30.0,
                floor_secs=1.0, max_secs=self.max_backoff_secs,
            )
            log.warn("wechat getupdates 429; sleeping", retry_after=wait)
            if self._shutdown.wait(wait):
                return None
            status, resp, raw, hdrs = self._post_json(
                "/ilink/bot/getupdates", body, token=token,
            )
        if status in (401, 403):
            # Token expired / revoked. Clear the cache so the next
            # poll-loop iteration triggers a fresh QR login instead
            # of looping forever against a dead token.
            snippet = raw[:200].decode("utf-8", "replace") if raw else ""
            log.error(
                "wechat token rejected by iLink; "
                "clearing cache and re-running QR login on next iteration",
                status=status, snippet=snippet,
            )
            with self._token_lock:
                self.bot_token = None
            raise RuntimeError(
                f"wechat getupdates auth rejected (status={status})",
            )
        if status != 200 or not isinstance(resp, dict):
            snippet = raw[:200].decode("utf-8", "replace") if raw else ""
            raise RuntimeError(
                f"wechat getupdates failed "
                f"(status={status}): {snippet}",
            )
        return resp

    # ---- outbound send -----------------------------------------------

    def _send_text(
        self, to_user_id: str, context_token: str, text: str,
    ) -> None:
        token = self._get_token()
        if token is None:
            raise RuntimeError("wechat send: not logged in")
        if not text:
            return
        chunks = _split_message(text, MAX_MESSAGE_LEN)
        for chunk in chunks:
            client_id = str(uuid.uuid4())
            body = {
                "msg": {
                    "from_user_id": "",
                    "to_user_id": to_user_id,
                    "client_id": client_id,
                    "message_type": 2,
                    "message_state": 2,
                    "context_token": context_token,
                    "item_list": [{
                        "type": ITEM_TYPE_TEXT,
                        "text_item": {"text": chunk},
                    }],
                },
                "base_info": {"channel_version": CHANNEL_VERSION},
            }
            status, resp, raw, hdrs = self._post_json(
                "/ilink/bot/sendmessage", body, token=token,
            )
            if status == 429:
                wait = _parse_retry_after(
                    hdrs, default_secs=30.0,
                    floor_secs=1.0, max_secs=self.max_backoff_secs,
                )
                log.warn(
                    "wechat sendmessage 429; sleeping",
                    retry_after=wait,
                )
                if self._shutdown.wait(wait):
                    return
                status, resp, raw, hdrs = self._post_json(
                    "/ilink/bot/sendmessage", body, token=token,
                )
            if status in (401, 403):
                # Outbound saw the token expire. Clear the cache so
                # the next poll-loop iteration re-runs QR; the send
                # still fails (caller's responsibility to retry), but
                # subsequent inbounds will resume working once the
                # operator re-scans.
                with self._token_lock:
                    self.bot_token = None
                snippet = raw[:200].decode("utf-8", "replace") if raw else ""
                raise RuntimeError(
                    f"wechat sendmessage auth rejected "
                    f"(status={status}): {snippet}",
                )
            if status < 200 or status >= 300:
                snippet = raw[:200].decode("utf-8", "replace") if raw else ""
                raise RuntimeError(
                    f"wechat sendmessage error "
                    f"(status={status}): {snippet}",
                )

    def _send_typing(self, to_user_id: str) -> None:
        """``POST /ilink/bot/sendtyping`` with the cached
        ``typing_ticket``. Best-effort: no token / no ticket /
        non-200 are all silent no-ops, mirroring wechat.rs:773-817."""
        if not to_user_id:
            return
        token = self._get_token()
        if token is None:
            return  # not logged in yet
        ticket = self._typing_ticket
        if not ticket:
            return  # no ticket primed yet (getconfig pending)
        body = {"to_user_id": to_user_id, "typing_ticket": ticket}
        try:
            status, _resp, _raw, _hdrs = self._post_json(
                "/ilink/bot/sendtyping", body, token=token,
            )
        except Exception as e:  # noqa: BLE001 — best-effort
            log.debug("wechat sendtyping error", error=str(e))
            return
        if status < 200 or status >= 300:
            log.debug("wechat sendtyping non-2xx", status=status)

    # ---- sidecar surface ---------------------------------------------

    async def produce(self, emit: Callable[[dict], None]) -> None:
        loop = asyncio.get_event_loop()
        await loop.run_in_executor(None, self._poll_loop, emit)

    async def on_shutdown(self) -> None:
        self._shutdown.set()

    async def on_command(self, cmd) -> None:
        """Dispatch inbound daemon commands. `Send` falls through to
        the base class which routes to `on_send`; `TypingCmd` triggers
        a best-effort `sendtyping` post."""
        from librefang.sidecar.protocol import Send, TypingCmd
        if isinstance(cmd, TypingCmd):
            user_id = cmd.channel_id or ""
            if not user_id:
                return
            loop = asyncio.get_event_loop()
            await loop.run_in_executor(None, self._send_typing, user_id)
            return
        if isinstance(cmd, Send):
            await self.on_send(cmd)
            return

    async def on_send(self, cmd) -> None:
        user_id = (
            cmd.channel_id
            or (cmd.user.get("platform_id") if cmd.user else "")
            or ""
        )
        if not user_id:
            log.warn("wechat on_send: missing user_id, dropping")
            return

        # Precedence: process-local cache (freshest token Lark/iLink
        # issued) → `cmd.user["librefang_user"]` (round-tripped from
        # the inbound that triggered this reply; survives sidecar
        # restart). Both are best-effort; an empty token still posts
        # the message (iLink falls back to a fresh top-level frame).
        with self._context_lock:
            context_token = self._user_context_tokens.get(user_id, "")
        if not context_token and cmd.user:
            candidate = cmd.user.get("librefang_user")
            # Guard: librefang_user is shared across channels (dingtalk
            # puts a sessionWebhook URL, telegram puts @username, …).
            # iLink context_token is an opaque base64-ish string —
            # generic URL/whitespace/@ guard is enough.
            if (isinstance(candidate, str) and candidate
                    and not candidate.startswith(("http://", "https://", "@"))
                    and " " not in candidate
                    and "\t" not in candidate):
                context_token = candidate

        content = cmd.content
        text = cmd.text or ""
        if isinstance(content, dict) and "Text" in content:
            inner = content["Text"]
            if isinstance(inner, str):
                text = inner
        elif content and not (isinstance(content, dict) and "Text" in content):
            # Media variants degrade to placeholder (matches Rust
            # wechat.rs:749-768 — media upload is not yet wired).
            text = "[Unsupported content type — media upload not yet supported]"

        if not text:
            return

        loop = asyncio.get_event_loop()
        try:
            await loop.run_in_executor(
                None,
                lambda: self._send_text(user_id, context_token, text),
            )
        except Exception as e:  # noqa: BLE001
            log.error("wechat send failed", to=user_id, error=str(e))
            raise

    # ---- poll loop ---------------------------------------------------

    def _poll_loop(self, emit: Callable[[dict], None]) -> None:
        # Step 1: log in (QR or persisted token).
        if self._get_token() is None:
            try:
                token = self._qr_login(emit=emit)
            except Exception as e:  # noqa: BLE001
                if self._shutdown.is_set():
                    return
                log.error("wechat QR login failed", error=str(e))
                raise
            self._set_token(token)
        else:
            log.info("wechat using persisted bot token")

        # Step 2: prime the typing ticket.
        token = self._get_token() or ""
        if token:
            self._refresh_typing_ticket(token)

        log.info("wechat starting message polling loop")
        backoff = self.initial_backoff_secs
        while not self._shutdown.is_set():
            token = self._get_token()
            if token is None:
                # Token cleared by an auth-rejection path
                # (`_poll_updates` 401/403 handler). Re-run QR login
                # rather than spinning. If the persisted env-var
                # token was already wrong on first start, the same
                # flow makes the operator aware via the QR-code log.
                log.info("wechat re-running QR login (token cleared)")
                try:
                    token = self._qr_login(emit=emit)
                except Exception as e:  # noqa: BLE001
                    if self._shutdown.is_set():
                        return
                    log.error("wechat QR re-login failed", error=str(e))
                    if self._shutdown.wait(backoff):
                        return
                    backoff = min(backoff * 2.0, self.max_backoff_secs)
                    continue
                self._set_token(token)
                continue
            try:
                data = self._poll_updates(token)
            except Exception as e:  # noqa: BLE001
                if self._shutdown.is_set():
                    return
                log.warn("wechat poll error; backing off",
                         error=str(e), delay=backoff)
                if self._shutdown.wait(backoff):
                    return
                backoff = min(backoff * 2.0, self.max_backoff_secs)
                continue
            if data is None:
                return
            backoff = self.initial_backoff_secs

            cursor = data.get("get_updates_buf")
            if isinstance(cursor, str):
                self._cursor = cursor

            ticket = data.get("typing_ticket")
            if isinstance(ticket, str) and ticket:
                self._typing_ticket = ticket

            msgs = data.get("msgs")
            if isinstance(msgs, list):
                self._dispatch_messages(msgs, emit)

            timeout_ms = data.get("longpolling_timeout_ms")
            try:
                timeout_ms_int = int(timeout_ms) if timeout_ms is not None else 0
            except (TypeError, ValueError):
                timeout_ms_int = 0
            if timeout_ms_int == 0:
                # Server didn't hold the connection; pace ourselves.
                if self._shutdown.wait(1.0):
                    return

    def _dispatch_messages(
        self, msgs: list, emit: Callable[[dict], None],
    ) -> None:
        for msg in msgs:
            ev = parse_wechat_msg(msg, account_id=self.account_id)
            if ev is None:
                continue
            params = ev.get("params", {})
            from_user = params.get("user_id", "")

            if self.allowed_users and from_user not in self.allowed_users:
                log.debug("wechat sender not in allowlist, dropping",
                          user=from_user)
                continue

            # Inbound dedupe on message_id (improvement over Rust).
            msg_id = params.get("message_id")
            if isinstance(msg_id, str) and msg_id and not self._seen.mark(msg_id):
                log.debug("wechat duplicate msg_id, dropping",
                          message_id=msg_id)
                continue

            # Stash reply context BEFORE emit so a Send back from the
            # daemon picks it up immediately. Only store non-empty
            # `context_token` — an empty token from a subsequent
            # inbound (rare but possible per the iLink protocol,
            # e.g. system events) would otherwise blow away the real
            # token from an earlier user message and break threading
            # on the very next outbound.
            meta = params.get("metadata", {})
            ctx = meta.get("context_token") if isinstance(meta, dict) else None
            if isinstance(ctx, str) and ctx:
                with self._context_lock:
                    self._user_context_tokens[from_user] = ctx
            emit(ev)


# ---------------------------------------------------------------------------
# Env helpers
# ---------------------------------------------------------------------------


def _env_float(name: str, default: float) -> float:
    raw = os.environ.get(name, "").strip()
    if not raw:
        return default
    try:
        v = float(raw)
        if v <= 0:
            return default
        return v
    except ValueError:
        log.warn(
            f"wechat {name} not a number; using default",
            value=raw, default=default,
        )
        return default


if __name__ == "__main__":
    run_stdio_main(WeChatAdapter)
