"""Tests for librefang.sidecar.adapters.discord.

Deterministic, no network: urllib is monkeypatched. The gateway
WebSocket client is replaced with a fake that yields a scripted frame
sequence so the dispatch loop is exercised without TLS or sockets.

Asserts the sidecar preserves the behaviour of the removed in-process
Rust ``librefang-channels::discord`` adapter, plus one explicitly-
acknowledged improvement: periodic client-side heartbeats. The Rust
adapter captured ``heartbeat_interval`` but never sent its own
heartbeats — connections silently dropped after ~45s with a re-
identify cycle. This sidecar runs proper periodic heartbeats so
sessions survive long-running idle periods.
"""

import io
import json
import os
import urllib.error
import urllib.parse

import pytest


os.environ.setdefault("DISCORD_BOT_TOKEN", "test-bot-token")
from librefang.sidecar.adapters import discord as da  # noqa: E402

from _sidecar_fakes import _FakeResp, _FakeUrlopen, _HdrShim


# ---- _FakeUrlopen scaffolding (shared shape with reddit tests) -----


def _adapter(**env):
    """Construct a DiscordAdapter with deterministic env, suitable for
    unit tests. Each call resets the relevant env vars; missing vars
    fall back to safe defaults."""
    defaults = {
        "DISCORD_BOT_TOKEN": "test-bot-token",
        "DISCORD_ALLOWED_GUILDS": "",
        "DISCORD_ALLOWED_USERS": "",
        "DISCORD_INTENTS": "",
        "DISCORD_IGNORE_BOTS": "",
        "DISCORD_MENTION_PATTERNS": "",
        "DISCORD_ACCOUNT_ID": "",
    }
    for k, v in defaults.items():
        os.environ[k] = env.get(k, v)
    a = da.DiscordAdapter()
    return a


# ---- env handling --------------------------------------------------


def test_default_api_base_and_intents():
    a = _adapter()
    assert a.api_base == "https://discord.com/api/v10"
    assert a.intents == 37376
    assert a.ignore_bots is True
    assert a.account_id is None


def test_missing_token_exits_2():
    os.environ["DISCORD_BOT_TOKEN"] = ""
    with pytest.raises(SystemExit) as exc:
        da.DiscordAdapter()
    assert exc.value.code == 2
    os.environ["DISCORD_BOT_TOKEN"] = "test-bot-token"


def test_custom_intents_and_account_id():
    a = _adapter(DISCORD_INTENTS="513", DISCORD_ACCOUNT_ID="guild-42")
    assert a.intents == 513
    assert a.account_id == "guild-42"


def test_invalid_intents_exits_2():
    with pytest.raises(SystemExit) as exc:
        _adapter(DISCORD_INTENTS="not-a-number")
    assert exc.value.code == 2


def test_ignore_bots_false_via_env():
    a = _adapter(DISCORD_IGNORE_BOTS="false")
    assert a.ignore_bots is False
    a2 = _adapter(DISCORD_IGNORE_BOTS="0")
    assert a2.ignore_bots is False
    a3 = _adapter(DISCORD_IGNORE_BOTS="anything-else")
    assert a3.ignore_bots is True


def test_allowed_lists_split_on_commas_with_whitespace():
    a = _adapter(DISCORD_ALLOWED_GUILDS="111, 222 ,333",
                 DISCORD_ALLOWED_USERS=" user-a , user-b ")
    assert a.allowed_guilds == ["111", "222", "333"]
    assert a.allowed_users == ["user-a", "user-b"]


def test_mention_patterns_split():
    a = _adapter(DISCORD_MENTION_PATTERNS="hey bot , !ask")
    assert a.mention_patterns == ["hey bot", "!ask"]


# ---- _split_to_utf16_chunks ---------------------------------------


def test_split_utf16_ascii_under_limit():
    assert da._split_to_utf16_chunks("hello", 100) == ["hello"]


def test_split_utf16_ascii_at_boundary():
    s = "a" * 2000
    out = da._split_to_utf16_chunks(s, 2000)
    assert out == [s]


