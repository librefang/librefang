"""Tests for librefang.sidecar.adapters.feishu.

Deterministic, no network: urllib is monkeypatched against the
shared _FakeUrlopen helper. Asserts parity with the in-process Rust
``librefang-channels::feishu`` adapter (citations are in the
sidecar module's docstring).
"""
from __future__ import annotations

import base64
import hashlib
import json
import os
from typing import Optional

import pytest

from _sidecar_fakes import _FakeResp, _FakeUrlopen, _HdrShim  # noqa: F401

os.environ.setdefault("FEISHU_APP_ID", "cli_test_app")
os.environ.setdefault("FEISHU_APP_SECRET", "secret-shh")
from librefang.sidecar.adapters import feishu as fs  # noqa: E402


# ---- Test helpers ----------------------------------------------------


def _adapter(**env):
    defaults = {
        "FEISHU_APP_ID": "cli_test_app",
        "FEISHU_APP_SECRET": "secret-shh",
        "FEISHU_REGION": "",
        "FEISHU_RECEIVE_MODE": "",
        "FEISHU_WEBHOOK_PORT": "",
        "FEISHU_VERIFICATION_TOKEN": "",
        "FEISHU_ENCRYPT_KEY": "",
        "FEISHU_ACCOUNT_ID": "",
        "FEISHU_API_BASE_OVERRIDE": "",
    }
    for k, v in defaults.items():
        os.environ[k] = env.get(k, v)
    return fs.FeishuAdapter()


def _seed_token(adapter, token: str = "tk_seeded", ttl: float = 3600.0) -> None:
    adapter._token_cache.set(token, ttl)


def _make_v2_message(
    *,
    event_id: str = "evt_1",
    text: str = "hello",
    chat_id: str = "oc_chat_1",
    msg_id: str = "om_msg_1",
    sender_type: str = "user",
    sender_open_id: str = "ou_alice",
    chat_type: str = "p2p",
    mentions=None,
) -> dict:
    content = json.dumps({"text": text})
    message = {
        "message_id": msg_id,
        "chat_id": chat_id,
        "chat_type": chat_type,
        "message_type": "text",
        "content": content,
    }
    if mentions is not None:
        message["mentions"] = mentions
    return {
        "schema": "2.0",
        "header": {
            "event_id": event_id,
            "event_type": "im.message.receive_v1",
        },
        "event": {
            "message": message,
            "sender": {
                "sender_type": sender_type,
                "sender_id": {"open_id": sender_open_id},
            },
        },
    }


def _aes_encrypt_block(block, exp):
    """One-block AES-256 encrypt, mirror of feishu._aes_decrypt_block_256
    for testing the round-trip. Lives only in test code."""
    state = list(block)
    nr = 14
    sbox = fs._AES_SBOX

    def add_round_key(rk):
        for c in range(4):
            w = exp[rk * 4 + c]
            for r in range(4):
                state[c * 4 + r] ^= w[r]

    def sub_bytes():
        for i in range(16):
            state[i] = sbox[state[i]]

    def shift_rows():
        s = state[:]
        state[0], state[4], state[8], state[12] = s[0], s[4], s[8], s[12]
        state[1], state[5], state[9], state[13] = s[5], s[9], s[13], s[1]
        state[2], state[6], state[10], state[14] = s[10], s[14], s[2], s[6]
        state[3], state[7], state[11], state[15] = s[15], s[3], s[7], s[11]

    def mix_columns():
        for c in range(4):
            a = state[c * 4:c * 4 + 4]
            state[c * 4 + 0] = (
                fs._aes_mul(a[0], 2) ^ fs._aes_mul(a[1], 3) ^ a[2] ^ a[3]
            )
            state[c * 4 + 1] = (
                a[0] ^ fs._aes_mul(a[1], 2) ^ fs._aes_mul(a[2], 3) ^ a[3]
            )
            state[c * 4 + 2] = (
                a[0] ^ a[1] ^ fs._aes_mul(a[2], 2) ^ fs._aes_mul(a[3], 3)
            )
            state[c * 4 + 3] = (
                fs._aes_mul(a[0], 3) ^ a[1] ^ a[2] ^ fs._aes_mul(a[3], 2)
            )

    add_round_key(0)
    for rnd in range(1, nr):
        sub_bytes()
        shift_rows()
        mix_columns()
        add_round_key(rnd)
    sub_bytes()
    shift_rows()
    add_round_key(nr)
    return bytes(state)


def _aes256_cbc_encrypt_pkcs7(key: bytes, iv: bytes, plain: bytes) -> bytes:
    pad = 16 - (len(plain) % 16)
    plain = plain + bytes([pad] * pad)
    exp = fs._aes_expand_key_256(key)
    prev = iv
    out = bytearray()
    for i in range(0, len(plain), 16):
        block = bytes(b ^ p for b, p in zip(plain[i:i + 16], prev))
        ct = _aes_encrypt_block(block, exp)
        out.extend(ct)
        prev = ct
    return bytes(out)


