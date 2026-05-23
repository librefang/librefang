"""Direct tests for ``librefang.sidecar.common``.

The shared helpers are exercised transitively through every sidecar
test suite, but that means a behaviour change in (say) ``SeenSet``
forces churn across 18 test files before anyone can verify the
shared module on its own. These tests cover the module's public
surface directly — they're cheap, fast, and exist so future
maintenance (caps, parameters, new helpers) has a single place to
land regression coverage.
"""
from __future__ import annotations

import io
import json
import threading
import urllib.error
import urllib.request

import pytest

from librefang.sidecar.common import (
    DEFAULT_SEEN_EVICT,
    DEFAULT_SEEN_MAX,
    MAX_BACKOFF_SECS,
    RETRY_AFTER_DEFAULT_SECS,
    SeenSet,
    http_request,
    parse_retry_after,
    split_csv,
    split_message,
)


# ---- module-level constants ------------------------------------------


def test_canonical_constants():
    """These values are the canonical defaults every sidecar adapter
    inherits — change them only if you mean to change every adapter's
    behaviour."""
    assert MAX_BACKOFF_SECS == 60.0
    assert RETRY_AFTER_DEFAULT_SECS == 30.0
    assert DEFAULT_SEEN_MAX == 10_000
    assert DEFAULT_SEEN_EVICT == 5_000


# ---- split_message ---------------------------------------------------


def test_split_message_short_passthrough():
    assert split_message("hi", 10) == ["hi"]


def test_split_message_exact_limit():
    """A string exactly at the limit is one chunk, not two."""
    assert split_message("a" * 5, 5) == ["aaaaa"]


def test_split_message_hard_cut_no_newline():
    """No newline in the window → hard cut at the limit."""
    assert split_message("abcdefghi", 3) == ["abc", "def", "ghi"]


def test_split_message_prefers_newline():
    assert split_message("abc\ndef\nghi", 5) == ["abc", "def", "ghi"]


def test_split_message_strips_leading_newline_after_cut():
    """After a newline-cut, the leftover's leading newlines are
    stripped so the next chunk doesn't start blank. The first
    chunk keeps trailing newlines up to (but not including) the
    chosen cut point — only the leftover gets ``lstrip("\\n")``."""
    out = split_message("abc\n\n\ndef", 5)
    # Second+ chunks never start with a newline.
    for c in out[1:]:
        assert not c.startswith("\n")
    # Joining preserves all original content except the newline(s) we
    # cut on.
    assert "".join(out).replace("\n", "") == "abcdef"


def test_split_message_empty_input():
    assert split_message("", 5) == [""]


def test_split_message_unicode_chars_use_char_count():
    """``split_message`` measures in characters, not bytes — a CJK
    character is one unit regardless of UTF-8 width."""
    s = "你好世界"  # 4 chars, 12 bytes in UTF-8
    assert split_message(s, 2) == ["你好", "世界"]


# ---- split_csv -------------------------------------------------------


def test_split_csv_empty_returns_empty_list():
    assert split_csv("") == []


def test_split_csv_basic():
    assert split_csv("a,b,c") == ["a", "b", "c"]


def test_split_csv_strips_whitespace():
    assert split_csv("  a , b  , c  ") == ["a", "b", "c"]


def test_split_csv_drops_empty_entries():
    """Trailing comma, double comma, all-whitespace entry → dropped."""
    assert split_csv("a,,b, ,c,") == ["a", "b", "c"]


def test_split_csv_preserves_order():
    assert split_csv("c,a,b") == ["c", "a", "b"]


# ---- parse_retry_after -----------------------------------------------


def test_retry_after_missing_returns_default():
    assert parse_retry_after({}, default_secs=30.0) == 30.0


def test_retry_after_integer_seconds():
    assert parse_retry_after(
        {"retry-after": "12"}, default_secs=30.0,
    ) == 12.0


def test_retry_after_decimal_seconds():
    assert parse_retry_after(
        {"retry-after": "1.5"}, default_secs=30.0,
    ) == 1.5


def test_retry_after_floor_clamps_zero_to_one():
    """Default floor is 1.0 — a 0 retry-after gets clamped up."""
    assert parse_retry_after(
        {"retry-after": "0"}, default_secs=30.0,
    ) == 1.0