def test_split_utf16_ascii_over_boundary():
    s = "a" * 2500
    out = da._split_to_utf16_chunks(s, 2000)
    assert out == ["a" * 2000, "a" * 500]


def test_split_utf16_emoji_uses_two_units():
    # 😀 is U+1F600 → 2 UTF-16 code units. A 5-emoji string has
    # UTF-16 length 10; with limit=4 we should get two chunks of 2
    # emojis (4 units each) plus a final chunk of 1 emoji.
    s = "😀" * 5
    out = da._split_to_utf16_chunks(s, 4)
    assert out == ["😀😀", "😀😀", "😀"]


def test_split_utf16_empty_returns_one_empty():
    assert da._split_to_utf16_chunks("", 100) == [""]


# ---- _split_csv ----------------------------------------------------


def test_split_csv_empty():
    assert da._split_csv("") == []
    assert da._split_csv("   ") == []
    assert da._split_csv(", ,") == []


def test_split_csv_strips_whitespace():
    assert da._split_csv(" a , b,  c ") == ["a", "b", "c"]


# ---- _parse_retry_after --------------------------------------------


def test_retry_after_missing_uses_default():
    assert da._parse_retry_after({}, default_secs=3.0) == 3.0


def test_retry_after_decimal_seconds():
    assert da._parse_retry_after({"retry-after": "1.5"}, default_secs=99) == 1.5


def test_retry_after_garbage_falls_back():
    assert da._parse_retry_after({"retry-after": "soon"}, default_secs=2.0) == 2.0


def test_retry_after_capped():
    huge = da._parse_retry_after({"retry-after": "99999"}, default_secs=1.0)
    assert huge == da.MAX_BACKOFF_SECS


# ---- parse_attachment ---------------------------------------------


def test_attachment_image_with_caption():
    content, ok = da.parse_attachment(
        [{"url": "https://cdn.discord.com/p.png",
          "filename": "p.png",
          "content_type": "image/png"}],
        "look at this",
    )
    assert ok
    assert "Image" in content
    assert content["Image"]["url"] == "https://cdn.discord.com/p.png"
    assert content["Image"]["caption"] == "look at this"
    assert content["Image"]["mime_type"] == "image/png"


def test_attachment_video_no_caption():
    content, ok = da.parse_attachment(
        [{"url": "https://cdn.discord.com/c.mp4",
          "filename": "c.mp4",
          "content_type": "video/mp4"}],
        "",
    )
    assert ok
    assert "Video" in content
    assert content["Video"]["filename"] == "c.mp4"
    assert content["Video"]["caption"] is None


def test_attachment_audio_drops_companion_text():
    # Audio has no caption channel — companion text must NOT be
    # silently attached (the Rust adapter logs and drops).
    content, ok = da.parse_attachment(
        [{"url": "https://cdn.discord.com/v.ogg",
          "filename": "v.ogg",
          "content_type": "audio/ogg"}],
        "ignore me",
    )
    assert ok
    assert "Voice" in content
    assert content["Voice"]["caption"] is None


def test_attachment_file_fallback():
    content, ok = da.parse_attachment(
        [{"url": "https://cdn.discord.com/doc.pdf",
          "filename": "report.pdf",
          "content_type": "application/pdf"}],
        "",
    )
    assert ok
    assert "File" in content
    assert content["File"]["filename"] == "report.pdf"


def test_attachment_missing_url_returns_text_fallback():
    content, ok = da.parse_attachment(
        [{"filename": "no-url.bin", "content_type": "application/octet-stream"}],
        "fallback",
    )
    assert not ok
    assert content == {"Text": "fallback"}


def test_attachment_empty_list_returns_text_fallback():
    content, ok = da.parse_attachment([], "leftover")
    assert not ok
    assert content == {"Text": "leftover"}


# ---- parse_message_create -----------------------------------------


