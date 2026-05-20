"""WeCom (企业微信) intelligent-bot sidecar adapter.

Mirrors the WebSocket mode of the in-process Rust adapter
(``crates/librefang-channels/src/wecom.rs``, removed in the same PR
that introduced this sidecar). The legacy **callback mode** —
HTTP webhook + AES-CBC-256 inbound payload decryption + HMAC-SHA1
signature verification — is **not** ported: Python's stdlib has no
AES, and the sidecar SDK contract forbids third-party deps. Operators
who relied on callback mode must either switch the bot to WebSocket
mode in the WeCom admin console or run a custom callback adapter that
brings its own AES implementation.

Behaviour parity with the deleted Rust WebSocket path:

* Connects to ``wss://openws.work.weixin.qq.com``.
* Subscribes with ``{"cmd": "aibot_subscribe", "body": {"bot_id":…,
  "secret":…}}`` and treats either an explicit ``cmd: "aibot_subscribe"``
  ack OR a server-style ack ``{"errcode": 0, "headers": {"req_id":
  "aibot_subscribe_…"}}`` as success (wecom.rs:128-142).
* Heartbeat: ``{"cmd": "ping"}`` every 30 s (wecom.rs:37, 612-622).
* Inbound: ``cmd: "aibot_msg_callback"`` frames; supports both
  ``cmd``/``action`` and ``body``/``data`` legacy keys; supports both
  ``userid``/``user_id`` and ``chattype``/``chat_type``; only
  ``msgtype: "text"`` is forwarded (wecom.rs:46-121).
* Outbound: if the user has a cached inbound ``req_id``, reply via
  ``cmd: "aibot_respond_msg"`` (one-shot, evicted on send); otherwise
  send proactively via ``cmd: "aibot_send_msg"``. Body is always
  ``msgtype: "markdown"`` since WeCom's ``aibot_respond_msg`` rejects
  plain text (wecom.rs:494-533).
* Chunking: 4096 chars per chunk via the shared ``split_message``.
* Reconnect: exponential 1 s → 30 s backoff on any error (wecom.rs:40-42).

**Three improvements over the Rust adapter**:

1. **Inbound dedupe on ``req_id``** — the Rust emit at wecom.rs:770 was
   unconditional, so a WS reconnect that races with the platform's
   redelivery would emit the same message twice. The sidecar threads
   ``req_id`` through ``SeenSet`` (capacity 10000, evict 5000), matching
   the dedupe envelope every other recent sidecar (qq, mattermost,
   signal, …) settled on.
2. **Heartbeat-and-send coexist on one socket via a stdlib ``queue.Queue``**
   — the Rust adapter used a bounded ``tokio::mpsc`` (wecom.rs:580); the
   sidecar uses an unbounded ``queue.Queue`` polled at the same read tick
   as inbound. on_send is non-blocking; the WS thread drains the queue
   between heartbeat ticks and message reads.
3. **Send result is observable in logs** — the Rust adapter only logged
   ``frame sent over WebSocket successfully`` (wecom.rs:631) before the
   server ACK arrived; the sidecar logs the same plus the server's
   ``errcode``/``errmsg`` (if non-zero) when the ACK frame echoes back,
   so operators can correlate a "send succeeded" log line with the
   actual platform-side outcome instead of having to enable DEBUG.
"""
from __future__ import annotations

import asyncio
import json
import os
import queue
import threading
import time
from typing import Any, Callable, Optional

from .. import logging as log
from .. import protocol
from ..common import (
    SeenSet as _SeenSet,
    split_csv as _split_csv,
    split_message as _split_message,
)
from ..protocol import Field, Schema
from ..runtime import SidecarAdapter, run_stdio_main
from ..ws import WebSocketClient as _WebSocketClient

# ── Constants ──────────────────────────────────────────────────────

#: WeCom intelligent-bot WebSocket endpoint.
WECOM_WS_URL = "wss://openws.work.weixin.qq.com"

#: Maximum text length per reply (matches Rust ``MAX_MESSAGE_LEN``).
WECOM_MAX_MESSAGE_LEN = 4096

#: WebSocket heartbeat interval.
HEARTBEAT_INTERVAL_SECS = 30.0

#: Reconnect backoff envelope.
INITIAL_BACKOFF_SECS = 1.0
WECOM_MAX_BACKOFF_SECS = 30.0  # The Rust adapter capped at 30 s, not 60 s.

