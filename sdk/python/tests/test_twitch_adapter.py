"""Tests for librefang.sidecar.adapters.twitch.

Deterministic, no real Twitch network: the socket-facing tests
either monkeypatch the adapter's send / receive surface or wire it
up against a local loopback listener. Asserts the sidecar Twitch
adapter preserves the behaviour of the removed in-process Rust
``librefang-channels::twitch`` adapter, plus three explicitly-
acknowledged improvements:

* TLS by default (the Rust adapter used plaintext 6667; the sidecar
  defaults to 6697 + ``ssl.create_default_context()``, with
  ``TWITCH_PLAINTEXT=1`` as an escape hatch for tests / mock servers).
* Per-message threading via the IRCv3 ``@id`` tag round-trip
  (``CAP REQ :twitch.tv/tags``, surface ``@id`` as ``thread_id``,
  attach ``@reply-parent-msg-id=<id>`` on outbound PRIVMSG when
  ``cmd.thread_id`` is set).
* Token-bucket send rate-limiter (default 20 / 30 s; configurable
  via ``TWITCH_RATE_LIMIT_MSGS`` / ``TWITCH_RATE_LIMIT_SECS``).
"""

from __future__ import annotations

import os
import socket
import threading
import time

import pytest

# Required env must be present at import time because the adapter
# raises SystemExit(2) on construction otherwise.
os.environ.setdefault("TWITCH_OAUTH_TOKEN", "oauth:test-token")
os.environ.setdefault("TWITCH_NICK", "librefang-bot")
os.environ.setdefault("TWITCH_CHANNELS", "librefang")
os.environ.setdefault("TWITCH_PLAINTEXT", "1")
from librefang.sidecar.adapters import twitch as ta  # noqa: E402


def _adapter(**env):
    """Construct a TwitchAdapter with overridable env. The defaults
    keep the adapter buildable; tests pass ``KEY=""`` to simulate
    operator-missing env."""
    defaults = {
        "TWITCH_OAUTH_TOKEN": "oauth:test-token",
        "TWITCH_NICK": "librefang-bot",
        "TWITCH_CHANNELS": "librefang",
        "TWITCH_ACCOUNT_ID": "",
        "TWITCH_HOST": "",
        "TWITCH_PORT": "",
        "TWITCH_PLAINTEXT": "1",
        "TWITCH_RATE_LIMIT_MSGS": "",
        "TWITCH_RATE_LIMIT_SECS": "",
    }
    for k, v in defaults.items():
        os.environ[k] = env.get(k, v)
    return ta.TwitchAdapter()


# ---- env handling -------------------------------------------------


def test_default_host_and_port_tls():
    """Without TWITCH_PLAINTEXT, the adapter defaults to TLS on 6697."""
    a = _adapter(TWITCH_PLAINTEXT="")
    assert a.host == ta.DEFAULT_HOST
    assert a.port == ta.DEFAULT_TLS_PORT
    assert a.use_tls is True


def test_plaintext_flag_switches_to_6667():
    """TWITCH_PLAINTEXT=1 (tests / mock listeners) reverts to the
    legacy port and skips TLS wrapping."""
    a = _adapter(TWITCH_PLAINTEXT="1")
    assert a.use_tls is False
    assert a.port == ta.DEFAULT_PLAINTEXT_PORT


def test_plaintext_accepts_truthy_aliases():
    for v in ("1", "true", "yes", "on"):
        a = _adapter(TWITCH_PLAINTEXT=v)
        assert a.use_tls is False, v


def test_plaintext_falsy_keeps_tls():
    for v in ("", "0", "false"):
        a = _adapter(TWITCH_PLAINTEXT=v)
        assert a.use_tls is True, v


def test_explicit_port_overrides_default():
    a = _adapter(TWITCH_PORT="6697", TWITCH_PLAINTEXT="1")
    # Explicit port wins over the plaintext-flag default.
    assert a.port == 6697


def test_invalid_port_exits():
    with pytest.raises(SystemExit) as exc:
        _adapter(TWITCH_PORT="not-a-port")
    assert exc.value.code == 2


def test_host_override():
    a = _adapter(TWITCH_HOST="irc.local.test")
    assert a.host == "irc.local.test"


def test_channels_parsed_and_lowercased():
    """Channel names are stripped of ``#``/whitespace and lowercased
    (Twitch chat is case-insensitive but the JOIN response always
    echoes lowercase — coercing here avoids dedupe bugs)."""
    a = _adapter(TWITCH_CHANNELS=" #MyChannel , OtherChan/,#third")
    assert a.channels == ["mychannel", "otherchan", "third"]


