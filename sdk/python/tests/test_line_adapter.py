"""Tests for librefang.sidecar.adapters.line.

Deterministic, no network: urllib is monkeypatched and the HTTP
webhook handler is exercised through ``_handle_webhook_body`` so
tests never bind a real socket. Asserts the sidecar preserves the
in-process Rust ``librefang-channels::line`` adapter's behaviour
plus the three improvements documented in the module header
(429 Retry-After, inbound dedupe, explicit HTTP timeouts).
"""

import base64
import hashlib
import hmac
import io
import json
import os
import threading
import urllib.error

import pytest


os.environ.setdefault("LINE_CHANNEL_SECRET", "test-secret")
os.environ.setdefault("LINE_CHANNEL_ACCESS_TOKEN", "test-access-token")
from librefang.sidecar.adapters import line as la  # noqa: E402

from _sidecar_fakes import _FakeResp, _FakeUrlopen, _HdrShim


# ---- _FakeUrlopen scaffolding ----------------------------------------


def _adapter(**env):
    defaults = {
        "LINE_CHANNEL_SECRET": "test-secret",
        "LINE_CHANNEL_ACCESS_TOKEN": "test-access-token",
        "LINE_WEBHOOK_PORT": "",
        "LINE_WEBHOOK_PATH": "",
        "LINE_BIND_HOST": "",
        "LINE_ACCOUNT_ID": "",
        "LINE_API_BASE": "",
    }
    for k, v in defaults.items():
        os.environ[k] = env.get(k, v)
    return la.LineAdapter()


# ---- env handling ----------------------------------------------------


def test_default_env_construction():
    a = _adapter()
    assert a.api_base == "https://api.line.me"
    assert a.channel_secret == "test-secret"
    assert a.access_token == "test-access-token"
    assert a.webhook_port == la.DEFAULT_WEBHOOK_PORT
    assert a.webhook_path == "/webhook"
    assert a.bind_host == la.DEFAULT_BIND_HOST
    assert a.account_id is None


def test_missing_channel_secret_exits_2():
    os.environ["LINE_CHANNEL_SECRET"] = ""
    os.environ["LINE_CHANNEL_ACCESS_TOKEN"] = "tok"
    with pytest.raises(SystemExit) as exc:
        la.LineAdapter()
    assert exc.value.code == 2
    os.environ["LINE_CHANNEL_SECRET"] = "test-secret"


def test_missing_access_token_exits_2():
    os.environ["LINE_CHANNEL_SECRET"] = "sec"
    os.environ["LINE_CHANNEL_ACCESS_TOKEN"] = ""
    with pytest.raises(SystemExit) as exc:
        la.LineAdapter()
    assert exc.value.code == 2
    os.environ["LINE_CHANNEL_ACCESS_TOKEN"] = "test-access-token"


def test_whitespace_only_token_exits_2():
    os.environ["LINE_CHANNEL_ACCESS_TOKEN"] = "   "
    with pytest.raises(SystemExit) as exc:
        la.LineAdapter()
    assert exc.value.code == 2
    os.environ["LINE_CHANNEL_ACCESS_TOKEN"] = "test-access-token"


def test_webhook_port_env_override():
    a = _adapter(LINE_WEBHOOK_PORT="18450")
    assert a.webhook_port == 18450


def test_webhook_port_invalid_falls_back_to_default():
    a = _adapter(LINE_WEBHOOK_PORT="not-a-number")
    assert a.webhook_port == la.DEFAULT_WEBHOOK_PORT


def test_webhook_path_env_override():
    a = _adapter(LINE_WEBHOOK_PATH="/line/cb")
    assert a.webhook_path == "/line/cb"


def test_webhook_path_prepends_slash():
    a = _adapter(LINE_WEBHOOK_PATH="hook")
    assert a.webhook_path == "/hook"


def test_account_id_passthrough():
    a = _adapter(LINE_ACCOUNT_ID="prod-1")
    assert a.account_id == "prod-1"


def test_account_id_empty_is_none():
    a = _adapter(LINE_ACCOUNT_ID="")
    assert a.account_id is None


def test_bind_host_env_override():
    a = _adapter(LINE_BIND_HOST="127.0.0.1")
    assert a.bind_host == "127.0.0.1"


def test_api_base_env_override():
    a = _adapter(LINE_API_BASE="https://mock.example")
    assert a.api_base == "https://mock.example"


# ---- _split_message --------------------------------------------------


def test_split_message_under_limit():
    assert la._split_message("hello", 100) == ["hello"]


def test_split_message_newline_cut():
    text = "a" * 80 + "\n" + "b" * 80
    out = la._split_message(text, 100)
    assert out[0] == "a" * 80
    assert out[1] == "b" * 80


def test_split_message_hard_cut_when_no_newline():
    text = "a" * 250
    out = la._split_message(text, 100)
    assert out == ["a" * 100, "a" * 100, "a" * 50]


def test_split_message_5000_cap_matches_rust():
    """Mirror MAX_MESSAGE_LEN in crates/librefang-channels/src/line.rs:39."""
    assert la.LINE_MSG_LIMIT == 5000
    text = "x" * (la.LINE_MSG_LIMIT + 100)
    out = la._split_message(text, la.LINE_MSG_LIMIT)
    assert len(out) == 2
    assert len(out[0]) == la.LINE_MSG_LIMIT
    assert len(out[1]) == 100


