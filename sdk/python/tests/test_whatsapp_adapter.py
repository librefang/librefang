"""Tests for librefang.sidecar.adapters.whatsapp.

Deterministic, no network — urllib monkeypatched via _http_request.
"""
from __future__ import annotations

import hashlib
import hmac
import json
import os

import pytest

# Prime env so the module import doesn't SystemExit on the Cloud API
# required-vars check.
os.environ.setdefault("WHATSAPP_PHONE_NUMBER_ID", "phone-id-fixture")
os.environ.setdefault("WHATSAPP_ACCESS_TOKEN", "tok-fixture")
from librefang.sidecar.adapters import whatsapp as wa  # noqa: E402


def _cloud_adapter(**env):
    defaults = {
        "WHATSAPP_PHONE_NUMBER_ID": "phone-id-fixture",
        "WHATSAPP_ACCESS_TOKEN": "tok-fixture",
        "WHATSAPP_VERIFY_TOKEN": "verify-tok",
        "WHATSAPP_APP_SECRET": "",
        "WHATSAPP_GATEWAY_URL": "",
        "WHATSAPP_WEBHOOK_PORT": "",
        "WHATSAPP_WEBHOOK_PATH": "",
        "WHATSAPP_BIND_HOST": "",
        "WHATSAPP_ALLOWED_USERS": "",
        "WHATSAPP_ACCOUNT_ID": "",
        "WHATSAPP_BOT_PHONE": "",
        "WHATSAPP_BOT_NAME": "",
        "WHATSAPP_DM_POLICY": "",
        "WHATSAPP_GROUP_POLICY": "",
        "WHATSAPP_CLOUD_API_BASE": "",
    }
    for k, v in defaults.items():
        os.environ[k] = env.get(k, v)
    return wa.WhatsAppAdapter()


def _gateway_adapter(**env):
    defaults = {
        "WHATSAPP_PHONE_NUMBER_ID": "",
        "WHATSAPP_ACCESS_TOKEN": "",
        "WHATSAPP_VERIFY_TOKEN": "",
        "WHATSAPP_APP_SECRET": "",
        "WHATSAPP_GATEWAY_URL": "http://localhost:3009",
        "WHATSAPP_WEBHOOK_PORT": "",
        "WHATSAPP_WEBHOOK_PATH": "",
        "WHATSAPP_BIND_HOST": "",
        "WHATSAPP_ALLOWED_USERS": "",
        "WHATSAPP_ACCOUNT_ID": "",
        "WHATSAPP_BOT_PHONE": "",
        "WHATSAPP_BOT_NAME": "",
        "WHATSAPP_DM_POLICY": "",
        "WHATSAPP_GROUP_POLICY": "",
    }
    for k, v in defaults.items():
        os.environ[k] = env.get(k, v)
    return wa.WhatsAppAdapter()


# ---- env handling ---------------------------------------------------


def test_cloud_default_env_construction():
    a = _cloud_adapter()
    assert a.phone_number_id == "phone-id-fixture"
    assert a.access_token == "tok-fixture"
    assert a.gateway_url is None
    assert a.webhook_port == wa.DEFAULT_WEBHOOK_PORT
    assert a.webhook_path == wa.DEFAULT_WEBHOOK_PATH
    assert a.dm_policy == wa.DM_RESPOND
    assert a.group_policy == wa.GROUP_ALL


def test_gateway_mode_construction():
    a = _gateway_adapter()
    assert a.gateway_url == "http://localhost:3009"
    # Cloud API token / phone_id not required when gateway is set.
    assert a.access_token == ""


def test_cloud_missing_access_token_raises():
    os.environ["WHATSAPP_PHONE_NUMBER_ID"] = "pid"
    os.environ["WHATSAPP_ACCESS_TOKEN"] = ""
    os.environ["WHATSAPP_GATEWAY_URL"] = ""
    with pytest.raises(SystemExit):
        wa.WhatsAppAdapter()


def test_cloud_missing_phone_id_raises():
    os.environ["WHATSAPP_PHONE_NUMBER_ID"] = ""
    os.environ["WHATSAPP_ACCESS_TOKEN"] = "tok"
    os.environ["WHATSAPP_GATEWAY_URL"] = ""
    with pytest.raises(SystemExit):
        wa.WhatsAppAdapter()


