"""Tests for librefang.sidecar.adapters.wechat.

Deterministic, no network — urllib is monkeypatched via the shared
_FakeUrlopen helper.
"""
from __future__ import annotations

import json
import os

import pytest

from _sidecar_fakes import _FakeResp, _FakeUrlopen, _HdrShim  # noqa: F401

os.environ.setdefault("WECHAT_BOT_TOKEN", "tok_test")
from librefang.sidecar.adapters import wechat as wc  # noqa: E402


def _adapter(**env):
    defaults = {
        "WECHAT_BOT_TOKEN": "tok_test",
        "WECHAT_ALLOWED_USERS": "",
        "WECHAT_ACCOUNT_ID": "",
        "WECHAT_INITIAL_BACKOFF_SECS": "",
        "WECHAT_MAX_BACKOFF_SECS": "",
        "WECHAT_API_BASE_OVERRIDE": "",
    }
    for k, v in defaults.items():
        os.environ[k] = env.get(k, v)
    return wc.WeChatAdapter()


# ---- env handling ----------------------------------------------------


def test_default_env_construction():
    a = _adapter()
    assert a.bot_token == "tok_test"
    assert a.allowed_users == []
    assert a.account_id is None
    assert a.initial_backoff_secs == 2.0
    assert a.max_backoff_secs == 60.0
    assert a.api_base == wc.ILINK_BASE


def test_bot_token_empty_means_qr_login():
    a = _adapter(WECHAT_BOT_TOKEN="")
    assert a.bot_token is None


def test_allowed_users_csv():
    a = _adapter(WECHAT_ALLOWED_USERS="alice@im.wechat, bob@im.wechat ,, charlie@im.wechat")
    assert a.allowed_users == [
        "alice@im.wechat", "bob@im.wechat", "charlie@im.wechat",
    ]


def test_account_id_passthrough():
    a = _adapter(WECHAT_ACCOUNT_ID="prod-bot")
    assert a.account_id == "prod-bot"


def test_backoff_overrides():
    a = _adapter(
        WECHAT_INITIAL_BACKOFF_SECS="5",
        WECHAT_MAX_BACKOFF_SECS="120",
    )
    assert a.initial_backoff_secs == 5.0
    assert a.max_backoff_secs == 120.0


def test_backoff_negative_uses_default():
    a = _adapter(WECHAT_INITIAL_BACKOFF_SECS="-1")
    assert a.initial_backoff_secs == 2.0


def test_backoff_garbage_uses_default():
    a = _adapter(WECHAT_INITIAL_BACKOFF_SECS="not-a-number")
    assert a.initial_backoff_secs == 2.0


def test_api_base_override():
    a = _adapter(WECHAT_API_BASE_OVERRIDE="https://mock.local")
    assert a.api_base == "https://mock.local"


# ---- generate_wechat_uin -------------------------------------------


def test_generate_wechat_uin_returns_base64_string():
    uin = wc.generate_wechat_uin()
    assert isinstance(uin, str)
    import base64
    decoded = base64.b64decode(uin)
    n = int(decoded.decode("ascii"))
    assert 0 <= n < 2 ** 32


def test_generate_wechat_uin_changes_each_call():
    uins = {wc.generate_wechat_uin() for _ in range(10)}
    # Vanishingly unlikely to repeat in 10 draws over a 4 billion-byte
    # space; if all 10 were equal something's wrong with the RNG.
    assert len(uins) > 5


# ---- parse_wechat_msg pure-function path ---------------------------


def _text_msg(
    *,
    text="hello world",
    from_user_id="alice@im.wechat",
    msg_id="msg_42",
    context_token="ctx_xyz",
):
    return {
        "from_user_id": from_user_id,
        "to_user_id": "bot@im.bot",
        "context_token": context_token,
        "message_type": 1,
        "msg_id": msg_id,
        "from_user_name": "Alice",
        "item_list": [{
            "type": wc.ITEM_TYPE_TEXT,
            "text_item": {"text": text},
        }],
    }


def test_parse_text_message_basic():
    ev = wc.parse_wechat_msg(_text_msg())
    assert ev is not None
    params = ev["params"]
    assert params["user_id"] == "alice@im.wechat"
    assert params["user_name"] == "Alice"
    assert params["channel_id"] == "alice@im.wechat"
    assert params["message_id"] == "msg_42"
    assert params["content"]["Text"] == "hello world"
    assert params["text"] == "hello world"
    meta = params["metadata"]
    assert meta["context_token"] == "ctx_xyz"
    assert meta["to_user_id"] == "bot@im.bot"


