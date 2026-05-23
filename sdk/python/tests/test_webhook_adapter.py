"""Tests for librefang.sidecar.adapters.webhook.

Deterministic, no network — urllib monkeypatched via _http_request.
"""
from __future__ import annotations

import hashlib
import hmac
import json
import os
import time

import pytest

os.environ.setdefault("WEBHOOK_SECRET", "test-secret")
from librefang.sidecar.adapters import webhook as wh  # noqa: E402


def _adapter(**env):
    defaults = {
        "WEBHOOK_SECRET": "test-secret",
        "WEBHOOK_LISTEN_PORT": "",
        "WEBHOOK_LISTEN_PATH": "",
        "WEBHOOK_BIND_HOST": "",
        "WEBHOOK_CALLBACK_URL": "",
        "WEBHOOK_DELIVER_ONLY": "",
        "WEBHOOK_DELIVER": "",
        "WEBHOOK_ACCOUNT_ID": "",
    }
    for k, v in defaults.items():
        os.environ[k] = env.get(k, v)
    return wh.WebhookAdapter()


# ---- env handling ---------------------------------------------------


def test_default_env_construction():
    a = _adapter()
    assert a.secret == "test-secret"
    assert a.listen_port == wh.DEFAULT_LISTEN_PORT
    assert a.listen_path == wh.DEFAULT_LISTEN_PATH
    assert a.callback_url is None
    assert a.deliver_only is False
    assert a.deliver_target is None
    assert a.account_id is None


def test_missing_secret_raises():
    os.environ["WEBHOOK_SECRET"] = ""
    with pytest.raises(SystemExit):
        wh.WebhookAdapter()


def test_listen_port_override():
    a = _adapter(WEBHOOK_LISTEN_PORT="9999")
    assert a.listen_port == 9999


def test_listen_path_normalized():
    a = _adapter(WEBHOOK_LISTEN_PATH="incoming")
    assert a.listen_path == "/incoming"


def test_callback_url_public_ok():
    a = _adapter(WEBHOOK_CALLBACK_URL="https://example.com/in")
    assert a.callback_url == "https://example.com/in"


def test_callback_url_private_ip_rejected():
    """SSRF guard: a callback pointing at a private IP must fail
    at adapter construction so the secret never leaks to
    localhost."""
    with pytest.raises(SystemExit):
        _adapter(WEBHOOK_CALLBACK_URL="https://127.0.0.1/in")


def test_callback_url_localhost_rejected():
    with pytest.raises(SystemExit):
        _adapter(WEBHOOK_CALLBACK_URL="http://localhost/in")


def test_callback_url_metadata_service_rejected():
    """169.254.169.254 is the AWS / GCP / Azure cloud metadata
    service — gating it is the whole point of the SSRF guard."""
    with pytest.raises(SystemExit):
        _adapter(WEBHOOK_CALLBACK_URL="http://169.254.169.254/latest/meta-data/")


def test_callback_url_file_scheme_rejected():
    with pytest.raises(SystemExit):
        _adapter(WEBHOOK_CALLBACK_URL="file:///etc/passwd")


def test_deliver_only_without_target_rejected():
    """Match the kernel's validation contract: deliver_only with no
    target = inbound silently dropped. Fail-closed at boot."""
    with pytest.raises(SystemExit):
        _adapter(WEBHOOK_DELIVER_ONLY="1", WEBHOOK_DELIVER="")


def test_deliver_only_with_target_accepted():
    a = _adapter(WEBHOOK_DELIVER_ONLY="1", WEBHOOK_DELIVER="telegram")
    assert a.deliver_only is True
    assert a.deliver_target == "telegram"


def test_deliver_only_bool_parsing():
    # All truthy strings are accepted.
    for v in ("1", "true", "TRUE", "yes", "on", "True"):
        a = _adapter(WEBHOOK_DELIVER_ONLY=v, WEBHOOK_DELIVER="telegram")
        assert a.deliver_only is True


def test_account_id_passthrough():
    a = _adapter(WEBHOOK_ACCOUNT_ID="production")
    assert a.account_id == "production"


