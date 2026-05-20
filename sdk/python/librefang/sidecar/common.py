"""Shared helpers for ``librefang.sidecar.adapters.*``.

Every reference sidecar adapter historically inlined a near-identical
copy of these helpers. This module is the single source of truth.

Pre-extraction hash audit (round 1 — landed in the same PR):

* ``_split_message`` — 14 files, all behaviour-identical
* ``_split_csv`` — 7 files, all behaviour-identical
* ``_parse_retry_after`` — 7 files, identical except for the lower
  clamp (parameterised via ``floor_secs``)

Pre-extraction hash audit (round 2 — this commit):

* ``_mark_seen`` / dedupe state triple — 10 files, all
  behaviour-identical
* ``_http`` ``urlopen`` wrapper — 11 files, 2 functional shapes:
  the 4-tuple version returning ``(status, parsed, raw, headers)``
  used by 10 adapters (mattermost/qq/signal/webex/zulip/line/
  nextcloud/reddit/rocketchat/discord), and slack's 3-tuple
  variant that throws away response headers. Slack migrates to
  the 4-tuple form (the extra dict is harmless)

The constants ``MAX_BACKOFF_SECS = 60.0`` and
``RETRY_AFTER_DEFAULT_SECS = 30.0`` are also hoisted here. Every
adapter that imports them gets the canonical values; the few
adapters that need a different cap pass their own value to
:func:`parse_retry_after` via ``max_secs=`` (and define their own
local constant).
"""
from __future__ import annotations

import json
import threading
import urllib.error
import urllib.request
from typing import Any, Mapping, Optional


# Canonical reconnect/backoff ceiling. 16/16 sidecars used this same
# value; hoisted so it lives in one place.
MAX_BACKOFF_SECS = 60.0

# Default ``Retry-After`` fallback when the server returns 429 with
# no parseable header. 11/13 retry-aware sidecars use this value.
RETRY_AFTER_DEFAULT_SECS = 30.0


def split_message(text: str, limit: int) -> list[str]:
    """Chunk ``text`` into <= ``limit`` pieces, preferring newline
    splits. Mirrors the shared Rust ``split_message`` helper in
    ``librefang-channels::types``.

    Splitting rule:

    * If ``text`` already fits, return ``[text]`` unchanged.
    * Otherwise scan a ``limit``-wide window for the rightmost
      newline; if found, split there (so messages break on a
      semantic boundary). If no newline is in the window, hard-cut
      at ``limit``.
    * The leftover after a newline-cut has its leading ``\\n``
      stripped so the next chunk doesn't start with a blank line.
    """
    if len(text) <= limit:
        return [text]
    chunks: list[str] = []
    rest = text
    while len(rest) > limit:
        window = rest[:limit]
        cut = window.rfind("\n")
        if cut <= 0:
            cut = limit
        chunks.append(rest[:cut])
        rest = rest[cut:].lstrip("\n") if cut < limit else rest[cut:]
    if rest:
        chunks.append(rest)
    return chunks


def split_csv(raw: str) -> list[str]:
    """Comma-separated env-var → cleaned list of strings.

    Empty input → empty list. Each item is whitespace-stripped;
    empty entries (e.g. trailing comma) are dropped. Order
    preserved.
    """
    if not raw:
        return []
    return [s.strip() for s in raw.split(",") if s.strip()]


def parse_retry_after(
    resp_hdrs: Mapping[str, str],
    *,
    default_secs: float,
    floor_secs: float = 1.0,
    max_secs: float = MAX_BACKOFF_SECS,
) -> float:
    """``Retry-After`` header parser used by every 429-aware sidecar.

    Looks up ``retry-after`` (case-insensitive — callers are
    expected to have already lower-cased their header dict; this is
    the existing convention across the sidecar HTTP helpers).

    Returns:

    * ``default_secs`` when the header is missing or not parseable
      as a float (RFC 7231 also allows an HTTP-date form; in
      practice the sidecar contract is "seconds-as-number" and any
      adapter caller that needs the date form must parse it
      themselves before calling us).
    * Otherwise the parsed value clamped to
      ``[floor_secs, max_secs]``.

    The floor exists so a server bug returning ``0`` can't pin the
    retry loop into a hot spin; discord overrides ``floor_secs=0.1``
    because its rate limiter operates at sub-second granularity, all
    other adapters keep the 1.0 default.
    """
    raw: Optional[str] = resp_hdrs.get("retry-after")
    if not raw:
        return default_secs
    try:
        v = float(raw)
    except (TypeError, ValueError):
        return default_secs
    return min(max(v, floor_secs), max_secs)