def test_allowed_users_csv():
    a = _cloud_adapter(WHATSAPP_ALLOWED_USERS="+1555, +1666 , , +1777")
    assert a.allowed_users == ["+1555", "+1666", "+1777"]


def test_webhook_path_normalized():
    a = _cloud_adapter(WHATSAPP_WEBHOOK_PATH="meta-webhook")
    assert a.webhook_path == "/meta-webhook"


def test_dm_policy_lowercased():
    a = _cloud_adapter(WHATSAPP_DM_POLICY="ALLOWED_ONLY")
    assert a.dm_policy == "allowed_only"


def test_group_policy_lowercased():
    a = _cloud_adapter(WHATSAPP_GROUP_POLICY="Mention_Only")
    assert a.group_policy == "mention_only"


# ---- X-Hub-Signature-256 verification ------------------------------


def _xhub(secret: bytes, body: bytes) -> str:
    return "sha256=" + hmac.new(secret, body, hashlib.sha256).hexdigest()


def test_xhub_valid_signature():
    body = b'{"k":"v"}'
    sig = _xhub(b"secret", body)
    assert wa.verify_xhub_signature(b"secret", body, sig) is True


def test_xhub_wrong_key():
    body = b"x"
    sig = _xhub(b"wrong", body)
    assert wa.verify_xhub_signature(b"correct", body, sig) is False


def test_xhub_wrong_body():
    sig = _xhub(b"k", b"original")
    assert wa.verify_xhub_signature(b"k", b"tampered", sig) is False


def test_xhub_missing_header():
    assert wa.verify_xhub_signature(b"k", b"body", None) is False


def test_xhub_empty_header():
    assert wa.verify_xhub_signature(b"k", b"body", "") is False


def test_xhub_wrong_prefix():
    """Meta uses `sha256=…` only — anything else rejects."""
    h = hmac.new(b"k", b"body", hashlib.sha256).hexdigest()
    assert wa.verify_xhub_signature(b"k", b"body", h) is False
    assert wa.verify_xhub_signature(b"k", b"body", f"sha1={h}") is False
    assert wa.verify_xhub_signature(b"k", b"body", f"md5={h}") is False


def test_xhub_non_hex_digest():
    assert wa.verify_xhub_signature(b"k", b"body", "sha256=not-hex!!!") is False


def test_xhub_empty_digest():
    assert wa.verify_xhub_signature(b"k", b"body", "sha256=") is False


# ---- is_bot_mentioned ----------------------------------------------


def test_mention_by_phone():
    assert wa.is_bot_mentioned(
        "hey +15551234567 how are you", bot_phone="+15551234567", bot_name=None,
    ) is True


def test_mention_by_at_prefix():
    """The Rust adapter at whatsapp.rs:166-172 matches the bare phone
    OR `@<digits-without-plus>` form, case-insensitive substring."""
    assert wa.is_bot_mentioned(
        "hey @15551234567 hi", bot_phone="+15551234567", bot_name=None,
    ) is True


def test_mention_by_name_case_insensitive():
    assert wa.is_bot_mentioned(
        "Hey BotName", bot_phone=None, bot_name="botname",
    ) is True


def test_mention_no_match():
    assert wa.is_bot_mentioned(
        "hello world", bot_phone="+15551234567", bot_name="alice",
    ) is False


def test_mention_empty_text_no_match():
    assert wa.is_bot_mentioned(
        "", bot_phone="+15551234567", bot_name="alice",
    ) is False


# ---- should_handle_message -----------------------------------------


def test_dm_respond_accepts_anyone():
    assert wa.should_handle_message(
        is_group=False, text="hi", sender_phone="+15551",
        dm_policy=wa.DM_RESPOND, group_policy=wa.GROUP_ALL,
        allowed_users=[], bot_phone=None, bot_name=None,
    ) is True


def test_dm_allowed_only_rejects_outsiders():
    assert wa.should_handle_message(
        is_group=False, text="hi", sender_phone="+15551",
        dm_policy=wa.DM_ALLOWED_ONLY, group_policy=wa.GROUP_ALL,
        allowed_users=["+19990"], bot_phone=None, bot_name=None,
    ) is False


