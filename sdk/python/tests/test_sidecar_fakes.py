"""Tests for ``tests._sidecar_fakes``.

This module is a test fixture, but it's load-bearing across 12
sidecar test files — a regression in ``FakeUrlopen`` would silently
break the entire suite. Direct coverage keeps regressions
detectable in isolation.
"""
from __future__ import annotations

import json
import urllib.error
import urllib.request

import pytest

from _sidecar_fakes import FakeResp, FakeUrlopen, HdrShim


# ---- HdrShim ---------------------------------------------------------


def test_hdrshim_items():
    h = HdrShim({"Content-Type": "application/json", "X-Foo": "1"})
    assert dict(h.items()) == {
        "Content-Type": "application/json", "X-Foo": "1",
    }


def test_hdrshim_empty_default():
    h = HdrShim()
    assert list(h.items()) == []


# ---- FakeResp --------------------------------------------------------


def test_fakeresp_is_context_manager():
    """Sidecar code reads responses inside ``with ... as resp:``
    blocks, so FakeResp must implement the CM protocol."""
    fr = FakeResp(200, b"hello")
    with fr as resp:
        assert resp.status == 200
        assert resp.read() == b"hello"


def test_fakeresp_default_headers_empty():
    fr = FakeResp(204)
    assert fr.headers.items() == []


# ---- FakeUrlopen — script handling ----------------------------------


def _make_req(url="https://x.test/", method="GET", body=None):
    return urllib.request.Request(url, data=body, method=method)


def test_fakeurlopen_returns_scripted_response():
    fake = FakeUrlopen([(200, {"ok": True})])
    resp = fake(_make_req())
    assert resp.status == 200
    assert json.loads(resp.read()) == {"ok": True}


def test_fakeurlopen_records_call_metadata():
    fake = FakeUrlopen([(200, {})])
    req = _make_req(
        "https://x.test/v2/post", method="POST",
        body=b'{"k":"v"}',
    )
    req.add_header("Authorization", "Bearer abc")
    fake(req, timeout=7.5)
    c = fake.calls[0]
    assert c["url"] == "https://x.test/v2/post"
    assert c["method"] == "POST"
    assert c["timeout"] == 7.5
    assert c["headers"]["authorization"] == "Bearer abc"


def test_fakeurlopen_decodes_body_in_call_record():
    """Each call record carries both ``body_raw`` (str), ``body``
    (parsed JSON), and ``params`` (form-decoded) so any historical
    inlined variant's assertions still find the field it reads."""
    fake = FakeUrlopen([(201, {})])
    fake(_make_req(
        "https://x.test/",
        method="POST",
        body=b'{"key": "value"}',
    ))
    c = fake.calls[0]
    assert c["body_raw"] == '{"key": "value"}'
    assert c["body"] == {"key": "value"}


def test_fakeurlopen_records_form_params():
    """Form-encoded bodies (mastodon's `POST /api/v1/statuses`) get
    parsed into ``params``."""
    fake = FakeUrlopen([(200, {})])
    fake(_make_req(
        "https://x.test/",
        method="POST",
        body=b"status=hi&visibility=public",
    ))
    assert fake.calls[0]["params"] == {
        "status": "hi", "visibility": "public",
    }


def test_fakeurlopen_3_tuple_with_headers():
    """The script's optional 3rd element is response headers."""
    fake = FakeUrlopen([(429, {"err": "rate"}, {"Retry-After": "5"})])
    with pytest.raises(urllib.error.HTTPError) as exc:
        fake(_make_req())
    assert exc.value.code == 429
    assert exc.value.headers.items() == [("Retry-After", "5")]


def test_fakeurlopen_4xx_raises_httperror():
    fake = FakeUrlopen([(404, {"err": "not found"})])
    with pytest.raises(urllib.error.HTTPError) as exc:
        fake(_make_req())
    assert exc.value.code == 404


def test_fakeurlopen_5xx_raises_httperror():
    fake = FakeUrlopen([(503, None)])
    with pytest.raises(urllib.error.HTTPError):
        fake(_make_req())


def test_fakeurlopen_script_exhaustion_asserts():
    """An extra urlopen call past the script's end fails the test
    loudly — silent infinity-mocking is the opposite of what we want."""
    fake = FakeUrlopen([(200, {})])
    fake(_make_req())  # consume the one entry
    with pytest.raises(AssertionError, match="unexpected extra urlopen"):
        fake(_make_req())


def test_fakeurlopen_empty_body_passthrough():
    fake = FakeUrlopen([(200, None)])
    resp = fake(_make_req())
    assert resp.status == 200
    assert resp.read() == b""


def test_fakeurlopen_bytes_body_passthrough():
    fake = FakeUrlopen([(200, b"plain text response")])
    resp = fake(_make_req())
    assert resp.read() == b"plain text response"


def test_fakeurlopen_backcompat_underscore_aliases():
    """Adapter test files import the underscore-prefixed names
    (``_FakeUrlopen``, ``_FakeResp``, ``_HdrShim``). The shared
    helper exposes both forms — the bare and underscore variants
    are the same class."""
    from _sidecar_fakes import _FakeResp, _FakeUrlopen, _HdrShim
    assert _FakeUrlopen is FakeUrlopen
    assert _FakeResp is FakeResp
    assert _HdrShim is HdrShim