# ---- SSRF guard --------------------------------------------------------


def test_validate_callback_url_returns_none_for_public():
    assert wh.validate_callback_url("https://example.com") is None
    assert wh.validate_callback_url("https://8.8.8.8/path") is None


def test_validate_callback_url_rejects_loopback_ipv4():
    assert wh.validate_callback_url("http://127.0.0.1") is not None
    assert wh.validate_callback_url("http://127.255.255.255") is not None


def test_validate_callback_url_rejects_rfc1918_ipv4():
    assert wh.validate_callback_url("http://10.0.0.1") is not None
    assert wh.validate_callback_url("http://172.16.0.1") is not None
    assert wh.validate_callback_url("http://172.31.255.254") is not None
    assert wh.validate_callback_url("http://192.168.1.1") is not None


def test_validate_callback_url_rejects_link_local_ipv4():
    """169.254/16 covers AWS metadata at 169.254.169.254."""
    assert wh.validate_callback_url("http://169.254.169.254") is not None
    assert wh.validate_callback_url("http://169.254.1.1") is not None


def test_validate_callback_url_rejects_carrier_grade_nat():
    """RFC 6598 — 100.64/10 carrier-grade NAT shared addresses."""
    assert wh.validate_callback_url("http://100.64.0.1") is not None
    assert wh.validate_callback_url("http://100.127.255.254") is not None


def test_validate_callback_url_rejects_multicast():
    assert wh.validate_callback_url("http://224.0.0.1") is not None
    assert wh.validate_callback_url("http://239.255.255.255") is not None


def test_validate_callback_url_rejects_loopback_ipv6():
    assert wh.validate_callback_url("http://[::1]") is not None


def test_validate_callback_url_rejects_link_local_ipv6():
    assert wh.validate_callback_url("http://[fe80::1]") is not None


def test_validate_callback_url_rejects_ipv4_mapped_ipv6():
    """`::ffff:127.0.0.1` delivers to 127.0.0.1 on the wire — guard
    must catch this even though the IPv6 itself looks "global"."""
    assert wh.validate_callback_url("http://[::ffff:127.0.0.1]") is not None


def test_validate_callback_url_rejects_localhost_hostname():
    assert wh.validate_callback_url("http://localhost") is not None
    # FQDN trailing dot must not bypass.
    assert wh.validate_callback_url("http://localhost.") is not None


def test_validate_callback_url_rejects_kubernetes_internal():
    assert (
        wh.validate_callback_url("http://kubernetes.default.svc.cluster.local") is not None
    )


def test_validate_callback_url_rejects_non_http_scheme():
    assert wh.validate_callback_url("ftp://example.com") is not None
    assert wh.validate_callback_url("file:///etc/passwd") is not None
    assert wh.validate_callback_url("gopher://example.com") is not None


def test_validate_callback_url_rejects_empty():
    assert wh.validate_callback_url("") is not None


def test_validate_callback_url_rejects_no_host():
    assert wh.validate_callback_url("http:///path-no-host") is not None


# ---- signature helpers ---------------------------------------------


def test_compute_signature_sha256_hex_prefix():
    sig = wh.compute_signature(b"key", b"body")
    assert sig.startswith("sha256=")
    assert len(sig) == 7 + 64  # sha256= + 64 hex chars


def test_compute_signature_deterministic():
    s1 = wh.compute_signature(b"k", b"data")
    s2 = wh.compute_signature(b"k", b"data")
    assert s1 == s2


def test_verify_signature_valid():
    body = b"payload"
    sig = wh.compute_signature(b"key", body)
    assert wh.verify_webhook_signature(b"key", body, sig) is True


def test_verify_signature_wrong_key():
    body = b"payload"
    sig = wh.compute_signature(b"key-a", body)
    assert wh.verify_webhook_signature(b"key-b", body, sig) is False


def test_verify_signature_wrong_body():
    sig = wh.compute_signature(b"k", b"original")
    assert wh.verify_webhook_signature(b"k", b"tampered", sig) is False