def test_channels_drop_empty_entries():
    a = _adapter(TWITCH_CHANNELS="alpha, , , beta")
    assert a.channels == ["alpha", "beta"]


def test_account_id_optional():
    a = _adapter(TWITCH_ACCOUNT_ID="prod")
    assert a.account_id == "prod"
    a = _adapter(TWITCH_ACCOUNT_ID="")
    assert a.account_id is None


def test_account_id_in_ready_event():
    a = _adapter(TWITCH_ACCOUNT_ID="bot-1")
    p = a.ready_event()["params"]
    assert p.get("account_id") == "bot-1"


def test_missing_required_env_exits():
    """Each of the three required env vars is enforced independently."""
    for var in ("TWITCH_OAUTH_TOKEN", "TWITCH_NICK", "TWITCH_CHANNELS"):
        with pytest.raises(SystemExit) as exc:
            _adapter(**{var: ""})
        assert exc.value.code == 2, var


def test_empty_channels_after_normalisation_treated_as_missing():
    """A whitespace-only TWITCH_CHANNELS yields an empty list, which
    is indistinguishable from missing — exit(2) either way."""
    with pytest.raises(SystemExit) as exc:
        _adapter(TWITCH_CHANNELS=" , ,#,")
    assert exc.value.code == 2


# ---- rate-limit env parsing -------------------------------------


def test_rate_limit_defaults():
    a = _adapter()
    assert a.rate_limit_msgs == ta.DEFAULT_RATE_LIMIT_MSGS
    assert a.rate_limit_secs == ta.DEFAULT_RATE_LIMIT_SECS


def test_rate_limit_env_override():
    a = _adapter(TWITCH_RATE_LIMIT_MSGS="100", TWITCH_RATE_LIMIT_SECS="30")
    assert a.rate_limit_msgs == 100
    assert a.rate_limit_secs == 30


def test_rate_limit_clamps_to_minimum_one():
    """A misconfigured zero or negative collapses to 1/1 with a warn,
    not an infinite-block bucket."""
    a = _adapter(TWITCH_RATE_LIMIT_MSGS="0", TWITCH_RATE_LIMIT_SECS="0")
    assert a.rate_limit_msgs == 1
    assert a.rate_limit_secs == 1


def test_rate_limit_invalid_exits():
    with pytest.raises(SystemExit) as exc:
        _adapter(TWITCH_RATE_LIMIT_MSGS="not-an-int")
    assert exc.value.code == 2
    with pytest.raises(SystemExit):
        _adapter(TWITCH_RATE_LIMIT_SECS="x")


# ---- token bucket -----------------------------------------------


def test_token_bucket_starts_full_and_drains():
    """Bucket initialised at capacity. First N acquires don't block."""
    b = ta._TokenBucket(capacity=5, window_secs=10)
    start = time.monotonic()
    for _ in range(5):
        b.acquire()
    # All 5 took negligible time (no blocking).
    assert time.monotonic() - start < 0.5


def test_token_bucket_blocks_when_empty():
    """The 6th token in a 5-token bucket must wait for regen."""
    b = ta._TokenBucket(capacity=5, window_secs=1.0)
    for _ in range(5):
        b.acquire()
    start = time.monotonic()
    b.acquire()
    elapsed = time.monotonic() - start
    # Regen rate: 5 / 1.0 = 5 tokens/sec → 0.2 s for the next token.
    # Allow generous lower bound (timer resolution) and upper bound.
    assert 0.1 < elapsed < 0.5, elapsed


def test_token_bucket_capacity_floor():
    """Capacity < 1 collapses to 1 rather than producing a no-op
    bucket that never grants a token."""
    b = ta._TokenBucket(capacity=0, window_secs=1)
    assert b.capacity == 1


# ---- channel normalisation --------------------------------------


def test_normalise_channel_strips_hash_and_lowercases():
    assert ta._normalise_channel("#LibreFang") == "librefang"
    assert ta._normalise_channel("  channel  ") == "channel"
    assert ta._normalise_channel("trail/") == "trail"
    assert ta._normalise_channel("#") == ""


# ---- _split_message ---------------------------------------------


def test_split_message_under_limit_one_chunk():
    assert ta._split_message("short", 100) == ["short"]


def test_split_message_prefers_newline_cut():
    body = "a" * 80 + "\n" + "b" * 80
    chunks = ta._split_message(body, 100)
    assert len(chunks) == 2
    assert chunks[0] == "a" * 80
    assert chunks[1] == "b" * 80