def test_parse_self_skip_bot_origin():
    """Messages whose from_user_id ends with @im.bot are bot-originated
    (the bot's own replies looped back). They must be silently dropped
    (mirror wechat.rs:401-403)."""
    msg = _text_msg(from_user_id="me@im.bot")
    assert wc.parse_wechat_msg(msg) is None


def test_parse_missing_from_user_id_drops():
    msg = _text_msg()
    del msg["from_user_id"]
    assert wc.parse_wechat_msg(msg) is None


def test_parse_empty_text_drops():
    assert wc.parse_wechat_msg(_text_msg(text="")) is None


def test_parse_no_items_drops():
    msg = _text_msg()
    msg["item_list"] = []
    assert wc.parse_wechat_msg(msg) is None


def test_parse_unsupported_item_type_drops():
    msg = _text_msg()
    msg["item_list"][0] = {"type": 99, "weird_item": {}}
    assert wc.parse_wechat_msg(msg) is None


def test_parse_image_message():
    msg = _text_msg()
    msg["item_list"][0] = {
        "type": wc.ITEM_TYPE_IMAGE,
        "image_item": {"url": "https://cdn.example.com/img.jpg"},
    }
    ev = wc.parse_wechat_msg(msg)
    assert ev["params"]["content"]["Image"]["url"] == "https://cdn.example.com/img.jpg"


def test_parse_image_falls_back_to_cdn_url():
    msg = _text_msg()
    msg["item_list"][0] = {
        "type": wc.ITEM_TYPE_IMAGE,
        "image_item": {"cdn_url": "https://cdn2.example.com/img.jpg"},
    }
    ev = wc.parse_wechat_msg(msg)
    assert ev["params"]["content"]["Image"]["url"] == "https://cdn2.example.com/img.jpg"


def test_parse_voice_message():
    msg = _text_msg()
    msg["item_list"][0] = {
        "type": wc.ITEM_TYPE_VOICE,
        "voice_item": {
            "url": "https://cdn.example.com/v.amr",
            "duration": 12,
        },
    }
    ev = wc.parse_wechat_msg(msg)
    assert ev["params"]["content"]["Voice"]["duration_seconds"] == 12


def test_parse_file_message():
    msg = _text_msg()
    msg["item_list"][0] = {
        "type": wc.ITEM_TYPE_FILE,
        "file_item": {
            "url": "https://cdn.example.com/doc.pdf",
            "file_name": "report.pdf",
        },
    }
    ev = wc.parse_wechat_msg(msg)
    assert ev["params"]["content"]["File"]["filename"] == "report.pdf"


def test_parse_video_message():
    msg = _text_msg()
    msg["item_list"][0] = {
        "type": wc.ITEM_TYPE_VIDEO,
        "video_item": {
            "url": "https://cdn.example.com/clip.mp4",
            "duration": 30,
        },
    }
    ev = wc.parse_wechat_msg(msg)
    assert ev["params"]["content"]["Video"]["duration_seconds"] == 30


def test_parse_falls_back_to_svr_msg_id():
    msg = _text_msg(msg_id="")
    msg["svr_msg_id"] = "svr_99"
    ev = wc.parse_wechat_msg(msg)
    assert ev["params"]["message_id"] == "svr_99"


def test_parse_falls_back_to_user_id_when_no_display_name():
    msg = _text_msg()
    del msg["from_user_name"]
    ev = wc.parse_wechat_msg(msg)
    assert ev["params"]["user_name"] == "alice@im.wechat"


def test_parse_account_id_injection():
    ev = wc.parse_wechat_msg(_text_msg(), account_id="prod")
    assert ev["params"]["metadata"]["account_id"] == "prod"


def test_parse_no_account_id_omits_field():
    ev = wc.parse_wechat_msg(_text_msg())
    assert "account_id" not in ev["params"]["metadata"]


def test_parse_non_dict_input_returns_none():
    assert wc.parse_wechat_msg(None) is None
    assert wc.parse_wechat_msg("not-a-dict") is None
    assert wc.parse_wechat_msg(42) is None


# ---- _send_text via mocked urlopen ---------------------------------


def test_send_text_basic(monkeypatch):
    fake = _FakeUrlopen([(200, {"errcode": 0})])
    monkeypatch.setattr(wc, "_http_request", lambda url, **kw: (200, {"errcode": 0}, b"", {}))
    a = _adapter()
    a._send_text("alice@im.wechat", "ctx_xyz", "hello")


