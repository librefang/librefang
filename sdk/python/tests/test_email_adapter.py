"""Tests for librefang.sidecar.adapters.email.

Pure-function tests for parsing / allowlist / subject extraction +
end-to-end adapter tests with stubbed IMAP / SMTP clients. No
network — we monkeypatch :mod:`imaplib` and :mod:`smtplib`.
"""
from __future__ import annotations

import email
import os
import ssl
from email.message import EmailMessage

import pytest

os.environ.setdefault("EMAIL_IMAP_HOST", "imap.test")
os.environ.setdefault("EMAIL_SMTP_HOST", "smtp.test")
os.environ.setdefault("EMAIL_USERNAME", "bot@test")
os.environ.setdefault("EMAIL_PASSWORD", "secret")
from librefang.sidecar.adapters import email as em  # noqa: E402


def _adapter(**env):
    defaults = {
        "EMAIL_IMAP_HOST": "imap.test",
        "EMAIL_IMAP_PORT": "",
        "EMAIL_SMTP_HOST": "smtp.test",
        "EMAIL_SMTP_PORT": "",
        "EMAIL_USERNAME": "bot@test",
        "EMAIL_PASSWORD": "secret",
        "EMAIL_IMAP_USERNAME": "",
        "EMAIL_IMAP_PASSWORD": "",
        "EMAIL_SMTP_USERNAME": "",
        "EMAIL_SMTP_PASSWORD": "",
        "EMAIL_POLL_INTERVAL_SECS": "",
        "EMAIL_FOLDERS": "",
        "EMAIL_ALLOWED_SENDERS": "",
        "EMAIL_ACCOUNT_ID": "",
        "EMAIL_TLS_ROOT_CA_PATH": "",
        "EMAIL_TLS_ACCEPT_INVALID_CERTS": "",
        "EMAIL_NET_TIMEOUT_SECS": "",
    }
    for k, v in defaults.items():
        os.environ[k] = env.get(k, v)
    return em.EmailAdapter()


# ---- env handling ----------------------------------------------------


def test_default_env_construction():
    a = _adapter()
    assert a.imap_host == "imap.test"
    assert a.imap_port == 993
    assert a.smtp_host == "smtp.test"
    assert a.smtp_port == 587
    assert a.imap_username == "bot@test"
    assert a.imap_password == "secret"
    assert a.smtp_username == "bot@test"
    assert a.smtp_password == "secret"
    assert a.folders == ["INBOX"]
    assert a.allowed_senders == []
    assert a.account_id is None
    assert a.tls_accept_invalid_certs is False
    assert a.poll_interval_secs == 30


def test_missing_imap_host_exits_2():
    os.environ["EMAIL_IMAP_HOST"] = ""
    with pytest.raises(SystemExit) as e:
        em.EmailAdapter()
    assert e.value.code == 2
    os.environ["EMAIL_IMAP_HOST"] = "imap.test"


def test_imap_specific_username_overrides():
    a = _adapter(EMAIL_IMAP_USERNAME="imap-only@test")
    assert a.imap_username == "imap-only@test"
    assert a.smtp_username == "bot@test"


def test_poll_interval_minimum():
    a = _adapter(EMAIL_POLL_INTERVAL_SECS="1")
    assert a.poll_interval_secs == 5  # floored


def test_folders_csv():
    a = _adapter(EMAIL_FOLDERS="INBOX, Sent ,, Archive")
    assert a.folders == ["INBOX", "Sent", "Archive"]


def test_garbage_int_uses_default(monkeypatch):
    a = _adapter(EMAIL_IMAP_PORT="not-a-port")
    assert a.imap_port == 993


def test_account_id_passthrough():
    a = _adapter(EMAIL_ACCOUNT_ID="prod")
    assert a.account_id == "prod"


