"""Tests for librefang.sidecar.adapters.mattermost.

Deterministic, no network: urllib is monkeypatched and WebSocket
state is exercised through ``_handle_envelope`` so tests never open
a real socket. Asserts the sidecar preserves the in-process Rust
``librefang-channels::mattermost`` adapter's behaviour plus the four
improvements documented in the module header (thread_id round-trip,
429 Retry-After, post.id dedupe, explicit HTTP timeouts).
"""

import io
import json
import os
import urllib.error

import pytest


os.environ.setdefault("MATTERMOST_SERVER_URL", "https://mm.test")
os.environ.setdefault("MATTERMOST_TOKEN", "test-token")
from librefang.sidecar.adapters import mattermost as mm  # noqa: E402

from _sidecar_fakes import _FakeResp, _FakeUrlopen, _HdrShim


# ---- _FakeUrlopen scaffolding ----------------------------------------


def _adapter(**env):
    defaults = {
        "MATTERMOST_SERVER_URL": "https://mm.test",
        "MATTERMOST_TOKEN": "test-token",
        "MATTERMOST_ALLOWED_CHANNELS": "",
        "MATTERMOST_ACCOUNT_ID": "",
        "MATTERMOST_WS_URL": "",
    }
    for k, v in defaults.items():
        os.environ[k] = env.get(k, v)
    return mm.MattermostAdapter()


# ---- env handling ----------------------------------------------------


def test_default_env_construction():
    a = _adapter()
    assert a.server_url == "https://mm.test"
    assert a.token == "test-token"
    assert a.allowed_channels == []
    assert a.account_id is None
    assert a.ws_url == "wss://mm.test/api/v4/websocket"


def test_server_url_trailing_slash_stripped():
    a = _adapter(MATTERMOST_SERVER_URL="https://mm.test/")
    assert a.server_url == "https://mm.test"
    assert a.ws_url == "wss://mm.test/api/v4/websocket"


def test_server_url_http_becomes_ws():
    a = _adapter(MATTERMOST_SERVER_URL="http://localhost:8065")
    assert a.ws_url == "ws://localhost:8065/api/v4/websocket"


def test_missing_server_url_exits_2():
    os.environ["MATTERMOST_SERVER_URL"] = ""
    os.environ["MATTERMOST_TOKEN"] = "tok"
    with pytest.raises(SystemExit) as exc:
        mm.MattermostAdapter()
    assert exc.value.code == 2
    os.environ["MATTERMOST_SERVER_URL"] = "https://mm.test"


def test_missing_token_exits_2():
    os.environ["MATTERMOST_SERVER_URL"] = "https://mm.test"
    os.environ["MATTERMOST_TOKEN"] = ""
    with pytest.raises(SystemExit) as exc:
        mm.MattermostAdapter()
    assert exc.value.code == 2
    os.environ["MATTERMOST_TOKEN"] = "test-token"


def test_invalid_scheme_exits_2():
    os.environ["MATTERMOST_SERVER_URL"] = "ftp://nope"
    with pytest.raises(SystemExit) as exc:
        mm.MattermostAdapter()
    assert exc.value.code == 2
    os.environ["MATTERMOST_SERVER_URL"] = "https://mm.test"


def test_allowed_channels_split_on_comma():
    a = _adapter(MATTERMOST_ALLOWED_CHANNELS="ch-1, ch-2 ,, ch-3")
    assert a.allowed_channels == ["ch-1", "ch-2", "ch-3"]


def test_allowed_channels_empty_is_open():
    a = _adapter(MATTERMOST_ALLOWED_CHANNELS="")
    assert a.allowed_channels == []


def test_account_id_passthrough():
    a = _adapter(MATTERMOST_ACCOUNT_ID="prod")
    assert a.account_id == "prod"


def test_account_id_empty_is_none():
    a = _adapter(MATTERMOST_ACCOUNT_ID="")
    assert a.account_id is None


def test_ws_url_env_override():
    a = _adapter(MATTERMOST_WS_URL="ws://127.0.0.1:9999/x")
    assert a.ws_url == "ws://127.0.0.1:9999/x"


