"""Tests for librefang.sidecar.adapters.nextcloud.

Deterministic, no network: urllib is monkeypatched. Asserts the
sidecar Nextcloud Talk adapter preserves the behaviour of the removed
in-process Rust ``librefang-channels::nextcloud`` adapter, plus four
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
* **P4**: user-oriented identity — ``user_id`` is the sender's
  ``actorId`` (not the room token), ``is_group`` comes from the
  room's Talk type (1 = 1-to-1 → DM), and a startup actor→DM-room
  map (built from the participants endpoint) lets ``channel_dm``
  resolve members who have never DMed the bot, while group messages
  carry a ``group_members`` roster for ``channel_members``.
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

from _sidecar_fakes import _FakeResp, _FakeUrlopen, _HdrShim


def _adapter(**env):
    defaults = {
        "NEXTCLOUD_SERVER_URL": "https://cloud.example.com",
        "NEXTCLOUD_TOKEN": "test-token",
        "NEXTCLOUD_ROOMS": "",
        "NEXTCLOUD_ACCOUNT_ID": "",
        "NEXTCLOUD_POLL_INTERVAL_SECS": "",
        "NEXTCLOUD_MEDIA_MAX_BYTES": "",
        "NEXTCLOUD_MEDIA_MIME_ALLOWLIST": "",
        "NEXTCLOUD_MEDIA_UPLOAD_DIR": "",
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
    rooms = a._list_joined_rooms()
    assert [r["token"] for r in rooms] == ["room1", "room2"]
    assert rooms[0]["name"] == "general"
    assert "/ocs/v2.php/apps/spreed/api/v4/room" in fake.calls[0]["url"]
    # Required OCS header present on discovery too.
    assert fake.calls[0]["headers"]["ocs-apirequest"] == "true"


def test_list_joined_rooms_raises_on_server_error(monkeypatch):
    a = _adapter()
    fake = _FakeUrlopen([(500, {"error": "boom"})])
    monkeypatch.setattr(nc.urllib.request, "urlopen", fake)
    with pytest.raises(nc._RoomDiscoveryError, match="status 500"):
        a._list_joined_rooms()


def test_list_joined_rooms_raises_on_transport_error(monkeypatch):
    a = _adapter()

    def boom(req, timeout=None):
        raise urllib.error.URLError("dns")

    monkeypatch.setattr(nc.urllib.request, "urlopen", boom)
    with pytest.raises(nc._RoomDiscoveryError, match="transport error"):
        a._list_joined_rooms()


def test_discovery_rechecks_after_genuine_empty_room_list(monkeypatch):
    a = _adapter()
    results = iter([[], ["room1"]])
    sleeps: list[float] = []
    monkeypatch.setattr(a, "_list_joined_rooms", lambda: next(results))
    monkeypatch.setattr(nc.time, "sleep", sleeps.append)

    assert a._discover_rooms_until_available() == ["room1"]
    assert sleeps == [nc.MAX_BACKOFF_SECS]


def test_discovery_retries_transient_error_with_backoff(monkeypatch):
    a = _adapter()
    calls = 0
    sleeps: list[float] = []

    def discover():
        nonlocal calls
        calls += 1
        if calls == 1:
            raise nc._RoomDiscoveryError("network unavailable")
        return ["room1"]

    monkeypatch.setattr(a, "_list_joined_rooms", discover)
    monkeypatch.setattr(nc.time, "sleep", sleeps.append)
    assert a._discover_rooms_until_available() == ["room1"]
    assert sleeps == [1.0]


def test_producer_uses_recovering_discovery_before_polling(monkeypatch):
    a = _adapter()
    polled: list[list[str]] = []

    class _StopProducer(Exception):
        pass

    monkeypatch.setattr(a, "_verify_credentials", lambda: "bot")
    monkeypatch.setattr(
        a, "_discover_rooms_until_available",
        lambda: [{"token": "room1", "type": 2}],
    )
    monkeypatch.setattr(
        a, "_fetch_participants", lambda _token: [],
    )
    monkeypatch.setattr(
        a, "_poll_once", lambda _emit, rooms: polled.append(list(rooms)),
    )
    monkeypatch.setattr(
        nc.time, "sleep", lambda _delay: (_ for _ in ()).throw(_StopProducer()),
    )

    with pytest.raises(_StopProducer):
        a._producer_blocking(lambda _event: None)
    assert polled == [["room1"]]


# ---- _parse_message ----------------------------------------------


def test_parse_basic_text():
    a = _adapter()
    a.own_user_id = "librefang-bot"
    ev = a._parse_message(_msg(), "ROOM1", room_type=2)
    assert ev is not None
    p = ev["params"]
    assert p["content"] == {"Text": "hello nextcloud"}
    assert p["message_id"] == "1"
    assert p["channel_id"] == "ROOM1"
    assert p["user_id"] == "alice"  # user-oriented identity (actorId)
    assert p["user_name"] == "Alice"
    assert p["is_group"] is True  # room_type=2 → group
    # P1: thread_id is the inbound id (top-level), not the room id.
    assert p["thread_id"] == "1"
    md = p["metadata"]
    assert md["actor_id"] == "alice"
    assert md["actor_type"] == "users"
    assert md["actor_display_name"] == "Alice"
    assert md["room_token"] == "ROOM1"
    assert md["room_type"] == 2
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
    assert 10 in a._seen.ids
    assert 11 in a._seen.ids


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
    assert 20 in a._seen.ids


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


def test_list_joined_rooms_429_surfaces_retry_after(monkeypatch):
    a = _adapter()
    fake = _FakeUrlopen([
        (429, {"message": "Throttled"}, {"Retry-After": "4"}),
    ])
    monkeypatch.setattr(nc.urllib.request, "urlopen", fake)
    with pytest.raises(nc._RoomDiscoveryError) as error:
        a._list_joined_rooms()
    assert error.value.retry_after == 4.0


def test_discovery_honours_429_retry_after(monkeypatch):
    a = _adapter()
    results = iter([
        nc._RoomDiscoveryError("throttled", retry_after=4.0),
        ["room1"],
    ])
    sleeps: list[float] = []

    def discover():
        result = next(results)
        if isinstance(result, Exception):
            raise result
        return result

    monkeypatch.setattr(a, "_list_joined_rooms", discover)
    monkeypatch.setattr(nc.time, "sleep", sleeps.append)
    assert a._discover_rooms_until_available() == ["room1"]
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


def test_poll_once_read_timeout_treated_as_empty_poll(monkeypatch, tmp_path):
    """An idle long-poll read timeout is expected steady-state, not a
    transport error: Talk holds `lookIntoFuture=1` connections for its
    whole long-poll window, so quiet rooms time out at the socket
    read on every pass (`socket.timeout` == builtin `TimeoutError`
    since Python 3.10, and not a `URLError` subclass). Must be
    treated like the 304 no-op: no warn, no raise, and the next room
    still polls in the same pass."""
    monkeypatch.setenv("LIBREFANG_HOME", str(tmp_path))
    a = _adapter()
    a.own_user_id = "librefang-bot"
    a._room_watermarks["R1"] = 7
    a._room_watermarks["R2"] = 0
    warns = []
    monkeypatch.setattr(
        nc.log, "warn", lambda *args, **kw: warns.append((args, kw)),
    )

    def fake_urlopen(req, timeout=None):
        if "/chat/R1" in req.full_url:
            raise TimeoutError("The read operation timed out")
        if "/chat/R2" in req.full_url:
            return _FakeResp(
                200,
                json.dumps({"ocs": {"data": [_msg(mid=50)]}}).encode("utf-8"),
            )
        raise AssertionError(f"unexpected {req.full_url}")

    monkeypatch.setattr(nc.urllib.request, "urlopen", fake_urlopen)
    emitted = []
    a._poll_once(emitted.append, ["R1", "R2"])  # must not raise
    assert [e["params"]["channel_id"] for e in emitted] == ["R2"]
    assert warns == []
    # Timed-out room's watermark untouched.
    assert a._room_watermarks["R1"] == 7


def test_poll_once_transport_error_still_warns(monkeypatch):
    """Real transport errors keep the warn + skip behaviour — the
    read-timeout fix must not swallow them."""
    a = _adapter()
    a._room_watermarks["R1"] = 0
    warns = []
    monkeypatch.setattr(
        nc.log, "warn", lambda *args, **kw: warns.append((args, kw)),
    )

    def fake_urlopen(req, timeout=None):
        raise urllib.error.URLError("dns error")

    monkeypatch.setattr(nc.urllib.request, "urlopen", fake_urlopen)
    emitted = []
    a._poll_once(emitted.append, ["R1"])  # must not raise
    assert emitted == []
    assert len(warns) == 1
    assert warns[0][0][0] == "nextcloud chat poll transport error"
    assert warns[0][1]["room_token"] == "R1"


def test_poll_once_poll_call_uses_raised_timeout(monkeypatch):
    """The long-poll call passes an explicit timeout above the server
    hold (POLL_TIMEOUT_SECS) so a slow server can't wedge the loop;
    every other _http call keeps the SEND_TIMEOUT_SECS default."""
    a = _adapter()
    a._room_watermarks["R1"] = 0
    fake = _FakeUrlopen([(200, {"ocs": {"data": []}})])
    monkeypatch.setattr(nc.urllib.request, "urlopen", fake)
    a._poll_once(lambda _: None, ["R1"])
    assert fake.calls[0]["timeout"] == nc.POLL_TIMEOUT_SECS
    assert nc.POLL_TIMEOUT_SECS > nc.SEND_TIMEOUT_SECS


def test_poll_once_logs_picked_up_message(monkeypatch):
    """Every emitted message logs an info pickup line (room token,
    message id, sender) — during debugging, silence was
    indistinguishable from failure. Self-skipped, deduped and system
    messages log nothing."""
    a = _adapter()
    a.own_user_id = "librefang-bot"
    a._room_watermarks["R1"] = 0
    infos = []
    monkeypatch.setattr(
        nc.log, "info", lambda *args, **kw: infos.append((args, kw)),
    )
    fake = _FakeUrlopen([
        (200, {"ocs": {"data": [
            _msg(mid=60, actor_id="alice"),
            _msg(mid=61, actor_id="librefang-bot", actor_type="users"),
            _msg(mid=62, text="hi", message_type="system"),
        ]}}),
        (200, {"ocs": {"data": [_msg(mid=60, actor_id="alice")]}}),
    ])
    monkeypatch.setattr(nc.urllib.request, "urlopen", fake)
    emitted = []
    a._poll_once(emitted.append, ["R1"])
    a._poll_once(emitted.append, ["R1"])  # dedupe repeat: no log
    pickups = [
        i for i in infos if i[0][0] == "nextcloud message picked up"
    ]
    assert len(pickups) == 1
    assert pickups[0][1] == {
        "room_token": "R1", "message_id": 60, "sender": "alice",
    }
    assert [e["params"]["message_id"] for e in emitted] == ["60"]


# ---- dedupe set capacity ------------------------------------------


def test_seen_messages_capacity_eviction():
    a = _adapter()
    for i in range(1, nc.SEEN_MESSAGES_MAX + 2):
        a._mark_seen(i)
    # ids 1..SEEN_MESSAGES_EVICT should have been evicted on overflow.
    assert 1 not in a._seen.ids
    assert nc.SEEN_MESSAGES_EVICT not in a._seen.ids
    assert nc.SEEN_MESSAGES_EVICT + 1 in a._seen.ids
    assert nc.SEEN_MESSAGES_MAX + 1 in a._seen.ids
    assert len(a._seen.order) == len(a._seen.ids)


def test_seen_messages_idempotent_mark():
    a = _adapter()
    a._mark_seen(5)
    a._mark_seen(5)
    assert a._seen.order.count(5) == 1


def test_seen_messages_empty_id_ignored():
    a = _adapter()
    a._mark_seen(0)
    assert a._seen.order == []


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
    """Forward-compat fallback: even with the daemon-default
    `threading=false`, this test's fabricated `thread_id` still
    reaches on_send because no `librefang_user` is present to
    take precedence. (In production with a real bridge round-trip
    the librefang_user path wins — see the regression-guard test
    below.)"""
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


def test_on_send_recovers_reply_to_from_user_librefang_user(monkeypatch):
    """Regression guard: the daemon-shape pre-fix bug meant
    `cmd.thread_id=None` so every chunked reply landed at room root
    despite the module docstring claiming to fix exactly that. The
    bridge round-trips librefang_user bytewise — recover from there."""
    a = _adapter()
    fake = _FakeUrlopen([(201, {"ocs": {"meta": {"status": "ok"}, "data": {}}})])
    monkeypatch.setattr(nc.urllib.request, "urlopen", fake)
    import asyncio as _asyncio
    _asyncio.run(a.on_send(_StubCmd(
        text="hi",
        thread_id=None,  # daemon-default
        user={"platform_id": "ROOM1", "librefang_user": "42"},
    )))
    parsed = dict(urllib.parse.parse_qsl(fake.calls[0]["body_raw"]))
    assert parsed == {"message": "hi", "replyTo": "42"}, \
        "on_send must thread via cmd.user.librefang_user when " \
        "cmd.thread_id is None (which is the daemon default)"


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


def test_on_send_non_text_content_fails_loudly(monkeypatch):
    """Non-text content that is neither FileData nor File{url}
    (e.g. a Reaction) → loud failure notice, never the old
    "(Unsupported content type)" placeholder."""
    a = _adapter()
    fake = _FakeUrlopen([(201, {"ocs": {"meta": {"status": "ok"}, "data": {}}})])
    monkeypatch.setattr(nc.urllib.request, "urlopen", fake)
    import asyncio as _asyncio
    _asyncio.run(a.on_send(_StubCmd(
        content={"Reaction": {"emoji": "👍"}},
        user={"platform_id": "ROOM1"},
    )))
    parsed = dict(urllib.parse.parse_qsl(fake.calls[0]["body_raw"]))
    assert parsed["message"] == (
        "[clawbster: outbound media failed — unsupported content shape]"
    )
    assert "(Unsupported content type)" not in parsed["message"]


def test_parse_dm_room_is_not_group():
    """Type 1 (1-to-1) rooms must surface as DMs, not group traffic —
    the daemon routes is_group=False into the user's DM session."""
    a = _adapter()
    a.own_user_id = "librefang-bot"
    ev = a._parse_message(_msg(), "DM1", room_type=1)
    p = ev["params"]
    assert p["user_id"] == "alice"
    assert p["channel_id"] == "DM1"  # reply target stays the room token
    # protocol.message omits is_group when False (absent = DM).
    assert "is_group" not in p
    assert p["metadata"]["room_type"] == 1


