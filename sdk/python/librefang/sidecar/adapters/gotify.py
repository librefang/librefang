#!/usr/bin/env python3
"""Gotify sidecar channel adapter for LibreFang.

Replaces the former in-process Rust ``librefang-channels::gotify``
adapter (removed in this sidecar migration; same pattern as ntfy
#5224 and telegram #5241). Behaviour is preserved:

* Inbound: subscribe to ``{server}/stream?token={client_token}`` via
  WebSocket. Each text frame is JSON ``{id, message, title, priority,
  appid}`` — empty ``message`` is skipped, a leading ``/`` makes it a
  Command, ``title`` becomes the sender display name (fallback
  ``app-{appid}``).
* Outbound: POST JSON to ``{server}/message?token={app_token}`` with
  body ``{title:"LibreFang", message, priority:5}``. Chunked at 65535
  chars, suffixing ``(i/N)`` to the title for multi-chunk sends.
* Reconnect with exponential backoff (1s → 60s).

Stdlib-only (the SDK has zero runtime deps): the WebSocket client is
a minimal RFC 6455 reader hand-rolled on ``socket`` + ``ssl``.
Gotify only sends server→client text frames, but the client still
responds to server pings with a (masked) pong as required by the
spec, and replies to a server close-frame with a close before
disconnecting.

Configure via ``[[sidecar_channels]]``:

    [[sidecar_channels]]
    name = "gotify"
    command = "python3"
    args = ["-m", "librefang.sidecar.adapters.gotify"]
    channel_type = "gotify"
    [sidecar_channels.env]
    GOTIFY_SERVER_URL = "https://gotify.example.com"
    GOTIFY_APP_TOKEN = "A..."              # from /application page
    GOTIFY_CLIENT_TOKEN = "C..."           # from /client page
    # GOTIFY_ACCOUNT_ID = "prod"           # optional multi-bot key
"""
from __future__ import annotations

import asyncio
import base64
import hashlib
import json
import os
import socket
import ssl
import struct
import time
import urllib.error
import urllib.parse
import urllib.request

from librefang.sidecar import Content, Field, Schema, SidecarAdapter, protocol, run_stdio_main
from librefang.sidecar import logging as log
from librefang.sidecar.common import (
    RETRY_AFTER_DEFAULT_SECS,
    SeenSet as _SeenSet,
    parse_retry_after as _parse_retry_after,
    split_message as _split_message,
)

# Gotify caps individual messages at this length (matches the Rust adapter).
MAX_MESSAGE_LEN = 65535
# Maximum bytes accepted for a single inbound WebSocket frame payload.
# RFC 6455 allows up to 2^63 — well beyond anything Gotify legitimately
# sends (its frames are short JSON). Capping protects against a hostile
# server that announces a 64-bit length to make us spin reading a
# multi-exabyte payload. 1 MiB leaves comfortable headroom over the
# largest realistic Gotify payload (a 65 535-char message body wrapped
# in a JSON envelope is well under 100 KiB).
MAX_FRAME_PAYLOAD = 1 << 20  # 1 MiB
SEND_TIMEOUT_SECS = 10
HANDSHAKE_TIMEOUT_SECS = 15.0
# RFC 6455 magic GUID used to derive Sec-WebSocket-Accept.
_WS_GUID = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11"
# WebSocket opcodes.
_OP_CONT = 0x0
_OP_TEXT = 0x1
_OP_BIN = 0x2
_OP_CLOSE = 0x8
_OP_PING = 0x9
_OP_PONG = 0xA

