#!/usr/bin/env python3
"""Telegram Bot API sidecar channel adapter for LibreFang.

A first-party adapter on the ``librefang.sidecar`` SDK (same shape as
``librefang.sidecar.adapters.ntfy``). The framework owns the ready/ack
handshake, supervised restart, and stdout protocol framing; this
module owns the Telegram transport.

PARITY STATUS (telegram-sidecar migration) — full parity with the
in-process ``crates/librefang-channels/src/telegram.rs`` so that
in-process adapter can be removed. Every subsystem below is a faithful
port of the audited Rust (function-by-function, not re-derived):

* DONE — outbound text prefers Telegram's native Rich Markdown
  (``sendRichMessage`` / ``editMessageText(rich_message=...)``, Bot API
  10.1+): Telegram parses the GFM itself, so tables, ``_italic_``,
  ``~~strikethrough~~`` and nested emphasis work, and the limit is
  32768 characters instead of 4096. Agent text goes through
  ``sanitize_rich_markdown`` first — a port of
  ``format::rich_sanitize`` — so quoted untrusted content cannot inject
  ``<tg-button>`` and friends.
* DONE — Markdown → Telegram-HTML formatter subsystem, now the
  fallback for Bot API servers older than 10.1: a byte-exact
  port of ``formatter::markdown_to_telegram_html`` + the
  ``sanitize_telegram_html`` security pass (tag/scheme allowlist,
  attribute-injection escaping, unclosed-tag balancing) + the
  ``message_truncator`` UTF-16/HTML-entity-aware chunker
  (``split_to_utf16_chunks``), sent with ``parse_mode=HTML``, with the
  same plain-text retry on Telegram's "can't parse entities" 400.
* DONE — full inbound parsing: text/bot-command, photo, document,
  audio, voice, animation, video, video_note, location, sticker;
  ``from`` / ``sender_chat`` sender extraction; ``callback_query`` →
  ButtonCallback; ``poll_answer`` → PollAnswer; ``edited_message``;
  reply-to context; getFile URL resolution with text fallback;
  ALLOWED_USERS by id *and* username.
* DONE — full outbound dispatch for every ``ChannelContent`` variant
  (Image→sendPhoto, File→sendDocument/sendVoice, Voice/Video/Audio/
  Animation→send*, Sticker, Location, MediaGroup, Poll, Interactive,
  EditInteractive, DeleteMessage), incl. private-URL → multipart
  upload and OGG/Opus voice routing, 429 ``retry_after`` retry.
* DONE — outbound rich capabilities: ``typing``, ``reaction`` (same
  emoji map, optional clear-on-done), ``interactive`` (inline
  keyboards), ``thread`` (forum ``message_thread_id``), ``streaming``
  (throttled editMessageText).

Stdlib-only (the SDK has zero runtime deps — no ``requests``).
Configure via ``[[sidecar_channels]]``:

    [[sidecar_channels]]
    name = "telegram"
    command = "python3"
    args = ["-m", "librefang.sidecar.adapters.telegram"]
    channel_type = "telegram"
    [sidecar_channels.env]
    TELEGRAM_BOT_TOKEN = "123456:ABC-..."     # from @BotFather (required)
    # ALLOWED_USERS = "111,@alice"            # optional id/username allowlist
    # TELEGRAM_CLEAR_DONE_REACTION = "1"      # clear ✅ instead of 🎉
"""
from __future__ import annotations

import asyncio
import html
import json
import os
import socket
import time
import urllib.error
import urllib.parse
import urllib.request
import uuid

from librefang.sidecar import Field, Schema, SidecarAdapter, protocol, run_stdio_main
from librefang.sidecar import logging as log
from librefang.sidecar.common import MAX_BACKOFF_SECS

LONGPOLL_SERVER_SECS = 30
LONGPOLL_CLIENT_SECS = 35
SEND_TIMEOUT_SECS = 10
# Telegram's message limit is 4096 *UTF-16 code units* (not chars).
TELEGRAM_MSG_LIMIT = 4096
# Rich message limit: "Up to 32768 UTF-8 characters in the rich message
# text" (Bot API, Rich Message Limits). Counted in characters, unlike the
# legacy sendMessage path which counts UTF-16 code units.
RICH_MSG_LIMIT = 32768
# Throttle streamed editMessageText (mirrors the Rust adapter's 1s).
STREAM_EDIT_INTERVAL = 1.0
RETRY_AFTER_DEFAULT_SECS = 2
# Cap how long we will sleep on a 429 retry_after from Telegram.
# A flood-wait can return hours; sleeping that long would stall the entire produce loop with no cancellation.
# Anything above this skips the retry and returns the original 429 response, matching telegram.rs's MAX_RETRY_AFTER_SECS.
MAX_RETRY_AFTER_SECS = 300
# Backoff cap for the `produce()` reconnect loop on transient network
# / DNS failures (#5111). Matches the convention every other polling
# sidecar (bluesky / discord / line / mastodon / mattermost /
# nextcloud / ntfy / reddit / rocketchat / twitch) settled on.
PARSE_MODE_HTML = "HTML"
# Max bytes downloaded for the private-URL → multipart fallback.
MAX_UPLOAD_BYTES = 50 * 1024 * 1024


# ====================================================================
# UTF-16 / HTML-entity aware chunking
# (port of crate::message_truncator)
# ====================================================================


def _utf16_len(s: str) -> int:
    """UTF-16 code-unit length (chars > U+FFFF count as 2)."""
    return sum(2 if ord(c) > 0xFFFF else 1 for c in s)


def _truncate_to_utf16_limit(s: str, limit: int) -> str:
    """Longest prefix of `s` whose UTF-16 length is <= `limit`."""
    if _utf16_len(s) <= limit:
        return s
    total = 0
    for idx, ch in enumerate(s):
        w = 2 if ord(ch) > 0xFFFF else 1
        if total + w > limit:
            return s[:idx]
        total += w
    return s


_ENTITY_PREFIXES = {
    "a", "am", "amp", "l", "lt", "g", "gt", "q", "qu", "quo", "quot",
    "n", "nb", "nbs", "nbsp", "#", "#x",
}


def _adjust_html_entity_boundary(chunk: str) -> str:
    """Shrink `chunk` so it never ends inside a partial HTML entity
    (`&lt`, `&#x1F6` …). Faithful port of
    ``message_truncator::adjust_html_entity_boundary``."""
    amp = chunk.rfind("&")
    if amp == -1:
        return chunk
    tail = chunk[amp:]
    if ";" in tail:
        return chunk
    after = tail[1:]
    is_entity_like = (
        after in _ENTITY_PREFIXES
        or (after.startswith("#") and after[1:].isdigit() and after[1:] != "")
        or (
            after.startswith("#x")
            and after[2:] != ""
            and all(c in "0123456789abcdefABCDEF" for c in after[2:])
        )
    )
    if not is_entity_like:
        return chunk
    return chunk[:amp]


def _split_to_utf16_chunks(s: str, limit: int = TELEGRAM_MSG_LIMIT) -> list:
    """Split `s` into chunks each <= `limit` UTF-16 units, preferring a
    newline boundary and never breaking an HTML entity. Faithful port
    of ``message_truncator::split_to_utf16_chunks`` incl. its
    zero-progress guards."""
    if _utf16_len(s) <= limit:
        return [s]
    chunks: list = []
    remaining = s
    while remaining:
        if _utf16_len(remaining) <= limit:
            chunks.append(remaining)
            break
        safe_prefix = _truncate_to_utf16_limit(remaining, limit)
        nl = safe_prefix.rfind("\n")
        if nl > 0 and safe_prefix[nl - 1] == "\r":
            split_at = nl - 1
        elif nl != -1:
            split_at = nl
        else:
            split_at = len(safe_prefix)
        chunk = remaining[:split_at]
        chunk = _adjust_html_entity_boundary(chunk)
        rest = remaining[len(chunk):]

        if chunk == "":
            if safe_prefix == "":
                # Even one char exceeds the limit — emit it anyway to
                # guarantee forward progress.
                nxt = remaining[:1] if remaining else remaining
                chunks.append(nxt)
                remaining = remaining[len(nxt):]
            else:
                # Entity guard collapsed the chunk: emit the full entity
                # (slightly oversized) if a ';' is within a short window,
                # else fall back to the size-respecting safe prefix.
                semi = remaining[:16].find(";")
                if semi != -1:
                    end = semi + 1
                    chunks.append(remaining[:end])
                    remaining = remaining[end:]
                else:
                    chunks.append(safe_prefix)
                    remaining = remaining[len(safe_prefix):]
            continue
        chunks.append(chunk)
        if rest.startswith("\r\n"):
            remaining = rest[2:]
        elif rest.startswith("\n"):
            remaining = rest[1:]
        else:
            remaining = rest
    return chunks


def _truncate_utf8(s: str, max_bytes: int) -> str:
    """Longest prefix of `s` that is <= `max_bytes` UTF-8 bytes,
    aligned to a char boundary (Telegram callback_data is 64 bytes)."""
    b = s.encode("utf-8")
    if len(b) <= max_bytes:
        return s
    return b[:max_bytes].decode("utf-8", "ignore")


def _truncate_with_ellipsis(text: str, max_bytes: int) -> str:
    b = text.encode("utf-8")
    if len(b) <= max_bytes:
        return text
    return b[:max_bytes].decode("utf-8", "ignore") + "..."


# ====================================================================
# Markdown → Telegram-HTML formatter  (port of crate::formatter)
# ====================================================================


def _escape_html(text: str) -> str:
    """formatter::escape_html — & first, then < and >."""
    return text.replace("&", "&amp;").replace("<", "&lt;").replace(">", "&gt;")


def _fence_delimiter(line: str):
    if line.startswith("```"):
        return "```"
    if line.startswith("~~~"):
        return "~~~"
    return None


def _heading_text(line: str):
    hashes = 0
    for c in line:
        if c == "#":
            hashes += 1
        else:
            break
    if 1 <= hashes <= 6 and hashes < len(line) and line[hashes] == " ":
        return line[hashes + 1:]
    return None


def _unordered_list_item(line: str):
    for prefix in ("- ", "* ", "+ "):
        if line.startswith(prefix):
            return line[len(prefix):]
    return None


def _ordered_list_item(line: str):
    digits = 0
    for c in line:
        if c.isdigit() and c in "0123456789":
            digits += 1
        else:
            break
    if digits == 0:
        return None
    rest = line[digits:]
    if rest.startswith(". "):
        return rest[2:]
    if rest.startswith(") "):
        return rest[2:]
    return None