#: How long to block in ``wait_readable`` per loop iteration. Lower
#: means snappier heartbeat / send-drain at the cost of wakeups.
READ_TICK_SECS = 1.0

#: Bounded dedupe envelope.
SEEN_MESSAGES_MAX = 10_000
SEEN_MESSAGES_EVICT = 5_000


# ── Pure helpers (test-friendly) ────────────────────────────────────


def _frame_cmd(frame: Any) -> Optional[str]:
    """Get ``cmd`` (preferred) or legacy ``action`` from a frame dict."""
    if not isinstance(frame, dict):
        return None
    v = frame.get("cmd")
    if isinstance(v, str):
        return v
    v = frame.get("action")
    if isinstance(v, str):
        return v
    return None


def _frame_body(frame: Any) -> Any:
    """Get ``body`` (preferred) or legacy ``data`` from a frame dict."""
    if not isinstance(frame, dict):
        return None
    return frame.get("body") if "body" in frame else frame.get("data")


def _frame_req_id(frame: Any) -> Optional[str]:
    """Extract ``req_id`` from ``headers.req_id`` first, then
    ``body.req_id`` / ``data.req_id`` as fallback."""
    if not isinstance(frame, dict):
        return None
    hdrs = frame.get("headers")
    if isinstance(hdrs, dict):
        v = hdrs.get("req_id")
        if isinstance(v, str):
            return v
    body = _frame_body(frame)
    if isinstance(body, dict):
        v = body.get("req_id")
        if isinstance(v, str):
            return v
    return None


def _is_subscribe_success(frame: Any) -> bool:
    """Whether a parsed frame is the ``aibot_subscribe`` ack.

    Two valid shapes:

    1. Explicit ``cmd: "aibot_subscribe"`` with ``errcode == 0`` or absent.
    2. Server-style ack (no ``cmd``) with ``errcode == 0`` and
       ``headers.req_id`` starting with ``"aibot_subscribe"``.

    Mirrors wecom.rs:128-142.
    """
    if not isinstance(frame, dict):
        return False
    errcode = frame.get("errcode")
    cmd = _frame_cmd(frame)
    if cmd is not None:
        if cmd != "aibot_subscribe":
            return False
        return errcode == 0 or errcode is None
    req_id = _frame_req_id(frame)
    if req_id is None:
        return False
    return req_id.startswith("aibot_subscribe") and errcode == 0


def parse_wecom_event(
    frame: Any,
    *,
    allowed_users: Optional[list[str]] = None,
    account_id: Optional[str] = None,
) -> Optional[dict]:
    """Translate a WeCom WS frame into a sidecar ``message`` event.

    Returns ``None`` for any frame that should be silently skipped
    (wrong cmd, missing fields, non-text msgtype, user not allowed).
    Pure function — does NOT mark dedupe state; callers do that
    themselves so this helper stays testable without a SeenSet.
    """
    if _frame_cmd(frame) != "aibot_msg_callback":
        return None
    body = _frame_body(frame)
    if not isinstance(body, dict):
        return None

    req_id = _frame_req_id(frame) or body.get("req_id")
    if not isinstance(req_id, str) or not req_id:
        return None

    from_obj = body.get("from")
    from_user = ""
    if isinstance(from_obj, dict):
        for key in ("userid", "user_id"):
            v = from_obj.get(key)
            if isinstance(v, str) and v:
                from_user = v
                break
    if not from_user:
        return None

    msgtype = body.get("msgtype")
    if msgtype != "text":
        # Image / voice / event / etc. are silently dropped — same as
        # wecom.rs:103-106 (only text replies are supported).
        return None

    text_obj = body.get("text")
    content_text = ""
    if isinstance(text_obj, dict):
        c = text_obj.get("content")
        if isinstance(c, str):
            content_text = c
    if not content_text:
        return None

    chat_raw = body.get("chattype")
    if chat_raw is None:
        chat_raw = body.get("chat_type")
    is_group = isinstance(chat_raw, str) and chat_raw == "group"

    if allowed_users and from_user not in allowed_users:
        return None

    metadata: dict[str, Any] = {"wecom_req_id": req_id}
    response_url = body.get("response_url")
    if isinstance(response_url, str) and response_url:
        # Surface for diagnostics; the WS-only sidecar doesn't use the
        # one-shot HTTPS URL (that's callback-mode territory), but the
        # bridge may want it for observability or future reuse.
        metadata["wecom_response_url"] = response_url
    if account_id:
        metadata["account_id"] = account_id

    return protocol.message(
        user_id=from_user,
        user_name=from_user,
        content=protocol.Content.text(content_text),
        message_id=req_id,
        platform="wecom",
        is_group=is_group,
        metadata=metadata,
    )


