"""Tests for librefang.sidecar.adapters.signal.

Deterministic, no network: urllib is monkeypatched, polling worker
loop is exercised through ``_poll_once`` / ``_producer_blocking``
without binding a real socket, SSRF DNS lookups go through a
monkeypatched ``socket.getaddrinfo``. Asserts the sidecar preserves
the in-process Rust ``librefang-channels::signal`` adapter's
behaviour plus the four improvements documented in the module header
(429 Retry-After, timestamp dedupe, explicit HTTP timeouts,
exponential backoff).
"""

import io
import json
import os
import socket
import urllib.error

import pytest


os.environ.setdefault("SIGNAL_API_URL", "https://signal.test")
os.environ.setdefault("SIGNAL_NUMBER", "+15555550100")
os.environ.setdefault("SIGNAL_ALLOW_LOCAL", "1")
from librefang.sidecar.adapters import signal as sg  # noqa: E402

from _sidecar_fakes import _FakeResp, _FakeUrlopen, _HdrShim


# ---- _FakeUrlopen scaffolding ----------------------------------------


def _adapter(**env):
    defaults = {
        "SIGNAL_API_URL": "https://signal.test",
        "SIGNAL_NUMBER": "+15555550100",
        "SIGNAL_API_KEY": "",
        "SIGNAL_ALLOWED_USERS": "",
        "SIGNAL_ACCOUNT_ID": "",
        "SIGNAL_POLL_INTERVAL_SECS": "",
        "SIGNAL_ALLOW_LOCAL": "1",  # default-on for tests; pull off explicitly
    }
    for k, v in defaults.items():
        os.environ[k] = env.get(k, v)
    return sg.SignalAdapter()


# ---- env handling ----------------------------------------------------


def test_default_env_construction():
    a = _adapter()
    assert a.api_url == "https://signal.test"
    assert a.phone_number == "+15555550100"
    assert a.api_key is None
    assert a.allowed_users == []
    assert a.account_id is None
    assert a.poll_interval == sg.DEFAULT_POLL_INTERVAL_SECS


def test_api_url_trailing_slash_stripped():
    a = _adapter(SIGNAL_API_URL="https://signal.test/")
    assert a.api_url == "https://signal.test"


def test_api_key_picked_up():
    a = _adapter(SIGNAL_API_KEY="api-key-xyz")
    assert a.api_key == "api-key-xyz"


def test_allowed_users_split():
    a = _adapter(SIGNAL_ALLOWED_USERS="+15555550199, +15555550200 ,, +15555550201")
    assert a.allowed_users == ["+15555550199", "+15555550200", "+15555550201"]


def test_account_id_passthrough():
    a = _adapter(SIGNAL_ACCOUNT_ID="prod-bot")
    assert a.account_id == "prod-bot"


def test_account_id_empty_is_none():
    a = _adapter(SIGNAL_ACCOUNT_ID="")
    assert a.account_id is None


def test_poll_interval_override():
    a = _adapter(SIGNAL_POLL_INTERVAL_SECS="5")
    assert a.poll_interval == 5.0


def test_poll_interval_floor_at_half_second():
    a = _adapter(SIGNAL_POLL_INTERVAL_SECS="0.01")
    assert a.poll_interval == 0.5


def test_poll_interval_invalid_falls_back_to_default():
    a = _adapter(SIGNAL_POLL_INTERVAL_SECS="garbage")
    assert a.poll_interval == sg.DEFAULT_POLL_INTERVAL_SECS


def test_missing_api_url_exits_2():
    os.environ["SIGNAL_API_URL"] = ""
    os.environ["SIGNAL_NUMBER"] = "+15555550100"
    with pytest.raises(SystemExit) as exc:
        sg.SignalAdapter()
    assert exc.value.code == 2
    os.environ["SIGNAL_API_URL"] = "https://signal.test"


def test_missing_number_exits_2():
    os.environ["SIGNAL_NUMBER"] = ""
    with pytest.raises(SystemExit) as exc:
        sg.SignalAdapter()
    assert exc.value.code == 2
    os.environ["SIGNAL_NUMBER"] = "+15555550100"


