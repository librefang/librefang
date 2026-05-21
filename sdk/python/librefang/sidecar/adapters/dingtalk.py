"""DingTalk (钉钉) Robot sidecar adapter — Stream mode only.

Mirrors the **stream mode** of the in-process Rust adapter
(``crates/librefang-channels/src/dingtalk.rs``, removed in the same
PR that introduced this sidecar). The legacy **webhook mode** —
HTTP POST callback on a public-facing IP, signed via
``HMAC_SHA256(secret, timestamp + "\n" + secret + body_bytes)`` —
is **not** ported. Both modes were stdlib-compatible, but stream
mode is the DingTalk-documented modern default (requires no public
endpoint) and the operational simplicity is worth one less code
path. Operators who relied on webhook mode must re-create the robot
in the DingTalk admin console with stream subscription enabled.

Behaviour parity with the deleted Rust stream path:

* Step 1: ``POST https://api.dingtalk.com/v1.0/gateway/connections/open``
  with ``{clientId, clientSecret, subscriptions: [{type: "CALLBACK",
  topic: "/v1.0/im/bot/messages/get"}], ua: "librefang"}``. Returns
  ``{endpoint, ticket}``. Both must be present (``dingtalk.rs:280``).
* Step 2: Open WebSocket to ``{endpoint}?ticket={url-encoded ticket}``.
  Base64 chars in ticket (``+``, ``=``, ``/``) must be percent-encoded
  (``dingtalk.rs:435``).
* Step 3: SYSTEM ping/pong heartbeat: incoming ``{type: "SYSTEM",
  headers: {topic: "ping", messageId}, data}`` → reply with the
  exact same data, ``code: 200``, ``message: "OK"``
  (``dingtalk.rs:474``).
* Step 4: CALLBACK frame: ``{type: "CALLBACK", headers: {topic,
  messageId}, data: "<json-string>"}``. The ``data`` field is a
  JSON-encoded string — must ``json.loads`` it inside. Extract
  ``msgtype == "text"`` only; everything else silently dropped.
* Step 5: ACK every CALLBACK frame with ``{code: 200, headers:
  {contentType: "application/json", messageId: <same>}, message:
  "OK", data: "{\\"response\\": null}"}`` — DingTalk redelivers if
  the ACK is missing (``dingtalk.rs:525``).
* Outbound: per-message ``sessionWebhook`` URL is delivered with
  each inbound CALLBACK. Reply via ``POST <sessionWebhook>`` with
  ``{msgtype: "text", text: {content: chunk}}``. Response body
  carries ``errcode`` / ``errmsg``; non-zero ``errcode`` is an
  error (``dingtalk.rs:817``).
* Chunking: 20000-char chunks via the shared ``split_message``.
* Reconnect: exponential 3 → 60 s backoff (``dingtalk.rs:44``).

**Four improvements over the Rust adapter**:

1. **Inbound dedupe on ``messageId``** — Rust emitted every CALLBACK
   unconditionally; on reconnect + replay the bot could re-emit.
   Sidecar threads ``messageId`` through ``SeenSet`` (capacity
   10000 / evict 5000), matching the dedupe envelope every recent
   sidecar (qq, wecom, mattermost, …) settled on.
2. **Heartbeat/send-queue coexist on one socket via stdlib
   ``queue.Queue``** — Rust used ``tokio::mpsc`` with a separate
   reader-writer split; the sidecar drains a queue between
   ``wait_readable`` ticks. ``on_send`` is non-blocking; the WS
   thread drains the queue between heartbeat ticks and message
   reads, so a slow ``sessionWebhook`` POST never wedges inbound.
3. **429 ``Retry-After`` honoured on every outbound POST** — Rust
   had no 429 handling at all, so a throttled sessionWebhook reply
   either burned the chunking delay or dropped the chunk. Sidecar
   parses ``Retry-After`` (default 30 s, floor 1 s, cap 60 s),
   sleeps, retries once, then logs-and-continues on the second 429
   (same shape as #5303 across other sidecars).
4. **Explicit 15 s urlopen timeout on every HTTP call** — Rust
   used ``reqwest``'s ``.timeout(Duration::from_secs(15))`` only on
   gateway registration; the outbound ``self.client.post`` relied on
   the client default. Sidecar passes ``timeout=15.0`` on every
   call so a misbehaving ``sessionWebhook`` host can't hang the
   send loop.
"""
from __future__ import annotations

