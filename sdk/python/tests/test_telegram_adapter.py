"""Tests for librefang.sidecar.adapters.telegram.

Deterministic, no network: HTTP is monkeypatched. Importing this
module proves the adapter is stdlib-only (no `requests`). The
formatter / sanitizer / chunker assertions are pinned to the Rust
oracles they are ported from (crate::formatter, crate::message_truncator,
telegram.rs sanitize_telegram_html) so the two implementations cannot
drift apart silently.
"""

import io
import os
import time
import urllib.error

import pytest

os.environ.setdefault("TELEGRAM_BOT_TOKEN", "T:tok")
from librefang.sidecar.adapters import telegram as tg  # noqa: E402


def _adapter(**env):
    defaults = {
        "TELEGRAM_BOT_TOKEN": "T:tok",
        "ALLOWED_USERS": "",
        "TELEGRAM_CLEAR_DONE_REACTION": "",
    }
    for k, v in defaults.items():
        os.environ[k] = env.get(k, v)
    return tg.TelegramAdapter()


def test_adapter_is_stdlib_only():
    src = open(tg.__file__, encoding="utf-8").read()
    assert "import requests" not in src
    assert "\nimport requests" not in src and "requests." not in src


# ---- formatter: byte-exact vs crate::formatter::tests ---------------


def test_markdown_to_telegram_html_matches_rust_oracle():
    m = tg.markdown_to_telegram_html
    assert m("Hello **world**!") == "Hello <b>world</b>!"
    assert m("Hello *world*!") == "Hello <i>world</i>!"
    assert m("Use `println!`") == "Use <code>println!</code>"
    assert m("[click here](https://example.com)") == (
        '<a href="https://example.com">click here</a>')
    assert m("## Result") == "<b>Result</b>"
    assert m("- alpha\n- beta") == "• alpha\n• beta"
    assert m("1. alpha\n2. beta") == "1. alpha\n2. beta"
    assert m("```rust\nfn main() {}\n```") == (
        "<pre><code>fn main() {}</code></pre>")
    assert m("> note\n> second line") == (
        "<blockquote>note\nsecond line</blockquote>")
    # paragraphs joined by blank line → "\n\n"
    assert m("para one\n\npara two") == "para one\n\npara two"
    # HTML-significant chars escaped before tag synthesis
    assert m("a < b & c > d") == "a &lt; b &amp; c &gt; d"


def test_markdown_link_query_ampersand_is_escaped_once():
    assert tg._format_and_sanitize(
        "[results](https://example.com/search?a=1&b=2)",
    ) == '<a href="https://example.com/search?a=1&amp;b=2">results</a>'


# ---- sanitizer: vs telegram.rs sanitize_telegram_html tests --------


def test_sanitize_telegram_html():
    s = tg.sanitize_telegram_html
    assert s("<b>bold</b>") == "<b>bold</b>"
    # unclosed allowed tag is balanced at the end
    assert s("<b>bold") == "<b>bold</b>"
    # unknown tag escaped, brackets too
    assert s("<thinking>hi</thinking>") == (
        "&lt;thinking&gt;hi&lt;/thinking&gt;")
    # <a> keeps only a safe href, attribute value escaped
    assert s('<a href="https://x?a=1&b=2">k</a>') == (
        '<a href="https://x?a=1&amp;b=2">k</a>')
    # javascript: scheme → opening <a> dropped (never enters open_tags);
    # the now-unmatched </a> is then escaped, exactly like Rust.
    assert s('<a href="javascript:alert(1)">k</a>') == "k&lt;/a&gt;"
    # closing tag never opened → escaped
    assert s("</b>") == "&lt;/b&gt;"
    # lone '<' with no '>' → escaped
    assert s("a < b") == "a &lt; b"
    # idempotent on already-safe markup
    once = s("<b>x</b> <i>y</i>")
    assert s(once) == once
    # tg-spoiler / tg-emoji allowed
    assert s("<tg-spoiler>x</tg-spoiler>") == "<tg-spoiler>x</tg-spoiler>"
    # self-closing allowed tag does not wrap all following text
    # (matches telegram.rs's self_closing_allowed_tag_does_not_wrap_following_text)
    assert s("<code/>after") == "<code></code>after"
    assert s('<tg-emoji emoji-id="42" />after') == (
        '<tg-emoji emoji-id="42"></tg-emoji>after')
    # self-closing tag nested inside an open tag does not leak onto the
    # enclosing tag's stack entry (matches telegram.rs's
    # self_closing_tag_nested_inside_open_tag_does_not_leak_onto_stack)
    assert s("<b>before<code/>after</b>tail") == (
        "<b>before<code></code>after</b>tail")
    # self-closing tag with trailing whitespace before `>` (valid HTML)
    # must still be detected as self-closing, matching sanitize.rs's
    # trim_end()-based check — without the rstrip() this regresses to
    # wrapping the following text instead of closing immediately.
    assert s("<b/ >after") == "<b></b>after"
    assert s("<code/ >after") == "<code></code>after"
    # crossed/mismatched nesting: closing the outer tag while an inner
    # tag is still open must close the inner tag first, matching
    # telegram.rs's stack-drain behavior — not just pop the matched
    # entry and leave the inner tag's close to land after it (which
    # would emit invalid crossed HTML Telegram cannot parse).
    assert s("<b><i>x</b>") == "<b><i>x</i></b>"