def test_parse_group_room_is_group():
    a = _adapter()
    a.own_user_id = "librefang-bot"
    ev = a._parse_message(_msg(), "GRP1", room_type=2)
    assert ev["params"]["is_group"] is True


def test_parse_public_room_is_group():
    """Type 3 (public) is group traffic too — classifying only
    `type == 2` as group would push public rooms into the DM path."""
    a = _adapter()
    a.own_user_id = "librefang-bot"
    ev = a._parse_message(_msg(), "PUB1", room_type=3)
    assert ev["params"]["is_group"] is True


def test_parse_unknown_room_type_defaults_group():
    """Conservative default when the room type is unknown (pre-fix
    behaviour)."""
    a = _adapter()
    a.own_user_id = "librefang-bot"
    ev = a._parse_message(_msg(), "ROOM1")
    assert ev["params"]["is_group"] is True


def _rooms_payload():
    return [
        {"token": "dm-token-abc123", "type": 1,
         "displayName": "Alice Anderson"},
        {"token": "room-token-def456", "type": 2,
         "displayName": "Group Team Chat"},
        {"token": "room-token-ghi789", "type": 3, "displayName": "Public"},
    ]


def _participants_payload():
    return [
        {"actorId": "alice", "actorType": "users",
         "displayName": "Alice Anderson"},
        {"actorId": "librefang-bot", "actorType": "users",
         "displayName": "librefang"},
    ]


