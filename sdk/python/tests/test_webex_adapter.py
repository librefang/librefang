"""Tests for librefang.sidecar.adapters.webex.

Deterministic, no network: urllib + WebSocket are monkeypatched /
replaced with a fake. Asserts the sidecar preserves the in-process
Rust ``librefang-channels::webex`` adapter's behaviour, plus the
four improvements documented in the module header.
"""

import io
import json
import os
import urllib.error

import pytest


os.environ.setdefault("WEBEX_BOT_TOKEN", "test-bot-token")
from librefang.sidecar.adapters import webex as wa  # noqa: E402


# ---- _FakeUrlopen scaffolding ----------------------------------------


class _HdrShim:
    def __init__(self, hdrs):
        self._hdrs = hdrs or {}

    def items(self):
        return list(self._hdrs.items())


class _FakeResp:
    def __init__(self, status, body=b"", headers=None):
        self.status = status
        self._body = body
        self.headers = headers if headers is not None else _HdrShim({})

    def read(self):
        return self._body

    def __enter__(self):
        return self

    def __exit__(self, *_):
        return False


class _FakeUrlopen:
    def __init__(self, script):
        self.script = list(script)
        self.calls = []

    def __call__(self, req, timeout=None):
        body_bytes = req.data
        try:
            decoded = body_bytes.decode("utf-8") if body_bytes else None
        except Exception:  # noqa: BLE001
            decoded = None
        self.calls.append({
            "url": req.full_url,
            "method": req.get_method(),
            "headers": {k.lower(): v for k, v in req.header_items()},
            "body_raw": decoded,
            "timeout": timeout,
        })
        if not self.script:
            raise AssertionError(
                f"unexpected extra urlopen call to {req.full_url}"
            )
        entry = self.script.pop(0)
        if len(entry) == 3:
            status, body, resp_hdrs = entry
        else:
            status, body = entry
            resp_hdrs = {}
        if status >= 400:
            raise urllib.error.HTTPError(
                req.full_url, status, "Error", _HdrShim(resp_hdrs),
                io.BytesIO(json.dumps(body or {}).encode("utf-8")),
            )
        if body is None:
            payload = b""
        elif isinstance(body, (dict, list)):
            payload = json.dumps(body).encode("utf-8")
        else:
            payload = body if isinstance(body, bytes) else str(body).encode("utf-8")
        return _FakeResp(status, payload, _HdrShim(resp_hdrs))


def _adapter(**env):
    defaults = {
        "WEBEX_BOT_TOKEN": "test-bot-token",
        "WEBEX_ALLOWED_ROOMS": "",
        "WEBEX_ACCOUNT_ID": "",
        "WEBEX_API_BASE": "",
        "WEBEX_WS_URL": "",
    }
    for k, v in defaults.items():
        os.environ[k] = env.get(k, v)
    return wa.WebexAdapter()


# ---- env handling ----------------------------------------------------


def test_default_api_base_and_token():
    a = _adapter()
    assert a.api_base == "https://webexapis.com/v1"
    assert a.bot_token == "test-bot-token"
    assert a.ws_url == wa.DEFAULT_WS_URL
    assert a.allowed_rooms == []
    assert a.account_id is None
    assert a.bot_user_id is None


def test_missing_bot_token_exits_2():
    os.environ["WEBEX_BOT_TOKEN"] = ""
    with pytest.raises(SystemExit) as exc:
        wa.WebexAdapter()
    assert exc.value.code == 2
    os.environ["WEBEX_BOT_TOKEN"] = "test-bot-token"


def test_whitespace_only_token_exits_2():
    os.environ["WEBEX_BOT_TOKEN"] = "   "
    with pytest.raises(SystemExit) as exc:
        wa.WebexAdapter()
    assert exc.value.code == 2
    os.environ["WEBEX_BOT_TOKEN"] = "test-bot-token"


def test_allowed_rooms_split_with_whitespace():
    a = _adapter(WEBEX_ALLOWED_ROOMS="Y2lz1, Y2lz2 ,Y2lz3")
    assert a.allowed_rooms == ["Y2lz1", "Y2lz2", "Y2lz3"]


def test_allowed_rooms_empty_means_all():
    a = _adapter(WEBEX_ALLOWED_ROOMS="")
    assert a.allowed_rooms == []


def test_account_id_passthrough():
    a = _adapter(WEBEX_ACCOUNT_ID="org-prod")
    assert a.account_id == "org-prod"


def test_account_id_empty_is_none():
    a = _adapter(WEBEX_ACCOUNT_ID="")
    assert a.account_id is None


def test_api_base_env_override():
    a = _adapter(WEBEX_API_BASE="https://mock.example/v1")
    assert a.api_base == "https://mock.example/v1"


