"""Tests for librefang.sidecar.adapters.nextcloud.

Deterministic, no network: urllib is monkeypatched. Asserts the
sidecar Nextcloud Talk adapter preserves the behaviour of the removed
in-process Rust ``librefang-channels::nextcloud`` adapter, plus three
explicitly-acknowledged improvements:

* **P1**: ``thread_id`` is the inbound ``id`` (or inbound
  ``parentMessage.id`` when the user was already in a thread), so
  ``on_send`` rounds it back to ``replyTo`` on the chat POST — fixes
  the Rust adapter bug where chunked / threaded replies always
  landed at the room root because ``replyTo`` was never sent.
* **P2**: self-skip on ``(actorType, actorId) == ("users",
  own_user_id)`` rather than ``actorId`` alone — a Talk guest /
  federated_users actor whose id happens to equal the bot's user id
  no longer spoofs self-skip.
* **P3**: dedupe set on ``id`` (matches reddit / rocketchat), so
  boundary repeats of the same id on retry / re-poll no longer
  re-emit messages.
"""

import io
import json
import os
import urllib.error

import pytest

# Required env must be present at import time because the adapter
# raises SystemExit(2) on missing values.
os.environ.setdefault("NEXTCLOUD_SERVER_URL", "https://cloud.example.com")
os.environ.setdefault("NEXTCLOUD_TOKEN", "test-token")
from librefang.sidecar.adapters import nextcloud as nc  # noqa: E402


def _adapter(**env):
    defaults = {
        "NEXTCLOUD_SERVER_URL": "https://cloud.example.com",
        "NEXTCLOUD_TOKEN": "test-token",
        "NEXTCLOUD_ROOMS": "",
        "NEXTCLOUD_ACCOUNT_ID": "",
        "NEXTCLOUD_POLL_INTERVAL_SECS": "",
    }
    for k, v in defaults.items():
        os.environ[k] = env.get(k, v)
    return nc.NextcloudAdapter()


# ---- env handling -------------------------------------------------


def test_required_env_present():
    a = _adapter()
    assert a.server_url == "https://cloud.example.com"
    assert a.token == "test-token"
    assert a.allowed_rooms == []
    assert a.account_id is None
    assert a.poll_interval == nc.DEFAULT_POLL_INTERVAL_SECS


def test_server_url_trailing_slash_stripped():
    """Mirrors the Rust adapter's `trim_end_matches('/')`."""
    a = _adapter(NEXTCLOUD_SERVER_URL="https://cloud.example.com/")
    assert a.server_url == "https://cloud.example.com"
    a = _adapter(NEXTCLOUD_SERVER_URL="https://cloud.example.com///")
    assert a.server_url == "https://cloud.example.com"


def test_server_url_scheme_validated():
    """Refuse to start on a bare hostname — silent failure here would
    explode mid-poll with a confusing urllib error."""
    with pytest.raises(SystemExit) as exc:
        _adapter(NEXTCLOUD_SERVER_URL="cloud.example.com")
    assert exc.value.code == 2


def test_missing_required_env_exits():
    for var in ("NEXTCLOUD_SERVER_URL", "NEXTCLOUD_TOKEN"):
        with pytest.raises(SystemExit) as exc:
            _adapter(**{var: ""})
        assert exc.value.code == 2, var


def test_rooms_parsed_comma_separated():
    a = _adapter(NEXTCLOUD_ROOMS=" abc , def ,ghi")
    assert a.allowed_rooms == ["abc", "def", "ghi"]


def test_rooms_empty_means_discover():
    a = _adapter(NEXTCLOUD_ROOMS="")
    assert a.allowed_rooms == []


def test_account_id_optional():
    a = _adapter(NEXTCLOUD_ACCOUNT_ID="prod")
    assert a.account_id == "prod"
    a = _adapter(NEXTCLOUD_ACCOUNT_ID="")
    assert a.account_id is None


def test_poll_interval_env_override():
    a = _adapter(NEXTCLOUD_POLL_INTERVAL_SECS="10")
    assert a.poll_interval == 10


def test_poll_interval_below_floor_clamped():
    a = _adapter(NEXTCLOUD_POLL_INTERVAL_SECS="0")
    assert a.poll_interval == nc.MIN_POLL_INTERVAL_SECS