def test_loopback_url_rejected_without_allow_local():
    os.environ["SIGNAL_API_URL"] = "http://127.0.0.1:8080"
    os.environ["SIGNAL_ALLOW_LOCAL"] = "0"
    with pytest.raises(SystemExit) as exc:
        sg.SignalAdapter()
    assert exc.value.code == 2
    os.environ["SIGNAL_API_URL"] = "https://signal.test"
    os.environ["SIGNAL_ALLOW_LOCAL"] = "1"


def test_loopback_url_allowed_with_allow_local():
    os.environ["SIGNAL_API_URL"] = "http://127.0.0.1:8080"
    os.environ["SIGNAL_ALLOW_LOCAL"] = "1"
    a = sg.SignalAdapter()
    assert a.api_url == "http://127.0.0.1:8080"
    os.environ["SIGNAL_API_URL"] = "https://signal.test"


# ---- SSRF guard ------------------------------------------------------


def test_ssrf_reject_loopback():
    err = sg.validate_api_url("http://127.0.0.1:8080", allow_local=False)
    assert err is not None
    assert "private/loopback" in err or "127" in err


def test_ssrf_reject_private_v4():
    for url in (
        "http://192.168.1.1/api",
        "http://10.0.0.1/api",
        "http://172.16.0.1/api",
    ):
        err = sg.validate_api_url(url, allow_local=False)
        assert err is not None, url


def test_ssrf_reject_link_local():
    err = sg.validate_api_url("http://169.254.169.254/", allow_local=False)
    assert err is not None


def test_ssrf_reject_cgnat():
    """100.64.0.0/10 — carrier-grade NAT — must be blocked."""
    err = sg.validate_api_url("http://100.64.0.1/", allow_local=False)
    assert err is not None


def test_ssrf_reject_unspecified():
    err = sg.validate_api_url("http://0.0.0.0/", allow_local=False)
    assert err is not None


def test_ssrf_reject_bad_scheme():
    err = sg.validate_api_url("ftp://example.com/api", allow_local=False)
    assert err is not None
    assert "scheme" in err


def test_ssrf_reject_file_scheme():
    err = sg.validate_api_url("file:///etc/passwd", allow_local=False)
    assert err is not None


def test_ssrf_allow_local_bypasses_private_check():
    assert sg.validate_api_url(
        "http://127.0.0.1:8080", allow_local=True,
    ) is None
    assert sg.validate_api_url(
        "http://10.0.0.1/api", allow_local=True,
    ) is None


def test_ssrf_accepts_public_dns(monkeypatch):
    """A public-looking hostname must pass. We stub
    ``socket.getaddrinfo`` so the test is deterministic and offline."""
    def fake_getaddrinfo(host, port, *_a, **_k):
        return [(socket.AF_INET, socket.SOCK_STREAM, 0, "", ("93.184.216.34", port))]
    monkeypatch.setattr(sg.socket, "getaddrinfo", fake_getaddrinfo)
    assert sg.validate_api_url("https://example.com/", allow_local=False) is None


def test_ssrf_blocks_dns_to_private(monkeypatch):
    """A public-looking hostname that resolves to a private IP must
    be rejected — the SSRF guard's job is to defend the DNS-resolved
    address, not just the literal."""
    def fake_getaddrinfo(host, port, *_a, **_k):
        return [(socket.AF_INET, socket.SOCK_STREAM, 0, "", ("10.0.0.5", port))]
    monkeypatch.setattr(sg.socket, "getaddrinfo", fake_getaddrinfo)
    err = sg.validate_api_url("https://attacker.test/", allow_local=False)
    assert err is not None
    assert "10.0.0.5" in err


def test_is_private_or_loopback_classifier():
    """Direct hits on the classifier — mirrors the Rust unit test."""
    assert sg._is_private_or_loopback("127.0.0.1")
    assert sg._is_private_or_loopback("192.168.1.1")
    assert sg._is_private_or_loopback("10.0.0.1")
    assert sg._is_private_or_loopback("172.16.0.1")
    assert sg._is_private_or_loopback("169.254.169.254")
    assert sg._is_private_or_loopback("100.64.0.1")
    assert sg._is_private_or_loopback("::1")
    assert sg._is_private_or_loopback("fe80::1")
    assert sg._is_private_or_loopback("fc00::1")
    assert not sg._is_private_or_loopback("1.1.1.1")
    assert not sg._is_private_or_loopback("8.8.8.8")
    assert not sg._is_private_or_loopback("2606:4700:4700::1111")


