"""Tests for librefang.sidecar.adapters.zulip.

Deterministic, no network: urllib is monkeypatched. Asserts the
sidecar Zulip adapter preserves the behaviour of the removed
in-process Rust ``librefang-channels::zulip`` adapter, plus four
explicitly-acknowledged improvements:

* **P1**: ``thread_id`` is the inbound ``message.subject`` (Zulip
  topic), and ``on_send`` round-trips it as the outbound ``topic``
  on stream sends — fixes the Rust adapter where ``send`` (line
  463 on the migrating tree) hard-coded ``topic="LibreFang"`` for
  every stream reply.
* **P2**: 429 ``Retry-After`` honoured on ``/users/me``,
  ``/register``, ``/events``, and ``/messages`` (Rust had no 429
  handling, only generic exponential backoff at zulip.rs:228-313).
* **P3**: bounded ``message.id`` dedupe (Rust emit was
  unconditional at zulip.rs:434; a queue re-register could
  re-emit).
* **P4**: self-skip prefers ``sender_id == own_user_id`` (the
  stable integer ``/users/me`` returns) with ``sender_email ==
  bot_email`` as fallback — Rust compared only ``sender_email``
  (zulip.rs:357).
"""
from __future__ import annotations

import base64
import io
import json
import os
import urllib.error

import pytest

# Required env must be present at import time because the adapter
# raises SystemExit(2) on missing values.
os.environ.setdefault("ZULIP_SERVER_URL", "https://zulip.example.com")
os.environ.setdefault("ZULIP_BOT_EMAIL", "bot@example.com")
os.environ.setdefault("ZULIP_API_KEY", "test-key")
from librefang.sidecar.adapters import zulip as zu  # noqa: E402

from _sidecar_fakes import _FakeResp, _FakeUrlopen, _HdrShim


def _adapter(**env):
    defaults = {
        "ZULIP_SERVER_URL": "https://zulip.example.com",
        "ZULIP_BOT_EMAIL": "bot@example.com",
        "ZULIP_API_KEY": "test-key",
        "ZULIP_STREAMS": "",
        "ZULIP_ACCOUNT_ID": "",
    }
    for k, v in defaults.items():
        os.environ[k] = env.get(k, v)
    a = zu.ZulipAdapter()
    a.own_user_id = 42  # default for tests that don't go through _validate
    return a


# ---- env handling -------------------------------------------------


def test_required_env_present():
    a = _adapter()
    assert a.server_url == "https://zulip.example.com"
    assert a.bot_email == "bot@example.com"
    assert a.api_key == "test-key"


def test_server_url_trailing_slash_stripped():
    a = _adapter(ZULIP_SERVER_URL="https://zulip.example.com/")
    assert a.server_url == "https://zulip.example.com"


def test_server_url_scheme_validated():
    """Anything other than http:// or https:// is rejected at boot."""
    os.environ["ZULIP_SERVER_URL"] = "ftp://zulip.example.com"
    os.environ["ZULIP_BOT_EMAIL"] = "bot@example.com"
    os.environ["ZULIP_API_KEY"] = "key"
    with pytest.raises(SystemExit):
        zu.ZulipAdapter()


def test_missing_required_env_exits():
    """Empty / whitespace ZULIP_API_KEY exits 2 (one of three)."""
    os.environ["ZULIP_SERVER_URL"] = "https://zulip.example.com"
    os.environ["ZULIP_BOT_EMAIL"] = "bot@example.com"
    os.environ["ZULIP_API_KEY"] = "   "
    with pytest.raises(SystemExit):
        zu.ZulipAdapter()


def test_streams_parsed_comma_separated():
    a = _adapter(ZULIP_STREAMS="engineering, general,dev")
    assert a.allowed_streams == ["engineering", "general", "dev"]


def test_streams_empty_means_all():
    a = _adapter(ZULIP_STREAMS="")
    assert a.allowed_streams == []


def test_account_id_optional():
    a = _adapter(ZULIP_ACCOUNT_ID="prod")
    assert a.account_id == "prod"
    a2 = _adapter(ZULIP_ACCOUNT_ID="")
    assert a2.account_id is None


# ---- helpers ------------------------------------------------------