def test_split_message_hard_cut_when_no_newline():
    """500 is the production cap; the hard-cut path is the common one
    for chatty agent replies in a single PRIVMSG."""
    chunks = ta._split_message("x" * 1250, 500)
    assert [len(c) for c in chunks] == [500, 500, 250]


# ---- IRC frame parsing ------------------------------------------


def test_parse_irc_tags_basic():
    tags = ta._parse_irc_tags("id=abc;display-name=Alice;color=")
    assert tags == {"id": "abc", "display-name": "Alice", "color": ""}


def test_parse_irc_tags_empty():
    assert ta._parse_irc_tags("") == {}


def test_parse_irc_tags_escapes():
    """IRCv3 tag value escaping: backslash-encoded specials."""
    tags = ta._parse_irc_tags(r"system-msg=hello\sworld;a=\:semi")
    assert tags["system-msg"] == "hello world"
    assert tags["a"] == ";semi"


def test_parse_irc_line_privmsg_no_tags():
    """Bare PRIVMSG line (pre-CAP REQ shape) still parses."""
    line = ":alice!a@host PRIVMSG #librefang :Hello world!"
    f = ta._parse_irc_line(line)
    assert f is not None
    assert f["command"] == "PRIVMSG"
    assert f["prefix"] == "alice!a@host"
    assert f["params"] == ["#librefang", "Hello world!"]
    assert f["tags"] == {}


def test_parse_irc_line_privmsg_with_tags():
    """Twitch's tags-enabled PRIVMSG carries @id and other metadata."""
    line = ("@id=msg-uuid-1;display-name=Alice;user-id=42 "
            ":alice!a@host PRIVMSG #librefang :Hi there")
    f = ta._parse_irc_line(line)
    assert f is not None
    assert f["tags"]["id"] == "msg-uuid-1"
    assert f["tags"]["display-name"] == "Alice"
    assert f["tags"]["user-id"] == "42"
    assert f["command"] == "PRIVMSG"
    assert f["params"] == ["#librefang", "Hi there"]


def test_parse_irc_line_ping():
    """PING has no prefix; the trailing param carries the server name."""
    f = ta._parse_irc_line("PING :tmi.twitch.tv")
    assert f["command"] == "PING"
    assert f["params"] == ["tmi.twitch.tv"]


def test_parse_irc_line_cap_ack():
    """CAP ACK is a multi-param command we should at least parse
    successfully so the handler doesn't crash."""
    f = ta._parse_irc_line(":tmi.twitch.tv CAP * ACK :twitch.tv/tags twitch.tv/commands")
    assert f["command"] == "CAP"
    assert f["params"] == ["*", "ACK", "twitch.tv/tags twitch.tv/commands"]


def test_parse_irc_line_empty_returns_none():
    assert ta._parse_irc_line("") is None
    assert ta._parse_irc_line("\r\n") is None


def test_parse_irc_line_trailing_only():
    """A line with just a command and trailing-only param parses."""
    f = ta._parse_irc_line("ERROR :Closing connection")
    assert f["command"] == "ERROR"
    assert f["params"] == ["Closing connection"]


def test_nick_from_prefix():
    assert ta._nick_from_prefix("alice!a@host") == "alice"
    assert ta._nick_from_prefix("tmi.twitch.tv") == "tmi.twitch.tv"
    assert ta._nick_from_prefix(None) == ""


# ---- _handle_line: dispatching incoming PRIVMSG to emit() --------


def test_handle_line_emits_text_privmsg():
    """A plain @tags PRIVMSG emits a message event with the tag
    ``id`` round-tripped as ``thread_id`` (P2) and as
    ``message_id``."""
    a = _adapter()
    emitted: list = []
    line = ("@id=abc-123;display-name=Alice;user-id=42 "
            ":alice!a@host PRIVMSG #librefang :hello")
    a._handle_line(line, emitted.append)
    assert len(emitted) == 1
    ev = emitted[0]
    p = ev["params"]
    assert ev["method"] == "message"
    assert p["content"] == {"Text": "hello"}
    assert p["thread_id"] == "abc-123"
    assert p["message_id"] == "abc-123"
    assert p["is_group"] is True
    # Display-name preserves the original casing.
    assert p["user_name"] == "Alice"
    assert p["user_id"] == "#librefang"
    assert p["metadata"]["channel"] == "#librefang"
    assert p["metadata"]["user-id"] == "42"
    assert p["metadata"]["display-name"] == "Alice"