# ---- _parse_retry_after ----------------------------------------------


def test_retry_after_missing_returns_default():
    assert la._parse_retry_after({}, default_secs=30.0) == 30.0


def test_retry_after_garbage_returns_default():
    assert la._parse_retry_after(
        {"retry-after": "garbage"}, default_secs=30.0
    ) == 30.0


def test_retry_after_parses_seconds():
    assert la._parse_retry_after(
        {"retry-after": "12"}, default_secs=30.0
    ) == 12.0


def test_retry_after_floor_one_second():
    # Floor at 1 s — a server sending 0 must not spin us into a
    # busy retry loop.
    assert la._parse_retry_after(
        {"retry-after": "0"}, default_secs=30.0
    ) == 1.0


def test_retry_after_caps_at_max_backoff():
    # Cap at MAX_BACKOFF_SECS so a server bug can't pin the loop for
    # hours.
    assert la._parse_retry_after(
        {"retry-after": "999999"}, default_secs=30.0
    ) == la.MAX_BACKOFF_SECS


# ---- verify_line_signature ------------------------------------------


def _make_signature(secret: bytes, body: bytes) -> str:
    return base64.b64encode(
        hmac.new(secret, body, hashlib.sha256).digest()
    ).decode("ascii")


def test_signature_round_trip():
    secret = b"channel-secret-bytes"
    body = br'{"events":[{"type":"message","message":{"text":"hi"}}]}'
    sig = _make_signature(secret, body)
    assert la.verify_line_signature(secret, body, sig)


def test_signature_wrong_secret_rejects():
    secret = b"channel-secret-bytes"
    body = br'{"x":1}'
    sig = _make_signature(secret, body)
    assert not la.verify_line_signature(b"other-secret", body, sig)


def test_signature_mutated_body_rejects():
    secret = b"channel-secret-bytes"
    body = br'{"x":1}'
    sig = _make_signature(secret, body)
    assert not la.verify_line_signature(secret, br'{"x":2}', sig)


def test_signature_empty_signature_rejects():
    """Regression for #3439: empty / whitespace-only signatures
    must never pass HMAC verification."""
    secret = b"s"
    body = b"{}"
    assert not la.verify_line_signature(secret, body, "")
    assert not la.verify_line_signature(secret, body, "   ")


def test_signature_non_base64_rejects():
    secret = b"s"
    body = b"{}"
    assert not la.verify_line_signature(secret, body, "not-base64!@#$%")


def test_signature_breaks_when_body_round_tripped_through_value():
    """Regression for the wire-bytes-vs-JSON-roundtrip bug
    (line.rs::test_line_signature_breaks_when_body_round_tripped_through_value).

    LINE's HMAC must verify the raw bytes the platform sent, not the
    bytes produced by re-serializing the JSON. The two diverge in
    key ordering and whitespace; round-tripped form must not match
    the original digest."""
    secret = b"channel-secret-bytes"
    # Wire body has b before a, plus extra whitespace.
    wire_body = br'{"b":1,  "a":2}'
    sig = _make_signature(secret, wire_body)
    assert la.verify_line_signature(secret, wire_body, sig)

    # Round-trip through Python json reorders keys and removes
    # whitespace, so the digest no longer matches.
    value = json.loads(wire_body)
    round_tripped = json.dumps(value).encode("utf-8")
    assert wire_body != round_tripped
    assert not la.verify_line_signature(secret, round_tripped, sig)


# ---- parse_line_event ------------------------------------------------


def _user_event(text="hello", user_id="U123", msg_id="m-1",
                reply_token="rt-1"):
    return {
        "type": "message",
        "replyToken": reply_token,
        "source": {"type": "user", "userId": user_id},
        "message": {
            "id": msg_id,
            "type": "text",
            "text": text,
        },
    }


def test_parse_text_user_message():
    ev = la.parse_line_event(_user_event(text="Hello from LINE!"))
    assert ev is not None
    assert ev["method"] == "message"
    p = ev["params"]
    assert p["user_id"] == "U123"
    assert p["message_id"] == "m-1"
    assert p.get("is_group") is not True
    assert p["content"] == {"Text": "Hello from LINE!"}
    md = p["metadata"]
    assert md["user_id"] == "U123"
    assert md["reply_to"] == "U123"
    assert md["reply_token"] == "rt-1"
    assert md["source_type"] == "user"


def test_parse_group_message_maps_groupid_to_reply_to():
    ev = la.parse_line_event({
        "type": "message",
        "replyToken": "rt",
        "source": {
            "type": "group",
            "groupId": "C-group",
            "userId": "U1",
        },
        "message": {"id": "m-2", "type": "text", "text": "yo"},
    })
    p = ev["params"]
    assert p["is_group"] is True
    assert p["user_id"] == "C-group"
    md = p["metadata"]
    assert md["reply_to"] == "C-group"
    assert md["source_type"] == "group"


def test_parse_room_message_maps_roomid_to_reply_to():
    ev = la.parse_line_event({
        "type": "message",
        "replyToken": "rt",
        "source": {
            "type": "room",
            "roomId": "R-room",
            "userId": "U1",
        },
        "message": {"id": "m-3", "type": "text", "text": "yo"},
    })
    p = ev["params"]
    assert p["is_group"] is True
    assert p["user_id"] == "R-room"
    assert p["metadata"]["reply_to"] == "R-room"
    assert p["metadata"]["source_type"] == "room"