def _render_inline_markdown(text: str) -> str:
    """formatter::render_inline_markdown — links, bold, code, italic."""
    result = _escape_html(text)

    # Links: [text](url) → <a href="url">text</a>
    while True:
        bs = result.find("[")
        if bs == -1:
            break
        be_rel = result[bs:].find("](")
        if be_rel == -1:
            break
        be = bs + be_rel
        pe_rel = result[be + 2:].find(")")
        if pe_rel == -1:
            break
        pe = be + 2 + pe_rel
        link_text = result[bs + 1:be]
        # The source string was escaped before parsing. Restore the URL;
        # the sanitizer performs the single canonical attribute escape.
        url = html.unescape(result[be + 2:pe])
        result = (
            result[:bs] + f'<a href="{url}">{link_text}</a>' + result[pe + 1:]
        )

    # Bold: **text** → <b>text</b>
    while True:
        start = result.find("**")
        if start == -1:
            break
        end_rel = result[start + 2:].find("**")
        if end_rel == -1:
            break
        end = start + 2 + end_rel
        inner = result[start + 2:end]
        result = result[:start] + f"<b>{inner}</b>" + result[end + 2:]

    # Inline code: `text` → <code>text</code>
    while True:
        start = result.find("`")
        if start == -1:
            break
        end_rel = result[start + 1:].find("`")
        if end_rel == -1:
            break
        end = start + 1 + end_rel
        inner = result[start + 1:end]
        result = result[:start] + f"<code>{inner}</code>" + result[end + 1:]

    # Italic: *text* → <i>text</i> (single star only)
    out = []
    in_italic = False
    prev = "\0"
    for i, ch in enumerate(result):
        nxt = result[i + 1] if i + 1 < len(result) else ""
        if ch == "*" and prev != "*" and nxt != "*":
            out.append("</i>" if in_italic else "<i>")
            in_italic = not in_italic
        else:
            out.append(ch)
        prev = ch
    return "".join(out)


def _rust_lines(s: str) -> list:
    """Match Rust ``str::lines``: split on '\\n', a single trailing
    newline does not yield a final empty line; "" yields no lines."""
    if s == "":
        return []
    parts = s.split("\n")
    if parts and parts[-1] == "":
        parts = parts[:-1]
    return parts


def markdown_to_telegram_html(text: str) -> str:
    """Byte-exact port of ``formatter::markdown_to_telegram_html``."""
    normalized = text.replace("\r\n", "\n").replace("\r", "\n")
    blocks: list = []
    lines = _rust_lines(normalized)
    i = 0
    n = len(lines)
    while i < n:
        line = lines[i]
        trimmed = line.strip()

        if trimmed == "":
            i += 1
            continue

        fence = _fence_delimiter(trimmed)
        if fence is not None:
            i += 1
            code_lines = []
            while i < n:
                candidate = lines[i].strip()
                if candidate.startswith(fence):
                    i += 1
                    break
                code_lines.append(lines[i])
                i += 1
            code = _escape_html("\n".join(code_lines))
            blocks.append(f"<pre><code>{code}</code></pre>")
            continue

        head = _heading_text(trimmed)
        if head is not None:
            blocks.append(f"<b>{_render_inline_markdown(head.strip())}</b>")
            i += 1
            continue

        if trimmed.startswith(">"):
            quote_lines = []
            while i < n:
                current = lines[i].strip()
                if current == "" or not current.startswith(">"):
                    break
                content = current[1:].lstrip() if current.startswith(">") \
                    else current
                quote_lines.append(_render_inline_markdown(content))
                i += 1
            blocks.append(
                "<blockquote>" + "\n".join(quote_lines) + "</blockquote>"
            )
            continue

        item = _unordered_list_item(trimmed)
        if item is not None:
            items = ["• " + _render_inline_markdown(item.strip())]
            i += 1
            while i < n:
                current = lines[i].strip()
                nxt = _unordered_list_item(current)
                if nxt is not None:
                    items.append(
                        "• " + _render_inline_markdown(nxt.strip())
                    )
                    i += 1
                elif current == "":
                    i += 1
                    break
                else:
                    break
            blocks.append("\n".join(items))
            continue

        item = _ordered_list_item(trimmed)
        if item is not None:
            items = ["1. " + _render_inline_markdown(item.strip())]
            counter = 2
            i += 1
            while i < n:
                current = lines[i].strip()
                nxt = _ordered_list_item(current)
                if nxt is not None:
                    items.append(
                        f"{counter}. " + _render_inline_markdown(nxt.strip())
                    )
                    counter += 1
                    i += 1
                elif current == "":
                    i += 1
                    break
                else:
                    break
            blocks.append("\n".join(items))
            continue

        # Paragraph
        paragraph = [trimmed]
        i += 1
        while i < n:
            current = lines[i].strip()
            if (
                current == ""
                or _fence_delimiter(current) is not None
                or _heading_text(current) is not None
                or current.startswith(">")
                or _unordered_list_item(current) is not None
                or _ordered_list_item(current) is not None
            ):
                break
            paragraph.append(current)
            i += 1
        blocks.append(_render_inline_markdown("\n".join(paragraph)))

    return "\n\n".join(blocks)


# ====================================================================
# Telegram HTML sanitizer  (port of telegram.rs sanitize_telegram_html)
# ====================================================================

_ALLOWED_TAGS = {
    "b", "i", "u", "s", "em", "strong", "a", "code", "pre",
    "blockquote", "tg-spoiler", "tg-emoji",
}
_ALLOWED_HREF_SCHEMES = {"https", "http", "mailto", "tg"}


def _escape_html_text(s: str) -> str:
    out = []
    for c in s:
        if c == "<":
            out.append("&lt;")
        elif c == ">":
            out.append("&gt;")
        elif c == "&":
            out.append("&amp;")
        elif c == '"':
            out.append("&quot;")
        else:
            out.append(c)
    return "".join(out)


def _is_safe_href(url: str) -> bool:
    trimmed = url.strip()
    colon = trimmed.find(":")
    if colon == -1:
        return False
    return trimmed[:colon].lower() in _ALLOWED_HREF_SCHEMES


def _parse_attrs(attrs: str) -> list:
    out = []
    i = 0
    n = len(attrs)
    while i < n:
        while i < n and attrs[i].isspace():
            i += 1
        if i >= n:
            break
        key_start = i
        while i < n and attrs[i] != "=" and not attrs[i].isspace():
            i += 1
        key = attrs[key_start:i].lower()
        if key == "":
            break
        while i < n and attrs[i].isspace():
            i += 1
        if i >= n or attrs[i] != "=":
            out.append((key, ""))
            continue
        i += 1  # consume '='
        while i < n and attrs[i].isspace():
            i += 1
        if i < n and attrs[i] in ("\"", "'"):
            quote = attrs[i]
            i += 1
            val_start = i
            while i < n and attrs[i] != quote:
                i += 1
            val = attrs[val_start:i]
            if i < n:
                i += 1
            out.append((key, val))
        else:
            val_start = i
            while i < n and not attrs[i].isspace():
                i += 1
            out.append((key, attrs[val_start:i]))
    return out


def _rebuild_safe_tag(tag_name: str, attrs_raw: str, self_closing: bool):
    attrs = _parse_attrs(attrs_raw)
    buf = "<" + tag_name
    lc = tag_name.lower()
    if lc == "a":
        href = next((v for k, v in attrs if k == "href"), None)
        if href is None or not _is_safe_href(href):
            return None
        buf += ' href="' + _escape_html_text(href) + '"'
    elif lc == "code":
        v = next((v for k, v in attrs if k == "class"), None)
        if v is not None:
            buf += ' class="' + _escape_html_text(v) + '"'
    elif lc == "tg-emoji":
        v = next((v for k, v in attrs if k == "emoji-id"), None)
        if v is not None:
            buf += ' emoji-id="' + _escape_html_text(v) + '"'
    buf += ">"
    if self_closing:
        # Telegram's HTML subset has no self-closing-tag syntax: emitting
        # a literal `<tag/>` would either be rejected by the Bot API's
        # "Unclosed start tag" check or (if tolerated) leave the tag open
        # for the rest of the message, matching telegram.rs's fix — close
        # it immediately instead of leaking the marker into the output.
        buf += "</" + tag_name + ">"
    return buf


def sanitize_telegram_html(text: str) -> str:
    """Port of telegram.rs ``sanitize_telegram_html``: drop tags
    outside the Telegram allowlist, escape unknown ones, enforce safe
    `<a href>` schemes, escape attribute values, and balance unclosed
    tags."""
    result = []
    open_tags: list = []
    i = 0
    n = len(text)
    while i < n:
        ch = text[i]
        if ch == "<":
            end_off = text[i:].find(">")
            if end_off != -1:
                tag_end = i + end_off
                tag_content = text[i + 1:tag_end]
                is_closing = tag_content.startswith("/")
                stripped = tag_content[1:] if is_closing else tag_content
                name_raw = ""
                for c in stripped:
                    if c.isspace() or c == "/" or c == ">":
                        break
                    name_raw += c
                if name_raw != "" and name_raw.lower() in _ALLOWED_TAGS:
                    name_lc = name_raw.lower()
                    if is_closing:
                        pos = None
                        for k in range(len(open_tags) - 1, -1, -1):
                            if open_tags[k] == name_lc:
                                pos = k
                                break
                        if pos is not None:
                            # Close every tag above (and including) the
                            # match, innermost first, mirroring
                            # telegram.rs — sanitiser priority is
                            # "produce valid HTML" not "preserve nesting
                            # depth" when tags cross.
                            for unclosed in reversed(open_tags[pos:]):
                                result.append("</" + unclosed + ">")
                            del open_tags[pos:]
                        else:
                            result.append("&lt;")
                            result.append(_escape_html_text(tag_content))
                            result.append("&gt;")
                    else:
                        # rstrip before checking for the marker so a
                        # self-closing tag with trailing whitespace before
                        # `>` (e.g. `<tag/ >`, valid HTML) is still detected
                        # — matches sanitize.rs's `attrs.trim_end().ends_with('/')`.
                        self_closing = tag_content.rstrip().endswith("/")
                        attrs_raw = tag_content[len(name_raw):]
                        if self_closing:
                            attrs_raw = attrs_raw.rstrip()[:-1]
                        attrs_raw = attrs_raw.strip()
                        rebuilt = _rebuild_safe_tag(
                            name_raw, attrs_raw, self_closing
                        )
                        if rebuilt is not None:
                            result.append(rebuilt)
                            if not self_closing:
                                open_tags.append(name_lc)
                else:
                    result.append("&lt;")
                    result.append(_escape_html_text(tag_content))
                    result.append("&gt;")
                i = tag_end + 1
            else:
                result.append("&lt;")
                i += 1
        else:
            result.append(ch)
            i += 1

    for tag in reversed(open_tags):
        result.append("</" + tag + ">")
    return "".join(result)


def _format_and_sanitize(text: str) -> str:
    """Daemon sends raw agent Markdown; the in-process adapter applied
    the formatter at the bridge. The sidecar owns it now: format →
    sanitize (defense-in-depth over the safe-tag subset)."""
    return sanitize_telegram_html(markdown_to_telegram_html(text))


# ====================================================================
# Rich Markdown sanitiser
# (port of format::rich_sanitize::sanitize_rich_markdown)
# ====================================================================

