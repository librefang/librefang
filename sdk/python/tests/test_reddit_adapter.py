"""Tests for librefang.sidecar.adapters.reddit.

Deterministic, no network: urllib is monkeypatched. Asserts the
sidecar Reddit adapter preserves the behaviour of the removed
in-process Rust ``librefang-channels::reddit`` adapter, plus two
explicitly-acknowledged improvements:

* P1 (b): ``thread_id = fullname`` on inbound, so ``on_send`` uses
  ``cmd.thread_id`` directly as ``thing_id`` for ``POST /api/comment``.
  The Rust adapter set ``thread_id = subreddit`` and tried to pass
  ``user.platform_id`` as the fullname — but ``user.platform_id``
  was the author username (parse_reddit_comment wrote it there), not
  the fullname Reddit's API requires.
* P2 (b): ``suppress_error_responses = True``. Reddit comments are
  public; never echo internal errors as a reply.
"""

import io
import json
import os
import urllib.error
import urllib.parse

import pytest

# Required env must be present at import time because the adapter
# raises SystemExit(2) if unset on construction.
os.environ.setdefault("REDDIT_CLIENT_ID", "test-client-id")
os.environ.setdefault("REDDIT_CLIENT_SECRET", "test-client-secret")
os.environ.setdefault("REDDIT_USERNAME", "test-user")
os.environ.setdefault("REDDIT_PASSWORD", "test-pass")
os.environ.setdefault("REDDIT_SUBREDDITS", "rust")
from librefang.sidecar.adapters import reddit as ra  # noqa: E402


def _adapter(**env):
    defaults = {
        "REDDIT_CLIENT_ID": "test-client-id",
        "REDDIT_CLIENT_SECRET": "test-client-secret",
        "REDDIT_USERNAME": "test-user",
        "REDDIT_PASSWORD": "test-pass",
        "REDDIT_SUBREDDITS": "rust",
        "REDDIT_ACCOUNT_ID": "",
        "REDDIT_USER_AGENT": "",
    }
    for k, v in defaults.items():
        os.environ[k] = env.get(k, v)
    a = ra.RedditAdapter()
    # Test URL injection (mirrors the Rust adapter's with_token_url /
    # with_api_base test hooks).
    if "TOKEN_URL" in env:
        a.token_url = env["TOKEN_URL"]
    if "API_BASE" in env:
        a.api_base = env["API_BASE"]
    return a


# ---- env handling -------------------------------------------------


def test_default_urls_and_user_agent():
    a = _adapter()
    assert a.token_url == "https://www.reddit.com/api/v1/access_token"
    assert a.api_base == "https://oauth.reddit.com"
    assert a.user_agent.startswith("librefang:")


def test_custom_user_agent():
    a = _adapter(REDDIT_USER_AGENT="my-bot/1.0 (by /u/me)")
    assert a.user_agent == "my-bot/1.0 (by /u/me)"


def test_missing_required_env_exits():
    for var in (
        "REDDIT_CLIENT_ID",
        "REDDIT_CLIENT_SECRET",
        "REDDIT_USERNAME",
        "REDDIT_PASSWORD",
        "REDDIT_SUBREDDITS",
    ):
        with pytest.raises(SystemExit) as exc:
            _adapter(**{var: ""})
        assert exc.value.code == 2, var


def test_subreddits_parsed_and_normalised():
    a = _adapter(REDDIT_SUBREDDITS="rust, r/programming ,r/librefang/")
    assert a.subreddits == ["rust", "programming", "librefang"]


def test_account_id_optional():
    a = _adapter(REDDIT_ACCOUNT_ID="prod")
    assert a.account_id == "prod"
    a = _adapter(REDDIT_ACCOUNT_ID="")
    assert a.account_id is None


# ---- P2 (b): suppress + capabilities ------------------------------


def test_suppress_error_responses_is_true_in_ready_event():
    """P2 (b): Reddit replies are public; never echo internal errors."""
    a = _adapter()
    assert a.suppress_error_responses is True
    p = a.ready_event()["params"]
    assert p.get("suppress_error_responses") is True


def test_capabilities_empty():
    a = _adapter()
    assert a.capabilities == []


def test_account_id_in_ready_event():
    a = _adapter(REDDIT_ACCOUNT_ID="account-1")
    p = a.ready_event()["params"]
    assert p.get("account_id") == "account-1"


# ---- _split_message ----------------------------------------------


def test_split_message_under_limit_one_chunk():
    assert ra._split_message("short", 100) == ["short"]