def test_split_message_under_limit_one_chunk():
    assert zu._split_message("hi", 1000) == ["hi"]


def test_split_message_prefers_newline_cut():
    text = "a" * 100 + "\n" + "b" * 100
    out = zu._split_message(text, 110)
    # Cut at the newline so the first chunk doesn't end mid-word.
    assert out[0] == "a" * 100
    assert out[1].startswith("b")


def test_split_message_hard_cut_when_no_newline():
    text = "a" * 200
    out = zu._split_message(text, 100)
    assert out == ["a" * 100, "a" * 100]


def test_split_message_10000_cap_matches_rust():
    assert zu.ZULIP_MSG_LIMIT == 10_000


def test_split_csv_basic():
    assert zu._split_csv("a, b,c , ,d") == ["a", "b", "c", "d"]


def test_split_csv_empty():
    assert zu._split_csv("") == []


def test_parse_retry_after_parses_seconds():
    assert zu._parse_retry_after({"retry-after": "5"}, default_secs=30) == 5.0
    assert zu._parse_retry_after({"retry-after": "0.5"}, default_secs=30) == 1.0


def test_parse_retry_after_cap_and_floor():
    assert (
        zu._parse_retry_after({"retry-after": "9999"}, default_secs=30)
        == zu.MAX_BACKOFF_SECS
    )
    assert zu._parse_retry_after({"retry-after": "0"}, default_secs=30) == 1.0


def test_parse_retry_after_fallback():
    assert zu._parse_retry_after({}, default_secs=30) == 30.0
    assert (
        zu._parse_retry_after({"retry-after": "wat"}, default_secs=30) == 30.0
    )


# ---- _FakeUrlopen scaffolding -------------------------------------


# ---- auth header --------------------------------------------------


def test_auth_header_basic_auth_shape():
    a = _adapter()
    h = a._auth_headers()
    expected = "Basic " + base64.b64encode(b"bot@example.com:test-key").decode()
    assert h["Authorization"] == expected
    assert "Content-Type" not in h  # only set when form=True
    assert "User-Agent" in h


def test_auth_header_form_content_type():
    a = _adapter()
    h = a._auth_headers(form=True)
    assert h["Content-Type"] == "application/x-www-form-urlencoded"


# ---- parse_zulip_event (pure function) ----------------------------


def _msg(**overrides):
    base = {
        "id": 1001,
        "type": "stream",
        "display_recipient": "engineering",
        "subject": "deploy-checklist",
        "sender_email": "alice@example.com",
        "sender_id": 200,
        "sender_full_name": "Alice",
        "content": "hello zulip",
    }
    base.update(overrides)
    return base


def _event(message=None, eid=1, etype="message"):
    return {
        "id": eid,
        "type": etype,
        "message": message if message is not None else _msg(),
    }


def test_parse_basic_stream_message():
    ev = zu.parse_zulip_event(
        _event(), own_user_id=42, own_email="bot@example.com",
        allowed_streams=[], account_id=None,
    )
    assert ev is not None
    p = ev["params"]
    assert p["user_id"] == "engineering"
    assert p["user_name"] == "Alice"
    assert p["content"] == {"Text": "hello zulip"}
    assert p["message_id"] == "1001"
    assert p["is_group"] is True
    # P1: thread_id is the inbound topic for stream messages.
    assert p["thread_id"] == "deploy-checklist"
    md = p["metadata"]
    assert md["sender_id"] == "200"
    assert md["sender_email"] == "alice@example.com"


def test_parse_basic_dm():
    ev = zu.parse_zulip_event(
        _event(_msg(type="private", display_recipient="", subject="")),
        own_user_id=42, own_email="bot@example.com",
        allowed_streams=[], account_id=None,
    )
    assert ev is not None
    p = ev["params"]
    # platform_id falls back to sender email for DMs (zulip.rs:380-384).
    assert p["user_id"] == "alice@example.com"
    # protocol.message omits `is_group` when False — absence == DM.
    assert p.get("is_group", False) is False
    # No topic on DMs → no thread_id.
    assert "thread_id" not in p or p.get("thread_id") is None