def _make_encrypted_payload(plain: dict, encrypt_key: str) -> dict:
    """Produce ``{"encrypt": "<base64>"}`` matching Feishu's wire shape."""
    key = hashlib.sha256(encrypt_key.encode("utf-8")).digest()
    iv = b"\x02" * 16
    body = json.dumps(plain).encode("utf-8")
    ct = _aes256_cbc_encrypt_pkcs7(key, iv, body)
    return {"encrypt": base64.b64encode(iv + ct).decode("ascii")}


# ---- env handling ---------------------------------------------------


def test_default_env_construction():
    a = _adapter()
    assert a.app_id == "cli_test_app"
    assert a.app_secret == "secret-shh"
    assert a.region == "cn"
    assert a.receive_mode == "websocket"
    assert a.webhook_port == 8453


def test_lark_region_picks_intl_base():
    a = _adapter(FEISHU_REGION="intl")
    assert a.region == "intl"
    assert "larksuite" in a.api_base


def test_invalid_region_falls_back_to_cn():
    a = _adapter(FEISHU_REGION="moon")
    assert a.region == "cn"


def test_invalid_mode_falls_back_to_websocket():
    a = _adapter(FEISHU_RECEIVE_MODE="ftp")
    assert a.receive_mode == "websocket"


def test_webhook_port_garbage_falls_back():
    a = _adapter(FEISHU_WEBHOOK_PORT="not-an-int")
    assert a.webhook_port == 8453


def test_missing_app_id_exits_2():
    os.environ["FEISHU_APP_ID"] = ""
    with pytest.raises(SystemExit) as e:
        fs.FeishuAdapter()
    assert e.value.code == 2
    os.environ["FEISHU_APP_ID"] = "cli_test_app"


def test_missing_app_secret_exits_2():
    os.environ["FEISHU_APP_SECRET"] = ""
    with pytest.raises(SystemExit) as e:
        fs.FeishuAdapter()
    assert e.value.code == 2
    os.environ["FEISHU_APP_SECRET"] = "secret-shh"


def test_account_id_passthrough():
    a = _adapter(FEISHU_ACCOUNT_ID="prod-bot")
    assert a.account_id == "prod-bot"


def test_account_id_empty_is_none():
    a = _adapter(FEISHU_ACCOUNT_ID="")
    assert a.account_id is None


# ---- region helpers --------------------------------------------------


def test_region_label_cn():
    assert fs.FeishuRegion.label("cn") == "Feishu"
    assert fs.FeishuRegion.channel_label("cn") == "feishu"


def test_region_label_intl():
    assert fs.FeishuRegion.label("intl") == "Lark"
    assert fs.FeishuRegion.channel_label("intl") == "lark"


def test_region_api_base():
    assert "feishu.cn" in fs.FeishuRegion.api_base("cn")
    assert "larksuite" in fs.FeishuRegion.api_base("intl")


# ---- AES + decrypt --------------------------------------------------


def test_aes_nist_decrypt_vector():
    import binascii
    key = binascii.unhexlify(
        "603deb1015ca71be2b73aef0857d77811f352c073b6108d72d9810a30914dff4",
    )
    iv = binascii.unhexlify("000102030405060708090a0b0c0d0e0f")
    plain = binascii.unhexlify("6bc1bee22e409f96e93d7e117393172a")
    ct = binascii.unhexlify("f58c4c04d6e5f1ba779eabfb5f7bfbd6")
    expanded = fs._aes_expand_key_256(key)
    got = fs._aes_decrypt_block_256(ct, expanded)
    # NIST SP 800-38A F.2.5: this is the CBC block-1 ciphertext;
    # decrypt gives plaintext ^ iv (we XOR back in the higher path).
    expected = bytes(p ^ i for p, i in zip(plain, iv))
    assert got == expected


def test_decrypt_feishu_payload_round_trip():
    encrypted = _make_encrypted_payload({"event": "fired"}, "secret-key")
    got = fs.decrypt_feishu_payload(encrypted["encrypt"], "secret-key")
    assert got == {"event": "fired"}


def test_decrypt_feishu_payload_wrong_key():
    encrypted = _make_encrypted_payload({"event": "fired"}, "secret-key")
    with pytest.raises(ValueError):
        fs.decrypt_feishu_payload(encrypted["encrypt"], "WRONG_KEY")


def test_decrypt_feishu_payload_too_short():
    short = base64.b64encode(b"abc").decode("ascii")
    with pytest.raises(ValueError, match="too short"):
        fs.decrypt_feishu_payload(short, "k")


def test_decrypt_feishu_payload_not_block_aligned():
    # 16-byte IV + 17-byte non-block-aligned ciphertext
    raw = base64.b64encode(b"\x00" * 16 + b"x" * 17).decode("ascii")
    with pytest.raises(ValueError, match="not block-aligned"):
        fs.decrypt_feishu_payload(raw, "k")


def test_decrypt_if_needed_passthrough_when_no_encrypt():
    out = fs.decrypt_feishu_payload_if_needed({"plain": True}, None)
    assert out == {"plain": True}


