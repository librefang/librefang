"""Tests for librefang.sidecar.adapters.gotify.

Deterministic, no network: urllib is monkeypatched, the WebSocket
client is exercised via a loopback TCP server. Asserts the sidecar
gotify adapter preserves the behaviour of the removed in-process
Rust `librefang-channels::gotify` adapter.
"""

import io
import json
import os
import socket
import threading
import time

import pytest

# Required env must be present at import time because the adapter
# raises SystemExit(2) if unset on construction. Tests then rebuild
# the adapter from a clean env per case via the _adapter() helper.
os.environ.setdefault("GOTIFY_SERVER_URL", "https://gotify.example.com")
os.environ.setdefault("GOTIFY_APP_TOKEN", "Aapp")
os.environ.setdefault("GOTIFY_CLIENT_TOKEN", "Cclient")
from librefang.sidecar.adapters import gotify as ga  # noqa: E402


def _adapter(**env):
    defaults = {
        "GOTIFY_SERVER_URL": "https://gotify.example.com",
        "GOTIFY_APP_TOKEN": "Aapp",
        "GOTIFY_CLIENT_TOKEN": "Cclient",
        "GOTIFY_ACCOUNT_ID": "",
    }
    for k, v in defaults.items():
        os.environ[k] = env.get(k, v)
    return ga.GotifyAdapter()


# ---- env / URL normalization --------------------------------------


def test_server_url_strips_trailing_slash():
    a = _adapter(GOTIFY_SERVER_URL="https://gotify.example.com/")
    assert a.server_url == "https://gotify.example.com"


def test_server_url_normalization_into_ws_scheme():
    a = _adapter(GOTIFY_SERVER_URL="https://gotify.example.com")
    assert a._build_ws_url() == (
        "wss://gotify.example.com/stream?token=Cclient"
    )
    a = _adapter(GOTIFY_SERVER_URL="http://localhost:8080")
    assert a._build_ws_url() == (
        "ws://localhost:8080/stream?token=Cclient"
    )


def test_missing_required_env_exits():
    # Empty server url is a config error.
    with pytest.raises(SystemExit) as exc:
        _adapter(GOTIFY_SERVER_URL="")
    assert exc.value.code == 2
    # Empty app token too.
    with pytest.raises(SystemExit):
        _adapter(GOTIFY_APP_TOKEN="")
    # And empty client token.
    with pytest.raises(SystemExit):
        _adapter(GOTIFY_CLIENT_TOKEN="")


def test_invalid_scheme_rejected():
    with pytest.raises(SystemExit) as exc:
        _adapter(GOTIFY_SERVER_URL="ftp://gotify.example.com")
    assert exc.value.code == 2


def test_account_id_optional_and_surfaced():
    a = _adapter(GOTIFY_ACCOUNT_ID="prod")
    assert a.account_id == "prod"
    a = _adapter(GOTIFY_ACCOUNT_ID="")
    assert a.account_id is None


# ---- _parse_frame: gotify WS JSON shape ---------------------------


def test_parse_frame_full_shape():
    a = _adapter()
    raw = json.dumps({
        "id": 42, "appid": 7, "message": "Hello Gotify",
        "title": "Test App", "priority": 5,
        "date": "2024-01-01T00:00:00Z",
    })
    assert a._parse_frame(raw) == (42, "Hello Gotify", "Test App", 5, 7)


def test_parse_frame_empty_message_is_skipped():
    a = _adapter()
    raw = json.dumps({"id": 1, "appid": 1, "message": "",
                      "title": "", "priority": 0})
    assert a._parse_frame(raw) is None


def test_parse_frame_minimal_fields_defaulted():
    a = _adapter()
    raw = json.dumps({"id": 1, "message": "hi"})
    res = a._parse_frame(raw)
    assert res is not None
    mid, msg, title, priority, app_id = res
    assert mid == 1
    assert msg == "hi"
    assert title == ""
    assert priority == 0
    assert app_id == 0


def test_parse_frame_bad_json_is_skipped():
    a = _adapter()
    assert a._parse_frame("not json") is None
    assert a._parse_frame("[]") is None  # non-dict top-level
    assert a._parse_frame('{"message":"hi"}') is None  # missing id


