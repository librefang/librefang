"""Tests for librefang.sidecar.adapters.teams.

Deterministic, no network — urllib monkey-patched via _http_request.
"""
from __future__ import annotations

import base64
import hashlib
import hmac
import json
import os

import pytest

# Ensure env is primed before module import (TeamsAdapter raises
# SystemExit at construction if required vars missing).
os.environ.setdefault("TEAMS_APP_ID", "app-id-fixture")
os.environ.setdefault("TEAMS_APP_PASSWORD", "pw-fixture")
from librefang.sidecar.adapters import teams as tm  # noqa: E402


def _adapter(**env):
    defaults = {
        "TEAMS_APP_ID": "app-id-fixture",
        "TEAMS_APP_PASSWORD": "pw-fixture",
        "TEAMS_SECURITY_TOKEN": "",
        "TEAMS_ALLOWED_TENANTS": "",
        "TEAMS_ACCOUNT_ID": "",
        "TEAMS_WEBHOOK_PORT": "",
        "TEAMS_WEBHOOK_PATH": "",
        "TEAMS_BIND_HOST": "",
        "TEAMS_OAUTH_TOKEN_URL": "",
        "TEAMS_SERVICE_URL": "",
    }
    for k, v in defaults.items():
        os.environ[k] = env.get(k, v)
    a = tm.TeamsAdapter()
    # Pre-prime the token cache so tests don't need to mock OAuth
    # for every send. Tests that exercise the OAuth path clear this
    # explicitly.
    a._cached_token = ("test_bearer_token", 9_999_999_999.0)
    return a


# ---- env handling ---------------------------------------------------


def test_default_env_construction():
    a = _adapter()
    assert a.app_id == "app-id-fixture"
    assert a.app_password == "pw-fixture"
    assert a.security_token_key is None  # empty → disabled
    assert a.allowed_tenants == []
    assert a.account_id is None
    assert a.webhook_port == tm.DEFAULT_WEBHOOK_PORT
    assert a.webhook_path == tm.DEFAULT_WEBHOOK_PATH
    assert a.bind_host == tm.DEFAULT_BIND_HOST
    assert a.oauth_token_url == tm.DEFAULT_OAUTH_TOKEN_URL
    assert a.default_service_url == tm.DEFAULT_SERVICE_URL


def test_missing_app_id_raises():
    os.environ["TEAMS_APP_ID"] = ""
    os.environ["TEAMS_APP_PASSWORD"] = "pw"
    with pytest.raises(SystemExit):
        tm.TeamsAdapter()


def test_missing_app_password_raises():
    os.environ["TEAMS_APP_ID"] = "id"
    os.environ["TEAMS_APP_PASSWORD"] = ""
    with pytest.raises(SystemExit):
        tm.TeamsAdapter()


def test_allowed_tenants_csv():
    a = _adapter(TEAMS_ALLOWED_TENANTS="tenant-a, tenant-b , , tenant-c")
    assert a.allowed_tenants == ["tenant-a", "tenant-b", "tenant-c"]


def test_account_id_passthrough():
    a = _adapter(TEAMS_ACCOUNT_ID="production")
    assert a.account_id == "production"


def test_security_token_base64_decoded():
    raw_key = b"\x01\x02\x03\x04\x05"
    b64 = base64.b64encode(raw_key).decode("ascii")
    a = _adapter(TEAMS_SECURITY_TOKEN=b64)
    assert a.security_token_key == raw_key


def test_security_token_invalid_base64_disables_verify():
    """Mirrors teams.rs:129-136: invalid base64 = WARN + disable."""
    a = _adapter(TEAMS_SECURITY_TOKEN="!!! not base64 !!!")
    assert a.security_token_key is None


def test_webhook_port_override():
    a = _adapter(TEAMS_WEBHOOK_PORT="8088")
    assert a.webhook_port == 8088


def test_webhook_port_garbage_uses_default():
    a = _adapter(TEAMS_WEBHOOK_PORT="not-a-port")
    assert a.webhook_port == tm.DEFAULT_WEBHOOK_PORT


def test_webhook_path_normalized():
    a = _adapter(TEAMS_WEBHOOK_PATH="messages")
    assert a.webhook_path == "/messages"


def test_oauth_url_override():
    a = _adapter(TEAMS_OAUTH_TOKEN_URL="http://localhost:8000/token")
    assert a.oauth_token_url == "http://localhost:8000/token"


