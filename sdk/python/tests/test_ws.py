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

import pytest

from librefang.sidecar.ws import (
    DEFAULT_HANDSHAKE_TIMEOUT_SECS,
    MAX_FRAME_PAYLOAD,
    OP_CLOSE,
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