def test_retry_after_floor_override():
    """discord overrides to 0.1 because its rate limiter is sub-second."""
    assert parse_retry_after(
        {"retry-after": "0"}, default_secs=30.0, floor_secs=0.1,
    ) == 0.1


def test_retry_after_max_clamp_default():
    """Default max is ``MAX_BACKOFF_SECS`` = 60.0."""
    assert parse_retry_after(
        {"retry-after": "9999"}, default_secs=30.0,
    ) == 60.0


def test_retry_after_max_clamp_override():
    """Custom max_secs caps higher."""
    assert parse_retry_after(
        {"retry-after": "9999"}, default_secs=30.0, max_secs=120.0,
    ) == 120.0


def test_retry_after_garbage_returns_default():
    assert parse_retry_after(
        {"retry-after": "later please"}, default_secs=30.0,
    ) == 30.0


def test_retry_after_negative_clamped_to_floor():
    """A server bug returning a negative value still clamps up to floor."""
    assert parse_retry_after(
        {"retry-after": "-5"}, default_secs=30.0,
    ) == 1.0


# ---- SeenSet ---------------------------------------------------------


def test_seenset_fresh_returns_true():
    s = SeenSet()
    assert s.mark("a") is True


def test_seenset_repeat_returns_false():
    s = SeenSet()
    s.mark("a")
    assert s.mark("a") is False


def test_seenset_distinct_ids_independent():
    s = SeenSet()
    assert s.mark("a") is True
    assert s.mark("b") is True
    assert s.mark("a") is False
    assert s.mark("b") is False


def test_seenset_empty_id_always_fresh_no_state():
    """Empty / None ids are treated as fresh (return True) but never
    stored — they don't participate in dedupe so the caller's "skip
    this anyway" guard runs at the caller's discretion. Webex
    overrides this externally in its `_mark_seen` shim."""
    s = SeenSet()
    assert s.mark("") is True
    assert s.mark(None) is True  # type: ignore[arg-type]
    assert "" not in s.ids
    assert None not in s.ids
    assert len(s) == 0


def test_seenset_eviction_drops_oldest_batch():
    s = SeenSet(max_size=4, evict=2)
    for x in "abcd":
        s.mark(x)
    # Still at cap, nothing evicted yet.
    assert s.ids == {"a", "b", "c", "d"}
    # Overflow → evict the oldest 2.
    s.mark("e")
    assert "a" not in s
    assert "b" not in s
    assert "c" in s
    assert "d" in s
    assert "e" in s


def test_seenset_contains_via_in_operator():
    s = SeenSet()
    s.mark("hello")
    assert "hello" in s
    assert "world" not in s


def test_seenset_len_tracks_unique_ids():
    s = SeenSet()
    assert len(s) == 0
    s.mark("a")
    s.mark("a")
    s.mark("b")
    assert len(s) == 2


def test_seenset_thread_safety_under_concurrent_marks():
    """Spin 8 threads each marking 1000 distinct ids; the set must
    contain all 8000 entries at the end without races dropping or
    duplicating ids. (The internal lock is the whole reason this
    class exists rather than a bare set.)"""
    s = SeenSet(max_size=100_000, evict=50_000)
    threads = []

    def worker(prefix):
        for i in range(1000):
            s.mark(f"{prefix}-{i}")

    for p in range(8):
        t = threading.Thread(target=worker, args=(p,))
        threads.append(t)
        t.start()
    for t in threads:
        t.join()

    assert len(s) == 8_000


def test_seenset_uses_int_ids():
    """nextcloud uses ``int`` ids (Talk message ids are positive
    integers). SeenSet must work generically — not just for strings."""
    s = SeenSet()
    assert s.mark(1) is True
    assert s.mark(1) is False
    assert s.mark(0) is True  # falsy → treated as fresh, not stored
    assert 0 not in s.ids


# ---- http_request ----------------------------------------------------


class _FakeResp:
    """Bare minimum stand-in for the http.client.HTTPResponse interface
    http_request() touches: ``.status``, ``.read()``, ``.headers``."""

    def __init__(self, status, body=b"", headers=None):
        self.status = status
        self._body = body
        self.headers = _Hdrs(headers or {})

    def read(self):
        return self._body

    def __enter__(self):
        return self

    def __exit__(self, *_):
        return False


class _Hdrs:
    def __init__(self, d):
        self._d = d

    def items(self):
        return list(self._d.items())


