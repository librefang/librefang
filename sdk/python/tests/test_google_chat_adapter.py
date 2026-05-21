"""Tests for librefang.sidecar.adapters.google_chat.

Deterministic, no network. The RSA-RS256 signer is exercised with a
generated 2048-bit key so the PEM/DER parser + PKCS#1 signing path is
covered without baking a real Google service-account key into the
fixture.
"""
from __future__ import annotations

import base64
import json
import os
from typing import Any
from unittest import mock

import pytest


# --- key fixture ---------------------------------------------------------


# Pre-generated 2048-bit RSA key in PKCS#8 PEM form. Generated once
# with ``openssl genpkey -algorithm RSA -outform PEM -pkeyopt
# rsa_keygen_bits:2048 | openssl pkcs8 -topk8 -nocrypt`` and inlined
# here so the test suite has no openssl-at-runtime dependency.
_TEST_PEM = """-----BEGIN PRIVATE KEY-----
MIIEvgIBADANBgkqhkiG9w0BAQEFAASCBKgwggSkAgEAAoIBAQDQ7t5p2EzKzu0a
ZqV8O8nm5g54p0QHk8m8C2j8nq9NQVgXOoY8KHnZK+kZcRDr4mphCp1Wbk1A6BCl
4nF7tIIu3jZuLm3Zo0xrPhh8DcUskw6X9TG0qrEAlxbA9eIA8gJxqYfFsmO1F35y
1AdSGy3OD5dBwUVZ8e+ZUC5SUL2j5kPgsmAUF/qHEBcKbA7TYz5gN1pCYDQqEoey
LXHRm0LO0Z3aJj9zsa+RU3M9zMfo2OcCe0qBl4PJp6gOM7VBPm3ZeGSpoz8AAamz
1nL8q4OEqxAgyk6OzZaCsCMLR2cmJZsTjFhRrZmcl2N+rkXP2P3Co5VgycwUjcLU
PqfvP8/HAgMBAAECggEACDwHkP1Y3l7lEXJrqOEHpJfwHo3Ozk5GMzbXdsTDxxFL
nvtfTNi3XmnzFOTNH2vIY9wt+l9TmuGEbBcs9pi7Z0F2K9xZD6BC1g9N0K77NLh3
gn80V/3+wHj6ZHfJC93GldwbqkujQQPHkbVwAvqsdSkjV0X8WBO5x6jq2BCO/9hQ
ftEUhDqWtJDOuZAkjz5OFb2L7m3WaXdHvIUtNAfXvHvz/JogYsM8aGl0EBzeFEYZ
WNUjkqezNTP6mUFGbckh+nO8VkrhYRTewMrZmCxgfpvCxbWyL3FvJB4hpu0WDi/E
ePM6QwSAFNVpaTbpkBDOY1WCkqzWiM+1XmEphTNZuQKBgQDtsVZqsXfwlxd8/2zS
qftqaIw60Z5IyVwfMTd9MzVdpKfx4yh1d/aczsP6PR/aDLN9hUUOcWFY9YeR2ZQO
2Q3rXgmJTd9HZl5o5kRGNn83WLDi9KIqLN0RaibIY7/wdjyU3DZxRdmZkSCAEEXY
zMmEgL+8d/v7N7TC2NUxoyAxOQKBgQDg5WJqf4uddSnqaCCWLNvOhA3KhO0gE3km
nNxs+0/oZ+L8MdYqDfFx7QGT/PUQZsv7iLmCMObOIM3UkkF8R5G3CR1ZuO5Ndg2v
B4Q8OWmiPbZL6vbEkSwcfYxOEDQpJaENC8AwxLLAvkU5HENF6P3pVfRfWaUuG7y0
qiKQGwwBfwKBgQCa+L6Zs8u9NXFmsR9NV3M+Apsa3I2RvB6mFP1WjBUjZRYRJq8c
WdjkS5VPNYrV+9tAVAk2lFY/UALWNRgEvgK6jHXKHHWZBjsLh6Qkjz9oWkCBQRrV
4qF9G7Q4sX7v+5R3FZH+OYNHfg/ks7QQOBT+4hNiHnt5KQDl9DLN/M30CQKBgFvA
+aWfx6/cVqVB7v0aLNw1V0HD5MNCMlSlYRf+rXIaWZUMTOIVAJsm6vSbcMK+y0L4
DGYzdC2nFPNkdJh2cYUjI3i+/QwehZjOzqQyaZ0KaXJh/8fbg5tsxq5VsCxLiVDh
gcSAtwc9KFNV5cw98tKBQJfL3WTrm0VkbjFm+JX1AoGBAJxhqRr3rfqK5T+JdLLG
n5KQ6NEMnGzwwk2Z0iLZJDsBgcVuc1MKR8aZcCRnYNzqK9oCqMNXYqj3Y6dwdGNS
o3Bf2g7T0xqlNV8AT8++NPmTcKKQUBeMrXFNoCqELwy6XRdsmf2yvxX7B+8eCApk
F8ts5xVNZX/U7JZkdkwhcFKj
-----END PRIVATE KEY-----
"""