def test_parse_slash_command_routes_as_command():
    ev = la.parse_line_event(_user_event(text="/status all systems"))
    p = ev["params"]
    assert p["content"] == {"Command": {"name": "status",
                                         "args": ["all", "systems"]}}


def test_parse_slash_command_no_args_emits_empty_list():
    ev = la.parse_line_event(_user_event(text="/ping"))
    p = ev["params"]
    assert p["content"] == {"Command": {"name": "ping", "args": []}}


def test_parse_non_message_event_returns_none():
    """Mirrors the Rust test ``test_parse_line_event_non_message``."""
    assert la.parse_line_event({
        "type": "follow",
        "replyToken": "rt",
        "source": {"type": "user", "userId": "U1"},
    }) is None


def test_parse_non_text_message_returns_none():
    """Mirrors the Rust test ``test_parse_line_event_non_text``."""
    assert la.parse_line_event({
        "type": "message",
        "replyToken": "rt",
        "source": {"type": "user", "userId": "U1"},
        "message": {
            "id": "m-x",
            "type": "sticker",
            "packageId": "1",
            "stickerId": "1",
        },
    }) is None


def test_parse_empty_text_returns_none():
    assert la.parse_line_event(_user_event(text="")) is None


def test_parse_missing_source_returns_none():
    assert la.parse_line_event({
        "type": "message",
        "replyToken": "rt",
        "message": {"id": "m", "type": "text", "text": "hi"},
    }) is None


def test_parse_omits_reply_token_when_absent():
    """A webhook event without a ``replyToken`` (e.g. from
    standby/group context) must not populate the reply_token
    metadata key."""
    ev = la.parse_line_event({
        "type": "message",
        "source": {"type": "user", "userId": "U1"},
        "message": {"id": "m", "type": "text", "text": "hi"},
    })
    assert "reply_token" not in ev["params"]["metadata"]


def test_parse_injects_account_id_metadata_when_present():
    """#5003: multi-bot routing needs the configured ``account_id``
    folded into inbound metadata so the bridge can scope
    ``ApprovalRequested`` delivery."""
    ev = la.parse_line_event(_user_event(), account_id="channel-42")
    assert ev["params"]["metadata"]["account_id"] == "channel-42"


def test_parse_omits_account_id_when_unset():
    ev = la.parse_line_event(_user_event(), account_id=None)
    assert "account_id" not in ev["params"]["metadata"]


# ---- _mark_seen ------------------------------------------------------


def test_mark_seen_first_returns_true_second_returns_false():
    a = _adapter()
    assert a._mark_seen("m-1") is True
    assert a._mark_seen("m-1") is False


def test_mark_seen_empty_id_returns_true_no_state_change():
    """An empty id can't be deduped — emit it (matches the
    handler's defensive code path)."""
    a = _adapter()
    assert a._mark_seen("") is True
    # Empty id must not be retained.
    assert "" not in a._seen.ids


def test_mark_seen_eviction_at_cap(monkeypatch):
    """When ``SEEN_MESSAGES_MAX`` is reached the oldest
    ``SEEN_MESSAGES_EVICT`` entries are dropped. Use small caps via
    monkeypatch so the test is fast."""
    monkeypatch.setattr(la, "SEEN_MESSAGES_MAX", 10)
    monkeypatch.setattr(la, "SEEN_MESSAGES_EVICT", 4)
    a = _adapter()
    for i in range(11):  # 11 > MAX = 10, triggers eviction
        a._mark_seen(f"m-{i}")
    # First 4 should have been evicted.
    assert "m-0" not in a._seen.ids
    assert "m-3" not in a._seen.ids
    # The remainder are still there.
    assert "m-4" in a._seen.ids
    assert "m-10" in a._seen.ids


# ---- _validate_token -------------------------------------------------


def test_validate_token_200(monkeypatch):
    fake = _FakeUrlopen([
        (200, {"displayName": "MyBot"}, {}),
    ])
    monkeypatch.setattr(la.urllib.request, "urlopen", fake)
    a = _adapter()
    assert a._validate_token() == "MyBot"
    assert fake.calls[0]["url"].endswith("/v2/bot/info")
    assert fake.calls[0]["headers"]["authorization"] == "Bearer test-access-token"
    assert fake.calls[0]["timeout"] == la.SEND_TIMEOUT_SECS


def test_validate_token_429_then_200(monkeypatch):
    """Improvement #1: 429 Retry-After honoured."""
    sleeps = []
    monkeypatch.setattr(la.time, "sleep", lambda s: sleeps.append(s))
    fake = _FakeUrlopen([
        (429, {}, {"Retry-After": "2"}),
        (200, {"displayName": "MyBot"}, {}),
    ])
    monkeypatch.setattr(la.urllib.request, "urlopen", fake)
    a = _adapter()
    assert a._validate_token() == "MyBot"
    assert sleeps == [2.0]


def test_validate_token_non_200_raises(monkeypatch):
    fake = _FakeUrlopen([(401, {"message": "invalid token"})])
    monkeypatch.setattr(la.urllib.request, "urlopen", fake)
    a = _adapter()
    with pytest.raises(RuntimeError) as exc:
        a._validate_token()
    assert "status=401" in str(exc.value)