def _fake_urlopen(script):
    """Returns a context manager that pops responses off a script."""
    state = {"script": list(script), "calls": []}

    def call(req, timeout=None):
        state["calls"].append({
            "url": req.full_url,
            "method": req.get_method(),
            "timeout": timeout,
            "body": req.data,
        })
        entry = state["script"].pop(0)
        if len(entry) == 3:
            status, body, hdrs = entry
        else:
            status, body = entry
            hdrs = {}
        if status >= 400:
            raise urllib.error.HTTPError(
                req.full_url, status, "Error", _Hdrs(hdrs),
                io.BytesIO(json.dumps(body or {}).encode("utf-8")),
            )
        if body is None:
            payload = b""
        elif isinstance(body, (dict, list)):
            payload = json.dumps(body).encode("utf-8")
        elif isinstance(body, bytes):
            payload = body
        else:
            payload = str(body).encode("utf-8")
        return _FakeResp(status, payload, hdrs)

    call.state = state
    return call


def test_http_request_200_parses_json_body(monkeypatch):
    fake = _fake_urlopen([(200, {"hello": "world"})])
    monkeypatch.setattr(urllib.request, "urlopen", fake)
    status, body, raw, hdrs = http_request("https://x.test/")
    assert status == 200
    assert body == {"hello": "world"}
    assert raw == b'{"hello": "world"}'


def test_http_request_records_method_and_body(monkeypatch):
    fake = _fake_urlopen([(201, {"ok": True})])
    monkeypatch.setattr(urllib.request, "urlopen", fake)
    http_request(
        "https://x.test/", method="POST",
        body=b'{"k":"v"}',
        headers={"X-Test": "1"},
        timeout=7.5,
    )
    c = fake.state["calls"][0]
    assert c["method"] == "POST"
    assert c["body"] == b'{"k":"v"}'
    assert c["timeout"] == 7.5


def test_http_request_lowercases_response_headers(monkeypatch):
    """``Retry-After`` lookups stay case-insensitive across servers
    that emit ``retry-after`` vs ``Retry-After``."""
    fake = _fake_urlopen([(429, {"err": "rate"}, {"Retry-After": "5"})])
    monkeypatch.setattr(urllib.request, "urlopen", fake)
    status, _body, _raw, hdrs = http_request("https://x.test/")
    assert status == 429
    assert hdrs == {"retry-after": "5"}


def test_http_request_4xx_surfaces_status_not_raises(monkeypatch):
    """HTTPError is caught and the status code is exposed in the
    tuple — adapters that want to raise must do so themselves."""
    fake = _fake_urlopen([(404, {"err": "not found"})])
    monkeypatch.setattr(urllib.request, "urlopen", fake)
    status, body, raw, _hdrs = http_request("https://x.test/")
    assert status == 404
    assert body == {"err": "not found"}


def test_http_request_5xx_surfaces_status(monkeypatch):
    fake = _fake_urlopen([(503, None)])
    monkeypatch.setattr(urllib.request, "urlopen", fake)
    status, _body, _raw, _hdrs = http_request("https://x.test/")
    assert status == 503


def test_http_request_empty_body_returns_none(monkeypatch):
    fake = _fake_urlopen([(204, None)])
    monkeypatch.setattr(urllib.request, "urlopen", fake)
    status, body, raw, _hdrs = http_request("https://x.test/")
    assert status == 204
    assert body is None
    assert raw == b""


def test_http_request_non_json_body_returns_raw_only(monkeypatch):
    fake = _fake_urlopen([(200, b"hello plain text")])
    monkeypatch.setattr(urllib.request, "urlopen", fake)
    status, body, raw, _hdrs = http_request("https://x.test/")
    assert status == 200
    assert body is None  # not parseable as JSON
    assert raw == b"hello plain text"


def test_http_request_default_timeout(monkeypatch):
    fake = _fake_urlopen([(200, None)])
    monkeypatch.setattr(urllib.request, "urlopen", fake)
    http_request("https://x.test/")
    assert fake.state["calls"][0]["timeout"] == 15.0


def test_http_request_get_method_default(monkeypatch):
    """Default method is GET — adapters that need a body still
    have to pass method='POST' explicitly."""
    fake = _fake_urlopen([(200, {})])
    monkeypatch.setattr(urllib.request, "urlopen", fake)
    http_request("https://x.test/")
    assert fake.state["calls"][0]["method"] == "GET"
