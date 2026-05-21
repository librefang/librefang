"""Stdlib RFC 6455 WebSocket client for ``librefang.sidecar.adapters.*``.

The sidecar SDK ships zero third-party deps by policy. Every WS-using
sidecar (discord, slack, webex, mattermost, qq) previously inlined a
near-identical copy of this client — 5 copies, 4 byte-identical and
1 (discord) with the same code plus extra comments. This module is
the single source of truth.

Usage::

    from librefang.sidecar.ws import (
        WebSocketClient,
        OP_TEXT, OP_PING, OP_PONG, OP_CLOSE,
        MAX_FRAME_PAYLOAD,
        DEFAULT_HANDSHAKE_TIMEOUT_SECS,
    )

    with WebSocketClient("wss://gateway.example.com/ws",
                        headers={"Authorization": "Bearer ..."}) as ws:
        ws.send_text(json.dumps({"op": 2, "d": {...}}))
        while True:
            if not ws.wait_readable(30.0):
                continue
            text, close = ws.recv_frame()
            if close is not None:
                code, reason = close
                break
            if text is not None:
                handle(text)

Behaviour:

* TLS via ``ssl.create_default_context()`` when scheme is ``wss``.
* Client→server frames are masked per RFC 6455 with a fresh 4-byte
  key each send.
* ``recv_frame`` answers pings inline (sends a pong) and returns
  ``(None, None)`` for non-text frames the caller doesn't need to
  see (binary, pong). Close frames surface as
  ``(None, (close_code, reason_bytes))``.
* Text continuation frames are reassembled internally; the caller
  always gets a complete message.
* ``wait_readable`` correctly handles TLS-buffered bytes that
  ``select`` can't see (via ``ssl.SSLSocket.pending()``).
"""
from __future__ import annotations

import base64
import hashlib
import os
import select
import socket
import ssl
import struct
import threading
import urllib.parse
from typing import Optional

# RFC 6455 magic GUID for the handshake accept-key hash.
WS_GUID = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11"

# Opcode constants. Re-export under both bare and ``OP_``-prefixed
# names so callers can use either style.
OP_CONT = 0x0
OP_TEXT = 0x1
OP_BIN = 0x2
OP_CLOSE = 0x8
OP_PING = 0x9
OP_PONG = 0xA

# Cap on a single inbound payload (text or continuation). 4 MiB.
# Any frame larger than this raises ``RuntimeError`` rather than
# silently truncating — every legitimate sidecar protocol stays
# well under this.
MAX_FRAME_PAYLOAD = 1 << 22

DEFAULT_HANDSHAKE_TIMEOUT_SECS = 15.0