def test_send_text_chunks_long_messages(monkeypatch):
    monkeypatch.setattr(wc, "MAX_MESSAGE_LEN", 5)
    calls: list = []

    def _fake_http(url, **kw):
        body = kw.get("body")
        calls.append(json.loads(body.decode("utf-8")))
        return (200, {"errcode": 0}, b"", {})

    monkeypatch.setattr(wc, "_http_request", _fake_http)
    a = _adapter()
    a._send_text("alice@im.wechat", "ctx", "abcdefghijk")  # 11 chars at limit=5 → 3 chunks
    assert len(calls) >= 2
    # Each chunk gets a fresh client_id (idempotency)
    cids = {c["msg"]["client_id"] for c in calls}
    assert len(cids) == len(calls)


def test_send_text_empty_drops(monkeypatch):
    calls: list = []

    def _fake_http(url, **kw):
        calls.append(url)
        return (200, {}, b"", {})

    monkeypatch.setattr(wc, "_http_request", _fake_http)
    a = _adapter()
    a._send_text("alice@im.wechat", "ctx", "")
    assert calls == []


def test_send_text_no_token_raises(monkeypatch):
    a = _adapter(WECHAT_BOT_TOKEN="")
    with pytest.raises(RuntimeError, match="not logged in"):
        a._send_text("alice@im.wechat", "ctx", "hi")


def test_send_text_http_error_raises(monkeypatch):
    monkeypatch.setattr(
        wc, "_http_request",
        lambda url, **kw: (500, None, b"server boom", {}),
    )
    a = _adapter()
    with pytest.raises(RuntimeError, match="sendmessage error"):
        a._send_text("alice@im.wechat", "ctx", "hi")


def test_send_text_429_retries_once(monkeypatch):
    """First 429 sleeps + retries; second response succeeds."""
    responses = [
        (429, None, b"rate limited", {"retry-after": "1"}),
        (200, {"errcode": 0}, b"", {}),
    ]
    calls: list = []

    def _fake_http(url, **kw):
        calls.append(url)
        return responses.pop(0)

    monkeypatch.setattr(wc, "_http_request", _fake_http)
    monkeypatch.setattr(wc, "_parse_retry_after", lambda h, **kw: 0.0)
    a = _adapter()
    a._send_text("alice@im.wechat", "ctx", "hi")
    assert len(calls) == 2  # one 429 + one success


def test_send_text_request_body_shape(monkeypatch):
    captured: list = []

    def _fake_http(url, **kw):
        captured.append(json.loads(kw["body"].decode("utf-8")))
        return (200, {"errcode": 0}, b"", {})

    monkeypatch.setattr(wc, "_http_request", _fake_http)
    a = _adapter()
    a._send_text("alice@im.wechat", "ctx_xyz", "hi")
    body = captured[0]
    assert body["msg"]["to_user_id"] == "alice@im.wechat"
    assert body["msg"]["context_token"] == "ctx_xyz"
    assert body["msg"]["item_list"][0]["type"] == wc.ITEM_TYPE_TEXT
    assert body["msg"]["item_list"][0]["text_item"]["text"] == "hi"
    assert body["msg"]["message_type"] == 2
    assert body["msg"]["message_state"] == 2
    assert body["base_info"]["channel_version"] == wc.CHANNEL_VERSION


# ---- on_send dispatch ----------------------------------------------


def _send_cmd(channel_id="alice@im.wechat", text="hi", content=None,
              thread_id=None, user=None):
    from librefang.sidecar.protocol import Send
    return Send(channel_id, text, content, thread_id, user or {})


@pytest.mark.asyncio
async def test_on_send_basic_text(monkeypatch):
    sent: list = []

    def _fake_http(url, **kw):
        sent.append(json.loads(kw["body"].decode("utf-8")))
        return (200, {"errcode": 0}, b"", {})

    monkeypatch.setattr(wc, "_http_request", _fake_http)
    a = _adapter()
    await a.on_send(_send_cmd(text="hello", content={"Text": "hello"}))
    assert sent[0]["msg"]["item_list"][0]["text_item"]["text"] == "hello"


@pytest.mark.asyncio
async def test_on_send_uses_cached_context_token(monkeypatch):
    sent: list = []

    def _fake_http(url, **kw):
        sent.append(json.loads(kw["body"].decode("utf-8")))
        return (200, {"errcode": 0}, b"", {})

    monkeypatch.setattr(wc, "_http_request", _fake_http)
    a = _adapter()
    a._user_context_tokens["alice@im.wechat"] = "ctx_persisted"
    await a.on_send(_send_cmd(text="hi", content={"Text": "hi"}))
    assert sent[0]["msg"]["context_token"] == "ctx_persisted"