# ---- _to_event: command vs text, sender, metadata -----------------


def test_to_event_text_uses_title_as_sender_display():
    a = _adapter()
    ev = a._to_event(42, "Hello", "Backup Service", 5, 7)
    assert ev["method"] == "message"
    params = ev["params"]
    assert params["user_id"] == "app-7"
    assert params["user_name"] == "Backup Service"
    assert params["content"] == {"Text": "Hello"}
    assert params["message_id"] == "gotify-42"
    assert params["metadata"] == {
        "title": "Backup Service",
        "priority": 5,
        "app_id": 7,
    }
    # is_group defaults False for gotify (1:1 push); a False value is
    # omitted by protocol.message rather than emitted explicitly.
    assert "is_group" not in params


def test_to_event_falls_back_to_app_id_when_title_empty():
    a = _adapter()
    ev = a._to_event(1, "hi", "", 0, 7)
    params = ev["params"]
    assert params["user_id"] == "app-7"
    assert params["user_name"] == "app-7"


def test_to_event_slash_prefix_routes_to_command():
    a = _adapter()
    ev = a._to_event(1, "/help me out", "App", 0, 0)
    params = ev["params"]
    assert params["content"] == {
        "Command": {"name": "help", "args": ["me", "out"]}
    }


def test_to_event_lone_slash_command_no_args():
    a = _adapter()
    ev = a._to_event(1, "/ping", "App", 0, 0)
    params = ev["params"]
    assert params["content"] == {
        "Command": {"name": "ping", "args": []}
    }


# ---- _split_message -----------------------------------------------


def test_split_message_under_limit_is_one_chunk():
    assert ga._split_message("short", 100) == ["short"]


def test_split_message_prefers_newline_cut():
    body = "a" * 80 + "\n" + "b" * 80
    chunks = ga._split_message(body, 100)
    assert len(chunks) == 2
    assert chunks[0] == "a" * 80
    assert chunks[1] == "b" * 80


def test_split_message_hard_cut_when_no_newline():
    chunks = ga._split_message("x" * 250, 100)
    assert [len(c) for c in chunks] == [100, 100, 50]


# ---- _publish: REST send shape ------------------------------------


class _FakeUrlopen:
    """Stand-in for ``urllib.request.urlopen`` capturing the request
    and returning a fake response."""

    def __init__(self, status=200):
        self.calls: list[dict] = []
        self.status = status

    def __call__(self, req, timeout=None):
        body = req.data
        try:
            decoded = json.loads(body.decode("utf-8"))
        except Exception:
            decoded = None
        self.calls.append({
            "url": req.full_url,
            "method": req.get_method(),
            "headers": {k.lower(): v for k, v in req.header_items()},
            "body_raw": body,
            "body_json": decoded,
            "timeout": timeout,
        })
        return _FakeResp(self.status)


class _FakeResp:
    def __init__(self, status):
        self.status = status

    def __enter__(self):
        return self

    def __exit__(self, *_):
        return False


def test_publish_posts_to_message_endpoint_with_app_token(monkeypatch):
    a = _adapter()
    fake = _FakeUrlopen()
    monkeypatch.setattr(ga.urllib.request, "urlopen", fake)
    a._publish("Hello")
    assert len(fake.calls) == 1
    call = fake.calls[0]
    assert call["url"] == (
        "https://gotify.example.com/message?token=Aapp"
    )
    assert call["method"] == "POST"
    assert call["headers"]["content-type"] == "application/json"
    assert call["body_json"] == {
        "title": "LibreFang",
        "message": "Hello",
        "priority": 5,
    }


def test_publish_chunks_with_numbered_title(monkeypatch):
    a = _adapter()
    fake = _FakeUrlopen()
    monkeypatch.setattr(ga.urllib.request, "urlopen", fake)
    big = "x" * (ga.MAX_MESSAGE_LEN + 100)
    a._publish(big)
    assert len(fake.calls) == 2
    assert fake.calls[0]["body_json"]["title"] == "LibreFang (1/2)"
    assert fake.calls[1]["body_json"]["title"] == "LibreFang (2/2)"


