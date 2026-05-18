"""Tests for librefang.sidecar.adapters.telegram.

Deterministic, no network: HTTP is monkeypatched. Importing this
module proves the adapter is stdlib-only (no `requests`). The
formatter / sanitizer / chunker assertions are pinned to the Rust
oracles they are ported from (crate::formatter, crate::message_truncator,
telegram.rs sanitize_telegram_html) so the two implementations cannot
drift apart silently.
"""

import os

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
