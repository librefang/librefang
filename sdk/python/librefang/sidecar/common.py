"""Shared helpers for ``librefang.sidecar.adapters.*``.

Every reference sidecar adapter historically inlined a near-identical
copy of:

* ``_split_message`` — newline-preferring chunker for outbound text;
  14 byte-identical copies (only the parameter name drifted between
  ``max_len`` / ``limit``).
* ``_split_csv`` — comma-separated env-var → cleaned list;
  3 hash groups across 7 sidecars, all behaviourally identical.
* ``_parse_retry_after`` — ``Retry-After`` header parser used by
  every 429-aware adapter; 6 hash groups across 7 sidecars. The only
  meaningful drift is the lower clamp (discord at 0.1 s, every other
  adapter at 1.0 s).

This module is the single source of truth. Adapters should
``from librefang.sidecar.common import split_message, split_csv,
parse_retry_after`` rather than re-implementing.

Hash audit at extraction time:
- ``_split_message``: 14 files, all behaviour-identical
- ``_split_csv``: 7 files, all behaviour-identical
- ``_parse_retry_after``: 7 files, identical except for floor
  (parameterised via ``floor_secs``)
"""
from __future__ import annotations

from typing import Mapping, Optional


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
    max_secs: float = 60.0,
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