def test_tls_accept_invalid_certs_truthy():
    for v in ("1", "true", "yes", "on", "TRUE"):
        a = _adapter(EMAIL_TLS_ACCEPT_INVALID_CERTS=v)
        assert a.tls_accept_invalid_certs is True, v


def test_tls_accept_invalid_certs_falsy():
    for v in ("", "0", "false", "garbage"):
        a = _adapter(EMAIL_TLS_ACCEPT_INVALID_CERTS=v)
        assert a.tls_accept_invalid_certs is False, v


# ---- pure helpers ----------------------------------------------------


def test_extract_email_addr_bracketed():
    assert em.extract_email_addr('"Alice" <alice@example.com>') == "alice@example.com"


def test_extract_email_addr_bare():
    assert em.extract_email_addr("alice@example.com") == "alice@example.com"


def test_extract_email_addr_empty():
    assert em.extract_email_addr("") == ""
    assert em.extract_email_addr(None) == ""


def test_sender_matches_allowlist_exact():
    assert em.sender_matches_allowlist(
        "alice@example.com", ["alice@example.com"],
    )


def test_sender_matches_allowlist_domain():
    assert em.sender_matches_allowlist(
        '"Alice" <alice@example.com>', ["@example.com"],
    )


def test_sender_matches_allowlist_case_insensitive():
    assert em.sender_matches_allowlist(
        "ALICE@Example.COM", ["alice@example.com"],
    )
    assert em.sender_matches_allowlist(
        "alice@example.com", ["@EXAMPLE.COM"],
    )


def test_sender_matches_allowlist_no_substring_3463():
    # Critical regression — `@evil.com` MUST NOT match
    # `victim.@evil.com.attacker.io` (the bug fixed in #3463).
    assert not em.sender_matches_allowlist(
        "victim@evil.com.attacker.io", ["@evil.com"],
    )


def test_sender_matches_allowlist_empty_returns_false():
    # The function returns False when nothing matches; callers
    # treat empty-list as allow-all separately.
    assert not em.sender_matches_allowlist("alice@example.com", [])


def test_sender_matches_allowlist_no_at_sign():
    # No `@` at all → rejected (mirrors email.rs:263-265).
    assert not em.sender_matches_allowlist("not-an-email", ["@example.com"])


def test_sender_matches_allowlist_bare_domain():
    # `@example.com` (no local-part) IS accepted against `@example.com`
    # entry. Rust does the same (email.rs:263-281 doesn't check the
    # local-part); some legitimate bounce / system mail uses bare
    # domain `from` addresses.
    assert em.sender_matches_allowlist("@example.com", ["@example.com"])


def test_extract_agent_from_subject_basic():
    assert em.extract_agent_from_subject("[coder] Fix the bug") == "coder"


def test_extract_agent_from_subject_with_whitespace():
    assert em.extract_agent_from_subject("  [coder]  Fix") == "coder"


def test_extract_agent_from_subject_no_tag():
    assert em.extract_agent_from_subject("No tag here") is None


def test_extract_agent_from_subject_empty_brackets():
    assert em.extract_agent_from_subject("[] Empty") is None


def test_strip_agent_tag():
    assert em.strip_agent_tag("[coder] Fix the bug") == "Fix the bug"


def test_strip_agent_tag_no_tag():
    assert em.strip_agent_tag("No tag") == "No tag"


def test_strip_agent_tag_only_tag():
    assert em.strip_agent_tag("[coder]") == ""


# ---- MIME body extraction --------------------------------------------


def _build_plaintext_mail(body: str, subject="hello", sender="alice@test"):
    msg = EmailMessage()
    msg["From"] = sender
    msg["To"] = "bot@test"
    msg["Subject"] = subject
    msg["Message-ID"] = "<abc123@test>"
    msg.set_content(body)
    return msg.as_bytes()


