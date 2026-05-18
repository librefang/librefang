"""Tests for librefang.sidecar.adapters.telegram.

Deterministic, no network: urllib is monkeypatched. Importing this
module at all proves the adapter is stdlib-only (no `requests`) — the
whole point of the follow-up to #5228. Behaviour parity with the
former hand-rolled adapter is asserted via the pure helpers.
"""

import os

import pytest

os.environ.setdefault("TELEGRAM_BOT_TOKEN", "T:tok")
from librefang.sidecar.adapters import telegram as tg  # noqa: E402


def _adapter(**env):
    # Reset every adapter-read env var each call (overridable) so state
    # never leaks between tests in this in-process suite.
    defaults = {
        "TELEGRAM_BOT_TOKEN": "T:tok",
        "ALLOWED_USERS": "",
        "TELEGRAM_CLEAR_DONE_REACTION": "",
    }
    for k, v in defaults.items():
        os.environ[k] = env.get(k, v)
    return tg.TelegramAdapter()


def test_adapter_is_stdlib_only():
    # The follow-up's whole point: no third-party deps. Importing the
    # module (done above) already proves it loads without `requests`
    # installed; assert the source carries no such import either.
    src = open(tg.__file__, encoding="utf-8").read()
    # No third-party import. (The docstring/comments may *mention*
    # "requests" to explain the migration — only the import matters.)
    assert "import requests" not in src
    assert "\nimport requests" not in src and "requests." not in src


def test_update_to_event_text_sender_channel_platform():
    a = _adapter()
    ev = a._update_to_event({
        "update_id": 7,
        "message": {
            "text": "hello",
            "from": {"id": 42, "first_name": "Alice", "username": "al"},
            "chat": {"id": -100123},
        },
    })
    p = ev["params"]
    assert ev["method"] == "message"
    assert p["user_id"] == "42" and p["user_name"] == "Alice"
    assert p["content"] == {"Text": "hello"}
    assert p["channel_id"] == "-100123"
    assert p["platform"] == "telegram"

    # username fallback, then "unknown"
    e2 = a._update_to_event({
        "message": {"text": "x", "from": {"id": 1, "username": "bob"},
                    "chat": {"id": 9}},
    })["params"]
    assert e2["user_name"] == "bob"
    e3 = a._update_to_event({
        "message": {"text": "x", "from": {"id": 2}, "chat": {"id": 9}},
    })["params"]
    assert e3["user_name"] == "unknown"

    # non-text and no-message are skipped
    assert a._update_to_event({"message": {"from": {"id": 1},
                                           "chat": {"id": 9}}}) is None
    assert a._update_to_event({"edited_message": {"text": "x"}}) is None


def test_update_to_event_whitelist():
    a = _adapter(ALLOWED_USERS="111, 222")
    mk = lambda uid: {  # noqa: E731
        "message": {"text": "hi", "from": {"id": uid}, "chat": {"id": 1}}
    }
    assert a._update_to_event(mk(999)) is None          # not allowed
    assert a._update_to_event(mk(111))["params"]["user_id"] == "111"