def test_ws_url_env_override():
    a = _adapter(WEBEX_WS_URL="wss://mock.example/ws")
    assert a.ws_url == "wss://mock.example/ws"


# ---- _split_message --------------------------------------------------


def test_split_message_under_limit():
    assert wa._split_message("hello", 100) == ["hello"]


def test_split_message_newline_cut():
    text = "a" * 80 + "\n" + "b" * 80
    out = wa._split_message(text, 100)
    assert out[0] == "a" * 80
    assert out[1] == "b" * 80


def test_split_message_hard_cut_when_no_newline():
    text = "a" * 250
    out = wa._split_message(text, 100)
    assert out == ["a" * 100, "a" * 100, "a" * 50]


def test_split_message_7439_cap_matches_rust():
    """Mirror MAX_MESSAGE_LEN in crates/librefang-channels/src/webex.rs:29."""
    assert wa.WEBEX_MSG_LIMIT == 7439
    text = "x" * (wa.WEBEX_MSG_LIMIT + 100)
    out = wa._split_message(text, wa.WEBEX_MSG_LIMIT)
    assert len(out) == 2
    assert len(out[0]) == wa.WEBEX_MSG_LIMIT
    assert len(out[1]) == 100


# ---- _split_csv ------------------------------------------------------


def test_split_csv_empty_and_whitespace():
    assert wa._split_csv("") == []
    assert wa._split_csv(", ,") == []
    assert wa._split_csv(" a , b") == ["a", "b"]


# ---- _parse_retry_after ----------------------------------------------


def test_retry_after_missing_uses_default():
    assert wa._parse_retry_after({}, default_secs=5.0) == 5.0


def test_retry_after_integer_seconds():
    assert wa._parse_retry_after({"retry-after": "12"}, default_secs=5.0) == 12.0


def test_retry_after_decimal_seconds():
    assert wa._parse_retry_after(
        {"retry-after": "1.5"}, default_secs=99.0,
    ) == 1.5


def test_retry_after_garbage_falls_back():
    assert wa._parse_retry_after(
        {"retry-after": "later"}, default_secs=7.0,
    ) == 7.0


def test_retry_after_floored_at_1s():
    """Retry-After: 0 should still pause at least 1 s so we don't
    re-pound the throttle."""
    assert wa._parse_retry_after({"retry-after": "0"}, default_secs=5.0) == 1.0


def test_retry_after_capped_at_max_backoff():
    assert wa._parse_retry_after(
        {"retry-after": "9999"}, default_secs=5.0,
    ) == wa.MAX_BACKOFF_SECS


# ---- parse_webex_message --------------------------------------------


def _activity(**overrides):
    base = {
        "verb": "post",
        "actor": {"id": "USER_A"},
        "object": {"id": "MSG_1"},
        "target": {"id": "ROOM_1"},
    }
    for k, v in overrides.items():
        if k in ("actor", "object", "target") and isinstance(v, dict):
            base[k] = v
        else:
            base[k] = v
    return base


def _full_msg(**overrides):
    base = {
        "id": "MSG_1",
        "roomId": "ROOM_1",
        "roomType": "group",
        "text": "hello webex",
        "personEmail": "alice@example.com",
        "personId": "PERSON_A",
    }
    base.update(overrides)
    return base


def test_parse_basic_text():
    ev = wa.parse_webex_message(
        _full_msg(),
        _activity(),
        own_bot_id="BOT_ID",
        allowed_rooms=[],
        account_id=None,
    )
    assert ev is not None
    p = ev["params"]
    assert p["user_id"] == "ROOM_1"
    assert p["user_name"] == "alice@example.com"
    assert p["content"] == {"Text": "hello webex"}
    assert p["message_id"] == "MSG_1"
    assert p["is_group"] is True
    # Improvement #1: top-level message → thread_id = own id
    assert p["thread_id"] == "MSG_1"
    md = p["metadata"]
    assert md["sender_id"] == "PERSON_A"
    assert md["sender_email"] == "alice@example.com"


def test_parse_prefers_person_display_name_for_user_name():
    """When the /messages/<id> body carries `personDisplayName`, it
    drives `user_name` instead of `personEmail`. The Rust adapter at
    webex.rs:431 used personEmail unconditionally, leaking emails into
    bot logs / UI surfaces. personEmail stays in metadata for routing
    / audit; only the user-facing label changes."""
    ev = wa.parse_webex_message(
        _full_msg(personDisplayName="Alice"),
        _activity(),
        own_bot_id="BOT_ID", allowed_rooms=[], account_id=None,
    )
    assert ev is not None
    p = ev["params"]
    assert p["user_name"] == "Alice"
    # personEmail still recorded for downstream routing.
    assert p["metadata"]["sender_email"] == "alice@example.com"