def test_is_private_or_loopback_fails_closed_on_garbage():
    """SSRF guard contract: any address string the classifier cannot
    parse must be treated as private (default-deny). A future change
    that lets `socket.getaddrinfo` hand back a scoped IPv6 literal
    like ``fe80::1%eth0`` (which `ipaddress.ip_address` rejects) MUST
    NOT slip through as 'public, allow'."""
    assert sg._is_private_or_loopback("not-an-ip")
    assert sg._is_private_or_loopback("")
    assert sg._is_private_or_loopback("fe80::1%eth0")
    assert sg._is_private_or_loopback("999.999.999.999")


# ---- _parse_retry_after ----------------------------------------------


def test_retry_after_missing_returns_default():
    assert sg._parse_retry_after({}, default_secs=30.0) == 30.0


def test_retry_after_parses_seconds():
    assert sg._parse_retry_after(
        {"retry-after": "12"}, default_secs=30.0,
    ) == 12.0


def test_retry_after_floor_one_second():
    assert sg._parse_retry_after(
        {"retry-after": "0"}, default_secs=30.0,
    ) == 1.0


def test_retry_after_caps_at_max_backoff():
    assert sg._parse_retry_after(
        {"retry-after": "9999"}, default_secs=30.0,
    ) == sg.MAX_BACKOFF_SECS


def test_retry_after_garbage_returns_default():
    assert sg._parse_retry_after(
        {"retry-after": "junk"}, default_secs=30.0,
    ) == 30.0


# ---- parse_signal_envelope ------------------------------------------


def _envelope(*, source="+15555550199", text="hello", ts=1000,
              source_name="Alice"):
    return {
        "envelope": {
            "source": source,
            "sourceName": source_name,
            "timestamp": ts,
            "dataMessage": {"message": text},
        }
    }


def test_parse_basic_text_message():
    ev = sg.parse_signal_envelope(
        _envelope(),
        own_phone="+15555550100",
        allowed_users=[],
        account_id=None,
    )
    assert ev is not None
    assert ev["method"] == "message"
    p = ev["params"]
    assert p["user_id"] == "+15555550199"
    assert p["user_name"] == "Alice"
    assert p["message_id"] == "1000"
    assert p["content"] == {"Text": "hello"}
    assert p.get("is_group") is not True  # 1:1 chat


def test_parse_unwraps_envelope_when_present():
    """signal-cli-rest-api sometimes wraps the envelope under an
    ``envelope`` key, sometimes returns it bare. The parser must
    handle both shapes (mirrors signal.rs:359)."""
    bare = {
        "source": "+15555550199",
        "sourceName": "Bob",
        "timestamp": 999,
        "dataMessage": {"message": "hi from bare"},
    }
    ev = sg.parse_signal_envelope(
        bare,
        own_phone="+15555550100",
        allowed_users=[],
        account_id=None,
    )
    assert ev is not None
    assert ev["params"]["user_id"] == "+15555550199"
    assert ev["params"]["content"] == {"Text": "hi from bare"}


def test_parse_self_message_dropped():
    ev = sg.parse_signal_envelope(
        _envelope(source="+15555550100"),
        own_phone="+15555550100",
        allowed_users=[],
        account_id=None,
    )
    assert ev is None


def test_parse_allowlist_blocks_others():
    ev = sg.parse_signal_envelope(
        _envelope(source="+15555550199"),
        own_phone="+15555550100",
        allowed_users=["+15555559999"],
        account_id=None,
    )
    assert ev is None


def test_parse_allowlist_passes_match():
    ev = sg.parse_signal_envelope(
        _envelope(source="+15555550199"),
        own_phone="+15555550100",
        allowed_users=["+15555550199"],
        account_id=None,
    )
    assert ev is not None