def test_build_room_maps_dm_and_roster(monkeypatch):
    a = _adapter()
    a.own_user_id = "librefang-bot"
    fake = _FakeUrlopen([
        (200, {"ocs": {"data": _participants_payload()}}),  # DM room
        (200, {"ocs": {"data": _participants_payload()}}),  # group room
    ])
    monkeypatch.setattr(nc.urllib.request, "urlopen", fake)
    a._build_room_maps(_rooms_payload())
    assert a._room_types == {"dm-token-abc123": 1, "room-token-def456": 2, "room-token-ghi789": 3}
    # Only the non-self user of the 1-to-1 room maps to a DM target.
    assert a._dm_room_by_actor == {"alice": "dm-token-abc123"}
    # Group roster cached for group_members stamping.
    entries = a._room_participants["room-token-def456"]
    assert [e["user_id"] for e in entries] == ["alice", "librefang-bot"]
    assert entries[0]["display_name"] == "Alice Anderson"
    # DM roster entries carry the DM room token as the reply target.
    assert a._room_participants["dm-token-abc123"][0]["platform_id"] == "dm-token-abc123"
    # Participants endpoint hit once per type-1/2 room, with OCS header.
    assert len(fake.calls) == 2
    assert "/room/dm-token-abc123/participants" in fake.calls[0]["url"]
    assert fake.calls[0]["headers"]["ocs-apirequest"] == "true"


def test_build_room_maps_participants_failure_is_nonfatal(monkeypatch):
    """Roster / DM-map construction is best-effort: a participants
    fetch failure must never block polling."""
    a = _adapter()
    a.own_user_id = "librefang-bot"
    fake = _FakeUrlopen([
        (500, {"error": "boom"}),  # DM room participants
        (500, {"error": "boom"}),  # group room participants
    ])
    monkeypatch.setattr(nc.urllib.request, "urlopen", fake)
    a._build_room_maps(_rooms_payload())
    # Types still indexed; maps just stay empty.
    assert a._room_types["dm-token-abc123"] == 1
    assert a._dm_room_by_actor == {}
    # Failed fetches cache an empty roster (never blocks polling).
    assert a._room_participants == {"dm-token-abc123": [], "room-token-def456": []}


def test_poll_once_stamps_group_members(monkeypatch):
    """channel_members: group messages carry the room's cached
    participant roster as group_members metadata (slack #7086 shape)."""
    a = _adapter()
    a.own_user_id = "librefang-bot"
    a._room_watermarks["R1"] = 0
    a._room_types["R1"] = 2
    a._room_participants["R1"] = [
        {"user_id": "alice", "display_name": "Alice Anderson"},
        {"user_id": "librefang-bot", "display_name": "librefang"},
    ]
    fake = _FakeUrlopen([(200, {"ocs": {"data": [_msg(mid=10)]}})])
    monkeypatch.setattr(nc.urllib.request, "urlopen", fake)
    emitted = []
    a._poll_once(emitted.append, ["R1"])
    meta = emitted[0]["params"]["metadata"]
    assert meta["group_members"][0]["user_id"] == "alice"


def test_poll_once_dm_not_stamped_with_roster(monkeypatch):
    """DMs must not carry a group_members roster."""
    a = _adapter()
    a.own_user_id = "librefang-bot"
    a._room_watermarks["R1"] = 0
    a._room_types["R1"] = 1
    a._room_participants["R1"] = [{"user_id": "alice", "display_name": "S"}]
    fake = _FakeUrlopen([(200, {"ocs": {"data": [_msg(mid=10)]}})])
    monkeypatch.setattr(nc.urllib.request, "urlopen", fake)
    emitted = []
    a._poll_once(emitted.append, ["R1"])
    assert "group_members" not in emitted[0]["params"]["metadata"]


def test_on_send_resolves_member_user_id_to_dm_room(monkeypatch):
    """channel_dm: the daemon may pass the member's user id as the
    reply target; it must resolve to their 1-to-1 room token."""
    a = _adapter()
    a._room_types = {"room-token-def456": 2}
    a._dm_room_by_actor = {"alice": "dm-token-abc123"}
    fake = _FakeUrlopen([(201, {"ocs": {"meta": {"status": "ok"},
                                       "data": {}}})])
    monkeypatch.setattr(nc.urllib.request, "urlopen", fake)
    import asyncio as _asyncio
    _asyncio.run(a.on_send(_StubCmd(
        text="psst",
        user={"user_id": "alice", "platform_id": "alice"},
    )))
    assert "/chat/dm-token-abc123" in fake.calls[0]["url"]


def test_on_send_known_room_token_not_rewritten(monkeypatch):
    """A group reply whose platform_id is a known room token must
    never be rewritten through the DM map."""
    a = _adapter()
    a._room_types = {"room-token-def456": 2}
    a._dm_room_by_actor = {"alice": "dm-token-abc123"}
    fake = _FakeUrlopen([(201, {"ocs": {"meta": {"status": "ok"},
                                       "data": {}}})])
    monkeypatch.setattr(nc.urllib.request, "urlopen", fake)
    import asyncio as _asyncio
    _asyncio.run(a.on_send(_StubCmd(
        text="hi group",
        user={"user_id": "alice", "platform_id": "room-token-def456"},
    )))
    assert "/chat/room-token-def456" in fake.calls[0]["url"]


# ---- watermark persistence (Matrix since-cursor pattern) ----------


def test_watermark_state_path_uses_librefang_home_and_safe_user(monkeypatch, tmp_path):
    """Path is $LIBREFANG_HOME/sidecar-state/nextcloud-{safe_user}-
    watermarks.json, with the same sanitisation as Matrix."""
    monkeypatch.setenv("LIBREFANG_HOME", str(tmp_path))
    a = _adapter(NEXTCLOUD_ACCOUNT_ID="bot@cloud.example.com")
    assert a._watermark_state_path == str(
        tmp_path / "sidecar-state"
        / "nextcloud-bot@cloud.example.com-watermarks.json"
    )


def test_watermark_state_path_sanitises_unsafe_account_id(monkeypatch, tmp_path):
    monkeypatch.setenv("LIBREFANG_HOME", str(tmp_path))
    a = _adapter(NEXTCLOUD_ACCOUNT_ID="weird/id spaces!")
    assert a._watermark_state_path == str(
        tmp_path / "sidecar-state"
        / "nextcloud-weird_id_spaces_-watermarks.json"
    )


def test_watermark_state_path_defaults_when_no_account_id(monkeypatch, tmp_path):
    monkeypatch.setenv("LIBREFANG_HOME", str(tmp_path))
    a = _adapter(NEXTCLOUD_ACCOUNT_ID="")
    assert a._watermark_state_path == str(
        tmp_path / "sidecar-state" / "nextcloud-default-watermarks.json"
    )


def test_load_watermarks_no_file_gives_empty_dict(monkeypatch, tmp_path):
    """First run: no state file → empty dict, no error."""
    monkeypatch.setenv("LIBREFANG_HOME", str(tmp_path))
    a = _adapter()
    assert a._room_watermarks == {}
    assert not (tmp_path / "sidecar-state").exists()