def test_parse_slash_command_routes():
    ev = zu.parse_zulip_event(
        _event(_msg(content="/deploy prod now")),
        own_user_id=42, own_email="bot@example.com",
        allowed_streams=[], account_id=None,
    )
    assert ev is not None
    assert ev["params"]["content"] == {
        "Command": {"name": "deploy", "args": ["prod", "now"]}
    }


def test_parse_slash_command_no_args():
    ev = zu.parse_zulip_event(
        _event(_msg(content="/status")),
        own_user_id=42, own_email="bot@example.com",
        allowed_streams=[], account_id=None,
    )
    assert ev is not None
    assert ev["params"]["content"] == {
        "Command": {"name": "status", "args": []}
    }


def test_parse_self_skip_by_user_id():
    """P4: self-skip prefers stable sender_id over email."""
    ev = zu.parse_zulip_event(
        _event(_msg(sender_id=42, sender_email="renamed@example.com")),
        own_user_id=42, own_email="bot@example.com",
        allowed_streams=[], account_id=None,
    )
    # Self even though the bot got renamed; user_id is stable.
    assert ev is None


def test_parse_self_skip_falls_back_to_email_when_user_id_missing():
    """When sender_id is absent from the payload, fall back to email
    comparison — preserves Rust behaviour for malformed messages."""
    msg = _msg()
    msg.pop("sender_id")
    ev = zu.parse_zulip_event(
        _event(msg),
        own_user_id=42, own_email="alice@example.com",
        allowed_streams=[], account_id=None,
    )
    assert ev is None


def test_parse_non_self_not_skipped():
    """A different sender_id from a different user must not trip
    self-skip even if email coincidentally matches the bot's."""
    ev = zu.parse_zulip_event(
        _event(_msg(sender_id=999)),
        own_user_id=42, own_email="alice@example.com",
        allowed_streams=[], account_id=None,
    )
    assert ev is not None


def test_parse_self_skip_disabled_when_own_user_id_unknown_and_no_email():
    """Defensive: empty own_email AND own_user_id=None means we
    can't self-skip; every message routes."""
    ev = zu.parse_zulip_event(
        _event(),
        own_user_id=None, own_email="",
        allowed_streams=[], account_id=None,
    )
    assert ev is not None


def test_parse_stream_filter_rejects_unlisted():
    ev = zu.parse_zulip_event(
        _event(_msg(display_recipient="random")),
        own_user_id=42, own_email="bot@example.com",
        allowed_streams=["engineering", "general"],
        account_id=None,
    )
    assert ev is None


def test_parse_stream_filter_accepts_listed():
    ev = zu.parse_zulip_event(
        _event(_msg(display_recipient="general")),
        own_user_id=42, own_email="bot@example.com",
        allowed_streams=["engineering", "general"],
        account_id=None,
    )
    assert ev is not None


def test_parse_stream_filter_skipped_when_empty():
    ev = zu.parse_zulip_event(
        _event(_msg(display_recipient="random")),
        own_user_id=42, own_email="bot@example.com",
        allowed_streams=[],
        account_id=None,
    )
    assert ev is not None


def test_parse_account_id_injected():
    ev = zu.parse_zulip_event(
        _event(),
        own_user_id=42, own_email="bot@example.com",
        allowed_streams=[], account_id="prod",
    )
    assert ev["params"]["metadata"]["account_id"] == "prod"


def test_parse_skips_non_message_event_type():
    ev = zu.parse_zulip_event(
        {"id": 1, "type": "presence", "message": _msg()},
        own_user_id=42, own_email="bot@example.com",
        allowed_streams=[], account_id=None,
    )
    assert ev is None


def test_parse_skips_empty_content():
    ev = zu.parse_zulip_event(
        _event(_msg(content="")),
        own_user_id=42, own_email="bot@example.com",
        allowed_streams=[], account_id=None,
    )
    assert ev is None


def test_parse_handles_missing_sender_full_name():
    ev = zu.parse_zulip_event(
        _event(_msg(sender_full_name=None)),
        own_user_id=42, own_email="bot@example.com",
        allowed_streams=[], account_id=None,
    )
    assert ev is not None
    assert ev["params"]["user_name"] == "unknown"