def test_dm_allowed_only_accepts_listed():
    assert wa.should_handle_message(
        is_group=False, text="hi", sender_phone="+15551",
        dm_policy=wa.DM_ALLOWED_ONLY, group_policy=wa.GROUP_ALL,
        allowed_users=["+15551"], bot_phone=None, bot_name=None,
    ) is True


def test_dm_allowed_only_empty_allowlist_rejects_all():
    """DmPolicy::AllowedOnly with no allowlist = nobody. Match
    whatsapp.rs:152-156 — an unconfigured allowlist means
    `is_allowed("anybody") == false`."""
    assert wa.should_handle_message(
        is_group=False, text="hi", sender_phone="+15551",
        dm_policy=wa.DM_ALLOWED_ONLY, group_policy=wa.GROUP_ALL,
        allowed_users=[], bot_phone=None, bot_name=None,
    ) is False


def test_dm_ignore_drops_all():
    assert wa.should_handle_message(
        is_group=False, text="hi", sender_phone="+15551",
        dm_policy=wa.DM_IGNORE, group_policy=wa.GROUP_ALL,
        allowed_users=["+15551"], bot_phone=None, bot_name=None,
    ) is False


def test_group_all_accepts_everything():
    assert wa.should_handle_message(
        is_group=True, text="hi", sender_phone="+1",
        dm_policy=wa.DM_RESPOND, group_policy=wa.GROUP_ALL,
        allowed_users=[], bot_phone=None, bot_name=None,
    ) is True


def test_group_mention_only_requires_mention():
    assert wa.should_handle_message(
        is_group=True, text="hello world", sender_phone="+1",
        dm_policy=wa.DM_RESPOND, group_policy=wa.GROUP_MENTION_ONLY,
        allowed_users=[], bot_phone="+15551", bot_name="bot",
    ) is False
    assert wa.should_handle_message(
        is_group=True, text="hello @bot world", sender_phone="+1",
        dm_policy=wa.DM_RESPOND, group_policy=wa.GROUP_MENTION_ONLY,
        allowed_users=[], bot_phone=None, bot_name="bot",
    ) is True


def test_group_commands_only_requires_slash():
    assert wa.should_handle_message(
        is_group=True, text="hello", sender_phone="+1",
        dm_policy=wa.DM_RESPOND, group_policy=wa.GROUP_COMMANDS_ONLY,
        allowed_users=[], bot_phone=None, bot_name=None,
    ) is False
    assert wa.should_handle_message(
        is_group=True, text="/help", sender_phone="+1",
        dm_policy=wa.DM_RESPOND, group_policy=wa.GROUP_COMMANDS_ONLY,
        allowed_users=[], bot_phone=None, bot_name=None,
    ) is True
    assert wa.should_handle_message(
        is_group=True, text="   /help", sender_phone="+1",
        dm_policy=wa.DM_RESPOND, group_policy=wa.GROUP_COMMANDS_ONLY,
        allowed_users=[], bot_phone=None, bot_name=None,
    ) is True


def test_group_ignore_drops_all():
    assert wa.should_handle_message(
        is_group=True, text="/help", sender_phone="+1",
        dm_policy=wa.DM_RESPOND, group_policy=wa.GROUP_IGNORE,
        allowed_users=[], bot_phone=None, bot_name=None,
    ) is False


def test_unknown_policy_fails_closed():
    assert wa.should_handle_message(
        is_group=True, text="hi", sender_phone="+1",
        dm_policy=wa.DM_RESPOND, group_policy="weird",
        allowed_users=[], bot_phone=None, bot_name=None,
    ) is False


# ---- parse_cloud_api_message ---------------------------------------


def _wh_payload(
    *,
    msg_id="wamid.abc",
    from_phone="15551234567",
    text="hello",
    msg_type="text",
    contact_name="Alice",
):
    return {
        "object": "whatsapp_business_account",
        "entry": [{
            "id": "WABA",
            "changes": [{
                "field": "messages",
                "value": {
                    "messaging_product": "whatsapp",
                    "contacts": (
                        [{"wa_id": from_phone, "profile": {"name": contact_name}}]
                        if contact_name is not None else []
                    ),
                    "messages": [{
                        "id": msg_id,
                        "from": from_phone,
                        "type": msg_type,
                        **(
                            {"text": {"body": text}}
                            if msg_type == "text" and text is not None else {}
                        ),
                    }],
                },
            }],
        }],
    }