def test_load_watermarks_reads_saved_file(monkeypatch, tmp_path):
    monkeypatch.setenv("LIBREFANG_HOME", str(tmp_path))
    state = tmp_path / "sidecar-state"
    state.mkdir()
    (state / "nextcloud-default-watermarks.json").write_text(
        json.dumps({"R1": 42, "R2": 7}), encoding="utf-8",
    )
    a = _adapter()
    assert a._room_watermarks == {"R1": 42, "R2": 7}


def test_watermark_round_trip_persist_then_load(monkeypatch, tmp_path):
    monkeypatch.setenv("LIBREFANG_HOME", str(tmp_path))
    a = _adapter()
    a._room_watermarks = {"R1": 100, "R2": 5}
    a._persist_watermarks()
    # Fresh adapter instance = simulated restart.
    b = _adapter()
    assert b._room_watermarks == {"R1": 100, "R2": 5}
    # File is valid JSON with exactly the dict.
    on_disk = json.loads(
        (tmp_path / "sidecar-state" / "nextcloud-default-watermarks.json")
        .read_text(encoding="utf-8"),
    )
    assert on_disk == {"R1": 100, "R2": 5}


def test_persist_watermarks_is_atomic_no_tmp_left_behind(monkeypatch, tmp_path):
    """Atomic write: tmp file is replaced, not left dangling."""
    monkeypatch.setenv("LIBREFANG_HOME", str(tmp_path))
    a = _adapter()
    a._room_watermarks = {"R1": 3}
    a._persist_watermarks()
    state_dir = tmp_path / "sidecar-state"
    assert not (state_dir / "nextcloud-default-watermarks.json.tmp").exists()
    assert (state_dir / "nextcloud-default-watermarks.json").exists()


def test_persist_watermarks_creates_state_dir(monkeypatch, tmp_path):
    monkeypatch.setenv("LIBREFANG_HOME", str(tmp_path))
    a = _adapter()
    a._room_watermarks = {"R1": 3}
    a._persist_watermarks()
    assert (tmp_path / "sidecar-state").is_dir()


def test_poll_once_persists_watermark_bump(monkeypatch, tmp_path):
    """The poll loop persists after each watermark advance, so a
    crash between polls replays at most one pass."""
    monkeypatch.setenv("LIBREFANG_HOME", str(tmp_path))
    a = _adapter()
    a.own_user_id = "librefang-bot"
    a._room_watermarks["R1"] = 0
    fake = _FakeUrlopen([
        (200, {"ocs": {"data": [_msg(mid=10), _msg(mid=11)]}}),
    ])
    monkeypatch.setattr(nc.urllib.request, "urlopen", fake)
    a._poll_once(lambda ev: None, ["R1"])
    on_disk = json.loads(
        (tmp_path / "sidecar-state" / "nextcloud-default-watermarks.json")
        .read_text(encoding="utf-8"),
    )
    assert on_disk == {"R1": 11}


def test_restart_survival_no_replay(monkeypatch, tmp_path):
    """End-to-end: adapter A polls and bumps; adapter B (restart)
    starts from the persisted watermark and asks Talk for messages
    strictly newer than it."""
    monkeypatch.setenv("LIBREFANG_HOME", str(tmp_path))
    a = _adapter()
    a.own_user_id = "librefang-bot"
    a._room_watermarks["R1"] = 0
    fake = _FakeUrlopen([
        (200, {"ocs": {"data": [_msg(mid=10)]}}),
    ])
    monkeypatch.setattr(nc.urllib.request, "urlopen", fake)
    a._poll_once(lambda ev: None, ["R1"])

    b = _adapter()
    b.own_user_id = "librefang-bot"
    assert b._room_watermarks == {"R1": 10}
    fake2 = _FakeUrlopen([(200, {"ocs": {"data": []}})])
    monkeypatch.setattr(nc.urllib.request, "urlopen", fake2)
    b._poll_once(lambda ev: None, ["R1"])
    # lastKnownMessageId in the resumed query is the persisted 10.
    assert "lastKnownMessageId=10" in fake2.calls[0]["url"]


def test_load_watermarks_corrupt_json_degrades_gracefully(monkeypatch, tmp_path):
    """Corrupt JSON must not crash the adapter — warn and start
    fresh."""
    monkeypatch.setenv("LIBREFANG_HOME", str(tmp_path))
    state = tmp_path / "sidecar-state"
    state.mkdir()
    (state / "nextcloud-default-watermarks.json").write_text(
        "{not json at all", encoding="utf-8",
    )
    a = _adapter()
    assert a._room_watermarks == {}


def test_load_watermarks_non_dict_json_degrades_gracefully(monkeypatch, tmp_path):
    monkeypatch.setenv("LIBREFANG_HOME", str(tmp_path))
    state = tmp_path / "sidecar-state"
    state.mkdir()
    (state / "nextcloud-default-watermarks.json").write_text(
        "[1, 2, 3]", encoding="utf-8",
    )
    a = _adapter()
    assert a._room_watermarks == {}


def test_load_watermarks_skips_non_integer_values(monkeypatch, tmp_path):
    """Junk values are dropped, valid ones kept."""
    monkeypatch.setenv("LIBREFANG_HOME", str(tmp_path))
    state = tmp_path / "sidecar-state"
    state.mkdir()
    (state / "nextcloud-default-watermarks.json").write_text(
        json.dumps({"R1": 5, "R2": "abc", "R3": None, "R4": "9"}),
        encoding="utf-8",
    )
    a = _adapter()
    assert a._room_watermarks == {"R1": 5, "R4": 9}


def test_persist_watermarks_unwritable_path_warns_not_crashes(monkeypatch, tmp_path):
    """OSError on persist must degrade to a warn, never kill the
    poll loop."""
    monkeypatch.setenv("LIBREFANG_HOME", str(tmp_path))
    a = _adapter()
    a._room_watermarks = {"R1": 1}
    a._watermark_state_path = str(
        tmp_path / "no-such-parent" / "x" / "watermarks.json",
    )
    # os.makedirs fails: parent 'x' sits under a file, not a dir.
    blocker = tmp_path / "no-such-parent"
    blocker.write_text("i am a file", encoding="utf-8")
    a._persist_watermarks()  # must not raise


# ---- inbound media (Phase 1) ---------------------------------------
#
# Ground truth for these fixtures: spreed's
# SystemMessage.php::getFileFromShare resolves the shared node in
# the *viewer's* own user folder and emits `path` relative to that
# root (e.g. "Talk/photo.jpg"); talk-android downloads via
# baseUrl/remote.php/dav/files/<user>/<Uri.encode(path, "/")>.


def _file_param(**over):
    out = {
        "type": "file",
        "id": "17",
        "name": "photo.jpg",
        "path": "Talk/photo.jpg",
        "size": "2048",
        "mimetype": "image/jpeg",
        "link": "https://cloud.example.com/f/17",
    }
    out.update(over)
    return out


def _media_msg(**over):
    """Chat message carrying a `file` rich object — the shape Talk's
    OCS chat API delivers for shared files / voice messages."""
    msg = _msg(
        mid=over.pop("mid", 30),
        text=over.pop("text", "{file}"),
        actor_id=over.pop("actor_id", "alice"),
        message_type=over.pop("message_type", None),
    )
    msg["messageParameters"] = {"file": _file_param(**over.pop("file", {}))}
    msg.update(over)
    return msg


# -- config -----------------------------------------------------------


