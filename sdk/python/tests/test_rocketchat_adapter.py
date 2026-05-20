"""Tests for librefang.sidecar.adapters.rocketchat.

Deterministic, no network: urllib is monkeypatched. Asserts the
sidecar Rocket.Chat adapter preserves the behaviour of the removed
in-process Rust ``librefang-channels::rocketchat`` adapter, plus
three explicitly-acknowledged improvements:

* **P1**: ``thread_id`` is the inbound ``_id`` (or inbound ``tmid``
  when the user was already in a thread), so ``on_send`` rounds it
  back to ``tmid`` on ``chat.postMessage`` — fixes the Rust adapter
  bug where threaded replies always landed at the room root.
* **P2**: dedupe set on ``_id`` (matches reddit / bluesky), so two
  messages with the same RFC3339 ``ts`` no longer cause re-emission
  on the next poll.
* **P3**: self-skip by stable ``u._id`` (with username fallback)
  instead of username-only.
"""

import io
import json
import os
import urllib.error

import pytest

# Required env must be present at import time because the adapter
# raises SystemExit(2) on missing values.
os.environ.setdefault("ROCKETCHAT_SERVER_URL", "https://chat.example.com")
os.environ.setdefault("ROCKETCHAT_TOKEN", "test-token")
os.environ.setdefault("ROCKETCHAT_USER_ID", "BOT_UID")
from librefang.sidecar.adapters import rocketchat as ra  # noqa: E402


def _adapter(**env):
    defaults = {
        "ROCKETCHAT_SERVER_URL": "https://chat.example.com",
        "ROCKETCHAT_TOKEN": "test-token",
        "ROCKETCHAT_USER_ID": "BOT_UID",
        "ROCKETCHAT_CHANNELS": "",
        "ROCKETCHAT_ACCOUNT_ID": "",
        "ROCKETCHAT_POLL_INTERVAL_SECS": "",
    }
    for k, v in defaults.items():
        os.environ[k] = env.get(k, v)
    return ra.RocketChatAdapter()


# ---- env handling -------------------------------------------------


def test_required_env_present():
    a = _adapter()
    assert a.server_url == "https://chat.example.com"
    assert a.token == "test-token"
    assert a.user_id == "BOT_UID"
    assert a.allowed_channels == []
    assert a.account_id is None
    assert a.poll_interval == ra.DEFAULT_POLL_INTERVAL_SECS


def test_server_url_trailing_slash_stripped():
    """Mirrors the Rust adapter's `trim_end_matches('/')`."""
    a = _adapter(ROCKETCHAT_SERVER_URL="https://chat.example.com/")
    assert a.server_url == "https://chat.example.com"
    a = _adapter(ROCKETCHAT_SERVER_URL="https://chat.example.com///")
    assert a.server_url == "https://chat.example.com"


def test_server_url_scheme_validated():
    """Refuse to start on a bare hostname — silent failure here would
    explode mid-poll with a confusing urllib error."""
    with pytest.raises(SystemExit) as exc:
        _adapter(ROCKETCHAT_SERVER_URL="chat.example.com")
    assert exc.value.code == 2


def test_missing_required_env_exits():
    for var in ("ROCKETCHAT_SERVER_URL", "ROCKETCHAT_TOKEN",
                "ROCKETCHAT_USER_ID"):
        with pytest.raises(SystemExit) as exc:
            _adapter(**{var: ""})
        assert exc.value.code == 2, var


def test_channels_parsed_comma_separated():
    a = _adapter(ROCKETCHAT_CHANNELS=" GENERAL , room2 ,room3")
    assert a.allowed_channels == ["GENERAL", "room2", "room3"]


def test_channels_empty_means_discover():
    a = _adapter(ROCKETCHAT_CHANNELS="")
    assert a.allowed_channels == []


def test_account_id_optional():
    a = _adapter(ROCKETCHAT_ACCOUNT_ID="prod")
    assert a.account_id == "prod"
    a = _adapter(ROCKETCHAT_ACCOUNT_ID="")
    assert a.account_id is None


def test_poll_interval_env_override():
    a = _adapter(ROCKETCHAT_POLL_INTERVAL_SECS="10")
    assert a.poll_interval == 10


def test_poll_interval_below_floor_clamped():
    a = _adapter(ROCKETCHAT_POLL_INTERVAL_SECS="0")
    assert a.poll_interval == ra.MIN_POLL_INTERVAL_SECS


