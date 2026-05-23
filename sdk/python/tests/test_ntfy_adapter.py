"""Tests for librefang.sidecar.adapters.ntfy.

Deterministic, no network: urllib is monkeypatched. Asserts the
sidecar ntfy adapter preserves the behaviour of the removed in-process
Rust `librefang-channels::ntfy` adapter.
"""

import io
import os

import pytest

os.environ.setdefault("NTFY_TOPIC", "test-topic")
from librefang.sidecar.adapters import ntfy as na  # noqa: E402


def _adapter(**env):
    for k, v in {
        "NTFY_TOPIC": "test-topic",
        "NTFY_SERVER_URL": "",
        "NTFY_TOKEN": "",
        "NTFY_ACCOUNT_ID": "",
    }.items():
        os.environ[k] = env.get(k, v)
    return na.NtfyAdapter()


def test_split_message_prefers_newline_and_caps_length():
    assert na._split_message("short", 4096) == ["short"]
    body = ("a" * 100 + "\n") * 60  # 6060 chars, newline-separable
    chunks = na._split_message(body, 4096)
    assert all(len(c) <= 4096 for c in chunks)
    assert "".join(c if c.endswith("\n") else c + "\n"
                    for c in chunks).replace("\n\n", "\n").strip() \
        .replace("\n", "") == body.strip().replace("\n", "")
    # A single oversized token still gets hard-split.
    hard = na._split_message("x" * 9000, 4096)
    assert [len(c) for c in hard] == [4096, 4096, 808]


def test_parse_event_message_and_skips():
    a = _adapter()
    ok = a._parse_event(
        '{"id":"abc","event":"message","topic":"t","message":"hi",'
        '"title":"Alice"}'
    )
    assert ok == ("abc", "hi", "t", "Alice")
    # keepalive / open / empty-message / missing-id / invalid → None
    assert a._parse_event('{"id":"k","event":"keepalive","topic":"t"}') is None
    assert a._parse_event('{"id":"o","event":"open","topic":"t"}') is None
    assert a._parse_event(
        '{"id":"e","event":"message","topic":"t","message":""}'
    ) is None
    assert a._parse_event('{"event":"message","message":"x"}') is None
    assert a._parse_event("not json") is None


def test_to_event_text_command_sender_group_metadata():
    a = _adapter()
    # Plain text, title → sender; is_group; topic metadata.
    ev = a._to_event("id1", "hello world", "mytopic", "Bob")
    p = ev["params"]
    assert ev["method"] == "message"
    assert p["content"] == {"Text": "hello world"}
    assert p["user_id"] == "Bob" and p["user_name"] == "Bob"
    assert p["is_group"] is True
    assert p["metadata"] == {"topic": "mytopic"}

    # No title → default sender; leading "/" → Command with args.
    ev2 = a._to_event("id2", "/deploy prod now", "t", None)["params"]
    assert ev2["user_id"] == "ntfy-user"
    assert ev2["content"] == {
        "Command": {"name": "deploy", "args": ["prod", "now"]}
    }

    # Bare command, no args.
    ev3 = a._to_event("id3", "/status", "t", None)["params"]
    assert ev3["content"] == {"Command": {"name": "status", "args": []}}


def test_publish_posts_chunked_plaintext_with_title_and_auth(monkeypatch):
    captured = []

    class _Resp:
        status = 200

        def __enter__(self):
            return self

        def __exit__(self, *a):
            return False

    def fake_urlopen(req, *a, **k):
        captured.append(req)
        return _Resp()

    monkeypatch.setattr(na.urllib.request, "urlopen", fake_urlopen)

    a = _adapter(NTFY_TOKEN="tok123", NTFY_SERVER_URL="https://ntfy.example/")
    a._publish("x" * 5000)  # > MAX_MESSAGE_LEN → 2 chunks

    assert len(captured) == 2
    for req in captured:
        assert req.full_url == "https://ntfy.example/test-topic"
        assert req.get_method() == "POST"
        # urllib lowercases header keys via .headers
        assert req.headers["Content-type"] == "text/plain"
        assert req.headers["Title"] == "LibreFang"
        assert req.headers["Authorization"] == "Bearer tok123"
    assert sum(len(r.data) for r in captured) == 5000
    # No auth header when no token.
    captured.clear()
    b = _adapter()
    b._publish("hi")
    assert "Authorization" not in captured[0].headers