def test_parse_empty_text_returns_none():
    ev = sg.parse_signal_envelope(
        _envelope(text=""),
        own_phone="+15555550100",
        allowed_users=[],
        account_id=None,
    )
    assert ev is None


def test_parse_missing_data_message_returns_none():
    ev = sg.parse_signal_envelope(
        {"envelope": {"source": "+15555550199", "timestamp": 1}},
        own_phone="+15555550100",
        allowed_users=[],
        account_id=None,
    )
    assert ev is None


def test_parse_missing_source_returns_none():
    ev = sg.parse_signal_envelope(
        {"envelope": {"dataMessage": {"message": "hi"}}},
        own_phone="+15555550100",
        allowed_users=[],
        account_id=None,
    )
    assert ev is None


def test_parse_source_name_falls_back_to_source():
    ev = sg.parse_signal_envelope(
        _envelope(source_name=""),
        own_phone="+15555550100",
        allowed_users=[],
        account_id=None,
    )
    assert ev["params"]["user_name"] == "+15555550199"


def test_parse_slash_command_routes_as_command():
    ev = sg.parse_signal_envelope(
        _envelope(text="/status all systems"),
        own_phone="+15555550100",
        allowed_users=[],
        account_id=None,
    )
    assert ev["params"]["content"] == {
        "Command": {"name": "status", "args": ["all", "systems"]}
    }


def test_parse_slash_command_no_args_emits_empty_list():
    ev = sg.parse_signal_envelope(
        _envelope(text="/ping"),
        own_phone="+15555550100",
        allowed_users=[],
        account_id=None,
    )
    assert ev["params"]["content"] == {"Command": {"name": "ping", "args": []}}


def test_parse_account_id_injected_when_set():
    ev = sg.parse_signal_envelope(
        _envelope(),
        own_phone="+15555550100",
        allowed_users=[],
        account_id="prod-bot",
    )
    assert ev["params"]["metadata"]["account_id"] == "prod-bot"


def test_parse_account_id_omitted_when_unset():
    ev = sg.parse_signal_envelope(
        _envelope(),
        own_phone="+15555550100",
        allowed_users=[],
        account_id=None,
    )
    assert "account_id" not in (ev["params"].get("metadata") or {})


# ---- _mark_seen ------------------------------------------------------


def test_mark_seen_first_returns_true_second_returns_false():
    a = _adapter()
    assert a._mark_seen("1000") is True
    assert a._mark_seen("1000") is False


def test_mark_seen_empty_id_returns_true_no_state_change():
    a = _adapter()
    assert a._mark_seen("") is True
    assert a._mark_seen(None) is True  # type: ignore[arg-type]
    assert "" not in a._seen_ids


def test_mark_seen_eviction_at_cap(monkeypatch):
    monkeypatch.setattr(sg, "SEEN_MESSAGES_MAX", 10)
    monkeypatch.setattr(sg, "SEEN_MESSAGES_EVICT", 4)
    a = _adapter()
    for i in range(11):
        a._mark_seen(f"ts-{i}")
    assert "ts-0" not in a._seen_ids
    assert "ts-3" not in a._seen_ids
    assert "ts-4" in a._seen_ids
    assert "ts-10" in a._seen_ids


# ---- _poll_once ------------------------------------------------------


def test_poll_once_returns_envelopes_on_200(monkeypatch):
    fake = _FakeUrlopen([(200, [_envelope()])])
    monkeypatch.setattr(sg.urllib.request, "urlopen", fake)
    a = _adapter()
    envelopes, retry = a._poll_once()
    assert retry is None
    assert isinstance(envelopes, list) and len(envelopes) == 1
    c = fake.calls[0]
    assert "/v1/receive/" in c["url"]
    # The Rust adapter used reqwest's untouched URL path (signal.rs:336);
    # `+` was never percent-encoded. Sidecar mirrors with
    # ``urllib.parse.quote(..., safe='+')`` so the raw phone number
    # rides through unchanged — signal-cli-rest-api parses
    # ``/v1/receive/+1555...`` directly.
    assert c["url"].endswith("/+15555550100")
    assert c["headers"].get("authorization") is None  # no api_key set
    assert c["timeout"] == sg.POLL_TIMEOUT_SECS