def test_validate_token_missing_display_name_falls_back(monkeypatch):
    fake = _FakeUrlopen([(200, {})])  # body present but no displayName
    monkeypatch.setattr(la.urllib.request, "urlopen", fake)
    a = _adapter()
    assert a._validate_token() == "LINE Bot"


# ---- _push_text + _post_push ----------------------------------------


def test_push_text_single_chunk(monkeypatch):
    fake = _FakeUrlopen([(200, {})])
    monkeypatch.setattr(la.urllib.request, "urlopen", fake)
    a = _adapter()
    a._push_text("U1", "hi")
    assert len(fake.calls) == 1
    c = fake.calls[0]
    assert c["url"] == "https://api.line.me/v2/bot/message/push"
    assert c["method"] == "POST"
    assert c["headers"]["authorization"] == "Bearer test-access-token"
    assert c["headers"]["content-type"].startswith("application/json")
    body = json.loads(c["body_raw"])
    assert body == {
        "to": "U1",
        "messages": [{"type": "text", "text": "hi"}],
    }


def test_push_text_multi_chunk_makes_one_call_per_chunk(monkeypatch):
    fake = _FakeUrlopen([(200, {}), (200, {})])
    monkeypatch.setattr(la.urllib.request, "urlopen", fake)
    a = _adapter()
    big = "a" * (la.LINE_MSG_LIMIT + 50)
    a._push_text("U1", big)
    assert len(fake.calls) == 2
    first = json.loads(fake.calls[0]["body_raw"])
    second = json.loads(fake.calls[1]["body_raw"])
    assert first["messages"][0]["text"] == "a" * la.LINE_MSG_LIMIT
    assert second["messages"][0]["text"] == "a" * 50


def test_push_text_429_then_200_succeeds_after_one_retry(monkeypatch):
    """Improvement #1: a single throttled chunk retries once."""
    sleeps = []
    monkeypatch.setattr(la.time, "sleep", lambda s: sleeps.append(s))
    fake = _FakeUrlopen([
        (429, {}, {"Retry-After": "3"}),
        (200, {}),
    ])
    monkeypatch.setattr(la.urllib.request, "urlopen", fake)
    a = _adapter()
    a._push_text("U1", "hi")
    assert sleeps == [3.0]
    assert len(fake.calls) == 2


def test_push_text_persistent_429_is_fail_open(monkeypatch):
    """Improvement #1, fail-open clause: the second 429 logs and
    continues so a single throttled chunk doesn't drop the rest of
    a multi-chunk reply (matches webex/slack semantics)."""
    monkeypatch.setattr(la.time, "sleep", lambda _s: None)
    fake = _FakeUrlopen([
        (429, {}, {}),
        (429, {}, {}),  # second 429 - fail open
        (200, {}),  # second chunk succeeds
    ])
    monkeypatch.setattr(la.urllib.request, "urlopen", fake)
    a = _adapter()
    big = "a" * (la.LINE_MSG_LIMIT + 10)
    a._push_text("U1", big)
    # Three POSTs: chunk1 (429), chunk1 retry (429, fail-open),
    # chunk2 (200). The reply isn't fully dropped.
    assert len(fake.calls) == 3


def test_push_image_posts_image_then_caption(monkeypatch):
    fake = _FakeUrlopen([(200, {}), (200, {})])
    monkeypatch.setattr(la.urllib.request, "urlopen", fake)
    a = _adapter()
    a._push_image("U1", "https://example.com/p.jpg", caption="see this")
    assert len(fake.calls) == 2
    img_body = json.loads(fake.calls[0]["body_raw"])
    assert img_body == {
        "to": "U1",
        "messages": [{
            "type": "image",
            "originalContentUrl": "https://example.com/p.jpg",
            "previewImageUrl": "https://example.com/p.jpg",
        }],
    }
    cap_body = json.loads(fake.calls[1]["body_raw"])
    assert cap_body["messages"][0]["text"] == "see this"


def test_push_image_no_caption_makes_one_call(monkeypatch):
    fake = _FakeUrlopen([(200, {})])
    monkeypatch.setattr(la.urllib.request, "urlopen", fake)
    a = _adapter()
    a._push_image("U1", "https://example.com/p.jpg", caption=None)
    assert len(fake.calls) == 1


def test_push_image_empty_url_skipped(monkeypatch):
    """An empty image URL would otherwise produce an invalid LINE
    payload (originalContentUrl is required). Skip the image and
    fall through to caption-only when applicable."""
    fake = _FakeUrlopen([(200, {})])
    monkeypatch.setattr(la.urllib.request, "urlopen", fake)
    a = _adapter()
    a._push_image("U1", "", caption="just a caption")
    assert len(fake.calls) == 1
    body = json.loads(fake.calls[0]["body_raw"])
    assert body["messages"][0]["type"] == "text"


# ---- _handle_webhook_body -------------------------------------------


def _signed(body: bytes, secret: str = "test-secret") -> str:
    return _make_signature(secret.encode("utf-8"), body)


def test_handle_webhook_valid_signature_emits_message_event(monkeypatch):
    a = _adapter()
    body = json.dumps({"events": [_user_event(text="hello")]}).encode("utf-8")
    sig = _signed(body)
    emitted = []
    status = a._handle_webhook_body(body, sig, emitted.append)
    assert status == 200
    assert len(emitted) == 1
    assert emitted[0]["params"]["content"] == {"Text": "hello"}