def _build_multipart_mail(parts: list[tuple[str, str]]):
    msg = EmailMessage()
    msg["From"] = "alice@test"
    msg["To"] = "bot@test"
    msg["Subject"] = "multipart"
    msg["Message-ID"] = "<multi1@test>"
    msg.make_alternative()
    for ctype, body in parts:
        sub = EmailMessage()
        if ctype == "text/plain":
            sub.set_content(body)
        elif ctype == "text/html":
            sub.set_content(body, subtype="html")
        else:
            sub.set_content(body)
        msg.attach(sub)
    return msg.as_bytes()


def test_parse_email_message_text():
    raw = _build_plaintext_mail("Hello world\n", subject="hi")
    parsed = em.parse_email_message(raw)
    assert parsed is not None
    assert parsed["from_addr"] == "alice@test"
    assert parsed["subject"] == "hi"
    assert "Hello world" in parsed["body"]


def test_parse_email_message_message_id():
    raw = _build_plaintext_mail("body")
    parsed = em.parse_email_message(raw)
    assert parsed["message_id"].startswith("<abc123")


def test_parse_email_message_malformed_returns_none():
    assert em.parse_email_message(b"") is None
    assert em.parse_email_message(None) is None


def test_extract_text_body_prefers_text_plain_in_multipart():
    raw = _build_multipart_mail([
        ("text/html", "<p>html</p>"),
        ("text/plain", "PLAIN TEXT"),
    ])
    parsed = em.parse_email_message(raw)
    assert "PLAIN TEXT" in parsed["body"]


def test_extract_text_body_falls_back_to_first_subpart_when_no_plain():
    raw = _build_multipart_mail([
        ("text/html", "<p>only html</p>"),
    ])
    parsed = em.parse_email_message(raw)
    # Falls back to first subpart payload (the HTML).
    assert "only html" in parsed["body"]


def test_extract_text_body_handles_simple_plain():
    raw = _build_plaintext_mail("simple body")
    parsed = em.parse_email_message(raw)
    assert "simple body" in parsed["body"]


# ---- build_outbound_subject ------------------------------------------


def test_build_outbound_subject_explicit_prefix():
    subj, body = em.build_outbound_subject(
        "Subject: My Subject\n\nthe body", reply_subject=None,
    )
    assert subj == "My Subject"
    assert body == "the body"


def test_build_outbound_subject_no_prefix_uses_reply():
    subj, body = em.build_outbound_subject(
        "just text", reply_subject="Original",
    )
    assert subj == "Re: Original"
    assert body == "just text"


def test_build_outbound_subject_no_prefix_no_reply_default():
    subj, body = em.build_outbound_subject(
        "just text", reply_subject=None,
    )
    assert subj == "LibreFang Reply"
    assert body == "just text"


def test_build_outbound_subject_prefix_without_separator():
    subj, body = em.build_outbound_subject(
        "Subject: oops no body", reply_subject=None,
    )
    # No \n\n separator → not a valid Subject: prefix, fall back.
    assert subj == "LibreFang Reply"
    assert body == "Subject: oops no body"


# ---- ReplyCtxCache ---------------------------------------------------


def test_reply_ctx_cache_store_and_get():
    c = em._ReplyCtxCache()
    c.store("alice@test", "Original", "<id1@test>")
    got = c.get("alice@test")
    assert got == ("Original", "<id1@test>")


def test_reply_ctx_cache_case_insensitive():
    c = em._ReplyCtxCache()
    c.store("Alice@Test.com", "S", "<i>")
    assert c.get("alice@test.com") == ("S", "<i>")


def test_reply_ctx_cache_get_missing():
    c = em._ReplyCtxCache()
    assert c.get("nope@test") is None


# ---- IMAP fetch parsers ----------------------------------------------


def test_parse_uid_search_basic():
    assert em._parse_uid_search([b"1 2 3 42"]) == [1, 2, 3, 42]


def test_parse_uid_search_empty():
    assert em._parse_uid_search([b""]) == []
    assert em._parse_uid_search([None]) == []
    assert em._parse_uid_search([]) == []