def test_publish_raises_on_http_500(monkeypatch):
    a = _adapter()
    fake = _FakeUrlopen(status=500)
    monkeypatch.setattr(ga.urllib.request, "urlopen", fake)
    with pytest.raises(RuntimeError, match="HTTP 500"):
        a._publish("Hello")


def test_publish_raises_on_http_error(monkeypatch):
    a = _adapter()

    class _HTTPError(ga.urllib.error.HTTPError):
        def __init__(self):
            super().__init__("u", 401, "Unauthorized", {},
                             io.BytesIO(b'{"error":"bad token"}'))

    def _bad(req, timeout=None):
        raise _HTTPError()

    monkeypatch.setattr(ga.urllib.request, "urlopen", _bad)
    with pytest.raises(RuntimeError, match="401"):
        a._publish("Hello")


# ---- _WebSocketReader: frame parsing over a loopback server -------


def _server_pong_frame(payload: bytes) -> bytes:
    # Server→client frames are NOT masked.
    header = bytearray([0x8A])  # FIN | pong
    header.append(len(payload))
    return bytes(header) + payload


def _server_text_frame(text: str) -> bytes:
    payload = text.encode("utf-8")
    header = bytearray([0x81])  # FIN | text
    ln = len(payload)
    if ln < 126:
        header.append(ln)
    elif ln < 65536:
        header.append(126)
        header.extend(ln.to_bytes(2, "big"))
    else:
        header.append(127)
        header.extend(ln.to_bytes(8, "big"))
    return bytes(header) + payload


def _server_close_frame() -> bytes:
    return bytes([0x88, 0x00])  # FIN | close, length 0


def test_websocket_reader_handshake_and_text_frames():
    """Drive _WebSocketReader against a one-shot loopback server that
    performs a real RFC 6455 handshake, sends two text frames and a
    close. We assert both frames are yielded as decoded strings."""
    import base64 as _b64
    import hashlib as _hl

    server = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    server.bind(("127.0.0.1", 0))
    server.listen(1)
    host, port = server.getsockname()

    captured: dict = {}

    def _serve():
        conn, _addr = server.accept()
        try:
            req = b""
            while b"\r\n\r\n" not in req:
                chunk = conn.recv(4096)
                if not chunk:
                    return
                req += chunk
            head = req.split(b"\r\n\r\n", 1)[0]
            # Extract the client's Sec-WebSocket-Key for the response.
            key = None
            for line in head.split(b"\r\n")[1:]:
                name, _, val = line.partition(b":")
                if name.strip().lower() == b"sec-websocket-key":
                    key = val.strip()
                    break
            captured["key"] = key
            accept = _b64.b64encode(
                _hl.sha1(key + ga._WS_GUID.encode("ascii")).digest()
            )
            resp = (
                b"HTTP/1.1 101 Switching Protocols\r\n"
                b"Upgrade: websocket\r\n"
                b"Connection: Upgrade\r\n"
                b"Sec-WebSocket-Accept: " + accept + b"\r\n\r\n"
            )
            conn.sendall(resp)
            conn.sendall(_server_text_frame('{"id":1,"message":"hi"}'))
            conn.sendall(_server_text_frame('{"id":2,"message":"there"}'))
            conn.sendall(_server_close_frame())
            # Read the client's close echo so it's not aborted mid-write.
            time.sleep(0.05)
        finally:
            try:
                conn.close()
            except OSError:
                pass
            server.close()

    t = threading.Thread(target=_serve, daemon=True)
    t.start()

    url = f"ws://{host}:{port}/stream?token=Cclient"
    out: list[str] = []
    with ga._WebSocketReader(url, handshake_timeout=5.0) as ws:
        for text in ws:
            out.append(text)
            if len(out) >= 2:
                # Don't keep reading after the second frame; the close
                # frame still arrives and the iterator exits cleanly.
                pass

    t.join(timeout=2.0)
    assert out == [
        '{"id":1,"message":"hi"}',
        '{"id":2,"message":"there"}',
    ]
    assert captured.get("key") is not None