def test_decrypt_if_needed_errors_when_key_missing():
    enc = _make_encrypted_payload({"x": 1}, "k")
    with pytest.raises(ValueError, match="encrypt_key is configured"):
        fs.decrypt_feishu_payload_if_needed(enc, None)


def test_decrypt_if_needed_returns_plaintext():
    enc = _make_encrypted_payload({"answer": 42}, "k")
    out = fs.decrypt_feishu_payload_if_needed(enc, "k")
    assert out == {"answer": 42}


# ---- event dedup ----------------------------------------------------


def test_dedup_first_event_passes():
    d = fs._EventDedup()
    assert d.is_duplicate("evt_1") is False


def test_dedup_second_event_blocks():
    d = fs._EventDedup()
    d.is_duplicate("evt_1")
    assert d.is_duplicate("evt_1") is True


def test_dedup_no_event_id_not_dedupable():
    d = fs._EventDedup()
    assert d.is_duplicate(None) is False
    assert d.is_duplicate("") is False


def test_dedup_purges_when_over_cap():
    d = fs._EventDedup(window_secs=0.001, max_entries=3)
    d.is_duplicate("a")
    d.is_duplicate("b")
    d.is_duplicate("c")
    import time as _t
    _t.sleep(0.01)
    # At max — next insert triggers purge of expired (which is all).
    assert d.is_duplicate("d") is False
    assert d.is_duplicate("a") is False  # expired, not duplicate


# ---- approval card builder ------------------------------------------


def test_build_approval_card_shape():
    card = fs.build_approval_card(
        "req_1", "agent_x", "fs_write", "edit file.txt", "high",
    )
    assert card["header"]["template"] == "orange"
    assert "Permission Request" in card["header"]["title"]["content"]
    actions = card["elements"][-1]["actions"]
    assert actions[0]["value"]["action"] == "approve"
    assert actions[0]["value"]["request_id"] == "req_1"
    assert actions[1]["value"]["action"] == "reject"


def test_build_approval_card_color_default():
    card = fs.build_approval_card("r", "a", "t", "s", "unknown")
    assert card["header"]["template"] == "blue"


def test_build_approval_card_critical_red():
    card = fs.build_approval_card("r", "a", "t", "s", "critical")
    assert card["header"]["template"] == "red"


# ---- v2 event parsing ----------------------------------------------


def test_parse_v2_text_message():
    payload = _make_v2_message(text="hello world")
    out = fs.parse_feishu_event(payload, "cn")
    assert out is not None
    assert out["method"] == "message"
    params = out["params"]
    assert params["text"] == "hello world"
    assert params["channel_id"] == "oc_chat_1"


def test_parse_v2_slash_routes_to_command():
    payload = _make_v2_message(text="/help arg1 arg2")
    out = fs.parse_feishu_event(payload, "cn")
    assert out is not None
    content = out["params"]["content"]
    assert "Command" in content
    assert content["Command"]["name"] == "help"
    assert content["Command"]["args"] == ["arg1", "arg2"]


def test_parse_v2_self_skip_app_sender():
    payload = _make_v2_message(sender_type="app")
    assert fs.parse_feishu_event(payload, "cn") is None


def test_parse_v2_self_skip_bot_sender():
    payload = _make_v2_message(sender_type="bot")
    assert fs.parse_feishu_event(payload, "cn") is None


def test_parse_v2_group_chat_sets_is_group():
    payload = _make_v2_message(chat_type="group")
    out = fs.parse_feishu_event(payload, "cn")
    assert out is not None
    assert out["params"]["is_group"] is True


def test_parse_v2_p2p_chat_not_group():
    payload = _make_v2_message(chat_type="p2p")
    out = fs.parse_feishu_event(payload, "cn")
    # protocol.message() omits is_group when False — absence == not a group.
    assert out["params"].get("is_group", False) is False


def test_parse_v2_root_id_becomes_thread_id():
    payload = _make_v2_message()
    payload["event"]["message"]["root_id"] = "om_thread"
    out = fs.parse_feishu_event(payload, "cn")
    assert out["params"]["thread_id"] == "om_thread"


def test_parse_v2_no_root_id_thread_id_none():
    payload = _make_v2_message()
    out = fs.parse_feishu_event(payload, "cn")
    # protocol.message() omits thread_id when None — absence == no thread.
    assert "thread_id" not in out["params"]


def test_parse_v2_mention_user_replaced_with_name():
    payload = _make_v2_message(
        text="hi @_user_1, please review",
        mentions=[{"key": "@_user_1", "name": "Alice",
                   "id": {"open_id": "ou_alice"}}],
    )
    out = fs.parse_feishu_event(payload, "cn")
    assert "hi @Alice" in out["params"]["text"]


def test_parse_v2_mention_at_all_renders_as_at_all():
    payload = _make_v2_message(
        text="@_all heads up",
        mentions=[{"key": "@_all"}],
    )
    out = fs.parse_feishu_event(payload, "cn")
    assert out["params"]["text"].startswith("@all heads up")