@pytest.mark.asyncio
async def test_on_send_text_vs_unsupported(monkeypatch):
    sent = []
    monkeypatch.setattr(na.NtfyAdapter, "_publish",
                        lambda self, text: sent.append(text))
    a = _adapter()

    class Cmd:
        def __init__(self, text, content):
            self.text = text
            self.content = content

    await a.on_send(Cmd("hello", {"Text": "hello"}))
    await a.on_send(Cmd("", {"Image": {"url": "u"}}))
    await a.on_send(Cmd("plain", None))
    assert sent == ["hello", "(Unsupported content type)", "plain"]


# ---- 429 / Retry-After (ntfy rate-limiting) ---------------------


class _HdrShim:
    """Mimic the parts of ``email.message.Message`` the adapter
    touches — only ``.items()``."""

    def __init__(self, hdrs):
        self._hdrs = hdrs or {}

    def items(self):
        return list(self._hdrs.items())


def _http_error_429(url: str, resp_hdrs: dict | None = None):
    return na.urllib.error.HTTPError(
        url, 429, "Too Many Requests",
        _HdrShim(resp_hdrs or {}),
        io.BytesIO(b"throttled"),
    )


def test_retry_after_secs_parses_header_value():
    """``Retry-After`` (seconds form) is parsed as a float and capped
    at ``MAX_BACKOFF_SECS`` so a misreported value can't block the
    producer for more than a minute."""
    assert na.NtfyAdapter._retry_after_secs({"retry-after": "5"}) == 5.0
    assert na.NtfyAdapter._retry_after_secs({"retry-after": "0.5"}) == 1.0
    assert (
        na.NtfyAdapter._retry_after_secs({"retry-after": "9999"})
        == na.MAX_BACKOFF_SECS
    )


def test_retry_after_secs_falls_back_when_absent_or_invalid():
    """Without a ``Retry-After`` (or with an HTTP-date form we don't
    decode), fall back to ``RETRY_AFTER_DEFAULT_SECS`` rather than
    busy-looping at 1 s."""
    assert (
        na.NtfyAdapter._retry_after_secs({})
        == na.RETRY_AFTER_DEFAULT_SECS
    )
    assert (
        na.NtfyAdapter._retry_after_secs(
            {"retry-after": "Thu, 01 Jan 2099 00:00:00 GMT"},
        )
        == na.RETRY_AFTER_DEFAULT_SECS
    )


def test_sse_loop_429_on_subscribe_sleeps_then_raises(monkeypatch):
    """ntfy rate-limits topic subscriptions on hot reconnect. The
    sidecar must honour ``Retry-After`` before raising so the
    `produce` outer backoff doesn't immediately probe inside the
    server-side window."""
    a = _adapter()

    def fake_urlopen(req, *args, **kwargs):
        raise _http_error_429(req.full_url, {"Retry-After": "7"})

    monkeypatch.setattr(na.urllib.request, "urlopen", fake_urlopen)
    sleeps: list = []
    monkeypatch.setattr(na.time, "sleep", lambda s: sleeps.append(s))
    with pytest.raises(RuntimeError, match="429"):
        a._sse_loop(lambda _: None)
    assert sleeps == [7.0]


def test_sse_loop_429_without_header_uses_default(monkeypatch):
    """A 429 with no ``Retry-After`` falls back to
    ``RETRY_AFTER_DEFAULT_SECS`` instead of busy-looping at 1 s."""
    a = _adapter()

    def fake_urlopen(req, *args, **kwargs):
        raise _http_error_429(req.full_url, {})

    monkeypatch.setattr(na.urllib.request, "urlopen", fake_urlopen)
    sleeps: list = []
    monkeypatch.setattr(na.time, "sleep", lambda s: sleeps.append(s))
    with pytest.raises(RuntimeError, match="429"):
        a._sse_loop(lambda _: None)
    assert sleeps == [na.RETRY_AFTER_DEFAULT_SECS]


def test_publish_429_sleeps_retry_after_then_raises(monkeypatch):
    """Per-topic publish 429 must sleep the indicated interval and
    raise. ntfy.sh defaults to ~4800 messages/hour/topic on free tier;
    operators hitting that bucket should not have the sidecar retry
    in tight loop."""
    a = _adapter()

    def fake_urlopen(req, *args, **kwargs):
        raise _http_error_429(req.full_url, {"Retry-After": "6"})

    monkeypatch.setattr(na.urllib.request, "urlopen", fake_urlopen)
    sleeps: list = []
    monkeypatch.setattr(na.time, "sleep", lambda s: sleeps.append(s))
    with pytest.raises(RuntimeError, match="429"):
        a._publish("hi")
    assert sleeps == [6.0]