# ---- chunker: vs crate::message_truncator -------------------------


def test_utf16_and_chunking():
    assert tg._utf16_len("abc") == 3
    assert tg._utf16_len("😀") == 2
    out = tg._split_to_utf16_chunks("x" * 4090 + "😀" * 5, 4096)
    assert len(out) == 2
    assert all(tg._utf16_len(c) <= 4096 for c in out)
    assert "".join(out) == "x" * 4090 + "😀" * 5
    # newline boundary preferred
    body = ("a" * 1000 + "\n") * 6
    parts = tg._split_to_utf16_chunks(body, 4096)
    assert len(parts) > 1
    assert "".join(parts).replace("\n", "") == body.replace("\n", "")
    # never split inside an HTML entity
    chunks = tg._split_to_utf16_chunks("y" * 4094 + "&amp;tail", 4096)
    assert all("&am" not in c[-3:] for c in chunks)
    assert "".join(chunks) == "y" * 4094 + "&amp;tail"


def test_extract_retry_after_prefers_header_then_body_then_default():
    # HTTP delta-seconds header wins over the JSON body's
    # parameters.retry_after (matches telegram.rs's resolve_retry_after).
    assert tg._extract_retry_after(
        {"_retry_after_header": "19", "parameters": {"retry_after": 7}}, 5
    ) == 19
    # Non-numeric / HTTP-date header forms fall through to the body value.
    assert tg._extract_retry_after(
        {"_retry_after_header": "not-seconds", "parameters": {"retry_after": 7}}, 5
    ) == 7
    # Negative / signed header is rejected the same way (no negative sleep).
    assert tg._extract_retry_after(
        {"_retry_after_header": "-5", "parameters": {"retry_after": 7}}, 5
    ) == 7
    # Missing header and missing body -> default.
    assert tg._extract_retry_after({}, 5) == 5
    # Whitespace-padded numeric header is still honoured.
    assert tg._extract_retry_after({"_retry_after_header": " 19 "}, 5) == 19
    # No header at all still reads the body (pre-existing behavior).
    assert tg._extract_retry_after({"parameters": {"retry_after": 7}}, 5) == 7


def test_api_post_and_multipart_propagate_retry_after_header(monkeypatch):
    class FakeHeaders:
        def __init__(self, value):
            self._value = value

        def get(self, _key, default=None):
            return self._value if self._value is not None else default

    class FakeResponse:
        def __init__(self, body: bytes, retry_after):
            self._body = body
            self.headers = FakeHeaders(retry_after)

        def read(self):
            return self._body

        def __enter__(self):
            return self

        def __exit__(self, *exc):
            return False

    # Success path (2xx): header is stashed alongside the parsed body.
    monkeypatch.setattr(
        tg.urllib.request, "urlopen",
        lambda *a, **k: FakeResponse(b'{"ok": true}', "13"))
    resp = tg._api_post("https://x", {}, 1.0)
    assert resp["_retry_after_header"] == "13"

    resp2 = tg._multipart("https://x", {}, "f", "n", "m", b"", 1.0)
    assert resp2["_retry_after_header"] == "13"

    # No header present: key is absent, not None (callers use .get()).
    monkeypatch.setattr(
        tg.urllib.request, "urlopen",
        lambda *a, **k: FakeResponse(b'{"ok": true}', None))
    resp3 = tg._api_post("https://x", {}, 1.0)
    assert "_retry_after_header" not in resp3

    # Non-2xx path: header is stashed alongside `_http`.
    def raise_http_error(*_a, **_k):
        err = urllib.error.HTTPError(
            "https://x", 429, "Too Many Requests",
            {"Retry-After": "21"}, io.BytesIO(b'{"ok": false}'))
        raise err

    monkeypatch.setattr(tg.urllib.request, "urlopen", raise_http_error)
    resp4 = tg._api_post("https://x", {}, 1.0)
    assert resp4["_http"] == 429
    assert resp4["_retry_after_header"] == "21"


def test_call_retrying_skips_sleep_above_retry_after_cap(monkeypatch):
    # A flood-wait retry_after above MAX_RETRY_AFTER_SECS must not sleep
    # (an attacker-controlled or misbehaving server returning e.g. a
    # multi-hour delay must not hang the sidecar indefinitely) — the
    # original 429 response is returned unmodified instead.
    a = _adapter()
    calls = []
    monkeypatch.setattr(
        a, "_call",
        lambda method, payload: {
            "_http": 429,
            "parameters": {"retry_after": tg.MAX_RETRY_AFTER_SECS + 1},
        })
    monkeypatch.setattr(tg.time, "sleep",
                         lambda secs: calls.append(secs))
    resp = a._call_retrying("sendMessage", {"chat_id": 1, "text": "x"})
    assert calls == []
    assert resp["_http"] == 429

    # A delay within the cap still sleeps and retries exactly once.
    responses = iter([
        {"_http": 429, "parameters": {"retry_after": 1}},
        {"ok": True},
    ])
    monkeypatch.setattr(a, "_call", lambda method, payload: next(responses))
    resp2 = a._call_retrying("sendMessage", {"chat_id": 1, "text": "x"})
    assert calls == [1]
    assert resp2 == {"ok": True}