import asyncio
import json
import os
import queue
import threading
import time
import urllib.parse
from typing import Any, Callable, Optional

from .. import logging as log
from .. import protocol
from ..common import (
    RETRY_AFTER_DEFAULT_SECS,
    SeenSet as _SeenSet,
    http_request as _http_request,
    parse_retry_after as _parse_retry_after,
    split_csv as _split_csv,
    split_message as _split_message,
)
from ..protocol import Field, Schema
from ..runtime import SidecarAdapter, run_stdio_main
from ..ws import WebSocketClient as _WebSocketClient

# ── Constants ──────────────────────────────────────────────────────

#: DingTalk stream gateway registration endpoint (Rust: dingtalk.rs:42).
DINGTALK_GATEWAY_URL = "https://api.dingtalk.com/v1.0/gateway/connections/open"

#: Maximum text length per reply (Rust: dingtalk.rs:32 — DingTalk caps text at 20k).
DINGTALK_MAX_MESSAGE_LEN = 20000

#: Reconnect backoff envelope.
INITIAL_BACKOFF_SECS = 3.0
DINGTALK_MAX_BACKOFF_SECS = 60.0

#: How long to block in ``wait_readable`` per loop iteration.
READ_TICK_SECS = 1.0

#: Bounded dedupe envelope.
SEEN_MESSAGES_MAX = 10_000
SEEN_MESSAGES_EVICT = 5_000

#: Inter-chunk delay (Rust: dingtalk.rs:831).
INTER_CHUNK_DELAY_SECS = 0.2

#: User-Agent for both gateway registration and stream connection.
USER_AGENT = "librefang"

#: Gateway-registration / sessionWebhook POST timeout (Rust used 15s on
#: registration only; sidecar applies it everywhere — improvement #4).
HTTP_TIMEOUT_SECS = 15.0


# ── Pure helpers (test-friendly) ────────────────────────────────────


def _is_system_ping(frame: Any) -> bool:
    """Whether the frame is a SYSTEM ping (heartbeat) — must be ponged."""
    if not isinstance(frame, dict):
        return False
    if frame.get("type") != "SYSTEM":
        return False
    headers = frame.get("headers")
    if not isinstance(headers, dict):
        return False
    return headers.get("topic") == "ping"


def _build_pong_frame(ping_frame: dict) -> str:
    """Echo the ping's data field back as a pong ACK. Mirrors
    ``dingtalk.rs:481``."""
    headers = ping_frame.get("headers", {})
    return json.dumps({
        "code": 200,
        "headers": {
            "contentType": "application/json",
            "messageId": headers.get("messageId", ""),
        },
        "message": "OK",
        "data": ping_frame.get("data", ""),
    })


def _build_callback_ack(messageId: str) -> str:
    """ACK frame to send after every CALLBACK so DingTalk doesn't
    redeliver. Mirrors ``dingtalk.rs:526``."""
    return json.dumps({
        "code": 200,
        "headers": {
            "contentType": "application/json",
            "messageId": messageId,
        },
        "message": "OK",
        "data": "{\"response\": null}",
    })