def test_split_message_prefers_newline_cut():
    body = "a" * 80 + "\n" + "b" * 80
    chunks = ra._split_message(body, 100)
    assert len(chunks) == 2
    assert chunks[0] == "a" * 80
    assert chunks[1] == "b" * 80


def test_split_message_hard_cut_when_no_newline():
    chunks = ra._split_message("x" * 250, 100)
    assert [len(c) for c in chunks] == [100, 100, 50]


# ---- _parse_reddit_comment ----------------------------------------


def _comment(
    *,
    kind="t1",
    cid="abc123",
    fullname="t1_abc123",
    author="alice",
    body="Hello from Reddit!",
    subreddit="rust",
    link_id="t3_xyz789",
    parent_id="t3_xyz789",
    permalink="/r/rust/comments/xyz789/title/abc123/",
):
    return {
        "kind": kind,
        "data": {
            "id": cid,
            "name": fullname,
            "author": author,
            "body": body,
            "subreddit": subreddit,
            "link_id": link_id,
            "parent_id": parent_id,
            "permalink": permalink,
        },
    }


def test_parse_basic_text():
    ev = ra._parse_reddit_comment(_comment(), "bot-user")
    assert ev is not None
    p = ev["params"]
    assert p["user_id"] == "alice"
    assert p["user_name"] == "alice"
    assert p["content"] == {"Text": "Hello from Reddit!"}
    assert p["message_id"] == "abc123"
    # P1 (b): thread_id is the fullname (t1_abc123), not the subreddit
    assert p["thread_id"] == "t1_abc123"
    assert p["is_group"] is True
    md = p["metadata"]
    assert md["fullname"] == "t1_abc123"
    assert md["subreddit"] == "rust"
    assert md["link_id"] == "t3_xyz789"
    assert md["parent_id"] == "t3_xyz789"
    assert md["permalink"] == "/r/rust/comments/xyz789/title/abc123/"


def test_parse_skips_self_case_insensitive():
    assert ra._parse_reddit_comment(_comment(author="Bot-User"), "bot-user") is None


def test_parse_skips_deleted_and_removed():
    assert ra._parse_reddit_comment(_comment(author="[deleted]"), "bot") is None
    assert ra._parse_reddit_comment(_comment(author="[removed]"), "bot") is None


def test_parse_skips_empty_body():
    assert ra._parse_reddit_comment(_comment(body=""), "bot") is None


def test_parse_skips_posts_kind_t3():
    assert ra._parse_reddit_comment(_comment(kind="t3"), "bot") is None


def test_parse_command_form():
    ev = ra._parse_reddit_comment(_comment(body="/ask what is rust?"), "bot")
    assert ev["params"]["content"] == {
        "Command": {"name": "ask", "args": ["what", "is", "rust?"]},
    }


def test_parse_omits_permalink_when_absent():
    c = _comment(permalink="")
    ev = ra._parse_reddit_comment(c, "bot")
    assert "permalink" not in ev["params"]["metadata"]


def test_parse_returns_none_on_malformed():
    assert ra._parse_reddit_comment({}, "bot") is None
    assert ra._parse_reddit_comment({"kind": "t1"}, "bot") is None
    assert ra._parse_reddit_comment("nope", "bot") is None


# ---- _FakeUrlopen scaffolding --------------------------------------


class _FakeUrlopen:
    """Capture urllib.request.urlopen calls and return scripted
    responses. Each call pops the next response from `script`."""

    def __init__(self, script):
        self.script = list(script)
        self.calls = []

    def __call__(self, req, timeout=None):
        body_bytes = req.data
        try:
            decoded = body_bytes.decode("utf-8") if body_bytes else None
        except Exception:
            decoded = None
        self.calls.append({
            "url": req.full_url,
            "method": req.get_method(),
            "headers": {k.lower(): v for k, v in req.header_items()},
            "body_raw": decoded,
        })
        if not self.script:
            raise AssertionError(
                f"unexpected extra urlopen call to {req.full_url}"
            )
        status, body = self.script.pop(0)
        if status >= 400:
            raise urllib.error.HTTPError(
                req.full_url, status, "Error", {},
                io.BytesIO(json.dumps(body or {}).encode("utf-8")),
            )
        if body is None:
            payload = b""
        elif isinstance(body, (dict, list)):
            payload = json.dumps(body).encode("utf-8")
        else:
            payload = body if isinstance(body, bytes) else str(body).encode("utf-8")
        return _FakeResp(status, payload)


class _FakeResp:
    def __init__(self, status, body=b""):
        self.status = status
        self._body = body

    def read(self):
        return self._body

    def __enter__(self):
        return self

    def __exit__(self, *_):
        return False