def test_send_media_upload_skips_sleep_above_retry_after_cap(monkeypatch):
    a = _adapter()
    calls = []
    monkeypatch.setattr(
        tg, "_multipart",
        lambda *a_, **k_: {
            "_http": 429,
            "parameters": {"retry_after": tg.MAX_RETRY_AFTER_SECS + 1},
        })
    monkeypatch.setattr(tg.time, "sleep", lambda secs: calls.append(secs))
    resp = a._send_media_upload("sendPhoto", "photo", 1, b"data", "f.png",
                                "image/png")
    assert calls == []
    assert resp["_http"] == 429


def test_truncate_utf8_callback_data():
    assert tg._truncate_utf8("abc", 64) == "abc"
    assert tg._truncate_utf8("x" * 100, 64) == "x" * 64


# ---- reaction map (vs telegram.rs map_reaction_emoji) --------------


def test_map_reaction_matches_rust_table():
    assert tg._map_reaction("⏳") == "👀"
    assert tg._map_reaction("⚙️") == "⚡"
    assert tg._map_reaction("✅") == "🎉"
    assert tg._map_reaction("❌") == "👎"
    assert tg._map_reaction("🤔") == "🤔"


# ---- inbound parsing ----------------------------------------------


def test_inbound_text_command_sender_allowed():
    a = _adapter()
    ev = a._update_to_event({
        "update_id": 7,
        "message": {
            "text": "hello",
            "from": {"id": 42, "first_name": "Alice", "last_name": "B",
                     "username": "al"},
            "chat": {"id": -100123, "type": "supergroup"},
            "message_id": 9,
        },
    })
    p = ev["params"]
    assert p["user_id"] == "42" and p["user_name"] == "Alice B"
    assert p["content"] == {"Text": "hello"}
    assert p["channel_id"] == "-100123" and p["platform"] == "telegram"
    assert p["is_group"] is True and p["message_id"] == "9"

    cmd = a._update_to_event({
        "message": {
            "text": "/start@bot arg1 arg2",
            "entities": [{"type": "bot_command", "offset": 0, "length": 6}],
            "from": {"id": 1}, "chat": {"id": 1},
        },
    })["params"]
    assert cmd["content"] == {"Command": {"name": "start",
                                          "args": ["arg1", "arg2"]}}

    # sender_chat fallback
    sc = a._update_to_event({
        "message": {"text": "x", "sender_chat": {"id": -55, "title": "Chan"},
                    "chat": {"id": -55}},
    })["params"]
    assert sc["user_id"] == "-55" and sc["user_name"] == "Chan"


def test_inbound_mentions_and_sender_user_id():
    a = _adapter()
    # "@ambrogio help me" in a supergroup: a mention entity is surfaced as a
    # routable name, and the individual sender id is attached so the bridge
    # can scope a per-peer conversation claim (#5323).
    ev = a._update_to_event({
        "message": {
            "text": "@ambrogio help me",
            "entities": [{"type": "mention", "offset": 0, "length": 9}],
            "from": {"id": 42, "first_name": "Alice"},
            "chat": {"id": -100123, "type": "supergroup"},
            "message_id": 9,
        },
    })
    meta = ev["params"]["metadata"]
    assert meta["mention_names"] == ["ambrogio"]
    assert meta["sender_user_id"] == "42"

    # Emoji before the mention: entity offsets are UTF-16 code units, so the
    # extractor must slice in UTF-16 to stay correct.
    ev2 = a._update_to_event({
        "message": {
            "text": "😀 @ambrogio",
            "entities": [{"type": "mention", "offset": 3, "length": 9}],
            "from": {"id": 7, "first_name": "Bob"},
            "chat": {"id": -100123, "type": "supergroup"},
            "message_id": 10,
        },
    })
    assert ev2["params"]["metadata"]["mention_names"] == ["ambrogio"]

    # DM with no mentions: lean envelope, no metadata block.
    dm = a._update_to_event({
        "message": {"text": "hi", "from": {"id": 42, "first_name": "A"},
                    "chat": {"id": 42, "type": "private"}, "message_id": 1},
    })
    assert dm["params"].get("metadata") is None