def test_poll_interval_invalid_exits():
    with pytest.raises(SystemExit) as exc:
        _adapter(NEXTCLOUD_POLL_INTERVAL_SECS="not-a-number")
    assert exc.value.code == 2


# ---- suppress + capabilities --------------------------------------


def test_suppress_error_responses_is_true_in_ready_event():
    """Nextcloud Talk rooms are typically multi-participant; never
    echo internal errors back as a chat message."""
    a = _adapter()
    assert a.suppress_error_responses is True
    p = a.ready_event()["params"]
    assert p.get("suppress_error_responses") is True


def test_capabilities_empty():
    a = _adapter()
    assert a.capabilities == []


def test_account_id_in_ready_event():
    a = _adapter(NEXTCLOUD_ACCOUNT_ID="acct-1")
    p = a.ready_event()["params"]
    assert p.get("account_id") == "acct-1"


# ---- _split_message ----------------------------------------------


def test_split_message_under_limit_one_chunk():
    assert nc._split_message("short", 100) == ["short"]


def test_split_message_prefers_newline_cut():
    body = "a" * 80 + "\n" + "b" * 80
    chunks = nc._split_message(body, 100)
    assert len(chunks) == 2
    assert chunks[0] == "a" * 80
    assert chunks[1] == "b" * 80


def test_split_message_hard_cut_when_no_newline():
    chunks = nc._split_message("x" * 250, 100)
    assert [len(c) for c in chunks] == [100, 100, 50]


def test_split_message_32000_cap_matches_rust():
    """The Rust adapter's MAX_MESSAGE_LEN is 32000."""
    assert nc.MAX_MESSAGE_LEN == 32000


# ---- _FakeUrlopen scaffolding --------------------------------------


class _HdrShim:
    """Mimic the parts of ``email.message.Message`` that ``_http``
    touches — only ``.items()``. urllib's real response headers are a
    Message; tests want a dict shape, so wrap it."""

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
    mid=1,
    text="hello nextcloud",
    actor_id="alice",
    actor_type="users",
    actor_display="Alice",
    parent_id=None,
    message_type=None,
    reference_id=None,
):
    out = {
        "id": mid,
        "message": text,
        "actorId": actor_id,
        "actorType": actor_type,
        "actorDisplayName": actor_display,
    }
    if parent_id is not None:
        out["parentMessage"] = {"id": parent_id}
    if message_type is not None:
        out["messageType"] = message_type
    if reference_id is not None:
        out["referenceId"] = reference_id
    return out


# ---- /cloud/user + OCS / Bearer headers ----------------------------


def test_verify_credentials_sends_ocs_and_bearer_headers(monkeypatch):
    a = _adapter()
    fake = _FakeUrlopen([(200, {"ocs": {"data": {"id": "librefang-bot"}}})])
    monkeypatch.setattr(nc.urllib.request, "urlopen", fake)
    uid = a._verify_credentials()
    assert uid == "librefang-bot"
    assert a.own_user_id == "librefang-bot"
    call = fake.calls[0]
    assert call["url"] == "https://cloud.example.com/ocs/v2.php/cloud/user?format=json"
    assert call["headers"]["authorization"] == "Bearer test-token"
    assert call["headers"]["ocs-apirequest"] == "true"
    assert call["headers"]["accept"] == "application/json"


def test_verify_credentials_raises_on_401(monkeypatch):
    a = _adapter()
    fake = _FakeUrlopen([(401, {"status": "error"})])
    monkeypatch.setattr(nc.urllib.request, "urlopen", fake)
    with pytest.raises(RuntimeError, match="authentication failed 401"):
        a._verify_credentials()


def test_verify_credentials_accepts_missing_id(monkeypatch):
    """If `/cloud/user` 200s but omits `data.id`, the adapter must
    keep running — self-skip falls back to a no-op (every message
    routes to the bot)."""
    a = _adapter()
    fake = _FakeUrlopen([(200, {"ocs": {"data": {}}})])
    monkeypatch.setattr(nc.urllib.request, "urlopen", fake)
    uid = a._verify_credentials()
    assert uid == "unknown"
    assert a.own_user_id == ""


# ---- spreed room list -----------------------------------------------