def test_websocket_reader_rejects_non_ws_url():
    with pytest.raises(ValueError, match="not a websocket url"):
        with ga._WebSocketReader("http://example.com/stream"):
            pass


# ---- WS server pings, close echo, oversized frame rejection -------


def _server_ping_frame(payload: bytes) -> bytes:
    # Server→client control frame (FIN | ping), unmasked.
    return bytes([0x89, len(payload)]) + payload


def _parse_client_frame(blob: bytes) -> tuple[int, bytes]:
    """Decode one client→server frame. Client frames are always masked."""
    assert len(blob) >= 2
    fin_opcode = blob[0]
    opcode = fin_opcode & 0x0F
    mask_bit = (blob[1] & 0x80) != 0
    assert mask_bit, "client→server frames must be masked"
    ln = blob[1] & 0x7F
    off = 2
    if ln == 126:
        ln = int.from_bytes(blob[off:off + 2], "big")
        off += 2
    elif ln == 127:
        ln = int.from_bytes(blob[off:off + 8], "big")
        off += 8
    mask = blob[off:off + 4]
    off += 4
    raw = blob[off:off + ln]
    unmasked = bytes(b ^ mask[i % 4] for i, b in enumerate(raw))
    return opcode, unmasked


def _ws_handshake_reply(req_head: bytes) -> bytes:
    import base64 as _b64
    import hashlib as _hl
    key = None
    for line in req_head.split(b"\r\n")[1:]:
        name, _, val = line.partition(b":")
        if name.strip().lower() == b"sec-websocket-key":
            key = val.strip()
            break
    assert key is not None
    accept = _b64.b64encode(
        _hl.sha1(key + ga._WS_GUID.encode("ascii")).digest()
    )
    return (
        b"HTTP/1.1 101 Switching Protocols\r\n"
        b"Upgrade: websocket\r\n"
        b"Connection: Upgrade\r\n"
        b"Sec-WebSocket-Accept: " + accept + b"\r\n\r\n"
    )


def test_websocket_reader_responds_to_server_ping_with_pong():
    """Server sends ping with a payload between two text frames; the
    client must reply with a masked pong carrying the same payload,
    and continue yielding the text frames around it."""
    server = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    server.bind(("127.0.0.1", 0))
    server.listen(1)
    host, port = server.getsockname()

    captured: dict = {"client_frames": []}

    def _serve():
        conn, _ = server.accept()
        try:
            req = b""
            while b"\r\n\r\n" not in req:
                req += conn.recv(4096)
            head = req.split(b"\r\n\r\n", 1)[0]
            conn.sendall(_ws_handshake_reply(head))
            conn.sendall(_server_text_frame('{"id":1,"message":"a"}'))
            conn.sendall(_server_ping_frame(b"ping-payload"))
            conn.sendall(_server_text_frame('{"id":2,"message":"b"}'))
            conn.sendall(_server_close_frame())
            # Read whatever the client sends back (pong + close echo).
            conn.settimeout(2.0)
            try:
                while True:
                    chunk = conn.recv(4096)
                    if not chunk:
                        break
                    captured["client_frames"].append(chunk)
            except (OSError, socket.timeout):
                pass
        finally:
            conn.close()
            server.close()

    t = threading.Thread(target=_serve, daemon=True)
    t.start()

    out: list[str] = []
    url = f"ws://{host}:{port}/stream?token=Cclient"
    with ga._WebSocketReader(url, handshake_timeout=5.0) as ws:
        for text in ws:
            out.append(text)

    t.join(timeout=3.0)
    assert out == [
        '{"id":1,"message":"a"}',
        '{"id":2,"message":"b"}',
    ]
    # Concatenate everything the server saw — the client should have
    # sent one pong (opcode 0xA, payload b"ping-payload") and one
    # close echo (opcode 0x8, payload b"").
    blob = b"".join(captured["client_frames"])
    opcodes = []
    i = 0
    while i < len(blob):
        # Each client frame: 1 (fin|op) + 1 (mask|len7) + maybe 2/8 +
        # 4 mask + payload. Header is at least 2 bytes here because
        # we send small payloads.
        opcode = blob[i] & 0x0F
        ln = blob[i + 1] & 0x7F
        off = 2
        if ln == 126:
            ln = int.from_bytes(blob[i + 2:i + 4], "big")
            off = 4
        elif ln == 127:
            ln = int.from_bytes(blob[i + 2:i + 10], "big")
            off = 10
        off += 4  # mask
        opcodes.append((opcode, blob[i:i + off + ln]))
        i += off + ln
    # Find the pong and verify payload.
    pongs = [_parse_client_frame(raw) for op, raw in opcodes if op == 0xA]
    closes = [_parse_client_frame(raw) for op, raw in opcodes if op == 0x8]
    assert len(pongs) == 1, f"expected 1 pong, got {len(pongs)}"
    assert pongs[0] == (0xA, b"ping-payload")
    assert len(closes) == 1, f"expected 1 close echo, got {len(closes)}"
    assert closes[0] == (0x8, b"")