def test_poll_interval_invalid_exits():
    with pytest.raises(SystemExit) as exc:
        _adapter(ROCKETCHAT_POLL_INTERVAL_SECS="not-a-number")
    assert exc.value.code == 2


# ---- suppress + capabilities --------------------------------------


def test_suppress_error_responses_is_true_in_ready_event():
    """Rocket.Chat messages are public to a room; never echo errors."""
    a = _adapter()
    assert a.suppress_error_responses is True
    p = a.ready_event()["params"]
    assert p.get("suppress_error_responses") is True


def test_capabilities_empty():
    a = _adapter()
    assert a.capabilities == []


def test_account_id_in_ready_event():
    a = _adapter(ROCKETCHAT_ACCOUNT_ID="acct-1")
    p = a.ready_event()["params"]
    assert p.get("account_id") == "acct-1"


# ---- _split_message ----------------------------------------------


def test_split_message_under_limit_one_chunk():
    assert ra._split_message("short", 100) == ["short"]


def test_split_message_prefers_newline_cut():
    body = "a" * 80 + "\n" + "b" * 80
    chunks = ra._split_message(body, 100)
    assert len(chunks) == 2
    assert chunks[0] == "a" * 80
    assert chunks[1] == "b" * 80


def test_split_message_hard_cut_when_no_newline():
    chunks = ra._split_message("x" * 250, 100)
    assert [len(c) for c in chunks] == [100, 100, 50]


def test_split_message_4096_cap_matches_rust():
    """The Rust adapter's MAX_MESSAGE_LEN is 4096."""
    assert ra.MAX_MESSAGE_LEN == 4096


# ---- _FakeUrlopen scaffolding --------------------------------------


class _HdrShim:
    def __init__(self, hdrs: dict):
        self._hdrs = hdrs or {}

    def items(self):
        return list(self._hdrs.items())


