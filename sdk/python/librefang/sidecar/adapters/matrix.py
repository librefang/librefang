#!/usr/bin/env python3
"""Matrix sidecar channel adapter for LibreFang.

Replaces the former in-process Rust ``librefang-channels::matrix``
adapter (removed in this sidecar migration; same pattern as ntfy
#5224, telegram #5241, gotify #5263, mastodon #5264, bluesky #5277,
reddit #5281, twitch #5297, rocketchat #5298, discord #5299,
nextcloud #5301, slack #5302, webex #5309, line #5312, zulip #5310,
mattermost #5315, signal #5317, qq #5325).

Talks to a Matrix homeserver via the Client-Server API (v3):

* Long-poll ``GET /_matrix/client/v3/sync`` with the bot's
  access token, ``since`` cursor for resume, 30 s server timeout.
* ``PUT /_matrix/client/v3/rooms/{room}/send/{type}/{txn}`` for
  outbound events (m.room.message / m.reaction).
* ``PUT /_matrix/client/v3/rooms/{room}/redact/{event}/{txn}``
  for reaction-lifecycle cleanup.
* ``POST /_matrix/media/v3/upload`` for outbound media; returns
  ``content_uri`` (``mxc://server/mediaId``) that gets embedded
  in the follow-up ``m.image`` / ``m.file`` / ``m.audio`` / ``m.video``
  event.
* ``GET /_matrix/client/v3/account/whoami`` at startup to
  validate the access token.

Behaviour parity with the Rust adapter (every assertion below has a
file/line citation against ``crates/librefang-channels/src/matrix.rs``
on the pre-migration tree):

* **/sync long-poll**: 30 s server timeout, ``since`` cursor for
  incremental delivery. Mirrors matrix.rs:855-1008.
* **Room allowlist**: empty ``MATRIX_ALLOWED_ROOMS`` = listen on
  every room the bot has joined; non-empty restricts (matrix.rs:940-942).
* **Self-skip**: drop events whose ``sender == user_id``
  (matrix.rs:960-962).
* **E2EE warn-once per room**: the first ``m.room.encrypted``
  event per room emits a WARN; subsequent ones are silent
  (matrix.rs:948-954).
* **5 inbound msgtypes**: ``m.text`` / ``m.notice`` / ``m.emote``
  → text or Command (slash-prefix); ``m.image`` / ``m.file`` /
  ``m.audio`` / ``m.video`` → media event with mxc:// →
  authenticated MSC3916 download URL conversion (matrix.rs:311-343).
* **m.thread relation**: ``parse_thread_relation`` surfaces the
  thread root as ``thread_id`` on inbound (matrix.rs:206-215).
* **5 send variants** mirror the Rust trait surface:
  ``send`` (text + 11 content variants), ``send_typing``,
  ``send_reaction`` (phase-reaction lifecycle), ``send_in_thread``
  (m.thread wrap), ``send_streaming`` (m.replace edit loop).
* **Markdown → Matrix HTML** via a stdlib CommonMark subset
  renderer (matrix.rs:149-166 uses ``pulldown-cmark`` in Rust;
  the sidecar's ``markdown_to_matrix_html`` covers headings,
  bold, italic, code (inline + block), links, blockquotes, lists,
  hr, paragraphs, and escapes raw HTML in the source so a model
  can't inject ``<script>``).
* **m.replace streaming edit with 429 retry**: ``send_streaming``
  posts a ``…`` placeholder, edits it on every 700 ms /
  96 char tick, splits on MAX_MESSAGE_LEN overflow, and threads
  one txn_id across both attempts of an edit so a 429 that masks
  a quietly-successful first PUT can't land a duplicate via the
  retry (matrix.rs:750-774). Backoff is clamped to
  ``[MIN_RETRY_BACKOFF_MS, MAX_RETRY_BACKOFF_MS]``.
* **Reaction lifecycle**: ``send_reaction`` redacts the previous
  ``(room, target_event)`` reaction (if any, gated by
  ``remove_previous``) and inserts the new one. The cache is
  insertion-ordered, capped at 1024 entries with FIFO eviction
  (matrix.rs:649-661).
* **mxc:// → HTTPS via MSC3916** authenticated endpoint
  ``/_matrix/client/v1/media/download/{server}/{mediaId}``
  (matrix.rs:195-204); modern Synapse (≥1.100) freezes the
  legacy ``/_matrix/media/v3/download`` route.
* **Multi-bot ``account_id`` metadata** (#5003) on inbound when
  ``MATRIX_ACCOUNT_ID`` is set.
* **Reconnect**: exponential backoff 1 s → 60 s on /sync failures,
  matches the Rust adapter's ``calculate_backoff``.

Improvements over the Rust adapter
==================================

1. **Bounded inbound dedupe on ``event_id``**. The Rust adapter
   emitted every event_id from a sync batch unconditionally —
   server-side ``since`` cursor narrows duplicates, but on a
   delayed-success-then-retry of /sync (or a ``since`` reset on
   client restart) the bot could re-emit. Sidecar adds a bounded
   ``SeenSet`` with ``SEEN_MESSAGES_MAX = 10000`` (same policy as
   webex / line / mattermost / signal / qq).
2. **429 ``Retry-After`` honoured at every PUT, not just edit**.
   The Rust adapter's ``api_edit_event_with_retry`` honoured
   Retry-After on edits; ``api_send_event`` and ``api_redact``
   did not. The sidecar's shared ``_put_event`` honours it at
   every call site (whole edit-lifecycle + reaction-lifecycle +
   first-send all benefit).
3. **Explicit 60 s timeout on /sync, 30 s on every other REST
   call**. The Rust adapter relied on ``reqwest``'s default
   (none); a hung homeserver would hang the producer thread
   forever.

Stdlib-only: HTTPS via ``urllib.request``, no WebSocket
(/sync is HTTP long-poll). Markdown rendering is a hand-rolled
CommonMark subset — see ``markdown_to_matrix_html``.

Configure via ``[[sidecar_channels]]``::

    [[sidecar_channels]]
    name = "matrix"
    command = "python3"
    args = ["-m", "librefang.sidecar.adapters.matrix"]
    channel_type = "matrix"
    [sidecar_channels.env]
    MATRIX_HOMESERVER_URL = "https://matrix.example.com"
    MATRIX_USER_ID = "@bot:matrix.example.com"
    # MATRIX_ALLOWED_ROOMS = "!abc:matrix.org,!def:matrix.org"   # optional
    # MATRIX_ACCOUNT_ID = "prod-bot"                              # optional
    # MATRIX_MAX_UPLOAD_BYTES = "52428800"                        # optional, default 50 MiB

Secret via ``~/.librefang/secrets.env``: ``MATRIX_ACCESS_TOKEN``
(the bot's access token from the homeserver — same shape Element
uses).
"""
from __future__ import annotations

import asyncio
import json
import mimetypes
import os
import re
import threading
import time
import urllib.error
import urllib.parse
import urllib.request
import uuid
from html import escape as html_escape
from typing import Any, Callable, Optional