def test_parse_basic_text_message():
    events = wa.parse_cloud_api_message(_wh_payload())
    assert len(events) == 1
    p = events[0]["params"]
    assert p["user_id"] == "15551234567"
    assert p["user_name"] == "Alice"
    assert p["content"]["Text"] == "hello"
    assert p["message_id"] == "wamid.abc"


def test_parse_non_text_dropped():
    """image / audio / video / sticker are silently dropped — only
    text emits, mirroring whatsapp.rs:523-609."""
    events = wa.parse_cloud_api_message(_wh_payload(msg_type="image"))
    assert events == []


def test_parse_empty_text_dropped():
    events = wa.parse_cloud_api_message(_wh_payload(text=""))
    assert events == []


def test_parse_missing_text_field_dropped():
    pl = _wh_payload()
    # Strip the inner text body.
    pl["entry"][0]["changes"][0]["value"]["messages"][0].pop("text", None)
    events = wa.parse_cloud_api_message(pl)
    assert events == []


def test_parse_falls_back_to_phone_when_no_contact():
    events = wa.parse_cloud_api_message(_wh_payload(contact_name=None))
    assert events[0]["params"]["user_name"] == "15551234567"


def test_parse_multiple_messages():
    pl = {
        "entry": [{
            "id": "WABA",
            "changes": [{
                "value": {
                    "messages": [
                        {"id": "m1", "from": "+1", "type": "text",
                         "text": {"body": "one"}},
                        {"id": "m2", "from": "+2", "type": "text",
                         "text": {"body": "two"}},
                    ],
                },
            }],
        }],
    }
    events = wa.parse_cloud_api_message(pl)
    assert len(events) == 2


def test_parse_account_id_injection():
    events = wa.parse_cloud_api_message(_wh_payload(), account_id="prod")
    assert events[0]["params"]["metadata"]["account_id"] == "prod"


def test_parse_non_dict_returns_empty():
    assert wa.parse_cloud_api_message(None) == []
    assert wa.parse_cloud_api_message("nope") == []
    assert wa.parse_cloud_api_message(42) == []


def test_parse_missing_entry_returns_empty():
    assert wa.parse_cloud_api_message({"object": "whatsapp"}) == []


# ---- inbound webhook end-to-end ------------------------------------


def test_get_verify_subscribe_match():
    a = _cloud_adapter(WHATSAPP_VERIFY_TOKEN="my-token")
    status, body = a._handle_get_verify(
        "hub.mode=subscribe&hub.verify_token=my-token&hub.challenge=ECHO123"
    )
    assert status == 200
    assert body == b"ECHO123"


def test_get_verify_wrong_token_rejected():
    a = _cloud_adapter(WHATSAPP_VERIFY_TOKEN="my-token")
    status, _body = a._handle_get_verify(
        "hub.mode=subscribe&hub.verify_token=wrong&hub.challenge=ECHO"
    )
    assert status == 403


def test_get_verify_wrong_mode_rejected():
    a = _cloud_adapter(WHATSAPP_VERIFY_TOKEN="my-token")
    status, _body = a._handle_get_verify(
        "hub.mode=unsubscribe&hub.verify_token=my-token&hub.challenge=ECHO"
    )
    assert status == 403


def test_post_webhook_signature_disabled_accepts_any():
    """When WHATSAPP_APP_SECRET is empty, signature is skipped —
    operator was warned at startup."""
    a = _cloud_adapter()  # app_secret = ""
    body = json.dumps(_wh_payload()).encode("utf-8")
    emitted: list = []
    status = a._handle_post_webhook(body, None, lambda ev: emitted.append(ev))
    assert status == 200
    assert len(emitted) == 1