# ---- HMAC verification ---------------------------------------------


def test_verify_signature_valid():
    key = b"\x10\x20\x30\x40"
    body = b'{"type":"message","text":"hi"}'
    digest = hmac.new(key, body, hashlib.sha256).digest()
    auth = "HMAC " + base64.b64encode(digest).decode("ascii")
    assert tm.verify_teams_signature(key, body, auth) is True


def test_verify_signature_wrong_key_rejected():
    body = b"x"
    digest = hmac.new(b"correct-key", body, hashlib.sha256).digest()
    auth = "HMAC " + base64.b64encode(digest).decode("ascii")
    assert tm.verify_teams_signature(b"different-key", body, auth) is False


def test_verify_signature_wrong_body_rejected():
    key = b"k"
    digest = hmac.new(key, b"original", hashlib.sha256).digest()
    auth = "HMAC " + base64.b64encode(digest).decode("ascii")
    assert tm.verify_teams_signature(key, b"tampered", auth) is False


def test_verify_signature_missing_header_rejected():
    assert tm.verify_teams_signature(b"k", b"body", None) is False


def test_verify_signature_empty_header_rejected():
    assert tm.verify_teams_signature(b"k", b"body", "") is False


def test_verify_signature_wrong_prefix_rejected():
    """Header must start with `HMAC ` — anything else (Basic, Bearer,
    no scheme) is rejected. Mirrors teams.rs:38-41."""
    key = b"k"
    digest = hmac.new(key, b"body", hashlib.sha256).digest()
    raw = base64.b64encode(digest).decode("ascii")
    assert tm.verify_teams_signature(key, b"body", raw) is False
    assert tm.verify_teams_signature(key, b"body", f"Basic {raw}") is False
    assert tm.verify_teams_signature(key, b"body", f"Bearer {raw}") is False


def test_verify_signature_non_base64_rejected():
    assert tm.verify_teams_signature(b"k", b"body", "HMAC !!!") is False


def test_verify_signature_empty_base64_rejected():
    assert tm.verify_teams_signature(b"k", b"body", "HMAC ") is False


# ---- parse_teams_activity -------------------------------------------


def _msg_activity(
    *,
    activity_id="msg_42",
    from_id="alice-id",
    from_name="Alice",
    text="hello world",
    conversation_id="conv-1",
    is_group=False,
    tenant_id=None,
    service_url="https://smba.region.example.com/teams/",
):
    activity = {
        "type": "message",
        "id": activity_id,
        "from": {"id": from_id, "name": from_name},
        "text": text,
        "conversation": {"id": conversation_id, "isGroup": is_group},
        "serviceUrl": service_url,
    }
    if tenant_id:
        activity["channelData"] = {"tenant": {"id": tenant_id}}
    return activity


def test_parse_basic_message():
    ev = tm.parse_teams_activity(
        _msg_activity(), app_id="bot-app-id", allowed_tenants=[],
    )
    assert ev is not None
    params = ev["params"]
    assert params["user_id"] == "conv-1"
    assert params["user_name"] == "Alice"
    assert params["channel_id"] == "conv-1"
    assert params["message_id"] == "msg_42"
    assert params["content"]["Text"] == "hello world"
    meta = params["metadata"]
    assert meta["serviceUrl"] == "https://smba.region.example.com/teams/"
    assert "is_group" not in meta


def test_parse_self_skip_bot_origin():
    """`from.id == app_id` is the bot's own message bouncing back."""
    msg = _msg_activity(from_id="bot-app-id")
    assert tm.parse_teams_activity(msg, app_id="bot-app-id", allowed_tenants=[]) is None


def test_parse_non_message_type_dropped():
    msg = _msg_activity()
    msg["type"] = "conversationUpdate"
    assert tm.parse_teams_activity(msg, app_id="x", allowed_tenants=[]) is None


def test_parse_missing_from_dropped():
    msg = _msg_activity()
    del msg["from"]
    assert tm.parse_teams_activity(msg, app_id="x", allowed_tenants=[]) is None


def test_parse_empty_text_dropped():
    assert tm.parse_teams_activity(
        _msg_activity(text=""), app_id="x", allowed_tenants=[],
    ) is None


def test_parse_tenant_allowed():
    ev = tm.parse_teams_activity(
        _msg_activity(tenant_id="tenant-allowed"),
        app_id="x",
        allowed_tenants=["tenant-allowed", "tenant-other"],
    )
    assert ev is not None


