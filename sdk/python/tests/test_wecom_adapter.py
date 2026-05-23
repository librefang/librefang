"""Tests for librefang.sidecar.adapters.wecom.

Deterministic, no network: WebSocket is replaced with an in-memory
transcript so the producer can drive subscribe / heartbeat / dispatch
without binding a real socket. Asserts the sidecar preserves the
in-process Rust ``librefang-channels::wecom`` adapter's WebSocket-mode
behaviour plus the three improvements documented in the module header
(req_id dedupe, shared queue heartbeat+send coexistence, observable
server ACK errcodes). Callback mode is **not** ported (no AES in
stdlib) and is therefore out of scope for this suite.
"""
import json
import os
import queue
import threading
import time

import pytest


os.environ.setdefault("WECOM_BOT_ID", "aibtest")
os.environ.setdefault("WECOM_BOT_SECRET", "test-secret")
from librefang.sidecar.adapters import wecom as wecom_mod  # noqa: E402
from librefang.sidecar import protocol  # noqa: E402


def _adapter(**env):
    defaults = {
        "WECOM_BOT_ID": "aibtest",
        "WECOM_BOT_SECRET": "test-secret",
        "WECOM_ALLOWED_USERS": "",
        "WECOM_ACCOUNT_ID": "",
        "WECOM_WS_URL": "",
    }
    for k, v in defaults.items():
        os.environ[k] = env.get(k, v)
    return wecom_mod.WeComAdapter()


# ─── Env enforcement ────────────────────────────────────────────────


def test_default_env_construction():
    a = _adapter()
    assert a.bot_id == "aibtest"
    assert a.secret == "test-secret"
    assert a.allowed_users == []
    assert a.account_id is None
    assert a.ws_url == wecom_mod.WECOM_WS_URL


def test_bot_id_whitespace_stripped():
    a = _adapter(WECOM_BOT_ID="  aibwhite  ")
    assert a.bot_id == "aibwhite"


def test_allowed_users_csv_split():
    a = _adapter(WECOM_ALLOWED_USERS="alice, bob ,, carol")
    assert a.allowed_users == ["alice", "bob", "carol"]


def test_account_id_passthrough():
    a = _adapter(WECOM_ACCOUNT_ID="prod-bot")
    assert a.account_id == "prod-bot"


def test_account_id_empty_is_none():
    a = _adapter(WECOM_ACCOUNT_ID="")
    assert a.account_id is None


def test_ws_url_override():
    a = _adapter(WECOM_WS_URL="wss://mock.test/ws")
    assert a.ws_url == "wss://mock.test/ws"


def test_missing_bot_id_exits_2():
    os.environ.pop("WECOM_BOT_ID", None)
    os.environ["WECOM_BOT_SECRET"] = "x"
    with pytest.raises(SystemExit) as exc:
        wecom_mod.WeComAdapter()
    assert exc.value.code == 2


def test_missing_secret_exits_2():
    os.environ["WECOM_BOT_ID"] = "x"
    os.environ.pop("WECOM_BOT_SECRET", None)
    with pytest.raises(SystemExit) as exc:
        wecom_mod.WeComAdapter()
    assert exc.value.code == 2


def test_whitespace_only_secret_exits_2():
    os.environ["WECOM_BOT_ID"] = "x"
    os.environ["WECOM_BOT_SECRET"] = "   "
    with pytest.raises(SystemExit) as exc:
        wecom_mod.WeComAdapter()
    assert exc.value.code == 2


# ─── SCHEMA / --describe ─────────────────────────────────────────────


def test_schema_round_trip():
    a = _adapter()
    d = a.SCHEMA.to_dict()
    assert d["name"] == "wecom"
    assert d["display_name"] == "WeCom"
    keys = {f["key"] for f in d["fields"]}
    assert keys == {
        "WECOM_BOT_ID", "WECOM_BOT_SECRET",
        "WECOM_ALLOWED_USERS", "WECOM_ACCOUNT_ID",
    }


def test_schema_required_fields():
    a = _adapter()
    by_key = {f["key"]: f for f in a.SCHEMA.to_dict()["fields"]}
    assert by_key["WECOM_BOT_ID"]["required"] is True
    assert by_key["WECOM_BOT_SECRET"]["required"] is True
    assert by_key["WECOM_BOT_SECRET"]["type"] == "secret"
    assert by_key["WECOM_ALLOWED_USERS"]["advanced"] is True
    assert by_key["WECOM_ACCOUNT_ID"]["advanced"] is True


def test_capabilities_empty():
    a = _adapter()
    assert a.capabilities == []