def test_list_joined_rooms_parses_tokens(monkeypatch):
    a = _adapter()
    fake = _FakeUrlopen([(200, {"ocs": {"data": [
        {"token": "room1", "name": "general"},
        {"token": "room2", "name": "random"},
        {"name": "no-token"},  # skipped
    ]}})])
    monkeypatch.setattr(nc.urllib.request, "urlopen", fake)
    ids = a._list_joined_rooms()
    assert ids == ["room1", "room2"]
    assert "/ocs/v2.php/apps/spreed/api/v4/room" in fake.calls[0]["url"]
    # Required OCS header present on discovery too.
    assert fake.calls[0]["headers"]["ocs-apirequest"] == "true"


def test_list_joined_rooms_returns_empty_on_error(monkeypatch):
    a = _adapter()
    fake = _FakeUrlopen([(500, {"error": "boom"})])
    monkeypatch.setattr(nc.urllib.request, "urlopen", fake)
    assert a._list_joined_rooms() == []


def test_list_joined_rooms_handles_transport_error(monkeypatch):
    a = _adapter()

    def boom(req, timeout=None):
        raise urllib.error.URLError("dns")

    monkeypatch.setattr(nc.urllib.request, "urlopen", boom)
    assert a._list_joined_rooms() == []


# ---- _parse_message ----------------------------------------------


def test_parse_basic_text():
    a = _adapter()
    a.own_user_id = "librefang-bot"
    ev = a._parse_message(_msg(), "ROOM1")
    assert ev is not None
    p = ev["params"]
    assert p["content"] == {"Text": "hello nextcloud"}
    assert p["message_id"] == "1"
    assert p["channel_id"] == "ROOM1"
    assert p["user_id"] == "ROOM1"  # platform routing key
    assert p["user_name"] == "Alice"
    assert p["is_group"] is True
    # P1: thread_id is the inbound id (top-level), not the room id.
    assert p["thread_id"] == "1"
    md = p["metadata"]
    assert md["actor_id"] == "alice"
    assert md["actor_type"] == "users"
    assert md["actor_display_name"] == "Alice"
    assert md["room_token"] == "ROOM1"
    assert "parent_message_id" not in md


def test_parse_thread_reply_uses_parent_id_as_thread_id():
    """P1: when the inbound message is itself a thread reply, the
    outbound thread_id MUST be the existing parent message id —
    otherwise the bot would start a child thread under the user's
    reply instead of threading alongside them."""
    a = _adapter()
    a.own_user_id = "librefang-bot"
    ev = a._parse_message(
        _msg(mid=42, parent_id=7, text="reply"),
        "ROOM1",
    )
    p = ev["params"]
    assert p["thread_id"] == "7"
    assert p["metadata"]["parent_message_id"] == "7"


def test_parse_thread_reply_accepts_string_parent_id():
    """Talk sometimes serialises `parentMessage.id` as a string;
    accept both shapes."""
    a = _adapter()
    a.own_user_id = "librefang-bot"
    msg = _msg(mid=42)
    msg["parentMessage"] = {"id": "STR_PARENT"}
    ev = a._parse_message(msg, "ROOM1")
    assert ev["params"]["thread_id"] == "STR_PARENT"


def test_parse_skips_self_by_actor_id():
    """P2: self-skip when (actorType, actorId) == ("users",
    own_user_id)."""
    a = _adapter()
    a.own_user_id = "librefang-bot"
    skip = a._parse_message(
        _msg(actor_id="librefang-bot", actor_type="users"),
        "ROOM1",
    )
    assert skip is None


def test_parse_guest_with_matching_id_is_not_self():
    """P2: a guest whose `actorId` happens to equal the bot's user id
    must NOT trigger self-skip — the bot would silently ignore the
    guest's messages otherwise."""
    a = _adapter()
    a.own_user_id = "librefang-bot"
    ev = a._parse_message(
        _msg(actor_id="librefang-bot", actor_type="guests"),
        "ROOM1",
    )
    assert ev is not None
    assert ev["params"]["metadata"]["actor_type"] == "guests"


def test_parse_self_skip_disabled_when_own_user_id_empty():
    """If `/cloud/user` returned no id (own_user_id == ""), the
    self-skip check must short-circuit to False so the bot doesn't
    silently swallow every message whose actorId happens to be ""."""
    a = _adapter()
    a.own_user_id = ""
    msg = _msg(actor_id="", actor_type="users")
    ev = a._parse_message(msg, "ROOM1")
    assert ev is not None