def test_parse_handles_missing_message_id():
    ev = zu.parse_zulip_event(
        _event(_msg(id=None)),
        own_user_id=42, own_email="bot@example.com",
        allowed_streams=[], account_id=None,
    )
    assert ev is not None
    assert "message_id" not in ev["params"] or ev["params"].get("message_id") is None


def test_parse_string_sender_id_coerced():
    """Some payloads return sender_id as a stringified int; coerce
    rather than fall through to the email-fallback path."""
    ev = zu.parse_zulip_event(
        _event(_msg(sender_id="42", sender_email="other@example.com")),
        own_user_id=42, own_email="bot@example.com",
        allowed_streams=[], account_id=None,
    )
    assert ev is None  # self-skip wins via coerced sender_id


def test_parse_malformed_event_returns_none():
    """Non-dict event must return None (defensive)."""
    assert zu.parse_zulip_event(
        "not-a-dict",
        own_user_id=42, own_email="b@e.com",
        allowed_streams=[], account_id=None,
    ) is None


def test_parse_malformed_message_returns_none():
    """Non-dict ``message`` field must return None."""
    ev = zu.parse_zulip_event(
        {"id": 1, "type": "message", "message": "not-a-dict"},
        own_user_id=42, own_email="b@e.com",
        allowed_streams=[], account_id=None,
    )
    assert ev is None


# ---- _mark_seen dedupe --------------------------------------------


def test_mark_seen_first_time_emits():
    a = _adapter()
    assert a._mark_seen("M1") is True


def test_mark_seen_repeat_suppresses():
    a = _adapter()
    assert a._mark_seen("M1") is True
    assert a._mark_seen("M1") is False


def test_mark_seen_empty_id_always_fresh():
    """Empty / None ids aren't dedupe keys — return True so the
    caller still emits when message_id is absent."""
    a = _adapter()
    assert a._mark_seen(None) is True
    assert a._mark_seen("") is True


def test_mark_seen_capacity_eviction():
    """At cap, oldest half evicts. The eviction policy must be
    deterministic: the first SEEN_MESSAGES_EVICT ids are dropped,
    everything else stays."""
    a = _adapter()
    for i in range(zu.SEEN_MESSAGES_MAX):
        a._mark_seen(f"M{i}")
    # Trigger eviction with one more id.
    a._mark_seen(f"M{zu.SEEN_MESSAGES_MAX}")
    # Oldest dropped — now fresh again.
    assert a._mark_seen("M0") is True
    # Most recent retained.
    assert a._mark_seen(f"M{zu.SEEN_MESSAGES_MAX}") is False


# ---- _validate ----------------------------------------------------


def test_validate_happy_path(monkeypatch):
    a = _adapter()
    fake = _FakeUrlopen([
        (200, {"user_id": 42, "full_name": "Bot", "result": "success"}),
    ])
    monkeypatch.setattr(zu.urllib.request, "urlopen", fake)
    uid, name = a._validate()
    assert uid == 42
    assert name == "Bot"
    assert a.own_user_id == 42
    # Bearer-basic auth header injected.
    auth = fake.calls[0]["headers"]["authorization"]
    assert auth.startswith("Basic ")


def test_validate_401_raises(monkeypatch):
    a = _adapter()
    fake = _FakeUrlopen([(401, {"result": "error"})])
    monkeypatch.setattr(zu.urllib.request, "urlopen", fake)
    with pytest.raises(RuntimeError, match="401"):
        a._validate()


def test_validate_missing_user_id_raises(monkeypatch):
    a = _adapter()
    fake = _FakeUrlopen([(200, {"full_name": "Bot"})])
    monkeypatch.setattr(zu.urllib.request, "urlopen", fake)
    with pytest.raises(RuntimeError, match="missing user_id"):
        a._validate()


def test_validate_429_sleeps_then_retries(monkeypatch):
    """P2: a transient 429 with Retry-After must sleep the indicated
    interval then retry once (success on the retry)."""
    a = _adapter()
    fake = _FakeUrlopen([
        (429, {"result": "error"}, {"Retry-After": "2"}),
        (200, {"user_id": 42, "full_name": "Bot"}),
    ])
    monkeypatch.setattr(zu.urllib.request, "urlopen", fake)
    sleeps: list = []
    monkeypatch.setattr(zu.time, "sleep", lambda s: sleeps.append(s))
    uid, _ = a._validate()
    assert uid == 42
    assert sleeps == [2.0]