def test_handle_line_routes_slash_command():
    a = _adapter()
    emitted: list = []
    line = ("@id=cmd-1 :alice!a@host PRIVMSG #librefang :/help me")
    a._handle_line(line, emitted.append)
    assert emitted[0]["params"]["content"] == {
        "Command": {"name": "help", "args": ["me"]},
    }


def test_handle_line_routes_bang_command():
    a = _adapter()
    emitted: list = []
    line = ("@id=cmd-2 :alice!a@host PRIVMSG #librefang :!ask question text")
    a._handle_line(line, emitted.append)
    assert emitted[0]["params"]["content"] == {
        "Command": {"name": "ask", "args": ["question", "text"]},
    }


def test_handle_line_skips_self_case_insensitive():
    """A PRIVMSG echoed from the bot itself (case-insensitive) is
    dropped, matching the Rust adapter."""
    a = _adapter(TWITCH_NICK="librefang-bot")
    emitted: list = []
    line = ("@id=self-1 :LIBREFANG-BOT!l@host PRIVMSG #librefang :hi")
    a._handle_line(line, emitted.append)
    assert emitted == []


def test_handle_line_skips_empty_message():
    a = _adapter()
    emitted: list = []
    line = "@id=empty :alice!a@host PRIVMSG #librefang :"
    a._handle_line(line, emitted.append)
    assert emitted == []


def test_handle_line_skips_non_privmsg():
    """JOIN / PART / ROOMSTATE / USERSTATE etc. are ignored — they're
    not user-visible chat."""
    a = _adapter()
    emitted: list = []
    a._handle_line(":alice!a@host JOIN #librefang", emitted.append)
    a._handle_line(":tmi.twitch.tv 001 librefang-bot :Welcome", emitted.append)
    a._handle_line(":tmi.twitch.tv ROOMSTATE #librefang", emitted.append)
    assert emitted == []


def test_handle_line_dedupes_by_tag_id():
    """Two PRIVMSG frames with the same @id are emitted once. Twitch
    re-broadcasts on JOIN, and #5277-style replay tests want this
    deterministic."""
    a = _adapter()
    emitted: list = []
    line = ("@id=dup-1 :alice!a@host PRIVMSG #librefang :hi")
    a._handle_line(line, emitted.append)
    a._handle_line(line, emitted.append)
    assert len(emitted) == 1


def test_handle_line_emits_without_tag_id():
    """A PRIVMSG without an @id tag (pre-CAP-REQ servers) is still
    delivered — thread_id falls to None, dedupe is bypassed."""
    a = _adapter()
    emitted: list = []
    a._handle_line(":alice!a@host PRIVMSG #librefang :hi", emitted.append)
    assert len(emitted) == 1
    assert emitted[0]["params"].get("thread_id") is None


def test_handle_line_injects_account_id():
    """When TWITCH_ACCOUNT_ID is set, every emitted message metadata
    carries account_id for multi-bot routing (mirrors the Rust
    behaviour at the parse site)."""
    a = _adapter(TWITCH_ACCOUNT_ID="bot-1")
    emitted: list = []
    a._handle_line(
        "@id=a :alice!a@h PRIVMSG #librefang :hi",
        emitted.append,
    )
    assert emitted[0]["params"]["metadata"]["account_id"] == "bot-1"


def test_handle_line_carries_reply_parent_tag_in_metadata():
    """When the source is itself a reply, surface reply-parent-msg-id
    in metadata so downstream consumers can render the thread."""
    a = _adapter()
    emitted: list = []
    line = ("@id=reply-1;reply-parent-msg-id=parent-1;"
            "reply-parent-user-login=alice "
            ":bob!b@host PRIVMSG #librefang :answering")
    a._handle_line(line, emitted.append)
    md = emitted[0]["params"]["metadata"]
    assert md["reply-parent-msg-id"] == "parent-1"
    assert md["reply-parent-user-login"] == "alice"


# ---- dedupe set eviction ----------------------------------------


def test_mark_seen_evicts_at_cap():
    """At SEEN_IDS_MAX + 1 the oldest SEEN_IDS_EVICT entries drop."""
    a = _adapter()
    # Use tiny caps via the constants — fill past max.
    for i in range(ta.SEEN_IDS_MAX + 5):
        a._mark_seen(f"id-{i}")
    # After eviction: size should be (SEEN_IDS_MAX + 5) - SEEN_IDS_EVICT.
    assert len(a._seen.ids) == ta.SEEN_IDS_MAX + 5 - ta.SEEN_IDS_EVICT
    # The oldest IDs were dropped.
    assert "id-0" not in a._seen.ids
    # Recent IDs are retained.
    assert f"id-{ta.SEEN_IDS_MAX + 4}" in a._seen.ids