class _WebSocketReader:
    """Minimal RFC 6455 client. Iterating yields each completed text
    message as ``str`` (continuation frames are reassembled). Handles
    server pings (replies with pong), close frames (echoes close and
    exits), and binary frames (ignored — gotify is JSON text). Use as
    a context manager so the underlying socket is always closed.

    Server→client frames are never masked; client→server frames (only
    pong / close in this adapter) are masked with a fresh 4-byte key
    per frame as the spec requires.
    """

    def __init__(self, url: str, headers: dict | None = None,
                 handshake_timeout: float = HANDSHAKE_TIMEOUT_SECS) -> None:
        self.url = url
        self.headers = dict(headers or {})
        self._sock: socket.socket | None = None
        self._leftover = b""
        self._handshake_timeout = handshake_timeout

    @staticmethod
    def _parse_url(url: str) -> tuple[str, int, str, bool]:
        u = urllib.parse.urlparse(url)
        scheme = u.scheme.lower()
        if scheme not in ("ws", "wss"):
            raise ValueError(f"not a websocket url: {url!r}")
        if not u.hostname:
            raise ValueError(f"websocket url missing host: {url!r}")
        is_tls = scheme == "wss"
        port = u.port or (443 if is_tls else 80)
        path = u.path or "/"
        if u.query:
            path += "?" + u.query
        return u.hostname, port, path, is_tls

    def __enter__(self) -> "_WebSocketReader":
        host, port, path, is_tls = self._parse_url(self.url)
        sock = socket.create_connection((host, port),
                                         timeout=self._handshake_timeout)
        if is_tls:
            ctx = ssl.create_default_context()
            sock = ctx.wrap_socket(sock, server_hostname=host)
        key = base64.b64encode(os.urandom(16)).decode("ascii")
        lines = [
            f"GET {path} HTTP/1.1",
            f"Host: {host}:{port}",
            "Upgrade: websocket",
            "Connection: Upgrade",
            f"Sec-WebSocket-Key: {key}",
            "Sec-WebSocket-Version: 13",
        ]
        for k, v in self.headers.items():
            lines.append(f"{k}: {v}")
        req = ("\r\n".join(lines) + "\r\n\r\n").encode("ascii")
        sock.sendall(req)
        # Read response head; tolerate the server piggy-backing the
        # first WS frame on the tail of the same TCP segment.
        buf = b""
        while b"\r\n\r\n" not in buf:
            chunk = sock.recv(4096)
            if not chunk:
                sock.close()
                raise RuntimeError("connection closed during handshake")
            buf += chunk
            if len(buf) > 65536:
                sock.close()
                raise RuntimeError("handshake response too large")
        head, _, leftover = buf.partition(b"\r\n\r\n")
        head_lines = head.split(b"\r\n")
        status = head_lines[0]
        if not status.startswith(b"HTTP/1.1 101 "):
            sock.close()
            raise RuntimeError(
                f"handshake failed: {status.decode('ascii', 'replace')}"
            )
        expected = base64.b64encode(
            hashlib.sha1((key + _WS_GUID).encode("ascii")).digest()
        ).decode("ascii")
        got = None
        for line in head_lines[1:]:
            name, _, val = line.partition(b":")
            if name.strip().lower() == b"sec-websocket-accept":
                got = val.strip().decode("ascii", "replace")
                break
        if got != expected:
            sock.close()
            raise RuntimeError("handshake Sec-WebSocket-Accept mismatch")
        # No read deadline in steady state — this is a long-lived stream.
        sock.settimeout(None)
        self._sock = sock
        self._leftover = leftover
        return self

    def __exit__(self, *_exc) -> None:
        if self._sock is not None:
            try:
                self._sock.close()
            except OSError:
                pass
            self._sock = None

    def _recv_exact(self, n: int) -> bytes:
        if n <= 0:
            return b""
        buf = bytearray()
        while len(buf) < n:
            if self._leftover:
                take = min(n - len(buf), len(self._leftover))
                buf.extend(self._leftover[:take])
                self._leftover = self._leftover[take:]
                continue
            assert self._sock is not None
            chunk = self._sock.recv(n - len(buf))
            if not chunk:
                raise EOFError("websocket closed mid-frame")
            buf.extend(chunk)
        return bytes(buf)

    def _send_frame(self, opcode: int, payload: bytes) -> None:
        assert self._sock is not None
        header = bytearray([0x80 | (opcode & 0x0F)])
        ln = len(payload)
        if ln < 126:
            header.append(0x80 | ln)
        elif ln < 65536:
            header.append(0x80 | 126)
            header.extend(struct.pack(">H", ln))
        else:
            header.append(0x80 | 127)
            header.extend(struct.pack(">Q", ln))
        mask = os.urandom(4)
        header.extend(mask)
        masked = bytes(b ^ mask[i % 4] for i, b in enumerate(payload))
        self._sock.sendall(bytes(header) + masked)

    def __iter__(self):
        message_buf = bytearray()
        message_op: int | None = None
        while True:
            h2 = self._recv_exact(2)
            fin = (h2[0] & 0x80) != 0
            opcode = h2[0] & 0x0F
            masked = (h2[1] & 0x80) != 0
            ln = h2[1] & 0x7F
            if ln == 126:
                ln = struct.unpack(">H", self._recv_exact(2))[0]
            elif ln == 127:
                ln = struct.unpack(">Q", self._recv_exact(8))[0]
            if ln > MAX_FRAME_PAYLOAD:
                raise RuntimeError(
                    f"websocket frame payload {ln} exceeds cap "
                    f"{MAX_FRAME_PAYLOAD}; failing the stream"
                )
            mask_key = self._recv_exact(4) if masked else None
            payload = self._recv_exact(ln)
            if mask_key is not None:
                payload = bytes(
                    b ^ mask_key[i % 4] for i, b in enumerate(payload)
                )
            if opcode == _OP_PING:
                # Echo as pong (must be masked since we're the client).
                self._send_frame(_OP_PONG, payload)
                continue
            if opcode == _OP_PONG:
                continue
            if opcode == _OP_CLOSE:
                # RFC 6455 §5.5.1: reply with close before tearing down.
                try:
                    self._send_frame(_OP_CLOSE, b"")
                except OSError:
                    pass
                return
            if opcode in (_OP_TEXT, _OP_BIN):
                message_op = opcode
                message_buf = bytearray(payload)
            elif opcode == _OP_CONT:
                message_buf.extend(payload)
            else:
                # Unknown / reserved opcode — drop to avoid breaking the stream.
                continue
            if fin and message_op is not None:
                if message_op == _OP_TEXT:
                    yield bytes(message_buf).decode("utf-8", "replace")
                # binary frames ignored (gotify is text/JSON only)
                message_buf = bytearray()
                message_op = None