# ---- _register_queue ----------------------------------------------


def test_register_queue_basic_shape(monkeypatch):
    a = _adapter()
    fake = _FakeUrlopen([
        (200, {"queue_id": "Q1", "last_event_id": 7, "result": "success"}),
    ])
    monkeypatch.setattr(zu.urllib.request, "urlopen", fake)
    qid, lid = a._register_queue()
    assert qid == "Q1"
    assert lid == 7
    body = fake.calls[0]["body_raw"]
    # Form-encoded body must carry the event_types JSON literal.
    assert "event_types=" in body
    assert "%5B%22message%22%5D" in body  # url-encoded ["message"]


def test_register_queue_with_streams_includes_narrow(monkeypatch):
    a = _adapter(ZULIP_STREAMS="eng,general")
    fake = _FakeUrlopen([
        (200, {"queue_id": "Q2", "last_event_id": 11}),
    ])
    monkeypatch.setattr(zu.urllib.request, "urlopen", fake)
    a._register_queue()
    body = fake.calls[0]["body_raw"]
    assert "narrow=" in body
    # urlencode escapes space as `+`; unquote_plus reverses both `%XX`
    # AND `+`. The form-encoded body should round-trip back to the
    # JSON list of `["stream", "<name>"]` tuples.
    decoded = urllib.parse.unquote_plus(body)
    assert '["stream", "eng"]' in decoded
    assert '["stream", "general"]' in decoded


def test_register_queue_4xx_raises(monkeypatch):
    a = _adapter()
    fake = _FakeUrlopen([(400, {"result": "error", "msg": "Bad"})])
    monkeypatch.setattr(zu.urllib.request, "urlopen", fake)
    with pytest.raises(RuntimeError, match="400"):
        a._register_queue()


def test_register_queue_429_retries(monkeypatch):
    """P2 on /register."""
    a = _adapter()
    fake = _FakeUrlopen([
        (429, {"result": "error"}, {"Retry-After": "3"}),
        (200, {"queue_id": "Q3", "last_event_id": 0}),
    ])
    monkeypatch.setattr(zu.urllib.request, "urlopen", fake)
    sleeps: list = []
    monkeypatch.setattr(zu.time, "sleep", lambda s: sleeps.append(s))
    qid, _ = a._register_queue()
    assert qid == "Q3"
    assert sleeps == [3.0]


def test_register_queue_429_without_header_uses_default(monkeypatch):
    a = _adapter()
    fake = _FakeUrlopen([
        (429, {"result": "error"}),
        (200, {"queue_id": "Q4", "last_event_id": 0}),
    ])
    monkeypatch.setattr(zu.urllib.request, "urlopen", fake)
    sleeps: list = []
    monkeypatch.setattr(zu.time, "sleep", lambda s: sleeps.append(s))
    a._register_queue()
    assert sleeps == [zu.RETRY_AFTER_DEFAULT_SECS]


# ---- _poll_once ---------------------------------------------------


def test_poll_once_emits_message_event(monkeypatch):
    a = _adapter()
    fake = _FakeUrlopen([
        (200, {"events": [_event(eid=10)]}),
    ])
    monkeypatch.setattr(zu.urllib.request, "urlopen", fake)
    emitted: list = []
    new_last, sig = a._poll_once(emitted.append, "Q1", 9)
    assert sig is None
    assert new_last == 10
    assert len(emitted) == 1
    assert emitted[0]["params"]["content"] == {"Text": "hello zulip"}


def test_poll_once_dedupes_id_repeats(monkeypatch):
    """P3: the same message.id arriving twice (e.g. after a queue
    re-register) must emit only once."""
    a = _adapter()
    fake = _FakeUrlopen([
        (200, {"events": [_event(eid=10, message=_msg(id=1001))]}),
        (200, {"events": [_event(eid=11, message=_msg(id=1001))]}),
    ])
    monkeypatch.setattr(zu.urllib.request, "urlopen", fake)
    emitted: list = []
    a._poll_once(emitted.append, "Q1", 9)
    a._poll_once(emitted.append, "Q1", 10)
    assert len(emitted) == 1