def test_verify_signature_missing():
    assert wh.verify_webhook_signature(b"k", b"body", None) is False
    assert wh.verify_webhook_signature(b"k", b"body", "") is False


def test_verify_signature_wrong_prefix():
    body = b"payload"
    raw_hex = hmac.new(b"k", body, hashlib.sha256).hexdigest()
    # Bare hex without sha256= prefix → False (length mismatch).
    assert wh.verify_webhook_signature(b"k", body, raw_hex) is False
    # Wrong-but-same-length prefix → also False.
    assert (
        wh.verify_webhook_signature(b"k", body, f"sha512={raw_hex}") is False
    )


def test_verify_signature_short():
    """Length mismatch must reject — bypass guard for an attacker
    feeding a short prefix that happens to match the start of the
    expected hex."""
    expected = wh.compute_signature(b"k", b"body")
    assert wh.verify_webhook_signature(b"k", b"body", expected[:-1]) is False


# ---- parse_webhook_body --------------------------------------------


def test_parse_basic_message():
    p = wh.parse_webhook_body({
        "sender_id": "user-1",
        "sender_name": "Alice",
        "message": "hello",
        "thread_id": "t-1",
        "is_group": False,
        "metadata": {"k": "v"},
    })
    assert p["message"] == "hello"
    assert p["sender_id"] == "user-1"
    assert p["sender_name"] == "Alice"
    assert p["thread_id"] == "t-1"
    assert p["is_group"] is False
    assert p["metadata"] == {"k": "v"}


def test_parse_missing_message_returns_none():
    assert wh.parse_webhook_body({"sender_id": "u", "sender_name": "Alice"}) is None


def test_parse_empty_message_returns_none():
    assert wh.parse_webhook_body({"message": ""}) is None


def test_parse_default_sender_id():
    p = wh.parse_webhook_body({"message": "hi"})
    assert p["sender_id"] == "webhook-user"
    assert p["sender_name"] == "Webhook User"


def test_parse_falls_back_on_non_string_fields():
    """Defensive: malformed types coerce to defaults instead of
    raising."""
    p = wh.parse_webhook_body({
        "message": "hi",
        "sender_id": 42,
        "sender_name": None,
        "thread_id": 99,
        "is_group": "true-ish",
        "metadata": "not-a-dict",
    })
    assert p["sender_id"] == "webhook-user"
    assert p["sender_name"] == "Webhook User"
    assert p["thread_id"] is None
    # `bool("true-ish")` is truthy → is_group propagates that.
    assert p["is_group"] is True
    assert p["metadata"] == {}


def test_parse_non_dict_input_returns_none():
    assert wh.parse_webhook_body(None) is None
    assert wh.parse_webhook_body("string") is None
    assert wh.parse_webhook_body(42) is None
    assert wh.parse_webhook_body([1, 2]) is None


# ---- _verify_request end-to-end ------------------------------------


def _sig(secret: bytes, body: bytes) -> str:
    return wh.compute_signature(secret, body)


def test_verify_request_valid_sig_no_timestamp_ok():
    """No timestamp = sig-only fallback (with a WARN log)."""
    a = _adapter()
    body = b'{"message":"hi"}'
    sig = _sig(b"test-secret", body)
    ok, reason, status = a._verify_request(body, sig, None, now_secs=1_000_000)
    assert ok and status == 200


def test_verify_request_valid_sig_and_timestamp_ok():
    a = _adapter()
    body = b'{"message":"hi"}'
    sig = _sig(b"test-secret", body)
    now = 1_000_000
    ts_ms = now * 1000
    ok, _r, status = a._verify_request(body, sig, str(ts_ms), now)
    assert ok and status == 200


def test_verify_request_missing_signature():
    a = _adapter()
    ok, reason, status = a._verify_request(b"x", None, None, now_secs=1_000_000)
    assert not ok
    assert status == 403


def test_verify_request_empty_signature():
    a = _adapter()
    ok, _r, status = a._verify_request(b"x", "", None, now_secs=1_000_000)
    assert not ok
    assert status == 403