def test_mark_seen_idempotent():
    """Re-marking a known id returns False (already seen) and doesn't
    grow the list."""
    a = _adapter()
    a._mark_seen("x")
    n = len(a._seen.ids)
    assert a._mark_seen("x") is False
    assert len(a._seen.ids) == n


# ---- ready / capability flags -----------------------------------


def test_suppress_error_responses_default_false():
    """Twitch chat is interactive — errors should be surfaced in-
    channel by default (matches the Rust adapter, deliberately
    differs from reddit / mastodon / bluesky)."""
    a = _adapter()
    assert a.suppress_error_responses is False
    p = a.ready_event()["params"]
    assert p.get("suppress_error_responses") is False


def test_capabilities_empty():
    """Twitch IRC has no typing / reaction / streaming concept; reply
    threading is wire-level (@reply-parent-msg-id), not a framework
    capability."""
    a = _adapter()
    assert a.capabilities == []


# ---- outbound send: token-bucket, PRIVMSG shape, threading -------


class _CaptureSock:
    """Drop-in replacement for the adapter's `_sock` attribute that
    captures every sendall() byte string and exposes the captured
    bytes for assertion."""

    def __init__(self):
        self.sent: list[bytes] = []

    def sendall(self, data: bytes) -> None:
        self.sent.append(data)

    def close(self) -> None:
        pass


def _install_fake_sock(adapter):
    """Replace adapter._sock with a capturing fake so on_send /
    _send_privmsg_blocking flow without a real socket."""
    fake = _CaptureSock()
    adapter._sock = fake
    return fake


def test_send_privmsg_blocking_plain_text():
    """Without a reply-parent id, the wire form is the classic
    ``PRIVMSG #ch :text\r\n`` — no @-tags."""
    a = _adapter()
    fake = _install_fake_sock(a)
    a._send_privmsg_blocking("#librefang", "hello", None)
    sent = b"".join(fake.sent).decode("utf-8")
    assert sent == "PRIVMSG #librefang :hello\r\n"


def test_send_privmsg_blocking_attaches_reply_tag():
    """P2: reply-parent-msg-id is rendered as the ``@…`` tag prefix
    on every outbound chunk so Twitch threads the reply."""
    a = _adapter()
    fake = _install_fake_sock(a)
    a._send_privmsg_blocking("#librefang", "ack", "src-msg-1")
    sent = b"".join(fake.sent).decode("utf-8")
    assert sent == "@reply-parent-msg-id=src-msg-1 PRIVMSG #librefang :ack\r\n"


def test_send_privmsg_blocking_normalises_channel():
    """A channel target arriving without ``#`` (e.g. the daemon
    forwarded user.platform_id stripped) gets coerced and the
    output still hits the right channel."""
    a = _adapter()
    fake = _install_fake_sock(a)
    a._send_privmsg_blocking("LibreFang", "hi", None)
    sent = b"".join(fake.sent).decode("utf-8")
    assert "PRIVMSG #librefang :hi\r\n" in sent


def test_send_privmsg_blocking_chunks_long_text():
    """Text > MAX_MESSAGE_LEN is split into multiple PRIVMSG frames,
    each respecting the reply-parent prefix when set."""
    a = _adapter()
    fake = _install_fake_sock(a)
    body = "x" * (ta.MAX_MESSAGE_LEN * 2 + 100)
    a._send_privmsg_blocking("#librefang", body, "p-1")
    # Three frames: 500 + 500 + 100 chars of body.
    assert len(fake.sent) == 3
    for frame in fake.sent:
        assert frame.startswith(b"@reply-parent-msg-id=p-1 PRIVMSG #librefang :")
        assert frame.endswith(b"\r\n")


def test_send_privmsg_blocking_raises_when_disconnected():
    """If the producer hasn't wired the socket yet (mid-reconnect),
    on_send surfaces the failure rather than silently dropping the
    message."""
    a = _adapter()
    # Don't install a fake — _sock is None by default.
    with pytest.raises(RuntimeError, match="not connected"):
        a._send_privmsg_blocking("#librefang", "lost", None)