def test_parse_uid_search_garbage_tokens_dropped():
    assert em._parse_uid_search([b"1 oops 3"]) == [1, 3]


def test_parse_fetch_response_pairs_uid_with_body():
    raw_body = _build_plaintext_mail("the body")
    data = [
        (b"1 (UID 42 RFC822 {1234}", raw_body),
        b")",
        (b"2 (UID 43 RFC822 {1234}", raw_body),
        b")",
    ]
    out = em._parse_fetch_response(data)
    assert set(out.keys()) == {42, 43}
    assert out[42] == raw_body


def test_parse_fetch_response_empty():
    assert em._parse_fetch_response([]) == {}
    assert em._parse_fetch_response(None) == {}


def test_parse_fetch_response_skips_non_tuples():
    data = [b")", b"junk"]
    assert em._parse_fetch_response(data) == {}


# ---- IMAP / SMTP end-to-end via stubs --------------------------------


class _FakeIMAP:
    """Stub IMAP4_SSL — script-driven by setting `_select_replies`,
    `_search_replies`, `_fetch_replies` before passing to the
    adapter."""

    def __init__(
        self, host, port, ssl_context=None, timeout=None,
    ):
        self.host = host
        self.port = port
        self.ssl_context = ssl_context
        self.timeout = timeout
        self.login_calls: list[tuple[str, str]] = []
        self.authenticate_calls: list[str] = []
        self.select_calls: list[str] = []
        self.search_calls: list[tuple] = []
        self.fetch_calls: list[tuple] = []
        self.store_calls: list[tuple] = []
        self.logout_called = False
        # Defaults — tests override these after construction.
        self.login_should_fail = False
        self.select_response = "OK"
        self.search_response: list = [("OK", [b""])]
        self.fetch_response: tuple = ("OK", [])

    def login(self, user, pw):
        self.login_calls.append((user, pw))
        if self.login_should_fail:
            raise __import__("imaplib").IMAP4.error("LOGIN rejected")
        return ("OK", [b"LOGIN done"])

    def authenticate(self, mech, responder):
        self.authenticate_calls.append(mech)
        # responder is called with the (empty) challenge by stdlib;
        # we mimic that here so tests cover the SASL PLAIN payload.
        self._sasl_payload = responder(b"")
        return ("OK", [b"AUTH done"])

    def select(self, mailbox, readonly=False):
        self.select_calls.append(mailbox)
        return (self.select_response, [b""])

    def uid(self, op, *args):
        if op == "SEARCH":
            self.search_calls.append((op, args))
            if not self.search_response:
                return ("NO", [b"no more"])
            return self.search_response.pop(0)
        if op == "FETCH":
            self.fetch_calls.append((op, args))
            return self.fetch_response
        if op == "STORE":
            self.store_calls.append((op, args))
            return ("OK", [b"STORE done"])
        return ("OK", [b""])

    def logout(self):
        self.logout_called = True
        return ("BYE", [b""])


class _FakeSMTP:
    """Stub SMTP transport that records messages instead of sending."""

    sent: list = []

    def __init__(self, host, port, timeout=None, context=None):
        self.host = host
        self.port = port

    def __enter__(self):
        return self

    def __exit__(self, *_exc):
        return False

    def ehlo(self):
        pass

    def has_extn(self, name):
        return True

    def starttls(self, context=None):
        pass

    def login(self, user, pw):
        pass

    def send_message(self, msg):
        _FakeSMTP.sent.append(msg)


@pytest.fixture(autouse=True)
def _reset_smtp():
    _FakeSMTP.sent = []


# Make a fake IMAP4_SSL constructor that returns our pre-baked stub.
def _patch_imaplib(monkeypatch, fake: _FakeIMAP):
    def _ctor(host, port, ssl_context=None, timeout=None):
        fake.host = host
        fake.port = port
        fake.ssl_context = ssl_context
        fake.timeout = timeout
        return fake
    monkeypatch.setattr(em.imaplib, "IMAP4_SSL", _ctor)