def _build_reply_frame(req_id: str, text: str) -> str:
    """Build an ``aibot_respond_msg`` frame for passive reply to an
    inbound message. WeCom rejects ``msgtype: "text"`` for this cmd —
    we always wrap as markdown (wecom.rs:494-513)."""
    return json.dumps({
        "cmd": "aibot_respond_msg",
        "headers": {"req_id": req_id},
        "body": {
            "msgtype": "markdown",
            "markdown": {"content": text},
        },
    })


def _build_send_frame(user_id: str, text: str) -> str:
    """Build a proactive ``aibot_send_msg`` frame (wecom.rs:515-533)."""
    return json.dumps({
        "cmd": "aibot_send_msg",
        "headers": {
            "req_id": f"aibot_send_msg_{int(time.time() * 1000)}",
        },
        "body": {
            "receiver": {"userid": user_id},
            "msgtype": "markdown",
            "markdown": {"content": text},
        },
    })


def _build_subscribe_frame(bot_id: str, secret: str) -> str:
    """Build the ``aibot_subscribe`` handshake frame (wecom.rs:586-596)."""
    return json.dumps({
        "cmd": "aibot_subscribe",
        "headers": {
            "req_id": f"aibot_subscribe_{int(time.time() * 1000)}",
        },
        "body": {"bot_id": bot_id, "secret": secret},
    })


def _build_ping_frame() -> str:
    """Build a heartbeat ping frame (wecom.rs:612-618)."""
    return json.dumps({
        "cmd": "ping",
        "headers": {"req_id": f"ping_{int(time.time() * 1000)}"},
    })


# ── Adapter ─────────────────────────────────────────────────────────