# Passive formatting tags safe to let Telegram's rich parser handle.
# Everything else is escaped to literal text. An allowlist (rather than a
# denylist of known-active tags) means any tag a future Bot API version
# adds is inert here until someone deliberately allows it.
_ALLOWED_RICH_TAGS = frozenset({
    # inline emphasis
    "b", "strong", "i", "em", "u", "ins", "s", "strike", "del", "mark",
    "sub", "sup", "tg-spoiler", "tg-emoji",
    # code
    "code", "pre",
    # links (href scheme is checked separately, see _anchor_href_allowed)
    "a",
    # block structure
    "p", "br", "hr", "blockquote",
    "h1", "h2", "h3", "h4", "h5", "h6",
    "ul", "ol", "li",
    "table", "thead", "tbody", "tr", "td", "th",
})

_TAG_NAME_CHARS = frozenset(
    "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789-"
)


def _fence_at(s: str, i: int):
    """Marker char, run length and whether an info string follows, for a code
    fence line at `i`; None when the line is not a fence. CommonMark allows an
    info string only on the *opening* fence, so callers matching a closing
    fence must reject ``has_info``. Up to three leading spaces still open a
    fence."""
    j = i
    spaces = 0
    while j < len(s) and s[j] == " " and spaces < 3:
        j += 1
        spaces += 1
    if j >= len(s):
        return None
    marker = s[j]
    if marker not in ("`", "~"):
        return None
    run = 0
    while j + run < len(s) and s[j + run] == marker:
        run += 1
    if run < 3:
        return None
    rest = s[j + run:_line_end(s, i)]
    return (marker, run, rest.strip(" \t\r\n") != "")


# Schemes a link destination may carry. Mirrors the Rust
# `rich_sanitize::ALLOWED_HREF_SCHEMES` and `sanitize_telegram_html`'s own
# allowlist, whose guarantee (javascript: / data: never reach a live tag) the
# legacy path enforces and this path must not weaken.
_ALLOWED_RICH_HREF_SCHEMES = ("https:", "http:", "mailto:", "tg:")
_SCHEME_CHARS = frozenset(
    "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789+.-"
)


def _decode_entity(entity: str):
    """Decode the entity forms that can stand in for a character inside a
    scheme: ``&#58;``, ``&#x3a;`` and the named ``&colon;``."""
    if entity.lower() == "colon":
        return ":"
    if not entity.startswith("#"):
        return None
    digits = entity[1:]
    try:
        if digits[:1].lower() == "x":
            return chr(int(digits[1:], 16))
        return chr(int(digits))
    except (ValueError, OverflowError):
        return None


def _normalise_destination(dest: str) -> str:
    """Strip a ``<...>`` wrapper, decode HTML entities, and drop whitespace and
    control characters, so the scheme check sees the destination the parser
    will resolve. Only the leading run matters, so this stops at the first
    delimiter."""
    trimmed = dest.strip()
    if trimmed.startswith("<"):
        trimmed = trimmed[1:]
        if trimmed.endswith(">"):
            trimmed = trimmed[:-1]

    out = []
    i = 0
    n = len(trimmed)
    while i < n:
        ch = trimmed[i]
        if ch == "&":
            j = i + 1
            while j < n and trimmed[j] != ";" and j - i - 1 < 8:
                j += 1
            entity = trimmed[i + 1:j]
            if j < n and trimmed[j] == ";":
                j += 1
            decoded = _decode_entity(entity)
            # A decoded character goes through the same filter as a literal
            # one. Appending it unconditionally let `&#32;javascript:` start
            # with a space, which reads as "no scheme here" while the HTML and
            # URL parsers both discard that space and resolve the scheme.
            # An entity we cannot decode is skipped rather than ending the
            # scan: `&Tab;` and `&NewLine;` are real HTML5 entities we do not
            # know, and stopping truncated the destination so the scheme
            # behind it was never examined.
            if decoded is not None and not (decoded.isspace()
                                            or ord(decoded) < 32
                                            or ord(decoded) == 127):
                out.append(decoded)
            i = j
            continue
        if not (ch.isspace() or ord(ch) < 32 or ord(ch) == 127):
            out.append(ch)
        i += 1
        # A scheme is short; past this the answer cannot change.
        if len(out) > 64:
            break
    return "".join(out)


def _scheme_is_allowed(dest: str) -> bool:
    """True when `dest` carries no scheme at all (a relative target or
    ``#anchor``) or carries one on the allowlist.

    The destination is normalised first, because the check has to hold on what
    the *parser* sees rather than on the literal bytes: ``<...>``-wrapped
    destinations, HTML entities standing in for the colon
    (``javascript&#58;...``) and embedded whitespace or control characters
    (``java\\tscript:...``) all reach a live scheme otherwise."""
    dest = _normalise_destination(dest)
    if not dest or not dest[0].isascii() or not dest[0].isalpha():
        return True  # no scheme possible
    j = 0
    while j < len(dest) and dest[j] in _SCHEME_CHARS:
        j += 1
    if j >= len(dest) or dest[j] != ":":
        return True  # not a scheme, just text before a slash or space
    scheme = dest[:j + 1].lower()
    return scheme in _ALLOWED_RICH_HREF_SCHEMES


def _link_destination_at(s: str, i: int):
    """Destination of a ``[label](destination)`` starting at `i` (which must be
    ``[``), or None when this is not an inline link.

    The label may contain balanced brackets (``[a[b]c]``) and the destination
    may contain balanced parentheses or be ``<...>``-wrapped, so both are
    scanned with a depth counter rather than to the first closer."""
    n = len(s)
    depth = 0
    j = i
    while j < n:
        ch = s[j]
        if ch == "\\":
            j += 2
            continue
        if ch == "[":
            depth += 1
        elif ch == "]":
            depth -= 1
            if depth == 0:
                break
        j += 1
    if j >= n or j + 1 >= n or s[j + 1] != "(":
        return None
    start = j + 2
    # A `<...>` destination may hold anything up to the `>`.
    if start < n and s[start] == "<":
        k = start + 1
        while k < n and s[k] not in ">\n":
            k += 1
        return s[start:k + 1] if k < n and s[k] == ">" else None
    # A bare destination ends at the first whitespace or at the closing `)`.
    # Whitespace (including one line break) may then separate it from an
    # optional title and the `)` — treating a break as "not a link" left
    # `[x](javascript:...\n)` unescaped even though Markdown resolves it.
    k = start
    parens = 0
    while k < n:
        ch = s[k]
        if ch == "\\":
            k += 2
            continue
        if ch == "(":
            parens += 1
        elif ch == ")":
            if parens == 0:
                return s[start:k]
            parens -= 1
        elif _is_ascii_space(ch):
            break
        k += 1
    dest = s[start:k]
    k = _skip_ascii_space(s, k)
    if k < n and s[k] in "\"'(":
        closer = ")" if s[k] == "(" else s[k]
        k += 1
        while k < n and s[k] != closer:
            k += 1
        k = _skip_ascii_space(s, k + 1)
    return dest if k < n and s[k] == ")" else None


def _skip_ascii_space(s: str, k: int) -> int:
    while k < len(s) and _is_ascii_space(s[k]):
        k += 1
    return k


def _reference_definition_at(s: str, i: int):
    """Destination of a link reference definition ``[label]: destination``
    starting at `i`, or None when the line is not a definition.

    Without this the rich path is weaker than the legacy one it replaces: a
    ``[x][ref]`` reference plus a ``[ref]: javascript:...`` definition renders
    a live link here, while the legacy pipeline leaves it as inert text."""
    n = len(s)
    j = i + 1
    while j < n and s[j] != "]" and not _is_line_break(s[j]):
        j += 1
    if j >= n or s[j] != "]" or j + 1 >= n or s[j + 1] != ":":
        return None
    start = _skip_ascii_space(s, j + 2)
    end = start
    while end < n and not _is_ascii_space(s[end]) and s[end] != ">":
        end += 1
    # `<...>` wrapping is allowed here too; _normalise_destination strips it.
    if end < n and s[end] == ">":
        end += 1
    return s[start:end] if end > start else None


def _anchor_href_allowed(s: str, i: int) -> bool:
    """True when the ``<a ...>`` opening at `i` has no href, or one whose
    scheme is allowed. A closing ``</a>`` carries no href and always passes.

    Attributes are walked as ``name [= value]`` pairs rather than by searching
    for the substring ``href``: HTML attribute names are case-insensitive
    (``HREF``), and a substring search matches inside *other* attributes, so
    ``<a data-href="https://ok" href="...">`` would be judged on the decoy and
    the real destination never inspected."""
    if i + 1 < len(s) and s[i + 1] == "/":
        return True
    # Exclusive of the `>`; a tag may span lines, so this must not
    # stop at a break.
    tag_end = max(_tag_end(s, i) - 1, i + 1)

    # Skip the tag name.
    j = i + 1
    while j < tag_end and not _is_ascii_space(s[j]):
        j += 1

    while j < tag_end:
        while j < tag_end and _is_ascii_space(s[j]):
            j += 1
        if j >= tag_end:
            break
        name_start = j
        while j < tag_end and not _is_ascii_space(s[j]) and s[j] != "=":
            j += 1
        name = s[name_start:j]
        k = j
        while k < tag_end and _is_ascii_space(s[k]):
            k += 1
        if k >= tag_end or s[k] != "=":
            # Valueless attribute; nudge past an empty name to guarantee
            # forward progress.
            if j == name_start:
                j += 1
            continue
        k += 1
        while k < tag_end and _is_ascii_space(s[k]):
            k += 1
        if k < tag_end and s[k] in "\"'":
            quote = s[k]
            start = k + 1
            end = start
            while end < tag_end and s[end] != quote:
                end += 1
            nxt = min(end + 1, tag_end)
        else:
            start = k
            end = start
            while end < tag_end and not _is_ascii_space(s[end]):
                end += 1
            nxt = end
        if name.lower() == "href":
            return _scheme_is_allowed(s[start:end])
        j = nxt if nxt > j else j + 1
    return True  # no href attribute at all


def _is_ascii_space(ch: str) -> bool:
    """ASCII whitespace only. HTML5 defines tag whitespace as ASCII and Rust's
    ``is_ascii_whitespace`` matches; Python's ``str.isspace`` is
    Unicode-aware and would treat U+00A0 / U+2028 as separators,
    diverging from the Rust port."""
    return ch in " \t\n\r\x0c"


def _is_bullet(s: str, j: int) -> bool:
    """A list bullet: ``-``, ``*`` or ``+`` followed by a space or tab."""
    return (j < len(s) and s[j] in "-*+"
            and j + 1 < len(s) and s[j + 1] in " \t")


def _thematic_break_at(s: str, j: int) -> bool:
    """CommonMark thematic break: three or more ``-``, ``*`` or ``_``, all the
    same, with only spaces and tabs between them and nothing else."""
    if j >= len(s) or s[j] not in "-*_":
        return False
    marker = s[j]
    count = 0
    k = j
    while k < len(s):
        ch = s[k]
        if ch == marker:
            count += 1
        elif ch in " \t":
            pass
        elif ch in "\n\r":
            break
        else:
            return False
        k += 1
    return count >= 3


def _setext_underline_at(s: str, j: int) -> bool:
    """Setext heading underline: a run of ``=`` or ``-``, optionally followed
    by spaces. A ``-`` run of three or more is already a thematic break, but
    ``-`` or ``--`` alone is a setext underline and nothing else."""
    if j >= len(s) or s[j] not in "=-":
        return False
    marker = s[j]
    k = j
    while k < len(s) and s[k] == marker:
        k += 1
    if k == j:
        return False
    while k < len(s) and s[k] in " \t":
        k += 1
    return k >= len(s) or s[k] in "\n\r"