# We don't run the live signer against the real fixture (key above is
# random and Google would reject the JWT anyway); instead the tests
# stub urllib.request.urlopen.


def _service_account_blob(*, with_jwt: bool = True, with_token: bool = False):
    sa = {
        "client_email": "test-sa@example.iam.gserviceaccount.com",
        "token_uri": "https://oauth2.googleapis.com/token",
    }
    if with_jwt:
        sa["private_key"] = _TEST_PEM
    if with_token:
        sa["access_token"] = "pre-supplied-token"
    return json.dumps(sa)


def _set_env(*, sa_blob: str, spaces: str = "", port: str = "8090",
             account_id: str = "", api_base: str = ""):
    os.environ["GOOGLE_CHAT_SERVICE_ACCOUNT_JSON"] = sa_blob
    os.environ["GOOGLE_CHAT_SPACE_IDS"] = spaces
    os.environ["GOOGLE_CHAT_WEBHOOK_PORT"] = port
    os.environ["GOOGLE_CHAT_ACCOUNT_ID"] = account_id
    if api_base:
        os.environ["GOOGLE_CHAT_API_BASE"] = api_base
    else:
        os.environ.pop("GOOGLE_CHAT_API_BASE", None)


# Module import — must be after env preset for SCHEMA-only use, but
# the GoogleChatAdapter ctor reads env so we always set first.
from librefang.sidecar.adapters import google_chat as gc  # noqa: E402


# ---- env handling -------------------------------------------------------


def test_missing_sa_blob_raises():
    os.environ.pop("GOOGLE_CHAT_SERVICE_ACCOUNT_JSON", None)
    with pytest.raises(RuntimeError, match="GOOGLE_CHAT_SERVICE_ACCOUNT_JSON"):
        gc.GoogleChatAdapter()


def test_bad_json_raises_value_error():
    _set_env(sa_blob="not-json")
    with pytest.raises(ValueError, match="invalid service account key JSON"):
        gc.GoogleChatAdapter()


def test_missing_auth_paths_raises():
    # Neither client_email/private_key NOR access_token → can't auth.
    sa = json.dumps({"token_uri": "https://oauth2.googleapis.com/token"})
    _set_env(sa_blob=sa)
    with pytest.raises(RuntimeError, match="neither.*JWT.*nor access_token"):
        gc.GoogleChatAdapter()


def test_pre_supplied_token_path_constructs():
    _set_env(sa_blob=_service_account_blob(with_jwt=False, with_token=True))
    a = gc.GoogleChatAdapter()
    # Pre-supplied path seeds the cache so _get_access_token doesn't
    # need to sign anything.
    assert a._get_access_token() == "pre-supplied-token"


def test_jwt_path_construction_parses_pem():
    # A construction-time error here would mean the stdlib PEM/DER
    # parser broke on a valid PKCS#8 key.
    _set_env(sa_blob=_service_account_blob(with_jwt=True))
    a = gc.GoogleChatAdapter()
    assert a._rsa_key is not None
    n, d = a._rsa_key
    # 2048-bit modulus → 256-byte representation.
    assert n.bit_length() == 2048


def test_spaces_csv_parsed():
    _set_env(
        sa_blob=_service_account_blob(with_jwt=False, with_token=True),
        spaces="spaces/AAAA, spaces/BBBB ,  ,spaces/CCCC",
    )
    a = gc.GoogleChatAdapter()
    assert a._space_ids == ["spaces/AAAA", "spaces/BBBB", "spaces/CCCC"]


def test_account_id_propagated():
    _set_env(
        sa_blob=_service_account_blob(with_jwt=False, with_token=True),
        account_id="workspace-prod",
    )
    a = gc.GoogleChatAdapter()
    assert a.account_id == "workspace-prod"