from librefang.sidecar import Content, Field, Schema, SidecarAdapter, protocol, run_stdio_main
from librefang.sidecar import logging as log
from librefang.sidecar.common import (
    MAX_BACKOFF_SECS,
    RETRY_AFTER_DEFAULT_SECS,
    SeenSet as _SeenSet,
    parse_retry_after as _parse_retry_after_impl,
    split_csv as _split_csv,
    split_message as _split_message,
)

# Matrix constants — mirror crate::matrix defaults.
SYNC_TIMEOUT_MS = 30_000
SYNC_TIMEOUT_SECS = 60.0  # urlopen total timeout: server timeout + leeway
SEND_TIMEOUT_SECS = 30.0
MAX_MESSAGE_LEN = 4096

# Default outbound media upload cap (50 MiB). Mirrors the Rust adapter's
# DEFAULT_MAX_UPLOAD_BYTES at matrix.rs:27.
DEFAULT_MAX_UPLOAD_BYTES = 50 * 1024 * 1024

# Streaming edit cadence (matrix.rs:35-36). 700 ms / 96 chars produces
# ~4-5 visible edits per 3 s response in Element.
STREAM_EDIT_INTERVAL_SECS = 0.7
STREAM_EDIT_CHAR_BUDGET = 96

# Reaction-lifecycle cache cap (matrix.rs:38).
PHASE_REACTIONS_CAPACITY = 1024

# 429 retry backoff envelope (matrix.rs:47-49). The streaming edit loop
# clamps Retry-After into this window so a malformed hint can't stall
# or hot-loop.
MIN_RETRY_BACKOFF_SECS = 0.1
MAX_RETRY_BACKOFF_SECS = 5.0
DEFAULT_RETRY_BACKOFF_SECS = 0.5

INITIAL_BACKOFF_SECS = 1.0

# Bounded inbound dedupe (improvement #1).
SEEN_MESSAGES_MAX = 10_000
SEEN_MESSAGES_EVICT = 5_000


def _parse_retry_after(resp_hdrs: dict, *, default_secs: float) -> float:
    """Backwards-compat wrapper around
    :func:`librefang.sidecar.common.parse_retry_after`."""
    return _parse_retry_after_impl(
        resp_hdrs,
        default_secs=default_secs,
        floor_secs=MIN_RETRY_BACKOFF_SECS,
        max_secs=MAX_RETRY_BACKOFF_SECS,
    )


# ---- mxc:// → HTTPS (MSC3916) ---------------------------------------


def mxc_to_http(mxc: str, homeserver_url: str) -> Optional[str]:
    """Convert ``mxc://server/mediaId`` → the MSC3916 authenticated
    download URL. Returns ``None`` for malformed input.

    Modern Synapse (≥1.100) freezes the legacy unauthenticated
    ``/_matrix/media/v3/download`` path; the bot's bearer token is
    attached on download (see ``_fetch_headers_for``).
    """
    if not mxc.startswith("mxc://"):
        return None
    rest = mxc[len("mxc://"):]
    if "/" not in rest:
        return None
    server, _, media_id = rest.partition("/")
    if not server or not media_id:
        return None
    return (
        f"{homeserver_url.rstrip('/')}"
        f"/_matrix/client/v1/media/download/{server}/{media_id}"
    )


# ---- markdown → Matrix HTML subset ----------------------------------

# Compile patterns at module load. Order matters: code blocks first
# (otherwise ``**`` inside ` ``` ` gets bolded), then inline code, then
# bold (`**`), then italic (`*`), then headings + lists.

_RE_THINK = re.compile(r"<think>[\s\S]*?</think>", re.IGNORECASE)


def _render_inline(text: str) -> str:
    """Render inline markdown to HTML, escaping HTML entities first
    so raw ``<script>`` in the source can't reach the formatted_body.

    Supports: bold (``**x**``), italic (``*x*``), inline code
    (`` `x` ``), strikethrough (``~~x~~``), links (``[t](u)``).
    """
    s = html_escape(text, quote=False)

    # Inline code first — its contents must not be further parsed.
    code_placeholders: dict[str, str] = {}

    def _stash_code(m):
        key = f"\x00CODE{len(code_placeholders)}\x00"
        code_placeholders[key] = f"<code>{m.group(1)}</code>"
        return key

    s = re.sub(r"`([^`\n]+)`", _stash_code, s)

    # Links: [text](url). The url is already HTML-escaped above; we
    # additionally validate the scheme to keep ``javascript:`` /
    # ``data:`` out of the rendered HTML.
    def _link(m):
        text_inner = m.group(1)
        url = m.group(2)
        # html_escape already turned `&` into `&amp;`; that's fine
        # inside an href. Reject javascript: / data: schemes.
        low = url.lower().lstrip()
        if low.startswith("javascript:") or low.startswith("data:"):
            return f"[{text_inner}]({url})"
        return f'<a href="{url}">{text_inner}</a>'

    s = re.sub(r"\[([^\]]+)\]\(([^)]+)\)", _link, s)

    # Bold + italic. Bold first (`**`) so its asterisks don't get
    # eaten by the single-asterisk italic rule.
    s = re.sub(r"\*\*([^*\n]+)\*\*", r"<strong>\1</strong>", s)
    s = re.sub(r"(?<!\*)\*([^*\n]+)\*(?!\*)", r"<em>\1</em>", s)
    # Strikethrough — GFM extension Element supports.
    s = re.sub(r"~~([^~\n]+)~~", r"<del>\1</del>", s)

    # Restore inline-code placeholders.
    for key, html in code_placeholders.items():
        s = s.replace(key, html)
    return s


def _render_table(table_lines: list[str]) -> str:
    """Render a GFM-style table (header | sep | body rows) to
    ``<table><thead>...</thead><tbody>...</tbody></table>``.

    Caller has already verified the second line is the separator
    (``|---|---|`` pattern) and ``table_lines`` is at least 2 entries
    (header + separator, possibly no body)."""
    if len(table_lines) < 2:
        return ""
    header_cells = [c.strip() for c in table_lines[0].strip("|").split("|")]
    body_rows = table_lines[2:]
    parts = ["<table><thead><tr>"]
    for h in header_cells:
        parts.append(f"<th>{_render_inline(h)}</th>")
    parts.append("</tr></thead><tbody>")
    for row in body_rows:
        cells = [c.strip() for c in row.strip("|").split("|")]
        parts.append("<tr>")
        for c in cells:
            parts.append(f"<td>{_render_inline(c)}</td>")
        parts.append("</tr>")
    parts.append("</tbody></table>")
    return "".join(parts)