def _msg(**overrides):
    """Build a canonical MESSAGE_CREATE ``d`` payload with a single
    author/content. Overrides win."""
    base = {
        "id": "msg1",
        "channel_id": "ch1",
        "content": "hello",
        "author": {
            "id": "user-1",
            "username": "alice",
            "discriminator": "0",
            "bot": False,
        },
        "timestamp": "2024-01-01T00:00:00+00:00",
    }
    base.update(overrides)
    return base


def test_parse_basic_text():
    ev = da.parse_message_create(
        _msg(content="Hello agent!"),
        bot_user_id="bot-123",
        allowed_guilds=[], allowed_users=[],
        ignore_bots=True, mention_patterns=[],
        account_id=None,
    )
    assert ev is not None
    params = ev["params"]
    # Rust adapter uses channel_id as user_id (=sender.platform_id).
    assert params["user_id"] == "ch1"
    assert params["user_name"] == "alice"
    assert params["content"] == {"Text": "Hello agent!"}
    assert params["message_id"] == "msg1"
    # No guild → not a group.
    assert "is_group" not in params or params["is_group"] is False


def test_parse_filters_self_message():
    ev = da.parse_message_create(
        _msg(author={"id": "bot-123", "username": "librefang",
                     "discriminator": "0"},
             content="my own"),
        bot_user_id="bot-123",
        allowed_guilds=[], allowed_users=[],
        ignore_bots=True, mention_patterns=[],
        account_id=None,
    )
    assert ev is None


def test_parse_ignore_bots_filters_other_bots():
    ev = da.parse_message_create(
        _msg(author={"id": "other-bot", "username": "somebot",
                     "discriminator": "0", "bot": True}),
        bot_user_id="bot-123",
        allowed_guilds=[], allowed_users=[],
        ignore_bots=True, mention_patterns=[],
        account_id=None,
    )
    assert ev is None


def test_parse_ignore_bots_false_allows_other_bots():
    ev = da.parse_message_create(
        _msg(author={"id": "other-bot", "username": "somebot",
                     "discriminator": "0", "bot": True}),
        bot_user_id="bot-123",
        allowed_guilds=[], allowed_users=[],
        ignore_bots=False, mention_patterns=[],
        account_id=None,
    )
    assert ev is not None
    assert ev["params"]["user_name"] == "somebot"


def test_parse_ignore_bots_false_still_filters_self():
    """Even with ignore_bots=False, the bot's own messages MUST stay
    filtered. Matches the Rust precedence: self-id check runs before
    the ignore_bots check."""
    ev = da.parse_message_create(
        _msg(author={"id": "bot-123", "username": "librefang",
                     "discriminator": "0", "bot": True}),
        bot_user_id="bot-123",
        allowed_guilds=[], allowed_users=[],
        ignore_bots=False, mention_patterns=[],
        account_id=None,
    )
    assert ev is None


def test_parse_guild_filter_rejects_unlisted():
    ev = da.parse_message_create(
        _msg(guild_id="999"),
        bot_user_id="bot-123",
        allowed_guilds=["111", "222"], allowed_users=[],
        ignore_bots=True, mention_patterns=[],
        account_id=None,
    )
    assert ev is None


def test_parse_guild_filter_accepts_listed():
    ev = da.parse_message_create(
        _msg(guild_id="111"),
        bot_user_id="bot-123",
        allowed_guilds=["111", "222"], allowed_users=[],
        ignore_bots=True, mention_patterns=[],
        account_id=None,
    )
    assert ev is not None
    assert ev["params"]["is_group"] is True


def test_parse_allowed_users_filter():
    ev = da.parse_message_create(
        _msg(author={"id": "user-9", "username": "bob",
                     "discriminator": "0"}),
        bot_user_id="bot-123",
        allowed_guilds=[], allowed_users=["user-1", "user-2"],
        ignore_bots=True, mention_patterns=[],
        account_id=None,
    )
    assert ev is None


def test_parse_slash_command():
    ev = da.parse_message_create(
        _msg(content="/agent hello-world"),
        bot_user_id=None,
        allowed_guilds=[], allowed_users=[],
        ignore_bots=True, mention_patterns=[],
        account_id=None,
    )
    assert ev is not None
    assert ev["params"]["content"] == {
        "Command": {"name": "agent", "args": ["hello-world"]},
    }


