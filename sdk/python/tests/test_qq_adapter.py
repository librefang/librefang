"""Tests for librefang.sidecar.adapters.qq.

Deterministic, no network: urllib is monkeypatched for REST, the WS
client is replaced with an in-memory transcript so the producer can
drive HELLO/IDENTIFY/READY/HEARTBEAT/DISPATCH without binding a real
socket. Asserts the sidecar preserves the in-process Rust
``librefang-channels::qq`` adapter's behaviour plus the four
improvements documented in the module header (reply-context
round-trip, msg.id dedupe, 429 Retry-After, explicit HTTP timeouts).
"""

import io
import json
import os
import time
import urllib.error

import pytest


os.environ.setdefault("QQ_APP_ID", "test-app")
os.environ.setdefault("QQ_APP_SECRET", "test-secret")
os.environ.setdefault("QQ_API_BASE", "https://qq.test")
os.environ.setdefault("QQ_TOKEN_URL", "https://qq-token.test/getAppAccessToken")
from librefang.sidecar.adapters import qq as qq_mod  # noqa: E402

from _sidecar_fakes import _FakeResp, _FakeUrlopen, _HdrShim


# ---- _FakeUrlopen scaffolding ----------------------------------------


def _adapter(**env):
    defaults = {
        "QQ_APP_ID": "test-app",
        "QQ_APP_SECRET": "test-secret",
        "QQ_ALLOWED_USERS": "",
        "QQ_ACCOUNT_ID": "",
        "QQ_INTENTS": "",
        "QQ_API_BASE": "https://qq.test",
        "QQ_TOKEN_URL": "https://qq-token.test/getAppAccessToken",
        "QQ_WS_URL": "",
    }
    for k, v in defaults.items():
        os.environ[k] = env.get(k, v)
    return qq_mod.QqAdapter()


# ---- env handling ----------------------------------------------------


def test_default_env_construction():
    a = _adapter()
    assert a.app_id == "test-app"
    assert a.app_secret == "test-secret"
    assert a.allowed_users == []
    assert a.account_id is None
    assert a.intents == qq_mod.DEFAULT_INTENTS
    assert a.api_base == "https://qq.test"
    assert a.token_url == "https://qq-token.test/getAppAccessToken"


def test_api_base_trailing_slash_stripped():
    a = _adapter(QQ_API_BASE="https://qq.test/")
    assert a.api_base == "https://qq.test"


def test_allowed_users_csv_split():
    a = _adapter(QQ_ALLOWED_USERS="open-1, open-2 ,, open-3")
    assert a.allowed_users == ["open-1", "open-2", "open-3"]


def test_account_id_passthrough():
    a = _adapter(QQ_ACCOUNT_ID="prod-bot")
    assert a.account_id == "prod-bot"


def test_account_id_empty_is_none():
    a = _adapter(QQ_ACCOUNT_ID="")
    assert a.account_id is None


def test_intents_decimal_override():
    a = _adapter(QQ_INTENTS="42")
    assert a.intents == 42


def test_intents_hex_override():
    a = _adapter(QQ_INTENTS="0x80")
    assert a.intents == 0x80


def test_intents_garbage_falls_back_to_default():
    a = _adapter(QQ_INTENTS="not-a-number")
    assert a.intents == qq_mod.DEFAULT_INTENTS


def test_missing_app_id_exits_2():
    os.environ["QQ_APP_ID"] = ""
    os.environ["QQ_APP_SECRET"] = "x"
    with pytest.raises(SystemExit) as exc:
        qq_mod.QqAdapter()
    assert exc.value.code == 2
    os.environ["QQ_APP_ID"] = "test-app"


def test_missing_app_secret_exits_2():
    os.environ["QQ_APP_SECRET"] = ""
    with pytest.raises(SystemExit) as exc:
        qq_mod.QqAdapter()
    assert exc.value.code == 2
    os.environ["QQ_APP_SECRET"] = "test-secret"


def test_ws_url_override_picked_up():
    a = _adapter(QQ_WS_URL="wss://mock.local/gw")
    assert a.ws_url_override == "wss://mock.local/gw"


# ---- strip_markdown --------------------------------------------------