class WebSocketClient:
    """Minimal RFC 6455 client (text-only). Use as a context manager.

    Server→client frames are never masked; client→server frames MUST
    be masked with a fresh 4-byte key per frame.
    """

    def __init__(
        self,
        url: str,
        *,
        headers: Optional[dict] = None,
        handshake_timeout: float = DEFAULT_HANDSHAKE_TIMEOUT_SECS,
    ) -> None:
        self.url = url
        self.headers = dict(headers or {})
        self._sock: Optional[socket.socket] = None
        self._leftover = b""
        self._handshake_timeout = handshake_timeout
        self._send_lock = threading.Lock()
        self.closed = False

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

    def __enter__(self) -> "WebSocketClient":
        host, port, path, is_tls = self._parse_url(self.url)
        sock = socket.create_connection(
            (host, port), timeout=self._handshake_timeout,
        )
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
        buf = b""
        while b"\r\n\r\n" not in buf:
            chunk = sock.recv(4096)
            if not chunk:
                sock.close()
                raise RuntimeError("connection closed during ws handshake")
            buf += chunk
            if len(buf) > 65536:
                sock.close()
                raise RuntimeError("ws handshake response too large")
        head, _, leftover = buf.partition(b"\r\n\r\n")
        head_lines = head.split(b"\r\n")
        status = head_lines[0]
        if not status.startswith(b"HTTP/1.1 101 "):
            sock.close()
            raise RuntimeError(
                f"ws handshake failed: {status.decode('ascii', 'replace')}"
            )
        expected = base64.b64encode(
            hashlib.sha1((key + WS_GUID).encode("ascii")).digest()
        ).decode("ascii")
        got = None
        for line in head_lines[1:]:
            name, _, val = line.partition(b":")
            if name.strip().lower() == b"sec-websocket-accept":
                got = val.strip().decode("ascii", "replace")
                break
        if got != expected:
            sock.close()
            raise RuntimeError("ws handshake Sec-WebSocket-Accept mismatch")
        self._sock = sock
        self._leftover = leftover
        return self

    def __exit__(self, *_exc) -> None:
        self.closed = True
        if self._sock is not None:
            try:
                self._sock.close()
            except OSError:
                pass
            self._sock = None

    def settimeout(self, timeout: Optional[float]) -> None:
        if self._sock is not None:
            self._sock.settimeout(timeout)

    def wait_readable(self, timeout: float) -> bool:
        """Return True when bytes are ready (or already buffered),
        False on a clean timeout. Used to gate the heartbeat tick
        BEFORE we start consuming a frame, so a mid-frame stall
        becomes a hard transport error instead of corrupting state.

        TLS-on-Python is the awkward bit: ``ssl.SSLSocket.pending()``
        exposes already-decrypted bytes that aren't visible to
        ``select`` (TLS records can come back from the OS but stay
        in the SSL layer's read buffer). We check leftover-from-
        handshake first, then ``ssl.pending()``, then ``select``.
        """
        if self._leftover:
            return True
        sock = self._sock
        if sock is None:
            return False
        # SSLSocket exposes already-decrypted bytes via `.pending()` —
        # those don't show up in select() because the OS has already
        # given them to the SSL layer.
        pending = getattr(sock, "pending", None)
        if callable(pending):
            try:
                if pending() > 0:
                    return True
            except Exception:  # noqa: BLE001 — closed socket etc.
                pass
        try:
            r, _, _ = select.select([sock], [], [], max(0.0, timeout))
        except (OSError, ValueError):
            # ``select`` rejects closed/negative-fd sockets — treat as
            # "no data, will fail on next recv".
            return False
        return bool(r)

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
        with self._send_lock:
            self._sock.sendall(bytes(header) + masked)

    def send_text(self, s: str) -> None:
        self._send_frame(OP_TEXT, s.encode("utf-8"))

    def send_close(self) -> None:
        try:
            self._send_frame(OP_CLOSE, b"")
        except OSError:
            pass

    def recv_frame(self) -> tuple[Optional[str], Optional[tuple[int, bytes]]]:
        """Read one frame and return either ``(text, None)`` for a
        completed text message, or ``(None, (close_code, reason))``
        for a close frame the server sent. Pings are answered inline
        and skipped. Returns ``(None, None)`` for non-text frames we
        ignore (binary, pong).
        """
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
        if opcode == OP_PING:
            self._send_frame(OP_PONG, payload)
            return None, None
        if opcode == OP_PONG:
            return None, None
        if opcode == OP_CLOSE:
            code = 1005  # "no status received" if payload < 2 bytes
            reason = b""
            if len(payload) >= 2:
                code = struct.unpack(">H", payload[:2])[0]
                reason = payload[2:]
            return None, (code, reason)
        if opcode == OP_TEXT:
            buf = bytearray(payload)
            while not fin:
                h2 = self._recv_exact(2)
                fin = (h2[0] & 0x80) != 0
                opcode2 = h2[0] & 0x0F
                masked2 = (h2[1] & 0x80) != 0
                ln2 = h2[1] & 0x7F
                if ln2 == 126:
                    ln2 = struct.unpack(">H", self._recv_exact(2))[0]
                elif ln2 == 127:
                    ln2 = struct.unpack(">Q", self._recv_exact(8))[0]
                if ln2 > MAX_FRAME_PAYLOAD:
                    raise RuntimeError("ws continuation payload too large")
                mk = self._recv_exact(4) if masked2 else None
                payload2 = self._recv_exact(ln2)
                if mk is not None:
                    payload2 = bytes(
                        b ^ mk[i % 4] for i, b in enumerate(payload2)
                    )
                if opcode2 != OP_CONT:
                    # Unexpected interleaved frame — bail.
                    raise RuntimeError(
                        f"ws unexpected interleaved opcode {opcode2}"
                    )
                buf.extend(payload2)
            return buf.decode("utf-8", "replace"), None
        # Binary / unknown — ignore.
        return None, None

    def recv_any_frame(
        self,
    ) -> tuple[Optional[str], Optional[bytes], Optional[tuple[int, bytes]]]:
        """Like :meth:`recv_frame` but also surfaces binary frames.

        Returns ``(text, binary, close)`` — exactly one is non-None
        per call, or all-None for opcodes we still skip (pong).
        Adapters that need to see protobuf-wrapped binary payloads
        (Feishu gateway, etc.) use this; everyone else uses
        ``recv_frame`` which keeps the simpler 2-tuple shape.
        """
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
        if opcode == OP_PING:
            self._send_frame(OP_PONG, payload)
            return None, None, None
        if opcode == OP_PONG:
            return None, None, None
        if opcode == OP_CLOSE:
            code = 1005
            reason = b""
            if len(payload) >= 2:
                code = struct.unpack(">H", payload[:2])[0]
                reason = payload[2:]
            return None, None, (code, reason)
        if opcode in (OP_TEXT, OP_BIN):
            buf = bytearray(payload)
            while not fin:
                h2 = self._recv_exact(2)
                fin = (h2[0] & 0x80) != 0
                opcode2 = h2[0] & 0x0F
                masked2 = (h2[1] & 0x80) != 0
                ln2 = h2[1] & 0x7F
                if ln2 == 126:
                    ln2 = struct.unpack(">H", self._recv_exact(2))[0]
                elif ln2 == 127:
                    ln2 = struct.unpack(">Q", self._recv_exact(8))[0]
                if ln2 > MAX_FRAME_PAYLOAD:
                    raise RuntimeError("ws continuation payload too large")
                mk = self._recv_exact(4) if masked2 else None
                payload2 = self._recv_exact(ln2)
                if mk is not None:
                    payload2 = bytes(
                        b ^ mk[i % 4] for i, b in enumerate(payload2)
                    )
                if opcode2 != OP_CONT:
                    raise RuntimeError(
                        f"ws unexpected interleaved opcode {opcode2}"
                    )
                buf.extend(payload2)
            if opcode == OP_TEXT:
                return buf.decode("utf-8", "replace"), None, None
            return None, bytes(buf), None
        # Unknown opcode — skip.
        return None, None, None