def test_suppress_error_responses_false():
    # Chat-surface precedent (qq / line / mattermost) — failures should
    # surface to the user rather than vanish into the daemon log.
    a = _adapter()
    assert a.suppress_error_responses is False


# ─── Frame helpers ───────────────────────────────────────────────────


def test_frame_cmd_prefers_cmd_over_action():
    assert wecom_mod._frame_cmd({"cmd": "a", "action": "b"}) == "a"


def test_frame_cmd_falls_back_to_action():
    assert wecom_mod._frame_cmd({"action": "legacy"}) == "legacy"


def test_frame_cmd_none_when_neither_present():
    assert wecom_mod._frame_cmd({"foo": "bar"}) is None


def test_frame_body_prefers_body_over_data():
    assert wecom_mod._frame_body({"body": {"x": 1}, "data": {"y": 2}}) == {"x": 1}


def test_frame_body_falls_back_to_data():
    assert wecom_mod._frame_body({"data": {"y": 2}}) == {"y": 2}


def test_frame_req_id_from_headers():
    f = {"headers": {"req_id": "r1"}, "body": {"req_id": "r2"}}
    assert wecom_mod._frame_req_id(f) == "r1"


def test_frame_req_id_from_body_fallback():
    f = {"body": {"req_id": "rb"}}
    assert wecom_mod._frame_req_id(f) == "rb"


def test_frame_req_id_missing():
    assert wecom_mod._frame_req_id({}) is None


# ─── _is_subscribe_success ───────────────────────────────────────────


def test_subscribe_success_explicit_cmd():
    assert wecom_mod._is_subscribe_success({
        "cmd": "aibot_subscribe", "errcode": 0,
    })


def test_subscribe_success_no_cmd_with_req_id_prefix():
    # Server-style ack (no cmd, just errcode + headers.req_id).
    assert wecom_mod._is_subscribe_success({
        "errcode": 0,
        "headers": {"req_id": "aibot_subscribe_12345"},
    })


def test_subscribe_success_explicit_no_errcode():
    # Some server builds omit errcode on success.
    assert wecom_mod._is_subscribe_success({"cmd": "aibot_subscribe"})


def test_subscribe_failure_explicit_nonzero():
    assert not wecom_mod._is_subscribe_success({
        "cmd": "aibot_subscribe", "errcode": 40001,
    })


def test_subscribe_failure_other_cmd():
    assert not wecom_mod._is_subscribe_success({
        "cmd": "pong", "errcode": 0,
    })


# ─── parse_wecom_event ───────────────────────────────────────────────


def _msg_frame(**overrides):
    base = {
        "cmd": "aibot_msg_callback",
        "headers": {"req_id": "req-1"},
        "body": {
            "from": {"userid": "alice"},
            "msgtype": "text",
            "text": {"content": "hello"},
        },
    }
    for k, v in overrides.items():
        if k == "body":
            base["body"].update(v)
        else:
            base[k] = v
    return base


def test_parse_text_message():
    ev = wecom_mod.parse_wecom_event(_msg_frame())
    assert ev["method"] == "message"
    p = ev["params"]
    assert p["user_id"] == "alice"
    assert p["user_name"] == "alice"
    assert p["content"] == {"Text": "hello"}
    assert p["message_id"] == "req-1"
    assert p["platform"] == "wecom"
    assert p["metadata"]["wecom_req_id"] == "req-1"
    assert "is_group" not in p  # default False is omitted


def test_parse_legacy_action_data_keys():
    # Backwards-compat for older WeCom server payloads.
    ev = wecom_mod.parse_wecom_event({
        "action": "aibot_msg_callback",
        "headers": {"req_id": "rL"},
        "data": {
            "from": {"user_id": "bob"},
            "msgtype": "text",
            "text": {"content": "yo"},
            "chat_type": "group",
        },
    })
    assert ev is not None
    assert ev["params"]["user_id"] == "bob"
    assert ev["params"]["is_group"] is True


def test_parse_group_message():
    ev = wecom_mod.parse_wecom_event(_msg_frame(body={"chattype": "group"}))
    assert ev["params"]["is_group"] is True


def test_parse_ignores_non_text():
    for mt in ("image", "voice", "video", "event"):
        f = _msg_frame(body={"msgtype": mt})
        assert wecom_mod.parse_wecom_event(f) is None


def test_parse_ignores_other_cmd():
    f = _msg_frame(cmd="aibot_event_callback")
    assert wecom_mod.parse_wecom_event(f) is None