# ---- _imap_login path ----


def test_imap_login_login_first(monkeypatch):
    fake = _FakeIMAP("imap.test", 993)
    _patch_imaplib(monkeypatch, fake)
    a = _adapter()
    conn = a._connect_imap()
    assert fake.login_calls == [("bot@test", "secret")]
    assert fake.authenticate_calls == []
    assert conn is fake


def test_imap_login_falls_back_to_plain(monkeypatch):
    fake = _FakeIMAP("imap.test", 993)
    fake.login_should_fail = True
    _patch_imaplib(monkeypatch, fake)
    a = _adapter()
    a._connect_imap()
    assert fake.authenticate_calls == ["PLAIN"]
    # SASL PLAIN payload shape — \\0user\\0pass
    assert fake._sasl_payload == b"\0bot@test\0secret"


# ---- _poll_once integration ----


def test_poll_once_emits_message_and_marks_seen(monkeypatch):
    fake = _FakeIMAP("imap.test", 993)
    raw = _build_plaintext_mail("test body", subject="[coder] hello")
    fake.search_response = [("OK", [b"1"])]
    fake.fetch_response = ("OK", [
        (b"1 (UID 1 RFC822 {1234}", raw),
        b")",
    ])
    _patch_imaplib(monkeypatch, fake)

    a = _adapter()
    emitted: list[dict] = []
    a._poll_once(lambda ev: emitted.append(ev))

    assert len(emitted) == 1
    params = emitted[0]["params"]
    assert params["user_id"] == "alice@test"
    assert "hello" in params["text"]  # subject prefix
    assert "test body" in params["text"]
    meta = params["metadata"]
    assert meta["target_agent"] == "coder"
    assert meta["subject"] == "hello"

    # Marked seen
    assert len(fake.store_calls) >= 1
    flag_call = fake.store_calls[0]
    assert flag_call[1][1] == "+FLAGS"
    assert "\\Seen" in flag_call[1][2]


def test_poll_once_quarantines_disallowed_sender(monkeypatch):
    fake = _FakeIMAP("imap.test", 993)
    raw = _build_plaintext_mail("evil", sender="evil@bad.com")
    fake.search_response = [("OK", [b"1"])]
    fake.fetch_response = ("OK", [
        (b"1 (UID 1 RFC822 {1234}", raw),
        b")",
    ])
    _patch_imaplib(monkeypatch, fake)

    a = _adapter(EMAIL_ALLOWED_SENDERS="@example.com")
    emitted: list[dict] = []
    a._poll_once(lambda ev: emitted.append(ev))

    assert emitted == []
    # Quarantined → +FLAGS includes the Librefang-Quarantine keyword
    flag_call = fake.store_calls[0]
    assert "Librefang-Quarantine" in flag_call[1][2]


def test_poll_once_quarantines_unparseable(monkeypatch):
    fake = _FakeIMAP("imap.test", 993)
    fake.search_response = [("OK", [b"1"])]
    # garbage body → parse fails
    fake.fetch_response = ("OK", [
        (b"1 (UID 1 RFC822 {3}", b""),
        b")",
    ])
    _patch_imaplib(monkeypatch, fake)

    a = _adapter()
    emitted: list[dict] = []
    a._poll_once(lambda ev: emitted.append(ev))
    assert emitted == []
    flag_call = fake.store_calls[0]
    assert "Librefang-Quarantine" in flag_call[1][2]