def test_strip_markdown_bold():
    assert qq_mod.strip_markdown("**bold**") == "bold"


def test_strip_markdown_italic():
    assert qq_mod.strip_markdown("*italic*") == "italic"


def test_strip_markdown_inline_code():
    assert qq_mod.strip_markdown("a `code` b") == "a code b"


def test_strip_markdown_code_block():
    assert qq_mod.strip_markdown("```python\nprint('hi')\n```") == "print('hi')"


def test_strip_markdown_heading():
    assert qq_mod.strip_markdown("# Heading") == "Heading"


def test_strip_markdown_link():
    assert qq_mod.strip_markdown("[label](https://example.com)") == "label"


def test_strip_markdown_quote():
    assert qq_mod.strip_markdown("> quoted") == "quoted"


def test_strip_markdown_table_separator():
    # The table-separator line must be removed (the header/body rows
    # pass through unchanged so the agent's tabular text is still
    # readable).
    text = "| a | b |\n|---|---|\n| 1 | 2 |"
    out = qq_mod.strip_markdown(text)
    assert "---" not in out


def test_strip_markdown_hr():
    assert "---" not in qq_mod.strip_markdown("before\n---\nafter")


def test_strip_markdown_triple_newlines_collapse():
    out = qq_mod.strip_markdown("a\n\n\n\nb")
    assert out == "a\n\nb"


def test_strip_markdown_think_tags_stripped():
    inp = "<think>reasoning here</think>The actual response"
    assert qq_mod.strip_markdown(inp) == "The actual response"


def test_strip_markdown_think_block_multiline():
    inp = "<think>\nstep 1\nstep 2\n</think>\nFinal."
    assert qq_mod.strip_markdown(inp) == "Final."


def test_strip_markdown_empty_input():
    assert qq_mod.strip_markdown("") == ""


# ---- _parse_retry_after ---------------------------------------------


def test_retry_after_missing_returns_default():
    assert qq_mod._parse_retry_after({}, default_secs=30.0) == 30.0


def test_retry_after_parses_seconds():
    assert qq_mod._parse_retry_after(
        {"retry-after": "12"}, default_secs=30.0,
    ) == 12.0


def test_retry_after_floor_one_second():
    assert qq_mod._parse_retry_after(
        {"retry-after": "0"}, default_secs=30.0,
    ) == 1.0


def test_retry_after_caps_at_max_backoff():
    assert qq_mod._parse_retry_after(
        {"retry-after": "9999"}, default_secs=30.0,
    ) == qq_mod.MAX_BACKOFF_SECS


def test_retry_after_garbage_returns_default():
    assert qq_mod._parse_retry_after(
        {"retry-after": "junk"}, default_secs=30.0,
    ) == 30.0


# ---- parse_qq_event --------------------------------------------------


def test_parse_message_create_guild():
    data = {
        "id": "msg-1",
        "channel_id": "chan-1",
        "content": "hello bot",
        "author": {"id": "user-1", "username": "Alice"},
    }
    ev = qq_mod.parse_qq_event(
        "MESSAGE_CREATE", data, allowed_users=[], account_id=None,
    )
    assert ev is not None
    p = ev["params"]
    assert p["user_id"] == "user-1"
    assert p["user_name"] == "Alice"
    assert p["channel_id"] == "/channels/chan-1/messages"
    assert p["thread_id"] == "msg-1"
    assert p["message_id"] == "msg-1"
    assert p["is_group"] is True
    assert p["content"] == {"Text": "hello bot"}


def test_parse_at_message_create_routes_same_as_message_create():
    data = {
        "id": "msg-2",
        "channel_id": "chan-2",
        "content": "ping",
        "author": {"id": "user-2", "username": "Bob"},
    }
    ev = qq_mod.parse_qq_event(
        "AT_MESSAGE_CREATE", data, allowed_users=[], account_id=None,
    )
    assert ev is not None
    assert ev["params"]["channel_id"] == "/channels/chan-2/messages"