def test_poll_once_attaches_bearer_when_api_key_set(monkeypatch):
    fake = _FakeUrlopen([(200, [])])
    monkeypatch.setattr(sg.urllib.request, "urlopen", fake)
    a = _adapter(SIGNAL_API_KEY="api-key-xyz")
    a._poll_once()
    assert fake.calls[0]["headers"]["authorization"] == "Bearer api-key-xyz"


def test_poll_once_429_returns_retry_after(monkeypatch):
    fake = _FakeUrlopen([(429, {}, {"Retry-After": "5"})])
    monkeypatch.setattr(sg.urllib.request, "urlopen", fake)
    a = _adapter()
    envelopes, retry = a._poll_once()
    assert envelopes is None
    assert retry == 5.0


def test_poll_once_5xx_returns_none(monkeypatch):
    fake = _FakeUrlopen([(503, {"error": "down"})])
    monkeypatch.setattr(sg.urllib.request, "urlopen", fake)
    a = _adapter()
    envelopes, retry = a._poll_once()
    assert envelopes is None
    assert retry is None


def test_poll_once_non_array_returns_none(monkeypatch):
    """signal-cli-rest-api always responds with an array on 200; a
    rogue response (an object, a number) must not crash the parser
    — return ``None`` so the producer applies backoff."""
    fake = _FakeUrlopen([(200, {"unexpected": "object"})])
    monkeypatch.setattr(sg.urllib.request, "urlopen", fake)
    a = _adapter()
    envelopes, _ = a._poll_once()
    assert envelopes is None


# ---- _post_send -----------------------------------------------------


def test_post_send_basic(monkeypatch):
    fake = _FakeUrlopen([(201, {})])
    monkeypatch.setattr(sg.urllib.request, "urlopen", fake)
    a = _adapter()
    a._post_send("+15555550199", "hello")
    c = fake.calls[0]
    assert c["url"].endswith("/v2/send")
    assert c["method"] == "POST"
    body = json.loads(c["body_raw"])
    assert body == {
        "message": "hello",
        "number": "+15555550100",
        "recipients": ["+15555550199"],
    }


def test_post_send_429_then_201_retries_once(monkeypatch):
    sleeps = []
    monkeypatch.setattr(sg.time, "sleep", lambda s: sleeps.append(s))
    fake = _FakeUrlopen([
        (429, {}, {"Retry-After": "3"}),
        (201, {}),
    ])
    monkeypatch.setattr(sg.urllib.request, "urlopen", fake)
    a = _adapter()
    a._post_send("+15555550199", "hi")
    assert sleeps == [3.0]
    assert len(fake.calls) == 2


def test_post_send_persistent_429_is_fail_open(monkeypatch):
    """Second 429 logs and continues — matches webex / line /
    mattermost. The producer keeps polling, the operator sees a
    warning."""
    monkeypatch.setattr(sg.time, "sleep", lambda _s: None)
    fake = _FakeUrlopen([
        (429, {}, {}),
        (429, {}, {}),
    ])
    monkeypatch.setattr(sg.urllib.request, "urlopen", fake)
    a = _adapter()
    a._post_send("+15555550199", "hi")  # must not raise
    assert len(fake.calls) == 2


def test_post_send_empty_recipient_is_noop(monkeypatch):
    fake = _FakeUrlopen([])
    monkeypatch.setattr(sg.urllib.request, "urlopen", fake)
    a = _adapter()
    a._post_send("", "hi")
    assert fake.calls == []


# ---- on_send --------------------------------------------------------


def _send_cmd(channel_id="+15555550199", text="hi", content=None,
              thread_id=None, user=None):
    from librefang.sidecar.protocol import Send
    return Send(channel_id, text, content, thread_id, user or {})


@pytest.mark.asyncio
async def test_on_send_text(monkeypatch):
    fake = _FakeUrlopen([(201, {})])
    monkeypatch.setattr(sg.urllib.request, "urlopen", fake)
    a = _adapter()
    await a.on_send(_send_cmd(text="hello", content={"Text": "hello"}))
    body = json.loads(fake.calls[0]["body_raw"])
    assert body["message"] == "hello"
    assert body["recipients"] == ["+15555550199"]