def test_parse_skips_system_messages():
    """Talk `messageType=system` (join/leave/etc.) must be filtered.
    Matches the Rust adapter at nextcloud.rs:331-334."""
    a = _adapter()
    a.own_user_id = "librefang-bot"
    skip = a._parse_message(
        _msg(message_type="system"),
        "ROOM1",
    )
    assert skip is None


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


def test_parse_reference_id_surfaced_in_metadata():
    a = _adapter()
    ev = a._parse_message(_msg(reference_id="ref-abc"), "ROOM1")
    assert ev["params"]["metadata"]["reference_id"] == "ref-abc"


def test_parse_malformed_inputs():
    a = _adapter()
    assert a._parse_message("not a dict", "ROOM1") is None
    # Missing id / message → falsy text → skip.
    assert a._parse_message({"actorId": "x"}, "ROOM1") is None


def test_parse_handles_non_integer_id_gracefully():
    """If Talk ever returns a non-integer `id`, we treat it as 0 and
    let it pass through unwarmarked — losing the dedupe boost but
    never crashing."""
    a = _adapter()
    a.own_user_id = ""
    msg = {
        "id": "not-a-number",
        "message": "hi",
        "actorId": "alice",
        "actorType": "users",
        "actorDisplayName": "Alice",
    }
    ev = a._parse_message(msg, "ROOM1")
    assert ev is not None
    # With a falsy parsed id, no message_id is surfaced and no
    # thread_id is built.
    assert ev["params"].get("message_id") is None
    assert ev["params"].get("thread_id") is None


# ---- _poll_once: emit + dedupe + watermark ------------------------


def test_poll_once_emits_messages_and_advances_watermark(monkeypatch):
    a = _adapter()
    a.own_user_id = "librefang-bot"
    a._room_watermarks["R1"] = 0
    fake = _FakeUrlopen([
        (200, {"ocs": {"data": [
            _msg(mid=10),
            _msg(mid=11, text="/cmd args"),
        ]}}),
    ])
    monkeypatch.setattr(nc.urllib.request, "urlopen", fake)
    emitted = []
    a._poll_once(emitted.append, ["R1"])
    assert len(emitted) == 2
    assert emitted[0]["params"]["message_id"] == "10"
    assert emitted[1]["params"]["content"] == {
        "Command": {"name": "cmd", "args": ["args"]},
    }
    # Watermark advanced to the newest id seen.
    assert a._room_watermarks["R1"] == 11
    # Both message ids tracked for dedupe.
    assert 10 in a._seen_messages_set
    assert 11 in a._seen_messages_set


def test_poll_once_dedupes_id_repeats(monkeypatch):
    """P3: a message id that appears in two consecutive poll responses
    (server-side retry boundary; lookIntoFuture can re-include a
    previously-seen id when watermark sync slips) must not re-emit.
    The Rust adapter only relied on the server-side lastKnownMessageId
    cursor; the sidecar additionally dedupes on id."""
    a = _adapter()
    a.own_user_id = "librefang-bot"
    a._room_watermarks["R1"] = 0
    fake = _FakeUrlopen([
        (200, {"ocs": {"data": [_msg(mid=10), _msg(mid=11)]}}),
        (200, {"ocs": {"data": [_msg(mid=10), _msg(mid=11), _msg(mid=12)]}}),
    ])
    monkeypatch.setattr(nc.urllib.request, "urlopen", fake)
    emitted = []
    a._poll_once(emitted.append, ["R1"])
    assert [e["params"]["message_id"] for e in emitted] == ["10", "11"]
    emitted.clear()
    a._poll_once(emitted.append, ["R1"])
    # Only the new id 12 emits on the second poll.
    assert [e["params"]["message_id"] for e in emitted] == ["12"]


def test_poll_once_self_skipped(monkeypatch):
    a = _adapter()
    a.own_user_id = "librefang-bot"
    a._room_watermarks["R1"] = 0
    fake = _FakeUrlopen([
        (200, {"ocs": {"data": [
            _msg(mid=20, actor_id="librefang-bot", actor_type="users"),
            _msg(mid=21, actor_id="USER_A"),
        ]}}),
    ])
    monkeypatch.setattr(nc.urllib.request, "urlopen", fake)
    emitted = []
    a._poll_once(emitted.append, ["R1"])
    assert [e["params"]["message_id"] for e in emitted] == ["21"]
    # Even self-skipped msg id 20 is still marked seen so we don't
    # reparse it.
    assert 20 in a._seen_messages_set