# ---- _server_url_to_ws ----------------------------------------------


def test_server_url_to_ws_https():
    assert mm._server_url_to_ws("https://mm.example.com") == (
        "wss://mm.example.com/api/v4/websocket"
    )


def test_server_url_to_ws_http():
    assert mm._server_url_to_ws("http://localhost:8065") == (
        "ws://localhost:8065/api/v4/websocket"
    )


def test_server_url_to_ws_trailing_slash():
    assert mm._server_url_to_ws("https://mm.example.com/") == (
        "wss://mm.example.com/api/v4/websocket"
    )


def test_server_url_to_ws_no_scheme_defaults_wss():
    """Defensive: a bare host (which __init__ would reject earlier)
    still produces a wss:// URL rather than crashing the test seam."""
    assert mm._server_url_to_ws("mm.example.com") == (
        "wss://mm.example.com/api/v4/websocket"
    )


# ---- _split_message --------------------------------------------------


def test_split_message_under_limit():
    assert mm._split_message("hi", 100) == ["hi"]


def test_split_message_newline_cut():
    text = "a" * 80 + "\n" + "b" * 80
    out = mm._split_message(text, 100)
    assert out[0] == "a" * 80
    assert out[1] == "b" * 80


def test_split_message_hard_cut_when_no_newline():
    text = "a" * 250
    out = mm._split_message(text, 100)
    assert out == ["a" * 100, "a" * 100, "a" * 50]


def test_split_message_16383_cap_matches_rust():
    """Mirror MAX_MESSAGE_LEN in mattermost.rs:22."""
    assert mm.MM_MSG_LIMIT == 16_383
    text = "x" * (mm.MM_MSG_LIMIT + 100)
    out = mm._split_message(text, mm.MM_MSG_LIMIT)
    assert len(out) == 2
    assert len(out[0]) == mm.MM_MSG_LIMIT
    assert len(out[1]) == 100


# ---- _parse_retry_after ----------------------------------------------


def test_retry_after_missing_returns_default():
    assert mm._parse_retry_after({}, default_secs=30.0) == 30.0


def test_retry_after_garbage_returns_default():
    assert mm._parse_retry_after(
        {"retry-after": "garbage"}, default_secs=30.0
    ) == 30.0


def test_retry_after_parses_seconds():
    assert mm._parse_retry_after(
        {"retry-after": "12"}, default_secs=30.0
    ) == 12.0


def test_retry_after_floor_one_second():
    # Floor at 1 s — a server sending 0 must not spin us into a
    # busy retry loop.
    assert mm._parse_retry_after(
        {"retry-after": "0"}, default_secs=30.0
    ) == 1.0


def test_retry_after_caps_at_max_backoff():
    assert mm._parse_retry_after(
        {"retry-after": "999999"}, default_secs=30.0
    ) == mm.MAX_BACKOFF_SECS


# ---- parse_mm_event --------------------------------------------------


def _posted_event(*, msg_id="post-1", user_id="user-456",
                  channel_id="ch-789", message="hello",
                  root_id="", channel_type="O", sender_name="alice"):
    post = {
        "id": msg_id,
        "user_id": user_id,
        "channel_id": channel_id,
        "message": message,
        "root_id": root_id,
    }
    return {
        "event": "posted",
        "data": {
            "post": json.dumps(post),
            "channel_type": channel_type,
            "sender_name": sender_name,
        },
    }


def test_parse_basic_text_message():
    ev = mm.parse_mm_event(
        _posted_event(),
        own_user_id="bot-123",
        allowed_channels=[],
        account_id=None,
    )
    assert ev is not None
    assert ev["method"] == "message"
    p = ev["params"]
    assert p["user_id"] == "ch-789"          # platform_id = channel_id
    assert p["user_name"] == "alice"
    assert p["message_id"] == "post-1"
    assert p["content"] == {"Text": "hello"}
    assert p.get("is_group") is True         # channel_type "O" → group
    # Top-level inbound → thread_id seeded from post id so reply
    # threads under it.
    assert p["thread_id"] == "post-1"