def test_parse_direct_message_create():
    data = {
        "id": "dm-1",
        "guild_id": "guild-1",
        "content": "private hi",
        "author": {"id": "user-3", "username": "Carol"},
    }
    ev = qq_mod.parse_qq_event(
        "DIRECT_MESSAGE_CREATE", data, allowed_users=[], account_id=None,
    )
    assert ev is not None
    p = ev["params"]
    assert p["channel_id"] == "/dms/guild-1/messages"
    assert p.get("is_group") is not True
    assert p["user_id"] == "user-3"


def test_parse_group_at_message_create():
    data = {
        "id": "g-1",
        "group_openid": "grp-X",
        "content": "yo",
        "author": {"member_openid": "mem-1"},
    }
    ev = qq_mod.parse_qq_event(
        "GROUP_AT_MESSAGE_CREATE", data, allowed_users=[], account_id=None,
    )
    assert ev is not None
    p = ev["params"]
    assert p["channel_id"] == "/v2/groups/grp-X/messages"
    assert p["user_id"] == "mem-1"
    assert p["is_group"] is True
    # member_openid → display name fallback ("GroupUser") mirrors
    # qq.rs:216.
    assert p["user_name"] == "GroupUser"


def test_parse_c2c_message_create():
    data = {
        "id": "c2c-1",
        "content": "DM body",
        "author": {"user_openid": "openid-9"},
    }
    ev = qq_mod.parse_qq_event(
        "C2C_MESSAGE_CREATE", data, allowed_users=[], account_id=None,
    )
    assert ev is not None
    p = ev["params"]
    assert p["channel_id"] == "/v2/users/openid-9/messages"
    assert p["user_id"] == "openid-9"
    assert p.get("is_group") is not True


def test_parse_empty_content_returns_none():
    data = {
        "id": "x",
        "channel_id": "c",
        "content": "",
        "author": {"id": "u", "username": "u"},
    }
    assert qq_mod.parse_qq_event(
        "MESSAGE_CREATE", data, allowed_users=[], account_id=None,
    ) is None


def test_parse_whitespace_only_content_returns_none():
    data = {
        "id": "x",
        "channel_id": "c",
        "content": "   ",
        "author": {"id": "u", "username": "u"},
    }
    assert qq_mod.parse_qq_event(
        "MESSAGE_CREATE", data, allowed_users=[], account_id=None,
    ) is None


def test_parse_unknown_event_type_returns_none():
    data = {"id": "x", "content": "hello"}
    assert qq_mod.parse_qq_event(
        "UNKNOWN_EVENT", data, allowed_users=[], account_id=None,
    ) is None


def test_parse_non_dict_data_returns_none():
    assert qq_mod.parse_qq_event(
        "MESSAGE_CREATE", "not a dict", allowed_users=[], account_id=None,
    ) is None


def test_parse_allowlist_blocks_others():
    data = {
        "id": "msg",
        "channel_id": "c",
        "content": "hi",
        "author": {"id": "user-x", "username": "X"},
    }
    ev = qq_mod.parse_qq_event(
        "MESSAGE_CREATE", data,
        allowed_users=["user-allowed"], account_id=None,
    )
    assert ev is None


def test_parse_allowlist_passes_match():
    data = {
        "id": "msg",
        "channel_id": "c",
        "content": "hi",
        "author": {"id": "user-allowed", "username": "X"},
    }
    ev = qq_mod.parse_qq_event(
        "MESSAGE_CREATE", data,
        allowed_users=["user-allowed"], account_id=None,
    )
    assert ev is not None


def test_parse_bot_mention_slash_prefix_stripped():
    # The Rust adapter at qq.rs:227 strips a leading '/' before the
    # slash-command check. A "/ping" inbound becomes a plain text
    # "ping" (the leading slash was the QQ at-mention sigil, not a
    # slash command).
    data = {
        "id": "msg",
        "channel_id": "c",
        "content": "/ping",
        "author": {"id": "u", "username": "u"},
    }
    ev = qq_mod.parse_qq_event(
        "MESSAGE_CREATE", data, allowed_users=[], account_id=None,
    )
    assert ev is not None
    assert ev["params"]["content"] == {"Text": "ping"}


