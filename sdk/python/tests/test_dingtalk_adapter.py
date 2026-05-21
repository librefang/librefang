"""Tests for librefang.sidecar.adapters.dingtalk.

Deterministic, no network: WebSocket is replaced with an in-memory
transcript, gateway HTTP via a monkeypatched _http_request. Asserts
the sidecar preserves the in-process Rust ``librefang-channels::
dingtalk`` stream-mode behaviour plus the four improvements
documented in the module header. Callback (webhook) mode is **not**
ported — out of scope for this suite.
"""
import json
import os
import queue
import threading
import time

import pytest


os.environ.setdefault("DINGTALK_APP_KEY", "dingtest")
os.environ.setdefault("DINGTALK_APP_SECRET", "test-secret")
from librefang.sidecar.adapters import dingtalk as dt_mod  # noqa: E402
from librefang.sidecar import protocol  # noqa: E402


def _adapter(**env):
    defaults = {
        "DINGTALK_APP_KEY": "dingtest",
        "DINGTALK_APP_SECRET": "test-secret",
        "DINGTALK_ALLOWED_USERS": "",
        "DINGTALK_ACCOUNT_ID": "",
        "DINGTALK_GATEWAY_URL": "",
    }
    for k, v in defaults.items():
        os.environ[k] = env.get(k, v)
    return dt_mod.DingTalkAdapter()


# ─── Env enforcement ────────────────────────────────────────────────


def test_default_env_construction():
    a = _adapter()
    assert a.app_key == "dingtest"
    assert a.app_secret == "test-secret"
    assert a.allowed_users == []
    assert a.account_id is None
    assert a.gateway_url == dt_mod.DINGTALK_GATEWAY_URL


def test_app_key_whitespace_stripped():
    a = _adapter(DINGTALK_APP_KEY="  dingwhite  ")
    assert a.app_key == "dingwhite"


def test_allowed_users_csv_split():
    a = _adapter(DINGTALK_ALLOWED_USERS="alice, bob ,, carol")
    assert a.allowed_users == ["alice", "bob", "carol"]


def test_account_id_passthrough():
    a = _adapter(DINGTALK_ACCOUNT_ID="prod-bot")
    assert a.account_id == "prod-bot"


def test_account_id_empty_is_none():
    a = _adapter(DINGTALK_ACCOUNT_ID="")
    assert a.account_id is None


def test_gateway_url_override():
    a = _adapter(DINGTALK_GATEWAY_URL="https://mock.test/gw")
    assert a.gateway_url == "https://mock.test/gw"


def test_missing_app_key_exits_2():
    os.environ.pop("DINGTALK_APP_KEY", None)
    os.environ["DINGTALK_APP_SECRET"] = "x"
    with pytest.raises(SystemExit) as exc:
        dt_mod.DingTalkAdapter()
    assert exc.value.code == 2


def test_missing_secret_exits_2():
    os.environ["DINGTALK_APP_KEY"] = "x"
    os.environ.pop("DINGTALK_APP_SECRET", None)
    with pytest.raises(SystemExit) as exc:
        dt_mod.DingTalkAdapter()
    assert exc.value.code == 2


def test_whitespace_only_secret_exits_2():
    os.environ["DINGTALK_APP_KEY"] = "x"
    os.environ["DINGTALK_APP_SECRET"] = "   "
    with pytest.raises(SystemExit) as exc:
        dt_mod.DingTalkAdapter()
    assert exc.value.code == 2


# ─── SCHEMA / --describe ─────────────────────────────────────────────


def test_schema_round_trip():
    a = _adapter()
    d = a.SCHEMA.to_dict()
    assert d["name"] == "dingtalk"
    assert d["display_name"] == "DingTalk"
    keys = {f["key"] for f in d["fields"]}
    assert keys == {
        "DINGTALK_APP_KEY", "DINGTALK_APP_SECRET",
        "DINGTALK_ALLOWED_USERS", "DINGTALK_ACCOUNT_ID",
    }


