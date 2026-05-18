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
    for k, v in {"TELEGRAM_BOT_TOKEN": "T:tok", "ALLOWED_USERS": ""}.items():
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


@pytest.mark.asyncio
async def test_on_send_text_vs_unsupported_and_skips(monkeypatch):
    sent = []
    monkeypatch.setattr(tg.TelegramAdapter, "_send",
                        lambda self, c, t: sent.append((c, t)))
    a = _adapter()

    class Cmd:
        def __init__(self, channel_id, text, content):
            self.channel_id = channel_id
            self.text = text
            self.content = content

    await a.on_send(Cmd("c1", "hi", {"Text": "hi"}))
    await a.on_send(Cmd("c1", "", {"Image": {"url": "u"}}))
    await a.on_send(Cmd("c1", "plain", None))
    await a.on_send(Cmd("", "no-chat", None))      # skipped
    await a.on_send(Cmd("c1", "", None))           # skipped (empty text)
    assert sent == [
        ("c1", "hi"),
        ("c1", "(Unsupported content type)"),
        ("c1", "plain"),
    ]