def test_parse_slash_command_no_args():
    ev = da.parse_message_create(
        _msg(content="/ping"),
        bot_user_id=None,
        allowed_guilds=[], allowed_users=[],
        ignore_bots=True, mention_patterns=[],
        account_id=None,
    )
    assert ev["params"]["content"] == {"Command": {"name": "ping", "args": []}}


def test_parse_empty_content_no_attachment_skipped():
    ev = da.parse_message_create(
        _msg(content=""),
        bot_user_id=None,
        allowed_guilds=[], allowed_users=[],
        ignore_bots=True, mention_patterns=[],
        account_id=None,
    )
    assert ev is None


def test_parse_discriminator_legacy_format():
    ev = da.parse_message_create(
        _msg(author={"id": "user-1", "username": "alice",
                     "discriminator": "1234"}),
        bot_user_id=None,
        allowed_guilds=[], allowed_users=[],
        ignore_bots=True, mention_patterns=[],
        account_id=None,
    )
    assert ev["params"]["user_name"] == "alice#1234"


def test_parse_attachment_only():
    ev = da.parse_message_create(
        _msg(content="",
             attachments=[{"url": "https://cdn.discord.com/x.pdf",
                           "filename": "x.pdf",
                           "content_type": "application/pdf"}]),
        bot_user_id=None,
        allowed_guilds=[], allowed_users=[],
        ignore_bots=True, mention_patterns=[],
        account_id=None,
    )
    assert ev is not None
    assert "File" in ev["params"]["content"]


def test_parse_attachment_takes_priority_over_slash_command():
    """When both attachment and ``/cmd`` text are present, the
    attachment wins and the command text becomes the image caption.
    Matches the explicit comment in parse_discord_message."""
    ev = da.parse_message_create(
        _msg(content="/upload photo",
             attachments=[{"url": "https://cdn.discord.com/p.png",
                           "filename": "p.png",
                           "content_type": "image/png"}]),
        bot_user_id=None,
        allowed_guilds=[], allowed_users=[],
        ignore_bots=True, mention_patterns=[],
        account_id=None,
    )
    assert ev is not None
    assert "Image" in ev["params"]["content"]
    assert ev["params"]["content"]["Image"]["caption"] == "/upload photo"


def test_parse_mention_via_mentions_array():
    ev = da.parse_message_create(
        _msg(guild_id="g1",
             content="hey <@bot-123> help",
             mentions=[{"id": "bot-123", "username": "librefang"}]),
        bot_user_id="bot-123",
        allowed_guilds=[], allowed_users=[],
        ignore_bots=True, mention_patterns=[],
        account_id=None,
    )
    assert ev["params"]["metadata"]["was_mentioned"] is True


def test_parse_mention_via_content_tag_no_array():
    ev = da.parse_message_create(
        _msg(guild_id="g1", content="hey <@!bot-123> help"),
        bot_user_id="bot-123",
        allowed_guilds=[], allowed_users=[],
        ignore_bots=True, mention_patterns=[],
        account_id=None,
    )
    assert ev["params"]["metadata"]["was_mentioned"] is True


def test_parse_mention_via_custom_pattern_case_insensitive():
    ev = da.parse_message_create(
        _msg(content="HEY BOT, help me"),
        bot_user_id="bot-123",
        allowed_guilds=[], allowed_users=[],
        ignore_bots=True, mention_patterns=["hey bot"],
        account_id=None,
    )
    assert ev["params"]["metadata"]["was_mentioned"] is True


def test_parse_no_mention_omits_metadata_flag():
    ev = da.parse_message_create(
        _msg(guild_id="g1", content="just chatting"),
        bot_user_id="bot-123",
        allowed_guilds=[], allowed_users=[],
        ignore_bots=True, mention_patterns=[],
        account_id=None,
    )
    assert ev is not None
    md = ev["params"].get("metadata") or {}
    assert "was_mentioned" not in md