def test_inbound_allowed_users_by_id_and_username():
    a = _adapter(ALLOWED_USERS="111, @Alice")
    mk = lambda f: {"message": {"text": "hi", "from": f,  # noqa: E731
                                "chat": {"id": 1}}}
    assert a._update_to_event(mk({"id": 999})) is None
    assert a._update_to_event(mk({"id": 111}))["params"]["user_id"] == "111"
    assert a._update_to_event(
        mk({"id": 7, "username": "alice"}))["params"]["user_id"] == "7"


def test_inbound_media_and_getfile(monkeypatch):
    a = _adapter()
    monkeypatch.setattr(
        a, "_get_file_url",
        lambda fid: f"https://api.telegram.org/file/botT:tok/p/{fid}.jpg")

    photo = a._update_to_event({"message": {
        "photo": [{"file_id": "small"}, {"file_id": "big"}],
        "caption": "cap", "from": {"id": 1}, "chat": {"id": 2}}})["params"]
    assert photo["content"]["Image"]["url"].endswith("big.jpg")
    assert photo["content"]["Image"]["caption"] == "cap"
    assert photo["content"]["Image"]["mime_type"] == "image/jpeg"

    loc = a._update_to_event({"message": {
        "location": {"latitude": 1.5, "longitude": -2.0},
        "from": {"id": 1}, "chat": {"id": 2}}})["params"]
    assert loc["content"] == {"Location": {"lat": 1.5, "lon": -2.0}}

    stk = a._update_to_event({"message": {
        "sticker": {"file_id": "S1"},
        "from": {"id": 1}, "chat": {"id": 2}}})["params"]
    assert stk["content"] == {"Sticker": {"file_id": "S1"}}

    # unsupported type → dropped
    assert a._update_to_event({"message": {
        "dice": {"value": 3}, "from": {"id": 1}, "chat": {"id": 2}}}) is None


def test_inbound_getfile_failure_text_fallback(monkeypatch):
    a = _adapter()
    monkeypatch.setattr(a, "_get_file_url", lambda fid: None)
    ev = a._update_to_event({"message": {
        "voice": {"file_id": "v", "duration": 5},
        "from": {"id": 1}, "chat": {"id": 2}}})["params"]
    assert ev["content"] == {"Text": "[Voice message, 5s]"}


def test_inbound_callback_query(monkeypatch):
    a = _adapter()
    answered = []
    monkeypatch.setattr(a, "_call",
                        lambda m, p: answered.append((m, p)) or {})
    ev = a._update_to_event({"callback_query": {
        "id": "cq1", "data": "do_it",
        "from": {"id": 5, "first_name": "Z"},
        "message": {"message_id": 88, "text": "pick",
                    "chat": {"id": 9, "type": "private"}},
    }})["params"]
    assert ev["content"] == {"ButtonCallback": {"action": "do_it",
                                                "message_text": "pick"}}
    assert ev["message_id"] == "88" and ev["channel_id"] == "9"
    assert ev["metadata"]["callback_query_id"] == "cq1"
    assert ("answerCallbackQuery", {"callback_query_id": "cq1"}) in answered


def test_inbound_poll_answer_matches_rust_oracle():
    # Oracle: crates/librefang-channels/src/telegram.rs:2129-2230 (the
    # in-process `poll_answer` handler). Telegram only fires
    # poll_answer for non-anonymous polls in private chats, so
    # `user.id` doubles as the DM chat_id (see SENDER_USER_ID_KEY
    # comment on line 2161). Field-by-field mirror.
    a = _adapter()
    ev = a._update_to_event({
        "update_id": 42,
        "poll_answer": {
            "poll_id": "p-9001",
            "user": {"id": 12345, "first_name": "Alice", "last_name": "B",
                     "username": "al"},
            "option_ids": [0, 2],
        },
    })
    assert ev is not None, "poll_answer must produce an event"
    p = ev["params"]
    # sender.platform_id = user.id; display_name = "first last"
    assert p["user_id"] == "12345"
    assert p["user_name"] == "Alice B"
    assert p["username"] == "al"
    # content = PollAnswer { poll_id, option_ids }
    assert p["content"] == {
        "PollAnswer": {"poll_id": "p-9001", "option_ids": [0, 2]}
    }
    # platform_message_id = poll_id
    assert p["message_id"] == "p-9001"
    # DM-only — channel_id falls back to the user id (Rust comment)
    assert p["channel_id"] == "12345"
    # is_group = false, no thread_id
    assert p.get("is_group", False) is False
    assert "thread_id" not in p
    # metadata mirrors the two keys the Rust path writes
    assert p["metadata"] == {"user_id": "12345", "sender_user_id": "12345"}


def test_inbound_poll_answer_user_only_first_name():
    # Rust: `last_name` defaults to empty; display_name = first_name
    # alone when last_name is empty.
    a = _adapter()
    ev = a._update_to_event({
        "poll_answer": {
            "poll_id": "p1",
            "user": {"id": 7, "first_name": "Solo"},
            "option_ids": [1],
        },
    })
    assert ev["params"]["user_name"] == "Solo"