@pytest.mark.asyncio
async def test_on_send_recovers_context_token_from_user_librefang_user_when_cache_cold(
    monkeypatch,
):
    """Regression guard for sidecar-restart fragility: the in-memory
    ``_user_context_tokens`` cache vanishes on restart, so the bot's
    first reply after restart would otherwise post with an empty
    ``context_token`` (iLink may reject or post out-of-thread).
    The bridge round-trips ``ChannelUser.librefang_user`` bytewise,
    so the parse-side stash of ``context_token`` there is the
    restart-survivable fallback. This test simulates that:
    in-memory cache empty (post-restart), but the daemon delivers
    ``cmd.user.librefang_user`` from the inbound that triggered the
    reply — on_send MUST recover from there."""
    sent: list = []

    def _fake_http(url, **kw):
        sent.append(json.loads(kw["body"].decode("utf-8")))
        return (200, {"errcode": 0}, b"", {})

    monkeypatch.setattr(wc, "_http_request", _fake_http)
    a = _adapter()
    # Simulate post-restart state: in-memory cache empty.
    assert a._user_context_tokens == {}
    await a.on_send(_send_cmd(
        text="hi",
        content={"Text": "hi"},
        user={
            "platform_id": "alice@im.wechat",
            "librefang_user": "ctx_from_inbound",
        },
    ))
    assert sent[0]["msg"]["context_token"] == "ctx_from_inbound", \
        "on_send must recover context_token from cmd.user.librefang_user " \
        "when the in-memory cache is empty (post-restart scenario)"


@pytest.mark.asyncio
async def test_on_send_ignores_url_shaped_librefang_user(monkeypatch):
    """``librefang_user`` is shared across channels — dingtalk puts a
    sessionWebhook URL there, telegram puts ``@username``. Must reject
    cross-channel pollution before sending a corrupted context_token."""
    sent: list = []

    def _fake_http(url, **kw):
        sent.append(json.loads(kw["body"].decode("utf-8")))
        return (200, {"errcode": 0}, b"", {})

    monkeypatch.setattr(wc, "_http_request", _fake_http)
    a = _adapter()
    await a.on_send(_send_cmd(
        text="hi",
        content={"Text": "hi"},
        user={
            "platform_id": "alice@im.wechat",
            "librefang_user": "https://oapi.dingtalk.com/sb?s=42",
        },
    ))
    # URL-shaped librefang_user rejected → context_token empty.
    assert sent[0]["msg"]["context_token"] == ""


@pytest.mark.asyncio
async def test_on_send_empty_user_drops(monkeypatch):
    calls: list = []
    monkeypatch.setattr(
        wc, "_http_request",
        lambda url, **kw: (calls.append(url), (200, {}, b"", {}))[1],
    )
    a = _adapter()
    await a.on_send(_send_cmd(channel_id="", user={}))
    assert calls == []


@pytest.mark.asyncio
async def test_on_send_falls_back_to_user_platform_id(monkeypatch):
    sent: list = []

    def _fake_http(url, **kw):
        sent.append(json.loads(kw["body"].decode("utf-8")))
        return (200, {"errcode": 0}, b"", {})

    monkeypatch.setattr(wc, "_http_request", _fake_http)
    a = _adapter()
    await a.on_send(_send_cmd(
        channel_id="", text="hi", content={"Text": "hi"},
        user={"platform_id": "alice@im.wechat"},
    ))
    assert sent[0]["msg"]["to_user_id"] == "alice@im.wechat"


@pytest.mark.asyncio
async def test_on_send_empty_text_drops(monkeypatch):
    calls: list = []
    monkeypatch.setattr(
        wc, "_http_request",
        lambda url, **kw: (calls.append(url), (200, {}, b"", {}))[1],
    )
    a = _adapter()
    await a.on_send(_send_cmd(text="", content={"Text": ""}))
    assert calls == []


@pytest.mark.asyncio
async def test_on_send_unsupported_content_sends_placeholder(monkeypatch):
    sent: list = []

    def _fake_http(url, **kw):
        sent.append(json.loads(kw["body"].decode("utf-8")))
        return (200, {"errcode": 0}, b"", {})

    monkeypatch.setattr(wc, "_http_request", _fake_http)
    a = _adapter()
    await a.on_send(_send_cmd(
        text="", content={"Image": {"url": "https://x"}},
    ))
    text = sent[0]["msg"]["item_list"][0]["text_item"]["text"]
    assert "Unsupported content type" in text


# ---- _dispatch_messages integration --------------------------------


def test_dispatch_messages_emits_and_stashes_context():
    emitted: list = []
    a = _adapter()
    msgs = [_text_msg(from_user_id="alice@im.wechat", context_token="ctx1")]
    a._dispatch_messages(msgs, lambda ev: emitted.append(ev))
    assert len(emitted) == 1
    assert a._user_context_tokens["alice@im.wechat"] == "ctx1"