def test_post_webhook_valid_signature_emits():
    a = _cloud_adapter(WHATSAPP_APP_SECRET="appsecret")
    body = json.dumps(_wh_payload()).encode("utf-8")
    sig = _xhub(b"appsecret", body)
    emitted: list = []
    status = a._handle_post_webhook(body, sig, lambda ev: emitted.append(ev))
    assert status == 200
    assert len(emitted) == 1


def test_post_webhook_invalid_signature_rejected():
    a = _cloud_adapter(WHATSAPP_APP_SECRET="appsecret")
    body = json.dumps(_wh_payload()).encode("utf-8")
    sig = _xhub(b"wrong-secret", body)
    emitted: list = []
    status = a._handle_post_webhook(body, sig, lambda ev: emitted.append(ev))
    assert status == 401
    assert emitted == []


def test_post_webhook_malformed_json_400():
    a = _cloud_adapter()
    status = a._handle_post_webhook(b"{not json", None, lambda _: None)
    assert status == 400


def test_post_webhook_dedupes_message_id():
    """Meta retries on non-200 — sidecar's SeenSet keeps the second
    delivery from double-emitting."""
    a = _cloud_adapter()
    body = json.dumps(_wh_payload(msg_id="dup-1")).encode("utf-8")
    emitted: list = []
    s1 = a._handle_post_webhook(body, None, lambda ev: emitted.append(ev))
    s2 = a._handle_post_webhook(body, None, lambda ev: emitted.append(ev))
    assert s1 == 200 and s2 == 200
    assert len(emitted) == 1


def test_post_webhook_applies_dm_policy():
    a = _cloud_adapter(
        WHATSAPP_DM_POLICY="allowed_only",
        WHATSAPP_ALLOWED_USERS="+19990",
    )
    body = json.dumps(_wh_payload(from_phone="15551234567")).encode("utf-8")
    emitted: list = []
    status = a._handle_post_webhook(body, None, lambda ev: emitted.append(ev))
    assert status == 200
    assert emitted == []


# ---- Cloud API outbound --------------------------------------------


def test_cloud_send_text_basic(monkeypatch):
    captured: list = []

    def _fake_http(url, **kw):
        captured.append((url, json.loads(kw["body"].decode("utf-8"))))
        return (200, {"messages": [{"id": "wamid.out"}]}, b"", {})

    monkeypatch.setattr(wa, "_http_request", _fake_http)
    a = _cloud_adapter()
    a._cloud_send_text("15551234567", "hello")
    assert len(captured) == 1
    url, body = captured[0]
    assert "/phone-id-fixture/messages" in url
    assert body == {
        "messaging_product": "whatsapp",
        "to": "15551234567",
        "type": "text",
        "text": {"body": "hello"},
    }


def test_cloud_send_text_chunks(monkeypatch):
    monkeypatch.setattr(wa, "MAX_MESSAGE_LEN", 5)
    calls: list = []
    monkeypatch.setattr(
        wa, "_http_request",
        lambda url, **kw: (
            calls.append(json.loads(kw["body"].decode("utf-8"))),
            (200, {}, b"", {}),
        )[1],
    )
    a = _cloud_adapter()
    a._cloud_send_text("+1", "abcdefghijk")
    assert len(calls) >= 2


def test_cloud_send_text_429_retries_once(monkeypatch):
    responses = [
        (429, None, b"slow", {"retry-after": "0"}),
        (200, {}, b"", {}),
    ]
    monkeypatch.setattr(
        wa, "_http_request",
        lambda url, **kw: responses.pop(0),
    )
    monkeypatch.setattr(wa, "_parse_retry_after", lambda h, **kw: 0.0)
    a = _cloud_adapter()
    a._cloud_send_text("+1", "hi")


def test_cloud_send_text_non_2xx_raises(monkeypatch):
    monkeypatch.setattr(
        wa, "_http_request",
        lambda url, **kw: (400, None, b"bad to phone", {}),
    )
    a = _cloud_adapter()
    with pytest.raises(RuntimeError, match="Cloud API send error"):
        a._cloud_send_text("+1", "hi")


def test_cloud_send_text_empty_text_drops(monkeypatch):
    calls: list = []
    monkeypatch.setattr(
        wa, "_http_request",
        lambda url, **kw: (calls.append(url), (200, {}, b"", {}))[1],
    )
    a = _cloud_adapter()
    a._cloud_send_text("+1", "")
    assert calls == []