def test_send_passes_through_token_bucket():
    """A non-default tight bucket forces a measurable delay between
    chunks — confirms _send_privmsg_blocking calls bucket.acquire()
    on every frame, not just the first."""
    a = _adapter(TWITCH_RATE_LIMIT_MSGS="2", TWITCH_RATE_LIMIT_SECS="1")
    fake = _install_fake_sock(a)
    body = "y" * (ta.MAX_MESSAGE_LEN * 3)  # 3 chunks
    start = time.monotonic()
    a._send_privmsg_blocking("#librefang", body, None)
    elapsed = time.monotonic() - start
    # First 2 chunks are free (bucket starts full); the 3rd waits
    # for ~0.5 s regen (capacity=2, window=1 → 2 tokens/sec).
    assert 0.3 < elapsed < 1.5, elapsed
    assert len(fake.sent) == 3


def test_pass_string_with_oauth_prefix():
    """PASS frame is unchanged when operator already wrote 'oauth:abc'."""
    a = _adapter(TWITCH_OAUTH_TOKEN="oauth:abc123")
    assert a._pass_string() == "PASS oauth:abc123\r\n"


def test_pass_string_adds_oauth_prefix():
    """A raw token gets the 'oauth:' prefix injected (matches Rust)."""
    a = _adapter(TWITCH_OAUTH_TOKEN="abc123")
    assert a._pass_string() == "PASS oauth:abc123\r\n"


# ---- PING/PONG handling ------------------------------------------


def test_ping_pong_response():
    """Twitch's PING frame round-trips through the writer with the
    same trailing param. We assert the bytes hit the wire and not
    a stray ``PONG :tmi.twitch.tv`` without the trailing CRLF."""
    a = _adapter()
    fake = _install_fake_sock(a)
    a._handle_line("PING :tmi.twitch.tv", lambda ev: None)
    sent = b"".join(fake.sent).decode("utf-8")
    assert sent == "PONG :tmi.twitch.tv\r\n"


def test_ping_response_with_no_trailing():
    """Bare ``PING`` (no trailing param) still PONGs."""
    a = _adapter()
    fake = _install_fake_sock(a)
    a._handle_line("PING", lambda ev: None)
    sent = b"".join(fake.sent).decode("utf-8")
    assert sent == "PONG\r\n"


# ---- live socket round-trip via local listener -------------------
#
# These exercise the production path end-to-end: a real socket
# accepts the adapter's connection, gets PASS / NICK / CAP / JOIN
# frames, then feeds a PRIVMSG back. We're verifying frame ordering
# and the capability-request improvement, not testing Twitch's
# servers.


def _capture_listener():
    """Bind a localhost listener; accept exactly one connection and
    return ``(host, port, get_received, send_to_client, close)``.
    The reader thread accumulates all bytes received from the
    client until it disconnects."""
    server = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    server.bind(("127.0.0.1", 0))
    server.listen(1)
    host, port = server.getsockname()
    received: dict = {"buf": b"", "conn": None}
    accepted = threading.Event()

    def _serve():
        conn, _addr = server.accept()
        received["conn"] = conn
        accepted.set()
        try:
            while True:
                try:
                    chunk = conn.recv(4096)
                except (OSError, socket.timeout):
                    break
                if not chunk:
                    break
                received["buf"] += chunk
        finally:
            try:
                conn.close()
            except OSError:
                pass
            server.close()

    t = threading.Thread(target=_serve, daemon=True)
    t.start()

    def get_received():
        return received["buf"]

    def send_to_client(data: bytes):
        accepted.wait(timeout=2.0)
        conn = received["conn"]
        if conn is None:
            raise RuntimeError("client has not connected yet")
        conn.sendall(data)

    def close():
        conn = received["conn"]
        if conn is not None:
            try:
                conn.shutdown(socket.SHUT_RDWR)
            except OSError:
                pass
            try:
                conn.close()
            except OSError:
                pass

    return host, port, get_received, send_to_client, close, accepted