class WeComAdapter(SidecarAdapter):
    """WeCom intelligent-bot sidecar, WebSocket mode only.

    Chat-room precedent (qq / line / mattermost / signal) says
    ``suppress_error_responses = False`` so a delivery failure surfaces
    to the user rather than vanishing into the daemon log.
    """

    capabilities: list = []
    suppress_error_responses: bool = False

    SCHEMA = Schema(
        name="wecom",
        display_name="WeCom",
        description=(
            "WeCom (企业微信) intelligent-bot WebSocket adapter "
            "(out-of-process sidecar)"
        ),
        fields=[
            Field("WECOM_BOT_ID", "Bot ID", "text",
                  required=True,
                  placeholder="aibxxxxxxx"),
            Field("WECOM_BOT_SECRET", "Bot Secret", "secret",
                  required=True,
                  placeholder="bot secret from WeCom admin console"),
            Field("WECOM_ALLOWED_USERS",
                  "Allowed sender userid list (comma-separated, "
                  "empty = all)",
                  "text",
                  placeholder="alice,bob",
                  advanced=True),
            Field("WECOM_ACCOUNT_ID",
                  "Account ID (multi-bot routing)",
                  "text",
                  placeholder="prod-bot",
                  advanced=True),
        ],
    )

    def __init__(self) -> None:
        bot_id = os.environ.get("WECOM_BOT_ID", "").strip()
        secret = os.environ.get("WECOM_BOT_SECRET", "").strip()
        missing: list[str] = []
        if not bot_id:
            missing.append("WECOM_BOT_ID")
        if not secret:
            missing.append("WECOM_BOT_SECRET")
        if missing:
            log.error("wecom required env vars missing", missing=missing)
            raise SystemExit(2)

        self.bot_id = bot_id
        self.secret = secret
        self.allowed_users = _split_csv(
            os.environ.get("WECOM_ALLOWED_USERS", "")
        )
        acct = os.environ.get("WECOM_ACCOUNT_ID", "").strip()
        self.account_id = acct or None

        # Test seam — real deployments leave unset.
        self.ws_url = (
            os.environ.get("WECOM_WS_URL", "").strip() or WECOM_WS_URL
        )

        # Per-user latest req_id, for passive replies via
        # aibot_respond_msg. Consumed (and removed) on first send to
        # that user; subsequent sends fall back to aibot_send_msg.
        self._pending_req_ids: dict[str, str] = {}
        self._pending_lock = threading.Lock()

        # Inbound dedupe on req_id (improvement #1).
        self._seen = _SeenSet(
            max_size=SEEN_MESSAGES_MAX, evict=SEEN_MESSAGES_EVICT,
        )

        # Outbound send queue drained by the WS producer thread
        # (improvement #2). Each entry is a pre-encoded JSON frame
        # string ready for `ws.send_text`.
        self._send_queue: "queue.Queue[str]" = queue.Queue()

        # Shutdown signal — set by ``on_shutdown`` so the producer
        # thread (running inside ``loop.run_in_executor``) can break
        # out of its reconnect loop. Without this the executor thread
        # outlives the asyncio loop on a clean ``Shutdown`` command;
        # the runtime cancels the Future but Python threads can't be
        # cancelled from the outside.
        self._shutdown = threading.Event()

    # ---- dedupe shim --------------------------------------------------

    def _mark_seen(self, req_id: Optional[str]) -> bool:
        return self._seen.mark(req_id)

    # ---- send-frame routing ------------------------------------------

    def _enqueue_text(self, user_id: str, text: str) -> None:
        """Enqueue text chunks as outbound WS frames. Uses
        ``aibot_respond_msg`` once per cached ``req_id``, then falls
        back to ``aibot_send_msg`` for the remainder."""
        if not text:
            # ``split_message("", N)`` returns ``[""]`` not ``[]``;
            # gate up front so empty/whitespace-stripped sends don't
            # ship an empty markdown body to WeCom.
            return
        chunks = _split_message(text, WECOM_MAX_MESSAGE_LEN)
        with self._pending_lock:
            req_id = self._pending_req_ids.pop(user_id, None)
        first = chunks[0]
        if req_id:
            self._send_queue.put(_build_reply_frame(req_id, first))
        else:
            self._send_queue.put(_build_send_frame(user_id, first))
        for chunk in chunks[1:]:
            # WeCom rejects re-using the same req_id for a second
            # respond_msg, so all subsequent chunks go via the
            # proactive send path (mirrors wecom.rs:1487-1517).
            self._send_queue.put(_build_send_frame(user_id, chunk))

    # ---- WS test seam -------------------------------------------------

    def _make_ws(self, url: str) -> _WebSocketClient:
        return _WebSocketClient(url)

    # ---- WS session ---------------------------------------------------

    def _run_session(
        self, ws: _WebSocketClient, emit: Callable[[dict], None],
    ) -> None:
        """Drive one WS session: subscribe, then loop on heartbeat /
        send-queue / inbound until the connection drops."""
        try:
            ws.send_text(_build_subscribe_frame(self.bot_id, self.secret))
        except OSError as e:
            log.warn("wecom subscribe send failed", error=str(e))
            return
        log.info("wecom WS connected", bot_id=self.bot_id)

        next_heartbeat = time.monotonic() + HEARTBEAT_INTERVAL_SECS
        subscribed = False

        while not self._shutdown.is_set():
            now = time.monotonic()
            if now >= next_heartbeat:
                try:
                    ws.send_text(_build_ping_frame())
                except OSError as e:
                    log.warn("wecom heartbeat send failed", error=str(e))
                    return
                next_heartbeat = now + HEARTBEAT_INTERVAL_SECS

            # Drain any pending outbound frames (one per loop iteration
            # is enough — if more are queued they get the next tick).
            try:
                outbound = self._send_queue.get_nowait()
            except queue.Empty:
                outbound = None
            if outbound is not None:
                try:
                    ws.send_text(outbound)
                except OSError as e:
                    log.warn("wecom outbound send failed", error=str(e))
                    # Re-queue so the next reconnect attempts redelivery.
                    self._send_queue.put(outbound)
                    return

            wait_for = max(0.0, min(READ_TICK_SECS, next_heartbeat - now))
            if not ws.wait_readable(wait_for):
                continue
            try:
                text, close = ws.recv_frame()
            except (EOFError, OSError) as e:
                log.warn("wecom ws socket dropped", error=str(e))
                return
            if close is not None:
                code, reason = close
                log.info("wecom ws closed",
                         code=code,
                         reason=reason.decode("utf-8", "replace"))
                return
            if text is None:
                continue
            try:
                frame = json.loads(text)
            except (ValueError, TypeError):
                log.warn("wecom ws: unparseable frame")
                continue

            if _is_subscribe_success(frame):
                if not subscribed:
                    log.info("wecom subscribed")
                    subscribed = True
                continue

            cmd = _frame_cmd(frame)

            # Explicit subscribe failure: errcode != 0 with the
            # subscribe cmd. Log and let the outer loop reconnect.
            if cmd == "aibot_subscribe":
                log.error(
                    "wecom subscribe failed",
                    errcode=frame.get("errcode"),
                    errmsg=frame.get("errmsg"),
                )
                return

            if cmd == "aibot_event_callback":
                log.debug("wecom event_callback", body=_frame_body(frame))
                continue

            if cmd == "pong":
                continue

            # Server ack frames for send/respond — surface non-zero
            # errcodes (improvement #3).
            if cmd in (None, "aibot_respond_msg", "aibot_send_msg"):
                req_id = _frame_req_id(frame)
                errcode = frame.get("errcode")
                if req_id is not None and (errcode is None or errcode == 0):
                    log.debug("wecom server ack", req_id=req_id, cmd=cmd)
                    continue
                if errcode is not None and errcode != 0:
                    log.warn(
                        "wecom server ack error",
                        req_id=req_id,
                        cmd=cmd,
                        errcode=errcode,
                        errmsg=frame.get("errmsg"),
                    )
                    continue

            if cmd != "aibot_msg_callback":
                log.debug("wecom unhandled frame", cmd=cmd)
                continue

            event = parse_wecom_event(
                frame,
                allowed_users=self.allowed_users,
                account_id=self.account_id,
            )
            if event is None:
                continue

            req_id = event["params"].get("message_id")
            if isinstance(req_id, str) and not self._mark_seen(req_id):
                # Duplicate redelivery on reconnect / platform retry —
                # drop it. (improvement #1)
                continue

            from_user = event["params"]["user_id"]
            if isinstance(req_id, str) and req_id:
                with self._pending_lock:
                    self._pending_req_ids[from_user] = req_id
            emit(event)

    # ---- outer reconnect loop ----------------------------------------

    def _producer_blocking(self, emit: Callable[[dict], None]) -> None:
        backoff = INITIAL_BACKOFF_SECS
        while not self._shutdown.is_set():
            try:
                log.info("wecom ws connecting", url=self.ws_url)
                with self._make_ws(self.ws_url) as ws:
                    self._run_session(ws, emit)
                # Clean session end → reset backoff for next reconnect.
                backoff = INITIAL_BACKOFF_SECS
            except Exception as e:  # noqa: BLE001 — transport varies
                if self._shutdown.is_set():
                    return
                log.warn("wecom ws error; backing off",
                         error=str(e), delay=backoff)
                # Use Event.wait so shutdown interrupts the backoff
                # instead of blocking the executor thread for up to
                # WECOM_MAX_BACKOFF_SECS while the runtime tries to
                # exit cleanly.
                if self._shutdown.wait(backoff):
                    return
                backoff = min(backoff * 2.0, WECOM_MAX_BACKOFF_SECS)

    # ---- public sidecar surface --------------------------------------

    async def produce(self, emit: Callable[[dict], None]) -> None:
        loop = asyncio.get_event_loop()
        await loop.run_in_executor(None, self._producer_blocking, emit)

    async def on_shutdown(self) -> None:
        # Wake the producer thread out of its reconnect backoff /
        # session loop. Without this the runtime's task cancel hits
        # only the awaited Future; the executor's Python thread keeps
        # running until the next socket read tick.
        self._shutdown.set()

    async def on_send(self, cmd) -> None:
        # The inbound message_id (== wecom req_id) is the bridge's
        # channel_id round-trip handle. Fall back to user.platform_id
        # when the bridge didn't carry it (proactive sends from agent).
        user_id = (
            cmd.channel_id
            or (cmd.user.get("platform_id") if cmd.user else "")
            or ""
        )
        if not user_id:
            log.warn("wecom on_send: no user_id, dropping")
            return

        content = cmd.content
        text = cmd.text or ""
        if isinstance(content, dict) and "Text" in content:
            text = content["Text"]
        elif content and not (
            isinstance(content, dict) and "Text" in content
        ):
            # Non-text content: surface a placeholder so the operator
            # sees the failure mode (same shape as qq / line /
            # mattermost / signal).
            text = "(Unsupported content type)"

        if not text:
            return
        self._enqueue_text(user_id, text)


if __name__ == "__main__":
    run_stdio_main(WeComAdapter)