def test_parse_slash_command_after_bot_mention():
    # When the bot is mentioned AND the user gives a slash command:
    # original content is "/  /status all". After the leading "/" is
    # stripped, the remaining text starts with "/status" — that's a
    # real command.
    data = {
        "id": "msg",
        "channel_id": "c",
        "content": "/ /status all systems",
        "author": {"id": "u", "username": "u"},
    }
    ev = qq_mod.parse_qq_event(
        "MESSAGE_CREATE", data, allowed_users=[], account_id=None,
    )
    assert ev is not None
    assert ev["params"]["content"] == {
        "Command": {"name": "status", "args": ["all", "systems"]}
    }


def test_parse_only_slash_returns_none():
    # A bare "/" is the bot-mention sigil with no payload — drop.
    data = {
        "id": "msg",
        "channel_id": "c",
        "content": "/",
        "author": {"id": "u", "username": "u"},
    }
    ev = qq_mod.parse_qq_event(
        "MESSAGE_CREATE", data, allowed_users=[], account_id=None,
    )
    assert ev is None


def test_parse_account_id_injected_when_set():
    data = {
        "id": "msg",
        "channel_id": "c",
        "content": "hi",
        "author": {"id": "u", "username": "u"},
    }
    ev = qq_mod.parse_qq_event(
        "MESSAGE_CREATE", data,
        allowed_users=[], account_id="prod-bot",
    )
    assert ev["params"]["metadata"]["account_id"] == "prod-bot"


def test_parse_account_id_omitted_when_unset():
    data = {
        "id": "msg",
        "channel_id": "c",
        "content": "hi",
        "author": {"id": "u", "username": "u"},
    }
    ev = qq_mod.parse_qq_event(
        "MESSAGE_CREATE", data, allowed_users=[], account_id=None,
    )
    assert "account_id" not in (ev["params"].get("metadata") or {})


def test_parse_username_falls_back_to_default():
    # Mirrors qq.rs:199 — author.username defaults to "User" when
    # absent / non-string.
    data = {
        "id": "msg",
        "channel_id": "c",
        "content": "hi",
        "author": {"id": "u"},
    }
    ev = qq_mod.parse_qq_event(
        "MESSAGE_CREATE", data, allowed_users=[], account_id=None,
    )
    assert ev is not None
    assert ev["params"]["user_name"] == "User"


def test_parse_missing_msg_id_omits_thread_and_message_id():
    data = {
        "channel_id": "c",
        "content": "hi",
        "author": {"id": "u", "username": "u"},
    }
    ev = qq_mod.parse_qq_event(
        "MESSAGE_CREATE", data, allowed_users=[], account_id=None,
    )
    assert ev is not None
    assert "thread_id" not in ev["params"]
    assert "message_id" not in ev["params"]


# ---- _mark_seen ------------------------------------------------------


def test_mark_seen_first_true_second_false():
    a = _adapter()
    assert a._mark_seen("msg-1") is True
    assert a._mark_seen("msg-1") is False


def test_mark_seen_empty_id_always_true_no_state_change():
    a = _adapter()
    assert a._mark_seen("") is True
    assert a._mark_seen(None) is True  # type: ignore[arg-type]
    assert "" not in a._seen_ids


def test_mark_seen_eviction_at_cap(monkeypatch):
    monkeypatch.setattr(qq_mod, "SEEN_MESSAGES_MAX", 10)
    monkeypatch.setattr(qq_mod, "SEEN_MESSAGES_EVICT", 4)
    a = _adapter()
    for i in range(11):
        a._mark_seen(f"msg-{i}")
    assert "msg-0" not in a._seen_ids
    assert "msg-3" not in a._seen_ids
    assert "msg-4" in a._seen_ids
    assert "msg-10" in a._seen_ids


# ---- _fetch_token ----------------------------------------------------


def test_fetch_token_happy_path(monkeypatch):
    fake = _FakeUrlopen([(200, {"access_token": "tok-1", "expires_in": 7200})])
    monkeypatch.setattr(qq_mod.urllib.request, "urlopen", fake)
    a = _adapter()
    assert a._fetch_token() == "tok-1"
    c = fake.calls[0]
    assert c["url"] == "https://qq-token.test/getAppAccessToken"
    assert c["method"] == "POST"
    body = json.loads(c["body_raw"])
    assert body == {"appId": "test-app", "clientSecret": "test-secret"}
    assert c["timeout"] == qq_mod.SEND_TIMEOUT_SECS