def _form(call_body: str) -> dict:
    return dict(urllib.parse.parse_qsl(call_body or "", keep_blank_values=True))


# ---- token fetch / cache ------------------------------------------


def test_fetch_token_populates_cache(monkeypatch):
    a = _adapter()
    fake = _FakeUrlopen([(200, {
        "access_token": "tok-1",
        "token_type": "bearer",
        "expires_in": 3600,
    })])
    monkeypatch.setattr(ra.urllib.request, "urlopen", fake)
    tok = a._get_token()
    assert tok == "tok-1"
    assert a._cached_token is not None
    # Subsequent _get_token re-uses the cache (no second urlopen call).
    tok2 = a._get_token()
    assert tok2 == "tok-1"
    assert len(fake.calls) == 1


def test_fetch_token_sends_basic_auth_and_password_grant(monkeypatch):
    a = _adapter()
    fake = _FakeUrlopen([(200, {
        "access_token": "tok",
        "expires_in": 3600,
    })])
    monkeypatch.setattr(ra.urllib.request, "urlopen", fake)
    a._fetch_token()
    call = fake.calls[0]
    assert call["url"] == ra.DEFAULT_TOKEN_URL
    assert call["method"] == "POST"
    # Basic auth header built from client_id:client_secret
    import base64
    expected = "Basic " + base64.b64encode(
        b"test-client-id:test-client-secret"
    ).decode("ascii")
    assert call["headers"]["authorization"] == expected
    form = _form(call["body_raw"])
    assert form == {
        "grant_type": "password",
        "username": "test-user",
        "password": "test-pass",
    }


def test_fetch_token_raises_on_non_200(monkeypatch):
    a = _adapter()
    fake = _FakeUrlopen([(401, {"error": "invalid_grant"})])
    monkeypatch.setattr(ra.urllib.request, "urlopen", fake)
    with pytest.raises(RuntimeError, match="OAuth2 token error 401"):
        a._fetch_token()


def test_fetch_token_raises_on_missing_field(monkeypatch):
    a = _adapter()
    fake = _FakeUrlopen([(200, {"token_type": "bearer"})])  # no access_token
    monkeypatch.setattr(ra.urllib.request, "urlopen", fake)
    with pytest.raises(RuntimeError, match="missing access_token"):
        a._fetch_token()


def test_token_refresh_buffer_subtracted(monkeypatch):
    """expires_in=600 - TOKEN_REFRESH_BUFFER_SECS(300) → ~300s remaining."""
    a = _adapter()
    fake = _FakeUrlopen([(200, {"access_token": "tok", "expires_in": 600})])
    monkeypatch.setattr(ra.urllib.request, "urlopen", fake)
    before = ra.time.monotonic()
    a._get_token()
    _tok, expiry = a._cached_token
    delta = expiry - before
    # Allow generous slack: should be ~300 seconds, certainly not 600.
    assert 250 < delta < 350


# ---- verify_credentials -------------------------------------------


def test_verify_credentials_sets_own_username(monkeypatch):
    a = _adapter()
    fake = _FakeUrlopen([
        (200, {"access_token": "tok", "expires_in": 3600}),
        (200, {"name": "test-user"}),
    ])
    monkeypatch.setattr(ra.urllib.request, "urlopen", fake)
    name = a._verify_credentials()
    assert name == "test-user"
    assert a.own_username == "test-user"
    assert fake.calls[1]["url"].endswith("/api/v1/me")
    assert fake.calls[1]["headers"]["authorization"] == "Bearer tok"
    assert fake.calls[1]["headers"]["user-agent"] == a.user_agent


def test_verify_credentials_raises_on_401(monkeypatch):
    a = _adapter()
    fake = _FakeUrlopen([
        (200, {"access_token": "tok", "expires_in": 3600}),
        (401, {"message": "Unauthorized"}),
    ])
    monkeypatch.setattr(ra.urllib.request, "urlopen", fake)
    with pytest.raises(RuntimeError, match="authentication failed 401"):
        a._verify_credentials()


# ---- _post_comment: send-path -------------------------------------


def test_post_comment_basic_shape(monkeypatch):
    a = _adapter()
    a._cached_token = ("tok-cached", ra.time.monotonic() + 600)
    fake = _FakeUrlopen([
        (200, {"json": {"errors": [], "data": {"things": []}}}),
    ])
    monkeypatch.setattr(ra.urllib.request, "urlopen", fake)
    a._post_comment("t1_abc123", "hello reddit")
    call = fake.calls[0]
    assert call["url"] == "https://oauth.reddit.com/api/comment"
    assert call["method"] == "POST"
    assert call["headers"]["authorization"] == "Bearer tok-cached"
    assert call["headers"]["user-agent"] == a.user_agent
    form = _form(call["body_raw"])
    assert form == {
        "api_type": "json",
        "thing_id": "t1_abc123",
        "text": "hello reddit",
    }