def test_dispatch_messages_dedupes_by_msg_id():
    emitted: list = []
    a = _adapter()
    msg = _text_msg(msg_id="dup_1")
    a._dispatch_messages([msg], lambda ev: emitted.append(ev))
    a._dispatch_messages([msg], lambda ev: emitted.append(ev))
    assert len(emitted) == 1


def test_dispatch_messages_filters_by_allowlist():
    emitted: list = []
    a = _adapter(WECHAT_ALLOWED_USERS="bob@im.wechat")
    msgs = [_text_msg(from_user_id="alice@im.wechat")]
    a._dispatch_messages(msgs, lambda ev: emitted.append(ev))
    assert emitted == []


def test_dispatch_messages_allowlist_accepts_listed_user():
    emitted: list = []
    a = _adapter(WECHAT_ALLOWED_USERS="alice@im.wechat")
    msgs = [_text_msg(from_user_id="alice@im.wechat")]
    a._dispatch_messages(msgs, lambda ev: emitted.append(ev))
    assert len(emitted) == 1


def test_dispatch_messages_empty_context_token_does_not_overwrite():
    """A subsequent inbound with `context_token == ""` (rare but
    possible for iLink system events) must NOT clobber the real
    context_token from an earlier user message. Otherwise the next
    outbound reply lands without threading."""
    emitted: list = []
    a = _adapter()
    # First message stashes a real ctx.
    first = _text_msg(from_user_id="alice@im.wechat", context_token="real_ctx",
                       msg_id="m1")
    a._dispatch_messages([first], lambda ev: emitted.append(ev))
    assert a._user_context_tokens["alice@im.wechat"] == "real_ctx"
    # Second message from same user has empty ctx — must not clobber.
    second = _text_msg(from_user_id="alice@im.wechat", context_token="",
                        msg_id="m2")
    a._dispatch_messages([second], lambda ev: emitted.append(ev))
    assert a._user_context_tokens["alice@im.wechat"] == "real_ctx", (
        "empty context_token must not overwrite the previously-stored value"
    )


def test_dispatch_messages_skips_bot_origin():
    emitted: list = []
    a = _adapter()
    msgs = [_text_msg(from_user_id="self@im.bot")]
    a._dispatch_messages(msgs, lambda ev: emitted.append(ev))
    assert emitted == []


def test_dispatch_messages_injects_account_id():
    emitted: list = []
    a = _adapter(WECHAT_ACCOUNT_ID="prod-bot")
    msgs = [_text_msg()]
    a._dispatch_messages(msgs, lambda ev: emitted.append(ev))
    assert emitted[0]["params"]["metadata"]["account_id"] == "prod-bot"


# ---- QR login flow --------------------------------------------------


def test_qr_login_happy_path(monkeypatch):
    """Single status poll returns `confirmed` + a bot_token."""
    responses = [
        (200, {"qrcode": "QR_DATA_BLOB"}, b"", {}),
        (200, {"status": "confirmed", "bot_token": "new_tok"}, b"", {}),
    ]

    def _fake_http(url, **kw):
        return responses.pop(0)

    monkeypatch.setattr(wc, "_http_request", _fake_http)
    a = _adapter(WECHAT_BOT_TOKEN="")
    token = a._qr_login()
    assert token == "new_tok"


def test_qr_login_expired_raises(monkeypatch):
    responses = [
        (200, {"qrcode": "QR"}, b"", {}),
        (200, {"status": "expired"}, b"", {}),
    ]
    monkeypatch.setattr(wc, "_http_request", lambda url, **kw: responses.pop(0))
    a = _adapter(WECHAT_BOT_TOKEN="")
    with pytest.raises(RuntimeError, match="expired"):
        a._qr_login()


def test_qr_login_no_qrcode_in_response(monkeypatch):
    monkeypatch.setattr(
        wc, "_http_request",
        lambda url, **kw: (200, {}, b"", {}),
    )
    a = _adapter(WECHAT_BOT_TOKEN="")
    with pytest.raises(RuntimeError, match="missing 'qrcode'"):
        a._qr_login()


def test_send_text_401_clears_token(monkeypatch):
    """A 401/403 on sendmessage clears the cached token so the
    next poll-loop iteration triggers QR re-login. The send itself
    still raises — caller decides whether to retry."""
    monkeypatch.setattr(
        wc, "_http_request",
        lambda url, **kw: (401, None, b"token expired", {}),
    )
    a = _adapter()
    assert a._get_token() == "tok_test"
    with pytest.raises(RuntimeError, match="auth rejected"):
        a._send_text("alice@im.wechat", "ctx", "hi")
    assert a._get_token() is None