@pytest.mark.asyncio
async def test_on_send_unsupported_content_falls_back_to_placeholder(monkeypatch):
    """The Rust adapter supported attachments inline; the sidecar
    keeps the surface small and routes anything other than text to
    the same ``(Unsupported content type)`` placeholder we use across
    sidecars. A future follow-up can wire up base64_attachments."""
    fake = _FakeUrlopen([(201, {})])
    monkeypatch.setattr(sg.urllib.request, "urlopen", fake)
    a = _adapter()
    await a.on_send(_send_cmd(
        text="",
        content={"Image": {"url": "https://x/y.jpg", "caption": None,
                            "mime_type": None}},
    ))
    body = json.loads(fake.calls[0]["body_raw"])
    assert body["message"] == "(Unsupported content type)"


@pytest.mark.asyncio
async def test_on_send_empty_recipient_drops_silently(monkeypatch):
    fake = _FakeUrlopen([])
    monkeypatch.setattr(sg.urllib.request, "urlopen", fake)
    a = _adapter()
    await a.on_send(_send_cmd(channel_id="", user={}))
    assert fake.calls == []


@pytest.mark.asyncio
async def test_on_send_falls_back_to_user_platform_id(monkeypatch):
    fake = _FakeUrlopen([(201, {})])
    monkeypatch.setattr(sg.urllib.request, "urlopen", fake)
    a = _adapter()
    await a.on_send(_send_cmd(
        channel_id="",
        text="hi",
        content={"Text": "hi"},
        user={"platform_id": "+15555550199"},
    ))
    body = json.loads(fake.calls[0]["body_raw"])
    assert body["recipients"] == ["+15555550199"]


# ---- producer integration: dedupe path ------------------------------


def test_producer_dedupes_repeated_timestamp(monkeypatch):
    """End-to-end: when ``_poll_once`` returns the same timestamp
    twice in successive ticks, ``emit`` is only called once. Drives
    the producer for two ticks and then asks it to stop."""
    fake = _FakeUrlopen([
        (200, [_envelope(ts=1000)]),
        (200, [_envelope(ts=1000)]),  # redelivery on reconnect
    ])
    monkeypatch.setattr(sg.urllib.request, "urlopen", fake)
    a = _adapter()
    # Compress poll cadence so the test finishes fast.
    a.poll_interval = 0.0
    emitted = []
    def emit(ev):
        emitted.append(ev)
        if len(fake.calls) >= 2:
            # We've consumed the script; ask the worker to stop.
            a._shutdown.set()
    # The worker exits once the shutdown event is set after the
    # second poll. To make sure it actually exits even when the
    # second envelope was deduped, also flip shutdown after a small
    # sentinel call counter.
    import threading
    timer = threading.Timer(2.0, a._shutdown.set)
    timer.start()
    try:
        a._producer_blocking(emit)
    finally:
        timer.cancel()
    assert len(emitted) == 1


# ---- schema (--describe) -------------------------------------------


def test_schema_round_trip():
    schema = sg.SignalAdapter.SCHEMA.to_dict()
    assert schema["name"] == "signal"
    keys = {f["key"] for f in schema["fields"]}
    expected = {
        "SIGNAL_API_URL",
        "SIGNAL_NUMBER",
        "SIGNAL_API_KEY",
        "SIGNAL_ALLOWED_USERS",
        "SIGNAL_POLL_INTERVAL_SECS",
        "SIGNAL_ALLOW_LOCAL",
        "SIGNAL_ACCOUNT_ID",
    }
    assert expected.issubset(keys), f"missing: {expected - keys}"
    secret_fields = {
        f["key"] for f in schema["fields"] if f["type"] == "secret"
    }
    assert secret_fields == {"SIGNAL_API_KEY"}


def test_capabilities_empty():
    """signal-cli-rest-api has no native typing / reaction endpoints
    we can wire up — keep capabilities empty rather than over-claim.
    Mirrors the line / zulip pattern."""
    assert sg.SignalAdapter.capabilities == []