def test_post_comment_chunks_join_with_separator(monkeypatch):
    """Reddit only allows one reply per parent — chunks join with
    CHUNK_JOIN rather than being posted as multiple comments
    (matches Rust adapter)."""
    a = _adapter()
    a._cached_token = ("tok", ra.time.monotonic() + 600)
    fake = _FakeUrlopen([
        (200, {"json": {"errors": [], "data": {"things": []}}}),
    ])
    monkeypatch.setattr(ra.urllib.request, "urlopen", fake)
    long_text = ("x" * ra.MAX_MESSAGE_LEN) + "\n" + ("y" * 100)
    a._post_comment("t1_xyz", long_text)
    assert len(fake.calls) == 1, "must be one POST regardless of chunk count"
    form = _form(fake.calls[0]["body_raw"])
    assert ra.CHUNK_JOIN in form["text"]


def test_post_comment_missing_fullname_raises():
    a = _adapter()
    with pytest.raises(RuntimeError, match="missing parent fullname"):
        a._post_comment("", "hi")


def test_post_comment_5xx_surfaced(monkeypatch):
    a = _adapter()
    a._cached_token = ("tok", ra.time.monotonic() + 600)
    fake = _FakeUrlopen([(500, {"error": "ServerError"})])
    monkeypatch.setattr(ra.urllib.request, "urlopen", fake)
    with pytest.raises(RuntimeError, match="500"):
        a._post_comment("t1_x", "hi")


def test_post_comment_401_refreshes_token_and_retries(monkeypatch):
    a = _adapter()
    a._cached_token = ("stale", ra.time.monotonic() + 600)
    fake = _FakeUrlopen([
        (401, {"message": "Unauthorized"}),  # first attempt fails
        (200, {"access_token": "fresh", "expires_in": 3600}),  # refetch
        (200, {"json": {"errors": [], "data": {}}}),  # retry succeeds
    ])
    monkeypatch.setattr(ra.urllib.request, "urlopen", fake)
    a._post_comment("t1_abc", "retry me")
    assert fake.calls[0]["headers"]["authorization"] == "Bearer stale"
    assert fake.calls[1]["url"].endswith("/api/v1/access_token")
    assert fake.calls[2]["headers"]["authorization"] == "Bearer fresh"


# ---- _poll_once: round-trip + dedupe + 401 ------------------------


def _children(*comments):
    return {"data": {"children": list(comments)}}


def test_poll_once_emits_parsed_comments(monkeypatch):
    a = _adapter()
    a.own_username = "test-user"
    a._cached_token = ("tok", ra.time.monotonic() + 600)
    fake = _FakeUrlopen([
        (200, _children(
            _comment(cid="c1", fullname="t1_c1", body="hi"),
            _comment(kind="t3", cid="p1"),  # skipped: post
            _comment(cid="c2", fullname="t1_c2", body="/help me"),
        )),
    ])
    monkeypatch.setattr(ra.urllib.request, "urlopen", fake)
    emitted = []
    a._poll_once(emitted.append)
    assert len(emitted) == 2
    assert emitted[0]["params"]["message_id"] == "c1"
    assert emitted[0]["params"]["thread_id"] == "t1_c1"
    assert emitted[1]["params"]["content"] == {
        "Command": {"name": "help", "args": ["me"]},
    }
    # Both comments tracked for dedupe (the t3 post is also tracked
    # under its id to avoid reparsing).
    assert "c1" in a._seen_comments_set
    assert "c2" in a._seen_comments_set


def test_poll_once_dedupes_seen_comments(monkeypatch):
    a = _adapter()
    a.own_username = "bot"
    a._cached_token = ("tok", ra.time.monotonic() + 600)
    a._mark_seen("c1")  # already seen
    fake = _FakeUrlopen([
        (200, _children(_comment(cid="c1"))),  # should be skipped
    ])
    monkeypatch.setattr(ra.urllib.request, "urlopen", fake)
    emitted = []
    a._poll_once(emitted.append)
    assert emitted == []


def test_poll_once_401_clears_token_and_raises(monkeypatch):
    a = _adapter()
    a.own_username = "bot"
    a._cached_token = ("stale", ra.time.monotonic() + 600)
    fake = _FakeUrlopen([(401, {"message": "Unauthorized"})])
    monkeypatch.setattr(ra.urllib.request, "urlopen", fake)
    with pytest.raises(RuntimeError, match="401"):
        a._poll_once(lambda _: None)
    assert a._cached_token is None