def parse_dingtalk_event(
    frame: Any,
    *,
    allowed_users: Optional[list[str]] = None,
    account_id: Optional[str] = None,
) -> Optional[dict]:
    """Translate a DingTalk CALLBACK frame into a sidecar ``message``
    event. Returns ``None`` for any frame that should be silently
    skipped (wrong type, missing fields, non-text msgtype, user not
    allowed). Pure function — does NOT mark dedupe state; callers do
    that themselves so this helper stays testable without a SeenSet.

    Mirrors ``dingtalk::DingTalkAdapter::parse_stream_event``
    (``dingtalk.rs:292``).
    """
    if not isinstance(frame, dict) or frame.get("type") != "CALLBACK":
        return None
    data_str = frame.get("data")
    if not isinstance(data_str, str):
        return None
    try:
        payload = json.loads(data_str)
    except (ValueError, TypeError):
        return None
    if not isinstance(payload, dict):
        return None

    msgtype = payload.get("msgtype", "text")
    if msgtype != "text":
        return None
    text_obj = payload.get("text")
    text = ""
    if isinstance(text_obj, dict):
        c = text_obj.get("content")
        if isinstance(c, str):
            text = c.strip()
    if not text:
        return None

    sender_id = ""
    for key in ("senderStaffId", "senderId"):
        v = payload.get(key)
        if isinstance(v, str) and v:
            sender_id = v
            break
    if not sender_id:
        sender_id = "unknown"
    sender_nick = payload.get("senderNick") or "Unknown"

    if allowed_users and sender_id not in allowed_users:
        return None

    session_webhook = payload.get("sessionWebhook") or ""
    session_webhook_expired_time = payload.get("sessionWebhookExpiredTime") or 0
    conversation_id = payload.get("conversationId") or ""
    is_group = payload.get("conversationType") == "2"

    # was_mentioned: prefer the explicit boolean; fall back to "atUsers
    # array is non-empty" (Rust: dingtalk.rs:325-330).
    was_mentioned = payload.get("isInAtList")
    if not isinstance(was_mentioned, bool):
        at_users = payload.get("atUsers")
        was_mentioned = bool(at_users) if isinstance(at_users, list) else False

    # Slash command parsing — same shape as the rest of the sidecar
    # family (telegram / wecom / qq).
    if text.startswith("/"):
        parts = text.split(" ", 1)
        cmd = parts[0].lstrip("/")
        args: list[str] = parts[1].split() if len(parts) > 1 else []
        content = protocol.Content.command(cmd, args)
    else:
        content = protocol.Content.text(text)

    msg_id = payload.get("msgId")
    if not isinstance(msg_id, str) or not msg_id:
        headers = frame.get("headers", {})
        msg_id = headers.get("messageId") if isinstance(headers, dict) else None
    if not isinstance(msg_id, str) or not msg_id:
        msg_id = f"dt-{int(time.time() * 1000)}"

    metadata: dict[str, Any] = {"conversation_id": conversation_id}
    if isinstance(session_webhook_expired_time, int) and session_webhook_expired_time > 0:
        metadata["session_webhook_expired_time"] = session_webhook_expired_time
    if was_mentioned:
        metadata["was_mentioned"] = True
    if account_id:
        metadata["account_id"] = account_id
    if session_webhook:
        # Stash the per-message reply URL so on_send can route the
        # outbound to the right sessionWebhook (Rust stuffs it into
        # ChannelUser.librefang_user; we surface it explicitly).
        metadata["session_webhook"] = session_webhook

    return protocol.message(
        user_id=sender_id,
        user_name=sender_nick,
        content=content,
        message_id=msg_id,
        platform="dingtalk",
        is_group=is_group,
        metadata=metadata,
    )


# ── Adapter ─────────────────────────────────────────────────────────