def test_verify_request_invalid_timestamp_header_returns_400():
    """Malformed timestamp header returns 400 (vs 403 for the
    auth-rejection paths) so the operator can tell "client bug"
    from "auth probe"."""
    a = _adapter()
    body = b'{}'
    sig = _sig(b"test-secret", body)
    ok, _r, status = a._verify_request(body, sig, "not-a-number", now_secs=1_000_000)
    assert not ok
    assert status == 400


def test_verify_request_stale_timestamp_rejected():
    """±5 min skew tolerance; 10 minutes old → reject."""
    a = _adapter()
    body = b'{}'
    sig = _sig(b"test-secret", body)
    now = 1_000_000
    stale_ms = (now - 600) * 1000  # 10 min old
    ok, reason, status = a._verify_request(body, sig, str(stale_ms), now)
    assert not ok
    assert "too old" in reason
    assert status == 403


def test_verify_request_future_timestamp_rejected():
    a = _adapter()
    body = b'{}'
    sig = _sig(b"test-secret", body)
    now = 1_000_000
    future_ms = (now + 600) * 1000  # 10 min in the future
    ok, reason, status = a._verify_request(body, sig, str(future_ms), now)
    assert not ok
    assert "future" in reason
    assert status == 403


def test_verify_request_wrong_sig_rejected():
    a = _adapter()
    body = b'{}'
    bad_sig = _sig(b"wrong-secret", body)
    ok, reason, status = a._verify_request(body, bad_sig, None, now_secs=1_000_000)
    assert not ok
    assert "invalid signature" in reason
    assert status == 403


def test_verify_request_skew_boundary():
    """Exactly at ±300s skew is OK; 301s is not."""
    a = _adapter()
    body = b'{}'
    sig = _sig(b"test-secret", body)
    now = 1_000_000
    # Exactly at the boundary
    boundary_ms = (now - 300) * 1000
    ok, _r, status = a._verify_request(body, sig, str(boundary_ms), now)
    assert ok and status == 200
    # One second past
    past_ms = (now - 301) * 1000
    ok, _r, status = a._verify_request(body, sig, str(past_ms), now)
    assert not ok


# ---- _handle_webhook_body end-to-end -------------------------------


def test_handle_webhook_body_happy_path():
    a = _adapter()
    body = json.dumps({"message": "hello", "sender_id": "u1"}).encode("utf-8")
    sig = _sig(b"test-secret", body)
    emitted: list = []
    status = a._handle_webhook_body(body, sig, None, lambda ev: emitted.append(ev))
    assert status == 200
    assert len(emitted) == 1
    params = emitted[0]["params"]
    assert params["user_id"] == "u1"
    assert params["content"]["Text"] == "hello"


def test_handle_webhook_body_slash_command():
    a = _adapter()
    body = json.dumps({"message": "/help me", "sender_id": "u"}).encode("utf-8")
    sig = _sig(b"test-secret", body)
    emitted: list = []
    a._handle_webhook_body(body, sig, None, lambda ev: emitted.append(ev))
    content = emitted[0]["params"]["content"]
    assert "Command" in content
    assert content["Command"]["name"] == "help"
    assert content["Command"]["args"] == ["me"]


def test_handle_webhook_body_slash_command_no_args():
    a = _adapter()
    body = json.dumps({"message": "/ping", "sender_id": "u"}).encode("utf-8")
    sig = _sig(b"test-secret", body)
    emitted: list = []
    a._handle_webhook_body(body, sig, None, lambda ev: emitted.append(ev))
    content = emitted[0]["params"]["content"]
    assert content["Command"]["name"] == "ping"
    assert content["Command"]["args"] == []


def test_handle_webhook_body_dedupes_message_id():
    """When the inbound carries `metadata.message_id`, repeated
    deliveries with the same ID drop to one emit."""
    a = _adapter()
    body = json.dumps({
        "message": "hi", "sender_id": "u",
        "metadata": {"message_id": "fixed-id"},
    }).encode("utf-8")
    sig = _sig(b"test-secret", body)
    emitted: list = []
    s1 = a._handle_webhook_body(body, sig, None, lambda ev: emitted.append(ev))
    s2 = a._handle_webhook_body(body, sig, None, lambda ev: emitted.append(ev))
    assert s1 == 200 and s2 == 200
    assert len(emitted) == 1


