"""Tests for librefang.sidecar.adapters.mastodon.

Deterministic, no network: urllib is monkeypatched. Asserts the
sidecar Mastodon adapter preserves the behaviour of the removed
in-process Rust `librefang-channels::mastodon` adapter.
"""

import io
import json
import os

import pytest

# Required env must be present at import time because the adapter
# raises SystemExit(2) if unset on construction. Tests rebuild from
# a clean env via the _adapter() helper per case.
os.environ.setdefault("MASTODON_INSTANCE_URL", "https://mastodon.example.com")
os.environ.setdefault("MASTODON_ACCESS_TOKEN", "tk_test")
from librefang.sidecar.adapters import mastodon as ma  # noqa: E402


def _adapter(**env):
    defaults = {
        "MASTODON_INSTANCE_URL": "https://mastodon.example.com",
        "MASTODON_ACCESS_TOKEN": "tk_test",
        "MASTODON_ACCOUNT_ID": "",
        "MASTODON_VISIBILITY": "",
        "MASTODON_MAX_MESSAGE_LEN": "",
    }
    for k, v in defaults.items():
        os.environ[k] = env.get(k, v)
    return ma.MastodonAdapter()


# ---- env / URL normalization ------------------------------------


def test_instance_url_strips_trailing_slash():
    a = _adapter(MASTODON_INSTANCE_URL="https://mastodon.example.com/")
    assert a.instance_url == "https://mastodon.example.com"


def test_missing_required_env_exits():
    with pytest.raises(SystemExit) as exc:
        _adapter(MASTODON_INSTANCE_URL="")
    assert exc.value.code == 2
    with pytest.raises(SystemExit):
        _adapter(MASTODON_ACCESS_TOKEN="")


def test_invalid_scheme_rejected():
    with pytest.raises(SystemExit) as exc:
        _adapter(MASTODON_INSTANCE_URL="gemini://mastodon.example.com")
    assert exc.value.code == 2


def test_default_visibility_is_unlisted():
    a = _adapter()
    assert a.default_visibility == "unlisted"


def test_visibility_override_validated():
    a = _adapter(MASTODON_VISIBILITY="public")
    assert a.default_visibility == "public"
    with pytest.raises(SystemExit) as exc:
        _adapter(MASTODON_VISIBILITY="loud")
    assert exc.value.code == 2


def test_max_message_len_default_and_override():
    a = _adapter()
    assert a.max_message_len == 500
    a = _adapter(MASTODON_MAX_MESSAGE_LEN="4000")
    assert a.max_message_len == 4000


def test_max_message_len_invalid_exits():
    with pytest.raises(SystemExit):
        _adapter(MASTODON_MAX_MESSAGE_LEN="not-a-number")
    with pytest.raises(SystemExit):
        _adapter(MASTODON_MAX_MESSAGE_LEN="-1")


def test_account_id_optional():
    a = _adapter(MASTODON_ACCOUNT_ID="prod")
    assert a.account_id == "prod"
    a = _adapter(MASTODON_ACCOUNT_ID="")
    assert a.account_id is None


def test_suppress_error_responses_is_true():
    a = _adapter()
    # Mastodon posts are public — operator errors must never echo.
    assert a.suppress_error_responses is True
    ev = a.ready_event()
    # ready_event() should advertise the suppress flag.
    params = ev["params"]
    assert params.get("suppress_error_responses") is True


# ---- HTML stripper ---------------------------------------------


def test_strip_html_plain_passthrough():
    assert ma._strip_html_tags("hello world") == "hello world"


def test_strip_html_basic_tags_removed():
    assert ma._strip_html_tags("<p>hello <b>world</b></p>") == "hello world"


def test_strip_html_block_close_inserts_newline():
    # </p>, </div>, </li>, <br> should produce newlines.
    out = ma._strip_html_tags("<p>line1</p><p>line2</p>")
    assert out == "line1\nline2"
    assert ma._strip_html_tags("a<br>b<br>c").startswith("a\nb")


