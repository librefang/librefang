"""Shared test fakes for the sidecar adapter test suites.

Every reference adapter test historically inlined a near-identical
copy of:

* ``_FakeUrlopen`` — script-driven ``urllib.request.urlopen``
  replacement that records each call (url, method, headers,
  body_raw, timeout) and returns pre-baked responses. 14 distinct
  hash variants existed at extraction time; **12** of them were
  the same script-driven shape with cosmetic drift (whether the
  call record stores ``timeout``, the field name for the decoded
  body — ``body_raw`` vs ``body`` — and a few docstring
  differences). 2 (gotify, mastodon) were genuinely different
  ("status-only" / "form-encoded with rotating reply ids") and
  are kept inline in those test files.
* ``_FakeResp`` — minimal stand-in for ``http.client.HTTPResponse``
  matching the subset the sidecar HTTP helper actually touches:
  ``.status``, ``.read()``, ``.headers``, and the context-manager
  protocol.
* ``_HdrShim`` — dict-backed shim that quacks like
  ``email.message.Message`` for the response-headers fast path.

This module replaces those 12 + 14 + 14 = up to 40 separate
definitions with a single source of truth. The exported names are
the same as the inlined ones (with the underscore prefix dropped)
so callers can ``from tests._sidecar_fakes import FakeUrlopen,
FakeResp, HdrShim``.

Every call record on a ``FakeUrlopen`` instance has the **union**
of fields any inlined variant ever populated:

* ``url`` — ``req.full_url``
* ``method`` — ``req.get_method()``
* ``headers`` — dict of lower-cased header name → value
* ``body_raw`` — utf-8 decoded request body (``str | None``)
* ``body`` — JSON-decoded body when parseable (``dict | list | None``)
* ``params`` — ``parse_qsl``-decoded form params (``dict`` always)
* ``timeout`` — the ``timeout=`` kwarg passed to ``urlopen``

so each adapter's existing assertions keep working regardless of
which field name they happen to read.
"""
from __future__ import annotations

import io
import json
import urllib.error
import urllib.parse
from typing import Any, Iterable, Optional


class HdrShim:
    """Tiny stand-in for ``email.message.Message``.

    Sidecar code only ever reads ``items()`` off the response
    headers (it lower-cases the keys itself). We don't need to
    emulate the full message API.
    """

    def __init__(self, hdrs: Optional[dict] = None) -> None:
        self._hdrs = hdrs or {}

    def items(self):
        return list(self._hdrs.items())

    def get(self, key: str, default: Any = None) -> Any:
        # Case-insensitive lookup to match email.message.Message and the
        # case-insensitive header handling sidecar code uses elsewhere.
        for k, v in self._hdrs.items():
            if k.lower() == key.lower():
                return v
        return default


class FakeResp:
    """Minimal stand-in for ``http.client.HTTPResponse``.

    Supports the context-manager protocol so it can be used in
    ``with urllib.request.urlopen(...) as resp:`` blocks.
    """

    def __init__(
        self,
        status: int,
        body: bytes = b"",
        headers: Optional[HdrShim] = None,
    ) -> None:
        self.status = status
        self._body = body
        self.headers = headers if headers is not None else HdrShim({})

    def read(self, size: Optional[int] = None) -> bytes:
        if size is None or size < 0:
            return self._body
        return self._body[:size]

    def __enter__(self) -> "FakeResp":
        return self

    def __exit__(self, *_exc) -> bool:
        return False


class FakeUrlopen:
    """Drop-in replacement for ``urllib.request.urlopen`` driven by a
    pre-baked script of ``(status, body[, headers])`` tuples.

    The script is consumed in order; an extra call past the end of
    the script raises ``AssertionError`` so the test fails loudly
    instead of silently mocking infinity. A status >= 400 surfaces
    as ``urllib.error.HTTPError`` (with ``headers`` and ``body``
    accessible the same way ``urllib`` exposes them).

    Each call appends a dict to ``self.calls`` carrying every field
    any pre-extraction inlined variant ever populated — see this
    module's docstring for the full list.
    """

    def __init__(self, script: Iterable[tuple]) -> None:
        self.script: list[tuple] = list(script)
        self.calls: list[dict] = []

    def __call__(self, req, timeout=None):
        body_bytes = req.data
        # str-decoded body (most common shape)
        try:
            body_raw: Optional[str] = (
                body_bytes.decode("utf-8") if body_bytes else None
            )
        except Exception:  # noqa: BLE001
            body_raw = None
        # JSON-parsed body (None on failure)
        body_json: Any = None
        if body_raw:
            try:
                body_json = json.loads(body_raw)
            except (ValueError, TypeError):
                body_json = None
        # Form-encoded params (empty on failure)
        params: dict = {}
        if body_raw:
            try:
                params = dict(urllib.parse.parse_qsl(body_raw))
            except Exception:  # noqa: BLE001
                params = {}

        self.calls.append({
            "url": req.full_url,
            "method": req.get_method(),
            "headers": {k.lower(): v for k, v in req.header_items()},
            "body_raw": body_raw,
            "body": body_json,
            "params": params,
            "timeout": timeout,
        })

        if not self.script:
            raise AssertionError(
                f"unexpected extra urlopen call to {req.full_url}"
            )
        entry = self.script.pop(0)
        if len(entry) == 3:
            status, body, resp_hdrs = entry
        else:
            status, body = entry
            resp_hdrs = {}
        if status >= 400:
            raise urllib.error.HTTPError(
                req.full_url, status, "Error", HdrShim(resp_hdrs),
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
        return FakeResp(status, payload, HdrShim(resp_hdrs))


# Back-compat aliases — keep the underscore-prefixed names callable
# so that adapter tests that import the symbol under its historical
# private name (and any in-flight branches that haven't been
# rebased onto this refactor yet) continue to work without edits.
_HdrShim = HdrShim
_FakeResp = FakeResp
_FakeUrlopen = FakeUrlopen