def test_cloud_send_text_empty_to_drops(monkeypatch):
    calls: list = []
    monkeypatch.setattr(
        wa, "_http_request",
        lambda url, **kw: (calls.append(url), (200, {}, b"", {}))[1],
    )
    a = _cloud_adapter()
    a._cloud_send_text("", "hi")
    assert calls == []


def test_cloud_send_audio_url(monkeypatch):
    captured: list = []
    monkeypatch.setattr(
        wa, "_http_request",
        lambda url, **kw: (
            captured.append(json.loads(kw["body"].decode("utf-8"))),
            (200, {}, b"", {}),
        )[1],
    )
    a = _cloud_adapter()
    a._cloud_send_audio_url("+1", "https://example.com/a.ogg")
    assert captured[0] == {
        "messaging_product": "whatsapp",
        "to": "+1",
        "type": "audio",
        "audio": {"link": "https://example.com/a.ogg"},
    }


def test_cloud_send_image_with_caption(monkeypatch):
    captured: list = []
    monkeypatch.setattr(
        wa, "_http_request",
        lambda url, **kw: (
            captured.append(json.loads(kw["body"].decode("utf-8"))),
            (200, {}, b"", {}),
        )[1],
    )
    a = _cloud_adapter()
    a._cloud_send_image("+1", "https://x/i.jpg", "caption-text")
    assert captured[0]["type"] == "image"
    assert captured[0]["image"] == {
        "link": "https://x/i.jpg",
        "caption": "caption-text",
    }


def test_cloud_send_image_no_caption_uses_empty(monkeypatch):
    captured: list = []
    monkeypatch.setattr(
        wa, "_http_request",
        lambda url, **kw: (
            captured.append(json.loads(kw["body"].decode("utf-8"))),
            (200, {}, b"", {}),
        )[1],
    )
    a = _cloud_adapter()
    a._cloud_send_image("+1", "https://x/i.jpg", None)
    assert captured[0]["image"]["caption"] == ""


def test_cloud_send_file(monkeypatch):
    captured: list = []
    monkeypatch.setattr(
        wa, "_http_request",
        lambda url, **kw: (
            captured.append(json.loads(kw["body"].decode("utf-8"))),
            (200, {}, b"", {}),
        )[1],
    )
    a = _cloud_adapter()
    a._cloud_send_file("+1", "https://x/doc.pdf", "report.pdf")
    assert captured[0]["type"] == "document"
    assert captured[0]["document"] == {
        "link": "https://x/doc.pdf",
        "filename": "report.pdf",
    }


def test_cloud_send_location(monkeypatch):
    captured: list = []
    monkeypatch.setattr(
        wa, "_http_request",
        lambda url, **kw: (
            captured.append(json.loads(kw["body"].decode("utf-8"))),
            (200, {}, b"", {}),
        )[1],
    )
    a = _cloud_adapter()
    a._cloud_send_location("+1", 37.7749, -122.4194)
    assert captured[0]["type"] == "location"
    assert captured[0]["location"] == {
        "latitude": 37.7749, "longitude": -122.4194,
    }


# ---- Gateway outbound -----------------------------------------------


def test_gateway_send_text(monkeypatch):
    captured: list = []
    monkeypatch.setattr(
        wa, "_http_request",
        lambda url, **kw: (
            captured.append((url, json.loads(kw["body"].decode("utf-8")))),
            (200, {}, b"", {}),
        )[1],
    )
    a = _gateway_adapter()
    a._gateway_send_text("+1", "hello")
    url, body = captured[0]
    assert "/message/send" in url
    assert body == {"to": "+1", "text": "hello"}


def test_gateway_send_text_chunks(monkeypatch):
    monkeypatch.setattr(wa, "MAX_MESSAGE_LEN", 3)
    calls: list = []
    monkeypatch.setattr(
        wa, "_http_request",
        lambda url, **kw: (
            calls.append(json.loads(kw["body"].decode("utf-8"))),
            (200, {}, b"", {}),
        )[1],
    )
    a = _gateway_adapter()
    a._gateway_send_text("+1", "abcdefgh")
    assert len(calls) >= 2