def test_inbound_poll_answer_missing_first_name_defaults_unknown():
    # Rust: `first_name` defaults to "Unknown" when absent.
    a = _adapter()
    ev = a._update_to_event({
        "poll_answer": {
            "poll_id": "p1",
            "user": {"id": 8},
            "option_ids": [],
        },
    })
    assert ev["params"]["user_name"] == "Unknown"
    # empty option_ids is valid (user retracted their vote)
    assert ev["params"]["content"] == {
        "PollAnswer": {"poll_id": "p1", "option_ids": []}
    }


def test_inbound_poll_answer_empty_poll_id_dropped():
    # Rust: `if !poll_id.is_empty() && ...` — empty poll_id skipped.
    a = _adapter()
    assert a._update_to_event({
        "poll_answer": {"poll_id": "", "user": {"id": 1}, "option_ids": []},
    }) is None
    # Same for missing poll_id entirely.
    assert a._update_to_event({
        "poll_answer": {"user": {"id": 1}, "option_ids": []},
    }) is None


def test_inbound_poll_answer_respects_allowed_users():
    # Rust: `telegram_user_allowed(&allowed_users, user_id, username)`.
    a = _adapter(ALLOWED_USERS="111,@alice")
    # disallowed id+username
    assert a._update_to_event({
        "poll_answer": {"poll_id": "p", "user": {"id": 999},
                        "option_ids": [0]},
    }) is None
    # allowed by id
    assert a._update_to_event({
        "poll_answer": {"poll_id": "p", "user": {"id": 111},
                        "option_ids": [0]},
    })["params"]["user_id"] == "111"
    # allowed by username (case-insensitive, @ optional — see _allowed)
    assert a._update_to_event({
        "poll_answer": {"poll_id": "p",
                        "user": {"id": 7, "username": "Alice"},
                        "option_ids": [0]},
    })["params"]["user_id"] == "7"


def test_poll_answer_in_allowed_updates_subscription():
    # The long-poll request must subscribe to `poll_answer` or
    # Telegram simply never delivers it. Mirrors telegram.rs:2008.
    captured = {}

    def fake_get(url, params, timeout):
        captured["params"] = params
        return {"ok": True, "result": []}

    import librefang.sidecar.adapters.telegram as tg_mod
    a = _adapter()
    orig = tg_mod._api_get
    tg_mod._api_get = fake_get
    try:
        a._poll_once(lambda _ev: None, {"offset": 0})
    finally:
        tg_mod._api_get = orig
    import json as _json
    subs = _json.loads(captured["params"]["allowed_updates"])
    assert "poll_answer" in subs
    # don't accidentally drop existing subscriptions
    for required in ("message", "edited_message", "callback_query"):
        assert required in subs


def test_inbound_edited_message_and_reply(monkeypatch):
    a = _adapter()
    monkeypatch.setattr(a, "_get_file_url", lambda fid: None)
    ev = a._update_to_event({"edited_message": {
        "text": "fixed", "from": {"id": 1, "first_name": "E"},
        "chat": {"id": 3},
        "reply_to_message": {"from": {"first_name": "Q"}, "text": "orig"},
    }})["params"]
    assert ev["content"] == {"Text": '[Replying to Q: "orig"]\nfixed'}


# ---- outbound dispatch (all ChannelContent variants) --------------


@pytest.mark.asyncio
async def test_on_command_send_dispatches_every_variant(monkeypatch):
    calls = []
    monkeypatch.setattr(tg.TelegramAdapter, "_call",
                        lambda self, m, p: calls.append((m, p)) or {})
    a = _adapter()

    async def send(content):
        await a.on_command(tg.protocol.Send("c1", "", content, None, {}))

    await send({"Text": "**hi**"})
    await send({"Image": {"url": "u", "caption": "c"}})
    await send({"File": {"url": "u", "filename": "d.pdf"}})
    await send({"Voice": {"url": "u", "caption": None}})
    await send({"Video": {"url": "u", "caption": "v"}})
    await send({"Location": {"lat": 1.0, "lon": 2.0}})
    await send({"Sticker": {"file_id": "S"}})
    await send({"Animation": {"url": "u", "caption": None}})
    await send({"Audio": {"url": "u", "caption": None, "title": "T",
                          "performer": "P"}})
    await send({"Poll": {"question": "q?", "options": ["a", "b"],
                         "is_quiz": False}})
    await send({"Interactive": {"text": "pick", "buttons": [[
        {"label": "Yes", "action": "y"}]]}})
    await send({"DeleteMessage": {"message_id": "12"}})

    by = {}
    for m, p in calls:
        by.setdefault(m, []).append(p)
    assert by["sendMessage"][0]["text"] == "<b>hi</b>"
    assert by["sendMessage"][0]["parse_mode"] == "HTML"
    assert by["sendPhoto"][0] == {"photo": "u", "caption": "c",
                                  "parse_mode": "HTML", "chat_id": "c1"}
    assert by["sendDocument"][0]["document"] == "u"
    assert by["sendVoice"][0]["voice"] == "u"
    assert by["sendVideo"][0]["caption"] == "v"
    assert by["sendLocation"][0] == {"latitude": 1.0, "longitude": 2.0,
                                     "chat_id": "c1"}
    assert by["sendSticker"][0]["sticker"] == "S"
    assert by["sendAnimation"][0]["animation"] == "u"
    assert by["sendAudio"][0]["title"] == "T"
    assert by["sendPoll"][0]["question"] == "q?"
    assert by["sendPoll"][0]["type"] == "regular"
    kb = by["sendMessage"][1]["reply_markup"]["inline_keyboard"]
    assert kb == [[{"text": "Yes", "callback_data": "y"}]]
    assert by["deleteMessage"][0]["message_id"] == 12