def test_parse_falls_back_to_email_when_display_name_missing():
    """personDisplayName absent (older Webex orgs / service accounts)
    falls back to personEmail — matches the Rust adapter's user-facing
    label so existing operators don't see "unknown" / blank labels."""
    ev = wa.parse_webex_message(
        # No personDisplayName key in the body.
        _full_msg(),
        _activity(),
        own_bot_id="BOT_ID", allowed_rooms=[], account_id=None,
    )
    assert ev is not None
    assert ev["params"]["user_name"] == "alice@example.com"


def test_parse_falls_back_to_email_when_display_name_empty():
    """personDisplayName present but empty string treated the same as
    absent."""
    ev = wa.parse_webex_message(
        _full_msg(personDisplayName=""),
        _activity(),
        own_bot_id="BOT_ID", allowed_rooms=[], account_id=None,
    )
    assert ev is not None
    assert ev["params"]["user_name"] == "alice@example.com"


def test_parse_filters_non_post_verb():
    ev = wa.parse_webex_message(
        _full_msg(),
        _activity(verb="acknowledge"),
        own_bot_id=None, allowed_rooms=[], account_id=None,
    )
    assert ev is None


def test_parse_skips_self_actor():
    ev = wa.parse_webex_message(
        _full_msg(),
        _activity(actor={"id": "BOT_ID"}),
        own_bot_id="BOT_ID", allowed_rooms=[], account_id=None,
    )
    assert ev is None


def test_parse_self_skip_disabled_when_own_bot_id_empty():
    """When own_bot_id is None (not yet authenticated), nothing
    should be skipped on actor-id alone."""
    ev = wa.parse_webex_message(
        _full_msg(),
        _activity(actor={"id": "BOT_ID"}),
        own_bot_id=None, allowed_rooms=[], account_id=None,
    )
    assert ev is not None


def test_parse_skips_missing_object_id():
    ev = wa.parse_webex_message(
        _full_msg(),
        _activity(object={}),
        own_bot_id=None, allowed_rooms=[], account_id=None,
    )
    assert ev is None


def test_parse_skips_empty_text():
    ev = wa.parse_webex_message(
        _full_msg(text=""),
        _activity(),
        own_bot_id=None, allowed_rooms=[], account_id=None,
    )
    assert ev is None


def test_parse_room_filter_rejects_unlisted():
    ev = wa.parse_webex_message(
        _full_msg(),
        _activity(target={"id": "ROOM_OTHER"}),
        own_bot_id=None,
        allowed_rooms=["ROOM_ALLOWED"],
        account_id=None,
    )
    assert ev is None


def test_parse_room_filter_accepts_listed():
    ev = wa.parse_webex_message(
        _full_msg(roomId="ROOM_ALLOWED"),
        _activity(target={"id": "ROOM_ALLOWED"}),
        own_bot_id=None,
        allowed_rooms=["ROOM_ALLOWED"],
        account_id=None,
    )
    assert ev is not None
    assert ev["params"]["user_id"] == "ROOM_ALLOWED"


def test_parse_room_filter_empty_means_all():
    ev = wa.parse_webex_message(
        _full_msg(),
        _activity(),
        own_bot_id=None,
        allowed_rooms=[],
        account_id=None,
    )
    assert ev is not None


def test_parse_command_form():
    ev = wa.parse_webex_message(
        _full_msg(text="/echo hello world"),
        _activity(),
        own_bot_id=None, allowed_rooms=[], account_id=None,
    )
    assert ev["params"]["content"] == {
        "Command": {"name": "echo", "args": ["hello", "world"]},
    }


def test_parse_command_no_args():
    ev = wa.parse_webex_message(
        _full_msg(text="/ping"),
        _activity(),
        own_bot_id=None, allowed_rooms=[], account_id=None,
    )
    assert ev["params"]["content"] == {
        "Command": {"name": "ping", "args": []},
    }


def test_parse_dm_room_type_is_not_group():
    ev = wa.parse_webex_message(
        _full_msg(roomType="direct"),
        _activity(),
        own_bot_id=None, allowed_rooms=[], account_id=None,
    )
    # is_group only emitted as True; when False the helper omits the
    # field entirely (protocol.message convention).
    assert ev["params"].get("is_group", False) is False


def test_parse_account_id_injected_into_metadata():
    ev = wa.parse_webex_message(
        _full_msg(),
        _activity(),
        own_bot_id=None,
        allowed_rooms=[],
        account_id="org-42",
    )
    assert ev["params"]["metadata"]["account_id"] == "org-42"


def test_parse_thread_reply_uses_parent_id_as_thread_id():
    """Improvement #1: when the inbound message itself was already
    inside a thread, the bot should thread alongside (= use the
    inbound parentId), not start a nested child."""
    ev = wa.parse_webex_message(
        _full_msg(parentId="MSG_ROOT"),
        _activity(),
        own_bot_id=None, allowed_rooms=[], account_id=None,
    )
    assert ev["params"]["thread_id"] == "MSG_ROOT"