def test_handle_webhook_body_account_id_injection():
    a = _adapter(WEBHOOK_ACCOUNT_ID="prod")
    body = json.dumps({"message": "hi", "sender_id": "u"}).encode("utf-8")
    sig = _sig(b"test-secret", body)
    emitted: list = []
    a._handle_webhook_body(body, sig, None, lambda ev: emitted.append(ev))
    assert emitted[0]["params"]["metadata"]["account_id"] == "prod"


def test_handle_webhook_body_deliver_only_metadata():
    a = _adapter(WEBHOOK_DELIVER_ONLY="1", WEBHOOK_DELIVER="telegram")
    body = json.dumps({"message": "hi", "sender_id": "u"}).encode("utf-8")
    sig = _sig(b"test-secret", body)
    emitted: list = []
    a._handle_webhook_body(body, sig, None, lambda ev: emitted.append(ev))
    meta = emitted[0]["params"]["metadata"]
    assert meta["__deliver_only__"] is True
    assert meta["__deliver_target__"] == "telegram"


def test_handle_webhook_body_is_group_propagates():
    """`is_group` MUST land at the top level of params (the sidecar
    protocol's Message struct deserialises it from the top — see
    crates/librefang-channels/src/sidecar.rs:99-100). If we stuff
    it into metadata instead, the kernel's `msg.is_group` stays
    `false` and group conversations are silently mis-routed as
    DMs."""
    a = _adapter()
    body = json.dumps({
        "message": "hi", "sender_id": "u", "is_group": True,
    }).encode("utf-8")
    sig = _sig(b"test-secret", body)
    emitted: list = []
    a._handle_webhook_body(body, sig, None, lambda ev: emitted.append(ev))
    assert emitted[0]["params"]["is_group"] is True
    # And NOT in metadata — that was the bug shape before the fix.
    assert "is_group" not in emitted[0]["params"].get("metadata", {})


def test_handle_webhook_body_dm_does_not_set_is_group():
    """The omit-default contract: when `is_group=False`, the field
    is NOT present in params (mirrors protocol.message kwarg
    handling, which only emits `is_group` when truthy)."""
    a = _adapter()
    body = json.dumps({
        "message": "hi", "sender_id": "u",  # is_group implicit false
    }).encode("utf-8")
    sig = _sig(b"test-secret", body)
    emitted: list = []
    a._handle_webhook_body(body, sig, None, lambda ev: emitted.append(ev))
    assert "is_group" not in emitted[0]["params"]


def test_handle_webhook_body_invalid_signature_403():
    a = _adapter()
    body = b'{"message":"hi"}'
    bad_sig = _sig(b"wrong-secret", body)
    emitted: list = []
    status = a._handle_webhook_body(body, bad_sig, None, lambda ev: emitted.append(ev))
    assert status == 403
    assert emitted == []


def test_handle_webhook_body_malformed_json_400():
    a = _adapter()
    body = b"{not-json"
    sig = _sig(b"test-secret", body)
    status = a._handle_webhook_body(body, sig, None, lambda _: None)
    assert status == 400


def test_handle_webhook_body_empty_message_returns_200():
    """Empty `message` is benign — return 200 so caller doesn't
    retry (matches the Rust contract at webhook.rs:200-204)."""
    a = _adapter()
    body = json.dumps({"message": ""}).encode("utf-8")
    sig = _sig(b"test-secret", body)
    emitted: list = []
    status = a._handle_webhook_body(body, sig, None, lambda ev: emitted.append(ev))
    assert status == 200
    assert emitted == []