def test_handle_webhook_invalid_signature_returns_401():
    a = _adapter()
    body = json.dumps({"events": []}).encode("utf-8")
    emitted = []
    status = a._handle_webhook_body(body, "AAAA", emitted.append)
    assert status == 401
    assert emitted == []


def test_handle_webhook_missing_signature_returns_401():
    a = _adapter()
    body = json.dumps({"events": []}).encode("utf-8")
    emitted = []
    status = a._handle_webhook_body(body, "", emitted.append)
    assert status == 401
    assert emitted == []


def test_handle_webhook_bad_json_returns_400():
    a = _adapter()
    body = b"not-json"
    sig = _signed(body)
    emitted = []
    status = a._handle_webhook_body(body, sig, emitted.append)
    assert status == 400


def test_handle_webhook_non_object_body_returns_400():
    a = _adapter()
    body = b"[]"  # syntactically valid JSON, but not an object
    sig = _signed(body)
    emitted = []
    status = a._handle_webhook_body(body, sig, emitted.append)
    assert status == 400


def test_handle_webhook_empty_events_returns_200():
    """LINE pings ``{"destination":"...","events":[]}`` during
    webhook URL verification. The endpoint must respond 200 so the
    Developers Console marks the webhook as healthy."""
    a = _adapter()
    body = json.dumps({"destination": "U-bot", "events": []}).encode("utf-8")
    sig = _signed(body)
    emitted = []
    status = a._handle_webhook_body(body, sig, emitted.append)
    assert status == 200
    assert emitted == []


def test_handle_webhook_dedupes_repeated_message_id():
    """Improvement #2: an event with a previously-seen
    ``message.id`` is silently dropped on the second delivery."""
    a = _adapter()
    body = json.dumps({"events": [_user_event(msg_id="m-dup")]}).encode("utf-8")
    sig = _signed(body)
    emitted = []
    s1 = a._handle_webhook_body(body, sig, emitted.append)
    s2 = a._handle_webhook_body(body, sig, emitted.append)
    assert s1 == 200 and s2 == 200
    assert len(emitted) == 1


def test_handle_webhook_skips_non_message_event_without_dedupe_entry():
    """A ``follow`` event has no ``message`` block, so the dedupe
    set must stay empty — otherwise the very first real text
    message after a follow would be silently dropped."""
    a = _adapter()
    body = json.dumps({"events": [{
        "type": "follow",
        "source": {"type": "user", "userId": "U1"},
        "replyToken": "rt",
    }]}).encode("utf-8")
    sig = _signed(body)
    emitted = []
    status = a._handle_webhook_body(body, sig, emitted.append)
    assert status == 200
    assert emitted == []
    assert a._seen.ids == set()


def test_handle_webhook_account_id_injected_into_metadata(monkeypatch):
    a = _adapter(LINE_ACCOUNT_ID="prod-bot")
    body = json.dumps({"events": [_user_event()]}).encode("utf-8")
    sig = _signed(body)
    emitted = []
    a._handle_webhook_body(body, sig, emitted.append)
    assert emitted[0]["params"]["metadata"]["account_id"] == "prod-bot"


# ---- on_send (Send command) -----------------------------------------


def _send_cmd(channel_id="U1", text="hello", content=None, thread_id=None,
              user=None):
    from librefang.sidecar.protocol import Send
    return Send(channel_id, text, content, thread_id, user or {})


@pytest.mark.asyncio
async def test_on_send_text(monkeypatch):
    fake = _FakeUrlopen([(200, {})])
    monkeypatch.setattr(la.urllib.request, "urlopen", fake)
    a = _adapter()
    await a.on_send(_send_cmd(text="hello", content={"Text": "hello"}))
    assert len(fake.calls) == 1
    body = json.loads(fake.calls[0]["body_raw"])
    assert body == {
        "to": "U1",
        "messages": [{"type": "text", "text": "hello"}],
    }


@pytest.mark.asyncio
async def test_on_send_image(monkeypatch):
    fake = _FakeUrlopen([(200, {}), (200, {})])
    monkeypatch.setattr(la.urllib.request, "urlopen", fake)
    a = _adapter()
    await a.on_send(_send_cmd(
        text="",
        content={"Image": {"url": "https://x/y.jpg",
                            "caption": "look",
                            "mime_type": None}},
    ))
    # image push + caption push
    assert len(fake.calls) == 2
    img = json.loads(fake.calls[0]["body_raw"])
    assert img["messages"][0]["type"] == "image"
    cap = json.loads(fake.calls[1]["body_raw"])
    assert cap["messages"][0]["text"] == "look"


@pytest.mark.asyncio
async def test_on_send_unsupported_content_falls_back_to_placeholder(monkeypatch):
    """Matches the Rust adapter's fallback at line.rs:499-502."""
    fake = _FakeUrlopen([(200, {})])
    monkeypatch.setattr(la.urllib.request, "urlopen", fake)
    a = _adapter()
    await a.on_send(_send_cmd(
        text="",
        content={"Command": {"name": "noop", "args": []}},
    ))
    assert len(fake.calls) == 1
    body = json.loads(fake.calls[0]["body_raw"])
    assert body["messages"][0]["text"] == "(Unsupported content type)"