def test_parse_top_level_uses_own_id_as_thread_id():
    """Improvement #1: a top-level inbound message → thread_id =
    own id, so the bot's reply threads under what triggered it
    (mirrors rocketchat / nextcloud)."""
    ev = wa.parse_webex_message(
        _full_msg(),
        _activity(),
        own_bot_id=None, allowed_rooms=[], account_id=None,
    )
    assert ev["params"]["thread_id"] == "MSG_1"


def test_parse_room_type_defaults_to_group():
    """When roomType is missing, the Rust adapter defaulted to
    'group' (webex.rs:408). Mirror that so unfamiliar payloads
    still produce sane is_group."""
    ev = wa.parse_webex_message(
        _full_msg(roomType=None),
        _activity(),
        own_bot_id=None, allowed_rooms=[], account_id=None,
    )
    assert ev["params"].get("is_group") is True


def test_parse_missing_person_fields_fallback():
    ev = wa.parse_webex_message(
        _full_msg(personEmail=None, personId=None),
        _activity(),
        own_bot_id=None, allowed_rooms=[], account_id=None,
    )
    assert ev is not None
    assert ev["params"]["user_name"] == "unknown"
    assert ev["params"]["metadata"]["sender_id"] == ""


def test_parse_full_room_id_fallback_to_activity_target():
    """When full_msg has no roomId, fall back to activity.target.id
    (matches webex.rs:407)."""
    msg = _full_msg()
    msg["roomId"] = None
    ev = wa.parse_webex_message(
        msg,
        _activity(target={"id": "ROOM_FROM_ACTIVITY"}),
        own_bot_id=None, allowed_rooms=[], account_id=None,
    )
    assert ev["params"]["user_id"] == "ROOM_FROM_ACTIVITY"


def test_parse_malformed_activity():
    assert wa.parse_webex_message(
        _full_msg(),
        None,  # type: ignore[arg-type]
        own_bot_id=None, allowed_rooms=[], account_id=None,
    ) is None
    assert wa.parse_webex_message(
        None,  # type: ignore[arg-type]
        _activity(),
        own_bot_id=None, allowed_rooms=[], account_id=None,
    ) is None


# ---- _mark_seen (dedupe, improvement #3) ----------------------------


def test_mark_seen_first_time_returns_true():
    a = _adapter()
    assert a._mark_seen("MSG_X") is True


def test_mark_seen_repeat_returns_false():
    a = _adapter()
    assert a._mark_seen("MSG_X") is True
    assert a._mark_seen("MSG_X") is False


def test_mark_seen_empty_id_returns_false():
    a = _adapter()
    assert a._mark_seen("") is False


def test_mark_seen_capacity_eviction():
    """When we cross SEEN_MESSAGES_MAX, the oldest EVICT entries
    should be dropped — and an id that was evicted comes back as
    'fresh' on re-mark."""
    a = _adapter()
    for i in range(wa.SEEN_MESSAGES_MAX):
        assert a._mark_seen(f"ID_{i}") is True
    assert len(a._seen_ids) == wa.SEEN_MESSAGES_MAX
    # Trigger eviction
    assert a._mark_seen("ID_TRIGGER") is True
    assert len(a._seen_ids) == wa.SEEN_MESSAGES_MAX - wa.SEEN_MESSAGES_EVICT + 1
    # The earliest id should now have been evicted and be markable again.
    assert a._mark_seen("ID_0") is True


# ---- _validate_bot_token --------------------------------------------


def test_validate_bot_token_happy_path(monkeypatch):
    fake = _FakeUrlopen([
        (200, {"id": "BOT_ID", "displayName": "TestBot"}),
    ])
    monkeypatch.setattr(wa.urllib.request, "urlopen", fake)
    a = _adapter()
    bot_id, name = a._validate_bot_token()
    assert bot_id == "BOT_ID"
    assert name == "TestBot"
    assert fake.calls[0]["url"].endswith("/people/me")
    assert fake.calls[0]["headers"]["authorization"] == "Bearer test-bot-token"
    assert fake.calls[0]["method"] == "GET"
    # Improvement #4: explicit timeout passed
    assert fake.calls[0]["timeout"] == wa.SEND_TIMEOUT_SECS


def test_validate_bot_token_default_display_name(monkeypatch):
    fake = _FakeUrlopen([
        (200, {"id": "BOT_ID"}),
    ])
    monkeypatch.setattr(wa.urllib.request, "urlopen", fake)
    a = _adapter()
    bot_id, name = a._validate_bot_token()
    assert bot_id == "BOT_ID"
    assert name == "LibreFang Bot"