def _starts_new_block(s: str, i: int) -> bool:
    """True when the line starting at `i` ends the current paragraph.

    Bounds an inline code-span scan to a single paragraph. The set below is
    taken from CommonMark's "can interrupt a paragraph" rules rather than
    assembled from memory — a list built by recalling cases closes the inputs
    you thought of and leaves the hole open on the rest, which is exactly how
    ``***``, ``---``, ``___`` and HTML blocks survived the first attempt.

    Erring towards True is safe: the backticks are then treated as ordinary
    text and their content is escaped. Erring towards False copies the region
    verbatim, which is the injection this module exists to prevent. So ``|``
    (a GFM table row) and any ``<`` (HTML block) are included."""
    if i >= len(s):
        return True
    j = i
    while j < len(s) and s[j] in " \t":
        j += 1
    if j >= len(s) or s[j] in "\n\r":
        return True  # blank line; \r covers CRLF from quoted email/web content
    ch = s[j]
    # Block quotation, ATX heading, GFM table row, HTML block.
    if ch in ">#|<":
        return True
    if ch == "`":
        return _fence_at(s, i) is not None
    # `~` opens a fence but is not a list bullet.
    if ch == "~":
        return _fence_at(s, i) is not None or _thematic_break_at(s, j)
    if ch in "-*_=":
        return (_thematic_break_at(s, j) or _setext_underline_at(s, j)
                or _is_bullet(s, j))
    if ch == "+":
        return _is_bullet(s, j)
    if ch.isdigit():
        k = j
        while k < len(s) and s[k].isdigit():
            k += 1
        return (k < len(s) and s[k] in ".)"
                and k + 1 < len(s) and s[k + 1] in " \t")
    return False


def _is_line_break(ch: str) -> bool:
    """True for either line-break character. Markdown treats a lone ``\\r`` as a
    line ending, and lone-CR text reaches us verbatim from quoted mail and older
    transports — handling only ``\\n`` means the block-boundary checks never run
    on such input at all."""
    return ch in "\n\r"


def _next_line_start(s: str, j: int) -> int:
    """Index just past the line terminator at `j`, treating ``\\r\\n`` as one.
    Returns `j` unchanged when it is not a terminator."""
    if j >= len(s):
        return j
    if s[j] == "\r" and j + 1 < len(s) and s[j + 1] == "\n":
        return j + 2
    return j + 1 if _is_line_break(s[j]) else j


def _line_end(s: str, i: int) -> int:
    """Index just past the end of the line starting at `i`, including its
    terminator (``\\n``, ``\\r``, or the ``\\r\\n`` pair)."""
    j = i
    while j < len(s) and not _is_line_break(s[j]):
        j += 1
    return _next_line_start(s, j)


def _tag_end(s: str, i: int) -> int:
    """Index just past the ``>`` closing the tag opening at `i`, or end of input.
    A tag may span lines, so this deliberately does not stop at a line break:
    doing so left ``<a\\nhref="javascript:...">`` with no attributes to inspect."""
    k = i + 1
    while k < len(s) and s[k] != ">":
        k += 1
    return min(k + 1, len(s))


def _tag_name_at(s: str, i: int):
    """Lower-cased tag name of the HTML tag opening at `i` (which must be
    `<`), for both ``<foo ...>`` and ``</foo>``. None when not tag-shaped."""
    j = i + 1
    if j < len(s) and s[j] == "/":
        j += 1
    start = j
    while j < len(s) and s[j] in _TAG_NAME_CHARS:
        j += 1
    if j == start:
        return None
    # Must actually terminate like a tag, not be prose such as `5 <3 apples`.
    if j >= len(s) or not (s[j] in (">", "/") or s[j].isspace()):
        return None
    return s[start:j].lower()


def sanitize_rich_markdown(text: str) -> str:
    """Neutralise "active" constructs in agent-authored Rich Markdown
    before it is handed to ``sendRichMessage``.

    Rich Markdown "can contain arbitrary HTML" (Bot API 10.1+). The text
    we send is model output, and model output routinely *quotes*
    untrusted content — a fetched web page, an email body, a file the
    agent read. Without this pass, quoted content could render itself
    inline buttons::

        <tg-button type="callback_data" data="anything">Click me</tg-button>

    A tap then arrives back at the adapter as a ButtonCallback event with
    an attacker-chosen payload. Interactive buttons must stay an explicit
    ``ChannelContent::Interactive`` feature, never a side effect of
    formatting text.

    Code spans and fenced blocks are copied verbatim: Markdown does not
    interpret HTML there, so it is already inert, and escaping it would
    surface a literal ``&lt;`` inside the user's code sample."""
    out = []
    i = 0
    at_line_start = True
    n = len(text)

    while i < n:
        # Fenced code block: copy verbatim, including the fence lines.
        if at_line_start:
            fence = _fence_at(text, i)
            if fence is not None:
                marker, run, _ = fence
                pos = _line_end(text, i)
                out.append(text[i:pos])
                while pos < n:
                    end = _line_end(text, pos)
                    out.append(text[pos:end])
                    # A closing fence carries no info string — ```js inside a
                    # block is content, not a close.
                    closing = _fence_at(text, pos)
                    if (closing is not None and closing[0] == marker
                            and closing[1] >= run and not closing[2]):
                        pos = end
                        break
                    pos = end
                i = pos
                at_line_start = True
                continue

        ch = text[i]

        # A backslash escape makes the next punctuation character literal,
        # so `` \` `` does not open a code span. Copying both characters
        # here keeps the escaped backtick out of the span scan below, which
        # would otherwise pair it with a later one and copy everything
        # between them verbatim.
        if (ch == "\\" and i + 1 < n
                and text[i + 1] in "!\"#$%&'()*+,-./:;<=>?@[\\]^_`{|}~"):
            out.append(text[i:i + 2])
            at_line_start = False
            i += 2
            continue

        # Inline code span: copy verbatim so `<tg-button>` in a code
        # sample stays readable. A run of N backticks closes on the next
        # run of exactly N; an unclosed run is not a code span at all.
        #
        # The scan stops at any line that starts a new block. Markdown
        # resolves block structure *before* inline structure, so a
        # backtick in one block can never pair with one in another —
        # and pairing them here would copy everything between the two
        # verbatim, handing an attacker a way to smuggle a live
        # <tg-button> past this pass by planting a stray backtick on
        # either side of it. Stopping early is the safe direction.
        if ch == "`":
            run = 0
            while i + run < n and text[i + run] == "`":
                run += 1
            j = i + run
            closed = -1
            while j < n:
                if text[j] == "`":
                    close = 0
                    while j + close < n and text[j + close] == "`":
                        close += 1
                    if close == run:
                        closed = j + close
                        break
                    j += close
                    continue
                if (_is_line_break(text[j])
                        and _starts_new_block(text, _next_line_start(text, j))):
                    break
                j += 1
            if closed != -1:
                out.append(text[i:closed])
                i = closed
            else:
                out.append(text[i:i + run])
                i += run
            at_line_start = False
            continue

        # `![...](...)` is a real media block in Rich Markdown, fetched
        # from the URL — today it is inert text. Escape the `!` so the
        # link (if any) renders the way the HTML pipeline renders it,
        # without attaching media.
        if ch == "!" and i + 1 < n and text[i + 1] == "[":
            out.append("\\!")
            at_line_start = False
            i += 1
            continue

        # A Markdown link whose destination carries a scheme we do not
        # allow is escaped whole, so `[x](javascript:...)` stays literal
        # text. `sanitize_telegram_html` drops such links on the legacy
        # path; without this the rich path would be the weaker of the two.
        if ch == "[":
            # Inline form `[label](destination)`, and — at the start of a
            # line — the reference definition `[label]: destination`, which
            # resolves `[x][label]` elsewhere in the message. Escaping the
            # `[` breaks the definition, so the reference no longer resolves.
            dest = _link_destination_at(text, i)
            if dest is None and at_line_start:
                dest = _reference_definition_at(text, i)
            if dest is not None and not _scheme_is_allowed(dest):
                out.append("\\[")
                at_line_start = False
                i += 1
                continue

        if ch == "<":
            name = _tag_name_at(text, i)
            if name == "a":
                # <a> is allowed only when its href scheme is: Telegram does
                # not filter schemes for us.
                allowed = _anchor_href_allowed(text, i)
            else:
                allowed = name in _ALLOWED_RICH_TAGS
            if allowed:
                # Copy the whole tag, not just the `<`. Attribute values are
                # not Markdown: a backtick inside one (`<a title="`" ...>`)
                # would otherwise be read as opening a code span and copy the
                # rest of the line verbatim.
                end = _tag_end(text, i)
                out.append(text[i:end])
                i = end
            else:
                out.append("&lt;")
                i += 1
            at_line_start = False
            continue

        out.append(ch)
        at_line_start = _is_line_break(ch)
        i += 1

    return "".join(out)


def _is_api_rejection(resp: dict) -> bool:
    """True when Telegram answered with a definitive refusal, i.e. a 4xx.

    Everything else leaves the outcome unknown and must not be retried with
    different content: a 5xx can be returned after the message was already
    created, and re-sending then delivers the same answer twice. (Transport
    failures raise out of ``_api_post`` rather than reaching here, which has
    the same effect — no second send.) Mirrors the Rust
    ``dispatcher::is_api_rejection``."""
    code = resp.get("_http")
    if not isinstance(code, int):
        # Telegram also reports failures with HTTP 200 and ``ok: false``;
        # ``_api_post`` returns that body verbatim, so the verdict is in
        # ``error_code``. Rust's ``call_json`` builds ``Error::Api`` from the
        # same field. A 200 with ``ok: false`` and no ``error_code`` at all is
        # still a verdict: Telegram answered, in JSON, that it did not
        # create the message, so it counts as definitive. Reading it as
        # "unknown"
        # would silently disable the fallback for any Bot API deployment that
        # reports failures with a 200, which is exactly the self-hosted
        # pre-10.1 server the fallback exists for.
        if resp.get("ok") is False:
            code = resp.get("error_code")
            if not isinstance(code, int):
                return True
        else:
            code = None
    # 429 is the one 4xx that means "try later", not "not like this". Treating
    # it as a refusal re-sends the same answer into a chat Telegram has just
    # asked us to back off from.
    return isinstance(code, int) and code != 429 and 400 <= code < 500


def _prepare_rich_markdown(text: str):
    """Sanitised text for ``sendRichMessage``, or None when it exceeds the
    rich limit and the caller should fall back to the chunking pipeline."""
    sanitized = sanitize_rich_markdown(text)
    return sanitized if len(sanitized) <= RICH_MSG_LIMIT else None


# ====================================================================
# Reaction emoji map  (port of telegram.rs map_reaction_emoji)
# ====================================================================

_REACTION_MAP = {
    "⏳": "\U0001F440",          # ⏳ → 👀
    "⚙️": "⚡",        # ⚙️ → ⚡
    "✅": "\U0001F389",          # ✅ → 🎉
    "❌": "\U0001F44E",          # ❌ → 👎
}
_DONE_EMOJI = "✅"