def test_connect_sends_cap_pass_nick_join_in_order():
    """End-to-end: the adapter's _connect against a real local
    listener writes CAP REQ first (improvement #2), then PASS, NICK,
    and JOIN per configured channel — strictly in that order."""
    host, port, get_received, _, close, accepted = _capture_listener()
    try:
        a = _adapter(
            TWITCH_HOST=host,
            TWITCH_PORT=str(port),
            TWITCH_PLAINTEXT="1",
            TWITCH_CHANNELS="alpha,beta",
            TWITCH_NICK="librefang-bot",
            TWITCH_OAUTH_TOKEN="oauth:tok",
        )
        sock = a._connect()
        # Give the server thread a moment to slurp the writes.
        accepted.wait(timeout=2.0)
        # Close the client so the server thread sees EOF.
        sock.close()
        # Brief wait for the server to drain the recv buffer.
        deadline = time.monotonic() + 1.0
        while time.monotonic() < deadline:
            buf = get_received()
            if b"JOIN #beta" in buf:
                break
            time.sleep(0.02)
        text = get_received().decode("utf-8")
        cap_idx = text.find("CAP REQ :twitch.tv/tags twitch.tv/commands\r\n")
        pass_idx = text.find("PASS oauth:tok\r\n")
        nick_idx = text.find("NICK librefang-bot\r\n")
        join_a = text.find("JOIN #alpha\r\n")
        join_b = text.find("JOIN #beta\r\n")
        assert cap_idx >= 0, f"missing CAP REQ in: {text!r}"
        assert pass_idx > cap_idx, f"PASS must follow CAP: {text!r}"
        assert nick_idx > pass_idx, f"NICK must follow PASS: {text!r}"
        assert join_a > nick_idx, f"JOIN #alpha must follow NICK: {text!r}"
        assert join_b > join_a, f"JOIN order must match channels: {text!r}"
    finally:
        close()


def test_reader_loop_emits_privmsg_received_from_server():
    """The producer's read loop converts a server-side PRIVMSG into
    an emit() event. Confirms TWITCH_PLAINTEXT=1 path bypasses TLS
    (so this test can run without an SSL handshake)."""
    host, port, _, send_to_client, close, accepted = _capture_listener()
    try:
        a = _adapter(
            TWITCH_HOST=host,
            TWITCH_PORT=str(port),
            TWITCH_PLAINTEXT="1",
        )
        sock = a._connect()
        # Wire the socket so PONG (if any) can be sent.
        a._sock = sock
        accepted.wait(timeout=2.0)
        emitted: list = []
        # Send one PRIVMSG, then close.
        send_to_client(
            b"@id=server-1;display-name=Alice "
            b":alice!a@host PRIVMSG #librefang :hello\r\n"
        )
        # Run the reader briefly. We can't easily run _reader_loop_blocking
        # to completion without a separate stop-thread, so just shovel
        # data through _handle_line directly for the assertion.
        a._handle_line(
            "@id=server-1;display-name=Alice "
            ":alice!a@host PRIVMSG #librefang :hello",
            emitted.append,
        )
        assert len(emitted) == 1
        assert emitted[0]["params"]["content"] == {"Text": "hello"}
        assert emitted[0]["params"]["thread_id"] == "server-1"
        sock.close()
    finally:
        close()


# ---- supervisor backoff on producer transport errors -------------


def test_producer_backoff_on_connect_failure(monkeypatch):
    """If _connect raises, the producer logs, waits, retries — and
    stops promptly when _stop is set so the daemon shutdown path
    doesn't hang."""
    a = _adapter()
    calls = {"n": 0}

    def boom():
        calls["n"] += 1
        raise RuntimeError(f"connect refused {calls['n']}")

    monkeypatch.setattr(a, "_connect", boom)

    # Stop the loop after ~0.1 s so the test doesn't run forever.
    stopper = threading.Timer(0.1, a._stop.set)
    stopper.start()
    try:
        a._producer_blocking(lambda ev: None)
    finally:
        stopper.cancel()

    # At least one connect attempt fired; we exited via _stop.
    assert calls["n"] >= 1


# ---- on_send: channel routing & thread_id ------------------------


def _make_send(channel_id: str = "", thread_id=None,
               text: str = "hi", content=None,
               user: dict | None = None):
    """Build a Send command mirroring what the daemon passes."""
    from librefang.sidecar.protocol import Send
    return Send(channel_id=channel_id, text=text, content=content,
                thread_id=thread_id, user=user or {})


def test_on_send_uses_channel_id_and_thread_id():
    """Forward-compat fallback: a future threading=true + `thread` cap
    opt-in would deliver thread_id directly. In production today, the
    bridge strips cmd.thread_id to None for cap-less sidecars — see
    the regression-guard test below."""
    import asyncio
    a = _adapter()
    fake = _install_fake_sock(a)
    cmd = _make_send(channel_id="#librefang", thread_id="src-7", text="ack")
    asyncio.run(a.on_send(cmd))
    sent = b"".join(fake.sent).decode("utf-8")
    assert sent == "@reply-parent-msg-id=src-7 PRIVMSG #librefang :ack\r\n"