def test_bad_webhook_port_raises():
    _set_env(
        sa_blob=_service_account_blob(with_jwt=False, with_token=True),
        port="not-an-int",
    )
    with pytest.raises(RuntimeError, match="must be an integer"):
        gc.GoogleChatAdapter()


# ---- token_uri SSRF guard ----------------------------------------------


def test_jwt_token_uri_must_be_google():
    sa = json.dumps({
        "client_email": "x@y.iam.gserviceaccount.com",
        "private_key": _TEST_PEM,
        "token_uri": "https://attacker.example/token",
    })
    _set_env(sa_blob=sa)
    a = gc.GoogleChatAdapter()
    with pytest.raises(RuntimeError, match="untrusted token_uri"):
        gc._exchange_jwt_for_token(a._sa.token_uri, "fake-jwt")


# ---- webhook parsing ---------------------------------------------------


def _msg_event(text="hello", space_name="spaces/AAAA",
               space_type="ROOM", thread_name=None):
    msg = {
        "type": "MESSAGE",
        "space": {"name": space_name, "type": space_type},
        "message": {
            "name": f"{space_name}/messages/M1",
            "text": text,
            "sender": {
                "displayName": "Alice",
                "name": "users/123",
            },
        },
    }
    if thread_name is not None:
        msg["message"]["thread"] = {"name": thread_name}
    return msg


def test_parse_webhook_event_plain_text():
    event = gc._parse_webhook_event(_msg_event(), [])
    assert event is not None
    params = event["params"]
    assert params["user_id"] == "spaces/AAAA"
    assert params["user_name"] == "Alice"
    assert params["content"] == {"Text": "hello"}
    assert params.get("is_group") is True
    assert params["metadata"]["sender_id"] == "users/123"
    assert params["metadata"]["channel_label"] == "google_chat"


def test_parse_webhook_event_slash_command():
    event = gc._parse_webhook_event(_msg_event(text="/start foo bar"), [])
    assert event["params"]["content"] == {
        "Command": {"name": "start", "args": ["foo", "bar"]},
    }


def test_parse_webhook_event_dm_marks_not_group():
    event = gc._parse_webhook_event(
        _msg_event(space_type="DM"), [],
    )
    # `is_group=False` is omitted from params (protocol.message:242).
    assert event["params"].get("is_group", False) is False


def test_parse_webhook_event_threaded():
    event = gc._parse_webhook_event(
        _msg_event(thread_name="spaces/AAAA/threads/T1"), [],
    )
    assert event["params"]["thread_id"] == "spaces/AAAA/threads/T1"


def test_parse_webhook_event_drops_non_message():
    payload = {"type": "ADDED_TO_SPACE", "space": {"name": "spaces/AAAA"}}
    assert gc._parse_webhook_event(payload, []) is None


def test_parse_webhook_event_drops_empty_text():
    msg = _msg_event(text="")
    assert gc._parse_webhook_event(msg, []) is None


def test_parse_webhook_event_filters_disallowed_space():
    msg = _msg_event(space_name="spaces/ZZZZ")
    allowed = ["spaces/AAAA"]
    assert gc._parse_webhook_event(msg, allowed) is None
    # Sanity: same msg with the space in the allowlist passes.
    msg2 = _msg_event(space_name="spaces/AAAA")
    assert gc._parse_webhook_event(msg2, allowed) is not None


def test_parse_webhook_event_empty_space_filter_allows_all():
    msg = _msg_event(space_name="spaces/SOMEWHERE")
    assert gc._parse_webhook_event(msg, []) is not None


# ---- split_message UTF-8 chunking -------------------------------------


def test_split_message_short_passes_through():
    assert gc._split_message("hello", gc.MAX_MESSAGE_LEN) == ["hello"]


def test_split_message_empty_passes_through():
    assert gc._split_message("", gc.MAX_MESSAGE_LEN) == [""]


def test_split_message_chunks_at_byte_boundary():
    text = "a" * (gc.MAX_MESSAGE_LEN + 100)
    chunks = gc._split_message(text, gc.MAX_MESSAGE_LEN)
    assert len(chunks) == 2
    assert sum(len(c) for c in chunks) == len(text)
    assert all(len(c.encode("utf-8")) <= gc.MAX_MESSAGE_LEN for c in chunks)


def test_split_message_respects_utf8_multibyte_boundary():
    # Every '文' is 3 UTF-8 bytes; a chunk must not split inside a char.
    text = "文" * 2000  # 6000 bytes total
    chunks = gc._split_message(text, gc.MAX_MESSAGE_LEN)
    assert "".join(chunks) == text
    for c in chunks:
        # Re-encoding must round-trip without UnicodeError.
        assert c.encode("utf-8").decode("utf-8") == c
        assert len(c.encode("utf-8")) <= gc.MAX_MESSAGE_LEN