def _map_reaction(emoji: str) -> str:
    return _REACTION_MAP.get(emoji, emoji)


_IMAGE_EXT_MIME = [
    ((".jpg", ".jpeg"), "image/jpeg"),
    ((".png",), "image/png"),
    ((".gif",), "image/gif"),
    ((".webp",), "image/webp"),
    ((".bmp",), "image/bmp"),
    ((".tiff", ".tif"), "image/tiff"),
]


def _mime_type_from_telegram_path(url_or_path: str):
    low = url_or_path.lower()
    for exts, mime in _IMAGE_EXT_MIME:
        if any(low.endswith(e) for e in exts):
            return mime
    return None


def _is_private_url(url_str: str) -> bool:
    try:
        p = urllib.parse.urlparse(url_str)
        host = p.hostname
    except ValueError:
        return False
    if not host:
        return False
    if host.lower() == "localhost":
        return True
    try:
        import ipaddress
        ip = ipaddress.ip_address(host)
        return (
            ip.is_loopback or ip.is_private or ip.is_link_local
        )
    except ValueError:
        return False


def _url_filename(url_str: str, fallback: str) -> str:
    try:
        path = urllib.parse.urlparse(url_str).path
        seg = path.rsplit("/", 1)[-1]
        return seg if seg else fallback
    except ValueError:
        return fallback


def _is_telegram_voice_payload(mime_type: str, filename: str) -> bool:
    m = (mime_type or "").strip().lower()
    if m in ("audio/ogg", "audio/opus"):
        return True
    f = filename.lower()
    return f.endswith(".ogg") or f.endswith(".oga") or f.endswith(".opus")


def _is_ogg_opus(data: bytes) -> bool:
    return len(data) >= 36 and data[28:36] == b"OpusHead"


def _extract_retry_after(body, default: int) -> int:
    """Resolve a 429 delay: HTTP delta-seconds ``Retry-After`` header first (stashed by ``_api_post`` / ``_multipart`` as ``_retry_after_header``), then Telegram's JSON ``parameters.retry_after``, then ``default``.
    Mirrors ``telegram.rs``'s ``resolve_retry_after``: a header that isn't a bare non-negative integer (HTTP-date form, a decimal, a stray sign) falls through to the JSON value instead of raising or producing a negative delay."""
    if isinstance(body, dict):
        header = body.get("_retry_after_header")
        if isinstance(header, str):
            stripped = header.strip()
            if stripped and stripped.isascii() and stripped.isdigit():
                return int(stripped)
    try:
        v = body if isinstance(body, dict) else json.loads(body)
        ra = v.get("parameters", {}).get("retry_after")
        return int(ra) if ra is not None else default
    except (ValueError, AttributeError, TypeError):
        return default


# ====================================================================
# HTTP
# ====================================================================


def _api_get(url: str, params: dict, timeout: float) -> dict:
    full = f"{url}?{urllib.parse.urlencode(params)}"
    try:
        with urllib.request.urlopen(full, timeout=timeout) as resp:  # noqa: S310
            return json.loads(resp.read().decode("utf-8", "replace"))
    except urllib.error.HTTPError as e:
        body = e.read().decode("utf-8", "replace")
        try:
            return json.loads(body)
        except ValueError:
            return {"ok": False, "error": f"HTTP {e.code}: {body}"}
    except urllib.error.URLError as e:
        if isinstance(e.reason, (TimeoutError, socket.timeout)):
            raise TimeoutError(str(e.reason)) from e
        raise


def _api_post(url: str, payload: dict, timeout: float) -> dict:
    """POST a JSON body. Returns ``{"_http": code, ...}`` on an HTTP error instead of raising, so callers can implement Telegram's documented 400/429 recovery paths exactly like the Rust adapter.
    Also stashes any ``Retry-After`` response header as ``_retry_after_header`` (on both the 2xx-but-``ok:false`` path and the non-2xx path) so ``_extract_retry_after`` can honour it ahead of the JSON body's ``parameters.retry_after``, matching ``telegram.rs``'s header-first resolution."""
    req = urllib.request.Request(
        url,
        data=json.dumps(payload).encode("utf-8"),
        headers={"Content-Type": "application/json"},
        method="POST",
    )
    try:
        with urllib.request.urlopen(req, timeout=timeout) as resp:  # noqa: S310
            parsed = json.loads(resp.read().decode("utf-8", "replace"))
            retry_after_header = resp.headers.get("Retry-After")
            if retry_after_header is not None:
                parsed["_retry_after_header"] = retry_after_header
            return parsed
    except urllib.error.HTTPError as e:
        body = e.read().decode("utf-8", "replace")
        try:
            parsed = json.loads(body)
        except ValueError:
            parsed = {"ok": False, "description": body}
        parsed["_http"] = e.code
        retry_after_header = e.headers.get("Retry-After")
        if retry_after_header is not None:
            parsed["_retry_after_header"] = retry_after_header
        return parsed


def _multipart(url: str, fields: dict, file_field: str, filename: str,
                mime: str, data: bytes, timeout: float) -> dict:
    boundary = "----librefang" + uuid.uuid4().hex
    pre = []
    for k, v in fields.items():
        pre.append(f"--{boundary}\r\n")
        pre.append(f'Content-Disposition: form-data; name="{k}"\r\n\r\n')
        pre.append(f"{v}\r\n")
    head = "".join(pre).encode("utf-8")
    fhdr = (
        f"--{boundary}\r\n"
        f'Content-Disposition: form-data; name="{file_field}"; '
        f'filename="{filename}"\r\n'
        f"Content-Type: {mime}\r\n\r\n"
    ).encode("utf-8")
    tail = f"\r\n--{boundary}--\r\n".encode("utf-8")
    body = head + fhdr + data + tail
    req = urllib.request.Request(
        url, data=body, method="POST",
        headers={"Content-Type": f"multipart/form-data; boundary={boundary}"},
    )
    try:
        with urllib.request.urlopen(req, timeout=timeout) as resp:  # noqa: S310
            parsed = json.loads(resp.read().decode("utf-8", "replace"))
            retry_after_header = resp.headers.get("Retry-After")
            if retry_after_header is not None:
                parsed["_retry_after_header"] = retry_after_header
            return parsed
    except urllib.error.HTTPError as e:
        b = e.read().decode("utf-8", "replace")
        try:
            parsed = json.loads(b)
        except ValueError:
            parsed = {"ok": False, "description": b}
        parsed["_http"] = e.code
        retry_after_header = e.headers.get("Retry-After")
        if retry_after_header is not None:
            parsed["_retry_after_header"] = retry_after_header
        return parsed