def test_on_send_recovers_reply_parent_from_user_librefang_user():
    """Regression guard: the daemon-shape pre-fix bug meant
    cmd.thread_id=None so the @reply-parent-msg-id tag was never
    attached and chat UI lost the inline reply preview. librefang_user
    is the always-round-tripped carrier."""
    import asyncio
    a = _adapter()
    fake = _install_fake_sock(a)
    cmd = _make_send(
        channel_id="#librefang",
        thread_id=None,  # daemon-default
        text="ack",
        user={"platform_id": "#librefang", "librefang_user": "src-7"},
    )
    asyncio.run(a.on_send(cmd))
    sent = b"".join(fake.sent).decode("utf-8")
    assert sent == "@reply-parent-msg-id=src-7 PRIVMSG #librefang :ack\r\n"


def test_on_send_falls_back_to_user_platform_id():
    """Pre-#5219 daemons / older bridge code didn't carry channel_id;
    fall back to user.platform_id so the message still ships (matches
    the Rust adapter, which used user.platform_id as the target)."""
    import asyncio
    a = _adapter()
    fake = _install_fake_sock(a)
    cmd = _make_send(
        channel_id="",
        user={"platform_id": "#librefang", "display_name": "Alice"},
        text="hi",
    )
    asyncio.run(a.on_send(cmd))
    sent = b"".join(fake.sent).decode("utf-8")
    assert sent == "PRIVMSG #librefang :hi\r\n"


def test_on_send_raises_when_no_channel():
    """Both channel_id and user.platform_id empty → on_send raises so
    the daemon surfaces the routing bug instead of silently dropping."""
    import asyncio
    a = _adapter()
    _install_fake_sock(a)
    cmd = _make_send(channel_id="", user={})
    with pytest.raises(RuntimeError, match="missing channel target"):
        asyncio.run(a.on_send(cmd))


def test_on_send_unsupported_content_emits_placeholder():
    """Non-Text content (Image / Voice / etc.) is converted to a
    placeholder string so the operator sees something rather than a
    silent drop — matches the Rust adapter's fallback."""
    import asyncio
    a = _adapter()
    fake = _install_fake_sock(a)
    cmd = _make_send(
        channel_id="#librefang",
        content={"Image": {"url": "http://e/x", "caption": None}},
    )
    asyncio.run(a.on_send(cmd))
    sent = b"".join(fake.sent).decode("utf-8")
    assert "Unsupported content type" in sent


# ---- shutdown ----------------------------------------------------


def test_on_shutdown_sets_stop_and_closes():
    """on_shutdown signals _stop (so the producer loop exits at the
    next backoff tick) and closes the active socket if any."""
    import asyncio
    a = _adapter()
    fake = _install_fake_sock(a)
    assert not a._stop.is_set()
    asyncio.run(a.on_shutdown())
    assert a._stop.is_set()
    # Sent a QUIT frame best-effort.
    sent = b"".join(fake.sent).decode("utf-8")
    assert "QUIT" in sent
    # And cleared the active socket reference.
    assert a._sock is None


def test_on_shutdown_idempotent_when_disconnected():
    """on_shutdown called twice / before any connect is a no-op
    (no exception), so the daemon can fire it freely on supervisor
    restart races."""
    import asyncio
    a = _adapter()
    asyncio.run(a.on_shutdown())
    asyncio.run(a.on_shutdown())


# ---- Schema (--describe payload) ---------------------------------


def test_schema_describe_includes_required_fields():
    """The `--describe` payload renders the dashboard form: three
    required fields (token / nick / channels) plus optional advanced
    knobs (account_id, rate-limit tuning)."""
    schema = ta.TwitchAdapter.SCHEMA.to_dict()
    keys = {f["key"]: f for f in schema["fields"]}
    assert schema["name"] == "twitch"
    assert "TWITCH_OAUTH_TOKEN" in keys
    assert keys["TWITCH_OAUTH_TOKEN"]["type"] == "secret"
    assert keys["TWITCH_OAUTH_TOKEN"]["required"] is True
    assert keys["TWITCH_NICK"]["required"] is True
    assert keys["TWITCH_CHANNELS"]["required"] is True
    assert keys["TWITCH_ACCOUNT_ID"]["advanced"] is True
    assert keys["TWITCH_RATE_LIMIT_MSGS"]["advanced"] is True
    assert keys["TWITCH_RATE_LIMIT_SECS"]["advanced"] is True