def test_poll_once_dedupes_message_id(monkeypatch):
    fake = _FakeIMAP("imap.test", 993)
    raw = _build_plaintext_mail("body")
    fake.search_response = [
        ("OK", [b"1"]),
    ]
    fake.fetch_response = ("OK", [
        (b"1 (UID 1 RFC822 {1234}", raw),
        b")",
    ])
    _patch_imaplib(monkeypatch, fake)

    a = _adapter()
    emitted: list[dict] = []
    a._poll_once(lambda ev: emitted.append(ev))
    assert len(emitted) == 1

    # Second poll with same Message-ID — should be deduped
    fake.search_response = [("OK", [b"2"])]
    fake.fetch_response = ("OK", [
        (b"2 (UID 2 RFC822 {1234}", raw),
        b")",
    ])
    a._poll_once(lambda ev: emitted.append(ev))
    assert len(emitted) == 1  # unchanged


def test_poll_once_fallback_search_without_keyword(monkeypatch):
    """Server rejects ``UNKEYWORD Librefang-Quarantine`` → adapter
    falls back to plain ``UNSEEN``."""
    fake = _FakeIMAP("imap.test", 993)
    raw = _build_plaintext_mail("body")
    fake.search_response = [
        ("NO", [b"BAD KEYWORD"]),     # first call rejected
        ("OK", [b"1"]),               # fallback succeeds
    ]
    fake.fetch_response = ("OK", [
        (b"1 (UID 1 RFC822 {1234}", raw),
        b")",
    ])
    _patch_imaplib(monkeypatch, fake)

    a = _adapter()
    emitted: list[dict] = []
    a._poll_once(lambda ev: emitted.append(ev))
    assert len(emitted) == 1


def test_poll_once_injects_account_id(monkeypatch):
    fake = _FakeIMAP("imap.test", 993)
    raw = _build_plaintext_mail("body")
    fake.search_response = [("OK", [b"1"])]
    fake.fetch_response = ("OK", [
        (b"1 (UID 1 RFC822 {1234}", raw),
        b")",
    ])
    _patch_imaplib(monkeypatch, fake)

    a = _adapter(EMAIL_ACCOUNT_ID="tenant-a")
    emitted: list[dict] = []
    a._poll_once(lambda ev: emitted.append(ev))
    assert emitted[0]["params"]["metadata"]["account_id"] == "tenant-a"


def test_poll_once_stores_reply_context(monkeypatch):
    fake = _FakeIMAP("imap.test", 993)
    raw = _build_plaintext_mail("body", subject="Original Subject")
    fake.search_response = [("OK", [b"1"])]
    fake.fetch_response = ("OK", [
        (b"1 (UID 1 RFC822 {1234}", raw),
        b")",
    ])
    _patch_imaplib(monkeypatch, fake)

    a = _adapter()
    a._poll_once(lambda _ev: None)
    ctx = a._reply_ctx.get("alice@test")
    assert ctx is not None
    assert ctx[0] == "Original Subject"


# ---- on_send via SMTP stub -------------------------------------------


@pytest.mark.asyncio
async def test_on_send_basic(monkeypatch):
    monkeypatch.setattr(em.smtplib, "SMTP", _FakeSMTP)
    a = _adapter()
    from librefang.sidecar.protocol import Send
    cmd = Send(
        channel_id="alice@example.com",
        text="hello",
        content={"Text": "hello"},
        thread_id=None,
        user={},
    )
    await a.on_send(cmd)
    assert len(_FakeSMTP.sent) == 1
    msg = _FakeSMTP.sent[0]
    assert msg["To"] == "alice@example.com"
    assert msg["From"] == "bot@test"
    assert msg["Subject"] == "LibreFang Reply"
    assert "hello" in str(msg)


@pytest.mark.asyncio
async def test_on_send_uses_reply_context(monkeypatch):
    monkeypatch.setattr(em.smtplib, "SMTP", _FakeSMTP)
    a = _adapter()
    a._reply_ctx.store("alice@test", "Old Subject", "<msg-prev@test>")
    from librefang.sidecar.protocol import Send
    cmd = Send(
        channel_id="alice@test", text="reply text",
        content={"Text": "reply text"},
        thread_id=None, user={},
    )
    await a.on_send(cmd)
    msg = _FakeSMTP.sent[0]
    assert msg["Subject"] == "Re: Old Subject"
    assert msg["In-Reply-To"] == "<msg-prev@test>"
    assert msg["References"] == "<msg-prev@test>"