def test_strip_html_entities_decoded():
    # Named, decimal, hex.
    assert ma._strip_html_tags("&amp; &#65; &#x42;") == "& A B"


def test_strip_html_mention_anchor_typical_mastodon_shape():
    src = (
        '<p><span class="h-card"><a href="https://example/@bot" '
        'class="u-url mention">@<span>bot</span></a></span> hello</p>'
    )
    out = ma._strip_html_tags(src)
    assert "hello" in out
    assert "@bot" in out
    assert "<" not in out and ">" not in out


# ---- _parse_notification: shape preserved -------------------------


def _notif_fixture(mention_text="hello",
                   own_id="own-123",
                   sender_id="acc-7",
                   status_id="status-42",
                   in_reply_to=None,
                   visibility="public",
                   notif_type="mention"):
    return {
        "id": "notif-99",
        "type": notif_type,
        "account": {
            "id": sender_id,
            "username": "alice",
            "display_name": "Alice",
            "acct": "alice@example.com",
        },
        "status": {
            "id": status_id,
            "content": f"<p>{mention_text}</p>",
            "visibility": visibility,
            "in_reply_to_id": in_reply_to,
        },
    }, own_id


def test_parse_notification_full_shape():
    a = _adapter()
    a.own_account_id = "own-123"
    notif, _ = _notif_fixture()
    ev = a._parse_notification(notif)
    assert ev is not None
    assert ev["method"] == "message"
    p = ev["params"]
    assert p["user_id"] == "acc-7"
    assert p["user_name"] == "Alice"
    assert p["content"] == {"Text": "hello"}
    assert p["message_id"] == "status-42"
    assert p["metadata"] == {
        "status_id": "status-42",
        "notification_id": "notif-99",
        "acct": "alice@example.com",
        "visibility": "public",
    }


def test_parse_notification_thread_id_carries_in_reply_to():
    a = _adapter()
    a.own_account_id = "own-123"
    notif, _ = _notif_fixture(in_reply_to="status-prev")
    ev = a._parse_notification(notif)
    p = ev["params"]
    assert p["thread_id"] == "status-prev"
    assert p["metadata"]["in_reply_to_id"] == "status-prev"


def test_parse_notification_skips_non_mention():
    a = _adapter()
    a.own_account_id = "own-123"
    notif, _ = _notif_fixture(notif_type="favourite")
    assert a._parse_notification(notif) is None


def test_parse_notification_skips_self_mention():
    a = _adapter()
    a.own_account_id = "own-123"
    notif, _ = _notif_fixture(sender_id="own-123")
    assert a._parse_notification(notif) is None


def test_parse_notification_empty_text_skipped():
    a = _adapter()
    a.own_account_id = "own-123"
    notif, _ = _notif_fixture(mention_text="")
    notif["status"]["content"] = "<p></p>"
    assert a._parse_notification(notif) is None


def test_parse_notification_slash_command():
    a = _adapter()
    a.own_account_id = "own-123"
    notif, _ = _notif_fixture(mention_text="/help me out")
    ev = a._parse_notification(notif)
    p = ev["params"]
    assert p["content"] == {
        "Command": {"name": "help", "args": ["me", "out"]}
    }


def test_parse_notification_display_name_falls_back_to_username():
    a = _adapter()
    a.own_account_id = "own-123"
    notif, _ = _notif_fixture()
    notif["account"]["display_name"] = ""
    ev = a._parse_notification(notif)
    assert ev["params"]["user_name"] == "alice"


# ---- _split_message ---------------------------------------------


def test_split_message_under_limit_one_chunk():
    assert ma._split_message("short", 100) == ["short"]


def test_split_message_prefers_newline_cut():
    body = "a" * 80 + "\n" + "b" * 80
    chunks = ma._split_message(body, 100)
    assert len(chunks) == 2
    assert chunks[0] == "a" * 80
    assert chunks[1] == "b" * 80