def test_validate_bot_token_raises_on_401(monkeypatch):
    fake = _FakeUrlopen([
        (401, {"message": "Unauthorized"}),
    ])
    monkeypatch.setattr(wa.urllib.request, "urlopen", fake)
    a = _adapter()
    with pytest.raises(RuntimeError, match="people/me failed"):
        a._validate_bot_token()


def test_validate_bot_token_raises_when_id_missing(monkeypatch):
    fake = _FakeUrlopen([
        (200, {"displayName": "OnlyName"}),
    ])
    monkeypatch.setattr(wa.urllib.request, "urlopen", fake)
    a = _adapter()
    with pytest.raises(RuntimeError, match="missing 'id'"):
        a._validate_bot_token()


def test_validate_bot_token_429_retries_after(monkeypatch):
    """Improvement #2 on the auth probe — Webex 429s on startup
    are bruteforce-throttle hits, not auth failures."""
    fake = _FakeUrlopen([
        (429, {"message": "Too Many Requests"}, {"Retry-After": "0"}),
        (200, {"id": "BOT_ID", "displayName": "TestBot"}),
    ])
    sleeps: list[float] = []
    monkeypatch.setattr(wa.urllib.request, "urlopen", fake)
    monkeypatch.setattr(wa.time, "sleep", lambda s: sleeps.append(s))
    a = _adapter()
    bot_id, _ = a._validate_bot_token()
    assert bot_id == "BOT_ID"
    # floor 1.0 because Retry-After: 0
    assert sleeps == [1.0]


# ---- _fetch_message --------------------------------------------------


def test_fetch_message_happy_path(monkeypatch):
    fake = _FakeUrlopen([
        (200, _full_msg(text="hi")),
    ])
    monkeypatch.setattr(wa.urllib.request, "urlopen", fake)
    a = _adapter()
    msg = a._fetch_message("MSG_1")
    assert msg is not None
    assert msg["text"] == "hi"
    assert fake.calls[0]["url"].endswith("/messages/MSG_1")
    assert fake.calls[0]["method"] == "GET"
    assert fake.calls[0]["headers"]["authorization"] == "Bearer test-bot-token"


def test_fetch_message_url_quotes_special_chars(monkeypatch):
    """URL-encode the id so a webhook-style id with '/' or '+' doesn't
    break the path. urllib.parse.quote with safe='' rewrites them."""
    fake = _FakeUrlopen([
        (200, _full_msg()),
    ])
    monkeypatch.setattr(wa.urllib.request, "urlopen", fake)
    a = _adapter()
    a._fetch_message("MSG/foo+bar")
    assert "MSG%2Ffoo%2Bbar" in fake.calls[0]["url"]


def test_fetch_message_non_2xx_returns_none(monkeypatch):
    fake = _FakeUrlopen([
        (404, {"message": "Not Found"}),
    ])
    monkeypatch.setattr(wa.urllib.request, "urlopen", fake)
    a = _adapter()
    assert a._fetch_message("MISSING") is None


def test_fetch_message_429_retries_after(monkeypatch):
    """Improvement #2: 429 on /messages/<id> honours Retry-After."""
    fake = _FakeUrlopen([
        (429, {"message": "rate limited"}, {"Retry-After": "2"}),
        (200, _full_msg(text="after retry")),
    ])
    sleeps: list[float] = []
    monkeypatch.setattr(wa.urllib.request, "urlopen", fake)
    monkeypatch.setattr(wa.time, "sleep", lambda s: sleeps.append(s))
    a = _adapter()
    msg = a._fetch_message("MSG_1")
    assert msg["text"] == "after retry"
    assert sleeps == [2.0]


def test_fetch_message_429_without_header_uses_default(monkeypatch):
    fake = _FakeUrlopen([
        (429, {"message": "rate limited"}),  # no Retry-After
        (200, _full_msg()),
    ])
    sleeps: list[float] = []
    monkeypatch.setattr(wa.urllib.request, "urlopen", fake)
    monkeypatch.setattr(wa.time, "sleep", lambda s: sleeps.append(s))
    a = _adapter()
    a._fetch_message("MSG_1")
    assert sleeps == [wa.RETRY_AFTER_DEFAULT_SECS]


def test_fetch_message_double_429_returns_none(monkeypatch):
    fake = _FakeUrlopen([
        (429, {"message": "rate limited"}, {"Retry-After": "1"}),
        (429, {"message": "still"}, {"Retry-After": "1"}),
    ])
    monkeypatch.setattr(wa.urllib.request, "urlopen", fake)
    monkeypatch.setattr(wa.time, "sleep", lambda _: None)
    a = _adapter()
    assert a._fetch_message("MSG_1") is None


# ---- _post_message ---------------------------------------------------