def test_poll_updates_401_clears_token(monkeypatch):
    """401/403 from /getupdates means the persisted token is dead.
    The adapter MUST clear the cached token so the next poll-loop
    iteration re-runs the QR flow instead of looping forever on a
    rejected request."""
    monkeypatch.setattr(
        wc, "_http_request",
        lambda url, **kw: (401, None, b"token expired", {}),
    )
    a = _adapter()
    assert a._get_token() == "tok_test"  # primed
    with pytest.raises(RuntimeError, match="auth rejected"):
        a._poll_updates("tok_test")
    # Cache cleared so the next loop iteration drops into QR re-login.
    assert a._get_token() is None


def test_poll_updates_403_also_clears_token(monkeypatch):
    """Same as 401 — both auth-rejection statuses must clear cache."""
    monkeypatch.setattr(
        wc, "_http_request",
        lambda url, **kw: (403, None, b"forbidden", {}),
    )
    a = _adapter()
    with pytest.raises(RuntimeError, match="auth rejected"):
        a._poll_updates("tok_test")
    assert a._get_token() is None


def test_qr_login_status_non_200_continues(monkeypatch):
    """Transient non-200 on status poll keeps trying; eventual
    `confirmed` succeeds."""
    responses = [
        (200, {"qrcode": "QR"}, b"", {}),
        (500, None, b"oops", {}),
        (200, {"status": "confirmed", "bot_token": "tok"}, b"", {}),
    ]
    monkeypatch.setattr(wc, "_http_request", lambda url, **kw: responses.pop(0))
    a = _adapter(WECHAT_BOT_TOKEN="")
    # Force backoff to be small so the test runs quickly.
    a.initial_backoff_secs = 0.01
    assert a._qr_login() == "tok"


# ---- schema + capabilities ------------------------------------------


def test_schema_exposes_required_envs():
    schema = wc.WeChatAdapter.SCHEMA.to_dict()
    keys = {f["key"] for f in schema["fields"]}
    expected = {
        "WECHAT_BOT_TOKEN",
        "WECHAT_ALLOWED_USERS",
        "WECHAT_ACCOUNT_ID",
        "WECHAT_INITIAL_BACKOFF_SECS",
        "WECHAT_MAX_BACKOFF_SECS",
    }
    assert expected.issubset(keys)
    # WECHAT_BOT_TOKEN is a secret so QR-login-acquired tokens land in
    # secrets.env, not config.toml.
    secrets = {f["key"] for f in schema["fields"] if f["type"] == "secret"}
    assert "WECHAT_BOT_TOKEN" in secrets


def test_capabilities_declares_typing():
    # iLink's /sendtyping endpoint is the only out-of-band surface
    # the Rust adapter exposed beyond send; we re-claim it so the
    # daemon routes TypingCmd to the sidecar instead of silently
    # dropping. (No reaction / thread / streaming — iLink has no
    # analogue.)
    assert "typing" in wc.WeChatAdapter.capabilities


# ---- typing -------------------------------------------------------


def test_send_typing_basic(monkeypatch):
    sent: list = []

    def _fake_http(url, **kw):
        sent.append((url, json.loads(kw["body"].decode("utf-8"))))
        return (200, {"errcode": 0}, b"", {})

    monkeypatch.setattr(wc, "_http_request", _fake_http)
    a = _adapter()
    a._typing_ticket = "tk_typing"
    a._send_typing("alice@im.wechat")
    assert len(sent) == 1
    assert "/ilink/bot/sendtyping" in sent[0][0]
    body = sent[0][1]
    assert body == {"to_user_id": "alice@im.wechat", "typing_ticket": "tk_typing"}


def test_send_typing_no_ticket_no_call(monkeypatch):
    """No `typing_ticket` cached yet → silent no-op (matches Rust at
    wechat.rs:786-789)."""
    sent: list = []
    monkeypatch.setattr(
        wc, "_http_request",
        lambda url, **kw: (sent.append(url), (200, {}, b"", {}))[1],
    )
    a = _adapter()
    a._typing_ticket = None
    a._send_typing("alice@im.wechat")
    assert sent == []


def test_send_typing_no_token_no_call(monkeypatch):
    sent: list = []
    monkeypatch.setattr(
        wc, "_http_request",
        lambda url, **kw: (sent.append(url), (200, {}, b"", {}))[1],
    )
    a = _adapter(WECHAT_BOT_TOKEN="")
    a._typing_ticket = "tk"
    a._send_typing("alice@im.wechat")
    assert sent == []