def test_handle_webhook_body_replay_attack_with_old_timestamp():
    """A valid signature on a 10-minute-old request still rejects."""
    a = _adapter()
    body = json.dumps({"message": "old"}).encode("utf-8")
    sig = _sig(b"test-secret", body)
    # Simulate the request from 10 minutes ago — we send the
    # signature, but the timestamp header places it outside the
    # skew window.
    old_ts_ms = (int(time.time()) - 600) * 1000
    emitted: list = []
    status = a._handle_webhook_body(body, sig, str(old_ts_ms), lambda ev: emitted.append(ev))
    assert status == 403
    assert emitted == []


# ---- outbound _post_chunk ------------------------------------------


def test_send_text_basic(monkeypatch):
    sent: list = []

    def _fake_http(url, **kw):
        sent.append((url, json.loads(kw["body"].decode("utf-8")), kw.get("headers", {})))
        return (200, {}, b"", {})

    monkeypatch.setattr(wh, "_http_request", _fake_http)
    a = _adapter(WEBHOOK_CALLBACK_URL="https://example.com/in")
    a._send_text("user-1", "Alice", "hello")
    assert len(sent) == 1
    url, body, headers = sent[0]
    assert url == "https://example.com/in"
    assert body["message"] == "hello"
    assert body["recipient_id"] == "user-1"
    assert body["recipient_name"] == "Alice"
    assert headers.get("X-Webhook-Signature", "").startswith("sha256=")


def test_send_text_chunks_long_message(monkeypatch):
    monkeypatch.setattr(wh, "MAX_MESSAGE_LEN", 5)
    monkeypatch.setattr(wh, "INTER_CHUNK_DELAY_SECS", 0)
    sent: list = []
    monkeypatch.setattr(
        wh, "_http_request",
        lambda url, **kw: (
            sent.append(json.loads(kw["body"].decode("utf-8"))["message"]),
            (200, {}, b"", {}),
        )[1],
    )
    a = _adapter(WEBHOOK_CALLBACK_URL="https://example.com/in")
    a._send_text("u", "n", "abcdefghijk")
    assert len(sent) >= 2


def test_send_text_no_callback_logs_and_drops(monkeypatch):
    """deliver_only-mode setups don't have a callback URL.
    Reply attempts should log + drop, not raise."""
    a = _adapter(WEBHOOK_DELIVER_ONLY="1", WEBHOOK_DELIVER="telegram")
    calls: list = []
    monkeypatch.setattr(
        wh, "_http_request",
        lambda url, **kw: (calls.append(url), (200, {}, b"", {}))[1],
    )
    # Must not raise.
    a._send_text("u", "n", "hi")
    assert calls == []


def test_send_text_signature_matches_body(monkeypatch):
    """Receiver must be able to re-compute and verify the
    signature we send. Round-trip test."""
    captured: list = []

    def _fake_http(url, **kw):
        captured.append((kw["body"], kw["headers"]["X-Webhook-Signature"]))
        return (200, {}, b"", {})

    monkeypatch.setattr(wh, "_http_request", _fake_http)
    a = _adapter(WEBHOOK_CALLBACK_URL="https://example.com/in")
    a._send_text("u", "n", "hello")
    body, sig = captured[0]
    # Receiver-side verification with the same secret must pass.
    assert wh.verify_webhook_signature(b"test-secret", body, sig)


def test_send_text_429_retries_once(monkeypatch):
    responses = [
        (429, None, b"slow down", {"retry-after": "0"}),
        (200, {}, b"", {}),
    ]
    calls: list = []

    def _fake_http(url, **kw):
        calls.append(url)
        return responses.pop(0)

    monkeypatch.setattr(wh, "_http_request", _fake_http)
    monkeypatch.setattr(wh, "_parse_retry_after", lambda h, **kw: 0.0)
    a = _adapter(WEBHOOK_CALLBACK_URL="https://example.com/in")
    a._send_text("u", "n", "hi")
    assert len(calls) == 2


def test_send_text_non_2xx_raises(monkeypatch):
    monkeypatch.setattr(
        wh, "_http_request",
        lambda url, **kw: (500, None, b"server boom", {}),
    )
    a = _adapter(WEBHOOK_CALLBACK_URL="https://example.com/in")
    with pytest.raises(RuntimeError, match="callback error"):
        a._send_text("u", "n", "hi")