def test_post_message_basic_shape(monkeypatch):
    fake = _FakeUrlopen([
        (200, {"id": "REPLY_1"}),
    ])
    monkeypatch.setattr(wa.urllib.request, "urlopen", fake)
    a = _adapter()
    a._post_message("ROOM_1", "hello back")
    assert len(fake.calls) == 1
    c = fake.calls[0]
    assert c["url"].endswith("/messages")
    assert c["method"] == "POST"
    body = json.loads(c["body_raw"])
    assert body == {"roomId": "ROOM_1", "text": "hello back"}
    assert c["headers"]["authorization"] == "Bearer test-bot-token"
    assert c["headers"]["content-type"].startswith("application/json")


def test_post_message_with_parent_id(monkeypatch):
    """Improvement #1: parentId is sent when present."""
    fake = _FakeUrlopen([
        (200, {"id": "REPLY_1"}),
    ])
    monkeypatch.setattr(wa.urllib.request, "urlopen", fake)
    a = _adapter()
    a._post_message("ROOM_1", "threaded reply", parent_id="MSG_PARENT")
    body = json.loads(fake.calls[0]["body_raw"])
    assert body == {
        "roomId": "ROOM_1",
        "text": "threaded reply",
        "parentId": "MSG_PARENT",
    }


def test_post_message_chunks_preserve_parent_id(monkeypatch):
    fake = _FakeUrlopen([
        (200, {"id": f"REPLY_{i}"}) for i in range(2)
    ])
    monkeypatch.setattr(wa.urllib.request, "urlopen", fake)
    a = _adapter()
    text = "x" * (wa.WEBEX_MSG_LIMIT + 100)
    a._post_message("ROOM_1", text, parent_id="MSG_PARENT")
    assert len(fake.calls) == 2
    for c in fake.calls:
        body = json.loads(c["body_raw"])
        assert body["parentId"] == "MSG_PARENT"
        assert body["roomId"] == "ROOM_1"


def test_post_message_429_retries_after(monkeypatch):
    """Improvement #2: 429 on POST /messages honours Retry-After."""
    fake = _FakeUrlopen([
        (429, {"message": "rate limited"}, {"Retry-After": "3"}),
        (200, {"id": "REPLY_1"}),
    ])
    sleeps: list[float] = []
    monkeypatch.setattr(wa.urllib.request, "urlopen", fake)
    monkeypatch.setattr(wa.time, "sleep", lambda s: sleeps.append(s))
    a = _adapter()
    a._post_message("ROOM_1", "hi")
    assert sleeps == [3.0]
    assert len(fake.calls) == 2


def test_post_message_429_without_header_uses_default(monkeypatch):
    fake = _FakeUrlopen([
        (429, {"message": "rate limited"}),
        (200, {"id": "OK"}),
    ])
    sleeps: list[float] = []
    monkeypatch.setattr(wa.urllib.request, "urlopen", fake)
    monkeypatch.setattr(wa.time, "sleep", lambda s: sleeps.append(s))
    a = _adapter()
    a._post_message("ROOM_1", "hi")
    assert sleeps == [wa.RETRY_AFTER_DEFAULT_SECS]


def test_post_message_double_429_fails_open(monkeypatch):
    """Two consecutive 429s on the same chunk → log + drop that
    chunk, continue with the next. Mirrors the discord / slack
    fail-open behaviour."""
    fake = _FakeUrlopen([
        (429, {"message": "rate limited"}, {"Retry-After": "1"}),
        (429, {"message": "still"}, {"Retry-After": "1"}),
        (200, {"id": "OK_SECOND_CHUNK"}),
    ])
    monkeypatch.setattr(wa.urllib.request, "urlopen", fake)
    monkeypatch.setattr(wa.time, "sleep", lambda _: None)
    a = _adapter()
    text = "x" * (wa.WEBEX_MSG_LIMIT + 50)  # two chunks
    a._post_message("ROOM_1", text)
    # 2 429s on chunk 1, then chunk 2 succeeds
    assert len(fake.calls) == 3


def test_post_message_5xx_logged_and_continues(monkeypatch):
    fake = _FakeUrlopen([
        (500, {"message": "server error"}),
        (200, {"id": "OK_SECOND_CHUNK"}),
    ])
    monkeypatch.setattr(wa.urllib.request, "urlopen", fake)
    a = _adapter()
    text = "x" * (wa.WEBEX_MSG_LIMIT + 50)  # two chunks
    a._post_message("ROOM_1", text)
    # First chunk 500'd, second chunk still got POSTed.
    assert len(fake.calls) == 2


def test_post_message_explicit_timeout_passed(monkeypatch):
    """Improvement #4: every urlopen has timeout=SEND_TIMEOUT_SECS."""
    fake = _FakeUrlopen([
        (200, {"id": "REPLY_1"}),
    ])
    monkeypatch.setattr(wa.urllib.request, "urlopen", fake)
    a = _adapter()
    a._post_message("ROOM_1", "hi")
    assert fake.calls[0]["timeout"] == wa.SEND_TIMEOUT_SECS