def test_send_typing_http_error_swallowed(monkeypatch):
    """Sendtyping is best-effort — a non-2xx must not crash on_command."""
    monkeypatch.setattr(
        wc, "_http_request",
        lambda url, **kw: (500, None, b"oops", {}),
    )
    a = _adapter()
    a._typing_ticket = "tk"
    # Must not raise.
    a._send_typing("alice@im.wechat")


@pytest.mark.asyncio
async def test_on_command_routes_typing_cmd(monkeypatch):
    sent: list = []

    def _fake_http(url, **kw):
        sent.append((url, json.loads(kw["body"].decode("utf-8"))))
        return (200, {}, b"", {})

    monkeypatch.setattr(wc, "_http_request", _fake_http)
    a = _adapter()
    a._typing_ticket = "tk"
    from librefang.sidecar.protocol import TypingCmd
    cmd = TypingCmd(channel_id="alice@im.wechat")
    await a.on_command(cmd)
    assert any("/ilink/bot/sendtyping" in c[0] for c in sent)


@pytest.mark.asyncio
async def test_on_command_typing_empty_user_drops(monkeypatch):
    sent: list = []
    monkeypatch.setattr(
        wc, "_http_request",
        lambda url, **kw: (sent.append(url), (200, {}, b"", {}))[1],
    )
    a = _adapter()
    a._typing_ticket = "tk"
    from librefang.sidecar.protocol import TypingCmd
    cmd = TypingCmd(channel_id="")
    await a.on_command(cmd)
    assert sent == []


# ---- QR-login event emission (PR: dashboard QR rewire) ---------------
#
# The sidecar drives the QR lifecycle itself and emits `qr_ready` /
# `qr_status` so the daemon can cache `ChannelStatus.qr` for the
# dashboard's `GET /api/channels/{name}/qr` projection. These tests
# pin the contract: the right events fire on the right transitions,
# with the right payloads (qrcode string surfaced, terminal failure
# / expiry message populated, the raw token never on the protocol
# event — only a short SHA-256 fingerprint is logged at DEBUG, see
# `protocol.qr_status` docstring and issue #5543).


def _make_qr_responses(*pairs):
    """Build a counted-response queue for `_http_request`.

    Each `pairs` entry is `(status, body_dict)`; bodies are encoded as
    JSON so the existing `_http_request` JSON-decode path in wechat.py
    behaves the same as a real iLink response."""
    queue = list(pairs)

    def _fake(url, **kw):
        if not queue:
            raise RuntimeError(f"qr fake exhausted: {url}")
        status, body = queue.pop(0)
        raw = json.dumps(body).encode("utf-8")
        return (status, body, raw, {})

    return _fake


def test_qr_login_emits_ready_then_confirmed_without_token_on_wire(monkeypatch):
    monkeypatch.setattr(
        wc, "_http_request",
        _make_qr_responses(
            # iLink returns the opaque poll token AND the scannable image
            # content; the QR must encode the latter.
            (200, {"qrcode": "opaque-token",
                   "qrcode_img_content": "https://login.weixin.qq.com/l/AbC123"}),
            (200, {"status": "confirmed", "bot_token": "tok_live"}),  # poll → confirmed
        ),
    )
    a = _adapter(WECHAT_BOT_TOKEN="")
    captured: list = []
    token = a._qr_login(emit=captured.append)

    # The token is returned to the in-process caller (used to drive
    # the rest of the wechat session) but MUST NOT appear in the
    # outbound protocol event — that's the safety invariant.
    assert token == "tok_live"

    methods = [e["method"] for e in captured]
    assert methods == ["qr_ready", "qr_status"], f"unexpected order: {methods}"

    ready = captured[0]["params"]
    # The scannable payload must be `qrcode_img_content` (what the WeChat app
    # decodes on scan), NOT the opaque `qrcode` poll token. Encoding the token
    # makes WeChat show plain text instead of a login prompt — the #1560 / #1572
    # regression the sidecar migration (#5421) re-introduced.
    assert ready["qr_code"] == "https://login.weixin.qq.com/l/AbC123"
    # Sidecar passes expires_at as ISO 8601; format is wall-clock
    # derived, so just shape-check.
    assert "expires_at" in ready and "T" in ready["expires_at"]

    confirmed = captured[1]["params"]
    assert confirmed["status"] == "confirmed"
    assert "bot_token" not in confirmed, (
        "bot_token MUST NOT ride the qr_status event — the current "
        "configure endpoint would wipe other schema-managed fields on "
        "a partial save. See `protocol.qr_status` docstring."
    )
    # The confirmed message must guide the operator to the manual
    # secrets.env step in lieu of auto-persist.
    assert "WECHAT_BOT_TOKEN" in (confirmed.get("message") or "")
    assert "secrets.env" in (confirmed.get("message") or "")