# ---- send path with mocked urlopen ------------------------------------


def test_send_text_posts_to_messages_endpoint(monkeypatch):
    _set_env(
        sa_blob=_service_account_blob(with_jwt=False, with_token=True),
        api_base="https://chat.googleapis.com/v1",
    )
    a = gc.GoogleChatAdapter()

    captured: dict = {}

    def fake_urlopen(req, timeout=None):
        captured["url"] = req.full_url
        captured["headers"] = dict(req.headers)
        captured["body"] = req.data.decode("utf-8") if req.data else ""

        class _R:
            def read(self):
                return b""

            def __enter__(self):
                return self

            def __exit__(self, *exc):
                return False

        return _R()

    monkeypatch.setattr(gc.urllib.request, "urlopen", fake_urlopen)
    a._send_text("spaces/AAAA", "hi")

    assert captured["url"] == "https://chat.googleapis.com/v1/spaces/AAAA/messages"
    # The header capitalisation is normalised differently across Python
    # versions; check case-insensitively.
    assert any(
        k.lower() == "authorization" and v == "Bearer pre-supplied-token"
        for k, v in captured["headers"].items()
    )
    assert json.loads(captured["body"]) == {"text": "hi"}


def test_send_text_chunks_oversize_payload(monkeypatch):
    _set_env(
        sa_blob=_service_account_blob(with_jwt=False, with_token=True),
    )
    a = gc.GoogleChatAdapter()
    payloads = []

    def fake_urlopen(req, timeout=None):
        payloads.append(json.loads(req.data.decode("utf-8")))

        class _R:
            def read(self):
                return b""

            def __enter__(self):
                return self

            def __exit__(self, *exc):
                return False

        return _R()

    monkeypatch.setattr(gc.urllib.request, "urlopen", fake_urlopen)
    text = "a" * (gc.MAX_MESSAGE_LEN + 200)
    a._send_text("spaces/AAAA", text)

    assert len(payloads) == 2
    assert sum(len(p["text"]) for p in payloads) == len(text)


def test_send_text_401_clears_token_cache(monkeypatch):
    _set_env(
        sa_blob=_service_account_blob(with_jwt=False, with_token=True),
    )
    a = gc.GoogleChatAdapter()
    assert a._token_cache.get() == "pre-supplied-token"

    def fake_urlopen(req, timeout=None):
        import urllib.error

        raise urllib.error.HTTPError(
            req.full_url, 401, "Unauthorized",
            hdrs={}, fp=None,
        )

    monkeypatch.setattr(gc.urllib.request, "urlopen", fake_urlopen)
    with pytest.raises(RuntimeError, match="Google Chat API error 401"):
        a._send_text("spaces/AAAA", "hi")
    # 401 should have cleared the cache so the next send re-runs auth.
    assert a._token_cache.get() is None


# ---- JWT signing end-to-end against the test PEM ----------------------


def test_jwt_signing_round_trip_against_test_pem():
    # Build a JWT and decode its three parts; sanity-check structure.
    n, d = gc._parse_pkcs8_rsa_private_key(_TEST_PEM)
    claims = {"iss": "x@y", "exp": 0, "iat": 0}
    jwt = gc._sign_rs256_jwt(claims, n, d)
    header_b64, claims_b64, sig_b64 = jwt.split(".")

    def _decode(s):
        # Add the padding the JWT spec strips.
        padded = s + "=" * (-len(s) % 4)
        return base64.urlsafe_b64decode(padded)

    header = json.loads(_decode(header_b64))
    assert header == {"alg": "RS256", "typ": "JWT"}
    claims_decoded = json.loads(_decode(claims_b64))
    assert claims_decoded == claims
    # Signature byte length should match RSA modulus byte length.
    assert len(_decode(sig_b64)) == (n.bit_length() + 7) // 8


# ---- schema sanity ----------------------------------------------------


def test_schema_declares_required_service_account_field():
    schema = gc.GoogleChatAdapter.SCHEMA.to_dict()
    assert schema["name"] == "google_chat"
    sa_field = next(
        f for f in schema["fields"]
        if f["key"] == "GOOGLE_CHAT_SERVICE_ACCOUNT_JSON"
    )
    assert sa_field["required"] is True
    assert sa_field["type"] == "secret"