@pytest.mark.asyncio
async def test_on_send_empty_platform_id_drops_silently(monkeypatch):
    fake = _FakeUrlopen([])  # no calls expected
    monkeypatch.setattr(la.urllib.request, "urlopen", fake)
    a = _adapter()
    await a.on_send(_send_cmd(channel_id="", user={}))
    assert fake.calls == []


@pytest.mark.asyncio
async def test_on_send_falls_back_to_user_platform_id(monkeypatch):
    fake = _FakeUrlopen([(200, {})])
    monkeypatch.setattr(la.urllib.request, "urlopen", fake)
    a = _adapter()
    await a.on_send(_send_cmd(
        channel_id="",
        text="ping",
        content={"Text": "ping"},
        user={"platform_id": "Ufallback"},
    ))
    assert json.loads(fake.calls[0]["body_raw"])["to"] == "Ufallback"


# ---- LINE reply API (free / no-quota path) --------------------------


def test_parse_line_event_stashes_reply_token_via_librefang_user():
    """`librefang_user` is the only inbound field the daemon's
    bridge round-trips bytewise back to `cmd.user["librefang_user"]`
    on outbound — `metadata.reply_token` does not survive the trip.
    The fix stashes ``linereply:<token>:<event_ts_ms>`` here so
    `on_send` can reconstruct the token + freshness window."""
    event = {
        "type": "message",
        "replyToken": "RTOK123",
        "timestamp": 1_700_000_000_000,
        "source": {"type": "user", "userId": "U1"},
        "message": {"id": "M1", "type": "text", "text": "hi"},
    }
    parsed = la.parse_line_event(event)
    assert parsed is not None
    params = parsed["params"]
    assert params["librefang_user"] == (
        f"{la.LINE_REPLY_LIBREFANG_USER_PREFIX}RTOK123:1700000000000"
    )
    # The legacy `metadata.reply_token` continues to ship for
    # consumers that key on it (no breaking change in the metadata
    # shape).
    assert params["metadata"]["reply_token"] == "RTOK123"


def test_parse_line_event_omits_librefang_user_without_token():
    event = {
        "type": "message",
        # No replyToken — postback / webhook-verify shape.
        "timestamp": 1_700_000_000_000,
        "source": {"type": "user", "userId": "U1"},
        "message": {"id": "M1", "type": "text", "text": "hi"},
    }
    parsed = la.parse_line_event(event)
    assert parsed is not None
    assert "librefang_user" not in parsed["params"]


def test_parse_reply_carry_returns_token_and_deadline_inside_window():
    """A token whose embedded timestamp is within the safe 55 s
    window is recovered for the reply API. The returned tuple also
    carries the absolute epoch-ms deadline so `_post_reply` can skip
    a 429 retry that would overshoot it."""
    now_ms = 1_700_000_030_000  # 30 s after stash
    carry = f"{la.LINE_REPLY_LIBREFANG_USER_PREFIX}RTOK123:1700000000000"
    result = la.LineAdapter._parse_reply_carry(carry, now_ms=now_ms)
    assert result is not None
    token, deadline_ms = result
    assert token == "RTOK123"
    assert deadline_ms == 1_700_000_000_000 + int(
        la.LINE_REPLY_TOKEN_TTL_SECS * 1000
    )


def test_parse_reply_carry_rejects_expired_token():
    now_ms = 1_700_000_000_000 + int(la.LINE_REPLY_TOKEN_TTL_SECS * 1000) + 1
    carry = f"{la.LINE_REPLY_LIBREFANG_USER_PREFIX}RTOK123:1700000000000"
    # Older than the TTL → returns None so on_send degrades to push.
    assert la.LineAdapter._parse_reply_carry(carry, now_ms=now_ms) is None


def test_parse_reply_carry_rejects_cross_channel_garbage():
    """`librefang_user` is shared across channels (dingtalk stores a
    sessionWebhook URL there, telegram stores an @username, …) — a
    misrouted value must not be fed to LINE's reply endpoint. The
    `linereply:` prefix gate is the guard."""
    # dingtalk sessionWebhook shape
    assert la.LineAdapter._parse_reply_carry(
        "https://oapi.dingtalk.com/robot/sendBySession?session=...",
    ) is None
    # telegram @handle shape
    assert la.LineAdapter._parse_reply_carry("@alice") is None
    # bare reply token with no prefix / no timestamp
    assert la.LineAdapter._parse_reply_carry("RTOK123") is None
    # mangled trailing segment
    assert la.LineAdapter._parse_reply_carry(
        f"{la.LINE_REPLY_LIBREFANG_USER_PREFIX}RTOK123:not-a-number",
    ) is None
    # non-string
    assert la.LineAdapter._parse_reply_carry(None) is None
    assert la.LineAdapter._parse_reply_carry(12345) is None


@pytest.mark.asyncio
async def test_on_send_text_uses_reply_api_when_token_fresh(monkeypatch):
    """When the inbound event was recent, the outbound `on_send`
    POSTs to /v2/bot/message/reply (free; no quota burn) instead of
    /v2/bot/message/push."""
    fake = _FakeUrlopen([(200, {})])
    sleeps: list[float] = []
    monkeypatch.setattr(la.urllib.request, "urlopen", fake)
    monkeypatch.setattr(la.time, "sleep", sleeps.append)
    # Pretend "now" is 10 s after the embedded timestamp.
    now_ms = 1_700_000_010_000
    monkeypatch.setattr(la.time, "time", lambda: now_ms / 1000.0)
    a = _adapter()
    await a.on_send(_send_cmd(
        channel_id="U1",
        text="howdy",
        content={"Text": "howdy"},
        user={
            "librefang_user": (
                f"{la.LINE_REPLY_LIBREFANG_USER_PREFIX}RTOK123:1700000000000"
            ),
        },
    ))
    assert len(fake.calls) == 1
    assert fake.calls[0]["url"].endswith("/v2/bot/message/reply")
    body = json.loads(fake.calls[0]["body_raw"])
    assert body == {
        "replyToken": "RTOK123",
        "messages": [{"type": "text", "text": "howdy"}],
    }