def test_qr_login_qr_payload_falls_back_to_token_when_img_content_absent(
    monkeypatch, capsys
):
    # Degraded path: if iLink ever omits `qrcode_img_content`, the sidecar must
    # still emit a `qr_ready` (rather than crash) and warn the operator that the
    # rendered QR may not be scannable, falling back to the opaque token.
    monkeypatch.setattr(
        wc, "_http_request",
        _make_qr_responses(
            (200, {"qrcode": "opaque-token"}),  # no qrcode_img_content
            (200, {"status": "confirmed", "bot_token": "tok_live"}),
        ),
    )
    a = _adapter(WECHAT_BOT_TOKEN="")
    captured: list = []
    token = a._qr_login(emit=captured.append)
    assert token == "tok_live"

    ready = captured[0]["params"]
    assert ready["qr_code"] == "opaque-token"  # degraded fallback
    assert "qrcode_img_content" in capsys.readouterr().err


def test_qr_login_emits_expired_on_platform_expiry(monkeypatch):
    monkeypatch.setattr(
        wc, "_http_request",
        _make_qr_responses(
            (200, {"qrcode": "opaque-token",
                   "qrcode_img_content": "https://login.weixin.qq.com/l/AbC123"}),
            (200, {"status": "expired"}),
        ),
    )
    a = _adapter(WECHAT_BOT_TOKEN="")
    captured: list = []
    with pytest.raises(RuntimeError):
        a._qr_login(emit=captured.append)

    statuses = [
        e["params"]["status"] for e in captured if e["method"] == "qr_status"
    ]
    assert statuses == ["expired"]
    expired = next(e for e in captured if e["method"] == "qr_status")
    assert expired["params"].get("bot_token") is None


def test_qr_login_emits_failed_on_qrcode_request_error(monkeypatch):
    # iLink returns a non-200 — sidecar must emit a terminal `failed`
    # rather than just raise into the void (which would leave the
    # dashboard's `qr` projection stuck on `pending` forever).
    monkeypatch.setattr(
        wc, "_http_request",
        _make_qr_responses(
            (500, {"error": "internal"}),
        ),
    )
    a = _adapter(WECHAT_BOT_TOKEN="")
    captured: list = []
    with pytest.raises(RuntimeError):
        a._qr_login(emit=captured.append)

    assert captured, "must emit at least one event on failure"
    methods = [e["method"] for e in captured]
    assert methods == ["qr_status"], (
        f"failure pre-ready must emit ONLY qr_status (not qr_ready); "
        f"got: {methods}"
    )
    assert captured[-1]["params"]["status"] == "failed"


def test_qr_login_never_logs_full_bot_token(monkeypatch, capsys):
    # Regression for #5543: the captured bot_token MUST NOT appear in
    # any stderr log line (INFO or DEBUG). Only a short SHA-256
    # fingerprint may surface for cross-session correlation.
    import hashlib

    secret = "tok_live_secret_value"
    fp = hashlib.sha256(secret.encode("utf-8")).hexdigest()[:8]
    monkeypatch.setattr(
        wc, "_http_request",
        _make_qr_responses(
            (200, {"qrcode": "opaque-token",
                   "qrcode_img_content": "https://login.weixin.qq.com/l/AbC123"}),
            (200, {"status": "confirmed", "bot_token": secret}),
        ),
    )
    a = _adapter(WECHAT_BOT_TOKEN="")
    captured: list = []
    token = a._qr_login(emit=captured.append)
    assert token == secret  # in-process return path unchanged

    err = capsys.readouterr().err
    assert secret not in err, (
        "full bot_token leaked into stderr logs — see issue #5543"
    )
    assert fp in err, (
        "SHA-256 fingerprint missing from DEBUG log — operator loses "
        "cross-session correlation signal"
    )
    # Protocol event is the second guardrail.
    confirmed = next(e for e in captured if e["method"] == "qr_status")
    assert secret not in json.dumps(confirmed)


def test_qr_login_without_emit_still_returns_token(monkeypatch):
    # Backward-compat: the existing pytest paths that drive `_qr_login`
    # directly without an emit callback must continue to work — the
    # emit parameter is `Optional[Callable[[dict], None]] = None`.
    monkeypatch.setattr(
        wc, "_http_request",
        _make_qr_responses(
            (200, {"qrcode": "opaque-token",
                   "qrcode_img_content": "https://login.weixin.qq.com/l/AbC123"}),
            (200, {"status": "confirmed", "bot_token": "tok_live"}),
        ),
    )
    a = _adapter(WECHAT_BOT_TOKEN="")
    token = a._qr_login()  # no emit
    assert token == "tok_live"