@pytest.mark.asyncio
async def test_send_text_plain_fallback_on_parse_error(monkeypatch):
    calls = []

    def fake(self, method, payload):
        calls.append((method, payload))
        if len(calls) == 1:
            return {"_http": 400, "description": "Bad Request: can't "
                    "parse entities: unexpected"}
        return {"ok": True}

    monkeypatch.setattr(tg.TelegramAdapter, "_call", fake)
    a = _adapter()
    await a.on_command(tg.protocol.Send("c1", "x", {"Text": "<b>x"}, None, {}))
    assert calls[0][1]["parse_mode"] == "HTML"
    assert "parse_mode" not in calls[1][1]


def test_send_text_returns_first_chunk_response(monkeypatch):
    monkeypatch.setattr(tg, "TELEGRAM_MSG_LIMIT", 4)
    ids = iter((101, 102, 103))
    monkeypatch.setattr(
        tg.TelegramAdapter, "_call",
        lambda self, method, payload: {
            "ok": True, "result": {"message_id": next(ids)},
        },
    )
    a = _adapter()

    response = a._send_text("c1", "abcdefghij")

    assert response["result"]["message_id"] == 101


def test_interactive_send_and_edit_format_markdown(monkeypatch):
    calls = []
    monkeypatch.setattr(
        tg.TelegramAdapter, "_call",
        lambda self, method, payload: calls.append((method, payload)) or {},
    )
    a = _adapter()

    a._send_interactive("c1", "**pick**", [], None)
    a._edit_interactive("c1", 7, "`updated`", [])

    assert calls[0][1]["text"] == "<b>pick</b>"
    assert calls[1][1]["text"] == "<code>updated</code>"


def test_media_group_skips_unknown_item_without_invalid_request(monkeypatch):
    calls = []
    monkeypatch.setattr(
        tg.TelegramAdapter, "_call",
        lambda self, method, payload: calls.append((method, payload)) or {},
    )
    a = _adapter()

    result = a._send_media_group(
        "c1", [{"Photo": {"url": "p"}}, {"Audio": {"url": "a"}}], None,
    )

    assert result == {}
    assert calls == []


@pytest.mark.asyncio
async def test_streaming_initial_then_throttled_edit(monkeypatch):
    calls = []

    def fake_call(self, method, payload):
        calls.append((method, payload))
        return {"result": {"message_id": 4242}}

    monkeypatch.setattr(tg.TelegramAdapter, "_call", fake_call)
    a = _adapter()
    await a.on_command(tg.protocol.StreamStart("c1", "s1"))
    await a.on_command(tg.protocol.StreamDelta("s1", "Hel"))
    await a.on_command(tg.protocol.StreamDelta("s1", "lo"))
    await a.on_command(tg.protocol.StreamEnd("s1"))
    methods = [m for m, _ in calls]
    assert methods[0] == "sendMessage"
    assert "editMessageText" in methods
    final = [p for m, p in calls if m == "editMessageText"][-1]
    assert final["message_id"] == 4242 and final["text"] == "Hello"
    assert "s1" not in a._streams


@pytest.mark.asyncio
async def test_streaming_tracks_and_edits_every_message_chunk(monkeypatch):
    monkeypatch.setattr(tg, "TELEGRAM_MSG_LIMIT", 4)
    monkeypatch.setattr(tg, "STREAM_EDIT_INTERVAL", 0.0)
    calls = []
    next_id = 100

    def fake_call(self, method, payload):
        nonlocal next_id
        calls.append((method, payload))
        if method == "sendMessage":
            next_id += 1
            return {"ok": True, "result": {"message_id": next_id}}
        return {"ok": True}

    monkeypatch.setattr(tg.TelegramAdapter, "_call", fake_call)
    a = _adapter()
    await a.on_command(tg.protocol.StreamStart("c1", "multi"))
    await a.on_command(tg.protocol.StreamDelta("multi", "abcdefghij"))
    await a.on_command(tg.protocol.StreamDelta("multi", "kl"))
    await a.on_command(tg.protocol.StreamEnd("multi"))

    sends = [p for method, p in calls if method == "sendMessage"]
    edits = [p for method, p in calls if method == "editMessageText"]
    assert [p["message_id"] for p in edits[-3:]] == [101, 102, 103]
    assert len(sends) == 3
    assert all(tg._utf16_len(p["text"]) <= 4 for p in sends + edits)