@pytest.mark.asyncio
async def test_on_send_text_falls_back_to_push_when_token_stale(monkeypatch):
    """When the token has aged past the safe window, on_send skips
    the reply API and falls through to the push path so the agent's
    reply still lands."""
    fake = _FakeUrlopen([(200, {})])
    monkeypatch.setattr(la.urllib.request, "urlopen", fake)
    # Pretend "now" is 5 minutes past the embedded timestamp — well
    # over the 55 s window.
    monkeypatch.setattr(la.time, "time", lambda: 1_700_000_300.0)
    a = _adapter()
    await a.on_send(_send_cmd(
        channel_id="U1",
        text="howdy",
        content={"Text": "howdy"},
        user={
            "librefang_user": (
                f"{la.LINE_REPLY_LIBREFANG_USER_PREFIX}RTOK123:1700000000000"
            ),
        },
    ))
    assert len(fake.calls) == 1
    # Push path, not reply.
    assert fake.calls[0]["url"].endswith("/v2/bot/message/push")
    body = json.loads(fake.calls[0]["body_raw"])
    assert body["to"] == "U1"


@pytest.mark.asyncio
async def test_on_send_text_falls_back_to_push_on_reply_4xx(monkeypatch):
    """If LINE rejects the reply call (token already burned, etc.)
    the sidecar must still get the agent's response out via push so
    the user sees a reply. We assert two POSTs are issued: the
    rejected reply attempt then the push fallback."""
    fake = _FakeUrlopen([
        # reply attempt is rejected — LINE returns 400 with "Invalid
        # reply token".
        (400, {"message": "Invalid reply token"}),
        # push succeeds.
        (200, {}),
    ])
    monkeypatch.setattr(la.urllib.request, "urlopen", fake)
    monkeypatch.setattr(la.time, "time", lambda: 1_700_000_010.0)
    monkeypatch.setattr(la.time, "sleep", lambda _s: None)
    a = _adapter()
    await a.on_send(_send_cmd(
        channel_id="U1",
        text="howdy",
        content={"Text": "howdy"},
        user={
            "librefang_user": (
                f"{la.LINE_REPLY_LIBREFANG_USER_PREFIX}RTOK123:1700000000000"
            ),
        },
    ))
    assert len(fake.calls) == 2
    assert fake.calls[0]["url"].endswith("/v2/bot/message/reply")
    assert fake.calls[1]["url"].endswith("/v2/bot/message/push")
    body = json.loads(fake.calls[1]["body_raw"])
    assert body["to"] == "U1"


@pytest.mark.asyncio
async def test_on_send_image_does_not_attempt_reply(monkeypatch):
    """Image-and-caption is two pushes — the reply token would be
    one-shot-locked on the first call and the caption would error.
    Image path stays on push regardless of librefang_user shape."""
    fake = _FakeUrlopen([(200, {}), (200, {})])
    monkeypatch.setattr(la.urllib.request, "urlopen", fake)
    monkeypatch.setattr(la.time, "time", lambda: 1_700_000_010.0)
    a = _adapter()
    await a.on_send(_send_cmd(
        channel_id="U1",
        text="",
        content={"Image": {"url": "https://x/y.jpg",
                            "caption": "see this",
                            "mime_type": None}},
        user={
            "librefang_user": (
                f"{la.LINE_REPLY_LIBREFANG_USER_PREFIX}RTOK123:1700000000000"
            ),
        },
    ))
    # Both calls go through push, not reply.
    assert len(fake.calls) == 2
    for c in fake.calls:
        assert c["url"].endswith("/v2/bot/message/push")


@pytest.mark.asyncio
async def test_on_send_text_skips_reply_on_empty_text(monkeypatch):
    """LINE rejects empty text on both reply and push. The pre-fix
    push-only path made one wasted call; trying reply first would
    double the waste. Guard: skip reply entirely when text is empty
    (preserves the existing one-wasted-call behaviour and lets the
    push path handle the empty case as it always has)."""
    fake = _FakeUrlopen([(200, {})])
    monkeypatch.setattr(la.urllib.request, "urlopen", fake)
    monkeypatch.setattr(la.time, "time", lambda: 1_700_000_010.0)
    a = _adapter()
    await a.on_send(_send_cmd(
        channel_id="U1",
        text="",
        content={"Text": ""},
        user={
            "librefang_user": (
                f"{la.LINE_REPLY_LIBREFANG_USER_PREFIX}RTOK123:1700000000000"
            ),
        },
    ))
    # Exactly one call (push), reply is short-circuited.
    assert len(fake.calls) == 1
    assert fake.calls[0]["url"].endswith("/v2/bot/message/push")