def test_poll_once_injects_account_id(monkeypatch):
    a = _adapter(NEXTCLOUD_ACCOUNT_ID="prod")
    a.own_user_id = "librefang-bot"
    a._room_watermarks["R1"] = 0
    fake = _FakeUrlopen([
        (200, {"ocs": {"data": [_msg(mid=30)]}}),
    ])
    monkeypatch.setattr(nc.urllib.request, "urlopen", fake)
    emitted = []
    a._poll_once(emitted.append, ["R1"])
    assert emitted[0]["params"]["metadata"]["account_id"] == "prod"


def test_poll_once_401_raises(monkeypatch):
    a = _adapter()
    fake = _FakeUrlopen([(401, {"status": "error"})])
    monkeypatch.setattr(nc.urllib.request, "urlopen", fake)
    a._room_watermarks["R1"] = 0
    with pytest.raises(RuntimeError, match="401"):
        a._poll_once(lambda _: None, ["R1"])


def test_retry_after_secs_parses_header_value():
    """``Retry-After`` (seconds form) is parsed as a float and capped
    at ``MAX_BACKOFF_SECS`` so a misreported value can't block the
    poller for more than a minute."""
    assert nc.NextcloudAdapter._retry_after_secs({"retry-after": "5"}) == 5.0
    assert nc.NextcloudAdapter._retry_after_secs({"retry-after": "0.5"}) == 1.0
    assert (
        nc.NextcloudAdapter._retry_after_secs({"retry-after": "9999"})
        == nc.MAX_BACKOFF_SECS
    )


def test_retry_after_secs_falls_back_when_absent_or_invalid():
    """Without a ``Retry-After`` (or with a garbled value), fall back
    to ``RETRY_AFTER_DEFAULT_SECS`` rather than busy-looping at
    1 second."""
    assert (
        nc.NextcloudAdapter._retry_after_secs({})
        == nc.RETRY_AFTER_DEFAULT_SECS
    )
    assert (
        nc.NextcloudAdapter._retry_after_secs({"retry-after": "Thu, 01 Jan 2099 00:00:00 GMT"})
        == nc.RETRY_AFTER_DEFAULT_SECS
    )


def test_poll_once_429_sleeps_retry_after_then_raises(monkeypatch):
    """OCS bruteforce throttling returns 429 with a `Retry-After`
    header. The poll loop must sleep the indicated interval *then*
    raise so the producer loop's outer backoff doesn't immediately
    issue another probe inside the throttling window."""
    a = _adapter()
    a._room_watermarks["R1"] = 0
    fake = _FakeUrlopen([
        (429, {"message": "Throttled"}, {"Retry-After": "7"}),
    ])
    monkeypatch.setattr(nc.urllib.request, "urlopen", fake)
    sleeps: list = []
    monkeypatch.setattr(nc.time, "sleep", lambda s: sleeps.append(s))
    with pytest.raises(RuntimeError, match="429"):
        a._poll_once(lambda _: None, ["R1"])
    assert sleeps == [7.0], (
        "must honour Retry-After before raising — measured sleeps="
        f"{sleeps}"
    )


def test_poll_once_429_without_header_uses_default(monkeypatch):
    """Talk's bruteforce throttler usually sends ``Retry-After``, but
    spec doesn't require it. A 429 with no header must fall back to
    ``RETRY_AFTER_DEFAULT_SECS`` instead of looping at 1 s."""
    a = _adapter()
    a._room_watermarks["R1"] = 0
    fake = _FakeUrlopen([(429, {"message": "Throttled"})])
    monkeypatch.setattr(nc.urllib.request, "urlopen", fake)
    sleeps: list = []
    monkeypatch.setattr(nc.time, "sleep", lambda s: sleeps.append(s))
    with pytest.raises(RuntimeError, match="429"):
        a._poll_once(lambda _: None, ["R1"])
    assert sleeps == [nc.RETRY_AFTER_DEFAULT_SECS]


def test_verify_credentials_429_sleeps_retry_after_then_raises(monkeypatch):
    """OCS bruteforce throttling targets failed auth most aggressively,
    so the credential probe is the most likely place to see 429. The
    sidecar must honour ``Retry-After`` here too — otherwise the
    producer loop's auth-retry backoff would compound with the
    server-side block and extend the ban."""
    a = _adapter()
    fake = _FakeUrlopen([
        (429, {"message": "Throttled"}, {"Retry-After": "3"}),
    ])
    monkeypatch.setattr(nc.urllib.request, "urlopen", fake)
    sleeps: list = []
    monkeypatch.setattr(nc.time, "sleep", lambda s: sleeps.append(s))
    with pytest.raises(RuntimeError, match="429"):
        a._verify_credentials()
    assert sleeps == [3.0]