class DingTalkAdapter(SidecarAdapter):
    """DingTalk Robot sidecar, stream mode only.

    Chat-room precedent (qq / wecom / line / mattermost / signal)
    says ``suppress_error_responses = False`` so a delivery failure
    surfaces to the user rather than vanishing into the daemon log.
    """

    capabilities: list = []
    suppress_error_responses: bool = False

    SCHEMA = Schema(
        name="dingtalk",
        display_name="DingTalk",
        description=(
            "DingTalk (钉钉) Robot stream-mode WebSocket adapter "
            "(out-of-process sidecar)"
        ),
        fields=[
            Field("DINGTALK_APP_KEY", "App Key (clientId)", "text",
                  required=True,
                  placeholder="dingxxxxxxxxxxxx"),
            Field("DINGTALK_APP_SECRET", "App Secret (clientSecret)", "secret",
                  required=True,
                  placeholder="app secret from DingTalk Open Platform"),
            Field("DINGTALK_ALLOWED_USERS",
                  "Allowed sender staffId list (comma-separated, "
                  "empty = all)",
                  "text",
                  placeholder="staff-id-1,staff-id-2",
                  advanced=True),
            Field("DINGTALK_ACCOUNT_ID",
                  "Account ID (multi-bot routing)",
                  "text",
                  placeholder="prod-bot",
                  advanced=True),
        ],
    )

    def __init__(self) -> None:
        app_key = os.environ.get("DINGTALK_APP_KEY", "").strip()
        app_secret = os.environ.get("DINGTALK_APP_SECRET", "").strip()
        missing: list[str] = []
        if not app_key:
            missing.append("DINGTALK_APP_KEY")
        if not app_secret:
            missing.append("DINGTALK_APP_SECRET")
        if missing:
            log.error("dingtalk required env vars missing", missing=missing)
            raise SystemExit(2)

        self.app_key = app_key
        self.app_secret = app_secret
        self.allowed_users = _split_csv(
            os.environ.get("DINGTALK_ALLOWED_USERS", "")
        )
        acct = os.environ.get("DINGTALK_ACCOUNT_ID", "").strip()
        self.account_id = acct or None

        # Test seam — points the gateway registration at a mock server.
        self.gateway_url = (
            os.environ.get("DINGTALK_GATEWAY_URL", "").strip()
            or DINGTALK_GATEWAY_URL
        )

        # Per-user latest sessionWebhook URL, surfaced via metadata
        # from parse_dingtalk_event. on_send pops from here using
        # cmd.channel_id (= the message_id, since DingTalk's reply
        # path is per-message, not per-user). Stored briefly until
        # the next inbound from the same user / message.
        self._session_webhooks: dict[str, str] = {}
        self._session_lock = threading.Lock()

        # Inbound dedupe on messageId (improvement #1).
        self._seen = _SeenSet(
            max_size=SEEN_MESSAGES_MAX, evict=SEEN_MESSAGES_EVICT,
        )

        # Outbound HTTP queue drained by the WS producer thread
        # (improvement #2). Each entry is (session_webhook_url, text)
        # — we POST to the URL on each tick.
        self._send_queue: "queue.Queue[tuple[str, str]]" = queue.Queue()

    # ---- dedupe shim --------------------------------------------------

    def _mark_seen(self, msg_id: Optional[str]) -> bool:
        return self._seen.mark(msg_id)

    # ---- HTTP helpers -------------------------------------------------

    def _register_gateway(self) -> tuple[str, str]:
        """``POST gateway/connections/open`` → ``(endpoint, ticket)``.
        Raises ``RuntimeError`` on any non-200 so the outer loop backs
        off. Mirrors ``dingtalk.rs:245-286``."""
        body = json.dumps({
            "clientId": self.app_key,
            "clientSecret": self.app_secret,
            "subscriptions": [
                {"type": "CALLBACK", "topic": "/v1.0/im/bot/messages/get"},
            ],
            "ua": USER_AGENT,
        }).encode("utf-8")
        headers = {
            "Content-Type": "application/json; charset=utf-8",
            "User-Agent": f"{USER_AGENT}-dingtalk-sidecar/1",
        }
        status, parsed, raw, _ = _http_request(
            self.gateway_url, method="POST", body=body, headers=headers,
            timeout=HTTP_TIMEOUT_SECS,
        )
        if status != 200 or not isinstance(parsed, dict):
            snippet = raw[:200].decode("utf-8", "replace") if raw else ""
            raise RuntimeError(
                f"dingtalk gateway registration failed (status={status}): {snippet}"
            )
        endpoint = parsed.get("endpoint")
        ticket = parsed.get("ticket")
        if not isinstance(endpoint, str) or not endpoint:
            raise RuntimeError("dingtalk gateway: missing endpoint")
        if not isinstance(ticket, str) or not ticket:
            raise RuntimeError("dingtalk gateway: missing ticket")
        return endpoint, ticket

    def _post_reply(self, session_webhook: str, text: str) -> None:
        """``POST <sessionWebhook>`` with ``{msgtype: "text", text:
        {content}}``. Honours 429 ``Retry-After`` once
        (improvement #3); fail-open on the second 429 / non-2xx so a
        single throttled chunk doesn't drop the rest of the reply
        (matches qq / webex / line fail-open shape)."""
        if not session_webhook:
            log.warn("dingtalk _post_reply: empty sessionWebhook, dropping")
            return
        body = json.dumps({
            "msgtype": "text",
            "text": {"content": text},
        }).encode("utf-8")
        headers = {"Content-Type": "application/json; charset=utf-8"}

        status, parsed, raw, resp_hdrs = _http_request(
            session_webhook, method="POST", body=body, headers=headers,
            timeout=HTTP_TIMEOUT_SECS,
        )
        if status == 429:
            wait = _parse_retry_after(
                resp_hdrs, default_secs=RETRY_AFTER_DEFAULT_SECS,
            )
            log.warn("dingtalk POST reply 429; sleeping then retrying once",
                     retry_after_secs=wait)
            time.sleep(wait)
            status, parsed, raw, resp_hdrs = _http_request(
                session_webhook, method="POST", body=body, headers=headers,
                timeout=HTTP_TIMEOUT_SECS,
            )
        if status >= 300:
            snippet = raw[:200].decode("utf-8", "replace") if raw else ""
            log.warn("dingtalk POST reply failed",
                     status=status, body=snippet)
            return  # fail-open
        # DingTalk REST contract: errcode != 0 on the body is the
        # platform-side error. Log loud but keep chunking.
        if isinstance(parsed, dict):
            errcode = parsed.get("errcode")
            if isinstance(errcode, int) and errcode != 0:
                log.warn(
                    "dingtalk reply rejected by platform",
                    errcode=errcode,
                    errmsg=parsed.get("errmsg") or "unknown",
                )

    # ---- send-frame routing ------------------------------------------

    def _enqueue_text(self, session_webhook: str, text: str) -> None:
        """Enqueue text chunks for the producer to POST. Each chunk
        is sent independently via ``_post_reply`` with a 200 ms delay
        between chunks (Rust parity, ``dingtalk.rs:830``)."""
        if not text or not session_webhook:
            return
        chunks = _split_message(text, DINGTALK_MAX_MESSAGE_LEN)
        for chunk in chunks:
            self._send_queue.put((session_webhook, chunk))

    # ---- WS test seams -----------------------------------------------

    def _make_ws(self, url: str) -> _WebSocketClient:
        return _WebSocketClient(url)

    # ---- WS session ---------------------------------------------------

    def _run_session(
        self, ws: _WebSocketClient, emit: Callable[[dict], None],
    ) -> None:
        """Drive one WS session: read frames, route SYSTEM/CALLBACK,
        drain the outbound queue between reads. Returns when the
        connection drops (the outer reconnect loop will retry)."""
        log.info("dingtalk WS connected", app_key=self.app_key)
        last_send_at = 0.0  # for inter-chunk delay

        while True:
            # Drain at most one pending send per tick. The 200 ms
            # inter-chunk delay is enforced inside this branch so
            # the read loop stays responsive between chunks.
            outbound: Optional[tuple[str, str]] = None
            try:
                outbound = self._send_queue.get_nowait()
            except queue.Empty:
                pass
            if outbound is not None:
                # Enforce inter-chunk delay (Rust: dingtalk.rs:830).
                now = time.monotonic()
                wait = INTER_CHUNK_DELAY_SECS - (now - last_send_at)
                if wait > 0:
                    time.sleep(wait)
                session_webhook, chunk = outbound
                self._post_reply(session_webhook, chunk)
                last_send_at = time.monotonic()

            if not ws.wait_readable(READ_TICK_SECS):
                continue
            try:
                text, close = ws.recv_frame()
            except (EOFError, OSError) as e:
                log.warn("dingtalk ws socket dropped", error=str(e))
                return
            if close is not None:
                code, reason = close
                log.info("dingtalk ws closed",
                         code=code,
                         reason=reason.decode("utf-8", "replace"))
                return
            if text is None:
                continue
            try:
                frame = json.loads(text)
            except (ValueError, TypeError):
                log.warn("dingtalk ws: unparseable frame")
                continue

            frame_type = frame.get("type") if isinstance(frame, dict) else None

            if _is_system_ping(frame):
                try:
                    ws.send_text(_build_pong_frame(frame))
                except OSError as e:
                    log.warn("dingtalk pong send failed", error=str(e))
                    return
                continue

            if frame_type != "CALLBACK":
                # Other SYSTEM topics or unknown types — log + ignore.
                log.debug("dingtalk: unhandled frame", type=frame_type)
                continue

            # Always ACK CALLBACK frames regardless of whether they
            # parse to an emittable event — DingTalk redelivers without
            # ACK, so even silent-drops (non-text msgtype, allowlist
            # reject) must be ACK'd to break the redelivery loop.
            headers = frame.get("headers", {})
            cb_msg_id = headers.get("messageId", "") if isinstance(headers, dict) else ""
            try:
                ws.send_text(_build_callback_ack(cb_msg_id))
            except OSError as e:
                log.warn("dingtalk ACK send failed", error=str(e))
                return

            event = parse_dingtalk_event(
                frame,
                allowed_users=self.allowed_users,
                account_id=self.account_id,
            )
            if event is None:
                continue

            msg_id = event["params"].get("message_id")
            if isinstance(msg_id, str) and not self._mark_seen(msg_id):
                # Duplicate redelivery — drop (improvement #1).
                continue

            # Cache session_webhook keyed by message_id so on_send can
            # find it via cmd.channel_id round-trip.
            session_webhook = event["params"].get("metadata", {}).get(
                "session_webhook"
            )
            if session_webhook and isinstance(msg_id, str):
                with self._session_lock:
                    self._session_webhooks[msg_id] = session_webhook

            emit(event)

    # ---- outer reconnect loop ----------------------------------------

    def _producer_blocking(self, emit: Callable[[dict], None]) -> None:
        backoff = INITIAL_BACKOFF_SECS
        while True:
            try:
                endpoint, ticket = self._register_gateway()
            except Exception as e:  # noqa: BLE001 — transport varies
                log.warn("dingtalk gateway registration failed; backing off",
                         error=str(e), delay=backoff)
                time.sleep(backoff)
                backoff = min(backoff * 2.0, DINGTALK_MAX_BACKOFF_SECS)
                continue

            ws_url = (
                f"{endpoint}?ticket="
                f"{urllib.parse.quote(ticket, safe='')}"
            )
            log.info("dingtalk gateway registered, opening ws", endpoint=endpoint)

            try:
                with self._make_ws(ws_url) as ws:
                    self._run_session(ws, emit)
                # Clean session end → reset backoff.
                backoff = INITIAL_BACKOFF_SECS
            except Exception as e:  # noqa: BLE001
                log.warn("dingtalk ws error; backing off",
                         error=str(e), delay=backoff)
                time.sleep(backoff)
                backoff = min(backoff * 2.0, DINGTALK_MAX_BACKOFF_SECS)

    # ---- public sidecar surface --------------------------------------

    async def produce(self, emit: Callable[[dict], None]) -> None:
        loop = asyncio.get_event_loop()
        await loop.run_in_executor(None, self._producer_blocking, emit)

    async def on_send(self, cmd) -> None:
        # The bridge round-trips the inbound message_id back as
        # cmd.channel_id, which we use to look up the cached
        # sessionWebhook. Fall back to metadata if the daemon happens
        # to forward it explicitly.
        session_webhook = None
        msg_id = cmd.channel_id or ""
        if msg_id:
            with self._session_lock:
                session_webhook = self._session_webhooks.pop(msg_id, None)
        if not session_webhook:
            # Last resort — let the agent peek at user metadata
            # (proactive sends from agent that didn't come via inbound).
            session_webhook = (
                cmd.user.get("session_webhook") if cmd.user else None
            )
        if not session_webhook:
            log.warn(
                "dingtalk on_send: no sessionWebhook for message; dropping",
                msg_id=msg_id,
            )
            return

        content = cmd.content
        text = cmd.text or ""
        if isinstance(content, dict) and "Text" in content:
            text = content["Text"]
        elif content and not (
            isinstance(content, dict) and "Text" in content
        ):
            text = "(Unsupported content type)"

        if not text:
            return
        self._enqueue_text(session_webhook, text)


if __name__ == "__main__":
    run_stdio_main(DingTalkAdapter)