def test_fetch_token_429_then_200(monkeypatch):
    sleeps = []
    monkeypatch.setattr(qq_mod.time, "sleep", lambda s: sleeps.append(s))
    fake = _FakeUrlopen([
        (429, {}, {"Retry-After": "3"}),
        (200, {"access_token": "tok-after-retry"}),
    ])
    monkeypatch.setattr(qq_mod.urllib.request, "urlopen", fake)
    a = _adapter()
    assert a._fetch_token() == "tok-after-retry"
    assert sleeps == [3.0]
    assert len(fake.calls) == 2


def test_fetch_token_non_200_raises(monkeypatch):
    fake = _FakeUrlopen([(500, {"error": "boom"})])
    monkeypatch.setattr(qq_mod.urllib.request, "urlopen", fake)
    a = _adapter()
    with pytest.raises(RuntimeError) as exc:
        a._fetch_token()
    assert "500" in str(exc.value)


def test_fetch_token_missing_field_raises(monkeypatch):
    fake = _FakeUrlopen([(200, {"expires_in": 7200})])
    monkeypatch.setattr(qq_mod.urllib.request, "urlopen", fake)
    a = _adapter()
    with pytest.raises(RuntimeError) as exc:
        a._fetch_token()
    assert "access_token" in str(exc.value)


# ---- _fetch_gateway --------------------------------------------------


def test_fetch_gateway_happy_path(monkeypatch):
    fake = _FakeUrlopen([(200, {"url": "wss://gw.qq.test/ws"})])
    monkeypatch.setattr(qq_mod.urllib.request, "urlopen", fake)
    a = _adapter()
    a._token = "tok-1"
    assert a._fetch_gateway() == "wss://gw.qq.test/ws"
    c = fake.calls[0]
    assert c["url"] == "https://qq.test/gateway"
    assert c["method"] == "GET"
    assert c["headers"].get("authorization") == "Bearer tok-1"


def test_fetch_gateway_429_then_200(monkeypatch):
    sleeps = []
    monkeypatch.setattr(qq_mod.time, "sleep", lambda s: sleeps.append(s))
    fake = _FakeUrlopen([
        (429, {}, {"Retry-After": "2"}),
        (200, {"url": "wss://gw.qq.test/ws"}),
    ])
    monkeypatch.setattr(qq_mod.urllib.request, "urlopen", fake)
    a = _adapter()
    a._token = "tok-1"
    assert a._fetch_gateway() == "wss://gw.qq.test/ws"
    assert sleeps == [2.0]


def test_fetch_gateway_missing_url_raises(monkeypatch):
    fake = _FakeUrlopen([(200, {})])
    monkeypatch.setattr(qq_mod.urllib.request, "urlopen", fake)
    a = _adapter()
    a._token = "tok-1"
    with pytest.raises(RuntimeError) as exc:
        a._fetch_gateway()
    assert "url" in str(exc.value)


# ---- _post_message --------------------------------------------------


def test_post_message_basic(monkeypatch):
    fake = _FakeUrlopen([(200, {"id": "ok"})])
    monkeypatch.setattr(qq_mod.urllib.request, "urlopen", fake)
    a = _adapter()
    a._token = "tok-1"
    a._post_message("/v2/groups/grp-X/messages", "src-msg", "hello")
    c = fake.calls[0]
    assert c["url"] == "https://qq.test/v2/groups/grp-X/messages"
    assert c["method"] == "POST"
    assert c["headers"]["authorization"] == "Bearer tok-1"
    body = json.loads(c["body_raw"])
    assert body == {"content": "hello", "msg_type": 0, "msg_id": "src-msg"}


def test_post_message_chunks_long_text(monkeypatch):
    monkeypatch.setattr(qq_mod, "QQ_MSG_LIMIT", 5)
    fake = _FakeUrlopen([
        (200, {"id": "ok-1"}),
        (200, {"id": "ok-2"}),
        (200, {"id": "ok-3"}),
    ])
    monkeypatch.setattr(qq_mod.urllib.request, "urlopen", fake)
    a = _adapter()
    a._token = "tok"
    a._post_message("/v2/groups/G/messages", "src", "abcdefghijk")
    assert len(fake.calls) == 3
    bodies = [json.loads(c["body_raw"])["content"] for c in fake.calls]
    assert "".join(bodies) == "abcdefghijk"