def test_parse_v2_metadata_carries_chat_meta():
    payload = _make_v2_message()
    out = fs.parse_feishu_event(payload, "cn")
    meta = out["params"]["metadata"]
    assert meta["chat_id"] == "oc_chat_1"
    assert meta["message_id"] == "om_msg_1"
    assert meta["region"] == "feishu"
    assert meta["sender_id"] == "ou_alice"


def test_parse_v2_lark_region_metadata():
    payload = _make_v2_message()
    out = fs.parse_feishu_event(payload, "intl")
    assert out["params"]["metadata"]["region"] == "lark"


def test_parse_v2_wrong_event_type_returns_none():
    payload = _make_v2_message()
    payload["header"]["event_type"] = "im.something_else"
    assert fs.parse_feishu_event(payload, "cn") is None


def test_parse_v2_non_text_message_type_returns_none():
    payload = _make_v2_message()
    payload["event"]["message"]["message_type"] = "image"
    assert fs.parse_feishu_event(payload, "cn") is None


def test_parse_v2_empty_text_returns_none():
    payload = _make_v2_message(text="")
    assert fs.parse_feishu_event(payload, "cn") is None


def test_parse_v2_malformed_returns_none():
    assert fs.parse_feishu_event(None, "cn") is None
    assert fs.parse_feishu_event({}, "cn") is None


# ---- v1 event parsing ----------------------------------------------


def test_parse_v1_message():
    payload = {
        "event": {
            "type": "message",
            "open_id": "ou_v1",
            "text": "v1 hello",
            "open_chat_id": "oc_v1",
            "open_message_id": "om_v1",
            "chat_type": "group",
        },
    }
    out = fs.parse_feishu_event_v1(payload, "cn")
    assert out is not None
    assert out["params"]["text"] == "v1 hello"
    assert out["params"]["is_group"] is True


def test_parse_v1_empty_open_id_returns_none():
    payload = {
        "event": {"type": "message", "open_id": "", "text": "x"},
    }
    assert fs.parse_feishu_event_v1(payload, "cn") is None


def test_parse_v1_slash_command():
    payload = {
        "event": {
            "type": "message",
            "open_id": "ou_v1",
            "text": "/approve req_42",
        },
    }
    out = fs.parse_feishu_event_v1(payload, "cn")
    assert out["params"]["content"]["Command"]["name"] == "approve"


# ---- card action parsing -------------------------------------------


def _card_event(action_type: str = "approve") -> dict:
    return {
        "header": {
            "event_id": "card_evt_1",
            "event_type": "card.action.trigger",
        },
        "event": {
            "operator": {"open_id": "ou_clicker"},
            "open_chat_id": "oc_chat_card",
            "open_message_id": "om_card_msg",
            "action": {
                "value": {"action": action_type, "request_id": "req_X"},
            },
        },
    }


def test_parse_card_action_approve():
    out = fs.parse_card_action(_card_event("approve"), "cn")
    assert out is not None
    c = out["params"]["content"]
    assert c["Command"]["name"] == "approve"
    assert c["Command"]["args"] == ["req_X"]


def test_parse_card_action_reject():
    out = fs.parse_card_action(_card_event("reject"), "cn")
    assert out["params"]["content"]["Command"]["name"] == "reject"


def test_parse_card_action_unknown_returns_none():
    assert fs.parse_card_action(_card_event("delete"), "cn") is None


def test_parse_card_action_wrong_event_type():
    ev = _card_event()
    ev["header"]["event_type"] = "im.message.receive_v1"
    assert fs.parse_card_action(ev, "cn") is None


def test_parse_card_action_metadata_marks_card_action():
    out = fs.parse_card_action(_card_event(), "cn")
    assert out["params"]["metadata"]["card_action"] is True
    assert out["params"]["metadata"]["operator_id"] == "ou_clicker"


# ---- token cache --------------------------------------------------


def test_token_cache_returns_set_value():
    c = fs._TokenCache()
    c.set("abc", 3600)
    assert c.get() == "abc"


def test_token_cache_returns_none_when_expired():
    c = fs._TokenCache()
    # ttl shorter than refresh buffer → already-expired
    c.set("abc", ttl_secs=10)
    assert c.get() is None


def test_token_cache_clear():
    c = fs._TokenCache()
    c.set("abc", 3600)
    c.clear()
    assert c.get() is None


# ---- HTTP integration --------------------------------------------


def test_refresh_token_caches_response(monkeypatch):
    fake = _FakeUrlopen([(200, {"code": 0,
                                 "tenant_access_token": "ttok",
                                 "expire": 7200})])
    monkeypatch.setattr(fs.urllib.request, "urlopen", fake)
    a = _adapter()
    got = a._get_token()
    assert got == "ttok"
    # Second call hits the cache.
    got2 = a._get_token()
    assert got2 == "ttok"
    assert len(fake.calls) == 1