def test_parse_tenant_rejected():
    msg = _msg_activity(tenant_id="tenant-evil")
    ev = tm.parse_teams_activity(
        msg, app_id="x", allowed_tenants=["tenant-good"],
    )
    assert ev is None


def test_parse_tenant_missing_with_allowlist_rejected():
    """An activity with no channelData.tenant.id but a non-empty
    allowlist must be dropped (the tenant_id is "")."""
    msg = _msg_activity()  # no channelData
    ev = tm.parse_teams_activity(
        msg, app_id="x", allowed_tenants=["tenant-good"],
    )
    assert ev is None


def test_parse_group_conversation():
    ev = tm.parse_teams_activity(
        _msg_activity(is_group=True), app_id="x", allowed_tenants=[],
    )
    assert ev["params"]["metadata"]["is_group"] is True


def test_parse_command_routing():
    ev = tm.parse_teams_activity(
        _msg_activity(text="/help me please"),
        app_id="x", allowed_tenants=[],
    )
    content = ev["params"]["content"]
    assert "Command" in content
    assert content["Command"]["name"] == "help"
    assert content["Command"]["args"] == ["me", "please"]


def test_parse_command_no_args():
    ev = tm.parse_teams_activity(
        _msg_activity(text="/ping"),
        app_id="x", allowed_tenants=[],
    )
    assert ev["params"]["content"]["Command"]["name"] == "ping"
    assert ev["params"]["content"]["Command"]["args"] == []


def test_parse_account_id_injection():
    ev = tm.parse_teams_activity(
        _msg_activity(), app_id="x", allowed_tenants=[],
        account_id="prod",
    )
    assert ev["params"]["metadata"]["account_id"] == "prod"


def test_parse_no_account_id_omits_field():
    ev = tm.parse_teams_activity(
        _msg_activity(), app_id="x", allowed_tenants=[],
    )
    assert "account_id" not in ev["params"]["metadata"]


def test_parse_non_dict_returns_none():
    assert tm.parse_teams_activity(None, app_id="x", allowed_tenants=[]) is None
    assert tm.parse_teams_activity("nope", app_id="x", allowed_tenants=[]) is None
    assert tm.parse_teams_activity(42, app_id="x", allowed_tenants=[]) is None


# ---- _handle_webhook_body integration -----------------------------


def _sign(key: bytes, body: bytes) -> str:
    digest = hmac.new(key, body, hashlib.sha256).digest()
    return "HMAC " + base64.b64encode(digest).decode("ascii")


def test_webhook_body_emits_message():
    """End-to-end: valid signature + valid activity → emit one event."""
    raw_key = b"\x01\x02\x03"
    b64_key = base64.b64encode(raw_key).decode("ascii")
    a = _adapter(TEAMS_SECURITY_TOKEN=b64_key)
    payload = _msg_activity()
    body = json.dumps(payload).encode("utf-8")
    auth = _sign(raw_key, body)
    emitted: list = []
    status = a._handle_webhook_body(body, auth, lambda ev: emitted.append(ev))
    assert status == 200
    assert len(emitted) == 1


def test_webhook_body_bad_signature_rejected_401():
    raw_key = b"correct"
    b64_key = base64.b64encode(raw_key).decode("ascii")
    a = _adapter(TEAMS_SECURITY_TOKEN=b64_key)
    body = b'{"type":"message"}'
    auth = "HMAC " + base64.b64encode(b"x" * 32).decode("ascii")
    emitted: list = []
    status = a._handle_webhook_body(body, auth, lambda ev: emitted.append(ev))
    assert status == 401
    assert emitted == []


def test_webhook_body_missing_auth_rejected_400():
    raw_key = b"k"
    b64_key = base64.b64encode(raw_key).decode("ascii")
    a = _adapter(TEAMS_SECURITY_TOKEN=b64_key)
    status = a._handle_webhook_body(b"{}", None, lambda _: None)
    assert status == 400


def test_webhook_body_verification_disabled_accepts_any():
    """When TEAMS_SECURITY_TOKEN is empty, signature is skipped —
    the operator was warned at startup."""
    a = _adapter()  # no security token
    payload = _msg_activity()
    body = json.dumps(payload).encode("utf-8")
    emitted: list = []
    # Any (or no) Authorization header is accepted.
    status = a._handle_webhook_body(body, "garbage", lambda ev: emitted.append(ev))
    assert status == 200
    assert len(emitted) == 1