def test_post_message_empty_endpoint_is_noop(monkeypatch):
    fake = _FakeUrlopen([])
    monkeypatch.setattr(qq_mod.urllib.request, "urlopen", fake)
    a = _adapter()
    a._token = "tok"
    a._post_message("", "src", "hi")
    assert fake.calls == []


def test_post_message_omits_msg_id_when_none(monkeypatch):
    # The Rust adapter always included `msg_id`; the sidecar omits it
    # when absent so a proactive notification path stays correct if
    # the kernel ever surfaces one.
    fake = _FakeUrlopen([(200, {"id": "ok"})])
    monkeypatch.setattr(qq_mod.urllib.request, "urlopen", fake)
    a = _adapter()
    a._token = "tok"
    a._post_message("/v2/users/u/messages", None, "hi")
    body = json.loads(fake.calls[0]["body_raw"])
    assert "msg_id" not in body


def test_post_message_429_then_200_retries_once(monkeypatch):
    sleeps = []
    monkeypatch.setattr(qq_mod.time, "sleep", lambda s: sleeps.append(s))
    fake = _FakeUrlopen([
        (429, {}, {"Retry-After": "4"}),
        (201, {}),
    ])
    monkeypatch.setattr(qq_mod.urllib.request, "urlopen", fake)
    a = _adapter()
    a._token = "tok"
    a._post_message("/v2/groups/G/messages", "src", "hi")
    assert sleeps == [4.0]
    assert len(fake.calls) == 2


def test_post_message_persistent_429_is_fail_open(monkeypatch):
    monkeypatch.setattr(qq_mod.time, "sleep", lambda _s: None)
    fake = _FakeUrlopen([
        (429, {}, {}),
        (429, {}, {}),
    ])
    monkeypatch.setattr(qq_mod.urllib.request, "urlopen", fake)
    a = _adapter()
    a._token = "tok"
    a._post_message("/v2/groups/G/messages", "src", "hi")  # must not raise
    assert len(fake.calls) == 2


def test_post_message_5xx_is_fail_open_keeps_chunking(monkeypatch):
    monkeypatch.setattr(qq_mod, "QQ_MSG_LIMIT", 3)
    fake = _FakeUrlopen([
        (500, {"err": "boom"}),
        (200, {}),
        (200, {}),
    ])
    monkeypatch.setattr(qq_mod.urllib.request, "urlopen", fake)
    a = _adapter()
    a._token = "tok"
    # 9 chars at limit=3 → three chunks.
    a._post_message("/v2/groups/G/messages", "src", "abcdefghi")
    assert len(fake.calls) == 3


# ---- on_send ---------------------------------------------------------


def _make_send(channel_id="/v2/groups/G/messages", text="hi",
               content=None, thread_id="src-1", user=None):
    from librefang.sidecar.protocol import Send
    return Send(channel_id, text, content, thread_id, user or {})


@pytest.mark.asyncio
async def test_on_send_text(monkeypatch):
    fake = _FakeUrlopen([(200, {})])
    monkeypatch.setattr(qq_mod.urllib.request, "urlopen", fake)
    a = _adapter()
    a._token = "tok"
    await a.on_send(_make_send(text="hello", content={"Text": "hello"}))
    c = fake.calls[0]
    assert c["url"].endswith("/v2/groups/G/messages")
    body = json.loads(c["body_raw"])
    assert body["content"] == "hello"
    assert body["msg_id"] == "src-1"


@pytest.mark.asyncio
async def test_on_send_strips_markdown(monkeypatch):
    fake = _FakeUrlopen([(200, {})])
    monkeypatch.setattr(qq_mod.urllib.request, "urlopen", fake)
    a = _adapter()
    a._token = "tok"
    await a.on_send(_make_send(
        text="**hi** there [link](http://x)",
        content={"Text": "**hi** there [link](http://x)"},
    ))
    body = json.loads(fake.calls[0]["body_raw"])
    assert body["content"] == "hi there link"