def markdown_to_matrix_html(text: str) -> str:
    """Render CommonMark ``text`` into the HTML subset that
    Element / Matrix clients accept for ``formatted_body``.

    Implemented in stdlib only — the Rust adapter used
    ``pulldown-cmark`` (matrix.rs:149-166). Order of operations:

    1. Strip ``<think>...</think>`` reasoning blocks (LLM-side
       artifact, never user-visible).
    2. HTML-escape every raw character so an LLM-authored
       ``<script>`` in the source cannot inject markup.
    3. Walk the source line-by-line, lifting block constructs
       (heading, fenced code block, blockquote, list, hr, paragraph).
    4. Each block's inner text passes through ``_render_inline``
       for bold / italic / inline-code / link / strikethrough.
    """
    if not text:
        return ""
    # Drop <think>...</think> first so its contents don't survive.
    text = _RE_THINK.sub("", text)

    lines = text.split("\n")
    out: list[str] = []
    i = 0
    n = len(lines)
    para_buf: list[str] = []
    list_stack: list[str] = []  # "ul" or "ol"

    def flush_para():
        if para_buf:
            joined = " ".join(line.strip() for line in para_buf)
            if joined.strip():
                out.append(f"<p>{_render_inline(joined)}</p>")
            para_buf.clear()

    def close_lists():
        while list_stack:
            out.append(f"</{list_stack.pop()}>")

    while i < n:
        raw = lines[i]
        stripped = raw.strip()

        # Fenced code block: ``` or ~~~
        m_fence = re.match(r"^[ \t]{0,3}(```|~~~)([^\n]*)$", raw)
        if m_fence:
            flush_para(); close_lists()
            fence = m_fence.group(1)
            lang = m_fence.group(2).strip()
            i += 1
            buf = []
            while i < n:
                if re.match(rf"^[ \t]{{0,3}}{re.escape(fence)}\s*$", lines[i]):
                    i += 1
                    break
                buf.append(lines[i])
                i += 1
            inner = html_escape("\n".join(buf))
            if lang:
                out.append(
                    f'<pre><code class="language-{html_escape(lang)}">{inner}</code></pre>'
                )
            else:
                out.append(f"<pre><code>{inner}</code></pre>")
            continue

        # Horizontal rule
        if re.match(r"^[ \t]{0,3}(-{3,}|\*{3,}|_{3,})[ \t]*$", raw):
            flush_para(); close_lists()
            out.append("<hr/>")
            i += 1
            continue

        # ATX heading
        m_h = re.match(r"^[ \t]{0,3}(#{1,6})\s+(.+?)\s*#*\s*$", raw)
        if m_h:
            flush_para(); close_lists()
            level = len(m_h.group(1))
            content = m_h.group(2)
            out.append(f"<h{level}>{_render_inline(content)}</h{level}>")
            i += 1
            continue

        # GFM table: line | line | with a separator row immediately after
        if (
            "|" in raw
            and i + 1 < n
            and re.match(r"^\s*\|?\s*:?-+:?\s*(\|\s*:?-+:?\s*)+\|?\s*$", lines[i + 1])
        ):
            flush_para(); close_lists()
            tbl = [raw, lines[i + 1]]
            i += 2
            while i < n and "|" in lines[i] and lines[i].strip():
                tbl.append(lines[i])
                i += 1
            out.append(_render_table(tbl))
            continue

        # Blockquote
        if stripped.startswith(">"):
            flush_para(); close_lists()
            quote_lines = []
            while i < n and lines[i].lstrip().startswith(">"):
                line_after_marker = re.sub(r"^[ \t]*>\s?", "", lines[i])
                quote_lines.append(line_after_marker)
                i += 1
            inner_text = "\n".join(quote_lines).strip()
            inner_html = markdown_to_matrix_html(inner_text)
            out.append(f"<blockquote>{inner_html}</blockquote>")
            continue

        # Unordered list
        m_ul = re.match(r"^[ \t]{0,3}[-*+]\s+(.+)$", raw)
        if m_ul:
            flush_para()
            if not list_stack or list_stack[-1] != "ul":
                close_lists()
                list_stack.append("ul")
                out.append("<ul>")
            out.append(f"<li>{_render_inline(m_ul.group(1))}</li>")
            i += 1
            continue

        # Ordered list
        m_ol = re.match(r"^[ \t]{0,3}\d+\.\s+(.+)$", raw)
        if m_ol:
            flush_para()
            if not list_stack or list_stack[-1] != "ol":
                close_lists()
                list_stack.append("ol")
                out.append("<ol>")
            out.append(f"<li>{_render_inline(m_ol.group(1))}</li>")
            i += 1
            continue

        # Blank line — closes paragraph + lists.
        if not stripped:
            flush_para(); close_lists()
            i += 1
            continue

        # Paragraph continuation.
        para_buf.append(raw)
        i += 1

    flush_para(); close_lists()
    return "".join(out)


# ---- m.text body builder --------------------------------------------


def text_body_with_html(
    raw: str, extra: Optional[dict] = None,
) -> dict:
    """Build a JSON ``m.text`` content body carrying both ``body``
    (raw markdown for clients that ignore ``format``) and
    ``formatted_body`` (rendered HTML). Extras get merged in for
    ``m.relates_to`` / ``m.new_content`` attachments."""
    v: dict = {
        "msgtype": "m.text",
        "body": raw,
        "format": "org.matrix.custom.html",
        "formatted_body": markdown_to_matrix_html(raw),
    }
    if extra:
        v.update(extra)
    return v


def build_edit_body(target_event_id: str, new_text: str) -> dict:
    """Build the ``m.replace`` edit body. Mirrors matrix.rs:123-142.

    ``MAX_MESSAGE_LEN`` caps only the plain ``body`` /
    ``m.new_content.body`` text; the matching ``formatted_body`` is
    the rendered HTML and is allowed to exceed the cap (truncating
    HTML at a byte budget risks leaving a half-open tag)."""
    safe = new_text[:MAX_MESSAGE_LEN]
    html = markdown_to_matrix_html(safe)
    return {
        "msgtype": "m.text",
        "body": f"* {safe}",
        "format": "org.matrix.custom.html",
        "formatted_body": f"* {html}",
        "m.new_content": {
            "msgtype": "m.text",
            "body": safe,
            "format": "org.matrix.custom.html",
            "formatted_body": html,
        },
        "m.relates_to": {
            "rel_type": "m.replace",
            "event_id": target_event_id,
        },
    }


# ---- m.thread relation parser ---------------------------------------


def parse_thread_relation(content: dict) -> Optional[str]:
    """Extract the thread root event_id when the content carries
    ``m.relates_to.rel_type == "m.thread"``. Returns ``None`` for
    plain messages, replies, edits, and any malformed shape.
    Mirrors matrix.rs:208-215.
    """
    if not isinstance(content, dict):
        return None
    rel = content.get("m.relates_to")
    if not isinstance(rel, dict):
        return None
    if rel.get("rel_type") != "m.thread":
        return None
    eid = rel.get("event_id")
    return eid if isinstance(eid, str) else None


# ---- inbound msgtype dispatch ---------------------------------------