def test_schema_required_fields():
    a = _adapter()
    by_key = {f["key"]: f for f in a.SCHEMA.to_dict()["fields"]}
    assert by_key["DINGTALK_APP_KEY"]["required"] is True
    assert by_key["DINGTALK_APP_SECRET"]["required"] is True
    assert by_key["DINGTALK_APP_SECRET"]["type"] == "secret"
    assert by_key["DINGTALK_ALLOWED_USERS"]["advanced"] is True
    assert by_key["DINGTALK_ACCOUNT_ID"]["advanced"] is True


def test_capabilities_empty():
    a = _adapter()
    assert a.capabilities == []


def test_suppress_error_responses_false():
    a = _adapter()
    assert a.suppress_error_responses is False


# ─── Frame helpers ───────────────────────────────────────────────────


def test_is_system_ping_recognises_ping():
    assert dt_mod._is_system_ping({
        "type": "SYSTEM",
        "headers": {"topic": "ping", "messageId": "m1"},
    })


def test_is_system_ping_rejects_non_system():
    assert not dt_mod._is_system_ping({
        "type": "CALLBACK",
        "headers": {"topic": "ping"},
    })


def test_is_system_ping_rejects_other_topic():
    assert not dt_mod._is_system_ping({
        "type": "SYSTEM",
        "headers": {"topic": "pong"},
    })


def test_build_pong_echoes_data_and_message_id():
    ping = {
        "type": "SYSTEM",
        "headers": {"topic": "ping", "messageId": "ping-1"},
        "data": "payload-bytes",
    }
    pong = json.loads(dt_mod._build_pong_frame(ping))
    assert pong["code"] == 200
    assert pong["message"] == "OK"
    assert pong["headers"]["messageId"] == "ping-1"
    assert pong["headers"]["contentType"] == "application/json"
    assert pong["data"] == "payload-bytes"


def test_build_callback_ack_uses_message_id():
    ack = json.loads(dt_mod._build_callback_ack("cb-1"))
    assert ack["code"] == 200
    assert ack["message"] == "OK"
    assert ack["headers"]["messageId"] == "cb-1"
    # DingTalk requires `data` to be the literal JSON string
    # `{"response": null}` — a dict here would be rejected.
    assert ack["data"] == "{\"response\": null}"


# ─── parse_dingtalk_event ────────────────────────────────────────────


def _callback_frame(payload_overrides=None, **frame_overrides):
    payload = {
        "msgtype": "text",
        "text": {"content": "hello world"},
        "senderStaffId": "alice",
        "senderNick": "Alice",
        "conversationId": "cid1",
        "conversationType": "1",
        "sessionWebhook": "https://oapi.dingtalk.com/robot/sendBySession?session=xxx",
        "sessionWebhookExpiredTime": 1735000000000,
        "msgId": "m1",
    }
    if payload_overrides:
        payload.update(payload_overrides)
    base = {
        "type": "CALLBACK",
        "headers": {"topic": "/v1.0/im/bot/messages/get", "messageId": "mid1"},
        "data": json.dumps(payload),
    }
    base.update(frame_overrides)
    return base


def test_parse_text_message():
    ev = dt_mod.parse_dingtalk_event(_callback_frame())
    p = ev["params"]
    assert ev["method"] == "message"
    assert p["user_id"] == "alice"
    assert p["user_name"] == "Alice"
    assert p["content"] == {"Text": "hello world"}
    assert p["message_id"] == "m1"
    assert p["platform"] == "dingtalk"
    assert p["metadata"]["conversation_id"] == "cid1"
    assert p["metadata"]["session_webhook"].startswith("https://oapi.dingtalk.com")
    assert "is_group" not in p  # 1:1 → False is omitted


def test_parse_group_conversation():
    ev = dt_mod.parse_dingtalk_event(
        _callback_frame({"conversationType": "2"})
    )
    assert ev["params"]["is_group"] is True


def test_parse_slash_command():
    ev = dt_mod.parse_dingtalk_event(
        _callback_frame({"text": {"content": "/deploy prod canary"}})
    )
    assert ev["params"]["content"] == {
        "Command": {"name": "deploy", "args": ["prod", "canary"]}
    }


def test_parse_slash_command_no_args():
    ev = dt_mod.parse_dingtalk_event(
        _callback_frame({"text": {"content": "/help"}})
    )
    assert ev["params"]["content"] == {
        "Command": {"name": "help", "args": []}
    }