def test_post_reply_short_circuits_when_text_exceeds_5_chunks(monkeypatch):
    """LINE's reply endpoint accepts at most 5 messages per token.
    A 6+-chunk payload can't be delivered via reply at all — the
    helper must return False without an API call so on_send falls
    back to push. Without this guard the wire call would fail with
    a 400 and the agent would lose the reply-vs-push signal."""
    a = _adapter()
    # Six chunks at LINE_MSG_LIMIT each — well past the 5-cap.
    big_text = "x" * (la.LINE_MSG_LIMIT * 6)
    # urlopen is NOT monkeypatched — if _post_reply tried to call it
    # the test would explode on the real socket attempt.
    fake_calls = {"count": 0}

    def _exploding_urlopen(*args, **kwargs):
        fake_calls["count"] += 1
        raise AssertionError(
            "_post_reply must not make an HTTP call when chunks > 5"
        )

    monkeypatch.setattr(la.urllib.request, "urlopen", _exploding_urlopen)
    result = a._post_reply("RTOK123", big_text)
    assert result is False
    assert fake_calls["count"] == 0


@pytest.mark.asyncio
async def test_on_send_falls_back_to_push_when_text_exceeds_5_chunks(monkeypatch):
    """End-to-end: a >5-chunk reply lands via push (chunked) without
    burning the reply token on a 400."""
    # 6 chunks => 6 push calls.
    fake = _FakeUrlopen([(200, {})] * 6)
    monkeypatch.setattr(la.urllib.request, "urlopen", fake)
    monkeypatch.setattr(la.time, "time", lambda: 1_700_000_010.0)
    a = _adapter()
    big_text = "x" * (la.LINE_MSG_LIMIT * 6)
    await a.on_send(_send_cmd(
        channel_id="U1",
        text=big_text,
        content={"Text": big_text},
        user={
            "librefang_user": (
                f"{la.LINE_REPLY_LIBREFANG_USER_PREFIX}RTOK123:1700000000000"
            ),
        },
    ))
    # 6 push calls, 0 reply calls.
    assert len(fake.calls) == 6
    for c in fake.calls:
        assert c["url"].endswith("/v2/bot/message/push")


def test_post_reply_skips_429_retry_when_sleep_would_overshoot_deadline(monkeypatch):
    """A 429 ``Retry-After`` sleep that would land past the reply-
    token deadline (~60 s from inbound) guarantees the retry hits
    "Invalid reply token". Skipping the sleep + retry returns control
    to on_send so push can deliver the agent's reply without burning
    the Retry-After window first."""
    a = _adapter()
    # Reply token stashed at t=1700000000000; deadline = +55s = 1700000055000.
    # Pretend "now" is 1700000050.0 (5 s before deadline). Retry-After=20s
    # would land at 1700000070000ms — 15s past deadline.
    monkeypatch.setattr(la.time, "time", lambda: 1_700_000_050.0)
    sleeps: list[float] = []
    monkeypatch.setattr(la.time, "sleep", sleeps.append)
    fake = _FakeUrlopen([
        (429, {"message": "rate limited"}, {"Retry-After": "20"}),
    ])
    monkeypatch.setattr(la.urllib.request, "urlopen", fake)
    deadline_ms = 1_700_000_055_000  # ts_ms + 55s
    result = a._post_reply("RTOK123", "hi", deadline_ms=deadline_ms)
    # Skip the sleep, skip the retry, return False so push picks up.
    assert result is False
    assert sleeps == []  # no sleep — would have overshot
    assert len(fake.calls) == 1  # one call (the 429), no retry


def test_post_reply_still_retries_429_when_sleep_fits_under_deadline(monkeypatch):
    """When Retry-After fits comfortably under the deadline we still
    sleep + retry — the optimisation must not over-trigger and kill
    the reply path on every 429."""
    a = _adapter()
    monkeypatch.setattr(la.time, "time", lambda: 1_700_000_010.0)
    sleeps: list[float] = []
    monkeypatch.setattr(la.time, "sleep", sleeps.append)
    fake = _FakeUrlopen([
        (429, {"message": "rate limited"}, {"Retry-After": "2"}),
        (200, {}),
    ])
    monkeypatch.setattr(la.urllib.request, "urlopen", fake)
    deadline_ms = 1_700_000_055_000
    result = a._post_reply("RTOK123", "hi", deadline_ms=deadline_ms)
    # 2s sleep fits well under the 45-second remaining budget.
    assert result is True
    assert sleeps == [2.0]
    assert len(fake.calls) == 2


# ---- schema (--describe) --------------------------------------------


def test_schema_round_trip():
    """The ``--describe`` JSON payload must enumerate every env-var
    the daemon needs to render the dashboard form, including the
    two required secrets and the three advanced knobs."""
    schema = la.LineAdapter.SCHEMA.to_dict()
    assert schema["name"] == "line"
    keys = {f["key"] for f in schema["fields"]}
    assert "LINE_CHANNEL_SECRET" in keys
    assert "LINE_CHANNEL_ACCESS_TOKEN" in keys
    assert "LINE_WEBHOOK_PORT" in keys
    assert "LINE_WEBHOOK_PATH" in keys
    assert "LINE_ACCOUNT_ID" in keys
    secret_fields = {
        f["key"] for f in schema["fields"] if f["type"] == "secret"
    }
    assert secret_fields == {"LINE_CHANNEL_SECRET",
                             "LINE_CHANNEL_ACCESS_TOKEN"}