def test_refresh_token_api_error_raises(monkeypatch):
    fake = _FakeUrlopen([(200, {"code": 99, "msg": "denied"})])
    monkeypatch.setattr(fs.urllib.request, "urlopen", fake)
    a = _adapter()
    with pytest.raises(RuntimeError, match="token error: denied"):
        a._get_token()


def test_refresh_token_http_error_raises(monkeypatch):
    fake = _FakeUrlopen([(500, {})])
    monkeypatch.setattr(fs.urllib.request, "urlopen", fake)
    a = _adapter()
    with pytest.raises(RuntimeError, match="token request failed"):
        a._get_token()


def test_validate_returns_bot_name(monkeypatch):
    fake = _FakeUrlopen([
        (200, {"code": 0, "bot": {"app_name": "MyBot"}}),
    ])
    monkeypatch.setattr(fs.urllib.request, "urlopen", fake)
    a = _adapter()
    _seed_token(a)
    assert a._validate() == "MyBot"


def test_validate_missing_app_name_uses_default(monkeypatch):
    fake = _FakeUrlopen([(200, {"code": 0, "bot": {}})])
    monkeypatch.setattr(fs.urllib.request, "urlopen", fake)
    a = _adapter()
    _seed_token(a)
    assert a._validate() == "Feishu Bot"


def test_validate_api_error_raises(monkeypatch):
    fake = _FakeUrlopen([(200, {"code": 99, "msg": "no access"})])
    monkeypatch.setattr(fs.urllib.request, "urlopen", fake)
    a = _adapter()
    _seed_token(a)
    with pytest.raises(RuntimeError, match="bot info error: no access"):
        a._validate()


# ---- send paths -------------------------------------------------


def test_send_text_posts_to_messages_endpoint(monkeypatch):
    fake = _FakeUrlopen([(200, {"code": 0, "data": {}})])
    monkeypatch.setattr(fs.urllib.request, "urlopen", fake)
    a = _adapter()
    _seed_token(a)
    a._send_text("oc_chat_99", "hello")
    c = fake.calls[0]
    assert "/open-apis/im/v1/messages" in c["url"]
    assert "receive_id_type=chat_id" in c["url"]
    body = json.loads(c["body_raw"])
    assert body["receive_id"] == "oc_chat_99"
    assert body["msg_type"] == "text"
    assert json.loads(body["content"])["text"] == "hello"


def test_send_text_chunks_long_message(monkeypatch):
    monkeypatch.setattr(fs, "MAX_MESSAGE_LEN", 5)
    fake = _FakeUrlopen([
        (200, {"code": 0}),
        (200, {"code": 0}),
        (200, {"code": 0}),
    ])
    monkeypatch.setattr(fs.urllib.request, "urlopen", fake)
    a = _adapter()
    _seed_token(a)
    a._send_text("oc_chat_99", "abcdefghijklm")
    assert len(fake.calls) >= 2


def test_send_text_http_error_raises(monkeypatch):
    fake = _FakeUrlopen([(500, {})])
    monkeypatch.setattr(fs.urllib.request, "urlopen", fake)
    a = _adapter()
    _seed_token(a)
    with pytest.raises(RuntimeError, match="send message error"):
        a._send_text("oc_x", "hi")


def test_http_json_honours_retry_after_on_429(monkeypatch):
    """Feishu / Lark publish quotas (im.message.send: 1000/min per
    app, plus a tenant-wide bucket) surface as 429 with Retry-After.
    Before the fix the 4-tuple `_http_json` ignored 429 entirely and
    fell through to the `code != 0` arm, raising and dropping the
    chunk. The fix honours Retry-After and retries once via the
    shared `parse_retry_after` helper."""
    fake = _FakeUrlopen([
        (429, {}, {"Retry-After": "3"}),
        (200, {"code": 0, "data": {}}),
    ])
    sleeps: list[float] = []
    monkeypatch.setattr(fs.urllib.request, "urlopen", fake)
    monkeypatch.setattr(fs.time, "sleep", sleeps.append)
    a = _adapter()
    _seed_token(a)
    a._send_text("oc_x", "hi")
    assert sleeps == [3.0]
    # Retry actually re-posts to the same endpoint with the same
    # payload (not a stale snapshot).
    assert len(fake.calls) == 2
    body = json.loads(fake.calls[1]["body_raw"])
    assert body["receive_id"] == "oc_x"


def test_http_json_surfaces_second_429(monkeypatch):
    """After honouring Retry-After once, a second 429 surfaces as a
    normal error response (callers already handle non-2xx) — no
    infinite retry loop."""
    fake = _FakeUrlopen([
        (429, {}, {"Retry-After": "1"}),
        (429, {}, {"Retry-After": "1"}),
    ])
    monkeypatch.setattr(fs.urllib.request, "urlopen", fake)
    monkeypatch.setattr(fs.time, "sleep", lambda _s: None)
    a = _adapter()
    _seed_token(a)
    with pytest.raises(RuntimeError):
        a._send_text("oc_x", "hi")
    assert len(fake.calls) == 2