def test_parse_account_id_injected_into_metadata():
    ev = da.parse_message_create(
        _msg(),
        bot_user_id=None,
        allowed_guilds=[], allowed_users=[],
        ignore_bots=True, mention_patterns=[],
        account_id="guild-42",
    )
    assert ev["params"]["metadata"]["account_id"] == "guild-42"


def test_parse_dm_not_group():
    ev = da.parse_message_create(
        _msg(),  # no guild_id
        bot_user_id=None,
        allowed_guilds=[], allowed_users=[],
        ignore_bots=True, mention_patterns=[],
        account_id=None,
    )
    assert ev is not None
    # DM messages don't set is_group (or set it false). The protocol
    # builder only emits is_group when True, so we check it's absent.
    assert "is_group" not in ev["params"] or not ev["params"]["is_group"]


# ---- Gateway dispatch state machine -------------------------------


def test_handle_payload_ready_captures_session_state():
    a = _adapter()
    a._handle_payload(
        {
            "op": da.OP_DISPATCH, "s": 1, "t": "READY",
            "d": {
                "user": {"id": "bot-xyz", "username": "librefang"},
                "session_id": "sess-123",
                "resume_gateway_url": "wss://gateway-x.discord.gg",
            },
        },
        ws=_FakeWs(),
        emit=lambda _e: None,
    )
    assert a.bot_user_id == "bot-xyz"
    assert a.session_id == "sess-123"
    assert a.resume_gateway_url == "wss://gateway-x.discord.gg"
    assert a.last_seq == 1


def test_handle_payload_invalid_session_non_resumable_clears_state():
    a = _adapter()
    a.session_id = "old"
    a.last_seq = 99
    a.resume_gateway_url = "wss://old"
    ws = _FakeWs()
    with pytest.raises(RuntimeError, match="invalid_session"):
        a._handle_payload({"op": da.OP_INVALID_SESSION, "d": False},
                          ws=ws, emit=lambda _e: None)
    assert a.session_id is None
    assert a.last_seq is None
    assert a.resume_gateway_url is None


def test_handle_payload_invalid_session_resumable_preserves_state():
    a = _adapter()
    a.session_id = "keep"
    a.last_seq = 42
    a.resume_gateway_url = "wss://keep"
    ws = _FakeWs()
    with pytest.raises(RuntimeError, match="invalid_session"):
        a._handle_payload({"op": da.OP_INVALID_SESSION, "d": True},
                          ws=ws, emit=lambda _e: None)
    assert a.session_id == "keep"
    assert a.last_seq == 42
    assert a.resume_gateway_url == "wss://keep"


def test_handle_payload_reconnect_raises():
    a = _adapter()
    with pytest.raises(RuntimeError, match="reconnect"):
        a._handle_payload({"op": da.OP_RECONNECT}, ws=_FakeWs(),
                          emit=lambda _e: None)


def test_handle_payload_dispatch_message_create_emits():
    a = _adapter()
    a.bot_user_id = "bot-1"
    emitted = []
    a._handle_payload(
        {
            "op": da.OP_DISPATCH, "s": 2, "t": "MESSAGE_CREATE",
            "d": _msg(content="hi"),
        },
        ws=_FakeWs(),
        emit=emitted.append,
    )
    assert len(emitted) == 1
    assert emitted[0]["method"] == "message"
    assert a.last_seq == 2


def test_handle_payload_dispatch_message_update_emits():
    a = _adapter()
    a.bot_user_id = "bot-1"
    emitted = []
    a._handle_payload(
        {
            "op": da.OP_DISPATCH, "s": 3, "t": "MESSAGE_UPDATE",
            "d": _msg(content="edited"),
        },
        ws=_FakeWs(),
        emit=emitted.append,
    )
    assert len(emitted) == 1
    assert emitted[0]["params"]["content"] == {"Text": "edited"}