def test_schema_account_id_is_advanced():
    schema = gc.GoogleChatAdapter.SCHEMA.to_dict()
    aid = next(
        f for f in schema["fields"] if f["key"] == "GOOGLE_CHAT_ACCOUNT_ID"
    )
    assert aid["advanced"] is True
    assert aid["required"] is False


# ---- on_send dispatch (against the real Send dataclass) ---------------


def _send_cmd(channel_id="spaces/AAAA", text="hi", content=None,
              thread_id=None, user=None):
    from librefang.sidecar.protocol import Send
    return Send(channel_id, text, content, thread_id, user or {})


@pytest.mark.asyncio
async def test_on_send_basic_uses_channel_id(monkeypatch):
    """The framework passes a `Send` whose `channel_id` is the
    `user_id` of the inbound message event — for google_chat that
    means `spaces/AAAA`. This test pins that `on_send` reads
    `cmd.channel_id` and not some other field. A regression where
    `on_send` reaches for `cmd.user_id` (no such attribute on
    `Send`) would AttributeError on the first send."""
    _set_env(sa_blob=_service_account_blob(with_jwt=False, with_token=True))
    a = gc.GoogleChatAdapter()
    sent: list = []

    def fake_urlopen(req, timeout=None):
        sent.append((req.full_url, json.loads(req.data.decode("utf-8"))))

        class _R:
            def read(self):
                return b""

            def __enter__(self):
                return self

            def __exit__(self, *exc):
                return False

        return _R()

    monkeypatch.setattr(gc.urllib.request, "urlopen", fake_urlopen)
    await a.on_send(_send_cmd(channel_id="spaces/AAAA", text="hello"))
    assert len(sent) == 1
    url, body = sent[0]
    assert "/spaces/AAAA/messages" in url
    assert body == {"text": "hello"}


@pytest.mark.asyncio
async def test_on_send_falls_back_to_user_platform_id(monkeypatch):
    """When `channel_id` is empty, `on_send` falls back to
    `cmd.user.platform_id`. Mirrors teams.py / whatsapp.py
    behaviour so daemons that address by user still work."""
    _set_env(sa_blob=_service_account_blob(with_jwt=False, with_token=True))
    a = gc.GoogleChatAdapter()
    captured: list = []
    monkeypatch.setattr(
        gc.urllib.request, "urlopen",
        lambda req, timeout=None: (
            captured.append(req.full_url),
            type("R", (), {
                "read": lambda self: b"",
                "__enter__": lambda self: self,
                "__exit__": lambda self, *a: False,
            })(),
        )[1],
    )
    await a.on_send(_send_cmd(
        channel_id="", text="hi",
        user={"platform_id": "spaces/BBBB"},
    ))
    assert len(captured) == 1
    assert "/spaces/BBBB/messages" in captured[0]


@pytest.mark.asyncio
async def test_on_send_empty_channel_id_drops(monkeypatch):
    _set_env(sa_blob=_service_account_blob(with_jwt=False, with_token=True))
    a = gc.GoogleChatAdapter()
    calls: list = []
    monkeypatch.setattr(
        gc.urllib.request, "urlopen",
        lambda req, timeout=None: (calls.append(req.full_url), None)[1],
    )
    await a.on_send(_send_cmd(channel_id="", user={}))
    assert calls == [], "empty channel_id + empty user must drop without HTTP"


@pytest.mark.asyncio
async def test_on_send_non_space_channel_id_drops(monkeypatch):
    """`channel_id` that doesn't start with `spaces/` is rejected
    (defense against a daemon mistakenly routing a non-google-chat
    conversation here)."""
    _set_env(sa_blob=_service_account_blob(with_jwt=False, with_token=True))
    a = gc.GoogleChatAdapter()
    calls: list = []
    monkeypatch.setattr(
        gc.urllib.request, "urlopen",
        lambda req, timeout=None: (calls.append(req.full_url), None)[1],
    )
    await a.on_send(_send_cmd(channel_id="C12345", text="hi"))
    assert calls == [], "non-space channel_id must drop without HTTP"


@pytest.mark.asyncio
async def test_on_send_empty_text_drops(monkeypatch):
    _set_env(sa_blob=_service_account_blob(with_jwt=False, with_token=True))
    a = gc.GoogleChatAdapter()
    calls: list = []
    monkeypatch.setattr(
        gc.urllib.request, "urlopen",
        lambda req, timeout=None: (calls.append(req.full_url), None)[1],
    )
    await a.on_send(_send_cmd(text=""))
    assert calls == []