@pytest.mark.asyncio
async def test_on_send_explicit_subject_prefix(monkeypatch):
    monkeypatch.setattr(em.smtplib, "SMTP", _FakeSMTP)
    a = _adapter()
    a._reply_ctx.store("alice@test", "Old Subject", "<old@test>")
    from librefang.sidecar.protocol import Send
    cmd = Send(
        channel_id="alice@test",
        text="Subject: Custom\n\nthe body",
        content={"Text": "Subject: Custom\n\nthe body"},
        thread_id=None, user={},
    )
    await a.on_send(cmd)
    msg = _FakeSMTP.sent[0]
    # Explicit subject wins over reply-context.
    assert msg["Subject"] == "Custom"


@pytest.mark.asyncio
async def test_on_send_falls_back_to_user_platform_id(monkeypatch):
    monkeypatch.setattr(em.smtplib, "SMTP", _FakeSMTP)
    a = _adapter()
    from librefang.sidecar.protocol import Send
    cmd = Send(
        channel_id="", text="hello",
        content={"Text": "hello"},
        thread_id=None,
        user={"platform_id": "fallback@test"},
    )
    await a.on_send(cmd)
    msg = _FakeSMTP.sent[0]
    assert msg["To"] == "fallback@test"


@pytest.mark.asyncio
async def test_on_send_invalid_email_drops(monkeypatch):
    monkeypatch.setattr(em.smtplib, "SMTP", _FakeSMTP)
    a = _adapter()
    from librefang.sidecar.protocol import Send
    cmd = Send(
        channel_id="not-an-email", text="x",
        content={"Text": "x"},
        thread_id=None, user={},
    )
    await a.on_send(cmd)
    assert _FakeSMTP.sent == []


@pytest.mark.asyncio
async def test_on_send_unsupported_content_drops(monkeypatch):
    monkeypatch.setattr(em.smtplib, "SMTP", _FakeSMTP)
    a = _adapter()
    from librefang.sidecar.protocol import Send
    cmd = Send(
        channel_id="alice@test", text="",
        content={"Image": {"url": "https://x"}},
        thread_id=None, user={},
    )
    await a.on_send(cmd)
    assert _FakeSMTP.sent == []


@pytest.mark.asyncio
async def test_on_send_refuses_plaintext_when_starttls_missing(monkeypatch):
    """A server that doesn't advertise STARTTLS on a non-465 port
    must NOT receive credentials over the plain socket. Mirror
    lettre's `starttls_relay` semantics: fail loud instead of
    silently downgrading."""

    class _NoStarttlsSMTP(_FakeSMTP):
        def has_extn(self, name):
            # Server doesn't advertise STARTTLS — adapter must refuse.
            return name.lower() != "starttls"

    monkeypatch.setattr(em.smtplib, "SMTP", _NoStarttlsSMTP)
    a = _adapter()
    from librefang.sidecar.protocol import Send
    cmd = Send(
        channel_id="alice@test", text="hi",
        content={"Text": "hi"}, thread_id=None, user={},
    )
    with pytest.raises(Exception) as e:
        await a.on_send(cmd)
    assert "STARTTLS" in str(e.value)
    # Critically: send_message must NOT have been called — that's
    # the whole point. The login call short-circuits before any
    # message bytes hit the wire.
    assert _FakeSMTP.sent == []