def test_parse_missing_req_id_returns_none():
    f = {
        "cmd": "aibot_msg_callback",
        "body": {
            "from": {"userid": "alice"},
            "msgtype": "text",
            "text": {"content": "hi"},
        },
    }
    assert wecom_mod.parse_wecom_event(f) is None


def test_parse_missing_user_returns_none():
    f = _msg_frame(body={"from": {}})
    assert wecom_mod.parse_wecom_event(f) is None


def test_parse_empty_content_returns_none():
    f = _msg_frame(body={"text": {"content": ""}})
    assert wecom_mod.parse_wecom_event(f) is None


def test_parse_allowlist_passes_match():
    ev = wecom_mod.parse_wecom_event(
        _msg_frame(), allowed_users=["alice", "bob"],
    )
    assert ev is not None


def test_parse_allowlist_blocks_others():
    ev = wecom_mod.parse_wecom_event(
        _msg_frame(), allowed_users=["bob"],
    )
    assert ev is None


def test_parse_account_id_injected_when_set():
    ev = wecom_mod.parse_wecom_event(_msg_frame(), account_id="prod-bot")
    assert ev["params"]["metadata"]["account_id"] == "prod-bot"


def test_parse_account_id_omitted_when_unset():
    ev = wecom_mod.parse_wecom_event(_msg_frame())
    assert "account_id" not in ev["params"]["metadata"]


def test_parse_response_url_surfaces_in_metadata():
    ev = wecom_mod.parse_wecom_event(_msg_frame(
        body={"response_url": "https://qyapi.weixin.qq.com/cgi-bin/x"},
    ))
    assert ev["params"]["metadata"]["wecom_response_url"] == \
        "https://qyapi.weixin.qq.com/cgi-bin/x"


# ─── Frame builders ──────────────────────────────────────────────────


def test_build_subscribe_frame():
    s = wecom_mod._build_subscribe_frame("aib1", "sec1")
    f = json.loads(s)
    assert f["cmd"] == "aibot_subscribe"
    assert f["body"]["bot_id"] == "aib1"
    assert f["body"]["secret"] == "sec1"
    assert f["headers"]["req_id"].startswith("aibot_subscribe_")


def test_build_reply_frame_uses_markdown():
    s = wecom_mod._build_reply_frame("rX", "hello world")
    f = json.loads(s)
    assert f["cmd"] == "aibot_respond_msg"
    assert f["headers"]["req_id"] == "rX"
    assert f["body"]["msgtype"] == "markdown"
    assert f["body"]["markdown"]["content"] == "hello world"


def test_build_send_frame_includes_receiver():
    s = wecom_mod._build_send_frame("alice", "hi")
    f = json.loads(s)
    assert f["cmd"] == "aibot_send_msg"
    assert f["body"]["receiver"]["userid"] == "alice"
    assert f["body"]["msgtype"] == "markdown"
    assert f["body"]["markdown"]["content"] == "hi"
    assert f["headers"]["req_id"].startswith("aibot_send_msg_")


def test_build_ping_frame():
    s = wecom_mod._build_ping_frame()
    f = json.loads(s)
    assert f["cmd"] == "ping"
    assert f["headers"]["req_id"].startswith("ping_")


# ─── _enqueue_text routing ───────────────────────────────────────────


def test_enqueue_text_falls_back_to_send_msg_when_no_req_id():
    a = _adapter()
    a._enqueue_text("alice", "hello")
    frame = json.loads(a._send_queue.get_nowait())
    assert frame["cmd"] == "aibot_send_msg"


def test_enqueue_text_uses_respond_msg_when_req_id_cached():
    a = _adapter()
    with a._pending_lock:
        a._pending_req_ids["alice"] = "req-99"
    a._enqueue_text("alice", "hello")
    frame = json.loads(a._send_queue.get_nowait())
    assert frame["cmd"] == "aibot_respond_msg"
    assert frame["headers"]["req_id"] == "req-99"
    # Cache must be evicted after first consumption.
    assert "alice" not in a._pending_req_ids


def test_enqueue_text_chunks_long_message():
    a = _adapter()
    long_text = "x" * (wecom_mod.WECOM_MAX_MESSAGE_LEN + 100)
    a._enqueue_text("alice", long_text)
    # Should produce 2 frames.
    frames = [
        json.loads(a._send_queue.get_nowait()),
        json.loads(a._send_queue.get_nowait()),
    ]
    assert all(f["cmd"] == "aibot_send_msg" for f in frames)
    total = "".join(f["body"]["markdown"]["content"] for f in frames)
    assert len(total) == len(long_text)