def test_webhook_body_malformed_json_400():
    a = _adapter()
    status = a._handle_webhook_body(b"{not-json", None, lambda _: None)
    assert status == 400


def test_webhook_body_dedupes_repeated_activity_id():
    """Improvement #2: Bot Framework retries; the sidecar must
    dedupe on Activity ID."""
    a = _adapter()
    payload = _msg_activity(activity_id="dup-1")
    body = json.dumps(payload).encode("utf-8")
    emitted: list = []
    s1 = a._handle_webhook_body(body, None, lambda ev: emitted.append(ev))
    s2 = a._handle_webhook_body(body, None, lambda ev: emitted.append(ev))
    assert s1 == 200 and s2 == 200
    assert len(emitted) == 1


def test_webhook_body_does_not_dedupe_dropped_activities():
    """A dropped activity (empty text, wrong type, disallowed
    tenant) must NOT mark its Activity ID as seen — otherwise a
    legitimate retry that lands the parseable payload would be
    rejected as a duplicate. Bot Framework retries on non-2xx /
    timeout, so the second delivery may arrive with the fields
    the parse path needs."""
    a = _adapter()
    # First delivery: same activity_id, but text is empty → parse
    # drops, we should NOT mark seen.
    bad = _msg_activity(activity_id="retry-1", text="")
    body1 = json.dumps(bad).encode("utf-8")
    emitted: list = []
    a._handle_webhook_body(body1, None, lambda ev: emitted.append(ev))
    assert emitted == []
    # Second delivery: same activity_id, with text. Must emit.
    good = _msg_activity(activity_id="retry-1", text="now with text")
    body2 = json.dumps(good).encode("utf-8")
    a._handle_webhook_body(body2, None, lambda ev: emitted.append(ev))
    assert len(emitted) == 1
    assert emitted[0]["params"]["content"]["Text"] == "now with text"


def test_webhook_body_caches_service_url_per_conversation():
    """Improvement #1: the per-conversation serviceUrl gets cached
    so outbound replies hit the right region instead of the
    DEFAULT_SERVICE_URL."""
    a = _adapter()
    payload = _msg_activity(
        conversation_id="conv-region-1",
        service_url="https://smba.region.example.com/teams/",
    )
    body = json.dumps(payload).encode("utf-8")
    a._handle_webhook_body(body, None, lambda _: None)
    assert a._service_url_for("conv-region-1") == "https://smba.region.example.com/teams/"


def test_webhook_body_unknown_conversation_falls_back_to_default():
    a = _adapter()
    assert a._service_url_for("never-seen") == tm.DEFAULT_SERVICE_URL


# ---- _send_text via mocked http_request ----------------------------


def test_send_text_basic(monkeypatch):
    captured: list = []

    def _fake_http(url, **kw):
        captured.append((url, json.loads(kw["body"].decode("utf-8"))))
        return (200, {"id": "act-out-1"}, b"", {})

    monkeypatch.setattr(tm, "_http_request", _fake_http)
    a = _adapter()
    a._send_text("conv-1", "hello")
    assert len(captured) == 1
    url, body = captured[0]
    assert "/v3/conversations/conv-1/activities" in url
    assert body == {"type": "message", "text": "hello"}


def test_send_text_uses_cached_service_url(monkeypatch):
    """Improvement #1 in action: send hits the per-conversation
    service_url, not DEFAULT."""
    captured: list = []
    monkeypatch.setattr(
        tm, "_http_request",
        lambda url, **kw: (captured.append(url), (200, {}, b"", {}))[1],
    )
    a = _adapter()
    a._stash_service_url("conv-region-1", "https://smba.region.example.com/teams/")
    a._send_text("conv-region-1", "hi")
    assert captured[0].startswith("https://smba.region.example.com/teams/v3/")


def test_send_text_chunks_long_message(monkeypatch):
    monkeypatch.setattr(tm, "MAX_MESSAGE_LEN", 5)
    calls: list = []
    monkeypatch.setattr(
        tm, "_http_request",
        lambda url, **kw: (
            calls.append(json.loads(kw["body"].decode("utf-8"))),
            (200, {}, b"", {}),
        )[1],
    )
    a = _adapter()
    a._send_text("conv-1", "abcdefghijk")  # 11 chars at limit=5 → 3 chunks
    assert len(calls) >= 2