@pytest.mark.asyncio
async def test_on_send_unsupported_content_uses_placeholder(monkeypatch):
    fake = _FakeUrlopen([(200, {})])
    monkeypatch.setattr(qq_mod.urllib.request, "urlopen", fake)
    a = _adapter()
    a._token = "tok"
    await a.on_send(_make_send(
        text="",
        content={"Image": {"url": "https://x/y.jpg", "caption": None,
                            "mime_type": None}},
    ))
    body = json.loads(fake.calls[0]["body_raw"])
    assert body["content"] == "(Unsupported content type)"


@pytest.mark.asyncio
async def test_on_send_empty_endpoint_drops_silently(monkeypatch):
    fake = _FakeUrlopen([])
    monkeypatch.setattr(qq_mod.urllib.request, "urlopen", fake)
    a = _adapter()
    a._token = "tok"
    await a.on_send(_make_send(channel_id="", user={}))
    assert fake.calls == []


@pytest.mark.asyncio
async def test_on_send_falls_back_to_user_platform_id(monkeypatch):
    fake = _FakeUrlopen([(200, {})])
    monkeypatch.setattr(qq_mod.urllib.request, "urlopen", fake)
    a = _adapter()
    a._token = "tok"
    await a.on_send(_make_send(
        channel_id="",
        text="hi",
        content={"Text": "hi"},
        user={"platform_id": "/v2/users/u9/messages"},
    ))
    c = fake.calls[0]
    assert c["url"].endswith("/v2/users/u9/messages")


# ---- WS gateway flow (mock _WebSocketClient) ------------------------


class _FakeWS:
    """Lightweight stand-in for ``_WebSocketClient`` driven by a
    pre-baked transcript of inbound JSON frames. ``send_text`` records
    outbound frames so tests can assert on the IDENTIFY / heartbeat
    payloads."""

    def __init__(self, inbound):
        self._inbound = list(inbound)
        self._closed = False
        self.sent: list[dict] = []

    def __enter__(self):
        return self

    def __exit__(self, *_):
        self._closed = True
        return False

    def settimeout(self, _t):
        pass

    def wait_readable(self, _timeout):
        return bool(self._inbound)

    def recv_frame(self):
        if not self._inbound:
            raise EOFError("transcript exhausted")
        item = self._inbound.pop(0)
        if isinstance(item, dict):
            return json.dumps(item), None
        if isinstance(item, str):
            return item, None
        if item is None:
            return None, (1000, b"normal")
        raise AssertionError(f"unsupported transcript entry: {item!r}")

    def send_text(self, s):
        self.sent.append(json.loads(s))


def test_run_session_hello_then_identify(monkeypatch):
    a = _adapter()
    fake = _FakeWS([
        # HELLO
        {"op": 10, "d": {"heartbeat_interval": 45000}},
        # READY dispatch
        {"op": 0, "s": 1, "t": "READY", "d": {"user": {"username": "TestBot"}}},
        # Normal close
        None,
    ])
    emitted: list = []
    a._run_session(fake, "tok-X", emitted.append)
    # Sent at least one IDENTIFY.
    assert any(s.get("op") == 2 for s in fake.sent), fake.sent
    identify = next(s for s in fake.sent if s.get("op") == 2)
    assert identify["d"]["token"] == "QQBot tok-X"
    assert identify["d"]["intents"] == a.intents
    assert identify["d"]["shard"] == [0, 1]
    # READY does not produce an emitted event.
    assert emitted == []


def test_run_session_dispatch_emits_message(monkeypatch):
    a = _adapter()
    fake = _FakeWS([
        {"op": 10, "d": {"heartbeat_interval": 45000}},
        {"op": 0, "s": 1, "t": "READY", "d": {"user": {"username": "B"}}},
        {
            "op": 0, "s": 2, "t": "GROUP_AT_MESSAGE_CREATE",
            "d": {
                "id": "m-1",
                "group_openid": "grp-Z",
                "content": "hi",
                "author": {"member_openid": "mem-9"},
            },
        },
        None,
    ])
    emitted: list = []
    a._run_session(fake, "tok", emitted.append)
    assert len(emitted) == 1
    p = emitted[0]["params"]
    assert p["user_id"] == "mem-9"
    assert p["channel_id"] == "/v2/groups/grp-Z/messages"
    assert p["thread_id"] == "m-1"