def test_handle_payload_server_heartbeat_responds_immediately():
    a = _adapter()
    a.last_seq = 11
    ws = _FakeWs()
    sent_hb = a._handle_payload(
        {"op": da.OP_HEARTBEAT}, ws=ws, emit=lambda _e: None,
    )
    assert ws.sent_text
    payload = json.loads(ws.sent_text[0])
    assert payload == {"op": da.OP_HEARTBEAT, "d": 11}
    # The handler signals to `_run_session` that it already sent one
    # beat, so the outer scheduler must slide its next-deadline
    # forward and not double-beat on the next iteration.
    assert sent_hb is True


def test_handle_payload_dispatch_does_not_signal_heartbeat_sent():
    a = _adapter()
    a.bot_user_id = "bot-1"
    sent_hb = a._handle_payload(
        {
            "op": da.OP_DISPATCH, "s": 5, "t": "MESSAGE_CREATE",
            "d": _msg(content="hi"),
        },
        ws=_FakeWs(),
        emit=lambda _e: None,
    )
    assert sent_hb is False, \
        "DISPATCH must not claim to have sent a heartbeat — that would " \
        "make the outer loop skip a real beat and let the gateway drop the " \
        "connection after ~45s."


def test_raise_close_translates_fatal_code():
    with pytest.raises(da._FatalGatewayError, match="4014"):
        da.DiscordAdapter._raise_close((4014, b"disallowed intent"))


def test_raise_close_silent_on_non_fatal_code():
    # 1000 / 4000 etc. should not raise — the outer loop reconnects.
    da.DiscordAdapter._raise_close((1000, b"normal"))
    da.DiscordAdapter._raise_close((4000, b"transient"))


# ---- Fake WebSocket client -----------------------------------------


class _FakeWs:
    """Minimal stand-in for ``_WebSocketClient`` used by the handler
    tests. Records every ``send_text`` payload and lets the caller
    script ``recv_frame`` outputs."""

    def __init__(self, frames=None):
        self.sent_text = []
        self.sent_close = False
        self.frames = list(frames or [])
        self.timeouts = []
        self.readable_calls = 0

    def send_text(self, s):
        self.sent_text.append(s)

    def send_close(self):
        self.sent_close = True

    def settimeout(self, t):
        self.timeouts.append(t)

    def wait_readable(self, timeout):
        self.readable_calls += 1
        # Return True even when out of frames so the next recv_frame()
        # raises EOFError (signalling "stream closed"), letting
        # _run_session exit cleanly. Returning False here would loop
        # the heartbeat path forever in tests.
        return True

    def recv_frame(self):
        if not self.frames:
            raise EOFError("scripted close")
        return self.frames.pop(0)


# ---- _fetch_gateway_url -------------------------------------------


def test_fetch_gateway_url_appends_query(monkeypatch):
    fake = _FakeUrlopen([(200, {"url": "wss://gateway.discord.gg"})])
    monkeypatch.setattr(da.urllib.request, "urlopen", fake)
    a = _adapter()
    url = a._fetch_gateway_url()
    assert url == "wss://gateway.discord.gg?v=10&encoding=json"
    assert fake.calls[0]["url"].endswith("/gateway/bot")
    assert fake.calls[0]["headers"]["authorization"] == "Bot test-bot-token"


def test_fetch_gateway_url_429_raises(monkeypatch):
    fake = _FakeUrlopen([(429, {"message": "rate limited"})])
    monkeypatch.setattr(da.urllib.request, "urlopen", fake)
    a = _adapter()
    with pytest.raises(RuntimeError, match="429"):
        a._fetch_gateway_url()


def test_fetch_gateway_url_missing_url_raises(monkeypatch):
    fake = _FakeUrlopen([(200, {"shards": 1})])
    monkeypatch.setattr(da.urllib.request, "urlopen", fake)
    a = _adapter()
    with pytest.raises(RuntimeError, match="missing 'url'"):
        a._fetch_gateway_url()


# ---- _send_message: REST shape, chunking, 429 retry ---------------