# ---- bounded inbound dedupe ----------------------------------------


# Default capacity for ``SeenSet`` — matches the 10/10 sidecars that
# used the inlined ``SEEN_MESSAGES_MAX = 10_000`` / ``EVICT = 5_000``
# pair.
DEFAULT_SEEN_MAX = 10_000
DEFAULT_SEEN_EVICT = 5_000


class SeenSet:
    """Bounded thread-safe LRU-ish set of seen message IDs.

    Used by every sidecar that dedupes inbound platform messages
    (line / mattermost / nextcloud / qq / reddit / rocketchat /
    signal / twitch / webex / zulip — 10 adapters). Note that
    bluesky's ``_mark_seen`` is **not** a SeenSet client — it's a
    server-side ``updateSeen`` REST POST that happens to share the
    name; see ``bluesky.py`` for the actual function.

    Behaviour:

    * :meth:`mark` returns ``True`` iff the id is freshly seen
      (i.e. the caller should emit the event).
    * ``None`` or empty-string ids are always treated as fresh —
      they don't participate in dedupe. The one exception is
      webex, which historically returned ``False`` (drop) on
      empty; webex's ``_mark_seen`` shim adds that check inline
      before delegating here.
    * Once the set crosses ``max_size`` entries, the oldest
      ``evict`` ids are dropped in one batch (so the cleanup is
      amortised, not per-call).

    Adapter classes expose this instance as ``self._seen``; tests
    that need to inspect the internal state read
    ``adapter._seen.ids`` (the ``set``) or ``adapter._seen.order``
    (the insertion-ordered ``list``) directly. There are no
    @property shims on the adapter classes.
    """

    def __init__(
        self,
        *,
        max_size: int = DEFAULT_SEEN_MAX,
        evict: int = DEFAULT_SEEN_EVICT,
    ) -> None:
        self.max_size = max_size
        self.evict = evict
        self.ids: set = set()
        self.order: list = []
        self._lock = threading.Lock()

    def mark(self, id_: Any) -> bool:
        """Returns ``True`` iff the id is newly seen and should be
        emitted; ``False`` if it has already been seen and should
        be suppressed."""
        if not id_:
            return True
        with self._lock:
            if id_ in self.ids:
                return False
            self.ids.add(id_)
            self.order.append(id_)
            if len(self.order) > self.max_size:
                drop = self.order[:self.evict]
                self.order = self.order[self.evict:]
                for k in drop:
                    self.ids.discard(k)
            return True

    def __contains__(self, id_: Any) -> bool:
        return id_ in self.ids

    def __len__(self) -> int:
        return len(self.ids)


# ---- HTTP request wrapper ------------------------------------------


def http_request(
    url: str,
    *,
    method: str = "GET",
    body: Optional[bytes] = None,
    headers: Optional[dict] = None,
    timeout: float = 15.0,
) -> tuple[int, Any, bytes, dict]:
    """One-shot HTTP request, the shape every sidecar's ``_http``
    helper used to be.

    Returns ``(status, parsed_json_or_None, raw_bytes,
    response_headers)``. Response headers are lower-cased so 429
    ``Retry-After`` lookups (and every other header probe) are
    case-insensitive.

    On a non-2xx the function catches :class:`urllib.error.HTTPError`
    and surfaces it as the same 4-tuple (so the caller's status
    handling stays uniform) — adapters that want to raise on 4xx/5xx
    should check ``status`` and raise themselves.

    A connection failure / DNS error / socket timeout still
    propagates up (those are bugs, not protocol-level signals).
    """
    req = urllib.request.Request(
        url, data=body, headers=headers or {}, method=method,
    )
    resp_headers: dict = {}
    raw: bytes
    try:
        with urllib.request.urlopen(  # noqa: S310 — configured URL
            req, timeout=timeout,
        ) as resp:
            status = getattr(resp, "status", 200)
            raw = resp.read()
            if resp.headers is not None:
                resp_headers = {
                    k.lower(): v for k, v in resp.headers.items()
                }
    except urllib.error.HTTPError as e:
        status = e.code
        try:
            raw = e.read()
        except Exception:  # noqa: BLE001
            raw = b""
        if e.headers is not None:
            resp_headers = {k.lower(): v for k, v in e.headers.items()}
    if not raw:
        return status, None, b"", resp_headers
    try:
        return status, json.loads(raw.decode("utf-8")), raw, resp_headers
    except (ValueError, TypeError, UnicodeDecodeError):
        return status, None, raw, resp_headers