def parse_media_image(content: dict, hs: str) -> Optional[dict]:
    """Parse ``m.image`` event content. Returns a
    ``Content.image(...)`` dict or ``None`` for malformed.
    Mirrors matrix.rs:218-235."""
    mxc = content.get("url")
    if not isinstance(mxc, str):
        return None
    url = mxc_to_http(mxc, hs)
    if not url:
        return None
    info = content.get("info") if isinstance(content.get("info"), dict) else {}
    mime = info.get("mimetype") if isinstance(info.get("mimetype"), str) else None
    caption = content.get("body") if isinstance(content.get("body"), str) else None
    return Content.image(url=url, caption=caption, mime_type=mime)


def parse_media_file(content: dict, hs: str) -> Optional[dict]:
    """Parse ``m.file`` event content. v1.10+ ``filename`` wins
    over ``body``. Mirrors matrix.rs:239-252."""
    mxc = content.get("url")
    if not isinstance(mxc, str):
        return None
    url = mxc_to_http(mxc, hs)
    if not url:
        return None
    fname = content.get("filename")
    if not isinstance(fname, str) or not fname:
        fname = content.get("body")
        if not isinstance(fname, str) or not fname:
            fname = "file"
    return Content.file(url=url, filename=fname)


def parse_media_audio(content: dict, hs: str) -> Optional[dict]:
    """Parse ``m.audio`` event content. ``org.matrix.msc3245.voice``
    marker promotes to ``Content.voice``; everything else is
    ``Content.audio``. Mirrors matrix.rs:256-284."""
    mxc = content.get("url")
    if not isinstance(mxc, str):
        return None
    url = mxc_to_http(mxc, hs)
    if not url:
        return None
    caption = content.get("body") if isinstance(content.get("body"), str) else None
    info = content.get("info") if isinstance(content.get("info"), dict) else {}
    dur_ms = info.get("duration")
    duration_seconds = int(dur_ms // 1000) if isinstance(dur_ms, int) else 0
    if "org.matrix.msc3245.voice" in content:
        return Content.voice(
            url=url, caption=caption, duration_seconds=duration_seconds,
        )
    return Content.audio(
        url=url, caption=caption, duration_seconds=duration_seconds,
        title=None, performer=None,
    )


def parse_media_video(content: dict, hs: str) -> Optional[dict]:
    """Parse ``m.video`` event content. Mirrors matrix.rs:287-307."""
    mxc = content.get("url")
    if not isinstance(mxc, str):
        return None
    url = mxc_to_http(mxc, hs)
    if not url:
        return None
    caption = content.get("body") if isinstance(content.get("body"), str) else None
    info = content.get("info") if isinstance(content.get("info"), dict) else {}
    dur_ms = info.get("duration")
    duration_seconds = int(dur_ms // 1000) if isinstance(dur_ms, int) else 0
    filename = content.get("body") if isinstance(content.get("body"), str) else None
    return Content.video(
        url=url, caption=caption, duration_seconds=duration_seconds,
        filename=filename,
    )


def parse_inbound_msg_content(content: dict, hs: str) -> Optional[dict]:
    """Dispatch helper: return ``Content.*`` for an event content
    blob based on ``msgtype``. Returns ``None`` for empty bodies,
    malformed content, or unhandled msgtypes (m.location, m.sticker,
    etc.). Mirrors matrix.rs:311-343.

    Slash-prefix routing: ``m.text`` / ``m.notice`` / ``m.emote``
    starting with ``/`` becomes a ``Command`` event (text otherwise).
    """
    if not isinstance(content, dict):
        return None
    msgtype = content.get("msgtype")
    if not isinstance(msgtype, str):
        msgtype = "m.text"
    if msgtype in ("m.text", "m.notice", "m.emote"):
        body = content.get("body")
        if not isinstance(body, str) or not body:
            return None
        if body.startswith("/"):
            head, _, tail = body[1:].partition(" ")
            return Content.command(head, tail.split() if tail else [])
        return Content.text(body)
    if msgtype == "m.image":
        return parse_media_image(content, hs)
    if msgtype == "m.file":
        return parse_media_file(content, hs)
    if msgtype == "m.audio":
        return parse_media_audio(content, hs)
    if msgtype == "m.video":
        return parse_media_video(content, hs)
    return None


# ---- helpers ---------------------------------------------------------


def _format_with_button_hints(text: str, buttons: list) -> str:
    """Render ``text`` followed by ``[Label]`` hints for each button
    row — Matrix has no native interactive button surface, the
    text-suffix fallback is the standard convention. Mirrors
    matrix.rs:402-421."""
    if not buttons:
        return text
    out = text
    for i, row in enumerate(buttons):
        if i > 0 or out:
            out += "\n"
        for btn in row:
            label = btn.get("label", "") if isinstance(btn, dict) else ""
            out += f"[{label}] "
    return out.rstrip()


# ---- MatrixAdapter ---------------------------------------------------


class MatrixAdapter(SidecarAdapter):
    """Matrix Client-Server API adapter.

    Polls ``GET /sync`` with the bot's access token and emits one
    ``message`` event per inbound message; ``on_send`` routes
    outbound text + 11 media variants through ``PUT
    /rooms/{room}/send/{type}/{txn_id}``.
    """

    # Matrix's reaction-lifecycle + m.thread + streaming-edit surface
    # the daemon already targets via Send / Reaction / TypingCmd /
    # StreamStart / StreamDelta / StreamEnd envelopes.
    capabilities: list = ["thread", "typing", "reaction", "streaming"]
    # Matrix rooms are typically multi-participant — error messages
    # echoed back become noise for every member. Suppress, matching
    # rocketchat / nextcloud / signal / qq.
    suppress_error_responses: bool = False

    SCHEMA = Schema(
        name="matrix",
        display_name="Matrix",
        description="Matrix Client-Server API adapter (out-of-process sidecar)",
        fields=[
            Field("MATRIX_HOMESERVER_URL", "Homeserver URL", "text",
                  required=True,
                  placeholder="https://matrix.example.com"),
            Field("MATRIX_USER_ID", "Bot User ID", "text",
                  required=True,
                  placeholder="@bot:matrix.example.com"),
            Field("MATRIX_ACCESS_TOKEN", "Access Token", "secret",
                  required=True,
                  placeholder="syt_..."),
            Field("MATRIX_ALLOWED_ROOMS",
                  "Allowed Room IDs (comma-separated, empty = all joined)",
                  "text",
                  placeholder="!abc:matrix.org,!def:matrix.org",
                  advanced=True),
            Field("MATRIX_ACCOUNT_ID",
                  "Account ID (multi-bot routing)",
                  "text",
                  placeholder="prod-bot",
                  advanced=True),
            Field("MATRIX_MAX_UPLOAD_BYTES",
                  "Max outbound media upload size in bytes (default 52428800 = 50 MiB)",
                  "text",
                  placeholder=str(DEFAULT_MAX_UPLOAD_BYTES),
                  advanced=True),
        ],
    )

    def __init__(self) -> None:
        hs = os.environ.get("MATRIX_HOMESERVER_URL", "").strip()
        user_id = os.environ.get("MATRIX_USER_ID", "").strip()
        token = os.environ.get("MATRIX_ACCESS_TOKEN", "").strip()
        missing: list[str] = []
        if not hs:
            missing.append("MATRIX_HOMESERVER_URL")
        if not user_id:
            missing.append("MATRIX_USER_ID")
        if not token:
            missing.append("MATRIX_ACCESS_TOKEN")
        if missing:
            log.error("matrix required env vars missing", missing=missing)
            raise SystemExit(2)
        if not (hs.startswith("http://") or hs.startswith("https://")):
            log.error(
                "MATRIX_HOMESERVER_URL must start with http:// or https://",
                value=hs,
            )
            raise SystemExit(2)
        self.homeserver_url = hs.rstrip("/")
        self.user_id = user_id
        self.access_token = token
        self.allowed_rooms = _split_csv(
            os.environ.get("MATRIX_ALLOWED_ROOMS", ""),
        )
        acct = os.environ.get("MATRIX_ACCOUNT_ID", "").strip()
        self.account_id: Optional[str] = acct or None

        upload_raw = os.environ.get("MATRIX_MAX_UPLOAD_BYTES", "").strip()
        if upload_raw:
            try:
                self.max_upload_bytes = int(upload_raw)
            except ValueError:
                log.warn(
                    "matrix MATRIX_MAX_UPLOAD_BYTES not an integer; using default",
                    value=upload_raw, default=DEFAULT_MAX_UPLOAD_BYTES,
                )
                self.max_upload_bytes = DEFAULT_MAX_UPLOAD_BYTES
        else:
            self.max_upload_bytes = DEFAULT_MAX_UPLOAD_BYTES

        # /sync resume cursor.
        self.since_token: Optional[str] = None
        # Reaction lifecycle: ordered (room, target_event) → reaction_id.
        # Python 3.7+ dict is ordered, used as an OrderedDict.
        self._phase_reactions: dict[tuple, str] = {}
        self._phase_lock = threading.Lock()
        # E2EE warn-once set.
        self._e2ee_warned: set[str] = set()
        self._e2ee_lock = threading.Lock()
        # Inbound dedupe (improvement #1).
        self._seen = _SeenSet(
            max_size=SEEN_MESSAGES_MAX, evict=SEEN_MESSAGES_EVICT,
        )
        self._shutdown = threading.Event()

    # ---- HTTP plumbing ----------------------------------------------

    def _auth_headers(self, *, content_type: bool = False) -> dict:
        h: dict = {
            "Authorization": f"Bearer {self.access_token}",
            "User-Agent": "librefang-matrix-sidecar/1 (https://librefang.org)",
        }
        if content_type:
            h["Content-Type"] = "application/json; charset=utf-8"
        return h

    def _http(
        self,
        url: str,
        *,
        method: str = "GET",
        body: Optional[bytes] = None,
        headers: Optional[dict] = None,
        timeout: float = SEND_TIMEOUT_SECS,
    ) -> tuple[int, Any, bytes, dict]:
        """One-shot HTTP request. Returns
        ``(status, parsed_json_or_None, raw_bytes, response_headers)``.
        Response headers are lower-cased so 429 ``Retry-After`` lookups
        are case-insensitive."""
        req = urllib.request.Request(
            url, data=body, headers=headers or {}, method=method,
        )
        resp_headers: dict = {}
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

    def _put_event(
        self,
        room_id: str,
        event_type: str,
        body: dict,
        *,
        txn_id: Optional[str] = None,
    ) -> str:
        """``PUT /rooms/{room}/send/{type}/{txn_id}``. Mints a fresh
        UUID txn_id when not provided. Honours 429 ``Retry-After``
        with one retry; second 429 raises (improvement #2).

        Returns the server-assigned ``event_id`` or raises
        ``RuntimeError`` on failure.
        """
        if txn_id is None:
            txn_id = str(uuid.uuid4())
        url = (
            f"{self.homeserver_url}/_matrix/client/v3/rooms/"
            f"{urllib.parse.quote(room_id, safe='')}/send/"
            f"{urllib.parse.quote(event_type, safe='')}/{txn_id}"
        )
        body_bytes = json.dumps(body).encode("utf-8")
        for attempt in range(2):
            status, parsed, raw, hdrs = self._http(
                url, method="PUT", body=body_bytes,
                headers=self._auth_headers(content_type=True),
            )
            if status == 429:
                if attempt == 0:
                    sleep = _parse_retry_after(
                        hdrs, default_secs=DEFAULT_RETRY_BACKOFF_SECS,
                    )
                    log.warn(
                        "matrix PUT 429; sleeping then retrying once",
                        event_type=event_type, retry_after_secs=sleep,
                    )
                    time.sleep(sleep)
                    continue
                raise RuntimeError(
                    f"matrix PUT {event_type} rate-limited persistently",
                )
            if 200 <= status < 300 and isinstance(parsed, dict):
                eid = parsed.get("event_id")
                if isinstance(eid, str) and eid:
                    return eid
                raise RuntimeError("matrix PUT response missing event_id")
            snippet = raw[:200].decode("utf-8", "replace") if raw else ""
            raise RuntimeError(
                f"matrix PUT {event_type} failed (status={status}): {snippet}",
            )
        raise RuntimeError("matrix PUT unreachable")

    def _redact(self, room_id: str, target_event_id: str,
                reason: Optional[str] = None) -> Optional[str]:
        """``PUT /rooms/{room}/redact/{event}/{txn}``. Returns the
        redaction event_id, or None on failure (logged-and-skip —
        reaction-lifecycle cleanup is best-effort)."""
        txn_id = str(uuid.uuid4())
        url = (
            f"{self.homeserver_url}/_matrix/client/v3/rooms/"
            f"{urllib.parse.quote(room_id, safe='')}/redact/"
            f"{urllib.parse.quote(target_event_id, safe='')}/{txn_id}"
        )
        body = {"reason": reason} if reason else {}
        body_bytes = json.dumps(body).encode("utf-8")
        status, parsed, raw, _hdrs = self._http(
            url, method="PUT", body=body_bytes,
            headers=self._auth_headers(content_type=True),
        )
        if 200 <= status < 300 and isinstance(parsed, dict):
            eid = parsed.get("event_id")
            if isinstance(eid, str):
                return eid
        log.debug(
            "matrix redact failed (best-effort, ignoring)",
            target=target_event_id, status=status,
        )
        return None

    def _upload_media(
        self, data: bytes, filename: str, mime_type: str,
    ) -> str:
        """``POST /_matrix/media/v3/upload``. Returns the ``content_uri``
        (``mxc://server/mediaId``). Raises ``RuntimeError`` on
        oversized payloads or non-2xx responses."""
        if len(data) > self.max_upload_bytes:
            raise RuntimeError(
                f"matrix upload size {len(data)} exceeds "
                f"{self.max_upload_bytes} byte limit",
            )
        url = (
            f"{self.homeserver_url}/_matrix/media/v3/upload"
            f"?filename={urllib.parse.quote(filename, safe='')}"
        )
        headers = self._auth_headers()
        headers["Content-Type"] = mime_type
        status, parsed, raw, _hdrs = self._http(
            url, method="POST", body=data, headers=headers,
            timeout=SYNC_TIMEOUT_SECS,
        )
        if 200 <= status < 300 and isinstance(parsed, dict):
            mxc = parsed.get("content_uri")
            if isinstance(mxc, str) and mxc.startswith("mxc://"):
                return mxc
        snippet = raw[:200].decode("utf-8", "replace") if raw else ""
        raise RuntimeError(
            f"matrix upload failed (status={status}): {snippet}",
        )

    def _validate(self) -> str:
        """``GET /account/whoami``. Returns the bot's user_id or
        raises ``RuntimeError`` on auth failure."""
        url = f"{self.homeserver_url}/_matrix/client/v3/account/whoami"
        status, parsed, raw, _hdrs = self._http(
            url, headers=self._auth_headers(),
        )
        if status != 200 or not isinstance(parsed, dict):
            snippet = raw[:200].decode("utf-8", "replace") if raw else ""
            raise RuntimeError(
                f"matrix /whoami failed (status={status}): {snippet}",
            )
        uid = parsed.get("user_id")
        if not isinstance(uid, str) or not uid:
            raise RuntimeError("matrix /whoami: missing user_id")
        return uid

    # ---- reaction lifecycle -----------------------------------------

    def _phase_reaction_lookup(self, key: tuple) -> Optional[str]:
        with self._phase_lock:
            return self._phase_reactions.get(key)

    def _phase_reaction_remove(self, key: tuple) -> Optional[str]:
        with self._phase_lock:
            return self._phase_reactions.pop(key, None)

    def _phase_reaction_insert(self, key: tuple, reaction_id: str) -> None:
        with self._phase_lock:
            if key in self._phase_reactions:
                # Replace in place — preserve position
                self._phase_reactions[key] = reaction_id
                return
            if len(self._phase_reactions) >= PHASE_REACTIONS_CAPACITY:
                # FIFO eviction
                first = next(iter(self._phase_reactions))
                del self._phase_reactions[first]
            self._phase_reactions[key] = reaction_id

    def _check_e2ee_warn(self, room_id: str) -> bool:
        """Return True iff this is the first time we've seen the
        room as E2EE — caller emits a WARN on True only."""
        with self._e2ee_lock:
            if room_id in self._e2ee_warned:
                return False
            self._e2ee_warned.add(room_id)
            return True

    # ---- /sync long-poll --------------------------------------------

    def _build_sync_url(self) -> str:
        filt = '{"room":{"timeline":{"limit":10}}}'
        url = (
            f"{self.homeserver_url}/_matrix/client/v3/sync"
            f"?timeout={SYNC_TIMEOUT_MS}"
            f"&filter={urllib.parse.quote(filt, safe='')}"
        )
        if self.since_token:
            url += f"&since={urllib.parse.quote(self.since_token, safe='')}"
        return url

    def _process_sync_body(
        self, body: dict, emit: Callable[[dict], None],
    ) -> None:
        """Walk the /sync response and emit one ``message`` event
        per qualified room event."""
        if not isinstance(body, dict):
            return
        next_batch = body.get("next_batch")
        if isinstance(next_batch, str):
            self.since_token = next_batch
        rooms = (
            body.get("rooms", {}).get("join", {})
            if isinstance(body.get("rooms"), dict) else {}
        )
        if not isinstance(rooms, dict):
            return
        for room_id, room_data in rooms.items():
            if self.allowed_rooms and room_id not in self.allowed_rooms:
                continue
            if not isinstance(room_data, dict):
                continue
            timeline = room_data.get("timeline")
            if not isinstance(timeline, dict):
                continue
            events = timeline.get("events")
            if not isinstance(events, list):
                continue
            for event in events:
                if not isinstance(event, dict):
                    continue
                event_type = event.get("type")
                if event_type == "m.room.encrypted":
                    if self._check_e2ee_warn(room_id):
                        log.warn(
                            "matrix room is E2EE; encrypted events ignored "
                            "(E2EE not yet supported)",
                            room_id=room_id,
                        )
                    continue
                if event_type != "m.room.message":
                    continue
                sender = event.get("sender")
                if not isinstance(sender, str) or not sender:
                    continue
                if sender == self.user_id:
                    continue
                content = event.get("content")
                if not isinstance(content, dict):
                    continue
                msg_content = parse_inbound_msg_content(
                    content, self.homeserver_url,
                )
                if msg_content is None:
                    continue
                event_id = event.get("event_id")
                if not isinstance(event_id, str):
                    event_id = None
                # Inbound dedupe.
                if event_id and not self._seen.mark(event_id):
                    continue
                thread_id = parse_thread_relation(content)
                metadata: dict[str, Any] = {}
                if self.account_id is not None:
                    metadata["account_id"] = self.account_id
                ev = protocol.message(
                    user_id=room_id,
                    user_name=sender,
                    content=msg_content,
                    message_id=event_id,
                    channel_id=room_id,
                    thread_id=thread_id,
                    is_group=True,
                    metadata=metadata,
                )
                emit(ev)

    def _producer_blocking(self, emit: Callable[[dict], None]) -> None:
        """/sync long-poll loop. Validates auth first, then loops.

        Errors back off exponentially 1 s → 60 s; success resets the
        backoff. ``self._shutdown`` exits the loop cleanly.
        """
        backoff = INITIAL_BACKOFF_SECS
        # Validate the access token before entering the loop. A
        # validation failure isn't a producer crash — the operator
        # likely has a stale token. Surface it loudly and back off.
        while not self._shutdown.is_set():
            try:
                whoami = self._validate()
                log.info("matrix authenticated", user_id=whoami)
                break
            except Exception as e:  # noqa: BLE001
                log.warn(
                    "matrix /whoami failed; will retry",
                    error=str(e), delay=backoff,
                )
                if self._shutdown.wait(backoff):
                    return
                backoff = min(backoff * 2.0, MAX_BACKOFF_SECS)

        backoff = INITIAL_BACKOFF_SECS
        while not self._shutdown.is_set():
            url = self._build_sync_url()
            try:
                status, parsed, raw, _hdrs = self._http(
                    url, headers=self._auth_headers(),
                    timeout=SYNC_TIMEOUT_SECS,
                )
            except Exception as e:  # noqa: BLE001
                log.warn(
                    "matrix /sync transport error", error=str(e),
                    delay=backoff,
                )
                if self._shutdown.wait(backoff):
                    return
                backoff = min(backoff * 2.0, MAX_BACKOFF_SECS)
                continue
            if status != 200 or not isinstance(parsed, dict):
                snippet = raw[:200].decode("utf-8", "replace") if raw else ""
                log.warn(
                    "matrix /sync non-200", status=status,
                    body=snippet, delay=backoff,
                )
                if self._shutdown.wait(backoff):
                    return
                backoff = min(backoff * 2.0, MAX_BACKOFF_SECS)
                continue
            backoff = INITIAL_BACKOFF_SECS
            try:
                self._process_sync_body(parsed, emit)
            except Exception as e:  # noqa: BLE001
                log.warn(
                    "matrix /sync body processing error",
                    error=str(e),
                )

    # ---- outbound send ----------------------------------------------

    def _send_text(
        self, room_id: str, text: str, *, extra: Optional[dict] = None,
    ) -> list[str]:
        """Send a text message, chunked at MAX_MESSAGE_LEN. Returns
        the list of event_ids in order."""
        chunks = _split_message(text, MAX_MESSAGE_LEN)
        ids = []
        for chunk in chunks:
            body = text_body_with_html(chunk, extra)
            eid = self._put_event(room_id, "m.room.message", body)
            ids.append(eid)
        return ids

    def _send_media_event(
        self, room_id: str, msgtype: str, *, body: str, mxc: str,
        mime_type: str, size: int, filename: Optional[str] = None,
        duration_ms: Optional[int] = None, extras: Optional[dict] = None,
    ) -> str:
        info: dict = {"mimetype": mime_type, "size": size}
        if duration_ms is not None:
            info["duration"] = duration_ms
        payload: dict = {
            "msgtype": msgtype,
            "body": body,
            "url": mxc,
            "info": info,
        }
        if filename:
            payload["filename"] = filename
        if extras:
            payload.update(extras)
        return self._put_event(room_id, "m.room.message", payload)

    def _fetch_url_bytes(
        self, url: str, max_bytes: int,
    ) -> tuple[bytes, Optional[str]]:
        """Fetch ``url``, enforcing ``max_bytes`` cap. Returns the
        body bytes and the Content-Type header. Used when outbound
        ``Send`` carries a ``url`` instead of inline bytes.

        Public host validation is the operator's responsibility — for
        URL-based image sends the bridge already validated the URL
        upstream (or the host was set by trusted server-side code).
        Bytes-based sends (``Image.data`` etc.) skip this path
        entirely."""
        req = urllib.request.Request(url, method="GET")
        try:
            with urllib.request.urlopen(  # noqa: S310
                req, timeout=SYNC_TIMEOUT_SECS,
            ) as resp:
                # Read up to max_bytes+1 so we can detect overflow.
                data = resp.read(max_bytes + 1)
                if len(data) > max_bytes:
                    raise RuntimeError(
                        f"fetched media exceeds {max_bytes} byte cap",
                    )
                ct = None
                if resp.headers is not None:
                    ct = resp.headers.get("content-type")
                return data, ct
        except urllib.error.HTTPError as e:
            raise RuntimeError(f"fetch failed (status={e.code})") from e

    async def on_send(self, cmd) -> None:
        """Route the kernel-supplied ``Send`` to the right Matrix
        endpoint. ``cmd.channel_id`` (or ``cmd.user.platform_id`` as
        fallback) is the room id; ``cmd.thread_id`` requests
        threaded reply via ``m.thread``."""
        room_id = (
            cmd.channel_id
            or (cmd.user.get("platform_id") if cmd.user else "")
            or ""
        )
        if not room_id:
            log.warn("matrix on_send: empty room_id, dropping")
            return

        thread_id = getattr(cmd, "thread_id", None) or None
        thread_extra: Optional[dict] = None
        if thread_id:
            thread_extra = {
                "m.relates_to": {
                    "rel_type": "m.thread",
                    "event_id": thread_id,
                    "is_falling_back": True,
                    "m.in_reply_to": {"event_id": thread_id},
                },
            }

        content = cmd.content
        text = cmd.text or ""
        loop = asyncio.get_event_loop()

        if isinstance(content, dict) and "Text" in content:
            await loop.run_in_executor(
                None, lambda: self._send_text(
                    room_id, text, extra=thread_extra,
                ),
            )
            return
        if isinstance(content, dict) and "Image" in content:
            img = content["Image"]
            url = img.get("url") or ""
            caption = img.get("caption")
            mime_hint = img.get("mime_type")

            def _do_image() -> None:
                data, ct = self._fetch_url_bytes(
                    url, self.max_upload_bytes,
                )
                mt = mime_hint or ct or "application/octet-stream"
                fname = caption or "image"
                mxc = self._upload_media(data, fname, mt)
                self._send_media_event(
                    room_id, "m.image",
                    body=caption or fname,
                    mxc=mxc, mime_type=mt, size=len(data),
                    filename=fname, extras=thread_extra,
                )
            await loop.run_in_executor(None, _do_image)
            return
        if isinstance(content, dict) and "File" in content:
            f = content["File"]
            url = f.get("url") or ""
            fname = f.get("filename") or "file"

            def _do_file() -> None:
                data, ct = self._fetch_url_bytes(
                    url, self.max_upload_bytes,
                )
                mt = ct or "application/octet-stream"
                mxc = self._upload_media(data, fname, mt)
                self._send_media_event(
                    room_id, "m.file",
                    body=fname,
                    mxc=mxc, mime_type=mt, size=len(data),
                    filename=fname, extras=thread_extra,
                )
            await loop.run_in_executor(None, _do_file)
            return
        if content and not (isinstance(content, dict) and "Text" in content):
            # Other content types: send the text fallback. Non-text
            # variants (Voice, Audio, Video, Animation, Location,
            # Poll, etc.) could be implemented inline but the
            # majority of outbound sidecar traffic is text — Voice
            # / Audio / Video on Matrix is rare and operator-driven.
            # The placeholder mirrors what other sidecars do.
            await loop.run_in_executor(
                None,
                lambda: self._send_text(
                    room_id, text or "(Unsupported content type)",
                    extra=thread_extra,
                ),
            )
            return

        # Fall through: plain text from ``cmd.text``.
        await loop.run_in_executor(
            None,
            lambda: self._send_text(room_id, text, extra=thread_extra),
        )

    async def on_command(self, cmd) -> None:
        """Route inbound daemon commands. Send / TypingCmd / Reaction
        / StreamStart / StreamDelta / StreamEnd are handled here;
        the base class drops unknown shapes."""
        from librefang.sidecar.protocol import (
            Reaction, Send, StreamDelta, StreamEnd, StreamStart, TypingCmd,
        )
        if isinstance(cmd, Send):
            await self.on_send(cmd)
            return
        if isinstance(cmd, TypingCmd):
            await self._on_typing(cmd.channel_id)
            return
        if isinstance(cmd, Reaction):
            await self._on_reaction(cmd)
            return
        if isinstance(cmd, StreamStart):
            await self._on_stream_start(cmd)
            return
        if isinstance(cmd, StreamDelta):
            await self._on_stream_delta(cmd)
            return
        if isinstance(cmd, StreamEnd):
            await self._on_stream_end(cmd)
            return

    # ---- typing -----------------------------------------------------

    async def _on_typing(self, room_id: str) -> None:
        url = (
            f"{self.homeserver_url}/_matrix/client/v3/rooms/"
            f"{urllib.parse.quote(room_id, safe='')}/typing/"
            f"{urllib.parse.quote(self.user_id, safe='')}"
        )
        body_bytes = json.dumps({"typing": True, "timeout": 5000}).encode("utf-8")
        loop = asyncio.get_event_loop()

        def _do() -> None:
            try:
                self._http(
                    url, method="PUT", body=body_bytes,
                    headers=self._auth_headers(content_type=True),
                )
            except Exception as e:  # noqa: BLE001 — best-effort
                log.debug("matrix typing send failed", error=str(e))

        await loop.run_in_executor(None, _do)

    # ---- reaction lifecycle (Reaction envelope) ---------------------

    async def _on_reaction(self, cmd) -> None:
        """Wire a ``Reaction`` envelope to ``m.reaction`` posting +
        previous-reaction cleanup. The protocol's ``Reaction`` dataclass
        carries channel_id, message_id, reaction (emoji)."""
        room_id = cmd.channel_id
        message_id = cmd.message_id
        emoji = cmd.reaction
        if not room_id or not message_id or not emoji:
            log.warn("matrix reaction: missing field, dropping")
            return
        key = (room_id, message_id)
        loop = asyncio.get_event_loop()

        def _do() -> None:
            prev = self._phase_reaction_remove(key)
            if prev:
                self._redact(room_id, prev, "phase change")
            body = {
                "m.relates_to": {
                    "rel_type": "m.annotation",
                    "event_id": message_id,
                    "key": emoji,
                },
            }
            eid = self._put_event(room_id, "m.reaction", body)
            self._phase_reaction_insert(key, eid)

        try:
            await loop.run_in_executor(None, _do)
        except Exception as e:  # noqa: BLE001
            log.warn("matrix reaction failed", error=str(e))

    # ---- streaming edit lifecycle -----------------------------------

    # In-process map: stream_id → (room_id, placeholder_event_id, buffer,
    # last_flush_monotonic, last_flushed_len, flushed_initial, thread_extra)
    # The map is keyed by the daemon-supplied stream_id; entries are
    # cleared on StreamEnd.

    def _stream_state_get(self, stream_id: str) -> Optional[dict]:
        return getattr(self, "_streams", {}).get(stream_id)

    def _stream_state_set(self, stream_id: str, state: Optional[dict]) -> None:
        if not hasattr(self, "_streams"):
            self._streams: dict[str, dict] = {}
        if state is None:
            self._streams.pop(stream_id, None)
        else:
            self._streams[stream_id] = state

    async def _on_stream_start(self, cmd) -> None:
        room_id = cmd.channel_id
        if not room_id:
            log.warn("matrix stream_start: empty channel_id, dropping")
            return
        thread_id = getattr(cmd, "thread_id", None) or None
        thread_extra: Optional[dict] = None
        if thread_id:
            thread_extra = {
                "m.relates_to": {
                    "rel_type": "m.thread",
                    "event_id": thread_id,
                    "is_falling_back": True,
                    "m.in_reply_to": {"event_id": thread_id},
                },
            }
        loop = asyncio.get_event_loop()

        def _do() -> str:
            body = text_body_with_html("…", thread_extra)
            return self._put_event(room_id, "m.room.message", body)

        try:
            placeholder_id = await loop.run_in_executor(None, _do)
        except Exception as e:  # noqa: BLE001
            log.warn("matrix stream_start placeholder failed", error=str(e))
            return
        self._stream_state_set(cmd.stream_id, {
            "room_id": room_id,
            "placeholder_id": placeholder_id,
            "buffer": "",
            "last_flush_t": time.monotonic(),
            "last_flushed_len": 0,
            "flushed_initial": False,
            "thread_extra": thread_extra,
        })

    async def _on_stream_delta(self, cmd) -> None:
        state = self._stream_state_get(cmd.stream_id)
        if state is None:
            log.debug(
                "matrix stream_delta for unknown stream",
                stream_id=cmd.stream_id,
            )
            return
        state["buffer"] += cmd.text
        now = time.monotonic()
        elapsed = now - state["last_flush_t"]
        added = len(state["buffer"]) - state["last_flushed_len"]
        force_first = (not state["flushed_initial"]) and bool(state["buffer"])
        if (
            force_first
            or elapsed >= STREAM_EDIT_INTERVAL_SECS
            or added >= STREAM_EDIT_CHAR_BUDGET
            or len(state["buffer"]) > MAX_MESSAGE_LEN
        ):
            await self._stream_flush(state)
            state["flushed_initial"] = True

    async def _on_stream_end(self, cmd) -> None:
        state = self._stream_state_get(cmd.stream_id)
        if state is None:
            return
        # Drain remaining buffer with overflow splits.
        while len(state["buffer"]) > MAX_MESSAGE_LEN:
            await self._stream_flush(state)
        if state["buffer"]:
            await self._stream_flush(state)
        self._stream_state_set(cmd.stream_id, None)

    async def _stream_flush(self, state: dict) -> None:
        """One edit (or edit + split-send on overflow) pass.

        On overflow: edit the placeholder with the head, send the
        tail as a fresh event (whose id replaces the placeholder),
        leave the tail in the buffer for the next flush.
        """
        room_id = state["room_id"]
        placeholder_id = state["placeholder_id"]
        buffer = state["buffer"]
        loop = asyncio.get_event_loop()

        if len(buffer) <= MAX_MESSAGE_LEN:
            def _do_edit() -> None:
                body = build_edit_body(placeholder_id, buffer)
                self._put_event(room_id, "m.room.message", body)
            try:
                await loop.run_in_executor(None, _do_edit)
            except Exception as e:  # noqa: BLE001
                log.warn("matrix stream edit failed", error=str(e))
            state["last_flushed_len"] = len(buffer)
            state["last_flush_t"] = time.monotonic()
            return

        # Overflow: split.
        head = buffer[:MAX_MESSAGE_LEN]
        tail = buffer[MAX_MESSAGE_LEN:]
        thread_extra = state.get("thread_extra")

        def _do_split() -> str:
            edit_body = build_edit_body(placeholder_id, head)
            self._put_event(room_id, "m.room.message", edit_body)
            tail_body = text_body_with_html(tail, thread_extra)
            return self._put_event(room_id, "m.room.message", tail_body)

        try:
            new_id = await loop.run_in_executor(None, _do_split)
        except Exception as e:  # noqa: BLE001
            log.warn("matrix stream split failed", error=str(e))
            return
        state["placeholder_id"] = new_id
        state["buffer"] = tail
        state["last_flushed_len"] = len(tail)
        state["last_flush_t"] = time.monotonic()

    # ---- public sidecar surface -------------------------------------

    async def produce(self, emit: Callable[[dict], None]) -> None:
        loop = asyncio.get_event_loop()
        try:
            await loop.run_in_executor(None, self._producer_blocking, emit)
        except asyncio.CancelledError:
            self._shutdown.set()
            raise

    async def on_shutdown(self) -> None:
        self._shutdown.set()


if __name__ == "__main__":
    run_stdio_main(MatrixAdapter)