class TelegramAdapter(SidecarAdapter):
    capabilities = ["typing", "reaction", "interactive", "thread", "streaming"]

    SCHEMA = Schema(
        name="telegram",
        display_name="Telegram",
        description="Telegram Bot API adapter (out-of-process sidecar)",
        fields=[
            Field("TELEGRAM_BOT_TOKEN", "Bot Token", "secret",
                  required=True,
                  placeholder="123456:ABC-DEF1234ghIkl-zyx57W2v1u123ew11"),
            Field("ALLOWED_USERS", "Allowed User IDs", "list",
                  placeholder=(
                      "123456789, 987654321 — leave empty to allow ALL users "
                      "(insecure)"
                  ),
                  advanced=True),
            Field("TELEGRAM_CLEAR_DONE_REACTION", "Clear done reaction",
                  "bool", advanced=True),
        ],
    )

    def __init__(self) -> None:
        self.token = os.environ.get("TELEGRAM_BOT_TOKEN", "").strip()
        raw = os.environ.get("ALLOWED_USERS", "").strip()
        self.allowed = [u.strip() for u in raw.split(",") if u.strip()]
        self.clear_done = os.environ.get(
            "TELEGRAM_CLEAR_DONE_REACTION", ""
        ).strip().lower() in ("1", "true", "yes")
        if not self.token:
            log.error("TELEGRAM_BOT_TOKEN is required; exiting")
            raise SystemExit(2)
        self.api_root = "https://api.telegram.org"
        self.api_base = f"{self.api_root}/bot{self.token}"
        self._streams: dict = {}

    # ---- low-level API ------------------------------------------------

    def _call(self, method: str, payload: dict) -> dict:
        return _api_post(
            f"{self.api_base}/{method}", payload, SEND_TIMEOUT_SECS
        )

    def _call_retrying(self, method: str, payload: dict) -> dict:
        """`_call` + a single 429 retry honouring `retry_after` (mirrors api_send_media_request).
        A flood-wait above `MAX_RETRY_AFTER_SECS` skips the sleep and returns the original 429 response instead of stalling the caller indefinitely."""
        resp = self._call(method, payload)
        if resp.get("_http") == 429:
            delay = _extract_retry_after(resp, RETRY_AFTER_DEFAULT_SECS)
            if delay > MAX_RETRY_AFTER_SECS:
                log.warn("telegram rate limited; retry_after exceeds cap, "
                         "not retrying",
                         method=method, delay=delay)
                return resp
            log.warn("telegram rate limited; retrying",
                     method=method, delay=delay)
            time.sleep(delay)
            resp = self._call(method, payload)
        return resp

    def _get_file_url(self, file_id: str):
        resp = self._call("getFile", {"file_id": file_id})
        if resp.get("ok") is not True:
            return None
        fp = (resp.get("result") or {}).get("file_path")
        if not fp:
            return None
        return f"{self.api_root}/file/bot{self.token}/{fp}"

    # ---- outbound text (rich Markdown, HTML pipeline as fallback) ----

    def _send_text(self, chat_id, text: str, thread_id=None) -> dict:
        """Send outbound text.

        Prefers ``sendRichMessage`` (Bot API 10.1+), which hands the text
        to Telegram's own GFM-compatible parser. That gets us tables,
        ``_italic_``, ``~~strikethrough~~`` and nested emphasis — none of
        which ``markdown_to_telegram_html`` can express — and raises the
        size limit from 4096 to 32768, so ordinary replies stop being
        split mid-sentence. The text is sanitised first so quoted
        untrusted content cannot inject interactive elements.

        A definitive refusal by Telegram (4xx — e.g. ``sendRichMessage``
        missing on a self-hosted Bot API server older than 10.1) falls
        back to the legacy HTML pipeline. A 5xx or a transport failure
        does *not*: Telegram may have created the message already, and
        re-sending the same answer would deliver it twice."""
        resp = self._send_rich(chat_id, text, thread_id)
        if resp is not None:
            if resp.get("ok") is True:
                return resp
            if not _is_api_rejection(resp):
                # Outcome unknown — do not re-send the same answer.
                return resp
        responses = self._send_text_chunks(chat_id, text, thread_id)
        return responses[0] if responses else {}

    def _send_rich(self, chat_id, text: str, thread_id=None):
        """Raw ``sendRichMessage`` response for `text`, or None when the
        text is over the rich limit and the rich path cannot be used at
        all. Callers decide what a non-``ok`` response means — see
        ``_is_api_rejection``."""
        markdown = _prepare_rich_markdown(text)
        if markdown is None:
            return None
        payload = {"chat_id": chat_id, "rich_message": {"markdown": markdown}}
        if thread_id:
            payload["message_thread_id"] = thread_id
        return self._call_retrying("sendRichMessage", payload)

    def _edit_rich(self, chat_id, message_id, text: str) -> bool:
        """``editMessageText(rich_message=...)`` for `text`. True when the
        caller must NOT fall back to the legacy HTML edit — either the
        message now shows `text` (``message is not modified`` counts), or
        the outcome is unknown (5xx / transport), where a second edit with
        different content could overwrite one that did land."""
        markdown = _prepare_rich_markdown(text)
        if markdown is None:
            return False
        resp = self._call("editMessageText", {
            "chat_id": chat_id,
            "message_id": message_id,
            "rich_message": {"markdown": markdown},
        })
        if resp.get("ok") is True:
            return True
        if "message is not modified" in str(resp.get("description", "")):
            return True
        return not _is_api_rejection(resp)

    def _send_text_chunks(self, chat_id, text: str, thread_id=None) -> list:
        sanitized = _format_and_sanitize(text)
        responses = []
        for chunk in _split_to_utf16_chunks(sanitized, TELEGRAM_MSG_LIMIT):
            responses.append(
                self._send_formatted_chunk(chat_id, chunk, thread_id),
            )
        return responses

    def _send_formatted_chunk(self, chat_id, chunk: str, thread_id=None) -> dict:
        payload = {
            "chat_id": chat_id,
            "text": chunk,
            "parse_mode": PARSE_MODE_HTML,
        }
        if thread_id:
            payload["message_thread_id"] = thread_id
        resp = self._call_retrying("sendMessage", payload)
        if (resp.get("_http") == 400
                and "can't parse entities" in str(resp.get("description", ""))):
            plain = {"chat_id": chat_id, "text": chunk}
            if thread_id:
                plain["message_thread_id"] = thread_id
            resp = self._call("sendMessage", plain)
        return resp

    def _edit_formatted_chunk(
        self, chat_id, message_id, sanitized: str, plain_fallback: str,
    ) -> None:
        resp = self._call("editMessageText", {
            "chat_id": chat_id,
            "message_id": message_id,
            "text": sanitized,
            "parse_mode": PARSE_MODE_HTML,
        })
        desc = str(resp.get("description", ""))
        if resp.get("_http") and "message is not modified" not in desc:
            if resp.get("_http") == 400 and "can't parse entities" in desc:
                self._call("editMessageText", {
                    "chat_id": chat_id, "message_id": message_id,
                    "text": plain_fallback,
                })

    def _send_media_request(self, endpoint: str, chat_id, body: dict,
                            thread_id=None) -> dict:
        body = dict(body)
        body["chat_id"] = chat_id
        if thread_id:
            body["message_thread_id"] = thread_id
        return self._call_retrying(endpoint, body)

    def _send_media_upload(self, endpoint: str, field: str, chat_id,
                           data: bytes, filename: str, mime: str,
                           extra: dict = None, thread_id=None) -> dict:
        fields = {"chat_id": str(chat_id)}
        if thread_id:
            fields["message_thread_id"] = str(thread_id)
        if extra:
            fields.update({k: str(v) for k, v in extra.items()})
        url = f"{self.api_base}/{endpoint}"
        resp = _multipart(url, fields, field, filename, mime, data,
                          SEND_TIMEOUT_SECS)
        if resp.get("_http") == 429:
            delay = _extract_retry_after(resp, RETRY_AFTER_DEFAULT_SECS)
            if delay > MAX_RETRY_AFTER_SECS:
                log.warn("telegram rate limited; retry_after exceeds cap, "
                         "not retrying",
                         method=endpoint, delay=delay)
                return resp
            time.sleep(delay)
            resp = _multipart(url, fields, field, filename, mime, data,
                              SEND_TIMEOUT_SECS)
        return resp

    def _fetch_bytes(self, url: str):
        req = urllib.request.Request(url, method="GET")
        with urllib.request.urlopen(req, timeout=SEND_TIMEOUT_SECS) as r:  # noqa: S310
            data = r.read(MAX_UPLOAD_BYTES + 1)
            if len(data) > MAX_UPLOAD_BYTES:
                raise RuntimeError("upload exceeds size cap")
            ct = r.headers.get("Content-Type")
            return data, ct

    # ---- outbound media (one method per Telegram endpoint) ----------

    def _send_photo(self, chat_id, url, caption, thread_id):
        body = {"photo": url}
        if caption:
            body["caption"] = caption
            body["parse_mode"] = PARSE_MODE_HTML
        return self._send_media_request("sendPhoto", chat_id, body, thread_id)

    def _send_document(self, chat_id, url, filename, thread_id):
        if _is_private_url(url):
            data, ct = self._fetch_bytes(url)
            mime = ct or "application/octet-stream"
            return self._send_media_upload(
                "sendDocument", "document", chat_id, data, filename, mime,
                {"caption": filename}, thread_id)
        return self._send_media_request(
            "sendDocument", chat_id,
            {"document": url, "caption": filename}, thread_id)

    def _send_voice(self, chat_id, url, caption, thread_id):
        if _is_private_url(url):
            data, ct = self._fetch_bytes(url)
            extra = {}
            if caption:
                extra = {"caption": caption, "parse_mode": PARSE_MODE_HTML}
            return self._send_media_upload(
                "sendVoice", "voice", chat_id, data,
                _url_filename(url, "voice.ogg"), ct or "audio/ogg",
                extra, thread_id)
        body = {"voice": url}
        if caption:
            body["caption"] = caption
            body["parse_mode"] = PARSE_MODE_HTML
        return self._send_media_request("sendVoice", chat_id, body, thread_id)

    def _send_audio(self, chat_id, url, caption, title, performer, thread_id):
        if _is_private_url(url):
            data, ct = self._fetch_bytes(url)
            extra = {}
            if caption:
                extra["caption"] = caption
                extra["parse_mode"] = PARSE_MODE_HTML
            if title:
                extra["title"] = title
            if performer:
                extra["performer"] = performer
            return self._send_media_upload(
                "sendAudio", "audio", chat_id, data,
                _url_filename(url, "audio.mp3"), ct or "audio/mpeg",
                extra, thread_id)
        body = {"audio": url}
        if caption:
            body["caption"] = caption
            body["parse_mode"] = PARSE_MODE_HTML
        if title:
            body["title"] = title
        if performer:
            body["performer"] = performer
        return self._send_media_request("sendAudio", chat_id, body, thread_id)

    def _send_video(self, chat_id, url, caption, thread_id):
        body = {"video": url}
        if caption:
            body["caption"] = caption
            body["parse_mode"] = PARSE_MODE_HTML
        return self._send_media_request("sendVideo", chat_id, body, thread_id)

    def _send_animation(self, chat_id, url, caption, thread_id):
        body = {"animation": url}
        if caption:
            body["caption"] = caption
            body["parse_mode"] = PARSE_MODE_HTML
        return self._send_media_request(
            "sendAnimation", chat_id, body, thread_id)

    def _send_sticker(self, chat_id, file_id, thread_id):
        return self._send_media_request(
            "sendSticker", chat_id, {"sticker": file_id}, thread_id)

    def _send_location(self, chat_id, lat, lon, thread_id):
        return self._send_media_request(
            "sendLocation", chat_id,
            {"latitude": lat, "longitude": lon}, thread_id)

    def _send_media_group(self, chat_id, items, thread_id):
        if not items:
            return {}
        if not (2 <= len(items) <= 10):
            raise RuntimeError(
                f"Telegram sendMediaGroup requires 2-10 items, got "
                f"{len(items)}")
        media = []
        for it in items:
            if not isinstance(it, dict):
                log.warn(
                    "telegram media group skipping non-object item",
                    item_type=type(it).__name__,
                )
                continue
            if "Photo" in it:
                p = it["Photo"]
                v = {"type": "photo", "media": p.get("url")}
                if p.get("caption"):
                    v["caption"] = p["caption"]
                    v["parse_mode"] = PARSE_MODE_HTML
            elif "Video" in it:
                p = it["Video"]
                v = {"type": "video", "media": p.get("url"),
                     "duration": p.get("duration_seconds", 0)}
                if p.get("caption"):
                    v["caption"] = p["caption"]
                    v["parse_mode"] = PARSE_MODE_HTML
            else:
                log.warn(
                    "telegram media group skipping unsupported item",
                    kinds=list(it) if isinstance(it, dict) else [],
                )
                continue
            media.append(v)
        if len(media) < 2:
            log.warn(
                "telegram media group has fewer than two supported items; "
                "not sending",
                supported_items=len(media),
            )
            return {}
        body = {"chat_id": chat_id, "media": media}
        if thread_id:
            body["message_thread_id"] = thread_id
        return self._call_retrying("sendMediaGroup", body)

    def _send_poll(self, chat_id, question, options, is_quiz,
                   correct_option_id, explanation, thread_id):
        body = {
            "chat_id": chat_id,
            "question": question,
            "options": [{"text": o} for o in options],
            "type": "quiz" if is_quiz else "regular",
        }
        if is_quiz:
            if correct_option_id is not None:
                body["correct_option_id"] = correct_option_id
            if explanation is not None:
                body["explanation"] = explanation
        if thread_id:
            body["message_thread_id"] = thread_id
        resp = self._call_retrying("sendPoll", body)
        return ((resp.get("result") or {}).get("poll") or {}).get("id", "")

    def _inline_keyboard(self, buttons) -> dict:
        rows = []
        for row in buttons or []:
            out = []
            for b in row:
                if b.get("url"):
                    out.append({"text": b.get("label", ""), "url": b["url"]})
                else:
                    out.append({
                        "text": b.get("label", ""),
                        "callback_data": _truncate_utf8(
                            b.get("action", ""), 64),
                    })
            rows.append(out)
        return {"inline_keyboard": rows}

    def _send_interactive(self, chat_id, text, buttons, thread_id):
        body = {
            "chat_id": chat_id,
            "text": _format_and_sanitize(text),
            "parse_mode": PARSE_MODE_HTML,
            "reply_markup": self._inline_keyboard(buttons),
        }
        if thread_id:
            body["message_thread_id"] = thread_id
        return self._call("sendMessage", body)

    def _edit_interactive(self, chat_id, message_id, text, buttons):
        kb = self._inline_keyboard(buttons)
        resp = self._call("editMessageText", {
            "chat_id": chat_id, "message_id": int(message_id),
            "text": _format_and_sanitize(text),
            "parse_mode": PARSE_MODE_HTML,
            "reply_markup": kb,
        })
        desc = str(resp.get("description", ""))
        if (resp.get("_http") == 400 and "can't parse entities" in desc):
            self._call("editMessageText", {
                "chat_id": chat_id, "message_id": int(message_id),
                "text": text, "reply_markup": kb,
            })

    def _delete_message(self, chat_id, message_id):
        return self._call("deleteMessage", {
            "chat_id": chat_id, "message_id": int(message_id)})

    # ---- outbound ChannelContent dispatch (all variants) ------------

    def _dispatch_content(self, chat_id, content: dict, thread_id) -> None:
        kind, body = next(iter(content.items()))
        if kind == "Text":
            self._send_text(chat_id, body, thread_id)
        elif kind == "Image":
            self._send_photo(chat_id, body["url"], body.get("caption"),
                             thread_id)
        elif kind == "File":
            fn = body.get("filename", "document")
            if _is_telegram_voice_payload("", fn):
                self._send_voice(chat_id, body["url"], None, thread_id)
            else:
                self._send_document(chat_id, body["url"], fn, thread_id)
        elif kind == "FileData":
            self._dispatch_filedata(chat_id, body, thread_id)
        elif kind == "Voice":
            self._send_voice(chat_id, body["url"], body.get("caption"),
                             thread_id)
        elif kind == "Video":
            self._send_video(chat_id, body["url"], body.get("caption"),
                             thread_id)
        elif kind == "Location":
            self._send_location(chat_id, body["lat"], body["lon"], thread_id)
        elif kind == "Command":
            txt = f"/{body['name']} {' '.join(body.get('args', []))}".strip()
            self._send_text(chat_id, txt, thread_id)
        elif kind == "Interactive":
            self._send_interactive(chat_id, body["text"],
                                   body.get("buttons", []), thread_id)
        elif kind == "ButtonCallback":
            pass  # outbound ButtonCallback is meaningless — skip (Rust does)
        elif kind == "EditInteractive":
            self._edit_interactive(chat_id, body["message_id"], body["text"],
                                   body.get("buttons", []))
        elif kind == "DeleteMessage":
            self._delete_message(chat_id, body["message_id"])
        elif kind == "Audio":
            self._send_audio(chat_id, body["url"], body.get("caption"),
                             body.get("title"), body.get("performer"),
                             thread_id)
        elif kind == "Animation":
            self._send_animation(chat_id, body["url"], body.get("caption"),
                                 thread_id)
        elif kind == "Sticker":
            self._send_sticker(chat_id, body["file_id"], thread_id)
        elif kind == "MediaGroup":
            self._send_media_group(chat_id, body.get("items", []), thread_id)
        elif kind == "Poll":
            self._send_poll(chat_id, body["question"], body.get("options", []),
                            body.get("is_quiz", False),
                            body.get("correct_option_id"),
                            body.get("explanation"), thread_id)
        elif kind == "PollAnswer":
            pass  # outbound PollAnswer is meaningless — skip (Rust does)
        else:
            self._send_text(chat_id, "(Unsupported content type)", thread_id)

    def _dispatch_filedata(self, chat_id, body, thread_id):
        data = bytes(body.get("data", []))
        fn = body.get("filename", "file")
        mime = body.get("mime_type", "application/octet-stream")
        sniff = data[:36]
        if (_is_telegram_voice_payload(mime, fn)
                and sniff[:4] == b"OggS" and _is_ogg_opus(sniff)):
            self._send_media_upload("sendVoice", "voice", chat_id, data, fn,
                                    mime, None, thread_id)
        else:
            self._send_media_upload("sendDocument", "document", chat_id, data,
                                    fn, mime, None, thread_id)

    # ---- inbound -----------------------------------------------------

    def _allowed(self, user_id: str, username) -> bool:
        if not self.allowed:
            return True
        if user_id in self.allowed:
            return True
        if username:
            norm = username.lstrip("@").lower()
            return any(a.lstrip("@").lower() == norm for a in self.allowed)
        return False

    def _sender(self, message: dict):
        frm = message.get("from")
        if frm is not None:
            uid = frm.get("id")
            if not isinstance(uid, int):
                return None
            first = frm.get("first_name") or "Unknown"
            last = frm.get("last_name") or ""
            name = first if not last else f"{first} {last}"
            return str(uid), name, frm.get("username")
        sc = message.get("sender_chat")
        if sc is not None:
            uid = sc.get("id")
            if not isinstance(uid, int):
                return None
            return str(uid), sc.get("title") or "Unknown Channel", None
        return None

    def _extract_content(self, message: dict):
        txt = message.get("text")
        if isinstance(txt, str):
            ents = message.get("entities")
            if isinstance(ents, list) and any(
                e.get("type") == "bot_command" and e.get("offset") == 0
                for e in ents
            ):
                parts = txt.split(" ", 1)
                name = parts[0].lstrip("/").split("@")[0]
                args = parts[1].split() if len(parts) > 1 else []
                return protocol.Content.command(name, args)
            return protocol.Content.text(txt)

        photos = message.get("photo")
        if isinstance(photos, list) and photos:
            fid = photos[-1].get("file_id", "")
            cap = message.get("caption")
            url = self._get_file_url(fid)
            if url:
                return protocol.Content.image(
                    url, cap, _mime_type_from_telegram_path(url))
            return protocol.Content.text(
                f"[Photo received{f': {cap}' if cap else ''}]")

        if "document" in message:
            d = message["document"]
            fn = d.get("file_name") or "document"
            url = self._get_file_url(d.get("file_id", ""))
            return (protocol.Content.file(url, fn) if url
                    else protocol.Content.text(f"[Document received: {fn}]"))

        if "audio" in message:
            a = message["audio"]
            dur = a.get("duration", 0) or 0
            cap = message.get("caption")
            url = self._get_file_url(a.get("file_id", ""))
            if url:
                return protocol.Content.audio(
                    url, cap, dur, a.get("title"), a.get("performer"))
            return protocol.Content.text(
                f"[Audio received, {dur}s{f': {cap}' if cap else ''}]")

        if "voice" in message:
            v = message["voice"]
            dur = v.get("duration", 0) or 0
            url = self._get_file_url(v.get("file_id", ""))
            if url:
                return protocol.Content.voice(url, message.get("caption"), dur)
            return protocol.Content.text(f"[Voice message, {dur}s]")

        if "animation" in message:
            an = message["animation"]
            dur = an.get("duration", 0) or 0
            cap = message.get("caption")
            url = self._get_file_url(an.get("file_id", ""))
            if url:
                return protocol.Content.animation(url, cap, dur)
            return protocol.Content.text(
                f"[Animation received, {dur}s{f': {cap}' if cap else ''}]")

        if "video" in message:
            vd = message["video"]
            dur = vd.get("duration", 0) or 0
            cap = message.get("caption")
            url = self._get_file_url(vd.get("file_id", ""))
            if url:
                return protocol.Content.video(
                    url, cap, dur, vd.get("file_name"))
            return protocol.Content.text(
                f"[Video received, {dur}s{f': {cap}' if cap else ''}]")

        if "video_note" in message:
            vn = message["video_note"]
            dur = vn.get("duration", 0) or 0
            url = self._get_file_url(vn.get("file_id", ""))
            if url:
                return protocol.Content.video(url, None, dur, None)
            return protocol.Content.text(f"[Video note, {dur}s]")

        if "location" in message:
            loc = message["location"]
            return protocol.Content.location(
                loc.get("latitude", 0.0), loc.get("longitude", 0.0))

        if "sticker" in message:
            fid = message["sticker"].get("file_id", "")
            return protocol.Content.sticker(fid) if fid else None

        return None

    def _apply_reply(self, content, message: dict):
        reply = message.get("reply_to_message")
        if not reply:
            return content
        sender = (reply.get("from") or {}).get("first_name") or "Someone"
        rtext = reply.get("text") or reply.get("caption")
        rphotos = reply.get("photo")
        rphoto_url = None
        if isinstance(rphotos, list) and rphotos:
            fid = rphotos[-1].get("file_id", "")
            if fid:
                rphoto_url = self._get_file_url(fid)

        if rphoto_url:
            if rtext:
                qc = (f'[Replying to {sender}: '
                      f'"{_truncate_with_ellipsis(rtext, 200)}"]\n')
            else:
                qc = f"[Replying to {sender}'s photo]\n"
            if "Image" in content:
                img = dict(content["Image"])
                img["caption"] = qc + (img.get("caption") or "")
                return {"Image": img}
            if "Text" in content:
                return protocol.Content.image(
                    rphoto_url, qc + content["Text"],
                    _mime_type_from_telegram_path(rphoto_url))
            return content
        if rtext:
            prefix = (f'[Replying to {sender}: '
                      f'"{_truncate_with_ellipsis(rtext, 200)}"]\n')
            if "Text" in content:
                return protocol.Content.text(prefix + content["Text"])
        return content

    def _callback_to_event(self, callback: dict):
        cqid = callback.get("id")
        frm = callback.get("from")
        if not cqid or not frm:
            return None
        uid = frm.get("id")
        if not isinstance(uid, int):
            return None
        username = frm.get("username")
        if not self._allowed(str(uid), username):
            return None
        data = callback.get("data") or ""
        if not data:
            return None
        message = callback.get("message")
        if not message:
            return None
        chat_id = message.get("chat", {}).get("id")
        if not isinstance(chat_id, int):
            return None
        msg_id = message.get("message_id", 0)
        first = frm.get("first_name") or "Unknown"
        last = frm.get("last_name") or ""
        name = first if not last else f"{first} {last}"
        # Fire-and-forget answerCallbackQuery to dismiss the spinner.
        try:
            self._call("answerCallbackQuery", {"callback_query_id": cqid})
        except Exception:  # noqa: BLE001
            pass
        chat_type = message.get("chat", {}).get("type", "private")
        thread = message.get("message_thread_id")
        return protocol.message(
            user_id=str(uid),
            user_name=name,
            content=protocol.Content.button_callback(
                data, message.get("text")),
            message_id=str(msg_id),
            channel_id=str(chat_id),
            username=username,
            is_group=chat_type in ("group", "supergroup"),
            thread_id=str(thread) if thread is not None else None,
            metadata={"callback_query_id": cqid},
        )

    def _poll_answer_to_event(self, poll_answer: dict):
        # Mirrors in-process telegram.rs:2129-2230. Telegram only fires
        # `poll_answer` for non-anonymous polls in private chats, so
        # `user.id` doubles as the DM chat_id — see the Rust comment
        # near `SENDER_USER_ID_KEY` on line 2161.
        poll_id = poll_answer.get("poll_id")
        if not poll_id:
            return None
        user = poll_answer.get("user") or {}
        uid = user.get("id")
        if not isinstance(uid, int):
            return None
        username = user.get("username")
        if not self._allowed(str(uid), username):
            return None
        first = user.get("first_name") or "Unknown"
        last = user.get("last_name") or ""
        name = first if not last else f"{first} {last}"
        # Rust coerces each id to `u8` (Telegram uses 0-based option
        # indices, max 9 per poll); non-int / negative entries are
        # silently dropped, matching `filter_map(.. as u64 .. as u8)`.
        option_ids = [
            int(o) for o in (poll_answer.get("option_ids") or [])
            if isinstance(o, int) and 0 <= o <= 255
        ]
        return protocol.message(
            user_id=str(uid),
            user_name=name,
            content=protocol.Content.poll_answer(poll_id, option_ids),
            message_id=poll_id,
            channel_id=str(uid),
            username=username,
            is_group=False,
            metadata={"user_id": str(uid), "sender_user_id": str(uid)},
        )

    @staticmethod
    def _extract_mentions(message: dict) -> list:
        """`@`-mention handles in this message (#5323).

        Telegram surfaces ``@username`` tokens as ``mention`` entities and
        username-less user references as ``text_mention``. Entity offsets are
        UTF-16 code-unit indices, so slice the text in UTF-16 to stay correct
        when emoji or other astral characters precede the mention. The bridge
        resolves these against agent names/handles to route a group message to
        a specific non-default agent.
        """
        txt = message.get("text")
        ents = message.get("entities")
        if not isinstance(txt, str):
            txt = message.get("caption")
            ents = message.get("caption_entities")
        if not isinstance(txt, str) or not isinstance(ents, list):
            return []
        utf16 = txt.encode("utf-16-le")
        names: list = []
        for e in ents:
            etype = e.get("type")
            if etype == "mention":
                off, ln = e.get("offset"), e.get("length")
                if isinstance(off, int) and isinstance(ln, int):
                    frag = utf16[off * 2:(off + ln) * 2].decode(
                        "utf-16-le", "ignore")
                    handle = frag.lstrip("@").strip()
                    if handle and handle not in names:
                        names.append(handle)
            elif etype == "text_mention":
                uname = (e.get("user") or {}).get("username")
                if uname and uname not in names:
                    names.append(uname)
        return names

    def _message_metadata(self, message: dict, user_id: str,
                          is_group: bool):
        """Metadata that lets the bridge route and scope per-conversation.

        ``sender_user_id`` is the individual sender (the group ``channel_id``
        is the chat, not the user), so the conversation-ownership registry can
        key per peer. ``mention_names`` carries any ``@``-mention so a specific
        agent can be addressed in a multi-agent group (#5323). Returns ``None``
        when there is nothing to attach so DMs keep their lean envelope.
        """
        meta: dict = {}
        if is_group:
            meta["sender_user_id"] = user_id
        mentions = self._extract_mentions(message)
        if mentions:
            meta["mention_names"] = mentions
        return meta or None

    def _update_to_event(self, update: dict):
        callback = update.get("callback_query")
        if callback:
            return self._callback_to_event(callback)
        poll_answer = update.get("poll_answer")
        if poll_answer:
            return self._poll_answer_to_event(poll_answer)
        message = update.get("message") or update.get("edited_message")
        if not message:
            return None
        snd = self._sender(message)
        if snd is None:
            return None
        user_id, name, username = snd
        if not self._allowed(user_id, username):
            return None
        chat = message.get("chat", {})
        chat_id = chat.get("id")
        if chat_id is None:
            return None
        content = self._extract_content(message)
        if content is None:
            return None
        content = self._apply_reply(content, message)
        thread = message.get("message_thread_id")
        is_group = chat.get("type") in ("group", "supergroup")
        return protocol.message(
            user_id=user_id,
            user_name=name,
            content=content,
            message_id=str(message.get("message_id", "")),
            channel_id=str(chat_id),
            username=username,
            is_group=is_group,
            thread_id=str(thread) if thread is not None else None,
            platform="telegram",
            metadata=self._message_metadata(message, user_id, is_group),
        )

    def _poll_once(self, emit, state: dict) -> None:
        data = _api_get(
            f"{self.api_base}/getUpdates",
            {"offset": state["offset"], "timeout": LONGPOLL_SERVER_SECS,
             "allowed_updates": json.dumps(
                 ["message", "edited_message", "callback_query",
                  "poll_answer"])},
            LONGPOLL_CLIENT_SECS,
        )
        if not data.get("ok"):
            raise RuntimeError(f"Telegram API error: {data}")
        for update in data.get("result", []):
            state["offset"] = update.get("update_id", state["offset"]) + 1
            ev = self._update_to_event(update)
            if ev:
                emit(ev)

    async def produce(self, emit) -> None:
        # Reconnect / startup-recovery contract (#5111):
        # * The very first iteration is also the startup credential
        #   probe — a DNS resolution failure, TCP RST, or proxy 5xx
        #   raises through `_api_get` → `_poll_once` and is caught
        #   below. We do NOT crash the producer; the daemon supervisor
        #   restart was the pre-migration behaviour the issue
        #   complained about, and we'd just churn the process for the
        #   same transient.
        # * Mid-session: identical handler. A network blip flips
        #   getUpdates from "long-poll returning 200 with []" to
        #   "URLError"; the same backoff applies.
        # * Long-poll timeouts (the LONGPOLL_SERVER_SECS server-side
        #   block expiring with no updates) are normal — they reset
        #   backoff and skip the sleep so we re-enter the loop
        #   immediately.
        # * Successful recovery flips backoff back to 1.0 AND logs an
        #   INFO line if we were in a degraded state, so operators see
        #   the reconnect on their log timeline (the issue's
        #   "restored DNS — bridge does NOT recover" scenario).
        loop = asyncio.get_event_loop()
        state = {"offset": 0}
        backoff = 1.0
        retries_in_a_row = 0
        while True:
            try:
                await loop.run_in_executor(None, self._poll_once, emit, state)
                if retries_in_a_row > 0:
                    log.info("telegram poll recovered",
                             retries=retries_in_a_row,
                             last_backoff=backoff)
                backoff = 1.0
                retries_in_a_row = 0
            except asyncio.CancelledError:
                raise
            except TimeoutError:
                backoff = 1.0
                retries_in_a_row = 0
                continue
            except Exception as e:  # noqa: BLE001 - transport errors vary
                retries_in_a_row += 1
                log.warn("telegram poll error; backing off",
                         error=str(e),
                         delay=backoff,
                         retries=retries_in_a_row)
                await asyncio.sleep(backoff)
                backoff = min(backoff * 2, MAX_BACKOFF_SECS)

    # ---- streaming ---------------------------------------------------

    def _stream_delta(self, sid: str, chunk: str) -> None:
        st = self._streams.get(sid)
        if st is None:
            return
        st["text"] += chunk
        if not st["initial_attempted"]:
            st["initial_attempted"] = True
            self._sync_stream_messages(st)
            st["last_edit"] = time.monotonic()
        elif (st["message_ids"]
              and time.monotonic() - st["last_edit"] >= STREAM_EDIT_INTERVAL):
            self._sync_stream_messages(st)
            st["last_edit"] = time.monotonic()

    def _sync_stream_messages(self, st: dict) -> None:
        # Rich path: while the answer still fits one rich message (32768
        # chars vs 4096), stream it as a single message rather than a
        # chunked HTML one, so tables and nested emphasis render during
        # streaming exactly as they will in the finished reply. Once the
        # answer has already spilled into several messages, stay on the
        # legacy chunked path rather than restructuring mid-stream.
        if len(st["message_ids"]) <= 1:
            if st["message_ids"]:
                if self._edit_rich(st["chat_id"], st["message_ids"][0],
                                   st["text"]):
                    return
            else:
                resp = self._send_rich(st["chat_id"], st["text"],
                                       st["thread_id"])
                if resp is not None:
                    message_id = (resp.get("result") or {}).get("message_id")
                    if message_id is not None:
                        st["message_ids"].append(message_id)
                    if not _is_api_rejection(resp):
                        # Either it worked, or the outcome is unknown (5xx)
                        # and Telegram may have created the message anyway.
                        # Sending the chunked HTML version now would deliver
                        # the answer twice; the next throttled tick retries.
                        return

        sanitized = _format_and_sanitize(st["text"])
        chunks = _split_to_utf16_chunks(sanitized, TELEGRAM_MSG_LIMIT)
        for index, formatted_chunk in enumerate(chunks):
            if index < len(st["message_ids"]):
                self._edit_formatted_chunk(
                    st["chat_id"], st["message_ids"][index],
                    formatted_chunk, formatted_chunk,
                )
                continue
            resp = self._send_formatted_chunk(
                st["chat_id"], formatted_chunk, st["thread_id"],
            )
            message_id = (resp.get("result") or {}).get("message_id")
            if message_id is None:
                break
            st["message_ids"].append(message_id)
        if len(st["message_ids"]) > len(chunks):
            for obsolete_id in st["message_ids"][len(chunks):]:
                self._delete_message(st["chat_id"], obsolete_id)
            del st["message_ids"][len(chunks):]

    def _stream_end(self, sid: str) -> None:
        st = self._streams.pop(sid, None)
        if st is None or not st["text"]:
            return
        if st["message_ids"]:
            self._sync_stream_messages(st)
        else:
            # Retry a failed initial send once at the terminal event, not on
            # every accumulated delta.
            self._send_text(st["chat_id"], st["text"], st["thread_id"])

    # ---- command dispatch -------------------------------------------

    async def on_command(self, cmd) -> None:
        loop = asyncio.get_event_loop()
        if isinstance(cmd, protocol.Send):
            chat_id = cmd.channel_id
            if not chat_id:
                return
            if cmd.content:
                await loop.run_in_executor(
                    None, self._dispatch_content, chat_id, cmd.content,
                    cmd.thread_id)
            elif cmd.text:
                await loop.run_in_executor(
                    None, self._send_text, chat_id, cmd.text, cmd.thread_id)
        elif isinstance(cmd, protocol.TypingCmd):
            await loop.run_in_executor(None, self._call, "sendChatAction",
                                       {"chat_id": cmd.channel_id,
                                        "action": "typing"})
        elif isinstance(cmd, protocol.Reaction):
            await loop.run_in_executor(None, self._do_reaction, cmd)
        elif isinstance(cmd, protocol.Interactive):
            msg = cmd.message or {}
            await loop.run_in_executor(
                None, self._send_interactive, cmd.channel_id,
                msg.get("text", ""), msg.get("buttons", []), None)
        elif isinstance(cmd, protocol.StreamStart):
            self._streams[cmd.stream_id] = {
                "chat_id": cmd.channel_id,
                "thread_id": getattr(cmd, "thread_id", None),
                "text": "", "message_ids": [], "initial_attempted": False,
                "last_edit": 0.0,
            }
        elif isinstance(cmd, protocol.StreamDelta):
            await loop.run_in_executor(
                None, self._stream_delta, cmd.stream_id, cmd.text)
        elif isinstance(cmd, protocol.StreamEnd):
            await loop.run_in_executor(None, self._stream_end, cmd.stream_id)
        else:
            await super().on_command(cmd)

    def _do_reaction(self, cmd) -> None:
        clear = cmd.reaction == _DONE_EMOJI and self.clear_done
        reaction = [] if clear else [
            {"type": "emoji", "emoji": _map_reaction(cmd.reaction)}
        ]
        self._call("setMessageReaction", {
            "chat_id": cmd.channel_id,
            "message_id": int(cmd.message_id),
            "reaction": reaction,
        })

    async def on_send(self, cmd) -> None:
        if not cmd.channel_id:
            return
        if cmd.content:
            await asyncio.get_event_loop().run_in_executor(
                None, self._dispatch_content, cmd.channel_id, cmd.content,
                cmd.thread_id)
        elif cmd.text:
            await asyncio.get_event_loop().run_in_executor(
                None, self._send_text, cmd.channel_id, cmd.text,
                cmd.thread_id)


if __name__ == "__main__":
    run_stdio_main(TelegramAdapter)