def test_send_text_empty_text_drops(monkeypatch):
    calls: list = []
    monkeypatch.setattr(
        wh, "_http_request",
        lambda url, **kw: (calls.append(url), (200, {}, b"", {}))[1],
    )
    a = _adapter(WEBHOOK_CALLBACK_URL="https://example.com/in")
    a._send_text("u", "n", "")
    assert calls == []


# ---- on_send dispatch ----------------------------------------------


def _send_cmd(channel_id="user-1", text="hi", content=None,
              thread_id=None, user=None):
    from librefang.sidecar.protocol import Send
    return Send(channel_id, text, content, thread_id, user or {})


@pytest.mark.asyncio
async def test_on_send_basic_text(monkeypatch):
    sent: list = []
    monkeypatch.setattr(
        wh, "_http_request",
        lambda url, **kw: (
            sent.append(json.loads(kw["body"].decode("utf-8"))),
            (200, {}, b"", {}),
        )[1],
    )
    a = _adapter(WEBHOOK_CALLBACK_URL="https://example.com/in")
    await a.on_send(_send_cmd(content={"Text": "hello"}))
    assert sent[0]["message"] == "hello"
    assert sent[0]["recipient_id"] == "user-1"


@pytest.mark.asyncio
async def test_on_send_user_platform_id_fallback(monkeypatch):
    sent: list = []
    monkeypatch.setattr(
        wh, "_http_request",
        lambda url, **kw: (
            sent.append(json.loads(kw["body"].decode("utf-8"))),
            (200, {}, b"", {}),
        )[1],
    )
    a = _adapter(WEBHOOK_CALLBACK_URL="https://example.com/in")
    await a.on_send(_send_cmd(
        channel_id="", content={"Text": "hi"},
        user={"platform_id": "fallback-user", "display_name": "Fallback"},
    ))
    assert sent[0]["recipient_id"] == "fallback-user"
    assert sent[0]["recipient_name"] == "Fallback"


@pytest.mark.asyncio
async def test_on_send_empty_user_drops(monkeypatch):
    calls: list = []
    monkeypatch.setattr(
        wh, "_http_request",
        lambda url, **kw: (calls.append(url), (200, {}, b"", {}))[1],
    )
    a = _adapter(WEBHOOK_CALLBACK_URL="https://example.com/in")
    await a.on_send(_send_cmd(channel_id="", user={}))
    assert calls == []


@pytest.mark.asyncio
async def test_on_send_unsupported_content_placeholder(monkeypatch):
    sent: list = []
    monkeypatch.setattr(
        wh, "_http_request",
        lambda url, **kw: (
            sent.append(json.loads(kw["body"].decode("utf-8"))),
            (200, {}, b"", {}),
        )[1],
    )
    a = _adapter(WEBHOOK_CALLBACK_URL="https://example.com/in")
    await a.on_send(_send_cmd(
        text="", content={"Image": {"url": "https://x"}},
    ))
    assert sent[0]["message"] == "(Unsupported content type)"


# ---- schema + capabilities -----------------------------------------


def test_schema_exposes_required_envs():
    schema = wh.WebhookAdapter.SCHEMA.to_dict()
    keys = {f["key"] for f in schema["fields"]}
    expected = {
        "WEBHOOK_SECRET",
        "WEBHOOK_LISTEN_PORT",
        "WEBHOOK_LISTEN_PATH",
        "WEBHOOK_CALLBACK_URL",
        "WEBHOOK_DELIVER_ONLY",
        "WEBHOOK_DELIVER",
        "WEBHOOK_ACCOUNT_ID",
    }
    assert expected.issubset(keys)
    secrets = {f["key"] for f in schema["fields"] if f["type"] == "secret"}
    assert "WEBHOOK_SECRET" in secrets


def test_capabilities_text_only():
    # Webhook has no typing / reaction / thread surface in the
    # Rust adapter — sidecar preserves that.
    assert wh.WebhookAdapter.capabilities == []