def test_parse_non_callback_returns_none():
    f = _callback_frame()
    f["type"] = "SYSTEM"
    assert dt_mod.parse_dingtalk_event(f) is None


def test_parse_non_text_msgtype_returns_none():
    for mt in ("image", "voice", "video", "file"):
        f = _callback_frame({"msgtype": mt})
        assert dt_mod.parse_dingtalk_event(f) is None


def test_parse_empty_text_returns_none():
    assert dt_mod.parse_dingtalk_event(
        _callback_frame({"text": {"content": "   "}})
    ) is None


def test_parse_missing_text_returns_none():
    assert dt_mod.parse_dingtalk_event(
        _callback_frame({"text": None})
    ) is None


def test_parse_data_not_string_returns_none():
    # Frame-level malformed: data must be a JSON STRING, not a dict
    assert dt_mod.parse_dingtalk_event({
        "type": "CALLBACK",
        "headers": {"messageId": "mid"},
        "data": {"already": "parsed"},
    }) is None


def test_parse_data_unparseable_returns_none():
    assert dt_mod.parse_dingtalk_event({
        "type": "CALLBACK",
        "headers": {"messageId": "mid"},
        "data": "this is not json {",
    }) is None


def test_parse_sender_id_falls_back_to_sender_id_field():
    f = _callback_frame({
        "senderStaffId": None, "senderId": "fallback-id"
    })
    ev = dt_mod.parse_dingtalk_event(f)
    assert ev["params"]["user_id"] == "fallback-id"


def test_parse_was_mentioned_from_is_in_at_list():
    ev = dt_mod.parse_dingtalk_event(
        _callback_frame({"isInAtList": True})
    )
    assert ev["params"]["metadata"].get("was_mentioned") is True


def test_parse_was_mentioned_from_at_users_array():
    ev = dt_mod.parse_dingtalk_event(
        _callback_frame({"atUsers": [{"dingtalkId": "x"}]})
    )
    assert ev["params"]["metadata"].get("was_mentioned") is True


def test_parse_was_mentioned_omitted_when_neither():
    ev = dt_mod.parse_dingtalk_event(_callback_frame())
    assert "was_mentioned" not in ev["params"]["metadata"]


def test_parse_allowlist_blocks_others():
    ev = dt_mod.parse_dingtalk_event(
        _callback_frame(), allowed_users=["bob"]
    )
    assert ev is None


def test_parse_allowlist_passes_match():
    ev = dt_mod.parse_dingtalk_event(
        _callback_frame(), allowed_users=["alice", "bob"]
    )
    assert ev is not None


def test_parse_account_id_injected():
    ev = dt_mod.parse_dingtalk_event(
        _callback_frame(), account_id="prod-bot"
    )
    assert ev["params"]["metadata"]["account_id"] == "prod-bot"


def test_parse_account_id_omitted_when_unset():
    ev = dt_mod.parse_dingtalk_event(_callback_frame())
    assert "account_id" not in ev["params"]["metadata"]


def test_parse_message_id_falls_back_to_headers():
    f = _callback_frame({"msgId": None})
    ev = dt_mod.parse_dingtalk_event(f)
    assert ev["params"]["message_id"] == "mid1"


def test_parse_message_id_generates_when_all_missing():
    f = {
        "type": "CALLBACK",
        "headers": {},
        "data": json.dumps({
            "msgtype": "text",
            "text": {"content": "hi"},
            "senderStaffId": "u",
        }),
    }
    ev = dt_mod.parse_dingtalk_event(f)
    assert ev["params"]["message_id"].startswith("dt-")


def test_parse_omits_zero_expired_time():
    ev = dt_mod.parse_dingtalk_event(
        _callback_frame({"sessionWebhookExpiredTime": 0})
    )
    assert "session_webhook_expired_time" not in ev["params"]["metadata"]


# ─── _enqueue_text + _post_reply routing ─────────────────────────────