def test_poll_once_injects_account_id_into_metadata(monkeypatch):
    a = _adapter(REDDIT_ACCOUNT_ID="prod")
    a.own_username = "bot"
    a._cached_token = ("tok", ra.time.monotonic() + 600)
    fake = _FakeUrlopen([
        (200, _children(_comment(cid="c1"))),
    ])
    monkeypatch.setattr(ra.urllib.request, "urlopen", fake)
    emitted = []
    a._poll_once(emitted.append)
    assert emitted[0]["params"]["metadata"]["account_id"] == "prod"


def test_poll_once_skips_subreddit_on_transport_error(monkeypatch):
    """One bad subreddit doesn't take the loop down; the next
    subreddit's fetch still runs in the same poll pass."""
    a = _adapter(REDDIT_SUBREDDITS="rust,programming")
    a.own_username = "bot"
    a._cached_token = ("tok", ra.time.monotonic() + 600)
    calls_seen = []

    def fake_urlopen(req, timeout=None):
        url = req.full_url
        calls_seen.append(url)
        if "/r/rust/comments" in url:
            raise urllib.error.URLError("dns error")
        if "/r/programming/comments" in url:
            return _FakeResp(200, json.dumps(_children(
                _comment(cid="c1"),
            )).encode("utf-8"))
        raise AssertionError(f"unexpected url {url}")

    monkeypatch.setattr(ra.urllib.request, "urlopen", fake_urlopen)
    emitted = []
    a._poll_once(emitted.append)
    assert len(emitted) == 1
    assert emitted[0]["params"]["metadata"]["subreddit"] == "rust"  # parsed from response
    assert any("/r/rust/comments" in c for c in calls_seen)
    assert any("/r/programming/comments" in c for c in calls_seen)


# ---- seen_comments eviction ---------------------------------------


def test_seen_comments_capacity_eviction():
    a = _adapter()
    # Fill to one over the cap; oldest SEEN_COMMENTS_EVICT IDs evicted.
    for i in range(ra.SEEN_COMMENTS_MAX + 1):
        a._mark_seen(f"c{i}")
    # First half evicted; tail still present.
    assert "c0" not in a._seen_comments_set
    assert f"c{ra.SEEN_COMMENTS_EVICT - 1}" not in a._seen_comments_set
    assert f"c{ra.SEEN_COMMENTS_EVICT}" in a._seen_comments_set
    assert f"c{ra.SEEN_COMMENTS_MAX}" in a._seen_comments_set
    # List and set stay coherent.
    assert len(a._seen_comments) == len(a._seen_comments_set)


def test_seen_comments_idempotent_mark():
    a = _adapter()
    a._mark_seen("x")
    a._mark_seen("x")
    assert a._seen_comments.count("x") == 1


# ---- on_send: text fallback + thread_id round-trip ----------------


class _StubCmd:
    def __init__(self, *, text=None, content=None, thread_id=None):
        self.text = text
        self.content = content
        self.thread_id = thread_id


def test_on_send_uses_thread_id_as_parent_fullname(monkeypatch):
    """P1 (b): cmd.thread_id is the fullname; on_send must pass it
    straight to POST /api/comment as thing_id."""
    a = _adapter()
    a._cached_token = ("tok", ra.time.monotonic() + 600)
    fake = _FakeUrlopen([
        (200, {"json": {"errors": [], "data": {}}}),
    ])
    monkeypatch.setattr(ra.urllib.request, "urlopen", fake)
    import asyncio as _asyncio
    _asyncio.run(a.on_send(_StubCmd(text="hello", thread_id="t1_target")))
    form = _form(fake.calls[0]["body_raw"])
    assert form["thing_id"] == "t1_target"
    assert form["text"] == "hello"


def test_on_send_non_text_content_falls_back_to_placeholder(monkeypatch):
    a = _adapter()
    a._cached_token = ("tok", ra.time.monotonic() + 600)
    fake = _FakeUrlopen([
        (200, {"json": {"errors": [], "data": {}}}),
    ])
    monkeypatch.setattr(ra.urllib.request, "urlopen", fake)
    import asyncio as _asyncio
    _asyncio.run(a.on_send(_StubCmd(
        content={"Reaction": {"emoji": "👍"}},
        thread_id="t1_x",
    )))
    form = _form(fake.calls[0]["body_raw"])
    assert "Unsupported content type" in form["text"]