@pytest.mark.asyncio
async def test_failed_initial_stream_send_is_not_retried_per_delta(monkeypatch):
    calls = []
    monkeypatch.setattr(
        tg.TelegramAdapter, "_call",
        lambda self, method, payload: calls.append((method, payload)) or {
            "_http": 503,
        },
    )
    a = _adapter()
    await a.on_command(tg.protocol.StreamStart("c1", "failed"))
    await a.on_command(tg.protocol.StreamDelta("failed", "a"))
    await a.on_command(tg.protocol.StreamDelta("failed", "b"))
    await a.on_command(tg.protocol.StreamDelta("failed", "c"))

    assert [m for m, _ in calls] == ["sendMessage"]
    await a.on_command(tg.protocol.StreamEnd("failed"))
    assert [m for m, _ in calls] == ["sendMessage", "sendMessage"]


@pytest.mark.asyncio
async def test_typing_reaction_clear_on_done(monkeypatch):
    calls = []
    monkeypatch.setattr(tg.TelegramAdapter, "_call",
                        lambda self, m, p: calls.append((m, p)) or {})
    a = _adapter()
    await a.on_command(tg.protocol.TypingCmd("c1"))
    await a.on_command(tg.protocol.Reaction("c1", "55", "✅"))
    by = {m: p for m, p in calls}
    assert by["sendChatAction"] == {"chat_id": "c1", "action": "typing"}
    assert by["setMessageReaction"]["reaction"] == [
        {"type": "emoji", "emoji": "🎉"}]

    calls.clear()
    b = _adapter(TELEGRAM_CLEAR_DONE_REACTION="1")
    await b.on_command(tg.protocol.Reaction("c1", "9", "✅"))
    assert calls[0][1]["reaction"] == []


@pytest.mark.asyncio
async def test_on_send_text_and_content(monkeypatch):
    sent = []
    monkeypatch.setattr(
        tg.TelegramAdapter, "_dispatch_content",
        lambda self, c, ct, th: sent.append(("content", c, ct)))
    monkeypatch.setattr(
        tg.TelegramAdapter, "_send_text",
        lambda self, c, t, th=None: sent.append(("text", c, t)))
    a = _adapter()

    class Cmd:
        def __init__(self, channel_id, text, content, thread_id=None):
            self.channel_id = channel_id
            self.text = text
            self.content = content
            self.thread_id = thread_id

    await a.on_send(Cmd("c1", "hi", {"Text": "hi"}))
    await a.on_send(Cmd("c1", "plain", None))
    await a.on_send(Cmd("", "no-chat", None))
    assert sent == [
        ("content", "c1", {"Text": "hi"}),
        ("text", "c1", "plain"),
    ]


from librefang.sidecar import protocol  # noqa: E402,F401


# ---- reconnect / produce loop (#5111) -------------------------------


@pytest.mark.asyncio
async def test_produce_recovers_after_startup_network_failure(monkeypatch):
    """First poll raises (DNS / TCP / proxy failure on startup), the
    next poll succeeds → produce() backs off, retries, recovers
    without crashing the sidecar. Pre-#5111 the Rust adapter exited
    on startup failure and the bridge stayed dead; the sidecar's
    while-True/backoff loop must keep retrying until the network
    comes back. The recovery path also logs an INFO line so the
    operator's log timeline shows the reconnect."""
    import asyncio
    import urllib.error

    # Save the real `asyncio.sleep` BEFORE we monkeypatch — the test's
    # fake_sleep needs to actually yield to the event loop, but it
    # MUST use the unpatched callable to avoid infinite recursion
    # (`tg.asyncio` and the `asyncio` import in this test file point
    # at the same module object).
    real_sleep = asyncio.sleep

    a = _adapter()
    calls = {"n": 0}

    def fake_poll(self, emit, state):
        calls["n"] += 1
        if calls["n"] == 1:
            raise urllib.error.URLError("Name or service not known")

    async def fake_sleep(_d):
        await real_sleep(0)

    monkeypatch.setattr(tg.TelegramAdapter, "_poll_once", fake_poll)
    monkeypatch.setattr(tg.asyncio, "sleep", fake_sleep)

    info_calls: list[tuple[str, dict]] = []
    monkeypatch.setattr(tg.log, "info",
                        lambda msg, **kw: info_calls.append((msg, kw)))
    warn_calls: list[tuple[str, dict]] = []
    monkeypatch.setattr(tg.log, "warn",
                        lambda msg, **kw: warn_calls.append((msg, kw)))

    task = asyncio.create_task(a.produce(lambda _ev: None))
    # Drive the loop until the observable side-effects appear — poll1 → warn → sleep(0) → poll2 → info(recovered) — bounding the wait by wall-clock rather than by a fixed number of event-loop turns.
    # How many turns one produce() iteration costs is not fixed: it depends on how the executor schedules the polling thread, so any constant races on a loaded runner and the failure mode is a confusing "only saw N" assertion rather than a timeout.
    # `fake_sleep` consumes no real time, so this loop spins freely and the deadline only bounds the pathological case.
    deadline = time.monotonic() + 5.0
    while time.monotonic() < deadline:
        await real_sleep(0)
        if info_calls and calls["n"] >= 2:
            break
    task.cancel()
    try:
        await task
    except asyncio.CancelledError:
        pass

    assert calls["n"] >= 2, (
        "produce() must keep polling past the first network failure; "
        f"only saw {calls['n']} polls")
    assert any("backing off" in m for m, _ in warn_calls), (
        f"expected warn on transient failure; got {warn_calls}")
    assert any("recovered" in m for m, _ in info_calls), (
        f"expected info on recovery; got {info_calls}")


