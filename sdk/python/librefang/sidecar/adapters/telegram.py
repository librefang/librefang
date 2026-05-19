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

* DONE — Markdown → Telegram-HTML formatter subsystem: a byte-exact
  port of ``formatter::markdown_to_telegram_html`` + the
  ``sanitize_telegram_html`` security pass (tag/scheme allowlist,
  attribute-injection escaping, unclosed-tag balancing) + the
  ``message_truncator`` UTF-16/HTML-entity-aware chunker
  (``split_to_utf16_chunks``). Outbound text is now formatted and sent
  with ``parse_mode=HTML`` (was ``Markdown``), with the same
  plain-text retry on Telegram's "can't parse entities" 400.
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
import json
import os
import socket
import time
import urllib.error
import urllib.parse
import urllib.request
import uuid

from librefang.sidecar import Field, Schema, SidecarAdapter, protocol, run_stdio
from librefang.sidecar import logging as log

LONGPOLL_SERVER_SECS = 30
LONGPOLL_CLIENT_SECS = 35
SEND_TIMEOUT_SECS = 10
# Telegram's message limit is 4096 *UTF-16 code units* (not chars).
TELEGRAM_MSG_LIMIT = 4096
# Throttle streamed editMessageText (mirrors the Rust adapter's 1s).
STREAM_EDIT_INTERVAL = 1.0
RETRY_AFTER_DEFAULT_SECS = 2
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
        url = result[be + 2:pe]
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
    if self_closing:
        buf += "/"
    buf += ">"
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
                            open_tags.pop(pos)
                            result.append(text[i:tag_end + 1])
                        else:
                            result.append("&lt;")
                            result.append(_escape_html_text(tag_content))
                            result.append("&gt;")
                    else:
                        self_closing = tag_content.endswith("/")
                        attrs_raw = tag_content[len(name_raw):]
                        attrs_raw = attrs_raw.rstrip("/").strip()
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
    """POST a JSON body. Returns ``{"_http": code, ...}`` on an HTTP
    error instead of raising, so callers can implement Telegram's
    documented 400/429 recovery paths exactly like the Rust adapter."""
    req = urllib.request.Request(
        url,
        data=json.dumps(payload).encode("utf-8"),
        headers={"Content-Type": "application/json"},
        method="POST",
    )
    try:
        with urllib.request.urlopen(req, timeout=timeout) as resp:  # noqa: S310
            return json.loads(resp.read().decode("utf-8", "replace"))
    except urllib.error.HTTPError as e:
        body = e.read().decode("utf-8", "replace")
        try:
            parsed = json.loads(body)
        except ValueError:
            parsed = {"ok": False, "description": body}
        parsed["_http"] = e.code
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
            return json.loads(resp.read().decode("utf-8", "replace"))
    except urllib.error.HTTPError as e:
        b = e.read().decode("utf-8", "replace")
        try:
            parsed = json.loads(b)
        except ValueError:
            parsed = {"ok": False, "description": b}
        parsed["_http"] = e.code
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
                  placeholder="123456789, 987654321",
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
        """`_call` + a single 429 retry honouring `retry_after`
        (mirrors api_send_media_request)."""
        resp = self._call(method, payload)
        if resp.get("_http") == 429:
            delay = _extract_retry_after(resp, RETRY_AFTER_DEFAULT_SECS)
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

    # ---- outbound text (formatter + sanitize + chunk + HTML) ---------

    def _send_text(self, chat_id, text: str, thread_id=None) -> dict:
        sanitized = _format_and_sanitize(text)
        last: dict = {}
        for chunk in _split_to_utf16_chunks(sanitized, TELEGRAM_MSG_LIMIT):
            payload = {
                "chat_id": chat_id,
                "text": chunk,
                "parse_mode": PARSE_MODE_HTML,
            }
            if thread_id:
                payload["message_thread_id"] = thread_id
            resp = self._call_retrying("sendMessage", payload)
            if (resp.get("_http") == 400
                    and "can't parse entities" in str(resp.get("description",
                                                               ""))):
                plain = {"chat_id": chat_id, "text": chunk}
                if thread_id:
                    plain["message_thread_id"] = thread_id
                resp = self._call("sendMessage", plain)
            last = resp
        return last

    def _edit_text(self, chat_id, message_id, text: str) -> None:
        sanitized = _format_and_sanitize(text)
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
                    "text": text,
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
            time.sleep(_extract_retry_after(resp, RETRY_AFTER_DEFAULT_SECS))
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
            if "Photo" in it:
                p = it["Photo"]
                v = {"type": "photo", "media": p.get("url")}
                if p.get("caption"):
                    v["caption"] = p["caption"]
                    v["parse_mode"] = PARSE_MODE_HTML
            else:
                p = it["Video"]
                v = {"type": "video", "media": p.get("url"),
                     "duration": p.get("duration_seconds", 0)}
                if p.get("caption"):
                    v["caption"] = p["caption"]
                    v["parse_mode"] = PARSE_MODE_HTML
            media.append(v)
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
            "text": sanitize_telegram_html(text),
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
            "text": sanitize_telegram_html(text),
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
        return protocol.message(
            user_id=user_id,
            user_name=name,
            content=content,
            message_id=str(message.get("message_id", "")),
            channel_id=str(chat_id),
            username=username,
            is_group=chat.get("type") in ("group", "supergroup"),
            thread_id=str(thread) if thread is not None else None,
            platform="telegram",
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
        loop = asyncio.get_event_loop()
        state = {"offset": 0}
        backoff = 1.0
        while True:
            try:
                await loop.run_in_executor(None, self._poll_once, emit, state)
                backoff = 1.0
            except asyncio.CancelledError:
                raise
            except TimeoutError:
                backoff = 1.0
                continue
            except Exception as e:  # noqa: BLE001 - transport errors vary
                log.warn("telegram poll error; backing off",
                         error=str(e), delay=backoff)
                await asyncio.sleep(backoff)
                backoff = min(backoff * 2, 120.0)

    # ---- streaming ---------------------------------------------------

    def _stream_delta(self, sid: str, chunk: str) -> None:
        st = self._streams.get(sid)
        if st is None:
            return
        st["text"] += chunk
        if st["msg_id"] is None:
            resp = self._send_text(st["chat_id"], st["text"], st["thread_id"])
            st["msg_id"] = (resp.get("result") or {}).get("message_id")
            st["last_edit"] = time.monotonic()
        elif time.monotonic() - st["last_edit"] >= STREAM_EDIT_INTERVAL:
            self._edit_text(st["chat_id"], st["msg_id"], st["text"])
            st["last_edit"] = time.monotonic()

    def _stream_end(self, sid: str) -> None:
        st = self._streams.pop(sid, None)
        if st is None or not st["text"]:
            return
        if st["msg_id"] is not None:
            self._edit_text(st["chat_id"], st["msg_id"], st["text"])
        else:
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
                "text": "", "msg_id": None, "last_edit": 0.0,
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
    import sys as _sys
    if "--describe" in _sys.argv[1:]:
        # Describe-only path: don't instantiate (avoids env-required side
        # effects like the TELEGRAM_BOT_TOKEN check in __init__).
        from librefang.sidecar import describe_main
        raise SystemExit(describe_main(TelegramAdapter))
    run_stdio(TelegramAdapter())