def test_send_text_empty_drops(monkeypatch):
    calls: list = []
    monkeypatch.setattr(
        tm, "_http_request",
        lambda url, **kw: (calls.append(url), (200, {}, b"", {}))[1],
    )
    a = _adapter()
    a._send_text("conv-1", "")
    assert calls == []


def test_send_text_empty_conversation_drops(monkeypatch):
    calls: list = []
    monkeypatch.setattr(
        tm, "_http_request",
        lambda url, **kw: (calls.append(url), (200, {}, b"", {}))[1],
    )
    a = _adapter()
    a._send_text("", "hi")
    assert calls == []


def test_send_text_429_retries_once(monkeypatch):
    """First 429 sleeps + retries; second response succeeds."""
    responses = [
        (429, None, b"slow down", {"retry-after": "0"}),
        (200, {}, b"", {}),
    ]
    calls: list = []

    def _fake_http(url, **kw):
        calls.append(url)
        return responses.pop(0)

    monkeypatch.setattr(tm, "_http_request", _fake_http)
    monkeypatch.setattr(tm, "_parse_retry_after", lambda h, **kw: 0.0)
    a = _adapter()
    a._send_text("conv-1", "hi")
    assert len(calls) == 2


def test_send_text_5xx_warns_continues(monkeypatch):
    """Match Rust at teams.rs:254-258 — non-2xx warns but the
    overall send call doesn't raise (per-chunk fail-open)."""
    monkeypatch.setattr(
        tm, "_http_request",
        lambda url, **kw: (500, None, b"server boom", {}),
    )
    a = _adapter()
    # Must not raise.
    a._send_text("conv-1", "hi")


# ---- OAuth token cache ---------------------------------------------


def test_get_token_caches(monkeypatch):
    calls: list = []

    def _fake_http(url, **kw):
        calls.append(url)
        return (200, {"access_token": "fresh-tok", "expires_in": 3600}, b"", {})

    monkeypatch.setattr(tm, "_http_request", _fake_http)
    a = _adapter()
    a._cached_token = None
    t1 = a._get_token()
    t2 = a._get_token()
    assert t1 == t2 == "fresh-tok"
    assert len(calls) == 1  # second call hit the cache


def test_get_token_raises_on_non_2xx(monkeypatch):
    monkeypatch.setattr(
        tm, "_http_request",
        lambda url, **kw: (401, None, b"unauthorized", {}),
    )
    a = _adapter()
    a._cached_token = None
    with pytest.raises(RuntimeError, match="OAuth2 token error"):
        a._get_token()


def test_get_token_raises_on_missing_token(monkeypatch):
    """OAuth response without an access_token field is an error."""
    monkeypatch.setattr(
        tm, "_http_request",
        lambda url, **kw: (200, {"expires_in": 3600}, b"", {}),
    )
    a = _adapter()
    a._cached_token = None
    with pytest.raises(RuntimeError, match="missing access_token"):
        a._get_token()


def test_get_token_default_ttl_on_missing_expires_in(monkeypatch):
    monkeypatch.setattr(
        tm, "_http_request",
        lambda url, **kw: (200, {"access_token": "t"}, b"", {}),
    )
    a = _adapter()
    a._cached_token = None
    assert a._get_token() == "t"  # falls back to 3600 - 300 buffer


# ---- on_send dispatch ----------------------------------------------


def _send_cmd(channel_id="conv-1", text="hi", content=None,
              thread_id=None, user=None):
    from librefang.sidecar.protocol import Send
    return Send(channel_id, text, content, thread_id, user or {})


@pytest.mark.asyncio
async def test_on_send_basic(monkeypatch):
    sent: list = []
    monkeypatch.setattr(
        tm, "_http_request",
        lambda url, **kw: (
            sent.append(json.loads(kw["body"].decode("utf-8"))),
            (200, {}, b"", {}),
        )[1],
    )
    a = _adapter()
    await a.on_send(_send_cmd(text="hello", content={"Text": "hello"}))
    assert sent[0] == {"type": "message", "text": "hello"}


@pytest.mark.asyncio
async def test_on_send_fallback_to_user_platform_id(monkeypatch):
    sent: list = []
    captured_url = []
    monkeypatch.setattr(
        tm, "_http_request",
        lambda url, **kw: (
            captured_url.append(url),
            sent.append(json.loads(kw["body"].decode("utf-8"))),
            (200, {}, b"", {}),
        )[2],
    )
    a = _adapter()
    await a.on_send(_send_cmd(
        channel_id="", text="hi", content={"Text": "hi"},
        user={"platform_id": "fallback-conv"},
    ))
    assert "/conversations/fallback-conv/" in captured_url[0]