@pytest.mark.asyncio
async def test_produce_backoff_is_capped_at_max(monkeypatch):
    """Consecutive failures grow backoff exponentially (1 → 2 → 4 → …)
    and cap at MAX_BACKOFF_SECS so a persistent outage doesn't
    silently push retry intervals to hours. Regression for #5111 's
    "should retry with 2s → 4s → 8s → 16s → 30s (capped) backoff"."""
    import asyncio
    import urllib.error

    real_sleep = asyncio.sleep

    a = _adapter()

    def always_fail(self, emit, state):
        raise urllib.error.URLError("network unreachable")

    delays: list[float] = []

    async def fake_sleep(d):
        delays.append(d)
        # Yield once via the unpatched sleep so the producer can be
        # cancelled cleanly, but do not consume real wall-clock time.
        await real_sleep(0)

    monkeypatch.setattr(tg.TelegramAdapter, "_poll_once", always_fail)
    monkeypatch.setattr(tg.asyncio, "sleep", fake_sleep)
    monkeypatch.setattr(tg.log, "warn", lambda *_a, **_kw: None)

    task = asyncio.create_task(a.produce(lambda _ev: None))
    # The doubling sequence 1, 2, 4, 8, 16, 32, 60, 60, 60 caps after 7 failures, so wait for the cap itself to show up rather than for a fixed number of event-loop turns to elapse.
    # A saturated CI runner drove only 3 of those 7 iterations within 64 turns, failing on `got [1.0, 2.0, 4.0]` — the loop was healthy, the tick budget was not.
    deadline = time.monotonic() + 5.0
    while time.monotonic() < deadline:
        await real_sleep(0)
        if delays and delays[-1] >= tg.MAX_BACKOFF_SECS:
            break
    task.cancel()
    try:
        await task
    except asyncio.CancelledError:
        pass

    assert delays[:3] == [1.0, 2.0, 4.0], (
        f"backoff must double from 1s; got {delays[:3]}")
    assert max(delays) <= tg.MAX_BACKOFF_SECS, (
        f"backoff must cap at MAX_BACKOFF_SECS={tg.MAX_BACKOFF_SECS}; "
        f"got max={max(delays)}, full={delays}")
    assert tg.MAX_BACKOFF_SECS in delays, (
        f"backoff must actually reach the cap; got {delays}")


@pytest.mark.asyncio
async def test_produce_treats_longpoll_timeout_as_normal(monkeypatch):
    """Long-poll timeouts (LONGPOLL_SERVER_SECS expiring with no
    updates) are normal Telegram protocol behaviour, NOT a transport
    failure. They must reset backoff, skip the sleep, and re-enter
    the loop immediately — otherwise an idle channel would slowly
    push backoff to MAX over a few minutes."""
    import asyncio

    real_sleep = asyncio.sleep

    a = _adapter()
    calls = {"n": 0}

    def alternating(self, emit, state):
        calls["n"] += 1
        if calls["n"] % 2 == 1:
            raise TimeoutError("long-poll expired with no updates")

    sleep_calls: list[float] = []

    async def fake_sleep(d):
        sleep_calls.append(d)
        await real_sleep(0)

    monkeypatch.setattr(tg.TelegramAdapter, "_poll_once", alternating)
    monkeypatch.setattr(tg.asyncio, "sleep", fake_sleep)
    monkeypatch.setattr(tg.log, "warn", lambda *_a, **_kw: None)
    monkeypatch.setattr(tg.log, "info", lambda *_a, **_kw: None)

    task = asyncio.create_task(a.produce(lambda _ev: None))
    # Wait for the fourth poll to actually happen rather than for 32 event-loop turns to elapse — the same tick-budget race that broke the two tests above, which surfaced here as `only saw 3 polls`.
    deadline = time.monotonic() + 5.0
    while time.monotonic() < deadline:
        await real_sleep(0)
        if calls["n"] >= 4:
            break
    task.cancel()
    try:
        await task
    except asyncio.CancelledError:
        pass

    assert calls["n"] >= 4, (
        f"produce() must re-poll after a TimeoutError; only saw "
        f"{calls['n']} polls")
    assert sleep_calls == [], (
        "TimeoutError must NOT trigger the backoff sleep; got "
        f"sleeps={sleep_calls}")