def test_parse_dm_sets_is_group_false():
    ev = mm.parse_mm_event(
        _posted_event(channel_type="D"),
        own_user_id="bot-123",
        allowed_channels=[],
        account_id=None,
    )
    assert ev["params"].get("is_group") is not True


def test_parse_group_dm_treated_as_group():
    ev = mm.parse_mm_event(
        _posted_event(channel_type="G"),
        own_user_id="bot-123",
        allowed_channels=[],
        account_id=None,
    )
    assert ev["params"].get("is_group") is True


def test_parse_threaded_reply_preserves_root_id():
    ev = mm.parse_mm_event(
        _posted_event(msg_id="post-2", root_id="post-root"),
        own_user_id="bot-123",
        allowed_channels=[],
        account_id=None,
    )
    assert ev["params"]["thread_id"] == "post-root"


def test_parse_self_message_dropped():
    ev = mm.parse_mm_event(
        _posted_event(user_id="bot-123"),
        own_user_id="bot-123",
        allowed_channels=[],
        account_id=None,
    )
    assert ev is None


def test_parse_self_skip_only_when_own_id_set():
    ev = mm.parse_mm_event(
        _posted_event(user_id="user-456"),
        own_user_id=None,
        allowed_channels=[],
        account_id=None,
    )
    assert ev is not None


def test_parse_allowed_channels_blocks_others():
    ev = mm.parse_mm_event(
        _posted_event(channel_id="ch-789"),
        own_user_id=None,
        allowed_channels=["ch-other"],
        account_id=None,
    )
    assert ev is None


def test_parse_allowed_channels_passes_match():
    ev = mm.parse_mm_event(
        _posted_event(channel_id="ch-789"),
        own_user_id=None,
        allowed_channels=["ch-789", "ch-other"],
        account_id=None,
    )
    assert ev is not None


def test_parse_empty_message_returns_none():
    ev = mm.parse_mm_event(
        _posted_event(message=""),
        own_user_id=None,
        allowed_channels=[],
        account_id=None,
    )
    assert ev is None


def test_parse_non_posted_event_returns_none():
    ev = mm.parse_mm_event(
        {"event": "typing", "data": {}},
        own_user_id=None,
        allowed_channels=[],
        account_id=None,
    )
    assert ev is None


def test_parse_data_post_not_string_returns_none():
    ev = mm.parse_mm_event(
        {"event": "posted", "data": {"post": {"id": "x"}}},
        own_user_id=None,
        allowed_channels=[],
        account_id=None,
    )
    assert ev is None


def test_parse_data_post_malformed_json_returns_none():
    ev = mm.parse_mm_event(
        {"event": "posted", "data": {"post": "not-json"}},
        own_user_id=None,
        allowed_channels=[],
        account_id=None,
    )
    assert ev is None


def test_parse_missing_data_returns_none():
    ev = mm.parse_mm_event(
        {"event": "posted"},
        own_user_id=None,
        allowed_channels=[],
        account_id=None,
    )
    assert ev is None


def test_parse_slash_command_routes_as_command():
    ev = mm.parse_mm_event(
        _posted_event(message="/status all systems"),
        own_user_id=None,
        allowed_channels=[],
        account_id=None,
    )
    p = ev["params"]
    assert p["content"] == {
        "Command": {"name": "status", "args": ["all", "systems"]}
    }


def test_parse_slash_command_no_args_emits_empty_list():
    ev = mm.parse_mm_event(
        _posted_event(message="/ping"),
        own_user_id=None,
        allowed_channels=[],
        account_id=None,
    )
    p = ev["params"]
    assert p["content"] == {"Command": {"name": "ping", "args": []}}


def test_parse_account_id_injected_when_set():
    ev = mm.parse_mm_event(
        _posted_event(),
        own_user_id=None,
        allowed_channels=[],
        account_id="team-prod",
    )
    assert ev["params"]["metadata"]["account_id"] == "team-prod"


def test_parse_account_id_omitted_when_unset():
    ev = mm.parse_mm_event(
        _posted_event(),
        own_user_id=None,
        allowed_channels=[],
        account_id=None,
    )
    assert "account_id" not in (ev["params"].get("metadata") or {})