def test_poll_once_bad_event_queue_id_signals_reregister(monkeypatch):
    """Zulip's queue-expired sentinel: 400 with body.code =
    BAD_EVENT_QUEUE_ID. The poll must return ``reregister`` rather
    than raise — the producer then re-registers in-line."""
    a = _adapter()
    fake = _FakeUrlopen([
        (400, {"result": "error", "code": "BAD_EVENT_QUEUE_ID"}),
    ])
    monkeypatch.setattr(zu.urllib.request, "urlopen", fake)
    new_last, sig = a._poll_once(lambda _: None, "Q-expired", 9)
    assert sig == "reregister"
    assert new_last == 9  # Watermark unchanged.


def test_poll_once_other_400_raises(monkeypatch):
    """A 400 with a different code is a real error → raise so the
    producer backs off (e.g. malformed query)."""
    a = _adapter()
    fake = _FakeUrlopen([
        (400, {"result": "error", "code": "BAD_NARROW"}),
    ])
    monkeypatch.setattr(zu.urllib.request, "urlopen", fake)
    with pytest.raises(RuntimeError, match="400"):
        a._poll_once(lambda _: None, "Q1", 9)


def test_poll_once_429_sleeps_then_raises(monkeypatch):
    """P2: 429 on /events sleeps Retry-After then raises so the
    producer's outer backoff kicks in."""
    a = _adapter()
    fake = _FakeUrlopen([
        (429, {}, {"Retry-After": "4"}),
    ])
    monkeypatch.setattr(zu.urllib.request, "urlopen", fake)
    sleeps: list = []
    monkeypatch.setattr(zu.time, "sleep", lambda s: sleeps.append(s))
    with pytest.raises(RuntimeError, match="429"):
        a._poll_once(lambda _: None, "Q1", 9)
    assert sleeps == [4.0]


def test_poll_once_advances_last_event_id(monkeypatch):
    """Watermark must move to the max event.id seen in the batch."""
    a = _adapter()
    events = [
        _event(eid=20, message=_msg(id=2000)),
        _event(eid=22, message=_msg(id=2001)),  # max
        _event(eid=21, message=_msg(id=2002)),
    ]
    fake = _FakeUrlopen([(200, {"events": events})])
    monkeypatch.setattr(zu.urllib.request, "urlopen", fake)
    emitted: list = []
    new_last, _ = a._poll_once(emitted.append, "Q1", 9)
    assert new_last == 22
    assert len(emitted) == 3


def test_poll_once_long_poll_timeout_passed(monkeypatch):
    """The events fetch must use the long-poll timeout (60+10s),
    not the default 15s send timeout — Zulip holds the connection
    server-side."""
    a = _adapter()
    fake = _FakeUrlopen([(200, {"events": []})])
    monkeypatch.setattr(zu.urllib.request, "urlopen", fake)
    a._poll_once(lambda _: None, "Q1", 9)
    assert fake.calls[0]["timeout"] == zu.LONG_POLL_HTTP_TIMEOUT_SECS


def test_poll_once_skips_non_message_events(monkeypatch):
    """Mixed event types in the batch — non-message events still
    advance the watermark but don't emit."""
    a = _adapter()
    fake = _FakeUrlopen([(200, {"events": [
        {"id": 30, "type": "presence"},
        _event(eid=31, message=_msg(id=3001)),
    ]})])
    monkeypatch.setattr(zu.urllib.request, "urlopen", fake)
    emitted: list = []
    new_last, _ = a._poll_once(emitted.append, "Q1", 9)
    assert new_last == 31
    assert len(emitted) == 1


# ---- _post_message ------------------------------------------------


def test_post_stream_message_shape(monkeypatch):
    a = _adapter()
    fake = _FakeUrlopen([(200, {"result": "success", "id": 50})])
    monkeypatch.setattr(zu.urllib.request, "urlopen", fake)
    a._post_message(
        msg_type="stream", to="engineering",
        topic="deploy-checklist", text="ack",
    )
    body = fake.calls[0]["body_raw"]
    assert "type=stream" in body
    assert "to=engineering" in body
    assert "topic=deploy-checklist" in body
    assert "content=ack" in body