def test_list_joined_rooms_429_sleeps_then_returns_empty(monkeypatch):
    """Room discovery is one-shot; the producer just retries on the
    next pass, so a 429 here only needs to sleep — surfacing it as an
    empty-list signal (same as transport error) is enough."""
    a = _adapter()
    fake = _FakeUrlopen([
        (429, {"message": "Throttled"}, {"Retry-After": "4"}),
    ])
    monkeypatch.setattr(nc.urllib.request, "urlopen", fake)
    sleeps: list = []
    monkeypatch.setattr(nc.time, "sleep", lambda s: sleeps.append(s))
    out = a._list_joined_rooms()
    assert out == []
    assert sleeps == [4.0]


def test_post_message_429_sleeps_retry_after_then_raises(monkeypatch):
    """Talk rate-limits chat posting separately from auth; a 429 on
    POST /chat must honour ``Retry-After`` and raise (caller is
    `on_send`; `suppress_error_responses=True` prevents the raise from
    echoing back into the room)."""
    a = _adapter()
    fake = _FakeUrlopen([
        (429, {"message": "Throttled"}, {"Retry-After": "6"}),
    ])
    monkeypatch.setattr(nc.urllib.request, "urlopen", fake)
    sleeps: list = []
    monkeypatch.setattr(nc.time, "sleep", lambda s: sleeps.append(s))
    with pytest.raises(RuntimeError, match="429"):
        a._post_message("R1", "hi", reply_to=None)
    assert sleeps == [6.0]


def test_poll_once_304_treated_as_no_op(monkeypatch):
    """Talk returns 304 Not Modified when the long-poll window
    expires with no new messages. The Rust adapter handled this at
    nextcloud.rs:297-300; the sidecar must do the same instead of
    raising or surfacing it as a fault."""
    a = _adapter()
    a._room_watermarks["R1"] = 5
    fake = _FakeUrlopen([(304, None)])
    monkeypatch.setattr(nc.urllib.request, "urlopen", fake)
    emitted = []
    a._poll_once(emitted.append, ["R1"])  # must not raise
    assert emitted == []
    # Watermark unchanged.
    assert a._room_watermarks["R1"] == 5


def test_poll_once_skips_room_on_transport_error(monkeypatch):
    """One bad room doesn't take the loop down; the next room's fetch
    still runs in the same poll pass."""
    a = _adapter()
    a.own_user_id = "librefang-bot"
    a._room_watermarks["R1"] = 0
    a._room_watermarks["R2"] = 0
    calls = []

    def fake_urlopen(req, timeout=None):
        url = req.full_url
        calls.append(url)
        if "/chat/R1" in url:
            raise urllib.error.URLError("dns error")
        if "/chat/R2" in url:
            return _FakeResp(
                200,
                json.dumps({"ocs": {"data": [_msg(mid=40)]}}).encode("utf-8"),
            )
        raise AssertionError(f"unexpected {url}")

    monkeypatch.setattr(nc.urllib.request, "urlopen", fake_urlopen)
    emitted = []
    a._poll_once(emitted.append, ["R1", "R2"])
    assert len(emitted) == 1
    assert emitted[0]["params"]["channel_id"] == "R2"
    assert any("/chat/R1" in c for c in calls)
    assert any("/chat/R2" in c for c in calls)


def test_poll_once_non_200_logged_and_skipped(monkeypatch):
    """A 500 on one poll surfaces a warning but does not raise — the
    caller's exponential backoff would otherwise turn a transient
    upstream blip into a multi-minute pause."""
    a = _adapter()
    a._room_watermarks["R1"] = 0
    fake = _FakeUrlopen([(500, {"error": "transient"})])
    monkeypatch.setattr(nc.urllib.request, "urlopen", fake)
    emitted = []
    a._poll_once(emitted.append, ["R1"])  # must not raise
    assert emitted == []