# ---- _handle_envelope (end-to-end with REST fetch mocked) -----------


def _emitted():
    """Return ``(emit_fn, sink_list)``. ``emit_fn`` appends; tests
    assert on ``sink_list``."""
    sink: list[dict] = []
    return (lambda ev: sink.append(ev)), sink


def test_handle_envelope_full_flow(monkeypatch):
    fake = _FakeUrlopen([
        (200, _full_msg(text="hello")),
    ])
    monkeypatch.setattr(wa.urllib.request, "urlopen", fake)
    a = _adapter()
    a.bot_user_id = "BOT_ID"
    emit, sink = _emitted()
    a._handle_envelope({"data": {"activity": _activity()}}, emit)
    assert len(sink) == 1
    assert sink[0]["params"]["content"] == {"Text": "hello"}
    assert sink[0]["params"]["thread_id"] == "MSG_1"


def test_handle_envelope_self_skip(monkeypatch):
    """When the activity actor matches the bot, skip without making
    the REST follow-up call."""
    fake = _FakeUrlopen([])
    monkeypatch.setattr(wa.urllib.request, "urlopen", fake)
    a = _adapter()
    a.bot_user_id = "BOT_ID"
    emit, sink = _emitted()
    a._handle_envelope(
        {"data": {"activity": _activity(actor={"id": "BOT_ID"})}}, emit,
    )
    assert sink == []
    assert fake.calls == []


def test_handle_envelope_non_post_skip(monkeypatch):
    fake = _FakeUrlopen([])
    monkeypatch.setattr(wa.urllib.request, "urlopen", fake)
    a = _adapter()
    a.bot_user_id = "BOT_ID"
    emit, sink = _emitted()
    a._handle_envelope(
        {"data": {"activity": _activity(verb="acknowledge")}}, emit,
    )
    assert sink == []
    assert fake.calls == []


def test_handle_envelope_room_filter_skip(monkeypatch):
    fake = _FakeUrlopen([])
    monkeypatch.setattr(wa.urllib.request, "urlopen", fake)
    a = _adapter(WEBEX_ALLOWED_ROOMS="ROOM_OTHER")
    a.bot_user_id = "BOT_ID"
    emit, sink = _emitted()
    a._handle_envelope(
        {"data": {"activity": _activity(target={"id": "ROOM_1"})}}, emit,
    )
    assert sink == []
    assert fake.calls == []


def test_handle_envelope_dedupes_repeated_id(monkeypatch):
    """Improvement #3: repeated activity.object.id only emits once
    even when the envelope is re-delivered (reconnect / replay)."""
    fake = _FakeUrlopen([
        (200, _full_msg()),  # only one REST fetch — second envelope is deduped
    ])
    monkeypatch.setattr(wa.urllib.request, "urlopen", fake)
    a = _adapter()
    a.bot_user_id = "BOT_ID"
    emit, sink = _emitted()
    env = {"data": {"activity": _activity()}}
    a._handle_envelope(env, emit)
    a._handle_envelope(env, emit)
    assert len(sink) == 1


def test_handle_envelope_account_id_injected(monkeypatch):
    fake = _FakeUrlopen([
        (200, _full_msg()),
    ])
    monkeypatch.setattr(wa.urllib.request, "urlopen", fake)
    a = _adapter(WEBEX_ACCOUNT_ID="org-prod")
    a.bot_user_id = "BOT_ID"
    emit, sink = _emitted()
    a._handle_envelope({"data": {"activity": _activity()}}, emit)
    assert sink[0]["params"]["metadata"]["account_id"] == "org-prod"


def test_handle_envelope_fetch_failure_drops(monkeypatch):
    """When the REST follow-up fails, we don't emit and we don't
    crash. (Note: the id is still marked seen — operators should
    not see a flood of retries from the same redelivered envelope.)"""
    fake = _FakeUrlopen([
        (500, {"message": "server error"}),
    ])
    monkeypatch.setattr(wa.urllib.request, "urlopen", fake)
    a = _adapter()
    a.bot_user_id = "BOT_ID"
    emit, sink = _emitted()
    a._handle_envelope({"data": {"activity": _activity()}}, emit)
    assert sink == []


def test_handle_envelope_malformed_payloads(monkeypatch):
    fake = _FakeUrlopen([])
    monkeypatch.setattr(wa.urllib.request, "urlopen", fake)
    a = _adapter()
    a.bot_user_id = "BOT_ID"
    emit, sink = _emitted()
    a._handle_envelope({}, emit)
    a._handle_envelope({"data": None}, emit)
    a._handle_envelope({"data": {}}, emit)
    a._handle_envelope({"data": {"activity": "not-a-dict"}}, emit)
    assert sink == []
    assert fake.calls == []