@pytest.mark.asyncio
async def test_on_send_smtp_ssl_465(monkeypatch):
    """Port 465 path uses SMTP_SSL instead of SMTP+STARTTLS."""
    called: list[str] = []

    class _FakeSMTPSSL(_FakeSMTP):
        def __init__(self, host, port, context=None, timeout=None):
            called.append("ssl")
            super().__init__(host, port, timeout=timeout, context=context)

    monkeypatch.setattr(em.smtplib, "SMTP", _FakeSMTP)
    monkeypatch.setattr(em.smtplib, "SMTP_SSL", _FakeSMTPSSL)
    a = _adapter(EMAIL_SMTP_PORT="465")
    from librefang.sidecar.protocol import Send
    cmd = Send(
        channel_id="alice@test", text="hi",
        content={"Text": "hi"}, thread_id=None, user={},
    )
    await a.on_send(cmd)
    assert called == ["ssl"]
    assert len(_FakeSMTP.sent) == 1


# ---- schema + capability ---------------------------------------------


def test_schema_shape():
    schema = em.EmailAdapter.SCHEMA.to_dict()
    assert schema["name"] == "email"
    keys = {f["key"] for f in schema["fields"]}
    expected = {
        "EMAIL_IMAP_HOST",
        "EMAIL_IMAP_PORT",
        "EMAIL_SMTP_HOST",
        "EMAIL_SMTP_PORT",
        "EMAIL_USERNAME",
        "EMAIL_PASSWORD",
        "EMAIL_POLL_INTERVAL_SECS",
        "EMAIL_ALLOWED_SENDERS",
        "EMAIL_ACCOUNT_ID",
        "EMAIL_TLS_ROOT_CA_PATH",
        "EMAIL_TLS_ACCEPT_INVALID_CERTS",
    }
    assert expected.issubset(keys), f"missing: {expected - keys}"
    secret_fields = {f["key"] for f in schema["fields"] if f["type"] == "secret"}
    assert "EMAIL_PASSWORD" in secret_fields


def test_capabilities_text_only():
    assert em.EmailAdapter.capabilities == []


# ---- TLS context construction (security-sensitive) -----------------


def test_ssl_context_default_verifies_certs():
    """Default context must enforce hostname + cert validation. The
    presence of these knobs is the only guard between operator
    laziness and a MITM-vulnerable mailbox."""
    a = _adapter()
    ctx = a._build_ssl_context()
    import ssl as _ssl
    assert ctx.check_hostname is True
    assert ctx.verify_mode == _ssl.CERT_REQUIRED


def test_ssl_context_accept_invalid_certs_disables_validation(caplog):
    """`EMAIL_TLS_ACCEPT_INVALID_CERTS=1` is the documented dev escape
    hatch — must produce an unverified context AND log a warning so
    the risk doesn't go unnoticed (#4877)."""
    a = _adapter(EMAIL_TLS_ACCEPT_INVALID_CERTS="1")
    ctx = a._build_ssl_context()
    import ssl as _ssl
    # Unverified context: hostname check off, verify mode CERT_NONE.
    assert ctx.check_hostname is False
    assert ctx.verify_mode == _ssl.CERT_NONE


def test_ssl_context_root_ca_path_keeps_validation_on(tmp_path, monkeypatch):
    """`EMAIL_TLS_ROOT_CA_PATH` adds a custom CA on top of system roots
    — hostname / chain / signature validation MUST stay on (#4877).
    Verifies the factory call shape rather than building a real CA."""
    # Capture what `create_default_context` was called with rather than
    # standing up a self-signed CA fixture.
    captured: dict = {}
    real_factory = ssl.create_default_context

    def _capturing_factory(*args, **kwargs):
        captured["cafile"] = kwargs.get("cafile")
        captured["called"] = True
        # Return a real default context so downstream code stays happy.
        return real_factory()

    monkeypatch.setattr(ssl, "create_default_context", _capturing_factory)
    fake_path = str(tmp_path / "test-ca.pem")
    a = _adapter(EMAIL_TLS_ROOT_CA_PATH=fake_path)
    ctx = a._build_ssl_context()
    assert captured.get("cafile") == fake_path
    # Validation stays ON.
    assert ctx.check_hostname is True
    assert ctx.verify_mode == ssl.CERT_REQUIRED