def test_poll_once_request_shape(monkeypatch):
    """Asserts URL + auth headers for a single-room poll. Covers
    `lookIntoFuture=1`, `limit=100`, `lastKnownMessageId=<wm>`,
    `format=json` — the API contract every operator's reverse proxy
    will see, and the same params the Rust adapter sent
    (nextcloud.rs:273-276)."""
    a = _adapter()
    a._room_watermarks["R1"] = 42
    fake = _FakeUrlopen([(200, {"ocs": {"data": []}})])
    monkeypatch.setattr(nc.urllib.request, "urlopen", fake)
    a._poll_once(lambda _: None, ["R1"])
    call = fake.calls[0]
    assert call["url"].startswith(
        "https://cloud.example.com/ocs/v2.php/apps/spreed/api/v1/chat/R1?"
    )
    assert "lookIntoFuture=1" in call["url"]
    assert "lastKnownMessageId=42" in call["url"]
    assert "limit=100" in call["url"]
    assert "format=json" in call["url"]
    assert call["headers"]["authorization"] == "Bearer test-token"
    assert call["headers"]["ocs-apirequest"] == "true"


# ---- dedupe set capacity ------------------------------------------


def test_seen_messages_capacity_eviction():
    a = _adapter()
    for i in range(1, nc.SEEN_MESSAGES_MAX + 2):
        a._mark_seen(i)
    # ids 1..SEEN_MESSAGES_EVICT should have been evicted on overflow.
    assert 1 not in a._seen_messages_set
    assert nc.SEEN_MESSAGES_EVICT not in a._seen_messages_set
    assert nc.SEEN_MESSAGES_EVICT + 1 in a._seen_messages_set
    assert nc.SEEN_MESSAGES_MAX + 1 in a._seen_messages_set
    assert len(a._seen_messages) == len(a._seen_messages_set)


def test_seen_messages_idempotent_mark():
    a = _adapter()
    a._mark_seen(5)
    a._mark_seen(5)
    assert a._seen_messages.count(5) == 1


def test_seen_messages_empty_id_ignored():
    a = _adapter()
    a._mark_seen(0)
    assert a._seen_messages == []


# ---- _post_message: outbound + threading --------------------------


def test_post_message_basic_shape(monkeypatch):
    a = _adapter()
    fake = _FakeUrlopen([(201, {"ocs": {"meta": {"status": "ok"}, "data": {}}})])
    monkeypatch.setattr(nc.urllib.request, "urlopen", fake)
    a._post_message("ROOM1", "hello from librefang", None)
    call = fake.calls[0]
    assert call["url"] == (
        "https://cloud.example.com/ocs/v2.php/apps/spreed/api/v1/chat/ROOM1"
    )
    assert call["method"] == "POST"
    assert call["headers"]["authorization"] == "Bearer test-token"
    assert call["headers"]["ocs-apirequest"] == "true"
    assert call["headers"]["content-type"] == "application/x-www-form-urlencoded"
    # Form-encoded body — Talk's `replyTo` is a form parameter not
    # JSON. Order doesn't matter; only key set + value.
    parsed = dict(urllib.parse.parse_qsl(call["body_raw"]))
    assert parsed == {"message": "hello from librefang"}


def test_post_message_with_thread_includes_reply_to(monkeypatch):
    """P1: when on_send forwards thread_id, the outbound payload MUST
    include `replyTo` so Talk threads the reply under the parent
    message id."""
    a = _adapter()
    fake = _FakeUrlopen([(201, {"ocs": {"meta": {"status": "ok"}, "data": {}}})])
    monkeypatch.setattr(nc.urllib.request, "urlopen", fake)
    a._post_message("ROOM1", "threaded reply", "42")
    parsed = dict(urllib.parse.parse_qsl(fake.calls[0]["body_raw"]))
    assert parsed == {"message": "threaded reply", "replyTo": "42"}


def test_post_message_chunks_long_text(monkeypatch):
    """Long messages chunk at MAX_MESSAGE_LEN and post as separate
    messages. Matches the Rust per-chunk loop."""
    a = _adapter()
    fake = _FakeUrlopen([
        (201, {"ocs": {"meta": {"status": "ok"}, "data": {}}}),
        (201, {"ocs": {"meta": {"status": "ok"}, "data": {}}}),
        (201, {"ocs": {"meta": {"status": "ok"}, "data": {}}}),
    ])
    monkeypatch.setattr(nc.urllib.request, "urlopen", fake)
    long_text = "x" * (nc.MAX_MESSAGE_LEN * 2 + 100)
    a._post_message("ROOM1", long_text, None)
    assert len(fake.calls) == 3
    total = sum(
        len(dict(urllib.parse.parse_qsl(c["body_raw"]))["message"])
        for c in fake.calls
    )
    assert total == len(long_text)