def test_media_config_defaults():
    a = _adapter()
    p = a.media_policy
    assert p.enabled is True
    assert p.max_bytes == nc.DEFAULT_MEDIA_MAX_BYTES
    assert p.mime_allowlist == (
        "image/*", "audio/*", "video/*", "application/pdf",
    )
    assert p.upload_dir == "LibreFang"


def test_media_header_rules_pinned_to_server_host():
    """The DAV URL is always built from `server_url`, so the token
    only ever attaches to the configured host (the daemon
    exact-matches hosts). Declared only while media receive is on —
    slack #7087."""
    a = _adapter()
    assert a.header_rules == [
        ("cloud.example.com", [["Authorization", "Bearer test-token"]]),
    ]
    a = _adapter(NEXTCLOUD_MEDIA_MAX_BYTES="0")
    assert a.header_rules == []


def test_media_max_bytes_invalid_exits():
    with pytest.raises(SystemExit) as exc:
        _adapter(NEXTCLOUD_MEDIA_MAX_BYTES="abc")
    assert exc.value.code == 2


def test_media_max_bytes_negative_exits():
    with pytest.raises(SystemExit) as exc:
        _adapter(NEXTCLOUD_MEDIA_MAX_BYTES="-1")
    assert exc.value.code == 2


def test_media_max_bytes_zero_disables_receive():
    a = _adapter(NEXTCLOUD_MEDIA_MAX_BYTES="0")
    assert a.media_policy.enabled is False


def test_media_mime_allowlist_custom():
    a = _adapter(NEXTCLOUD_MEDIA_MIME_ALLOWLIST="image/png, text/plain")
    assert a.media_policy.mime_allowlist == ("image/png", "text/plain")
    assert a.media_policy.mime_allowed("image/png") is True
    assert a.media_policy.mime_allowed("image/jpeg") is False


def test_media_upload_dir_invalid_exits():
    """A nonsense upload dir must fail at startup, not on the first
    Phase 2 upload."""
    for bad in ("..", "a//b", "a/../b", "/"):
        with pytest.raises(SystemExit) as exc:
            _adapter(NEXTCLOUD_MEDIA_UPLOAD_DIR=bad)
        assert exc.value.code == 2, bad


def test_media_upload_dir_slashes_trimmed():
    a = _adapter(NEXTCLOUD_MEDIA_UPLOAD_DIR="/Talk/Media/")
    assert a.media_policy.upload_dir == "Talk/Media"


# -- policy -----------------------------------------------------------


def test_media_mime_allowed_wildcard_exact_and_case():
    p = _adapter().media_policy
    assert p.mime_allowed("image/png") is True
    assert p.mime_allowed("IMAGE/PNG") is True
    assert p.mime_allowed("application/pdf") is True
    assert p.mime_allowed("application/zip") is False
    # An empty / missing mimetype is never allowed — we cannot
    # verify what the daemon would download.
    assert p.mime_allowed("") is False
    assert p.mime_allowed(None) is False


def test_media_rejection_reason_size_string_over_cap():
    """Talk sends `size` as a string; over-cap files are rejected."""
    a = _adapter(NEXTCLOUD_MEDIA_MAX_BYTES="1024")
    reason = a.media_policy.rejection_reason(_file_param(size="2048"))
    assert reason is not None and "over the" in reason


def test_media_rejection_reason_size_absent_or_bad_not_rejected():
    """Absent / unparseable sizes are not rejections — the daemon's
    fetch has its own limits (same as slack)."""
    p = _adapter().media_policy
    assert p.rejection_reason(_file_param(size=None)) is None
    assert p.rejection_reason(_file_param(size="huge")) is None


def test_media_rejection_reason_no_path():
    p = _adapter().media_policy
    reason = p.rejection_reason(_file_param(path=""))
    assert reason is not None and "no path" in reason


def test_media_rejection_reason_mime_not_allowed():
    p = _adapter().media_policy
    reason = p.rejection_reason(
        _file_param(mimetype="application/zip")
    )
    assert reason is not None and "not in allowlist" in reason


def test_media_rejection_reason_disabled():
    a = _adapter(NEXTCLOUD_MEDIA_MAX_BYTES="0")
    reason = a.media_policy.rejection_reason(_file_param())
    assert reason is not None and "disabled" in reason


# -- DAV URL ----------------------------------------------------------


def test_dav_file_url_matches_official_clients():
    """talk-android: baseUrl/remote.php/dav/files/<user>/<Uri.encode(
    path, "/")> — segments percent-encoded, `/` kept as separator."""
    a = _adapter()
    a.own_user_id = "librefang-bot"
    url = a._dav_file_url(_file_param(path="Talk/room 1/photo final.jpg"))
    assert url == ("https://cloud.example.com/remote.php/dav/files/"
                   "librefang-bot/Talk/room%201/photo%20final.jpg")


def test_dav_file_url_encodes_query_and_unicode():
    """`?`/`&` must not leak into the URL's query; non-ASCII
    percent-encodes as UTF-8."""
    a = _adapter()
    a.own_user_id = "librefang-bot"
    url = a._dav_file_url(_file_param(path="Grüße/a&b=c?d.png"))
    assert url == ("https://cloud.example.com/remote.php/dav/files/"
                   "librefang-bot/Gr%C3%BC%C3%9Fe/a%26b%3Dc%3Fd.png")


def test_dav_file_url_collapses_empty_segments():
    a = _adapter()
    a.own_user_id = "librefang-bot"
    url = a._dav_file_url(_file_param(path="/Talk//photo.jpg/"))
    assert url == ("https://cloud.example.com/remote.php/dav/files/"
                   "librefang-bot/Talk/photo.jpg")


def test_dav_file_url_quotes_user_id():
    a = _adapter()
    a.own_user_id = "bot user"
    url = a._dav_file_url(_file_param(path="Talk/x.jpg"))
    assert url == ("https://cloud.example.com/remote.php/dav/files/"
                   "bot%20user/Talk/x.jpg")


# -- mapping ----------------------------------------------------------


def test_parse_image_share_maps_to_image_content():
    a = _adapter()
    a.own_user_id = "librefang-bot"
    ev = a._parse_message(_media_msg(), "R1")
    assert ev["params"]["content"] == {
        "Image": {
            "url": "https://cloud.example.com/remote.php/dav/files/"
                   "librefang-bot/Talk/photo.jpg",
            "caption": None,
            "mime_type": "image/jpeg",
        },
    }


def test_parse_voice_message_maps_to_voice_content():
    """Voice messages are file shares with messageType
    'voice-message' — the only messageType Talk sets reliably."""
    a = _adapter()
    a.own_user_id = "librefang-bot"
    ev = a._parse_message(
        _media_msg(
            message_type="voice-message",
            file={"mimetype": "audio/ogg", "name": "voice.ogg",
                  "path": "Talk/voice.ogg"},
        ),
        "R1",
    )
    assert ev["params"]["content"] == {
        "Voice": {
            "url": "https://cloud.example.com/remote.php/dav/files/"
                   "librefang-bot/Talk/voice.ogg",
            "caption": None,
            "duration_seconds": 0,
        },
    }


def test_parse_audio_share_maps_to_audio_content():
    """Plain audio shares parse as `comment` — messageType is not a
    reliable signal, so mapping keys off the mimetype."""
    a = _adapter()
    a.own_user_id = "librefang-bot"
    ev = a._parse_message(
        _media_msg(
            file={"mimetype": "audio/mpeg", "name": "song.mp3",
                  "path": "Music/song.mp3"},
        ),
        "R1",
    )
    assert ev["params"]["content"] == {
        "Audio": {
            "url": "https://cloud.example.com/remote.php/dav/files/"
                   "librefang-bot/Music/song.mp3",
            "caption": None,
            "duration_seconds": 0,
            "title": "song.mp3",
        },
    }