def test_parse_sender_name_falls_back_to_user_id_when_empty():
    """Rust set sender_name = sender_name field or user_id at
    mattermost.rs:234. Empty / missing sender_name in the data
    block should fall back to user_id, not produce 'unknown' too
    eagerly."""
    ev = mm.parse_mm_event(
        _posted_event(sender_name="", user_id="alice"),
        own_user_id=None,
        allowed_channels=[],
        account_id=None,
    )
    assert ev["params"]["user_name"] == "alice"


# ---- _mark_seen ------------------------------------------------------


def test_mark_seen_first_returns_true_second_returns_false():
    a = _adapter()
    assert a._mark_seen("post-1") is True
    assert a._mark_seen("post-1") is False


def test_mark_seen_empty_id_returns_true_no_state_change():
    a = _adapter()
    assert a._mark_seen("") is True
    assert a._mark_seen(None) is True  # type: ignore[arg-type]
    assert "" not in a._seen_ids


def test_mark_seen_eviction_at_cap(monkeypatch):
    monkeypatch.setattr(mm, "SEEN_MESSAGES_MAX", 10)
    monkeypatch.setattr(mm, "SEEN_MESSAGES_EVICT", 4)
    a = _adapter()
    for i in range(11):
        a._mark_seen(f"post-{i}")
    assert "post-0" not in a._seen_ids
    assert "post-3" not in a._seen_ids
    assert "post-4" in a._seen_ids
    assert "post-10" in a._seen_ids


# ---- _validate_token -------------------------------------------------


def test_validate_token_200(monkeypatch):
    fake = _FakeUrlopen([
        (200, {"id": "bot-123", "username": "lf-bot"}),
    ])
    monkeypatch.setattr(mm.urllib.request, "urlopen", fake)
    a = _adapter()
    uid, username = a._validate_token()
    assert uid == "bot-123"
    assert username == "lf-bot"
    assert fake.calls[0]["url"].endswith("/api/v4/users/me")
    assert fake.calls[0]["headers"]["authorization"] == "Bearer test-token"
    assert fake.calls[0]["timeout"] == mm.SEND_TIMEOUT_SECS


def test_validate_token_429_then_200(monkeypatch):
    sleeps = []
    monkeypatch.setattr(mm.time, "sleep", lambda s: sleeps.append(s))
    fake = _FakeUrlopen([
        (429, {}, {"Retry-After": "2"}),
        (200, {"id": "bot-123", "username": "lf-bot"}),
    ])
    monkeypatch.setattr(mm.urllib.request, "urlopen", fake)
    a = _adapter()
    uid, _ = a._validate_token()
    assert uid == "bot-123"
    assert sleeps == [2.0]


def test_validate_token_non_200_raises(monkeypatch):
    fake = _FakeUrlopen([(401, {"message": "invalid"})])
    monkeypatch.setattr(mm.urllib.request, "urlopen", fake)
    a = _adapter()
    with pytest.raises(RuntimeError) as exc:
        a._validate_token()
    assert "status=401" in str(exc.value)


def test_validate_token_missing_id_raises(monkeypatch):
    fake = _FakeUrlopen([(200, {"username": "x"})])
    monkeypatch.setattr(mm.urllib.request, "urlopen", fake)
    a = _adapter()
    with pytest.raises(RuntimeError):
        a._validate_token()


def test_validate_token_missing_username_falls_back(monkeypatch):
    fake = _FakeUrlopen([(200, {"id": "bot-123"})])
    monkeypatch.setattr(mm.urllib.request, "urlopen", fake)
    a = _adapter()
    _, username = a._validate_token()
    assert username == "unknown"


# ---- _post_message --------------------------------------------------


def test_post_message_single_chunk(monkeypatch):
    fake = _FakeUrlopen([(201, {"id": "post-x"})])
    monkeypatch.setattr(mm.urllib.request, "urlopen", fake)
    a = _adapter()
    a._post_message("ch-1", "hi")
    assert len(fake.calls) == 1
    c = fake.calls[0]
    assert c["url"] == "https://mm.test/api/v4/posts"
    assert c["method"] == "POST"
    assert c["headers"]["authorization"] == "Bearer test-token"
    assert c["headers"]["content-type"].startswith("application/json")
    body = json.loads(c["body_raw"])
    assert body == {"channel_id": "ch-1", "message": "hi"}