def test_gateway_send_text_non_2xx_raises(monkeypatch):
    monkeypatch.setattr(
        wa, "_http_request",
        lambda url, **kw: (500, None, b"gateway boom", {}),
    )
    a = _gateway_adapter()
    with pytest.raises(RuntimeError, match="gateway send error"):
        a._gateway_send_text("+1", "hi")


# ---- on_send dispatch ----------------------------------------------


def _send_cmd(channel_id="+15551234567", text="hi", content=None,
              thread_id=None, user=None):
    from librefang.sidecar.protocol import Send
    return Send(channel_id, text, content, thread_id, user or {})


@pytest.mark.asyncio
async def test_on_send_cloud_mode_text(monkeypatch):
    sent: list = []
    monkeypatch.setattr(
        wa, "_http_request",
        lambda url, **kw: (
            sent.append(json.loads(kw["body"].decode("utf-8"))),
            (200, {}, b"", {}),
        )[1],
    )
    a = _cloud_adapter()
    await a.on_send(_send_cmd(content={"Text": "hello"}))
    assert sent[0]["text"]["body"] == "hello"


@pytest.mark.asyncio
async def test_on_send_cloud_mode_image(monkeypatch):
    sent: list = []
    monkeypatch.setattr(
        wa, "_http_request",
        lambda url, **kw: (
            sent.append(json.loads(kw["body"].decode("utf-8"))),
            (200, {}, b"", {}),
        )[1],
    )
    a = _cloud_adapter()
    await a.on_send(_send_cmd(content={
        "Image": {"url": "https://x/i.jpg", "caption": "cap"},
    }))
    assert sent[0]["type"] == "image"
    assert sent[0]["image"]["link"] == "https://x/i.jpg"


@pytest.mark.asyncio
async def test_on_send_cloud_mode_voice(monkeypatch):
    sent: list = []
    monkeypatch.setattr(
        wa, "_http_request",
        lambda url, **kw: (
            sent.append(json.loads(kw["body"].decode("utf-8"))),
            (200, {}, b"", {}),
        )[1],
    )
    a = _cloud_adapter()
    await a.on_send(_send_cmd(content={
        "Voice": {"url": "https://x/a.ogg"},
    }))
    assert sent[0]["type"] == "audio"


@pytest.mark.asyncio
async def test_on_send_cloud_mode_file(monkeypatch):
    sent: list = []
    monkeypatch.setattr(
        wa, "_http_request",
        lambda url, **kw: (
            sent.append(json.loads(kw["body"].decode("utf-8"))),
            (200, {}, b"", {}),
        )[1],
    )
    a = _cloud_adapter()
    await a.on_send(_send_cmd(content={
        "File": {"url": "https://x/doc.pdf", "filename": "doc.pdf"},
    }))
    assert sent[0]["type"] == "document"


@pytest.mark.asyncio
async def test_on_send_cloud_mode_location(monkeypatch):
    sent: list = []
    monkeypatch.setattr(
        wa, "_http_request",
        lambda url, **kw: (
            sent.append(json.loads(kw["body"].decode("utf-8"))),
            (200, {}, b"", {}),
        )[1],
    )
    a = _cloud_adapter()
    await a.on_send(_send_cmd(content={
        "Location": {"lat": 37.5, "lon": -122.0},
    }))
    assert sent[0]["type"] == "location"
    assert sent[0]["location"]["latitude"] == 37.5


@pytest.mark.asyncio
async def test_on_send_gateway_mode_text(monkeypatch):
    sent: list = []
    monkeypatch.setattr(
        wa, "_http_request",
        lambda url, **kw: (
            sent.append((url, json.loads(kw["body"].decode("utf-8")))),
            (200, {}, b"", {}),
        )[1],
    )
    a = _gateway_adapter()
    await a.on_send(_send_cmd(content={"Text": "hi"}))
    url, body = sent[0]
    assert "/message/send" in url
    assert body == {"to": "+15551234567", "text": "hi"}


