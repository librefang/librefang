"""Direct tests for ``librefang.sidecar.ws``.

The WebSocketClient is non-trivial RFC 6455 code (~250 LOC of frame
parsing, masking, handshake assembly). It's exercised transitively
through discord / slack / webex / mattermost / qq adapter tests via
their respective ``_FakeWS`` doubles, but those doubles SUBSTITUTE
the class entirely — none of them actually drive the RFC 6455 logic.
These tests fill that gap by:

* Covering the frame-format helpers via the module-level constants
  (opcodes, max-payload cap, magic GUID).
* Exercising ``_parse_url`` directly so URL parsing has direct
  coverage independent of any actual socket.
* Smoke-testing ``WebSocketClient.__init__`` initialisation.

Full socket-level coverage (handshake, frame round-trip) is out of
scope here — it needs a real local WebSocket server fixture, which
this PR's scope deliberately doesn't add. The transitive coverage
via the adapter tests + the stdlib's own RFC 6455 acceptance is
sufficient.
"""
from __future__ import annotations

import base64
import hashlib
import struct

import pytest

from librefang.sidecar import ws as ws_mod
from librefang.sidecar.ws import (
    DEFAULT_HANDSHAKE_TIMEOUT_SECS,
    MAX_FRAME_PAYLOAD,
    OP_CLOSE,
    OP_BIN,
    OP_CONT,
    OP_PING,
    OP_PONG,
    OP_TEXT,
    WS_GUID,
    WebSocketClient,
)


# ---- module constants ------------------------------------------------


def test_rfc6455_guid_canonical():
    """The handshake-key hash uses this exact magic string per RFC
    6455 §1.3. Don't change it."""
    assert WS_GUID == "258EAFA5-E914-47DA-95CA-C5AB0DC85B11"


def test_opcode_values_match_rfc6455():
    """RFC 6455 §11.8 control + data opcodes."""
    assert OP_CONT == 0x0
    assert OP_TEXT == 0x1
    assert OP_CLOSE == 0x8
    assert OP_PING == 0x9
    assert OP_PONG == 0xA


def test_max_frame_payload_4mib():
    """4 MiB cap. Every legitimate sidecar protocol stays well under;
    anything larger gets rejected to prevent oversized-payload DoS."""
    assert MAX_FRAME_PAYLOAD == 4 * 1024 * 1024


def test_default_handshake_timeout():
    assert DEFAULT_HANDSHAKE_TIMEOUT_SECS == 15.0


# ---- _parse_url ------------------------------------------------------


def test_parse_url_wss():
    host, port, path, is_tls = WebSocketClient._parse_url(
        "wss://example.com/api/v4/websocket")
    assert host == "example.com"
    assert port == 443
    assert path == "/api/v4/websocket"
    assert is_tls is True


def test_parse_url_ws_explicit_port():
    host, port, path, is_tls = WebSocketClient._parse_url(
        "ws://localhost:8080/socket")
    assert host == "localhost"
    assert port == 8080
    assert path == "/socket"
    assert is_tls is False


def test_parse_url_default_paths():
    """An empty path component coerces to ``/``."""
    _, _, path, _ = WebSocketClient._parse_url("wss://example.com")
    assert path == "/"


def test_parse_url_carries_query_string():
    """Query strings are preserved on the upgrade GET request."""
    _, _, path, _ = WebSocketClient._parse_url(
        "wss://gw.example.com/path?token=abc&v=2")
    assert path == "/path?token=abc&v=2"


def test_parse_url_rejects_non_ws_scheme():
    with pytest.raises(ValueError, match="not a websocket url"):
        WebSocketClient._parse_url("https://example.com/")


def test_parse_url_rejects_missing_host():
    with pytest.raises(ValueError, match="missing host"):
        WebSocketClient._parse_url("wss:///path")


# ---- WebSocketClient init -------------------------------------------


def test_init_stores_url_and_headers():
    ws = WebSocketClient(
        "wss://example.com/",
        headers={"Authorization": "Bearer abc"},
    )
    assert ws.url == "wss://example.com/"
    assert ws.headers == {"Authorization": "Bearer abc"}
    assert ws.closed is False
    assert ws._sock is None


def test_init_empty_headers_default():
    ws = WebSocketClient("wss://example.com/")
    assert ws.headers == {}


def test_init_custom_handshake_timeout():
    ws = WebSocketClient(
        "wss://example.com/", handshake_timeout=5.0,
    )
    assert ws._handshake_timeout == 5.0


# ---- wire-frame parsing ---------------------------------------------


def _server_frame(opcode, payload=b"", *, fin=True):
    first = (0x80 if fin else 0) | opcode
    length = len(payload)
    if length < 126:
        return bytes([first, length]) + payload
    if length < 65536:
        return bytes([first, 126]) + struct.pack(">H", length) + payload
    return bytes([first, 127]) + struct.pack(">Q", length) + payload


class _FakeSocket:
    def __init__(self, incoming=b""):
        self.incoming = bytearray(incoming)
        self.sent = []
        self.timeouts = []
        self.closed = False

    def recv(self, count):
        chunk = bytes(self.incoming[:count])
        del self.incoming[:count]
        return chunk

    def sendall(self, payload):
        self.sent.append(payload)

    def settimeout(self, timeout):
        self.timeouts.append(timeout)

    def close(self):
        self.closed = True