def test_enqueue_text_chunks_long_message():
    a = _adapter()
    long_text = "x" * (dt_mod.DINGTALK_MAX_MESSAGE_LEN + 100)
    a._enqueue_text("https://session.test", long_text)
    f1 = a._send_queue.get_nowait()
    f2 = a._send_queue.get_nowait()
    assert f1[0] == f2[0] == "https://session.test"
    assert len(f1[1]) + len(f2[1]) == len(long_text)


def test_enqueue_text_empty_is_noop():
    a = _adapter()
    a._enqueue_text("https://x", "")
    with pytest.raises(queue.Empty):
        a._send_queue.get_nowait()


def test_enqueue_text_no_session_webhook_is_noop():
    a = _adapter()
    a._enqueue_text("", "hello")
    with pytest.raises(queue.Empty):
        a._send_queue.get_nowait()


# ─── _mark_seen / SeenSet integration ────────────────────────────────


def test_mark_seen_first_true_second_false():
    a = _adapter()
    assert a._mark_seen("r1") is True
    assert a._mark_seen("r1") is False


def test_mark_seen_empty_always_true():
    a = _adapter()
    assert a._mark_seen("") is True
    assert a._mark_seen(None) is True


# ─── on_send routing ─────────────────────────────────────────────────


class _FakeSend:
    def __init__(self, *, channel_id="", text="", content=None,
                 thread_id=None, user=None):
        self.channel_id = channel_id
        self.text = text
        self.content = content
        self.thread_id = thread_id
        self.user = user or {}


@pytest.mark.asyncio
async def test_on_send_uses_cached_session_webhook():
    a = _adapter()
    with a._session_lock:
        a._session_webhooks["m1"] = "https://oapi.dingtalk.com/robot/sendBySession?s=1"
    await a.on_send(_FakeSend(channel_id="m1", text="hi"))
    url, chunk = a._send_queue.get_nowait()
    assert url.endswith("s=1")
    assert chunk == "hi"
    # Cache must evict on consumption.
    assert "m1" not in a._session_webhooks


@pytest.mark.asyncio
async def test_on_send_recovers_session_webhook_from_user_librefang_user():
    """The production daemon rebuilds ``cmd.user`` from
    ``ChannelUser{platform_id, display_name, librefang_user}`` and
    sets ``cmd.channel_id = user.platform_id`` (NOT message_id). So
    ``on_send`` MUST recover the per-message reply URL from
    ``cmd.user["librefang_user"]`` (the field
    ``parse_dingtalk_event`` populates via ``protocol.message(
    librefang_user=session_webhook)``). Cache-by-message_id path is
    only a secondary route and not hit by the real bridge.

    Regression guard: the first sidecar landing had this backwards —
    it tried ``_session_webhooks[cmd.channel_id]`` first and read
    ``cmd.user["session_webhook"]`` (a key that doesn't exist in the
    daemon's serialization) as fallback, silently dropping every
    reply. See PR #5417 self-review."""
    a = _adapter()
    await a.on_send(_FakeSend(
        # Real daemon shape: channel_id is sender's staffId, not the
        # inbound message_id — the cache lookup misses.
        channel_id="alice",
        text="hello",
        user={
            "platform_id": "alice",
            "display_name": "Alice",
            "librefang_user": "https://oapi.dingtalk.com/robot/sendBySession?s=42",
        },
    ))
    url, chunk = a._send_queue.get_nowait()
    assert url.endswith("s=42"), \
        "on_send must recover the URL from cmd.user.librefang_user " \
        "(not cmd.user.session_webhook, not _session_webhooks[channel_id])"
    assert chunk == "hello"


@pytest.mark.asyncio
async def test_on_send_ignores_non_http_librefang_user():
    """`librefang_user` is reused across many channels (Telegram puts the
    Telegram username there, etc.). on_send must only treat it as a
    sessionWebhook URL when it actually looks like an HTTP(S) URL —
    otherwise we'd POST replies to garbage destinations."""
    a = _adapter()
    await a.on_send(_FakeSend(
        channel_id="alice",
        text="hi",
        user={"platform_id": "alice", "librefang_user": "alice"},
    ))
    with pytest.raises(queue.Empty):
        a._send_queue.get_nowait()


@pytest.mark.asyncio
async def test_on_send_drops_when_no_session_webhook():
    a = _adapter()
    await a.on_send(_FakeSend(channel_id="missing", text="hi"))
    with pytest.raises(queue.Empty):
        a._send_queue.get_nowait()