def test_websocket_reader_rejects_oversized_frame():
    """Server announces a 64-bit payload length exceeding the cap; the
    client must fail the stream with a RuntimeError instead of trying
    to read multi-gigabyte payload."""
    server = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    server.bind(("127.0.0.1", 0))
    server.listen(1)
    host, port = server.getsockname()

    def _serve():
        conn, _ = server.accept()
        try:
            req = b""
            while b"\r\n\r\n" not in req:
                req += conn.recv(4096)
            head = req.split(b"\r\n\r\n", 1)[0]
            conn.sendall(_ws_handshake_reply(head))
            # Text frame, 64-bit length, 1 GiB — well over the 1 MiB cap.
            payload_len = (1 << 30)
            frame = bytes([0x81, 127]) + payload_len.to_bytes(8, "big")
            conn.sendall(frame)
            conn.settimeout(2.0)
            try:
                while conn.recv(4096):
                    pass
            except (OSError, socket.timeout):
                pass
        finally:
            conn.close()
            server.close()

    t = threading.Thread(target=_serve, daemon=True)
    t.start()

    url = f"ws://{host}:{port}/stream?token=Cclient"
    with pytest.raises(RuntimeError, match="exceeds cap"):
        with ga._WebSocketReader(url, handshake_timeout=5.0) as ws:
            for _ in ws:
                pass
    t.join(timeout=3.0)


# ---- account_id surfaced via ready_event --------------------------


def test_account_id_surfaced_via_ready_event():
    """`GOTIFY_ACCOUNT_ID` env var should reach LibreFang through the
    base SidecarAdapter.ready_event() multi-bot routing key."""
    a = _adapter(GOTIFY_ACCOUNT_ID="prod-server")
    ev = a.ready_event()
    # ready event params include account_id when set.
    assert ev["method"] == "ready"
    params = ev["params"]
    assert params.get("account_id") == "prod-server"


def test_no_account_id_when_unset():
    a = _adapter(GOTIFY_ACCOUNT_ID="")
    ev = a.ready_event()
    params = ev["params"]
    # When None, protocol.ready should omit the field (matches ntfy
    # behaviour and the base-class contract).
    assert params.get("account_id") in (None, )


# ---- 429 Retry-After honour (sidecar cross-audit fix) -------------


class _Fake429Then200:
    """First call raises HTTPError(429) with a Retry-After header,
    subsequent calls return 200. Used to assert the retry-once path
    honours the server's backoff window before re-issuing the POST."""

    def __init__(self, retry_after: str = "2"):
        self.calls: list[dict] = []
        self._429_emitted = False
        self._retry_after = retry_after

    def __call__(self, req, timeout=None):
        body = req.data
        self.calls.append({"body": body, "timeout": timeout})
        if not self._429_emitted:
            self._429_emitted = True
            from email.message import Message
            hdrs = Message()
            hdrs["Retry-After"] = self._retry_after
            raise ga.urllib.error.HTTPError(
                req.full_url, 429, "Too Many Requests", hdrs,
                io.BytesIO(b"rate limited"),
            )
        return _FakeResp(200)