def test_enqueue_text_first_chunk_respond_rest_send():
    a = _adapter()
    with a._pending_lock:
        a._pending_req_ids["alice"] = "rA"
    long_text = "y" * (wecom_mod.WECOM_MAX_MESSAGE_LEN + 50)
    a._enqueue_text("alice", long_text)
    f1 = json.loads(a._send_queue.get_nowait())
    f2 = json.loads(a._send_queue.get_nowait())
    assert f1["cmd"] == "aibot_respond_msg"
    assert f2["cmd"] == "aibot_send_msg"


def test_enqueue_text_empty_is_noop():
    a = _adapter()
    a._enqueue_text("alice", "")
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
async def test_on_send_text_basic():
    a = _adapter()
    await a.on_send(_FakeSend(channel_id="alice", text="hi"))
    f = json.loads(a._send_queue.get_nowait())
    assert f["body"]["markdown"]["content"] == "hi"


@pytest.mark.asyncio
async def test_on_send_falls_back_to_user_platform_id():
    a = _adapter()
    await a.on_send(_FakeSend(
        text="hello", user={"platform_id": "bob"},
    ))
    f = json.loads(a._send_queue.get_nowait())
    assert f["body"]["receiver"]["userid"] == "bob"


@pytest.mark.asyncio
async def test_on_send_recovers_req_id_from_user_librefang_user_when_cache_cold():
    """Regression guard for sidecar-restart fragility: the in-memory
    ``_pending_req_ids`` cache vanishes on restart, so the bot's
    first reply after restart would fall back to ``aibot_send_msg``
    (active-message quota, blocked for unlicensed bots and rate-
    limited hard for licensed ones). The bridge round-trips
    ``ChannelUser.librefang_user`` bytewise, so the parse-side stash
    of ``req_id`` there is the restart-survivable fallback. This
    test simulates post-restart: ``_pending_req_ids`` empty,
    ``cmd.user.librefang_user`` carries the inbound req_id."""
    a = _adapter()
    # Post-restart: cache empty.
    assert a._pending_req_ids == {}
    await a.on_send(_FakeSend(
        channel_id="alice",
        text="hi",
        user={
            "platform_id": "alice",
            "librefang_user": "req-id-from-inbound-42",
        },
    ))
    f = json.loads(a._send_queue.get_nowait())
    # Must use aibot_respond_msg (passive) keyed on the recovered
    # req_id — NOT aibot_send_msg (active) which is what the
    # pre-fix code would degrade to.
    assert f["cmd"] == "aibot_respond_msg", \
        "on_send must recover req_id from cmd.user.librefang_user when " \
        "_pending_req_ids cache is empty (post-restart scenario)"
    assert f["headers"]["req_id"] == "req-id-from-inbound-42"


@pytest.mark.asyncio
async def test_on_send_in_memory_cache_wins_over_librefang_user():
    """The in-memory cache holds the FRESHEST req_id (the latest
    inbound's id) — it should take precedence over librefang_user
    (which is whichever inbound the daemon happens to round-trip
    back to us, possibly stale). Cache-first, librefang_user as
    fallback."""
    a = _adapter()
    a._pending_req_ids["alice"] = "req-cache-fresh"
    await a.on_send(_FakeSend(
        channel_id="alice",
        text="hi",
        user={
            "platform_id": "alice",
            "librefang_user": "req-stale-roundtrip",
        },
    ))
    f = json.loads(a._send_queue.get_nowait())
    assert f["headers"]["req_id"] == "req-cache-fresh"


@pytest.mark.asyncio
async def test_on_send_ignores_url_shaped_librefang_user():
    """``librefang_user`` is shared across channels — must reject
    cross-channel pollution (dingtalk URL, telegram @username, …)
    before using as a wecom req_id."""
    a = _adapter()
    # Cache empty + URL-shaped librefang_user → must NOT use it,
    # must fall through to aibot_send_msg.
    await a.on_send(_FakeSend(
        channel_id="alice",
        text="hi",
        user={
            "platform_id": "alice",
            "librefang_user": "https://oapi.dingtalk.com/sb?s=42",
        },
    ))
    f = json.loads(a._send_queue.get_nowait())
    assert f["cmd"] == "aibot_send_msg"


@pytest.mark.asyncio
async def test_on_send_unsupported_content_uses_placeholder():
    a = _adapter()
    await a.on_send(_FakeSend(
        channel_id="alice",
        content={"Image": {"url": "http://x/p.png"}},
    ))
    f = json.loads(a._send_queue.get_nowait())
    assert "(Unsupported content type)" in f["body"]["markdown"]["content"]