@pytest.mark.asyncio
async def test_on_send_empty_conversation_drops(monkeypatch):
    calls: list = []
    monkeypatch.setattr(
        tm, "_http_request",
        lambda url, **kw: (calls.append(url), (200, {}, b"", {}))[1],
    )
    a = _adapter()
    await a.on_send(_send_cmd(channel_id="", user={}))
    assert calls == []


@pytest.mark.asyncio
async def test_on_send_unsupported_content_placeholder(monkeypatch):
    sent: list = []
    monkeypatch.setattr(
        tm, "_http_request",
        lambda url, **kw: (
            sent.append(json.loads(kw["body"].decode("utf-8"))),
            (200, {}, b"", {}),
        )[1],
    )
    a = _adapter()
    await a.on_send(_send_cmd(
        text="", content={"Image": {"url": "https://x"}},
    ))
    assert sent[0]["text"] == "(Unsupported content type)"


@pytest.mark.asyncio
async def test_on_send_empty_text_drops(monkeypatch):
    calls: list = []
    monkeypatch.setattr(
        tm, "_http_request",
        lambda url, **kw: (calls.append(url), (200, {}, b"", {}))[1],
    )
    a = _adapter()
    await a.on_send(_send_cmd(text="", content={"Text": ""}))
    assert calls == []


# ---- typing --------------------------------------------------------


def test_send_typing_basic(monkeypatch):
    sent: list = []
    monkeypatch.setattr(
        tm, "_http_request",
        lambda url, **kw: (
            sent.append(json.loads(kw["body"].decode("utf-8"))),
            (200, {}, b"", {}),
        )[1],
    )
    a = _adapter()
    a._send_typing("conv-1")
    assert sent[0] == {"type": "typing"}


def test_send_typing_swallows_errors(monkeypatch):
    """Typing is best-effort — a 500 must not raise."""
    monkeypatch.setattr(
        tm, "_http_request",
        lambda url, **kw: (500, None, b"", {}),
    )
    a = _adapter()
    a._send_typing("conv-1")  # Must not raise.


def test_send_typing_empty_conv_skipped(monkeypatch):
    calls: list = []
    monkeypatch.setattr(
        tm, "_http_request",
        lambda url, **kw: (calls.append(url), (200, {}, b"", {}))[1],
    )
    a = _adapter()
    a._send_typing("")
    assert calls == []


@pytest.mark.asyncio
async def test_on_command_routes_typing_cmd(monkeypatch):
    sent: list = []
    monkeypatch.setattr(
        tm, "_http_request",
        lambda url, **kw: (
            sent.append(json.loads(kw["body"].decode("utf-8"))),
            (200, {}, b"", {}),
        )[1],
    )
    a = _adapter()
    from librefang.sidecar.protocol import TypingCmd
    await a.on_command(TypingCmd(channel_id="conv-1"))
    assert sent[0] == {"type": "typing"}


@pytest.mark.asyncio
async def test_on_command_typing_empty_channel_drops(monkeypatch):
    calls: list = []
    monkeypatch.setattr(
        tm, "_http_request",
        lambda url, **kw: (calls.append(url), (200, {}, b"", {}))[1],
    )
    a = _adapter()
    from librefang.sidecar.protocol import TypingCmd
    await a.on_command(TypingCmd(channel_id=""))
    assert calls == []


# ---- schema + capabilities -----------------------------------------


def test_schema_exposes_required_envs():
    schema = tm.TeamsAdapter.SCHEMA.to_dict()
    keys = {f["key"] for f in schema["fields"]}
    expected = {
        "TEAMS_APP_ID",
        "TEAMS_APP_PASSWORD",
        "TEAMS_SECURITY_TOKEN",
        "TEAMS_WEBHOOK_PORT",
        "TEAMS_ALLOWED_TENANTS",
        "TEAMS_ACCOUNT_ID",
    }
    assert expected.issubset(keys)
    secrets = {f["key"] for f in schema["fields"] if f["type"] == "secret"}
    # App password + security token go to secrets.env, not config.toml.
    assert "TEAMS_APP_PASSWORD" in secrets
    assert "TEAMS_SECURITY_TOKEN" in secrets


def test_capabilities_declares_typing():
    assert "typing" in tm.TeamsAdapter.capabilities