def test_send_posts_channel_message_with_bot_auth(monkeypatch):
    fake = _FakeUrlopen([(200, {"id": "1", "channel_id": "ch1"})])
    monkeypatch.setattr(da.urllib.request, "urlopen", fake)
    a = _adapter()
    a._send_message("ch1", "hello from librefang")
    call = fake.calls[0]
    assert call["url"].endswith("/channels/ch1/messages")
    assert call["method"] == "POST"
    assert call["headers"]["authorization"] == "Bot test-bot-token"
    assert call["headers"]["content-type"] == "application/json"
    assert json.loads(call["body_raw"]) == {"content": "hello from librefang"}


def test_send_chunks_long_message_at_2000_utf16(monkeypatch):
    # 3000 ASCII chars → two chunks (2000, 1000).
    fake = _FakeUrlopen([
        (200, {"id": "1"}),
        (200, {"id": "2"}),
    ])
    monkeypatch.setattr(da.urllib.request, "urlopen", fake)
    a = _adapter()
    a._send_message("ch1", "a" * 3000)
    assert len(fake.calls) == 2
    assert json.loads(fake.calls[0]["body_raw"])["content"] == "a" * 2000
    assert json.loads(fake.calls[1]["body_raw"])["content"] == "a" * 1000


def test_send_429_honours_retry_after_and_retries_once(monkeypatch):
    # First 429 with Retry-After, then 200. _send_message must retry
    # exactly once and not double-send.
    fake = _FakeUrlopen([
        (429, {"message": "rate limited"},
         {"Retry-After": "0.05"}),
        (200, {"id": "1"}),
    ])
    monkeypatch.setattr(da.urllib.request, "urlopen", fake)
    a = _adapter()
    a._send_message("ch1", "ping")
    assert len(fake.calls) == 2


def test_send_second_429_logs_and_returns(monkeypatch):
    """A second 429 must surface as a warn-and-drop (matching the
    Rust adapter's fail-open send-path) rather than blocking the
    send loop with another retry."""
    fake = _FakeUrlopen([
        (429, {"message": "again"}, {"Retry-After": "0.01"}),
        (429, {"message": "still"}, {"Retry-After": "0.01"}),
    ])
    monkeypatch.setattr(da.urllib.request, "urlopen", fake)
    a = _adapter()
    # Must NOT raise — fail-open behaviour.
    a._send_message("ch1", "ping")
    assert len(fake.calls) == 2


def test_send_5xx_logs_and_continues(monkeypatch):
    fake = _FakeUrlopen([(500, {"message": "server"})])
    monkeypatch.setattr(da.urllib.request, "urlopen", fake)
    a = _adapter()
    a._send_message("ch1", "ping")  # must not raise


# ---- on_send routing ----------------------------------------------


@pytest.mark.asyncio
async def test_on_send_routes_text_to_channel_id(monkeypatch):
    fake = _FakeUrlopen([(200, {"id": "1"})])
    monkeypatch.setattr(da.urllib.request, "urlopen", fake)
    a = _adapter()

    class _Cmd:
        channel_id = "ch-xyz"
        text = "hi"
        content = None
        thread_id = None
        user = {}

    await a.on_send(_Cmd())
    assert fake.calls[0]["url"].endswith("/channels/ch-xyz/messages")
    assert json.loads(fake.calls[0]["body_raw"]) == {"content": "hi"}


@pytest.mark.asyncio
async def test_on_send_falls_back_to_user_platform_id(monkeypatch):
    fake = _FakeUrlopen([(200, {"id": "1"})])
    monkeypatch.setattr(da.urllib.request, "urlopen", fake)
    a = _adapter()

    class _Cmd:
        channel_id = ""
        text = "fallback"
        content = None
        thread_id = None
        user = {"platform_id": "ch-fallback"}

    await a.on_send(_Cmd())
    assert fake.calls[0]["url"].endswith("/channels/ch-fallback/messages")