def test_parse_video_share_maps_to_video_content():
    a = _adapter()
    a.own_user_id = "librefang-bot"
    ev = a._parse_message(
        _media_msg(
            file={"mimetype": "video/mp4", "name": "clip.mp4",
                  "path": "Talk/clip.mp4"},
        ),
        "R1",
    )
    assert ev["params"]["content"] == {
        "Video": {
            "url": "https://cloud.example.com/remote.php/dav/files/"
                   "librefang-bot/Talk/clip.mp4",
            "caption": None,
            "duration_seconds": 0,
            "filename": "clip.mp4",
        },
    }


def test_parse_document_share_maps_to_file_content():
    a = _adapter()
    a.own_user_id = "librefang-bot"
    ev = a._parse_message(
        _media_msg(
            file={"mimetype": "application/pdf", "name": "report.pdf",
                  "path": "Talk/report.pdf"},
        ),
        "R1",
    )
    assert ev["params"]["content"] == {
        "File": {
            "url": "https://cloud.example.com/remote.php/dav/files/"
                   "librefang-bot/Talk/report.pdf",
            "filename": "report.pdf",
        },
    }


def test_parse_media_caption_becomes_caption():
    """A captioned share carries real text (Talk's
    talkMetaData.caption becomes the message text)."""
    a = _adapter()
    a.own_user_id = "librefang-bot"
    ev = a._parse_message(_media_msg(text="look at this"), "R1")
    assert ev["params"]["content"]["Image"]["caption"] == "look at this"


def test_parse_document_share_drops_companion_text():
    """The File variant has no caption field; companion text is
    warned about and dropped rather than silently lost."""
    a = _adapter()
    a.own_user_id = "librefang-bot"
    ev = a._parse_message(
        _media_msg(
            text="see attached",
            file={"mimetype": "application/pdf", "name": "report.pdf",
                  "path": "Talk/report.pdf"},
        ),
        "R1",
    )
    assert ev["params"]["content"] == {
        "File": {
            "url": "https://cloud.example.com/remote.php/dav/files/"
                   "librefang-bot/Talk/report.pdf",
            "filename": "report.pdf",
        },
    }


def test_parse_media_rejected_returns_none():
    """Rejected files drop the message rather than emitting the bare
    `{file}` placeholder as text."""
    a = _adapter(NEXTCLOUD_MEDIA_MAX_BYTES="1024")
    a.own_user_id = "librefang-bot"
    assert a._parse_message(
        _media_msg(file={"size": "999999"}), "R1",
    ) is None


def test_parse_media_mime_rejected_returns_none():
    a = _adapter()
    a.own_user_id = "librefang-bot"
    assert a._parse_message(
        _media_msg(file={"mimetype": "application/zip"}), "R1",
    ) is None


def test_parse_media_disabled_returns_none():
    a = _adapter(NEXTCLOUD_MEDIA_MAX_BYTES="0")
    a.own_user_id = "librefang-bot"
    assert a._parse_message(_media_msg(), "R1") is None


def test_parse_media_without_own_user_id_returns_none():
    """No user id → no DAV namespace to build a URL in; skip rather
    than emit a broken URL."""
    a = _adapter()
    a.own_user_id = ""
    assert a._parse_message(_media_msg(), "R1") is None


def test_parse_message_with_non_file_params_stays_text():
    """Mentions etc. are ordinary rich objects — only `type ==
    "file"` switches to the media path."""
    a = _adapter()
    a.own_user_id = "librefang-bot"
    msg = _msg(mid=31, text="hi {mention-user1}")
    msg["messageParameters"] = {
        "mention-user1": {"type": "user", "id": "alice",
                          "name": "Alice"},
    }
    ev = a._parse_message(msg, "R1")
    assert ev["params"]["content"] == {"Text": "hi {mention-user1}"}


def test_parse_self_shared_file_skipped():
    a = _adapter()
    a.own_user_id = "librefang-bot"
    assert a._parse_message(
        _media_msg(actor_id="librefang-bot"), "R1",
    ) is None


def test_parse_system_message_with_file_param_skipped():
    a = _adapter()
    a.own_user_id = "librefang-bot"
    assert a._parse_message(
        _media_msg(message_type="system"), "R1",
    ) is None


# -- poll integration --------------------------------------------------


def test_poll_once_emits_media_and_advances_watermark(monkeypatch):
    a = _adapter()
    a.own_user_id = "librefang-bot"
    a._room_watermarks["R1"] = 0
    fake = _FakeUrlopen([
        (200, {"ocs": {"data": [_media_msg(mid=40)]}}),
    ])
    monkeypatch.setattr(nc.urllib.request, "urlopen", fake)
    emitted = []
    a._poll_once(emitted.append, ["R1"])
    assert len(emitted) == 1
    assert emitted[0]["params"]["content"]["Image"]["url"] == (
        "https://cloud.example.com/remote.php/dav/files/"
        "librefang-bot/Talk/photo.jpg"
    )
    assert a._room_watermarks["R1"] == 40
    assert 40 in a._seen.ids


def test_poll_once_rejected_media_still_advances_watermark(monkeypatch):
    """A rejected file must not re-fetch forever: the watermark and
    dedupe set advance on every fetched id, emitted or not."""
    a = _adapter(NEXTCLOUD_MEDIA_MAX_BYTES="1024")
    a.own_user_id = "librefang-bot"
    a._room_watermarks["R1"] = 0
    fake = _FakeUrlopen([
        (200, {"ocs": {"data": [
            _media_msg(mid=41, file={"size": "999999"}),
        ]}}),
        (200, {"ocs": {"data": [
            _media_msg(mid=41, file={"size": "999999"}),
        ]}}),
    ])
    monkeypatch.setattr(nc.urllib.request, "urlopen", fake)
    emitted = []
    a._poll_once(emitted.append, ["R1"])
    assert emitted == []
    assert a._room_watermarks["R1"] == 41
    a._poll_once(emitted.append, ["R1"])
    assert emitted == []
    assert a._room_watermarks["R1"] == 41


# ---- on_send: Phase 2 outbound media (upload + share) ------------


def _media_ok_ocs():
    return {"ocs": {"meta": {"status": "ok"}, "data": {}}}


def _file_data_cmd(data, filename="photo.png", mime="image/png",
                   text=None, room="ROOM1"):
    content = {"FileData": {
        "data": data, "filename": filename, "mime_type": mime,
    }}
    return _StubCmd(
        text=text if text is not None else json.dumps(content),
        content=content,
        user={"platform_id": room},
    )


def test_on_send_file_data_uploads_then_shares(monkeypatch):
    """Happy path: FileData → DAV PUT (Content-Type from mime_type,
    filename in the URI, Bearer auth) → OC-FileId read from the PUT
    response → OCS share POST, form-encoded per the OCS layer
    (objectType + objectId + metaData as a JSON string whose values
    are ALL strings — the rich-object validator rejects non-string
    values). The daemon's JSON flattening of the content into `text`
    must NOT be posted as a caption."""
    a = _adapter()
    a.own_user_id = "librefang-bot"
    fake = _FakeUrlopen([
        (201, b"", {"OC-FileId": "12345"}),
        (201, _media_ok_ocs()),
    ])
    monkeypatch.setattr(nc.urllib.request, "urlopen", fake)
    import asyncio as _asyncio
    _asyncio.run(a.on_send(_file_data_cmd(list(b"PNGDATA"))))
    assert len(fake.calls) == 2
    put = fake.calls[0]
    assert put["url"] == (
        "https://cloud.example.com/remote.php/dav/files/"
        "librefang-bot/LibreFang/photo.png"
    )
    assert put["method"] == "PUT"
    assert put["headers"]["content-type"] == "image/png"
    assert put["headers"]["authorization"] == "Bearer test-token"
    assert put["body_raw"] == "PNGDATA"
    share = fake.calls[1]
    assert share["url"] == (
        "https://cloud.example.com/ocs/v2.php/apps/spreed/api/v1/"
        "chat/ROOM1/share"
    )
    assert share["method"] == "POST"
    assert share["headers"]["content-type"] == (
        "application/x-www-form-urlencoded"
    )
    assert share["headers"]["ocs-apirequest"] == "true"
    form = dict(urllib.parse.parse_qsl(share["body_raw"]))
    assert form["objectType"] == "file"
    assert form["objectId"] == "12345"
    meta = json.loads(form["metaData"])
    assert all(isinstance(v, str) for v in meta.values())
    assert meta == {
        "name": "photo.png",
        "path": "LibreFang/photo.png",
        "size": "7",
        "mimetype": "image/png",
    }