def test_run_session_dedupes_repeated_msg_id():
    a = _adapter()
    fake = _FakeWS([
        {"op": 10, "d": {"heartbeat_interval": 45000}},
        {"op": 0, "s": 1, "t": "READY", "d": {}},
        {
            "op": 0, "s": 2, "t": "MESSAGE_CREATE",
            "d": {"id": "m-1", "channel_id": "c",
                   "content": "hi",
                   "author": {"id": "u", "username": "u"}},
        },
        {
            "op": 0, "s": 3, "t": "MESSAGE_CREATE",
            "d": {"id": "m-1", "channel_id": "c",
                   "content": "hi",
                   "author": {"id": "u", "username": "u"}},
        },
        None,
    ])
    emitted: list = []
    a._run_session(fake, "tok", emitted.append)
    assert len(emitted) == 1


def test_run_session_reconnect_op_returns(monkeypatch):
    a = _adapter()
    fake = _FakeWS([
        {"op": 10, "d": {"heartbeat_interval": 45000}},
        {"op": 7},  # RECONNECT
    ])
    emitted: list = []
    # Must not raise — and must return without blocking on more frames.
    a._run_session(fake, "tok", emitted.append)


def test_run_session_invalid_session_sleeps_and_returns(monkeypatch):
    a = _adapter()
    sleeps = []
    monkeypatch.setattr(qq_mod.time, "sleep", lambda s: sleeps.append(s))
    fake = _FakeWS([
        {"op": 10, "d": {"heartbeat_interval": 45000}},
        {"op": 9},  # INVALID_SESSION
    ])
    a._run_session(fake, "tok", lambda _e: None)
    assert sleeps == [3.0]


def test_run_session_heartbeat_fires_after_interval(monkeypatch):
    a = _adapter()
    # Time travel: each call to time.monotonic() advances by ~50 ms.
    counter = {"t": 0.0}

    def fake_mono():
        return counter["t"]

    def advance(_):
        counter["t"] += 0.05

    monkeypatch.setattr(qq_mod.time, "monotonic", fake_mono)
    # Inbound: HELLO then a single brief readable tick, then close.
    fake = _FakeWS([
        {"op": 10, "d": {"heartbeat_interval": 100}},  # 100 ms = 0.1 s
        {"op": 11},  # HEARTBEAT_ACK (no-op)
        None,
    ])
    # Wrap wait_readable so each invocation advances the clock past
    # the 0.1 s heartbeat deadline.
    orig_wait = fake.wait_readable

    def wait(_t):
        counter["t"] += 0.5  # well past 0.1 s deadline
        return orig_wait(_t)
    fake.wait_readable = wait  # type: ignore[assignment]
    a._run_session(fake, "tok", lambda _e: None)
    # At least one IDENTIFY (op=2) plus at least one heartbeat (op=1).
    ops = [s.get("op") for s in fake.sent]
    assert 2 in ops, ops
    assert 1 in ops, ops


# ---- schema (--describe) -------------------------------------------


def test_schema_round_trip():
    schema = qq_mod.QqAdapter.SCHEMA.to_dict()
    assert schema["name"] == "qq"
    keys = {f["key"] for f in schema["fields"]}
    expected = {
        "QQ_APP_ID", "QQ_APP_SECRET", "QQ_ALLOWED_USERS",
        "QQ_ACCOUNT_ID", "QQ_INTENTS",
    }
    assert expected.issubset(keys), f"missing: {expected - keys}"
    secret_fields = {
        f["key"] for f in schema["fields"] if f["type"] == "secret"
    }
    assert secret_fields == {"QQ_APP_SECRET"}


def test_capabilities_empty():
    # QQ Bot API v2 has no public typing/reaction surface we can wire
    # up; keep capabilities empty rather than over-claim. Mirrors
    # line / zulip / signal.
    assert qq_mod.QqAdapter.capabilities == []