class _FakeUrlopen:
    """Capture urllib.request.urlopen calls and return scripted
    responses. Each script entry is ``(status, body)`` or
    ``(status, body, response_headers)``; the 2-tuple form keeps
    existing tests unchanged."""

    def __init__(self, script):
        self.script = list(script)
        self.calls = []

    def __call__(self, req, timeout=None):
        body_bytes = req.data
        try:
            decoded = body_bytes.decode("utf-8") if body_bytes else None
        except Exception:
            decoded = None
        self.calls.append({
            "url": req.full_url,
            "method": req.get_method(),
            "headers": {k.lower(): v for k, v in req.header_items()},
            "body_raw": decoded,
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


def _msg(
    *,
    mid="m1",
    text="hello rocketchat",
    ts="2026-01-01T00:00:00.000Z",
    u_id="USER_A",
    u_name="alice",
    tmid=None,
):
    out = {
        "_id": mid,
        "msg": text,
        "ts": ts,
        "u": {"_id": u_id, "username": u_name},
    }
    if tmid is not None:
        out["tmid"] = tmid
    return out


# ---- /api/v1/me + auth headers ------------------------------------


def test_verify_credentials_sends_auth_headers(monkeypatch):
    a = _adapter()
    fake = _FakeUrlopen([(200, {"username": "librefang-bot"})])
    monkeypatch.setattr(ra.urllib.request, "urlopen", fake)
    name = a._verify_credentials()
    assert name == "librefang-bot"
    assert a.own_username == "librefang-bot"
    call = fake.calls[0]
    assert call["url"] == "https://chat.example.com/api/v1/me"
    assert call["headers"]["x-auth-token"] == "test-token"
    assert call["headers"]["x-user-id"] == "BOT_UID"


def test_verify_credentials_raises_on_401(monkeypatch):
    a = _adapter()
    fake = _FakeUrlopen([(401, {"status": "error"})])
    monkeypatch.setattr(ra.urllib.request, "urlopen", fake)
    with pytest.raises(RuntimeError, match="authentication failed 401"):
        a._verify_credentials()


def test_verify_credentials_accepts_missing_username(monkeypatch):
    """If `/api/v1/me` 200s but omits `username`, the adapter must
    keep running — self-skip falls back to `ROCKETCHAT_USER_ID`. The
    Rust adapter accepted "unknown" silently; we log a warning."""
    a = _adapter()
    fake = _FakeUrlopen([(200, {})])
    monkeypatch.setattr(ra.urllib.request, "urlopen", fake)
    name = a._verify_credentials()
    assert name == "unknown"
    assert a.own_username == ""


# ---- channels.list.joined -----------------------------------------


def test_list_joined_channels_parses_ids(monkeypatch):
    a = _adapter()
    fake = _FakeUrlopen([(200, {"channels": [
        {"_id": "room1", "name": "general"},
        {"_id": "room2", "name": "random"},
        {"name": "no-id"},  # skipped
    ]})])
    monkeypatch.setattr(ra.urllib.request, "urlopen", fake)
    ids = a._list_joined_channels()
    assert ids == ["room1", "room2"]
    assert "channels.list.joined" in fake.calls[0]["url"]


def test_list_joined_channels_returns_empty_on_error(monkeypatch):
    a = _adapter()
    fake = _FakeUrlopen([(500, {"error": "boom"})])
    monkeypatch.setattr(ra.urllib.request, "urlopen", fake)
    assert a._list_joined_channels() == []


def test_list_joined_channels_handles_transport_error(monkeypatch):
    a = _adapter()

    def boom(req, timeout=None):
        raise urllib.error.URLError("dns")

    monkeypatch.setattr(ra.urllib.request, "urlopen", boom)
    assert a._list_joined_channels() == []


# ---- _parse_message ----------------------------------------------


def test_parse_basic_text():
    a = _adapter()
    a.own_username = "librefang-bot"
    ev = a._parse_message(_msg(), "ROOM1")
    assert ev is not None
    p = ev["params"]
    assert p["content"] == {"Text": "hello rocketchat"}
    assert p["message_id"] == "m1"
    assert p["channel_id"] == "ROOM1"
    assert p["user_id"] == "ROOM1"  # platform routing key
    assert p["user_name"] == "alice"
    assert p["is_group"] is True
    # P1: thread_id is the inbound _id (top-level), not the room id.
    assert p["thread_id"] == "m1"
    md = p["metadata"]
    assert md["sender_id"] == "USER_A"
    assert md["sender_username"] == "alice"
    assert md["room_id"] == "ROOM1"
    assert md["ts"] == "2026-01-01T00:00:00.000Z"
    assert "tmid" not in md


def test_parse_thread_reply_uses_tmid_as_thread_id():
    """P1: when the inbound message is itself a thread reply, the
    outbound thread_id MUST be the existing tmid — otherwise the bot
    would start a child thread under the user's reply instead of
    threading alongside them."""
    a = _adapter()
    a.own_username = "librefang-bot"
    ev = a._parse_message(
        _msg(mid="m2", tmid="PARENT_THREAD", text="reply"),
        "ROOM1",
    )
    p = ev["params"]
    assert p["thread_id"] == "PARENT_THREAD"
    assert p["metadata"]["tmid"] == "PARENT_THREAD"


def test_parse_skips_self_by_user_id():
    """P3: prefer u._id == ROCKETCHAT_USER_ID over username, so a bot
    that rotates its display name still self-skips correctly."""
    a = _adapter()
    a.own_username = "librefang-bot"
    skip = a._parse_message(
        _msg(u_id="BOT_UID", u_name="bot-display-name-changed"),
        "ROOM1",
    )
    assert skip is None


def test_parse_falls_back_to_username_when_uid_missing():
    """P3 fallback: when the inbound shape omits u._id (older
    Rocket.Chat versions / custom routes), self-skip by username."""
    a = _adapter()
    a.own_username = "librefang-bot"
    msg = {
        "_id": "m1",
        "msg": "from me",
        "ts": "2026-01-01T00:00:00.000Z",
        "u": {"username": "librefang-bot"},  # no _id
    }
    assert a._parse_message(msg, "ROOM1") is None


def test_parse_username_fallback_disabled_when_own_username_empty():
    """If `/api/v1/me` returned no username (own_username == ""), the
    fallback must not match an empty username on a message (or the
    bot would skip every message from anonymous senders)."""
    a = _adapter()
    a.own_username = ""
    msg = {
        "_id": "m1",
        "msg": "from someone",
        "ts": "2026-01-01T00:00:00.000Z",
        "u": {"username": ""},  # no _id, empty username
    }
    ev = a._parse_message(msg, "ROOM1")
    # Empty u.username should not trigger self-skip when own_username
    # is also empty — the message should pass through.
    assert ev is not None


def test_parse_skips_empty_body():
    a = _adapter()
    assert a._parse_message(_msg(text=""), "ROOM1") is None


def test_parse_command_form():
    a = _adapter()
    ev = a._parse_message(_msg(text="/help me out"), "ROOM1")
    assert ev["params"]["content"] == {
        "Command": {"name": "help", "args": ["me", "out"]},
    }


def test_parse_command_no_args():
    a = _adapter()
    ev = a._parse_message(_msg(text="/ping"), "ROOM1")
    assert ev["params"]["content"] == {
        "Command": {"name": "ping", "args": []},
    }


def test_parse_malformed_inputs():
    a = _adapter()
    assert a._parse_message("not a dict", "ROOM1") is None
    # Missing _id / msg → falsy text → skip.
    assert a._parse_message({"ts": "x"}, "ROOM1") is None


# ---- _poll_once: emit + dedupe + watermark ------------------------


def test_poll_once_emits_messages_and_advances_watermark(monkeypatch):
    a = _adapter()
    a.own_username = "librefang-bot"
    a._room_watermarks["R1"] = "2026-01-01T00:00:00.000Z"
    # Rocket.Chat's `channels.history` returns messages NEWEST-FIRST, so
    # the scripted payload has m2 (ts ...06) ahead of m1 (ts ...05) the
    # way the real API would. The adapter must re-order to chronological
    # before emitting.
    fake = _FakeUrlopen([
        (200, {"messages": [
            _msg(mid="m2", ts="2026-01-01T00:00:06.000Z",
                 text="/cmd args"),
            _msg(mid="m1", ts="2026-01-01T00:00:05.000Z"),
        ]}),
    ])
    monkeypatch.setattr(ra.urllib.request, "urlopen", fake)
    emitted = []
    a._poll_once(emitted.append, ["R1"])
    assert len(emitted) == 2
    # Emitted oldest-first despite the newest-first API order.
    assert emitted[0]["params"]["message_id"] == "m1"
    assert emitted[1]["params"]["message_id"] == "m2"
    assert emitted[1]["params"]["content"] == {
        "Command": {"name": "cmd", "args": ["args"]},
    }
    # Watermark advanced to the newest ts seen.
    assert a._room_watermarks["R1"] == "2026-01-01T00:00:06.000Z"
    # Both message ids tracked for dedupe.
    assert "m1" in a._seen_messages_set
    assert "m2" in a._seen_messages_set


def test_poll_once_emits_in_chronological_order(monkeypatch):
    """Regression: `channels.history` returns newest-first. A burst of
    several messages caught in one poll must reach the agent oldest →
    newest, not reversed. The Rust adapter (and the first cut of this
    sidecar) iterated the raw newest-first array and delivered the burst
    backwards."""
    a = _adapter()
    a.own_username = "librefang-bot"
    a._room_watermarks["R1"] = "2026-01-01T00:00:00.000Z"
    # API order: newest (m3) first, oldest (m1) last.
    fake = _FakeUrlopen([
        (200, {"messages": [
            _msg(mid="m3", ts="2026-01-01T00:00:30.000Z", text="third"),
            _msg(mid="m2", ts="2026-01-01T00:00:20.000Z", text="second"),
            _msg(mid="m1", ts="2026-01-01T00:00:10.000Z", text="first"),
        ]}),
    ])
    monkeypatch.setattr(ra.urllib.request, "urlopen", fake)
    emitted = []
    a._poll_once(emitted.append, ["R1"])
    assert [e["params"]["message_id"] for e in emitted] == ["m1", "m2", "m3"]
    assert [e["params"]["content"]["Text"] for e in emitted] == [
        "first", "second", "third",
    ]


def test_poll_once_dedupes_same_ts_repeats(monkeypatch):
    """P2: two messages with identical ts must not re-emit on the next
    poll. The Rust adapter advanced `last_timestamps` to the max ts
    seen and re-fetched `oldest=<ts>`; same-ts duplicates that landed
    in subsequent fetches would be emitted twice. The sidecar dedupes
    on _id."""
    a = _adapter()
    a.own_username = "librefang-bot"
    a._room_watermarks["R1"] = "2026-01-01T00:00:00.000Z"
    # First poll emits m1, m2.
    # Second poll returns the same two ids (server re-includes them
    # because oldest=ts is inclusive of the boundary).
    fake = _FakeUrlopen([
        (200, {"messages": [
            _msg(mid="m1", ts="2026-01-01T00:00:05.000Z"),
            _msg(mid="m2", ts="2026-01-01T00:00:05.000Z"),
        ]}),
        (200, {"messages": [
            _msg(mid="m1", ts="2026-01-01T00:00:05.000Z"),
            _msg(mid="m2", ts="2026-01-01T00:00:05.000Z"),
            _msg(mid="m3", ts="2026-01-01T00:00:06.000Z"),
        ]}),
    ])
    monkeypatch.setattr(ra.urllib.request, "urlopen", fake)
    emitted = []
    a._poll_once(emitted.append, ["R1"])
    # Order is asserted by the chronological-order tests; here we only
    # care that both same-ts ids are emitted exactly once.
    assert sorted(e["params"]["message_id"] for e in emitted) == ["m1", "m2"]
    emitted.clear()
    a._poll_once(emitted.append, ["R1"])
    # Only the new m3 emits on the second poll.
    assert [e["params"]["message_id"] for e in emitted] == ["m3"]


def test_poll_once_self_skipped(monkeypatch):
    a = _adapter()
    a.own_username = "librefang-bot"
    a._room_watermarks["R1"] = ""
    fake = _FakeUrlopen([
        (200, {"messages": [
            _msg(mid="m1", u_id="BOT_UID", u_name="librefang-bot"),
            _msg(mid="m2", u_id="USER_A"),
        ]}),
    ])
    monkeypatch.setattr(ra.urllib.request, "urlopen", fake)
    emitted = []
    a._poll_once(emitted.append, ["R1"])
    assert [e["params"]["message_id"] for e in emitted] == ["m2"]
    # Even self-skipped m1 is still marked seen so we don't reparse it.
    assert "m1" in a._seen_messages_set


def test_poll_once_injects_account_id(monkeypatch):
    a = _adapter(ROCKETCHAT_ACCOUNT_ID="prod")
    a.own_username = "librefang-bot"
    a._room_watermarks["R1"] = ""
    fake = _FakeUrlopen([
        (200, {"messages": [_msg(mid="m1")]}),
    ])
    monkeypatch.setattr(ra.urllib.request, "urlopen", fake)
    emitted = []
    a._poll_once(emitted.append, ["R1"])
    assert emitted[0]["params"]["metadata"]["account_id"] == "prod"


def test_poll_once_401_raises(monkeypatch):
    a = _adapter()
    fake = _FakeUrlopen([(401, {"status": "error"})])
    monkeypatch.setattr(ra.urllib.request, "urlopen", fake)
    with pytest.raises(RuntimeError, match="401"):
        a._poll_once(lambda _: None, ["R1"])


def test_poll_once_skips_room_on_transport_error(monkeypatch):
    """One bad room doesn't take the loop down; the next room's fetch
    still runs in the same poll pass."""
    a = _adapter()
    a.own_username = "librefang-bot"
    a._room_watermarks["R1"] = ""
    a._room_watermarks["R2"] = ""
    calls = []

    def fake_urlopen(req, timeout=None):
        url = req.full_url
        calls.append(url)
        if "roomId=R1" in url:
            raise urllib.error.URLError("dns error")
        if "roomId=R2" in url:
            return _FakeResp(200, json.dumps({"messages": [_msg(mid="m1")]}).encode("utf-8"))
        raise AssertionError(f"unexpected {url}")

    monkeypatch.setattr(ra.urllib.request, "urlopen", fake_urlopen)
    emitted = []
    a._poll_once(emitted.append, ["R1", "R2"])
    assert len(emitted) == 1
    assert emitted[0]["params"]["channel_id"] == "R2"
    assert any("roomId=R1" in c for c in calls)
    assert any("roomId=R2" in c for c in calls)


def test_poll_once_non_200_logged_and_skipped(monkeypatch):
    """A 500 on one poll surfaces a warning but does not raise — the
    caller's exponential backoff would otherwise turn a transient
    upstream blip into a multi-minute pause."""
    a = _adapter()
    a._room_watermarks["R1"] = ""
    fake = _FakeUrlopen([(500, {"error": "transient"})])
    monkeypatch.setattr(ra.urllib.request, "urlopen", fake)
    emitted = []
    a._poll_once(emitted.append, ["R1"])  # must not raise
    assert emitted == []


def test_poll_once_request_shape(monkeypatch):
    """Asserts URL + auth headers for a single-room poll. Covers
    `oldest=<watermark>`, `count=50`, `roomId=<R>` — the API contract
    every operator's reverse proxy / WAF will see."""
    a = _adapter()
    a._room_watermarks["R1"] = "2026-01-01T00:00:00.000Z"
    fake = _FakeUrlopen([(200, {"messages": []})])
    monkeypatch.setattr(ra.urllib.request, "urlopen", fake)
    a._poll_once(lambda _: None, ["R1"])
    call = fake.calls[0]
    assert call["url"].startswith(
        "https://chat.example.com/api/v1/channels.history?"
    )
    assert "roomId=R1" in call["url"]
    assert "oldest=2026-01-01T00%3A00%3A00.000Z" in call["url"]
    assert "count=50" in call["url"]
    assert call["headers"]["x-auth-token"] == "test-token"
    assert call["headers"]["x-user-id"] == "BOT_UID"


# ---- dedupe set capacity ------------------------------------------


def test_seen_messages_capacity_eviction():
    a = _adapter()
    for i in range(ra.SEEN_MESSAGES_MAX + 1):
        a._mark_seen(f"m{i}")
    assert "m0" not in a._seen_messages_set
    assert f"m{ra.SEEN_MESSAGES_EVICT - 1}" not in a._seen_messages_set
    assert f"m{ra.SEEN_MESSAGES_EVICT}" in a._seen_messages_set
    assert f"m{ra.SEEN_MESSAGES_MAX}" in a._seen_messages_set
    assert len(a._seen_messages) == len(a._seen_messages_set)


def test_seen_messages_idempotent_mark():
    a = _adapter()
    a._mark_seen("x")
    a._mark_seen("x")
    assert a._seen_messages.count("x") == 1


def test_seen_messages_empty_id_ignored():
    a = _adapter()
    a._mark_seen("")
    assert a._seen_messages == []


# ---- _post_message: outbound + threading --------------------------


def test_post_message_basic_shape(monkeypatch):
    a = _adapter()
    fake = _FakeUrlopen([(200, {"success": True})])
    monkeypatch.setattr(ra.urllib.request, "urlopen", fake)
    a._post_message("ROOM1", "hello from librefang", None)
    call = fake.calls[0]
    assert call["url"] == "https://chat.example.com/api/v1/chat.postMessage"
    assert call["method"] == "POST"
    assert call["headers"]["x-auth-token"] == "test-token"
    assert call["headers"]["x-user-id"] == "BOT_UID"
    assert call["headers"]["content-type"] == "application/json"
    body = json.loads(call["body_raw"])
    assert body == {"roomId": "ROOM1", "text": "hello from librefang"}


def test_post_message_with_thread_includes_tmid(monkeypatch):
    """P1: when on_send forwards thread_id, the outbound payload MUST
    include `tmid` so Rocket.Chat threads the reply."""
    a = _adapter()
    fake = _FakeUrlopen([(200, {"success": True})])
    monkeypatch.setattr(ra.urllib.request, "urlopen", fake)
    a._post_message("ROOM1", "threaded reply", "PARENT_MSG_ID")
    body = json.loads(fake.calls[0]["body_raw"])
    assert body == {
        "roomId": "ROOM1",
        "text": "threaded reply",
        "tmid": "PARENT_MSG_ID",
    }


def test_post_message_chunks_long_text(monkeypatch):
    """Long messages chunk at MAX_MESSAGE_LEN and post as separate
    messages. Matches the Rust per-chunk loop."""
    a = _adapter()
    fake = _FakeUrlopen([
        (200, {"success": True}),
        (200, {"success": True}),
        (200, {"success": True}),
    ])
    monkeypatch.setattr(ra.urllib.request, "urlopen", fake)
    long_text = "x" * (ra.MAX_MESSAGE_LEN * 2 + 100)
    a._post_message("ROOM1", long_text, None)
    assert len(fake.calls) == 3
    total = sum(
        len(json.loads(c["body_raw"])["text"]) for c in fake.calls
    )
    assert total == len(long_text)


def test_post_message_chunks_preserve_tmid(monkeypatch):
    """When the outbound is threaded AND chunked, every chunk must
    carry the same `tmid` so the whole multi-part reply lives in the
    same thread."""
    a = _adapter()
    fake = _FakeUrlopen([
        (200, {"success": True}),
        (200, {"success": True}),
    ])
    monkeypatch.setattr(ra.urllib.request, "urlopen", fake)
    long_text = "x" * (ra.MAX_MESSAGE_LEN + 10)
    a._post_message("ROOM1", long_text, "T1")
    for c in fake.calls:
        body = json.loads(c["body_raw"])
        assert body["tmid"] == "T1"


def test_post_message_missing_room_id_raises():
    a = _adapter()
    with pytest.raises(RuntimeError, match="missing room id"):
        a._post_message("", "hi", None)


def test_post_message_non_2xx_surfaces(monkeypatch):
    a = _adapter()
    fake = _FakeUrlopen([(401, {"error": "Unauthorized"})])
    monkeypatch.setattr(ra.urllib.request, "urlopen", fake)
    with pytest.raises(RuntimeError, match="401"):
        a._post_message("ROOM1", "hi", None)


def test_post_message_soft_error_logged(monkeypatch):
    """200 with success=false is a Rocket.Chat soft-error (e.g.
    permission denied). The Rust adapter ignored the body shape; the
    sidecar logs a warning so an operator notices."""
    a = _adapter()
    fake = _FakeUrlopen([(200, {
        "success": False,
        "error": "not-in-room",
    })])
    monkeypatch.setattr(ra.urllib.request, "urlopen", fake)
    # Must NOT raise — 2xx is still 2xx; this is a server-side soft
    # error the operator should see in logs but not a crash signal.
    a._post_message("ROOM1", "hi", None)


# ---- on_send wiring -----------------------------------------------


class _StubCmd:
    def __init__(self, *, text=None, content=None, thread_id=None,
                 user=None, channel_id=None):
        self.text = text
        self.content = content
        self.thread_id = thread_id
        self.user = user or {}
        self.channel_id = channel_id


def test_on_send_uses_platform_id_as_room(monkeypatch):
    """`cmd.user.platform_id` carries the room id from inbound
    (matches the Rust ChannelUser{platform_id: room_id} shape)."""
    a = _adapter()
    fake = _FakeUrlopen([(200, {"success": True})])
    monkeypatch.setattr(ra.urllib.request, "urlopen", fake)
    import asyncio as _asyncio
    _asyncio.run(a.on_send(_StubCmd(
        text="hi",
        user={"platform_id": "ROOM1"},
    )))
    body = json.loads(fake.calls[0]["body_raw"])
    assert body["roomId"] == "ROOM1"
    assert body["text"] == "hi"
    assert "tmid" not in body


def test_on_send_threads_via_thread_id(monkeypatch):
    """P1 round-trip: cmd.thread_id → tmid on the outbound."""
    a = _adapter()
    fake = _FakeUrlopen([(200, {"success": True})])
    monkeypatch.setattr(ra.urllib.request, "urlopen", fake)
    import asyncio as _asyncio
    _asyncio.run(a.on_send(_StubCmd(
        text="threaded reply",
        thread_id="PARENT_MSG",
        user={"platform_id": "ROOM1"},
    )))
    body = json.loads(fake.calls[0]["body_raw"])
    assert body["tmid"] == "PARENT_MSG"


def test_on_send_falls_back_to_channel_id(monkeypatch):
    """If `user.platform_id` is empty (pre-#5219 daemon stripping
    `user`), fall back to `cmd.channel_id` so the bot still routes."""
    a = _adapter()
    fake = _FakeUrlopen([(200, {"success": True})])
    monkeypatch.setattr(ra.urllib.request, "urlopen", fake)
    import asyncio as _asyncio
    _asyncio.run(a.on_send(_StubCmd(
        text="fallback",
        channel_id="ROOM2",
    )))
    body = json.loads(fake.calls[0]["body_raw"])
    assert body["roomId"] == "ROOM2"


def test_on_send_non_text_content_falls_back_to_placeholder(monkeypatch):
    a = _adapter()
    fake = _FakeUrlopen([(200, {"success": True})])
    monkeypatch.setattr(ra.urllib.request, "urlopen", fake)
    import asyncio as _asyncio
    _asyncio.run(a.on_send(_StubCmd(
        content={"Reaction": {"emoji": "👍"}},
        user={"platform_id": "ROOM1"},
    )))
    body = json.loads(fake.calls[0]["body_raw"])
    assert "Unsupported content type" in body["text"]


# ---- 429 / Retry-After (Rocket.Chat REST rate-limiting) ---------


def test_retry_after_secs_parses_header_value():
    """``Retry-After`` (seconds form) is parsed as a float and capped
    at ``MAX_BACKOFF_SECS`` so a misreported value can't block the
    poller for more than a minute."""
    assert ra.RocketChatAdapter._retry_after_secs({"retry-after": "5"}) == 5.0
    assert ra.RocketChatAdapter._retry_after_secs({"retry-after": "0.5"}) == 1.0
    assert (
        ra.RocketChatAdapter._retry_after_secs({"retry-after": "9999"})
        == ra.MAX_BACKOFF_SECS
    )


def test_retry_after_secs_falls_back_when_absent_or_invalid():
    """Without a ``Retry-After`` (or with an HTTP-date form we don't
    decode), fall back to ``RETRY_AFTER_DEFAULT_SECS`` rather than
    busy-looping at 1 s."""
    assert (
        ra.RocketChatAdapter._retry_after_secs({})
        == ra.RETRY_AFTER_DEFAULT_SECS
    )
    assert (
        ra.RocketChatAdapter._retry_after_secs(
            {"retry-after": "Thu, 01 Jan 2099 00:00:00 GMT"},
        )
        == ra.RETRY_AFTER_DEFAULT_SECS
    )


def test_verify_credentials_429_sleeps_retry_after_then_raises(monkeypatch):
    """Rocket.Chat rate-limits unauthenticated / failed-auth probes;
    the verify retry loop in `_producer_blocking` would otherwise
    compound with the server-side window."""
    a = _adapter()
    fake = _FakeUrlopen([(429, {"error": "throttled"}, {"Retry-After": "3"})])
    monkeypatch.setattr(ra.urllib.request, "urlopen", fake)
    sleeps: list = []
    monkeypatch.setattr(ra.time, "sleep", lambda s: sleeps.append(s))
    with pytest.raises(RuntimeError, match="429"):
        a._verify_credentials()
    assert sleeps == [3.0]


def test_verify_credentials_429_without_header_uses_default(monkeypatch):
    """A 429 with no ``Retry-After`` falls back to
    ``RETRY_AFTER_DEFAULT_SECS`` instead of busy-looping at 1 s."""
    a = _adapter()
    fake = _FakeUrlopen([(429, {"error": "throttled"})])
    monkeypatch.setattr(ra.urllib.request, "urlopen", fake)
    sleeps: list = []
    monkeypatch.setattr(ra.time, "sleep", lambda s: sleeps.append(s))
    with pytest.raises(RuntimeError, match="429"):
        a._verify_credentials()
    assert sleeps == [ra.RETRY_AFTER_DEFAULT_SECS]


def test_list_joined_channels_429_sleeps_then_returns_empty(monkeypatch):
    """Channel discovery is one-shot; the producer just retries on the
    next pass, so a 429 here only needs to sleep — surfacing it as an
    empty list (same as transport error) is enough."""
    a = _adapter()
    fake = _FakeUrlopen([(429, {"error": "throttled"}, {"Retry-After": "4"})])
    monkeypatch.setattr(ra.urllib.request, "urlopen", fake)
    sleeps: list = []
    monkeypatch.setattr(ra.time, "sleep", lambda s: sleeps.append(s))
    out = a._list_joined_channels()
    assert out == []
    assert sleeps == [4.0]


def test_poll_once_429_sleeps_retry_after_then_raises(monkeypatch):
    """channels.history 429 must sleep the indicated interval and
    raise so the outer backoff in `_producer_blocking` pauses before
    the next pass — otherwise the per-room loop probes inside the
    window and extends the throttling."""
    a = _adapter()
    a._room_watermarks["R1"] = ""
    fake = _FakeUrlopen([(429, {"error": "throttled"}, {"Retry-After": "7"})])
    monkeypatch.setattr(ra.urllib.request, "urlopen", fake)
    sleeps: list = []
    monkeypatch.setattr(ra.time, "sleep", lambda s: sleeps.append(s))
    with pytest.raises(RuntimeError, match="429"):
        a._poll_once(lambda _: None, ["R1"])
    assert sleeps == [7.0]


def test_post_message_429_sleeps_retry_after_then_raises(monkeypatch):
    """chat.postMessage is rate-limited independently of auth. A 429
    here must sleep and raise; `suppress_error_responses=True` keeps
    the raise from echoing as a public message."""
    a = _adapter()
    fake = _FakeUrlopen([(429, {"error": "throttled"}, {"Retry-After": "6"})])
    monkeypatch.setattr(ra.urllib.request, "urlopen", fake)
    sleeps: list = []
    monkeypatch.setattr(ra.time, "sleep", lambda s: sleeps.append(s))
    with pytest.raises(RuntimeError, match="429"):
        a._post_message("R1", "hi", tmid=None)
    assert sleeps == [6.0]