class GotifyAdapter(SidecarAdapter):
    # gotify has no typing / reaction / interactive / streaming concept;
    # declare nothing so LibreFang only routes plain text.
    capabilities: list = []

    SCHEMA = Schema(
        name="gotify",
        display_name="Gotify",
        description="Gotify push notifications (out-of-process sidecar)",
        fields=[
            Field("GOTIFY_SERVER_URL", "Server URL", "text",
                  required=True,
                  placeholder="https://gotify.example.com"),
            Field("GOTIFY_APP_TOKEN", "App Token (publish)", "secret",
                  required=True, placeholder="A..."),
            Field("GOTIFY_CLIENT_TOKEN", "Client Token (subscribe)", "secret",
                  required=True, placeholder="C..."),
            Field("GOTIFY_ACCOUNT_ID", "Account ID (multi-bot routing)",
                  "text", placeholder="prod", advanced=True),
        ],
    )

    def __init__(self) -> None:
        raw_server = os.environ.get("GOTIFY_SERVER_URL", "").strip()
        self.server_url = raw_server.rstrip("/")
        self.app_token = os.environ.get("GOTIFY_APP_TOKEN", "").strip()
        self.client_token = os.environ.get("GOTIFY_CLIENT_TOKEN", "").strip()
        acct = os.environ.get("GOTIFY_ACCOUNT_ID", "").strip()
        # Surfaced to LibreFang via the `ready` event for multi-bot routing.
        self.account_id = acct or None
        missing = []
        if not self.server_url:
            missing.append("GOTIFY_SERVER_URL")
        if not self.app_token:
            missing.append("GOTIFY_APP_TOKEN")
        if not self.client_token:
            missing.append("GOTIFY_CLIENT_TOKEN")
        if missing:
            log.error("gotify required env vars missing", missing=missing)
            raise SystemExit(2)
        if not (self.server_url.startswith("http://")
                or self.server_url.startswith("https://")):
            log.error(
                "GOTIFY_SERVER_URL must start with http:// or https://",
                server_url=self.server_url,
            )
            raise SystemExit(2)
        # Bounded dedupe keyed on the gotify message `id` (monotonic
        # per-server int). The WS stream can replay buffered messages
        # on reconnect — without dedupe an unstable network produces
        # duplicate agent triggers.
        self._seen = _SeenSet()

    # ---- inbound: WebSocket subscription -----------------------------

    def _build_ws_url(self) -> str:
        base = self.server_url
        if base.startswith("https://"):
            base = "wss://" + base[len("https://"):]
        else:  # http:// — already validated in __init__
            base = "ws://" + base[len("http://"):]
        return f"{base}/stream?token={self.client_token}"

    @staticmethod
    def _parse_frame(text: str):
        """Gotify WS JSON → (id, message, title, priority, app_id) or None.
        Mirrors the Rust ``parse_ws_message`` shape; an empty
        ``message`` skips the frame entirely (matches the Rust check)."""
        try:
            val = json.loads(text)
        except (ValueError, TypeError):
            return None
        if not isinstance(val, dict):
            return None
        mid = val.get("id")
        msg = val.get("message")
        if not isinstance(mid, int) or not isinstance(msg, str) or not msg:
            return None
        title = val.get("title") or ""
        priority = val.get("priority") or 0
        app_id = val.get("appid") or 0
        try:
            return (
                int(mid),
                msg,
                str(title),
                int(priority),
                int(app_id),
            )
        except (TypeError, ValueError):
            return None

    def _to_event(self, mid: int, message: str, title: str,
                  priority: int, app_id: int) -> dict:
        sender_id = f"app-{app_id}"
        display = title if title else sender_id
        if message.startswith("/"):
            head, _, tail = message[1:].partition(" ")
            content = Content.command(head, tail.split() if tail else [])
        else:
            content = Content.text(message)
        return protocol.message(
            user_id=sender_id,
            user_name=display,
            content=content,
            message_id=f"gotify-{mid}",
            is_group=False,
            metadata={
                "title": title,
                "priority": priority,
                "app_id": app_id,
            },
        )

    def _ws_loop(self, emit) -> None:
        """One subscribe pass; caller wraps in reconnect backoff."""
        with _WebSocketReader(self._build_ws_url()) as ws:
            log.info("gotify WS connected", server=self.server_url)
            for text in ws:
                parsed = self._parse_frame(text)
                if parsed is None:
                    continue
                mid = parsed[0]
                if not self._seen.mark(f"gotify-{mid}"):
                    continue
                emit(self._to_event(*parsed))

    async def produce(self, emit) -> None:
        loop = asyncio.get_event_loop()
        backoff = 1.0
        while True:
            try:
                await loop.run_in_executor(None, self._ws_loop, emit)
                # Clean stream end → reconnect promptly.
                backoff = 1.0
            except asyncio.CancelledError:
                raise
            except Exception as e:  # noqa: BLE001 — transport errors vary
                log.warn(
                    "gotify WS error; backing off",
                    error=str(e),
                    delay=backoff,
                )
                await asyncio.sleep(backoff)
                backoff = min(backoff * 2, 60.0)

    # ---- outbound: REST publish --------------------------------------

    def _publish(self, text: str) -> None:
        url = f"{self.server_url}/message?token={self.app_token}"
        chunks = _split_message(text, MAX_MESSAGE_LEN)
        n = len(chunks)
        for i, chunk in enumerate(chunks):
            title = "LibreFang" if n == 1 else f"LibreFang ({i + 1}/{n})"
            self._publish_chunk(url, title, chunk, retry_429=True)

    def _publish_chunk(self, url: str, title: str, chunk: str,
                       *, retry_429: bool) -> None:
        body = json.dumps({
            "title": title,
            "message": chunk,
            "priority": 5,
        }).encode("utf-8")
        req = urllib.request.Request(
            url,
            data=body,
            headers={"Content-Type": "application/json"},
            method="POST",
        )
        try:
            with urllib.request.urlopen(  # noqa: S310 — configured URL
                req, timeout=SEND_TIMEOUT_SECS,
            ) as resp:
                status = getattr(resp, "status", 200)
                if status >= 300:
                    raise RuntimeError(f"gotify publish HTTP {status}")
        except urllib.error.HTTPError as e:
            if e.code == 429 and retry_429:
                # Server-side rate limit. Honour Retry-After then retry
                # once. A second 429 raises so the caller surfaces the
                # backpressure rather than silently dropping the chunk.
                resp_hdrs = {
                    k.lower(): v for k, v in (e.headers or {}).items()
                }
                wait = _parse_retry_after(
                    resp_hdrs, default_secs=RETRY_AFTER_DEFAULT_SECS,
                )
                log.warn(
                    "gotify 429 on publish; sleeping then retrying once",
                    retry_after_secs=wait,
                )
                time.sleep(wait)
                self._publish_chunk(url, title, chunk, retry_429=False)
                return
            err_body = e.read().decode("utf-8", "replace")
            raise RuntimeError(
                f"gotify publish {e.code}: {err_body}"
            ) from e

    async def on_send(self, cmd) -> None:
        # Plain-text only, like the Rust adapter; structured content the
        # platform can't render falls back to the placeholder string.
        if cmd.content and not (
            isinstance(cmd.content, dict) and "Text" in cmd.content
        ):
            text = "(Unsupported content type)"
        else:
            text = cmd.text or ""
        await asyncio.get_event_loop().run_in_executor(
            None, self._publish, text,
        )


if __name__ == "__main__":
    run_stdio_main(GotifyAdapter)