def test_split_message_hard_cut_when_no_newline():
    chunks = ma._split_message("x" * 250, 100)
    assert [len(c) for c in chunks] == [100, 100, 50]


# ---- _post_status: REST shape ------------------------------------


class _FakeUrlopen:
    def __init__(self, status=200, reply_ids=None):
        self.calls: list[dict] = []
        self.status = status
        self._reply_ids = list(reply_ids) if reply_ids else ["resp-1", "resp-2", "resp-3"]
        self._idx = 0

    def __call__(self, req, timeout=None):
        body = req.data
        decoded_params = {}
        try:
            from urllib.parse import parse_qsl
            decoded_params = dict(parse_qsl(body.decode("utf-8")))
        except Exception:
            pass
        self.calls.append({
            "url": req.full_url,
            "method": req.get_method(),
            "headers": {k.lower(): v for k, v in req.header_items()},
            "params": decoded_params,
            "timeout": timeout,
        })
        idx = self._idx
        self._idx += 1
        rid = self._reply_ids[idx] if idx < len(self._reply_ids) else f"resp-{idx}"
        return _FakeResp(self.status, json.dumps({"id": rid}).encode("utf-8"))


class _FakeResp:
    def __init__(self, status, body=b"{}"):
        self.status = status
        self._body = body

    def read(self):
        return self._body

    def __enter__(self):
        return self

    def __exit__(self, *_):
        return False


def test_post_status_bearer_auth_form_visibility(monkeypatch):
    a = _adapter()
    fake = _FakeUrlopen()
    monkeypatch.setattr(ma.urllib.request, "urlopen", fake)
    a._post_status("Hello world", in_reply_to_id=None)
    assert len(fake.calls) == 1
    c = fake.calls[0]
    assert c["url"] == "https://mastodon.example.com/api/v1/statuses"
    assert c["method"] == "POST"
    assert c["headers"]["authorization"] == "Bearer tk_test"
    assert c["headers"]["content-type"] == "application/x-www-form-urlencoded"
    assert c["params"]["status"] == "Hello world"
    assert c["params"]["visibility"] == "unlisted"
    assert "in_reply_to_id" not in c["params"]


def test_post_status_chunks_chain_replies(monkeypatch):
    a = _adapter()
    fake = _FakeUrlopen(reply_ids=["id-1", "id-2", "id-3"])
    monkeypatch.setattr(ma.urllib.request, "urlopen", fake)
    long = "x" * (a.max_message_len * 2 + 50)
    a._post_status(long, in_reply_to_id=None)
    assert len(fake.calls) == 3
    # First chunk has no in_reply_to_id; subsequent chunks chain.
    assert "in_reply_to_id" not in fake.calls[0]["params"]
    assert fake.calls[1]["params"]["in_reply_to_id"] == "id-1"
    assert fake.calls[2]["params"]["in_reply_to_id"] == "id-2"


def test_post_status_reply_to_inbound_thread(monkeypatch):
    a = _adapter()
    fake = _FakeUrlopen()
    monkeypatch.setattr(ma.urllib.request, "urlopen", fake)
    a._post_status("reply", in_reply_to_id="orig-status-123")
    c = fake.calls[0]
    assert c["params"]["in_reply_to_id"] == "orig-status-123"


def test_post_status_http_error_surfaced(monkeypatch):
    a = _adapter()

    class _HTTPError(ma.urllib.error.HTTPError):
        def __init__(self):
            super().__init__("u", 401, "Unauthorized", {},
                             io.BytesIO(b'{"error":"invalid token"}'))

    def _bad(req, timeout=None):
        raise _HTTPError()

    monkeypatch.setattr(ma.urllib.request, "urlopen", _bad)
    with pytest.raises(RuntimeError, match="401"):
        a._post_status("hi", in_reply_to_id=None)