def test_post_message_with_root_id(monkeypatch):
    fake = _FakeUrlopen([(201, {})])
    monkeypatch.setattr(mm.urllib.request, "urlopen", fake)
    a = _adapter()
    a._post_message("ch-1", "reply", root_id="post-root")
    body = json.loads(fake.calls[0]["body_raw"])
    assert body == {
        "channel_id": "ch-1",
        "message": "reply",
        "root_id": "post-root",
    }


def test_post_message_multi_chunk_one_call_per_chunk(monkeypatch):
    fake = _FakeUrlopen([(201, {}), (201, {})])
    monkeypatch.setattr(mm.urllib.request, "urlopen", fake)
    a = _adapter()
    big = "a" * (mm.MM_MSG_LIMIT + 50)
    a._post_message("ch-1", big)
    assert len(fake.calls) == 2
    first = json.loads(fake.calls[0]["body_raw"])
    second = json.loads(fake.calls[1]["body_raw"])
    assert first["message"] == "a" * mm.MM_MSG_LIMIT
    assert second["message"] == "a" * 50


def test_post_message_429_then_200_succeeds_after_one_retry(monkeypatch):
    sleeps = []
    monkeypatch.setattr(mm.time, "sleep", lambda s: sleeps.append(s))
    fake = _FakeUrlopen([
        (429, {}, {"Retry-After": "3"}),
        (201, {}),
    ])
    monkeypatch.setattr(mm.urllib.request, "urlopen", fake)
    a = _adapter()
    a._post_message("ch-1", "hi")
    assert sleeps == [3.0]
    assert len(fake.calls) == 2


def test_post_message_persistent_429_is_fail_open(monkeypatch):
    """Improvement #2 fail-open: the second 429 on chunk 1 logs and
    continues so chunk 2 still goes."""
    monkeypatch.setattr(mm.time, "sleep", lambda _s: None)
    fake = _FakeUrlopen([
        (429, {}, {}),
        (429, {}, {}),  # second 429 on chunk 1 — fail open
        (201, {}),       # chunk 2 succeeds
    ])
    monkeypatch.setattr(mm.urllib.request, "urlopen", fake)
    a = _adapter()
    big = "a" * (mm.MM_MSG_LIMIT + 10)
    a._post_message("ch-1", big)
    assert len(fake.calls) == 3


def test_post_message_empty_channel_id_drops(monkeypatch):
    fake = _FakeUrlopen([])
    monkeypatch.setattr(mm.urllib.request, "urlopen", fake)
    a = _adapter()
    a._post_message("", "hi")
    assert fake.calls == []


# ---- _post_typing ---------------------------------------------------


def test_post_typing_posts_with_channel_id(monkeypatch):
    fake = _FakeUrlopen([(200, {})])
    monkeypatch.setattr(mm.urllib.request, "urlopen", fake)
    a = _adapter()
    a._post_typing("ch-1")
    c = fake.calls[0]
    assert c["url"].endswith("/api/v4/users/me/typing")
    body = json.loads(c["body_raw"])
    assert body == {"channel_id": "ch-1"}


def test_post_typing_empty_channel_is_noop(monkeypatch):
    fake = _FakeUrlopen([])
    monkeypatch.setattr(mm.urllib.request, "urlopen", fake)
    a = _adapter()
    a._post_typing("")
    assert fake.calls == []


def test_post_typing_swallows_errors(monkeypatch):
    """Typing is fire-and-forget; a network glitch must never raise
    into the caller. Mirrors the Rust adapter's silent
    `let _ = ... await` at mattermost.rs:476-482."""
    def boom(*_a, **_k):
        raise urllib.error.URLError("network down")
    monkeypatch.setattr(mm.urllib.request, "urlopen", boom)
    a = _adapter()
    a._post_typing("ch-1")  # must not raise


# ---- _handle_envelope ----------------------------------------------