@pytest.mark.asyncio
async def test_on_send_unsupported_content_uses_placeholder():
    a = _adapter()
    with a._session_lock:
        a._session_webhooks["m1"] = "https://oapi.dingtalk.com/x"
    await a.on_send(_FakeSend(
        channel_id="m1",
        content={"Image": {"url": "http://x/p.png"}},
    ))
    _, chunk = a._send_queue.get_nowait()
    assert "(Unsupported content type)" in chunk


@pytest.mark.asyncio
async def test_on_send_empty_text_drops():
    a = _adapter()
    with a._session_lock:
        a._session_webhooks["m1"] = "https://oapi.dingtalk.com/x"
    await a.on_send(_FakeSend(channel_id="m1", text=""))
    with pytest.raises(queue.Empty):
        a._send_queue.get_nowait()


# ─── _register_gateway via monkeypatched _http_request ──────────────


def _mock_http(monkeypatch, *, status=200, body=None):
    """Replace ``_http_request`` in the dingtalk module with a stub
    that returns the given (status, body) tuple. Captures call args
    so tests can assert."""
    captured: dict[str, list] = {"calls": []}

    def fake(url, *, method="GET", body=None, headers=None, timeout=15.0):
        captured["calls"].append({
            "url": url, "method": method, "body": body,
            "headers": headers or {}, "timeout": timeout,
        })
        return status, captured["next_body"], json.dumps(captured["next_body"]).encode("utf-8") if captured["next_body"] else b"", captured.get("next_headers", {})

    captured["next_body"] = body
    monkeypatch.setattr(dt_mod, "_http_request", fake)
    return captured


def test_register_gateway_returns_endpoint_and_ticket(monkeypatch):
    a = _adapter()
    cap = _mock_http(monkeypatch, status=200, body={
        "endpoint": "wss://stream.dingtalk.test/connection",
        "ticket": "tkt-aB+/=",
    })
    endpoint, ticket = a._register_gateway()
    assert endpoint == "wss://stream.dingtalk.test/connection"
    assert ticket == "tkt-aB+/="
    assert cap["calls"][0]["method"] == "POST"
    assert cap["calls"][0]["url"] == dt_mod.DINGTALK_GATEWAY_URL
    body = json.loads(cap["calls"][0]["body"])
    assert body["clientId"] == "dingtest"
    assert body["clientSecret"] == "test-secret"
    assert body["ua"] == "librefang"
    assert body["subscriptions"][0]["topic"] == "/v1.0/im/bot/messages/get"


def test_register_gateway_missing_endpoint_raises(monkeypatch):
    a = _adapter()
    _mock_http(monkeypatch, status=200, body={"ticket": "tkt"})
    with pytest.raises(RuntimeError, match="missing endpoint"):
        a._register_gateway()


def test_register_gateway_missing_ticket_raises(monkeypatch):
    a = _adapter()
    _mock_http(monkeypatch, status=200, body={"endpoint": "wss://x"})
    with pytest.raises(RuntimeError, match="missing ticket"):
        a._register_gateway()


def test_register_gateway_non_200_raises(monkeypatch):
    a = _adapter()
    _mock_http(monkeypatch, status=500, body={"errmsg": "internal"})
    with pytest.raises(RuntimeError, match="registration failed"):
        a._register_gateway()


# ─── _run_session via in-memory WS fake ──────────────────────────────


class _FakeWs:
    """In-memory WebSocketClient replacement. Same shape as the
    wecom test fake (single-step script, send capture)."""

    def __init__(self, inbound_script):
        self.script = list(inbound_script)
        self.sent: list[str] = []
        self._cursor = 0
        self.closed = False

    def send_text(self, s: str) -> None:
        self.sent.append(s)

    def wait_readable(self, timeout: float) -> bool:
        return self._cursor < len(self.script)

    def recv_frame(self):
        if self._cursor >= len(self.script):
            return None, (1000, b"")
        entry = self.script[self._cursor]
        self._cursor += 1
        if entry is None:
            return None, None
        if entry == "CLOSE":
            return None, (1000, b"server bye")
        if isinstance(entry, dict):
            return json.dumps(entry), None
        return entry, None

    def settimeout(self, t):
        pass