class _FakeAlways429:
    """Every call raises 429. Used to assert the second 429 surfaces
    instead of looping forever."""

    def __init__(self, retry_after: str = "1"):
        self.calls = 0
        self._retry_after = retry_after

    def __call__(self, req, timeout=None):
        self.calls += 1
        from email.message import Message
        hdrs = Message()
        hdrs["Retry-After"] = self._retry_after
        raise ga.urllib.error.HTTPError(
            req.full_url, 429, "Too Many Requests", hdrs,
            io.BytesIO(b"rate limited"),
        )


def test_publish_honours_retry_after_on_429_and_retries_once(monkeypatch):
    a = _adapter()
    fake = _Fake429Then200(retry_after="3")
    sleeps: list[float] = []
    monkeypatch.setattr(ga.urllib.request, "urlopen", fake)
    monkeypatch.setattr(ga.time, "sleep", sleeps.append)
    a._publish("hi")
    # One sleep observed at the server-suggested 3-second window.
    assert sleeps == [3.0]
    # Two POSTs: the rate-limited probe and the retry.
    assert len(fake.calls) == 2


def test_publish_raises_on_second_429(monkeypatch):
    a = _adapter()
    fake = _FakeAlways429(retry_after="1")
    monkeypatch.setattr(ga.urllib.request, "urlopen", fake)
    monkeypatch.setattr(ga.time, "sleep", lambda _s: None)
    with pytest.raises(RuntimeError, match="429"):
        a._publish("hi")
    # Exactly one sleep then a fresh 429 surfaces — no infinite loop.
    assert fake.calls == 2


def test_publish_429_without_retry_after_uses_default(monkeypatch):
    a = _adapter()

    class _Fake429ThenOK:
        def __init__(self):
            self.calls = 0

        def __call__(self, req, timeout=None):
            self.calls += 1
            if self.calls == 1:
                from email.message import Message
                raise ga.urllib.error.HTTPError(
                    req.full_url, 429, "Too Many Requests",
                    Message(), io.BytesIO(b""),
                )
            return _FakeResp(200)

    fake = _Fake429ThenOK()
    sleeps: list[float] = []
    monkeypatch.setattr(ga.urllib.request, "urlopen", fake)
    monkeypatch.setattr(ga.time, "sleep", sleeps.append)
    a._publish("hi")
    # Without an explicit header we fall back to
    # RETRY_AFTER_DEFAULT_SECS from the shared common module.
    from librefang.sidecar import common as _common
    assert sleeps == [_common.RETRY_AFTER_DEFAULT_SECS]


# ---- inbound dedupe (sidecar cross-audit fix) ---------------------


def test_ws_loop_dedupes_redelivered_frame_id():
    """Server replay across reconnect must not double-emit. The
    sidecar keys on ``gotify-<id>`` which is the message_id round-
    tripped to the daemon, so the dedupe slot matches what the
    bridge already keys on too."""
    a = _adapter()
    emitted: list[dict] = []

    class _Iter:
        def __init__(self):
            self.frames = [
                '{"id":42,"message":"hi"}',
                '{"id":42,"message":"hi"}',  # replay
                '{"id":43,"message":"there"}',
            ]
            self.i = 0

        def __enter__(self):
            return self

        def __exit__(self, *_):
            return False

        def __iter__(self):
            return self

        def __next__(self):
            if self.i >= len(self.frames):
                raise StopIteration
            v = self.frames[self.i]
            self.i += 1
            return v

    # Patch _WebSocketReader to yield our fixed sequence.
    def _fake_reader(url, *args, **kwargs):
        return _Iter()

    import librefang.sidecar.adapters.gotify as gotify_mod
    orig = gotify_mod._WebSocketReader
    gotify_mod._WebSocketReader = _fake_reader
    try:
        a._ws_loop(emitted.append)
    finally:
        gotify_mod._WebSocketReader = orig

    # The replay is suppressed; only the two distinct IDs land.
    assert [e["params"]["message_id"] for e in emitted] == [
        "gotify-42", "gotify-43",
    ]