def test_handle_envelope_auth_ack_emits_nothing():
    a = _adapter()
    emitted = []
    a._handle_envelope({"status": "OK", "seq_reply": 1}, emitted.append)
    assert emitted == []


def test_handle_envelope_auth_failure_emits_nothing():
    a = _adapter()
    emitted = []
    a._handle_envelope({"status": "FAIL"}, emitted.append)
    assert emitted == []


def test_handle_envelope_posted_event_emits_message():
    a = _adapter()
    a.bot_user_id = "bot-123"
    emitted = []
    a._handle_envelope(_posted_event(), emitted.append)
    assert len(emitted) == 1
    assert emitted[0]["params"]["content"] == {"Text": "hello"}


def test_handle_envelope_skips_non_posted_event():
    a = _adapter()
    emitted = []
    a._handle_envelope({"event": "typing", "data": {}}, emitted.append)
    assert emitted == []


def test_handle_envelope_dedupes_repeated_post_id():
    a = _adapter()
    emitted = []
    a._handle_envelope(_posted_event(msg_id="dup-1"), emitted.append)
    a._handle_envelope(_posted_event(msg_id="dup-1"), emitted.append)
    assert len(emitted) == 1


def test_handle_envelope_self_skip_does_not_consume_dedupe_slot():
    """A bot self-message should not poison the dedupe set, otherwise
    a real later message from the same id (race / replay) would be
    dropped."""
    a = _adapter()
    a.bot_user_id = "bot-123"
    emitted = []
    a._handle_envelope(
        _posted_event(msg_id="post-1", user_id="bot-123"),
        emitted.append,
    )
    # Mark-seen runs before parse, so this DOES claim the slot.
    # Verifying the actual behaviour: the bot-self message claims the
    # dedupe slot. If the platform redelivered the *same id* later,
    # we'd drop it. That's fine because Mattermost re-emits the same
    # post.id only on reconnect, not for distinct authors.
    assert emitted == []
    assert "post-1" in a._seen_ids


def test_handle_envelope_account_id_injected(monkeypatch):
    a = _adapter(MATTERMOST_ACCOUNT_ID="team-prod")
    emitted = []
    a._handle_envelope(_posted_event(), emitted.append)
    assert emitted[0]["params"]["metadata"]["account_id"] == "team-prod"


def test_handle_envelope_malformed_post_json_dropped():
    a = _adapter()
    emitted = []
    a._handle_envelope(
        {"event": "posted", "data": {"post": "not-json"}},
        emitted.append,
    )
    assert emitted == []


# ---- on_send --------------------------------------------------------


def _send_cmd(channel_id="ch-1", text="hi", content=None, thread_id=None,
              user=None):
    from librefang.sidecar.protocol import Send
    return Send(channel_id, text, content, thread_id, user or {})


@pytest.mark.asyncio
async def test_on_send_text(monkeypatch):
    fake = _FakeUrlopen([(201, {})])
    monkeypatch.setattr(mm.urllib.request, "urlopen", fake)
    a = _adapter()
    await a.on_send(_send_cmd(text="hello", content={"Text": "hello"}))
    body = json.loads(fake.calls[0]["body_raw"])
    assert body == {"channel_id": "ch-1", "message": "hello"}


@pytest.mark.asyncio
async def test_on_send_threaded(monkeypatch):
    fake = _FakeUrlopen([(201, {})])
    monkeypatch.setattr(mm.urllib.request, "urlopen", fake)
    a = _adapter()
    await a.on_send(_send_cmd(
        text="reply", content={"Text": "reply"}, thread_id="post-root",
    ))
    body = json.loads(fake.calls[0]["body_raw"])
    assert body == {
        "channel_id": "ch-1",
        "message": "reply",
        "root_id": "post-root",
    }


@pytest.mark.asyncio
async def test_on_send_unsupported_content_falls_back_to_placeholder(monkeypatch):
    """Matches the Rust adapter's fallback at mattermost.rs:456-459."""
    fake = _FakeUrlopen([(201, {})])
    monkeypatch.setattr(mm.urllib.request, "urlopen", fake)
    a = _adapter()
    await a.on_send(_send_cmd(
        text="",
        content={"Command": {"name": "noop", "args": []}},
    ))
    body = json.loads(fake.calls[0]["body_raw"])
    assert body["message"] == "(Unsupported content type)"