def test_api_get_ok_httperror_body_and_timeout(monkeypatch):
    import io
    import urllib.error

    class _R:
        def __init__(self, b):
            self._b = b

        def __enter__(self):
            return self

        def __exit__(self, *a):
            return False

        def read(self):
            return self._b

    def http_error(code, body):
        return urllib.error.HTTPError("u", code, "e", {}, io.BytesIO(body))

    # 200 OK → parsed straight through.
    monkeypatch.setattr(tg.urllib.request, "urlopen",
                        lambda *a, **k: _R(b'{"ok":true,"result":[]}'))
    assert tg._api_get("u", {}, 1) == {"ok": True, "result": []}

    # HTTPError with a JSON body → body surfaced, not raised
    # (Telegram returns {"ok":false} with a 4xx).
    monkeypatch.setattr(
        tg.urllib.request, "urlopen",
        lambda *a, **k: (_ for _ in ()).throw(
            http_error(401, b'{"ok":false,"error_code":401}')))
    assert tg._api_get("u", {}, 1) == {"ok": False, "error_code": 401}

    # HTTPError with a non-JSON body → synthesised ok:false.
    monkeypatch.setattr(
        tg.urllib.request, "urlopen",
        lambda *a, **k: (_ for _ in ()).throw(http_error(502, b"nope")))
    got = tg._api_get("u", {}, 1)
    assert got["ok"] is False and "502" in got["error"]

    # URLError wrapping a timeout → re-raised as TimeoutError so the
    # poll loop treats it as a normal empty long-poll.
    monkeypatch.setattr(
        tg.urllib.request, "urlopen",
        lambda *a, **k: (_ for _ in ()).throw(
            urllib.error.URLError(TimeoutError("timed out"))))
    with pytest.raises(TimeoutError):
        tg._api_get("u", {}, 1)


def test_poll_once_emits_and_advances_offset(monkeypatch):
    a = _adapter()
    payload = {"ok": True, "result": [
        {"update_id": 10, "message": {"text": "a", "from": {"id": 5},
                                      "chat": {"id": 8}}},
        {"update_id": 11, "message": {"text": "b", "from": {"id": 5},
                                      "chat": {"id": 8}}},
    ]}
    monkeypatch.setattr(tg, "_api_get", lambda *a, **k: payload)
    out = []
    state = {"offset": 0}
    a._poll_once(out.append, state)
    assert [e["params"]["content"]["Text"] for e in out] == ["a", "b"]
    assert state["offset"] == 12  # last update_id + 1

    monkeypatch.setattr(tg, "_api_get",
                        lambda *a, **k: {"ok": False, "error_code": 409})
    with pytest.raises(RuntimeError):
        a._poll_once(out.append, {"offset": 0})


from librefang.sidecar import protocol  # noqa: E402


def test_utf16_len_and_chunks16_split_on_surrogates_and_newline():
    assert tg._utf16_len("abc") == 3
    assert tg._utf16_len("😀") == 2  # astral char = 2 UTF-16 units
    # Hard split at the UTF-16 limit, never inside a surrogate pair.
    out = tg._chunks16("x" * 4090 + "😀" * 5, 4096)
    assert len(out) == 2
    assert all(tg._utf16_len(c) <= 4096 for c in out)
    assert "".join(out) == "x" * 4090 + "😀" * 5
    # Prefer a newline boundary when one exists in the window.
    body = ("a" * 1000 + "\n") * 6  # 6006 chars, newline-separable
    parts = tg._chunks16(body, 4096)
    assert len(parts) > 1
    assert all(tg._utf16_len(p) <= 4096 for p in parts)
    # Content preserved; only boundary newlines may be consumed at cuts.
    assert "".join(parts).replace("\n", "") == body.replace("\n", "")


def test_map_reaction_matches_rust_table():
    assert tg._map_reaction("⏳") == "👀"
    assert tg._map_reaction("⚙️") == "⚡"
    assert tg._map_reaction("✅") == "🎉"
    assert tg._map_reaction("❌") == "👎"
    assert tg._map_reaction("🤔") == "🤔"  # passthrough


@pytest.mark.asyncio
async def test_on_command_send_chunks_and_threads(monkeypatch):
    calls = []
    monkeypatch.setattr(tg.TelegramAdapter, "_call",
                        lambda self, m, p: calls.append((m, p)) or {})
    a = _adapter()

    await a.on_command(protocol.Send("c1", "hi", {"Text": "hi"}, None, {}))
    await a.on_command(protocol.Send("c2", "x", {"Text": "x"}, "777", {}))
    await a.on_command(protocol.Send("c3", "", {"Image": {"url": "u"}},
                                     None, {}))
    await a.on_command(protocol.Send("", "no-chat", None, None, {}))  # skip
    sends = [p for (m, p) in calls if m == "sendMessage"]
    assert sends[0]["chat_id"] == "c1" and sends[0]["text"] == "hi"
    assert "message_thread_id" not in sends[0]
    assert sends[1]["message_thread_id"] == "777"      # forum thread
    assert sends[2]["text"] == "(Unsupported content type)"
    assert len(sends) == 3                              # empty-chat skipped