def test_http_json_429_without_retry_after_uses_default(monkeypatch):
    fake = _FakeUrlopen([
        (429, {}, {}),
        (200, {"code": 0, "data": {}}),
    ])
    sleeps: list[float] = []
    monkeypatch.setattr(fs.urllib.request, "urlopen", fake)
    monkeypatch.setattr(fs.time, "sleep", sleeps.append)
    a = _adapter()
    _seed_token(a)
    a._send_text("oc_x", "hi")
    assert sleeps == [fs.RETRY_AFTER_DEFAULT_SECS]


def test_send_card_uses_interactive_msg_type(monkeypatch):
    fake = _FakeUrlopen([(200, {"code": 0})])
    monkeypatch.setattr(fs.urllib.request, "urlopen", fake)
    a = _adapter()
    _seed_token(a)
    card = fs.build_approval_card("r1", "ag", "t", "s", "high")
    a._send_card("oc_xy", card)
    body = json.loads(fake.calls[0]["body_raw"])
    assert body["msg_type"] == "interactive"
    inner = json.loads(body["content"])
    assert inner["header"]["template"] == "orange"


def test_send_card_api_error_raises(monkeypatch):
    fake = _FakeUrlopen([(200, {"code": 13, "msg": "bad card"})])
    monkeypatch.setattr(fs.urllib.request, "urlopen", fake)
    a = _adapter()
    _seed_token(a)
    with pytest.raises(RuntimeError, match="send card API error"):
        a._send_card("oc_xy", {})


# ---- on_send dispatch ------------------------------------------


def _send_cmd(channel_id="oc_x", text="hi", content=None, thread_id=None,
              user=None):
    from librefang.sidecar.protocol import Send
    return Send(channel_id, text, content, thread_id, user or {})


@pytest.mark.asyncio
async def test_on_send_text_path(monkeypatch):
    fake = _FakeUrlopen([(200, {"code": 0})])
    monkeypatch.setattr(fs.urllib.request, "urlopen", fake)
    a = _adapter()
    _seed_token(a)
    await a.on_send(_send_cmd(text="hello", content={"Text": "hello"}))
    assert len(fake.calls) == 1
    body = json.loads(fake.calls[0]["body_raw"])
    assert body["msg_type"] == "text"


@pytest.mark.asyncio
async def test_on_send_interactive_renders_button_hints(monkeypatch):
    fake = _FakeUrlopen([(200, {"code": 0})])
    monkeypatch.setattr(fs.urllib.request, "urlopen", fake)
    a = _adapter()
    _seed_token(a)
    await a.on_send(_send_cmd(content={"Interactive": {
        "text": "pick:",
        "buttons": [[{"label": "yes"}, {"label": "no"}]],
    }}))
    body = json.loads(fake.calls[0]["body_raw"])
    inner = json.loads(body["content"])
    assert "[yes]" in inner["text"]
    assert "[no]" in inner["text"]


@pytest.mark.asyncio
async def test_on_send_empty_channel_drops(monkeypatch):
    fake = _FakeUrlopen([])
    monkeypatch.setattr(fs.urllib.request, "urlopen", fake)
    a = _adapter()
    await a.on_send(_send_cmd(channel_id="", user={}))
    assert fake.calls == []


@pytest.mark.asyncio
async def test_on_send_falls_back_to_user_platform_id(monkeypatch):
    fake = _FakeUrlopen([(200, {"code": 0})])
    monkeypatch.setattr(fs.urllib.request, "urlopen", fake)
    a = _adapter()
    _seed_token(a)
    await a.on_send(_send_cmd(
        channel_id="", text="hi", content={"Text": "hi"},
        user={"platform_id": "oc_fallback"},
    ))
    body = json.loads(fake.calls[0]["body_raw"])
    assert body["receive_id"] == "oc_fallback"


# ---- WS endpoint + binary frame parsing -------------------------


def test_get_ws_endpoint_returns_url(monkeypatch):
    fake = _FakeUrlopen([(200, {
        "code": 0,
        "data": {
            "URL": "wss://gateway.feishu.cn/abc",
            "ClientConfig": {"PingInterval": 60},
        },
    })])
    monkeypatch.setattr(fs.urllib.request, "urlopen", fake)
    a = _adapter()
    url, cfg = a._get_ws_endpoint()
    assert url == "wss://gateway.feishu.cn/abc"
    assert cfg["PingInterval"] == 60


def test_get_ws_endpoint_api_error_raises(monkeypatch):
    fake = _FakeUrlopen([(200, {"code": 5, "msg": "bad creds"})])
    monkeypatch.setattr(fs.urllib.request, "urlopen", fake)
    a = _adapter()
    with pytest.raises(RuntimeError, match="ws endpoint error"):
        a._get_ws_endpoint()