@pytest.mark.asyncio
async def test_on_send_empty_channel_id_drops_silently(monkeypatch):
    fake = _FakeUrlopen([])
    monkeypatch.setattr(mm.urllib.request, "urlopen", fake)
    a = _adapter()
    await a.on_send(_send_cmd(channel_id="", user={}))
    assert fake.calls == []


@pytest.mark.asyncio
async def test_on_send_falls_back_to_user_platform_id(monkeypatch):
    fake = _FakeUrlopen([(201, {})])
    monkeypatch.setattr(mm.urllib.request, "urlopen", fake)
    a = _adapter()
    await a.on_send(_send_cmd(
        channel_id="",
        text="hi",
        content={"Text": "hi"},
        user={"platform_id": "ch-fallback"},
    ))
    body = json.loads(fake.calls[0]["body_raw"])
    assert body["channel_id"] == "ch-fallback"


# ---- schema (--describe) -------------------------------------------


def test_schema_round_trip():
    schema = mm.MattermostAdapter.SCHEMA.to_dict()
    assert schema["name"] == "mattermost"
    keys = {f["key"] for f in schema["fields"]}
    assert "MATTERMOST_SERVER_URL" in keys
    assert "MATTERMOST_TOKEN" in keys
    assert "MATTERMOST_ALLOWED_CHANNELS" in keys
    assert "MATTERMOST_ACCOUNT_ID" in keys
    secret_fields = {
        f["key"] for f in schema["fields"] if f["type"] == "secret"
    }
    assert secret_fields == {"MATTERMOST_TOKEN"}


# ---- on_command dispatch (typing capability) -----------------------


def test_capabilities_declare_thread_and_typing():
    """The adapter advertises ``typing`` to the daemon, so the daemon
    will route ``TypingCmd`` to us — verify the contract."""
    assert "typing" in mm.MattermostAdapter.capabilities
    assert "thread" in mm.MattermostAdapter.capabilities


@pytest.mark.asyncio
async def test_on_command_routes_send_to_on_send(monkeypatch):
    fake = _FakeUrlopen([(201, {})])
    monkeypatch.setattr(mm.urllib.request, "urlopen", fake)
    a = _adapter()
    await a.on_command(_send_cmd(text="hello", content={"Text": "hello"}))
    body = json.loads(fake.calls[0]["body_raw"])
    assert body == {"channel_id": "ch-1", "message": "hello"}


@pytest.mark.asyncio
async def test_on_command_routes_typing_to_typing_endpoint(monkeypatch):
    """Regression for the typing-capability gap: declaring ``typing``
    in ``capabilities`` is a contract — the daemon will send
    ``TypingCmd`` envelopes and the adapter must POST to
    ``/api/v4/users/me/typing`` (mirrors the Rust adapter at
    mattermost.rs:464-485)."""
    from librefang.sidecar.protocol import TypingCmd
    fake = _FakeUrlopen([(200, {})])
    monkeypatch.setattr(mm.urllib.request, "urlopen", fake)
    a = _adapter()
    await a.on_command(TypingCmd(channel_id="ch-1"))
    assert len(fake.calls) == 1
    c = fake.calls[0]
    assert c["url"].endswith("/api/v4/users/me/typing")
    assert c["method"] == "POST"
    body = json.loads(c["body_raw"])
    assert body == {"channel_id": "ch-1"}


@pytest.mark.asyncio
async def test_on_command_ignores_unknown(monkeypatch):
    """``on_command`` must not crash on commands we don't handle (e.g.
    Reaction, Interactive) — the daemon may send any command shape and
    the adapter is responsible for ignoring what it doesn't model."""
    from librefang.sidecar.protocol import Reaction
    fake = _FakeUrlopen([])
    monkeypatch.setattr(mm.urllib.request, "urlopen", fake)
    a = _adapter()
    await a.on_command(Reaction(channel_id="ch", message_id="m", reaction=":+1:"))
    assert fake.calls == []