@pytest.mark.asyncio
async def test_on_command_typing_reaction_interactive(monkeypatch):
    calls = []
    monkeypatch.setattr(tg.TelegramAdapter, "_call",
                        lambda self, m, p: calls.append((m, p)) or {})

    a = _adapter()
    await a.on_command(protocol.TypingCmd("c1"))
    await a.on_command(protocol.Reaction("c1", "55", "✅"))   # mapped → 🎉
    await a.on_command(protocol.Interactive("c1", {
        "text": "pick", "buttons": [[
            {"label": "Yes", "action": "y"},
            {"label": "Docs", "url": "https://x"},
        ]]}))
    by = {m: p for (m, p) in calls}
    assert by["sendChatAction"] == {"chat_id": "c1", "action": "typing"}
    assert by["setMessageReaction"]["message_id"] == 55
    assert by["setMessageReaction"]["reaction"] == [
        {"type": "emoji", "emoji": "🎉"}]
    kb = by["sendMessage"]["reply_markup"]["inline_keyboard"]
    assert kb == [[{"text": "Yes", "callback_data": "y"},
                   {"text": "Docs", "url": "https://x"}]]

    # clear-on-done when configured
    calls.clear()
    b = _adapter(TELEGRAM_CLEAR_DONE_REACTION="1")
    await b.on_command(protocol.Reaction("c1", "9", "✅"))
    assert calls[0][1]["reaction"] == []


@pytest.mark.asyncio
async def test_on_command_streaming_initial_then_throttled_edit(monkeypatch):
    calls = []

    def fake_call(self, method, payload):
        calls.append((method, payload))
        return {"result": {"message_id": 4242}}

    monkeypatch.setattr(tg.TelegramAdapter, "_call", fake_call)
    a = _adapter()

    await a.on_command(protocol.StreamStart("c1", "s1"))
    await a.on_command(protocol.StreamDelta("s1", "Hel"))   # first → send
    await a.on_command(protocol.StreamDelta("s1", "lo"))     # throttled
    await a.on_command(protocol.StreamEnd("s1"))             # final edit

    methods = [m for (m, _) in calls]
    assert methods[0] == "sendMessage"
    assert "editMessageText" in methods
    final = [p for (m, p) in calls if m == "editMessageText"][-1]
    assert final["message_id"] == 4242 and final["text"] == "Hello"
    assert "s1" not in a._streams  # cleaned up on end


@pytest.mark.asyncio
async def test_on_send_text_vs_unsupported_and_skips(monkeypatch):
    sent = []
    monkeypatch.setattr(tg.TelegramAdapter, "_send_text",
                        lambda self, c, t, th=None: sent.append((c, t, th)))
    a = _adapter()

    class Cmd:
        def __init__(self, channel_id, text, content, thread_id=None):
            self.channel_id = channel_id
            self.text = text
            self.content = content
            self.thread_id = thread_id

    await a.on_send(Cmd("c1", "hi", {"Text": "hi"}))
    await a.on_send(Cmd("c1", "", {"Image": {"url": "u"}}))
    await a.on_send(Cmd("c1", "plain", None, "9"))
    await a.on_send(Cmd("", "no-chat", None))      # skipped
    await a.on_send(Cmd("c1", "", None))           # skipped (empty text)
    assert sent == [
        ("c1", "hi", None),
        ("c1", "(Unsupported content type)", None),
        ("c1", "plain", "9"),
    ]