@pytest.mark.asyncio
async def test_on_send_no_user_id_drops_silently():
    a = _adapter()
    await a.on_send(_FakeSend(text="x"))
    with pytest.raises(queue.Empty):
        a._send_queue.get_nowait()


@pytest.mark.asyncio
async def test_on_send_empty_text_drops():
    a = _adapter()
    await a.on_send(_FakeSend(channel_id="alice", text=""))
    with pytest.raises(queue.Empty):
        a._send_queue.get_nowait()


# ─── _run_session via in-memory WS fake ──────────────────────────────


class _FakeWs:
    """In-memory WebSocketClient replacement. Test scripts a sequence
    of inbound frames (or ``None`` to mean "no data this tick"). Each
    call to ``recv_frame`` returns the next script entry. ``send_text``
    captures all outbound. ``close`` after the script ends.
    """

    def __init__(self, inbound_script):
        self.script = list(inbound_script)
        self.sent: list[str] = []
        self.closed = False
        self._cursor = 0

    def send_text(self, s: str) -> None:
        self.sent.append(s)

    def wait_readable(self, timeout: float) -> bool:
        if self._cursor >= len(self.script):
            self.closed = True
            return True  # so recv_frame can return the close signal
        return self.script[self._cursor] is not None

    def recv_frame(self):
        if self._cursor >= len(self.script):
            return None, (1000, b"")
        entry = self.script[self._cursor]
        self._cursor += 1
        if entry is None:
            return None, None  # idle tick
        if isinstance(entry, dict):
            return json.dumps(entry), None
        # Allow raw close signal
        if entry == "CLOSE":
            return None, (1000, b"server bye")
        return entry, None

    def settimeout(self, t):
        pass


def _drive_session(adapter, inbound_script, *, max_secs=2.0):
    """Run the producer's _run_session in a thread with a fake WS,
    collect emitted events. Stops when the WS script ends."""
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


def test_run_session_sends_subscribe_first():
    a = _adapter()
    ws, _ = _drive_session(a, ["CLOSE"])
    assert ws.sent, "subscribe frame should be the first send"
    first = json.loads(ws.sent[0])
    assert first["cmd"] == "aibot_subscribe"
    assert first["body"]["bot_id"] == "aibtest"


def test_run_session_emits_message_after_subscribe_ack():
    a = _adapter()
    ws, emitted = _drive_session(a, [
        {"errcode": 0, "headers": {"req_id": "aibot_subscribe_1"}},  # ack
        _msg_frame(),
        "CLOSE",
    ])
    assert len(emitted) == 1
    assert emitted[0]["params"]["user_id"] == "alice"


def test_run_session_dedupes_duplicate_req_id():
    a = _adapter()
    ws, emitted = _drive_session(a, [
        {"errcode": 0, "headers": {"req_id": "aibot_subscribe_1"}},
        _msg_frame(),  # req_id == "req-1"
        _msg_frame(),  # duplicate
        "CLOSE",
    ])
    assert len(emitted) == 1


def test_run_session_caches_req_id_for_passive_reply():
    a = _adapter()
    ws, emitted = _drive_session(a, [
        {"errcode": 0, "headers": {"req_id": "aibot_subscribe_1"}},
        _msg_frame(),
        "CLOSE",
    ])
    assert a._pending_req_ids.get("alice") == "req-1"


def test_run_session_subscribe_failure_returns():
    a = _adapter()
    ws, emitted = _drive_session(a, [
        {"cmd": "aibot_subscribe", "errcode": 40001, "errmsg": "bad secret"},
        _msg_frame(),  # should NOT be processed; session returned already
    ], max_secs=1.0)
    assert emitted == []


def test_run_session_ignores_non_msg_callback_frames():
    a = _adapter()
    ws, emitted = _drive_session(a, [
        {"errcode": 0, "headers": {"req_id": "aibot_subscribe_1"}},
        {"cmd": "aibot_event_callback", "body": {"event": "noop"}},
        {"cmd": "pong"},
        {"errcode": 0, "headers": {"req_id": "send-ack-1"}},  # server ack
        "CLOSE",
    ])
    assert emitted == []


def test_run_session_drains_send_queue():
    a = _adapter()
    a._send_queue.put(wecom_mod._build_send_frame("alice", "queued"))
    ws, _ = _drive_session(a, [
        {"errcode": 0, "headers": {"req_id": "aibot_subscribe_1"}},
        None,  # idle tick — gives the loop a chance to drain
        "CLOSE",
    ])
    # Both subscribe + queued send should appear in outbound transcript.
    cmds = [json.loads(s)["cmd"] for s in ws.sent]
    assert "aibot_subscribe" in cmds
    assert "aibot_send_msg" in cmds