def _drive_session(adapter, inbound_script, *, max_secs=2.0):
    ws = _FakeWs(inbound_script)
    emitted: list[dict] = []
    t = threading.Thread(
        target=adapter._run_session,
        args=(ws, lambda ev: emitted.append(ev)),
        daemon=True,
    )
    t.start()
    t.join(timeout=max_secs)
    return ws, emitted


def test_run_session_emits_callback_after_parse():
    a = _adapter()
    ws, emitted = _drive_session(a, [_callback_frame(), "CLOSE"])
    assert len(emitted) == 1
    assert emitted[0]["params"]["user_id"] == "alice"


def test_run_session_acks_every_callback():
    a = _adapter()
    ws, _ = _drive_session(a, [_callback_frame(), "CLOSE"])
    # First send must be the ACK frame for messageId "mid1"
    assert ws.sent, "ACK frame should have been sent"
    ack = json.loads(ws.sent[0])
    assert ack["code"] == 200
    assert ack["headers"]["messageId"] == "mid1"
    assert ack["data"] == "{\"response\": null}"


def test_run_session_acks_even_when_event_is_dropped():
    """Non-text msgtypes still get an ACK so DingTalk doesn't
    redeliver — important contract (Rust dingtalk.rs:525 always ACKs
    after parse, regardless of outcome)."""
    a = _adapter()
    image_frame = _callback_frame({"msgtype": "image"})
    ws, emitted = _drive_session(a, [image_frame, "CLOSE"])
    assert emitted == []  # dropped
    assert any(json.loads(s).get("message") == "OK" for s in ws.sent)


def test_run_session_responds_to_system_ping_with_pong():
    a = _adapter()
    ping = {
        "type": "SYSTEM",
        "headers": {"topic": "ping", "messageId": "ping-1"},
        "data": "echo-me",
    }
    ws, _ = _drive_session(a, [ping, "CLOSE"])
    assert ws.sent, "pong should have been sent"
    pong = json.loads(ws.sent[0])
    assert pong["headers"]["messageId"] == "ping-1"
    assert pong["data"] == "echo-me"


def test_run_session_dedupes_duplicate_message_id():
    a = _adapter()
    ws, emitted = _drive_session(
        a, [_callback_frame(), _callback_frame(), "CLOSE"],
    )
    # First emitted, second deduped (same msgId="m1")
    assert len(emitted) == 1


def test_run_session_caches_session_webhook_for_passive_reply():
    a = _adapter()
    ws, _ = _drive_session(a, [_callback_frame(), "CLOSE"])
    # parse_dingtalk_event's metadata.session_webhook should land in
    # the adapter's per-message cache so on_send can pop it.
    assert a._session_webhooks.get("m1", "").startswith("https://oapi.dingtalk.com")


def test_run_session_ignores_unknown_frame_types():
    a = _adapter()
    ws, emitted = _drive_session(a, [
        {"type": "UNKNOWN", "headers": {"topic": "noop"}},
        _callback_frame(),
        "CLOSE",
    ])
    assert len(emitted) == 1


def test_run_session_drains_send_queue_to_post_reply(monkeypatch):
    """Pending outbound chunk → _post_reply called with the right
    (url, body) pair."""
    a = _adapter()
    captured = []

    def fake_post(url, *, method="GET", body=None, headers=None, timeout=15.0):
        captured.append({"url": url, "method": method, "body": body})
        return 200, {"errcode": 0, "errmsg": "ok"}, b"{}", {}

    monkeypatch.setattr(dt_mod, "_http_request", fake_post)
    # Queue one pending reply and let _run_session drain it.
    a._send_queue.put(("https://oapi.dingtalk.com/sb?session=z", "hello"))
    ws, _ = _drive_session(a, [None, "CLOSE"])  # one idle tick then close
    assert any(c["url"] == "https://oapi.dingtalk.com/sb?session=z" for c in captured)
    posted_body = json.loads([c for c in captured if c["method"] == "POST"][0]["body"])
    assert posted_body == {"msgtype": "text", "text": {"content": "hello"}}