def test_post_direct_message_shape(monkeypatch):
    a = _adapter()
    fake = _FakeUrlopen([(200, {"result": "success", "id": 51})])
    monkeypatch.setattr(zu.urllib.request, "urlopen", fake)
    a._post_message(
        msg_type="direct", to="alice@example.com",
        topic="", text="hi",
    )
    body = fake.calls[0]["body_raw"]
    assert "type=direct" in body
    assert "to=alice%40example.com" in body  # url-encoded @
    # No topic on direct sends.
    assert "topic=" not in body
    assert "content=hi" in body


def test_post_message_chunks_long_body(monkeypatch):
    a = _adapter()
    fake = _FakeUrlopen([
        (200, {"result": "success"}),
        (200, {"result": "success"}),
    ])
    monkeypatch.setattr(zu.urllib.request, "urlopen", fake)
    long_text = "a" * (zu.ZULIP_MSG_LIMIT + 100)
    a._post_message(
        msg_type="stream", to="general", topic="t", text=long_text,
    )
    assert len(fake.calls) == 2


def test_post_message_429_retries_once_with_retry_after(monkeypatch):
    """P2: 429 on /messages honours Retry-After and retries once."""
    a = _adapter()
    fake = _FakeUrlopen([
        (429, {"result": "error"}, {"Retry-After": "2"}),
        (200, {"result": "success"}),
    ])
    monkeypatch.setattr(zu.urllib.request, "urlopen", fake)
    sleeps: list = []
    monkeypatch.setattr(zu.time, "sleep", lambda s: sleeps.append(s))
    a._post_message(
        msg_type="stream", to="general", topic="t", text="hi",
    )
    assert sleeps == [2.0]
    assert len(fake.calls) == 2


def test_post_message_double_429_raises(monkeypatch):
    """A second 429 falls through to the >=300 surface so the caller
    knows the send failed — no infinite retry loop."""
    a = _adapter()
    fake = _FakeUrlopen([
        (429, {"result": "error"}, {"Retry-After": "1"}),
        (429, {"result": "error"}, {"Retry-After": "1"}),
    ])
    monkeypatch.setattr(zu.urllib.request, "urlopen", fake)
    monkeypatch.setattr(zu.time, "sleep", lambda _: None)
    with pytest.raises(RuntimeError, match="429"):
        a._post_message(
            msg_type="stream", to="general", topic="t", text="hi",
        )


def test_post_message_5xx_raises(monkeypatch):
    a = _adapter()
    fake = _FakeUrlopen([(500, {"result": "error"})])
    monkeypatch.setattr(zu.urllib.request, "urlopen", fake)
    with pytest.raises(RuntimeError, match="500"):
        a._post_message(
            msg_type="stream", to="general", topic="t", text="hi",
        )


def test_post_message_missing_dest_raises():
    a = _adapter()
    with pytest.raises(RuntimeError, match="missing destination"):
        a._post_message(
            msg_type="stream", to="", topic="t", text="hi",
        )


# ---- on_send ------------------------------------------------------


class _Cmd:
    """Minimal cmd shape for on_send. Mirrors what the daemon passes."""

    def __init__(
        self, *, text=None, content=None, channel_id="", user=None,
        thread_id=None,
    ):
        self.text = text
        self.content = content
        self.channel_id = channel_id
        self.user = user
        self.thread_id = thread_id


def _run_send(adapter, cmd):
    import asyncio as _asyncio
    _asyncio.new_event_loop().run_until_complete(adapter.on_send(cmd))


def test_on_send_stream_uses_thread_id_as_topic(monkeypatch):
    """P1: cmd.thread_id round-trips as the outbound stream topic."""
    a = _adapter()
    fake = _FakeUrlopen([(200, {"result": "success"})])
    monkeypatch.setattr(zu.urllib.request, "urlopen", fake)
    _run_send(
        a,
        _Cmd(
            text="ack",
            channel_id="engineering",
            user={"platform_id": "engineering"},
            thread_id="deploy-checklist",
        ),
    )
    body = fake.calls[0]["body_raw"]
    assert "type=stream" in body
    assert "to=engineering" in body
    assert "topic=deploy-checklist" in body