def test_get_ws_endpoint_missing_url_raises(monkeypatch):
    fake = _FakeUrlopen([(200, {"code": 0, "data": {"URL": ""}})])
    monkeypatch.setattr(fs.urllib.request, "urlopen", fake)
    a = _adapter()
    with pytest.raises(RuntimeError, match="empty URL"):
        a._get_ws_endpoint()


def test_handle_ws_text_dispatches_event():
    emitted: list = []
    a = _adapter()
    payload = _make_v2_message()
    a._handle_ws_text(json.dumps(payload), lambda e: emitted.append(e))
    assert len(emitted) == 1
    assert emitted[0]["method"] == "message"


def test_handle_ws_text_pong_ignored():
    emitted: list = []
    a = _adapter()
    a._handle_ws_text(json.dumps({"type": "pong"}), lambda e: emitted.append(e))
    assert emitted == []


def test_handle_ws_text_malformed_json_ignored():
    emitted: list = []
    a = _adapter()
    a._handle_ws_text("not-json", lambda e: emitted.append(e))
    assert emitted == []


def test_handle_ws_binary_extracts_embedded_json():
    """Feishu sometimes wraps the JSON payload in a protobuf binary
    envelope. The adapter slices out the embedded JSON object."""
    emitted: list = []
    a = _adapter()
    payload = _make_v2_message(event_id="ws_bin_1")
    raw = b"PROTO_HDR\x00\x01" + json.dumps(payload).encode("utf-8") + b"\x00TRAILER"
    a._handle_ws_binary(raw, lambda e: emitted.append(e))
    assert len(emitted) == 1


def test_handle_ws_binary_no_json_object_ignored():
    emitted: list = []
    a = _adapter()
    a._handle_ws_binary(b"\x00\x01\x02 no curlies", lambda e: emitted.append(e))
    assert emitted == []


# ---- dispatch + dedup integration -------------------------------


def test_dispatch_event_drops_duplicate():
    emitted: list = []
    a = _adapter()
    payload = _make_v2_message(event_id="dup_1")
    a._dispatch_event(payload, lambda e: emitted.append(e))
    a._dispatch_event(payload, lambda e: emitted.append(e))
    assert len(emitted) == 1


def test_dispatch_event_injects_account_id():
    emitted: list = []
    a = _adapter(FEISHU_ACCOUNT_ID="tenant-A")
    payload = _make_v2_message(event_id="acct_1")
    a._dispatch_event(payload, lambda e: emitted.append(e))
    assert emitted[0]["params"]["metadata"]["account_id"] == "tenant-A"


def test_dispatch_v1_fallback_when_no_schema():
    emitted: list = []
    a = _adapter()
    payload = {
        "event": {
            "type": "message",
            "open_id": "ou_v1",
            "text": "hello v1",
            "open_chat_id": "oc_v1",
            "open_message_id": "om_v1",
        },
    }
    a._dispatch_event(payload, lambda e: emitted.append(e))
    assert len(emitted) == 1
    assert emitted[0]["params"]["text"] == "hello v1"


def test_dispatch_v2_card_action_via_fallback():
    emitted: list = []
    a = _adapter()
    # Card action carries schema 2.0 but a different event_type;
    # parse_feishu_event returns None and parse_card_action runs.
    payload = _card_event("approve")
    payload["schema"] = "2.0"
    a._dispatch_event(payload, lambda e: emitted.append(e))
    assert len(emitted) == 1
    assert emitted[0]["params"]["content"]["Command"]["name"] == "approve"


# ---- webhook handler -------------------------------------------


def _post_to_webhook(adapter, body: dict, *, emit_target: list,
                     headers: Optional[dict] = None,
                     path: str = "/webhook") -> tuple[int, bytes]:
    """Drive the webhook handler with a synthetic request.

    Builds a minimal BaseHTTPRequestHandler-like context, runs the
    request body through ``do_POST`` directly. Avoids spinning up an
    HTTP server in tests."""
    import io
    handler_cls = fs._make_webhook_handler(adapter, lambda e: emit_target.append(e))
    body_bytes = json.dumps(body).encode("utf-8")
    hdr_lines = [
        f"POST {path} HTTP/1.1",
        "Host: localhost",
        f"Content-Length: {len(body_bytes)}",
        "Content-Type: application/json",
    ]
    if headers:
        for k, v in headers.items():
            hdr_lines.append(f"{k}: {v}")
    request_text = "\r\n".join(hdr_lines) + "\r\n\r\n"
    raw = request_text.encode("ascii") + body_bytes

    class _Sock:
        def __init__(self, data: bytes) -> None:
            self._r = io.BytesIO(data)
            self.w = io.BytesIO()

        def makefile(self, mode, *_args, **_kw):
            return self._r if "r" in mode else self.w

        def sendall(self, data: bytes) -> None:
            self.w.write(data)

        def getsockname(self):
            return ("127.0.0.1", 0)

    sock = _Sock(raw)
    handler = handler_cls(sock, ("127.0.0.1", 9999), None)
    # BaseHTTPRequestHandler runs do_POST inside __init__ via handle().
    response_raw = sock.w.getvalue()
    # Status line is first.
    first_line = response_raw.split(b"\r\n", 1)[0]
    # "HTTP/1.0 200 OK" or similar.
    parts = first_line.split(b" ", 2)
    status = int(parts[1]) if len(parts) >= 2 else 0
    return status, response_raw