@pytest.mark.asyncio
async def test_on_send_non_text_content_uses_placeholder(monkeypatch):
    fake = _FakeUrlopen([(200, {"id": "1"})])
    monkeypatch.setattr(da.urllib.request, "urlopen", fake)
    a = _adapter()

    class _Cmd:
        channel_id = "ch1"
        text = ""
        content = {"Command": {"name": "noop", "args": []}}
        thread_id = None
        user = {}

    await a.on_send(_Cmd())
    assert json.loads(fake.calls[0]["body_raw"]) == {
        "content": "(Unsupported content type)",
    }


@pytest.mark.asyncio
async def test_on_send_drops_when_no_channel_id(monkeypatch):
    # If neither channel_id nor user.platform_id is set, we must drop
    # rather than POST to "/channels//messages" (which Discord 404s).
    fake = _FakeUrlopen([])  # no calls expected
    monkeypatch.setattr(da.urllib.request, "urlopen", fake)
    a = _adapter()

    class _Cmd:
        channel_id = ""
        text = "orphan"
        content = None
        thread_id = None
        user = {}

    await a.on_send(_Cmd())
    assert fake.calls == []


# ---- _run_session uses the heartbeat scheduling path --------------


def test_run_session_sends_identify_when_no_session(monkeypatch):
    """After HELLO, with no saved session, the adapter must send
    IDENTIFY (not RESUME). Captured via FakeWs.sent_text[0]."""
    a = _adapter()
    a.session_id = None
    a.last_seq = None
    a.resume_gateway_url = None
    ws = _FakeWs(frames=[
        # HELLO with a tiny heartbeat_interval so the rest of the
        # session test is fast.
        (json.dumps({"op": da.OP_HELLO,
                     "d": {"heartbeat_interval": 50}}), None),
    ])
    # After consuming HELLO, FakeWs.recv_frame() will raise EOFError
    # because we have no more frames; that's how we exit _run_session
    # cleanly.
    try:
        a._run_session(ws, lambda _e: None)
    except EOFError:
        pass
    # First sent payload must be IDENTIFY.
    assert ws.sent_text, "expected at least one outbound frame"
    first = json.loads(ws.sent_text[0])
    assert first["op"] == da.OP_IDENTIFY
    assert first["d"]["token"] == "test-bot-token"
    assert first["d"]["intents"] == 37376
    assert first["d"]["properties"]["browser"] == "librefang"


def test_run_session_sends_resume_when_session_known(monkeypatch):
    a = _adapter()
    a.session_id = "sess-1"
    a.last_seq = 7
    a.resume_gateway_url = "wss://gw"
    ws = _FakeWs(frames=[
        (json.dumps({"op": da.OP_HELLO,
                     "d": {"heartbeat_interval": 50}}), None),
    ])
    try:
        a._run_session(ws, lambda _e: None)
    except EOFError:
        pass
    first = json.loads(ws.sent_text[0])
    assert first["op"] == da.OP_RESUME
    assert first["d"]["session_id"] == "sess-1"
    assert first["d"]["seq"] == 7


def test_run_session_emits_message_create(monkeypatch):
    """End-to-end: HELLO + READY + MESSAGE_CREATE → one emit."""
    a = _adapter()
    a.session_id = None
    ready_d = {
        "user": {"id": "bot-1", "username": "lf"},
        "session_id": "sess-A",
        "resume_gateway_url": "wss://x",
    }
    msg_d = _msg(content="hello world")
    ws = _FakeWs(frames=[
        (json.dumps({"op": da.OP_HELLO,
                     "d": {"heartbeat_interval": 50}}), None),
        (json.dumps({"op": da.OP_DISPATCH, "s": 1, "t": "READY",
                     "d": ready_d}), None),
        (json.dumps({"op": da.OP_DISPATCH, "s": 2, "t": "MESSAGE_CREATE",
                     "d": msg_d}), None),
    ])
    emitted = []
    try:
        a._run_session(ws, emitted.append)
    except EOFError:
        pass
    assert a.bot_user_id == "bot-1"
    assert a.session_id == "sess-A"
    assert len(emitted) == 1
    assert emitted[0]["params"]["content"] == {"Text": "hello world"}