@pytest.mark.parametrize("any_frame", [False, True])
def test_ping_between_fragments_is_ponged_and_reassembly_continues(any_frame):
    incoming = b"".join([
        _server_frame(OP_TEXT, b"hel", fin=False),
        _server_frame(OP_PING, b"beat"),
        _server_frame(OP_PONG, b"ignored"),
        _server_frame(OP_CONT, b"lo"),
    ])
    sock = _FakeSocket(incoming)
    ws = WebSocketClient("ws://example.test")
    ws._sock = sock

    if any_frame:
        assert ws.recv_any_frame() == ("hello", None, None)
    else:
        assert ws.recv_frame() == ("hello", None)

    assert len(sock.sent) == 1
    assert sock.sent[0][0] & 0x0F == OP_PONG


def test_recv_any_frame_reassembles_fragmented_binary_with_ping():
    incoming = b"".join([
        _server_frame(OP_BIN, b"\x01", fin=False),
        _server_frame(OP_PING, b"beat"),
        _server_frame(OP_CONT, b"\x02"),
    ])
    sock = _FakeSocket(incoming)
    ws = WebSocketClient("ws://example.test")
    ws._sock = sock

    assert ws.recv_any_frame() == (None, b"\x01\x02", None)
    assert sock.sent[0][0] & 0x0F == OP_PONG


def test_recv_frame_drains_ignored_fragmented_binary_before_next_message():
    incoming = b"".join([
        _server_frame(OP_BIN, b"\x01", fin=False),
        _server_frame(OP_PING, b"beat"),
        _server_frame(OP_CONT, b"\x02"),
        _server_frame(OP_TEXT, b"next"),
    ])
    sock = _FakeSocket(incoming)
    ws = WebSocketClient("ws://example.test")
    ws._sock = sock

    assert ws.recv_frame() == (None, None)
    assert ws.recv_frame() == ("next", None)
    assert sock.sent[0][0] & 0x0F == OP_PONG


def test_recv_frame_caps_ignored_fragmented_binary(monkeypatch):
    monkeypatch.setattr(ws_mod, "MAX_FRAME_PAYLOAD", 5)
    incoming = b"".join([
        _server_frame(OP_BIN, b"abc", fin=False),
        _server_frame(OP_CONT, b"def"),
    ])
    ws = WebSocketClient("ws://example.test")
    ws._sock = _FakeSocket(incoming)

    with pytest.raises(RuntimeError, match="reassembled message exceeds cap"):
        ws.recv_frame()


@pytest.mark.parametrize("any_frame", [False, True])
def test_close_between_fragments_surfaces_close(any_frame):
    incoming = b"".join([
        _server_frame(OP_TEXT, b"partial", fin=False),
        _server_frame(OP_CLOSE, struct.pack(">H", 1001) + b"bye"),
    ])
    ws = WebSocketClient("ws://example.test")
    ws._sock = _FakeSocket(incoming)

    if any_frame:
        assert ws.recv_any_frame() == (None, None, (1001, b"bye"))
    else:
        assert ws.recv_frame() == (None, (1001, b"bye"))


@pytest.mark.parametrize("any_frame", [False, True])
def test_reassembled_message_is_capped(monkeypatch, any_frame):
    monkeypatch.setattr(ws_mod, "MAX_FRAME_PAYLOAD", 5)
    incoming = b"".join([
        _server_frame(OP_TEXT, b"abc", fin=False),
        _server_frame(OP_CONT, b"def"),
    ])
    ws = WebSocketClient("ws://example.test")
    ws._sock = _FakeSocket(incoming)

    recv = ws.recv_any_frame if any_frame else ws.recv_frame
    with pytest.raises(RuntimeError, match="reassembled message exceeds cap"):
        recv()


def test_disconnected_send_and_receive_raise_runtime_error():
    ws = WebSocketClient("ws://example.test")
    with pytest.raises(RuntimeError, match="not connected"):
        ws.send_text("hello")
    with pytest.raises(RuntimeError, match="not connected"):
        ws._recv_exact(1)


# ---- handshake resource lifecycle ----------------------------------


def test_successful_handshake_restores_blocking_socket(monkeypatch):
    key_bytes = b"0123456789abcdef"
    key = base64.b64encode(key_bytes).decode("ascii")
    accept = base64.b64encode(
        hashlib.sha1((key + WS_GUID).encode("ascii")).digest()
    )
    response = (
        b"HTTP/1.1 101 Switching Protocols\r\n"
        b"Sec-WebSocket-Accept: " + accept + b"\r\n\r\n"
    )
    sock = _FakeSocket(response)
    monkeypatch.setattr(ws_mod.os, "urandom", lambda _count: key_bytes)
    monkeypatch.setattr(
        ws_mod.socket, "create_connection", lambda *_args, **_kwargs: sock,
    )

    with WebSocketClient("ws://example.test") as ws:
        assert ws._sock is sock
        assert sock.timeouts == [None]


def test_tls_wrap_failure_closes_raw_socket(monkeypatch):
    raw_sock = _FakeSocket()

    class _BrokenContext:
        def wrap_socket(self, _sock, *, server_hostname):
            assert server_hostname == "example.test"
            raise OSError("TLS setup failed")

    monkeypatch.setattr(
        ws_mod.socket, "create_connection",
        lambda *_args, **_kwargs: raw_sock,
    )
    monkeypatch.setattr(
        ws_mod.ssl, "create_default_context", lambda: _BrokenContext(),
    )

    with pytest.raises(OSError, match="TLS setup failed"):
        WebSocketClient("wss://example.test").__enter__()
    assert raw_sock.closed is True