def test_post_message_chunks_preserve_reply_to(monkeypatch):
    """When the outbound is threaded AND chunked, every chunk must
    carry the same `replyTo` so the whole multi-part reply lives in
    the same thread."""
    a = _adapter()
    fake = _FakeUrlopen([
        (201, {"ocs": {"meta": {"status": "ok"}, "data": {}}}),
        (201, {"ocs": {"meta": {"status": "ok"}, "data": {}}}),
    ])
    monkeypatch.setattr(nc.urllib.request, "urlopen", fake)
    long_text = "x" * (nc.MAX_MESSAGE_LEN + 10)
    a._post_message("ROOM1", long_text, "T1")
    for c in fake.calls:
        parsed = dict(urllib.parse.parse_qsl(c["body_raw"]))
        assert parsed["replyTo"] == "T1"


def test_post_message_missing_room_token_raises():
    a = _adapter()
    with pytest.raises(RuntimeError, match="missing room token"):
        a._post_message("", "hi", None)


def test_post_message_non_2xx_surfaces(monkeypatch):
    a = _adapter()
    fake = _FakeUrlopen([(401, {"error": "Unauthorized"})])
    monkeypatch.setattr(nc.urllib.request, "urlopen", fake)
    with pytest.raises(RuntimeError, match="401"):
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
    """`cmd.user.platform_id` carries the room token from inbound
    (matches the Rust ChannelUser{platform_id: room_token} shape)."""
    a = _adapter()
    fake = _FakeUrlopen([(201, {"ocs": {"meta": {"status": "ok"}, "data": {}}})])
    monkeypatch.setattr(nc.urllib.request, "urlopen", fake)
    import asyncio as _asyncio
    _asyncio.run(a.on_send(_StubCmd(
        text="hi",
        user={"platform_id": "ROOM1"},
    )))
    assert "/chat/ROOM1" in fake.calls[0]["url"]
    parsed = dict(urllib.parse.parse_qsl(fake.calls[0]["body_raw"]))
    assert parsed == {"message": "hi"}


def test_on_send_threads_via_thread_id(monkeypatch):
    """P1 round-trip: cmd.thread_id → replyTo on the outbound."""
    a = _adapter()
    fake = _FakeUrlopen([(201, {"ocs": {"meta": {"status": "ok"}, "data": {}}})])
    monkeypatch.setattr(nc.urllib.request, "urlopen", fake)
    import asyncio as _asyncio
    _asyncio.run(a.on_send(_StubCmd(
        text="threaded reply",
        thread_id="42",
        user={"platform_id": "ROOM1"},
    )))
    parsed = dict(urllib.parse.parse_qsl(fake.calls[0]["body_raw"]))
    assert parsed == {"message": "threaded reply", "replyTo": "42"}


def test_on_send_falls_back_to_channel_id(monkeypatch):
    """If `user.platform_id` is empty (pre-#5219 daemon stripping
    `user`), fall back to `cmd.channel_id` so the bot still routes."""
    a = _adapter()
    fake = _FakeUrlopen([(201, {"ocs": {"meta": {"status": "ok"}, "data": {}}})])
    monkeypatch.setattr(nc.urllib.request, "urlopen", fake)
    import asyncio as _asyncio
    _asyncio.run(a.on_send(_StubCmd(
        text="fallback",
        channel_id="ROOM2",
    )))
    assert "/chat/ROOM2" in fake.calls[0]["url"]


def test_on_send_non_text_content_falls_back_to_placeholder(monkeypatch):
    a = _adapter()
    fake = _FakeUrlopen([(201, {"ocs": {"meta": {"status": "ok"}, "data": {}}})])
    monkeypatch.setattr(nc.urllib.request, "urlopen", fake)
    import asyncio as _asyncio
    _asyncio.run(a.on_send(_StubCmd(
        content={"Reaction": {"emoji": "👍"}},
        user={"platform_id": "ROOM1"},
    )))
    parsed = dict(urllib.parse.parse_qsl(fake.calls[0]["body_raw"]))
    assert "Unsupported content type" in parsed["message"]