def test_on_send_stream_falls_back_to_default_topic(monkeypatch):
    """No inbound thread_id (e.g. initial outbound from a cron
    trigger) → use DEFAULT_STREAM_TOPIC, matches the Rust adapter
    when it had no context."""
    a = _adapter()
    fake = _FakeUrlopen([(200, {"result": "success"})])
    monkeypatch.setattr(zu.urllib.request, "urlopen", fake)
    _run_send(
        a,
        _Cmd(
            text="kickoff",
            channel_id="general",
            user={"platform_id": "general"},
            thread_id=None,
        ),
    )
    body = fake.calls[0]["body_raw"]
    assert f"topic={zu.DEFAULT_STREAM_TOPIC}" in body


def test_on_send_dm_via_at_in_platform_id(monkeypatch):
    """Outbound DM detection: any '@' in the dest → type=direct."""
    a = _adapter()
    fake = _FakeUrlopen([(200, {"result": "success"})])
    monkeypatch.setattr(zu.urllib.request, "urlopen", fake)
    _run_send(
        a,
        _Cmd(
            text="hi alice",
            channel_id="alice@example.com",
            user={"platform_id": "alice@example.com"},
        ),
    )
    body = fake.calls[0]["body_raw"]
    assert "type=direct" in body
    assert "to=alice%40example.com" in body


def test_on_send_falls_back_to_channel_id(monkeypatch):
    """If cmd.user is missing/empty, channel_id takes over."""
    a = _adapter()
    fake = _FakeUrlopen([(200, {"result": "success"})])
    monkeypatch.setattr(zu.urllib.request, "urlopen", fake)
    _run_send(
        a,
        _Cmd(text="x", channel_id="engineering", user=None),
    )
    body = fake.calls[0]["body_raw"]
    assert "to=engineering" in body


def test_on_send_non_text_content_placeholder(monkeypatch):
    """Image / structured content → placeholder string (matches the
    Rust adapter's `"(Unsupported content type)"` at zulip.rs:453)."""
    a = _adapter()
    fake = _FakeUrlopen([(200, {"result": "success"})])
    monkeypatch.setattr(zu.urllib.request, "urlopen", fake)
    _run_send(
        a,
        _Cmd(
            content={"Image": {"url": "https://x"}},
            channel_id="general",
            user={"platform_id": "general"},
        ),
    )
    body = fake.calls[0]["body_raw"]
    assert "Unsupported+content+type" in body


def test_on_send_text_via_content_dict(monkeypatch):
    """The post-#5219 daemon uses cmd.content = {"Text": ...}; check
    the adapter takes the text from .text (which the daemon mirrors)
    when content is the plain-text shape."""
    a = _adapter()
    fake = _FakeUrlopen([(200, {"result": "success"})])
    monkeypatch.setattr(zu.urllib.request, "urlopen", fake)
    _run_send(
        a,
        _Cmd(
            text="hello",
            content={"Text": "hello"},
            channel_id="general",
            user={"platform_id": "general"},
            thread_id="topic-a",
        ),
    )
    body = fake.calls[0]["body_raw"]
    assert "content=hello" in body
    assert "topic=topic-a" in body


# ---- class attrs / SCHEMA -----------------------------------------


def test_schema_advertises_zulip():
    assert zu.ZulipAdapter.SCHEMA.name == "zulip"
    field_keys = {f.key for f in zu.ZulipAdapter.SCHEMA.fields}
    assert "ZULIP_SERVER_URL" in field_keys
    assert "ZULIP_BOT_EMAIL" in field_keys
    assert "ZULIP_API_KEY" in field_keys


def test_suppress_error_responses_is_false():
    """Chat-room precedent (slack / discord / webex) — surface errors
    so the operator gets a visible failure."""
    assert zu.ZulipAdapter.suppress_error_responses is False


def test_capabilities_threads():
    """Zulip topics are the thread analogue — declare so the daemon
    routes thread context through on_send."""
    assert "thread" in zu.ZulipAdapter.capabilities