def test_post_status_5xx_surfaced(monkeypatch):
    a = _adapter()
    fake = _FakeUrlopen(status=500)
    monkeypatch.setattr(ma.urllib.request, "urlopen", fake)
    with pytest.raises(RuntimeError, match="HTTP 500"):
        a._post_status("hi", in_reply_to_id=None)


def test_post_status_custom_visibility(monkeypatch):
    a = _adapter(MASTODON_VISIBILITY="private")
    fake = _FakeUrlopen()
    monkeypatch.setattr(ma.urllib.request, "urlopen", fake)
    a._post_status("private toot", in_reply_to_id=None)
    assert fake.calls[0]["params"]["visibility"] == "private"


# ---- account_id surfaced via ready_event --------------------------


def test_account_id_in_ready_event():
    a = _adapter(MASTODON_ACCOUNT_ID="instance-a")
    p = a.ready_event()["params"]
    assert p.get("account_id") == "instance-a"


def test_no_account_id_when_unset():
    a = _adapter(MASTODON_ACCOUNT_ID="")
    p = a.ready_event()["params"]
    assert p.get("account_id") in (None, )


# ---- _verify_credentials: discovers own_account_id ----------------


def _verify_resp(status=200, body=None):
    payload = json.dumps(body or {"id": "own-1", "username": "bot"}).encode("utf-8")
    return _FakeResp(status, payload)


def test_verify_credentials_sets_own_account_id(monkeypatch):
    """The Rust adapter calls /api/v1/accounts/verify_credentials at
    start to validate the token AND discover the bot's own account id.
    Without this, the self-mention guard in `_parse_notification` is
    silently disabled. Lock in that we (a) hit the right endpoint with
    Bearer auth, (b) populate own_account_id from the response."""
    a = _adapter()
    captured = {}

    def fake_urlopen(req, timeout=None):
        captured["url"] = req.full_url
        captured["auth"] = dict(req.header_items()).get("Authorization")
        return _verify_resp(body={"id": "bot-9001", "username": "myhandle"})

    monkeypatch.setattr(ma.urllib.request, "urlopen", fake_urlopen)
    username = a._verify_credentials()
    assert username == "myhandle"
    assert a.own_account_id == "bot-9001"
    assert captured["url"] == (
        "https://mastodon.example.com/api/v1/accounts/verify_credentials"
    )
    assert captured["auth"] == "Bearer tk_test"


def test_verify_credentials_raises_on_bad_token(monkeypatch):
    a = _adapter()

    class _HTTPError(ma.urllib.error.HTTPError):
        def __init__(self):
            super().__init__("u", 401, "Unauthorized", {},
                             io.BytesIO(b'{"error":"invalid token"}'))

    def _bad(req, timeout=None):
        raise _HTTPError()

    monkeypatch.setattr(ma.urllib.request, "urlopen", _bad)
    with pytest.raises(ma.urllib.error.HTTPError):
        a._verify_credentials()
    # On failure the field stays None so the self-mention guard never
    # short-circuits with a stale id.
    assert a.own_account_id is None


def test_self_mention_skipped_only_after_verify():
    """Belt-and-braces: confirm the parse path's guard depends on
    own_account_id being set. Before verify, a self-mention DOES come
    through (own_account_id None → guard disabled), so verify MUST be
    called to gate that. After verify, the same notification is
    silenced."""
    a = _adapter()
    assert a.own_account_id is None
    notif, _ = _notif_fixture(sender_id="own-1")
    # Pre-verify: the guard is `if self.own_account_id and …`, so when
    # own_account_id is None/falsy the self-mention is NOT filtered.
    # That's exactly the latent bug pattern — fixed by always calling
    # verify before _producer_blocking enters its SSE/poll loop.
    pre = a._parse_notification(notif)
    assert pre is not None, "pre-verify the guard is disabled — known latent shape"
    # After verify (simulated by setting the field), the same payload
    # is silenced.
    a.own_account_id = "own-1"
    assert a._parse_notification(notif) is None