def test_on_send_file_data_caption_after_share(monkeypatch):
    """A caption riding with the media (text that is NOT the JSON
    flattening of the content object) rides in the share's metaData
    AND is delivered as a separate text message AFTER the share."""
    a = _adapter()
    a.own_user_id = "librefang-bot"
    fake = _FakeUrlopen([
        (201, b"", {"OC-FileId": "12345"}),
        (201, _media_ok_ocs()),
        (201, _media_ok_ocs()),
    ])
    monkeypatch.setattr(nc.urllib.request, "urlopen", fake)
    import asyncio as _asyncio
    _asyncio.run(a.on_send(_file_data_cmd(
        list(b"PNGDATA"), text="look at this",
    )))
    assert len(fake.calls) == 3
    assert "/chat/ROOM1/share" in fake.calls[1]["url"]
    share_form = dict(urllib.parse.parse_qsl(fake.calls[1]["body_raw"]))
    assert json.loads(share_form["metaData"])["caption"] == "look at this"
    caption = fake.calls[2]
    assert "/chat/ROOM1" in caption["url"]
    assert "/share" not in caption["url"]
    parsed = dict(urllib.parse.parse_qsl(caption["body_raw"]))
    assert parsed == {"message": "look at this"}


def test_on_send_file_data_malformed_rejected_loudly(monkeypatch):
    """Malformed byte array → loud rejection (stderr + chat notice),
    no DAV PUT, no share — and never the old placeholder string."""
    a = _adapter()
    fake = _FakeUrlopen([(201, _media_ok_ocs())])
    monkeypatch.setattr(nc.urllib.request, "urlopen", fake)
    import asyncio as _asyncio
    _asyncio.run(a.on_send(_StubCmd(
        text=None,
        content={"FileData": {
            "data": "not-a-list", "filename": "x.png",
            "mime_type": "image/png",
        }},
        user={"platform_id": "ROOM1"},
    )))
    assert len(fake.calls) == 1
    call = fake.calls[0]
    assert "remote.php/dav" not in call["url"]
    parsed = dict(urllib.parse.parse_qsl(call["body_raw"]))
    assert parsed["message"] == (
        "[clawbster: outbound media failed — malformed byte array]"
    )
    assert "(Unsupported content type)" not in parsed["message"]


def test_on_send_file_data_bool_in_array_rejected(monkeypatch):
    """`isinstance(True, int)` is True in Python — bools inside the
    byte array are malformed (slack #7087 pattern) and must be
    rejected, not silently coerced to 1/0."""
    a = _adapter()
    fake = _FakeUrlopen([(201, _media_ok_ocs())])
    monkeypatch.setattr(nc.urllib.request, "urlopen", fake)
    import asyncio as _asyncio
    _asyncio.run(a.on_send(_StubCmd(
        text=None,
        content={"FileData": {
            "data": [80, True, 78], "filename": "x.png",
            "mime_type": "image/png",
        }},
        user={"platform_id": "ROOM1"},
    )))
    assert len(fake.calls) == 1
    assert "remote.php/dav" not in fake.calls[0]["url"]
    parsed = dict(urllib.parse.parse_qsl(fake.calls[0]["body_raw"]))
    assert parsed["message"] == (
        "[clawbster: outbound media failed — malformed byte array]"
    )


def test_on_send_file_data_above_cap_rejected_before_http(monkeypatch):
    """>64 MiB → rejected before any upload HTTP traffic; only the
    failure notice is posted."""
    a = _adapter()
    fake = _FakeUrlopen([(201, _media_ok_ocs())])
    monkeypatch.setattr(nc.urllib.request, "urlopen", fake)
    import asyncio as _asyncio
    _asyncio.run(a.on_send(_StubCmd(
        text=None,
        content={"FileData": {
            "data": bytes(nc.MAX_UPLOAD_BYTES + 1),
            "filename": "big.bin",
            "mime_type": "application/octet-stream",
        }},
        user={"platform_id": "ROOM1"},
    )))
    assert len(fake.calls) == 1
    assert "remote.php/dav" not in fake.calls[0]["url"]
    parsed = dict(urllib.parse.parse_qsl(fake.calls[0]["body_raw"]))
    assert parsed["message"] == (
        f"[clawbster: outbound media failed — payload above "
        f"{nc.MAX_UPLOAD_BYTES}-byte upload limit]"
    )


def test_on_send_file_data_oc_fileid_padded_instance_suffix(monkeypatch):
    """Real-world header shape (observed live on nextcloud.jsbart.com):
    zero-padded numeric id + 'oc' + instance id, e.g.
    '02861982octccaigsouv' → numeric 2861982. The share must be
    posted with the unpadded numeric id and NO PROPFIND fallback."""
    a = _adapter()
    a.own_user_id = "librefang-bot"
    fake = _FakeUrlopen([
        (201, b"", {"OC-FileId": "02861982octccaigsouv"}),
        (201, _media_ok_ocs()),
    ])
    monkeypatch.setattr(nc.urllib.request, "urlopen", fake)
    import asyncio as _asyncio
    _asyncio.run(a.on_send(_file_data_cmd(list(b"PNGDATA"))))
    assert len(fake.calls) == 2  # PUT + share — no PROPFIND
    share = fake.calls[1]
    assert share["url"] == (
        "https://cloud.example.com/ocs/v2.php/apps/spreed/api/v1/"
        "chat/ROOM1/share"
    )
    form = dict(urllib.parse.parse_qsl(share["body_raw"]))
    assert form["objectId"] == "2861982"
    assert all(
        isinstance(v, str)
        for v in json.loads(form["metaData"]).values()
    )


def test_on_send_file_data_oc_fileid_unparseable_propfind_fallback(monkeypatch):
    """Header present but unparseable → PROPFIND oc:fileid on the
    uploaded path (Depth 0) resolves the numeric id → share still
    posted."""
    a = _adapter()
    a.own_user_id = "librefang-bot"
    propfind_xml = (
        '<?xml version="1.0"?>'
        '<d:multistatus xmlns:d="DAV:" '
        'xmlns:s="http://sabredav.org/ns" '
        'xmlns:oc="http://owncloud.org/ns">'
        "<d:response>"
        "<d:href>/remote.php/dav/files/librefang-bot/"
        "LibreFang/photo.png</d:href>"
        "<d:propstat><d:prop>"
        "<oc:fileid>2861982</oc:fileid>"
        "</d:prop><d:status>HTTP/1.1 200 OK</d:status>"
        "</d:propstat></d:response></d:multistatus>"
    ).encode("utf-8")
    fake = _FakeUrlopen([
        (201, b"", {"OC-FileId": "urn:uuid:not-a-file-id"}),
        (207, propfind_xml, {}),
        (201, _media_ok_ocs()),
    ])
    monkeypatch.setattr(nc.urllib.request, "urlopen", fake)
    import asyncio as _asyncio
    _asyncio.run(a.on_send(_file_data_cmd(list(b"PNGDATA"))))
    assert len(fake.calls) == 3
    propfind = fake.calls[1]
    assert propfind["method"] == "PROPFIND"
    assert propfind["url"] == (
        "https://cloud.example.com/remote.php/dav/files/"
        "librefang-bot/LibreFang/photo.png"
    )
    assert propfind["headers"]["depth"] == "0"
    share = fake.calls[2]
    assert share["url"].endswith("/chat/ROOM1/share")
    form = dict(urllib.parse.parse_qsl(share["body_raw"]))
    assert form["objectId"] == "2861982"