@pytest.mark.asyncio
async def test_on_send_gateway_mode_voice_degrades_to_text(monkeypatch):
    """Gateway mode without raw bytes — voice URL becomes a text
    link with the `(Voice message: …)` placeholder, matching
    whatsapp.rs:493-499."""
    sent: list = []
    monkeypatch.setattr(
        wa, "_http_request",
        lambda url, **kw: (
            sent.append(json.loads(kw["body"].decode("utf-8"))),
            (200, {}, b"", {}),
        )[1],
    )
    a = _gateway_adapter()
    await a.on_send(_send_cmd(content={
        "Voice": {"url": "https://x/a.ogg"},
    }))
    assert "Voice message" in sent[0]["text"]
    assert "https://x/a.ogg" in sent[0]["text"]


@pytest.mark.asyncio
async def test_on_send_gateway_mode_image_uses_caption(monkeypatch):
    sent: list = []
    monkeypatch.setattr(
        wa, "_http_request",
        lambda url, **kw: (
            sent.append(json.loads(kw["body"].decode("utf-8"))),
            (200, {}, b"", {}),
        )[1],
    )
    a = _gateway_adapter()
    await a.on_send(_send_cmd(content={
        "Image": {"url": "https://x/i.jpg", "caption": "the caption"},
    }))
    assert sent[0]["text"] == "the caption"


@pytest.mark.asyncio
async def test_on_send_gateway_mode_image_no_caption_placeholder(monkeypatch):
    sent: list = []
    monkeypatch.setattr(
        wa, "_http_request",
        lambda url, **kw: (
            sent.append(json.loads(kw["body"].decode("utf-8"))),
            (200, {}, b"", {}),
        )[1],
    )
    a = _gateway_adapter()
    await a.on_send(_send_cmd(content={"Image": {"url": "https://x/i.jpg"}}))
    assert "Image" in sent[0]["text"]
    assert "Web mode" in sent[0]["text"]


@pytest.mark.asyncio
async def test_on_send_empty_recipient_drops(monkeypatch):
    calls: list = []
    monkeypatch.setattr(
        wa, "_http_request",
        lambda url, **kw: (calls.append(url), (200, {}, b"", {}))[1],
    )
    a = _cloud_adapter()
    await a.on_send(_send_cmd(channel_id="", user={}, content={"Text": "hi"}))
    assert calls == []


@pytest.mark.asyncio
async def test_on_send_fallback_to_user_platform_id(monkeypatch):
    sent: list = []
    monkeypatch.setattr(
        wa, "_http_request",
        lambda url, **kw: (
            sent.append(json.loads(kw["body"].decode("utf-8"))),
            (200, {}, b"", {}),
        )[1],
    )
    a = _cloud_adapter()
    await a.on_send(_send_cmd(
        channel_id="", content={"Text": "hi"},
        user={"platform_id": "+15551111"},
    ))
    assert sent[0]["to"] == "+15551111"


# ---- schema + capabilities -----------------------------------------


def test_schema_exposes_required_envs():
    schema = wa.WhatsAppAdapter.SCHEMA.to_dict()
    keys = {f["key"] for f in schema["fields"]}
    expected = {
        "WHATSAPP_PHONE_NUMBER_ID",
        "WHATSAPP_ACCESS_TOKEN",
        "WHATSAPP_VERIFY_TOKEN",
        "WHATSAPP_APP_SECRET",
        "WHATSAPP_GATEWAY_URL",
        "WHATSAPP_WEBHOOK_PORT",
        "WHATSAPP_ALLOWED_USERS",
        "WHATSAPP_ACCOUNT_ID",
        "WHATSAPP_BOT_PHONE",
        "WHATSAPP_BOT_NAME",
        "WHATSAPP_DM_POLICY",
        "WHATSAPP_GROUP_POLICY",
    }
    assert expected.issubset(keys)
    secrets = {f["key"] for f in schema["fields"] if f["type"] == "secret"}
    assert "WHATSAPP_ACCESS_TOKEN" in secrets
    assert "WHATSAPP_VERIFY_TOKEN" in secrets
    assert "WHATSAPP_APP_SECRET" in secrets


def test_capabilities_text_only():
    # No typing / reaction / thread surface for WhatsApp Cloud API
    # in the Rust adapter — sidecar preserves that.
    assert wa.WhatsAppAdapter.capabilities == []