def test_webhook_url_verification_echoes_challenge():
    emit_target: list = []
    a = _adapter()
    status, resp = _post_to_webhook(
        a, {"challenge": "abc-xyz", "token": "ignored"},
        emit_target=emit_target,
    )
    assert status == 200
    body_start = resp.find(b"\r\n\r\n") + 4
    body_json = json.loads(resp[body_start:])
    assert body_json["challenge"] == "abc-xyz"
    assert emit_target == []


def test_webhook_verification_token_rejects_bad_token():
    emit_target: list = []
    a = _adapter(FEISHU_VERIFICATION_TOKEN="goodtok")
    status, _ = _post_to_webhook(
        a, {"challenge": "x", "token": "wrong"},
        emit_target=emit_target,
    )
    assert status == 403


def test_webhook_verification_token_accepts_good_token():
    emit_target: list = []
    a = _adapter(FEISHU_VERIFICATION_TOKEN="goodtok")
    status, _ = _post_to_webhook(
        a, {"challenge": "ok", "token": "goodtok"},
        emit_target=emit_target,
    )
    assert status == 200


def test_webhook_event_dispatches():
    emit_target: list = []
    a = _adapter()
    payload = _make_v2_message(event_id="webhook_evt_1")
    status, _ = _post_to_webhook(a, payload, emit_target=emit_target)
    assert status == 200
    assert len(emit_target) == 1
    assert emit_target[0]["params"]["text"] == "hello"


def test_webhook_event_token_mismatch_returns_403():
    emit_target: list = []
    a = _adapter(FEISHU_VERIFICATION_TOKEN="goodtok")
    payload = _make_v2_message(event_id="bad_tok")
    payload["token"] = "wrong"
    status, _ = _post_to_webhook(a, payload, emit_target=emit_target)
    assert status == 403


def test_webhook_encrypted_payload_decrypts_and_dispatches():
    emit_target: list = []
    a = _adapter(FEISHU_ENCRYPT_KEY="secret-enc-key")
    plain = _make_v2_message(event_id="enc_evt")
    encrypted = _make_encrypted_payload(plain, "secret-enc-key")
    status, _ = _post_to_webhook(a, encrypted, emit_target=emit_target)
    assert status == 200
    assert len(emit_target) == 1


def test_webhook_encrypted_payload_no_key_returns_400():
    emit_target: list = []
    a = _adapter()  # no key
    encrypted = _make_encrypted_payload({"foo": "bar"}, "some-key")
    status, _ = _post_to_webhook(a, encrypted, emit_target=emit_target)
    assert status == 400


def test_webhook_404_for_non_webhook_path():
    emit_target: list = []
    a = _adapter()
    status, _ = _post_to_webhook(
        a, {}, emit_target=emit_target, path="/something-else",
    )
    assert status == 404


def test_webhook_oversized_body_returns_413(monkeypatch):
    """A malicious sender shouldn't be able to OOM the sidecar by
    advertising Content-Length: 10G. Cap the body at 1 MiB."""
    monkeypatch.setattr(fs, "WEBHOOK_MAX_BODY_BYTES", 64)
    emit_target: list = []
    a = _adapter()
    # 80-byte JSON object, over the patched 64 B cap.
    big_body = {"x": "A" * 70}
    status, _ = _post_to_webhook(a, big_body, emit_target=emit_target)
    assert status == 413
    assert emit_target == []




# ---- schema / capability contract ------------------------------


def test_schema_exposes_required_env():
    schema = fs.FeishuAdapter.SCHEMA.to_dict()
    keys = {f["key"] for f in schema["fields"]}
    expected = {
        "FEISHU_APP_ID",
        "FEISHU_APP_SECRET",
        "FEISHU_REGION",
        "FEISHU_RECEIVE_MODE",
        "FEISHU_WEBHOOK_PORT",
        "FEISHU_VERIFICATION_TOKEN",
        "FEISHU_ENCRYPT_KEY",
        "FEISHU_ACCOUNT_ID",
    }
    assert expected.issubset(keys), f"missing: {expected - keys}"
    secret_fields = {
        f["key"] for f in schema["fields"] if f["type"] == "secret"
    }
    assert secret_fields == {
        "FEISHU_APP_SECRET",
        "FEISHU_VERIFICATION_TOKEN",
        "FEISHU_ENCRYPT_KEY",
    }


def test_capabilities_empty_matches_rust_parity():
    # Rust adapter declared no capabilities — the sidecar matches.
    # Interactive content still flows through on_send via the
    # generic fallback path; declaring "interactive" would
    # misrepresent what the sidecar actually does (text + `[label]`
    # button-hint fallback, not real Feishu cards).
    assert fs.FeishuAdapter.capabilities == []