# ---- on_send wiring --------------------------------------------------


class _SendCmd:
    """Minimal stand-in for protocol.Send (we don't want to import
    the dataclass and force every test to fill every field)."""

    def __init__(
        self,
        *,
        channel_id: str = "",
        text: str = "",
        content=None,
        thread_id=None,
        user=None,
    ):
        self.channel_id = channel_id
        self.text = text
        self.content = content
        self.thread_id = thread_id
        self.user = user or {}


@pytest.mark.asyncio
async def test_on_send_uses_channel_id(monkeypatch):
    fake = _FakeUrlopen([
        (200, {"id": "REPLY_1"}),
    ])
    monkeypatch.setattr(wa.urllib.request, "urlopen", fake)
    a = _adapter()
    await a.on_send(_SendCmd(channel_id="ROOM_X", text="hi", content={"Text": "hi"}))
    body = json.loads(fake.calls[0]["body_raw"])
    assert body["roomId"] == "ROOM_X"
    assert body["text"] == "hi"
    assert "parentId" not in body


@pytest.mark.asyncio
async def test_on_send_falls_back_to_user_platform_id(monkeypatch):
    fake = _FakeUrlopen([
        (200, {"id": "REPLY_1"}),
    ])
    monkeypatch.setattr(wa.urllib.request, "urlopen", fake)
    a = _adapter()
    await a.on_send(_SendCmd(
        channel_id="",
        text="hi",
        content={"Text": "hi"},
        user={"platform_id": "ROOM_FROM_USER"},
    ))
    body = json.loads(fake.calls[0]["body_raw"])
    assert body["roomId"] == "ROOM_FROM_USER"


@pytest.mark.asyncio
async def test_on_send_round_trips_thread_id_as_parent_id(monkeypatch):
    """Improvement #1 round-trip: inbound thread_id → outbound
    parentId."""
    fake = _FakeUrlopen([
        (200, {"id": "REPLY_1"}),
    ])
    monkeypatch.setattr(wa.urllib.request, "urlopen", fake)
    a = _adapter()
    await a.on_send(_SendCmd(
        channel_id="ROOM_X",
        text="threaded reply",
        content={"Text": "threaded reply"},
        thread_id="MSG_PARENT",
    ))
    body = json.loads(fake.calls[0]["body_raw"])
    assert body["parentId"] == "MSG_PARENT"


@pytest.mark.asyncio
async def test_on_send_non_text_content_placeholder(monkeypatch):
    """Non-text content (Image / Voice / etc.) currently has no
    Webex outbound mapping — drop a placeholder so the bot still
    replies. Matches the Rust `send()` else-branch at webex.rs:488."""
    fake = _FakeUrlopen([
        (200, {"id": "REPLY_1"}),
    ])
    monkeypatch.setattr(wa.urllib.request, "urlopen", fake)
    a = _adapter()
    await a.on_send(_SendCmd(
        channel_id="ROOM_X",
        text="",
        content={"Image": {"url": "http://example/img.png"}},
    ))
    body = json.loads(fake.calls[0]["body_raw"])
    assert body["text"] == "(Unsupported content type)"


@pytest.mark.asyncio
async def test_on_send_empty_room_id_drops(monkeypatch):
    """No channel_id and no user.platform_id → silent drop with a
    warn log (no urllib call)."""
    fake = _FakeUrlopen([])
    monkeypatch.setattr(wa.urllib.request, "urlopen", fake)
    a = _adapter()
    await a.on_send(_SendCmd(channel_id="", text="hi", content={"Text": "hi"}))
    assert fake.calls == []


# ---- schema / SCHEMA describe ----------------------------------------


def test_schema_required_fields_listed():
    schema = wa.WebexAdapter.SCHEMA
    assert schema.name == "webex"
    field_keys = {f.key for f in schema.fields}
    assert "WEBEX_BOT_TOKEN" in field_keys
    assert "WEBEX_ALLOWED_ROOMS" in field_keys
    assert "WEBEX_ACCOUNT_ID" in field_keys
    # Only the bot token is required.
    required_keys = {f.key for f in schema.fields if f.required}
    assert required_keys == {"WEBEX_BOT_TOKEN"}


def test_schema_advanced_flags():
    schema = wa.WebexAdapter.SCHEMA
    by_key = {f.key: f for f in schema.fields}
    assert by_key["WEBEX_ALLOWED_ROOMS"].advanced is True
    assert by_key["WEBEX_ACCOUNT_ID"].advanced is True
    assert by_key["WEBEX_BOT_TOKEN"].advanced is False


def test_capabilities_includes_thread():
    """Improvement #1 — the sidecar advertises threading."""
    assert "thread" in wa.WebexAdapter.capabilities