def test_on_send_file_data_file_id_unresolvable_no_share(monkeypatch):
    """PUT succeeds but the OC-FileId header is missing AND the
    PROPFIND fallback fails (404 — no DAV entry) → failure path:
    no share POST, loud notice instead."""
    a = _adapter()
    a.own_user_id = "librefang-bot"
    fake = _FakeUrlopen([
        (201, b"", {}),  # PUT ok but no OC-FileId header
        (404, '<?xml version="1.0" encoding="utf-8"?><d:error xmlns:d="DAV:" xmlns:s="http://sabredav.org/ns"><s:exception>Sabre\\DAV\\Exception\\NotFound</s:exception></d:error>', {}),  # PROPFIND fallback fails
        (201, _media_ok_ocs()),  # the failure notice itself
    ])
    monkeypatch.setattr(nc.urllib.request, "urlopen", fake)
    import asyncio as _asyncio
    _asyncio.run(a.on_send(_file_data_cmd(list(b"PNGDATA"))))
    assert len(fake.calls) == 3
    assert fake.calls[1]["method"] == "PROPFIND"
    assert "/share" not in fake.calls[2]["url"]
    parsed = dict(urllib.parse.parse_qsl(fake.calls[2]["body_raw"]))
    assert parsed["message"] == (
        "[clawbster: outbound media failed — file id unresolvable: "
        "OC-FileId header None, DAV PROPFIND oc:fileid fallback "
        "failed]"
    )


def test_on_send_file_data_dav_put_error_no_share(monkeypatch):
    """DAV PUT fails (409) → failure path: no share POST, loud
    notice carrying the status."""
    a = _adapter()
    a.own_user_id = "librefang-bot"
    fake = _FakeUrlopen([
        (409, "conflict"),
        (201, _media_ok_ocs()),
    ])
    monkeypatch.setattr(nc.urllib.request, "urlopen", fake)
    import asyncio as _asyncio
    _asyncio.run(a.on_send(_file_data_cmd(list(b"PNGDATA"))))
    assert len(fake.calls) == 2
    assert "/share" not in fake.calls[1]["url"]
    parsed = dict(urllib.parse.parse_qsl(fake.calls[1]["body_raw"]))
    assert parsed["message"].startswith(
        "[clawbster: outbound media failed — DAV PUT error 409"
    )


def test_on_send_file_url_not_implemented_notice(monkeypatch):
    """File{url} variant → explicit not-implemented notice; no DAV
    PUT, no share, no placeholder string."""
    a = _adapter()
    fake = _FakeUrlopen([(201, _media_ok_ocs())])
    monkeypatch.setattr(nc.urllib.request, "urlopen", fake)
    import asyncio as _asyncio
    _asyncio.run(a.on_send(_StubCmd(
        text=None,
        content={"File": {
            "url": "https://elsewhere.example/x.png",
            "filename": "x.png",
        }},
        user={"platform_id": "ROOM1"},
    )))
    assert len(fake.calls) == 1
    call = fake.calls[0]
    assert "remote.php/dav" not in call["url"]
    assert "/share" not in call["url"]
    parsed = dict(urllib.parse.parse_qsl(call["body_raw"]))
    assert parsed["message"] == (
        "[clawbster: outbound media by URL is not implemented yet]"
    )
    assert "(Unsupported content type)" not in parsed["message"]


def test_on_send_unknown_content_shape_fails_loudly(monkeypatch):
    """Non-FileData / non-File structured content (e.g. an inbound
    Image variant echoed back) → loud failure, never the old
    placeholder."""
    a = _adapter()
    fake = _FakeUrlopen([(201, _media_ok_ocs())])
    monkeypatch.setattr(nc.urllib.request, "urlopen", fake)
    import asyncio as _asyncio
    _asyncio.run(a.on_send(_StubCmd(
        text=None,
        content={"Image": {"url": "https://x.example/y.png"}},
        user={"platform_id": "ROOM1"},
    )))
    assert len(fake.calls) == 1
    parsed = dict(urllib.parse.parse_qsl(fake.calls[0]["body_raw"]))
    assert parsed["message"] == (
        "[clawbster: outbound media failed — unsupported content shape]"
    )
    assert "(Unsupported content type)" not in parsed["message"]


def test_on_send_file_data_mime_fallback_octet_stream(monkeypatch):
    """Missing / non-string mime_type → Content-Type falls back to
    application/octet-stream on the DAV PUT."""
    a = _adapter()
    a.own_user_id = "librefang-bot"
    fake = _FakeUrlopen([
        (201, b"", {"OC-FileId": "7"}),
        (201, _media_ok_ocs()),
    ])
    monkeypatch.setattr(nc.urllib.request, "urlopen", fake)
    import asyncio as _asyncio
    _asyncio.run(a.on_send(_StubCmd(
        text=None,
        content={"FileData": {
            "data": list(b"BIN"), "filename": "blob.bin",
            "mime_type": None,
        }},
        user={"platform_id": "ROOM1"},
    )))
    assert fake.calls[0]["headers"]["content-type"] == (
        "application/octet-stream"
    )


def test_on_send_file_data_no_own_user_fails(monkeypatch):
    """own_user_id unknown (credential discovery failed) → loud
    failure, no DAV PUT (mirrors the Phase 1 inbound guard)."""
    a = _adapter()  # own_user_id stays "" — discovery never ran
    fake = _FakeUrlopen([(201, _media_ok_ocs())])
    monkeypatch.setattr(nc.urllib.request, "urlopen", fake)
    import asyncio as _asyncio
    _asyncio.run(a.on_send(_file_data_cmd(list(b"PNGDATA"))))
    assert len(fake.calls) == 1
    assert "remote.php/dav" not in fake.calls[0]["url"]
    parsed = dict(urllib.parse.parse_qsl(fake.calls[0]["body_raw"]))
    assert parsed["message"] == (
        "[clawbster: outbound media failed — "
        "own user id unknown (credential discovery failed)]"
    )


def test_on_send_file_data_filename_sanitised(monkeypatch):
    """A path-y / NUL-laden filename is sanitised to its basename
    before it reaches the DAV URI (no smuggled URI segments)."""
    a = _adapter()
    a.own_user_id = "librefang-bot"
    fake = _FakeUrlopen([
        (201, b"", {"OC-FileId": "9"}),
        (201, _media_ok_ocs()),
    ])
    monkeypatch.setattr(nc.urllib.request, "urlopen", fake)
    import asyncio as _asyncio
    _asyncio.run(a.on_send(_StubCmd(
        text=None,
        content={"FileData": {
            "data": list(b"PNGDATA"),
            "filename": "../../etc/passwd\x00.png",
            "mime_type": "image/png",
        }},
        user={"platform_id": "ROOM1"},
    )))
    assert fake.calls[0]["url"] == (
        "https://cloud.example.com/remote.php/dav/files/"
        "librefang-bot/LibreFang/passwd.png"
    )
    form = dict(urllib.parse.parse_qsl(fake.calls[1]["body_raw"]))
    meta = json.loads(form["metaData"])
    assert meta["name"] == "passwd.png"
    assert meta["path"] == "LibreFang/passwd.png"
