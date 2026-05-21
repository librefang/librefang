#!/usr/bin/env python3
"""Mastodon Streaming API sidecar channel adapter for LibreFang.

Replaces the former in-process Rust ``librefang-channels::mastodon``
adapter (removed in this sidecar migration; same pattern as ntfy
#5224, telegram #5241, gotify #5263). Behaviour is preserved:

* Inbound (primary): SSE subscribe to
  ``{instance}/api/v1/streaming/user``. Parse
  ``event: notification`` / ``data: <json>`` pairs; filter to
  ``type == "mention"``; strip HTML from ``status.content``;
  ``/cmd args`` → Command, else Text; sender = display_name
  fallback username; metadata carries status_id, notification_id,
  acct, visibility, in_reply_to_id; ``thread_id`` = in_reply_to_id
  for replies; skip own mentions.
* Inbound (fallback): when SSE fails, REST poll
  ``{instance}/api/v1/notifications?types[]=mention&limit=30&since_id={last}``.
  Newest-first ordering — capture the first ID as high-water mark
  before iterating.
* Outbound: POST ``/api/v1/statuses`` form-encoded with
  ``status``, ``visibility="unlisted"``, optional ``in_reply_to_id``.
  Chunked at 500 chars (default per-instance limit); thread chunks
  are chained by feeding each response's ``id`` into the next
  chunk's ``in_reply_to_id``. Replies in a thread use the inbound
  ``thread_id`` as ``in_reply_to_id``.
* OAuth Bearer access token is required; ``verify_credentials`` is
  called at startup to validate and discover the bot's own account
  id (used to suppress self-mention echoes).
* ``suppress_error_responses = True`` — Mastodon replies are public,
  so errors are logged but never posted as toots.
* Reconnect with exponential backoff (1s → 60s).

Stdlib-only (the SDK has zero runtime deps): SSE on
``urllib.request`` long-lived stream, HTML stripping with a
character-level state machine + ``html.unescape``, REST send /
polling with ``urllib.request`` + form-encoded body.

Configure via ``[[sidecar_channels]]``:

    [[sidecar_channels]]
    name = "mastodon"
    command = "python3"
    args = ["-m", "librefang.sidecar.adapters.mastodon"]
    channel_type = "mastodon"
    [sidecar_channels.env]
    MASTODON_INSTANCE_URL = "https://mastodon.social"
    # MASTODON_ACCOUNT_ID = "prod"    # optional, multi-bot routing key
    # MASTODON_VISIBILITY = "unlisted"  # public | unlisted | private | direct
    # MASTODON_MAX_MESSAGE_LEN = "500"  # raise for instances with larger caps

The OAuth access token is read from the ``MASTODON_ACCESS_TOKEN``
env var (lives in ``~/.librefang/secrets.env``).
"""
from __future__ import annotations

import asyncio
import html
import json
import os
import time
import urllib.error
import urllib.parse
import urllib.request

from librefang.sidecar import Content, Field, Schema, SidecarAdapter, protocol, run_stdio_main
from librefang.sidecar import logging as log
from librefang.sidecar.common import (
    MAX_BACKOFF_SECS,
    RETRY_AFTER_DEFAULT_SECS,
    split_message as _split_message,
)

# Mastodon's default per-status length limit. Some instances configure
# higher limits (1000–4000); override via MASTODON_MAX_MESSAGE_LEN.
DEFAULT_MAX_MESSAGE_LEN = 500
SSE_RECONNECT_DELAY_SECS = 5
POLL_INTERVAL_SECS = 5
SEND_TIMEOUT_SECS = 15
DEFAULT_VISIBILITY = "unlisted"
_ALLOWED_VISIBILITIES = {"public", "unlisted", "private", "direct"}

def _strip_html_tags(value: str) -> str:
    """Strip HTML tags from a Mastodon status body and decode entities.
    Inserts a newline for block-level closing tags so paragraphs stay
    readable. Mirrors the Rust ``strip_html_tags`` shape."""
    result: list[str] = []
    in_tag = False
    tag_buf: list[str] = []
    for ch in value:
        if ch == "<":
            in_tag = True
            tag_buf.clear()
        elif ch == ">" and in_tag:
            in_tag = False
            tag_lower = "".join(tag_buf).lower()
            if (tag_lower.startswith("br")
                    or tag_lower.startswith("/p")
                    or tag_lower.startswith("/div")
                    or tag_lower.startswith("/li")):
                result.append("\n")
            tag_buf.clear()
        elif in_tag:
            tag_buf.append(ch)
        else:
            result.append(ch)
    return html.unescape("".join(result)).strip()


class MastodonAdapter(SidecarAdapter):
    # Mastodon replies are public — never echo internal errors as toots.
    suppress_error_responses = True
    # No typing / reaction / interactive / streaming concept.
    capabilities: list = []

    SCHEMA = Schema(
        name="mastodon",
        display_name="Mastodon",
        description="Mastodon Streaming API (out-of-process sidecar)",
        fields=[
            Field("MASTODON_INSTANCE_URL", "Instance URL", "text",
                  required=True,
                  placeholder="https://mastodon.social"),
            Field("MASTODON_ACCESS_TOKEN", "Access Token", "secret",
                  required=True,
                  placeholder="from Settings → Development → Your apps"),
            Field("MASTODON_VISIBILITY", "Default Visibility", "text",
                  placeholder=DEFAULT_VISIBILITY, advanced=True),
            Field("MASTODON_MAX_MESSAGE_LEN", "Max status length", "text",
                  placeholder=str(DEFAULT_MAX_MESSAGE_LEN), advanced=True),
            Field("MASTODON_ACCOUNT_ID", "Account ID (multi-bot routing)",
                  "text", placeholder="prod", advanced=True),
        ],
    )

    def __init__(self) -> None:
        instance = os.environ.get("MASTODON_INSTANCE_URL", "").strip()
        self.instance_url = instance.rstrip("/")
        self.access_token = os.environ.get("MASTODON_ACCESS_TOKEN", "").strip()
        acct = os.environ.get("MASTODON_ACCOUNT_ID", "").strip()
        self.account_id = acct or None
        vis = os.environ.get("MASTODON_VISIBILITY", "").strip() or DEFAULT_VISIBILITY
        if vis not in _ALLOWED_VISIBILITIES:
            log.error(
                "MASTODON_VISIBILITY must be one of public/unlisted/private/direct",
                value=vis,
            )
            raise SystemExit(2)
        self.default_visibility = vis
        try:
            max_len_raw = os.environ.get("MASTODON_MAX_MESSAGE_LEN", "").strip()
            self.max_message_len = (
                int(max_len_raw) if max_len_raw else DEFAULT_MAX_MESSAGE_LEN
            )
            if self.max_message_len <= 0:
                raise ValueError("must be positive")
        except (TypeError, ValueError) as e:
            log.error("MASTODON_MAX_MESSAGE_LEN invalid", error=str(e))
            raise SystemExit(2) from e

        missing = []
        if not self.instance_url:
            missing.append("MASTODON_INSTANCE_URL")
        if not self.access_token:
            missing.append("MASTODON_ACCESS_TOKEN")
        if missing:
            log.error("mastodon required env vars missing", missing=missing)
            raise SystemExit(2)
        if not (self.instance_url.startswith("http://")
                or self.instance_url.startswith("https://")):
            log.error(
                "MASTODON_INSTANCE_URL must start with http:// or https://",
                instance_url=self.instance_url,
            )
            raise SystemExit(2)

        # Discovered at startup via verify_credentials; used to skip
        # self-mention echoes.
        self.own_account_id: str | None = None

    # ---- helpers -----------------------------------------------------

    def _auth_headers(self, extra: dict | None = None) -> dict:
        h = {"Authorization": f"Bearer {self.access_token}"}
        if extra:
            h.update(extra)
        return h

    @staticmethod
    def _response_headers(resp_or_err) -> dict:
        """Pull headers off either a successful response or an HTTPError
        and normalise keys to lowercase so callers can do
        case-insensitive lookups (notably for ``Retry-After`` on 429)."""
        hdrs = getattr(resp_or_err, "headers", None)
        if hdrs is None:
            return {}
        try:
            return {k.lower(): v for k, v in hdrs.items()}
        except Exception:  # noqa: BLE001 — defensive against odd shims
            return {}

    @staticmethod
    def _retry_after_secs(resp_headers: dict) -> float:
        """Parse ``Retry-After`` (seconds form). Falls back to
        ``RETRY_AFTER_DEFAULT_SECS`` if absent / unparseable, floored at
        1 s and capped at ``MAX_BACKOFF_SECS`` so a misreported value
        can't block the producer for more than a minute. We don't
        decode the HTTP-date form — Mastodon's rate-limit replies use
        seconds in practice, and the fallback covers any divergence."""
        raw = resp_headers.get("retry-after")
        if not raw:
            return RETRY_AFTER_DEFAULT_SECS
        try:
            return min(max(float(raw), 1.0), MAX_BACKOFF_SECS)
        except (TypeError, ValueError):
            return RETRY_AFTER_DEFAULT_SECS

    def _sleep_on_429_then_raise(self, resp_hdrs: dict, where: str) -> None:
        """Common 429 handler: honour ``Retry-After`` then raise so the
        producer's outer backoff pauses before its next pass. Without
        the sleep the 1 s → 60 s exponential backoff would keep probing
        inside the server-side rate-limit window and extend it."""
        wait = self._retry_after_secs(resp_hdrs)
        log.warn(
            f"mastodon 429 on {where}; sleeping",
            retry_after_secs=wait,
        )
        time.sleep(wait)
        raise RuntimeError("mastodon 429 — rate-limited")

    def _verify_credentials(self) -> str:
        """Validate the token and discover the bot's own account id.
        Returns the username for logging. Raises on auth failure."""
        url = f"{self.instance_url}/api/v1/accounts/verify_credentials"
        req = urllib.request.Request(url, headers=self._auth_headers())
        try:
            with urllib.request.urlopen(  # noqa: S310 — configured URL
                req, timeout=SEND_TIMEOUT_SECS,
            ) as resp:
                status = getattr(resp, "status", 200)
                if status != 200:
                    raise RuntimeError(
                        f"verify_credentials HTTP {status}"
                    )
                body = json.loads(resp.read().decode("utf-8"))
        except urllib.error.HTTPError as e:
            if e.code == 429:
                # Mastodon rate-limits unauthenticated / failed-auth
                # probes; the verify retry loop would otherwise compound
                # with the server-side window.
                self._sleep_on_429_then_raise(
                    self._response_headers(e), "verify_credentials",
                )
            raise
        self.own_account_id = body.get("id") or ""
        return body.get("username") or "unknown"

    # ---- inbound: SSE primary + REST polling fallback ----------------

    def _parse_notification(self, notif: dict) -> dict | None:
        if notif.get("type") != "mention":
            return None
        status = notif.get("status")
        account = notif.get("account")
        if not isinstance(status, dict) or not isinstance(account, dict):
            return None
        account_id = str(account.get("id") or "")
        # Skip own mentions (defensive — shouldn't appear in user stream).
        if self.own_account_id and account_id == self.own_account_id:
            return None
        text = _strip_html_tags(str(status.get("content") or ""))
        if not text:
            return None
        status_id = str(status.get("id") or "")
        notif_id = str(notif.get("id") or "")
        username = str(account.get("username") or "")
        display_name = str(account.get("display_name") or "") or username
        acct = str(account.get("acct") or "")
        visibility = str(status.get("visibility") or "public")
        in_reply_to = status.get("in_reply_to_id")
        in_reply_to = str(in_reply_to) if in_reply_to else None

        if text.startswith("/"):
            head, _, tail = text[1:].partition(" ")
            content = Content.command(head, tail.split() if tail else [])
        else:
            content = Content.text(text)

        metadata = {
            "status_id": status_id,
            "notification_id": notif_id,
            "acct": acct,
            "visibility": visibility,
        }
        if in_reply_to:
            metadata["in_reply_to_id"] = in_reply_to

        # `status_id` is the id of the mention itself — i.e. the status
        # we want to reply TO when the bot answers. The pre-fix
        # behaviour surfaced `in_reply_to` here (the PARENT the mention
        # was responding to), which had two bugs at once: (1) the wrong
        # target — the bot would reply to whoever the user was
        # responding to, not the user; (2) the daemon's bridge only
        # round-trips `thread_id` under
        # `[channels.mastodon.overrides] threading = true` AND `thread`
        # capability, neither of which mastodon has, so the field was
        # always `None` in `on_send` regardless. Both fixed by using
        # `librefang_user` (always round-tripped) as the carrier.
        return protocol.message(
            user_id=account_id,
            user_name=display_name,
            content=content,
            message_id=status_id,
            is_group=False,
            librefang_user=status_id or None,
            thread_id=status_id or None,
            metadata=metadata,
        )

    def _sse_loop(self, emit) -> None:
        """One SSE subscribe pass; caller wraps in reconnect backoff.
        Returns normally on stream end → reconnect promptly. Raises on
        transport error → caller backs off."""
        url = f"{self.instance_url}/api/v1/streaming/user"
        headers = self._auth_headers({"Accept": "text/event-stream"})
        req = urllib.request.Request(url, headers=headers)
        # No read timeout: SSE is a long-lived stream.
        try:
            resp_cm = urllib.request.urlopen(req)  # noqa: S310 — configured URL
        except urllib.error.HTTPError as e:
            if e.code == 429:
                # Initial SSE subscribe rate-limited; honour Retry-After
                # before the producer's reconnect path retries.
                self._sleep_on_429_then_raise(
                    self._response_headers(e), "SSE subscribe",
                )
            raise
        with resp_cm as resp:
            status = getattr(resp, "status", 200)
            if status != 200:
                raise RuntimeError(f"SSE HTTP {status}")
            log.info("mastodon SSE connected", instance=self.instance_url)
            event_type = ""
            data_buf: list[str] = []
            for rawline in resp:
                line = rawline.decode("utf-8", "replace").rstrip("\r\n")
                if line.startswith(":"):
                    # SSE comment / keepalive — skip.
                    continue
                if line == "":
                    # End of an event; dispatch if we have one buffered.
                    if event_type == "notification" and data_buf:
                        data = "\n".join(data_buf)
                        try:
                            notif = json.loads(data)
                        except (ValueError, TypeError):
                            pass
                        else:
                            ev = self._parse_notification(notif)
                            if ev is not None:
                                emit(ev)
                    event_type = ""
                    data_buf.clear()
                    continue
                if line.startswith("event:"):
                    event_type = line[len("event:"):].strip()
                elif line.startswith("data:"):
                    data_buf.append(line[len("data:"):].lstrip(" "))

    def _poll_once(self, emit, since_id: str | None) -> str | None:
        """One REST notification poll. Returns the new high-water-mark
        id (newest seen) or `since_id` unchanged. Raises on HTTP error."""
        url = f"{self.instance_url}/api/v1/notifications?types[]=mention&limit=30"
        if since_id:
            url += f"&since_id={urllib.parse.quote(since_id)}"
        req = urllib.request.Request(url, headers=self._auth_headers())
        try:
            with urllib.request.urlopen(  # noqa: S310 — configured URL
                req, timeout=SEND_TIMEOUT_SECS,
            ) as resp:
                status = getattr(resp, "status", 200)
                if status != 200:
                    raise RuntimeError(f"poll HTTP {status}")
                notifs = json.loads(resp.read().decode("utf-8"))
        except urllib.error.HTTPError as e:
            if e.code == 429:
                # Polling rate-limited; honour Retry-After then raise so
                # the outer backoff pauses before the next poll pass.
                self._sleep_on_429_then_raise(
                    self._response_headers(e), "notifications poll",
                )
            raise
        if not isinstance(notifs, list):
            return since_id
        # Mastodon returns newest-first. Capture the first ID as the
        # next high-water mark *before* iterating — matches the Rust
        # adapter's anti-redelivery guard.
        newest = None
        if notifs and isinstance(notifs[0], dict):
            newest = str(notifs[0].get("id") or "") or None
        # Emit oldest-first so a burst of mentions caught in one poll
        # reaches the agent in conversation order. The list is
        # newest-first off the wire (and the high-water mark above is
        # taken from notifs[0] accordingly); the Rust adapter iterated it
        # as-is and delivered multi-mention bursts backwards.
        for notif in reversed(notifs):
            if not isinstance(notif, dict):
                continue
            ev = self._parse_notification(notif)
            if ev is not None:
                emit(ev)
        return newest or since_id

    def _producer_blocking(self, emit) -> None:
        """Run verify-then-(SSE-with-polling-fallback) loop in a thread.
        Mirrors the Rust adapter's `validate()`-then-stream pattern:
        `verify_credentials` must succeed before any events are emitted
        so `own_account_id` is populated and the self-mention guard in
        `_parse_notification` works. Retries verify with exponential
        backoff on transient failure rather than terminating the
        sidecar — the supervisor can still kill us, and a temporary
        500 from the instance shouldn't take the process down."""
        verify_backoff = 1.0
        while True:
            try:
                username = self._verify_credentials()
                log.info(
                    "mastodon authenticated",
                    username=username,
                    account_id=self.own_account_id,
                )
                break
            except Exception as e:  # noqa: BLE001
                log.warn(
                    "mastodon verify_credentials failed; will retry",
                    error=str(e),
                    delay=verify_backoff,
                )
                time.sleep(verify_backoff)
                verify_backoff = min(verify_backoff * 2, MAX_BACKOFF_SECS)

        backoff = 1.0
        last_notif_id: str | None = None
        use_streaming = True
        while True:
            if use_streaming:
                try:
                    self._sse_loop(emit)
                    # Clean stream end — reconnect promptly.
                    backoff = 1.0
                    continue
                except (urllib.error.HTTPError,
                        urllib.error.URLError,
                        RuntimeError) as e:
                    log.warn(
                        "mastodon SSE failed; falling back to polling",
                        error=str(e),
                    )
                    use_streaming = False
                except Exception as e:  # noqa: BLE001
                    log.warn(
                        "mastodon SSE unexpected error; backing off",
                        error=str(e),
                        delay=backoff,
                    )
                    time.sleep(backoff)
                    backoff = min(backoff * 2, MAX_BACKOFF_SECS)
                    continue
            try:
                last_notif_id = self._poll_once(emit, last_notif_id)
                backoff = 1.0
            except Exception as e:  # noqa: BLE001
                log.warn(
                    "mastodon poll error; backing off",
                    error=str(e),
                    delay=backoff,
                )
                time.sleep(backoff)
                backoff = min(backoff * 2, MAX_BACKOFF_SECS)
            time.sleep(POLL_INTERVAL_SECS)

    async def produce(self, emit) -> None:
        # Run the blocking inbound loop in a worker thread; CancelledError
        # propagates and tears it down at the await boundary.
        loop = asyncio.get_event_loop()
        await loop.run_in_executor(None, self._producer_blocking, emit)

    # ---- outbound: POST /api/v1/statuses -----------------------------

    def _post_status(self, text: str,
                     in_reply_to_id: str | None) -> None:
        """Post a status, chunking at MAX_MESSAGE_LEN; chained chunks
        thread under the response id of the previous chunk so a long
        reply stays as a single conversational thread."""
        url = f"{self.instance_url}/api/v1/statuses"
        reply_id = in_reply_to_id
        for chunk in _split_message(text, self.max_message_len):
            params = {
                "status": chunk,
                "visibility": self.default_visibility,
            }
            if reply_id:
                params["in_reply_to_id"] = reply_id
            body = urllib.parse.urlencode(params).encode("utf-8")
            req = urllib.request.Request(
                url,
                data=body,
                headers=self._auth_headers({
                    "Content-Type": "application/x-www-form-urlencoded",
                }),
                method="POST",
            )
            try:
                with urllib.request.urlopen(  # noqa: S310 — configured URL
                    req, timeout=SEND_TIMEOUT_SECS,
                ) as resp:
                    status = getattr(resp, "status", 200)
                    if status >= 300:
                        raise RuntimeError(f"post HTTP {status}")
                    resp_body = json.loads(resp.read().decode("utf-8"))
            except urllib.error.HTTPError as e:
                if e.code == 429:
                    # POST /statuses is rate-limited independently of
                    # auth. Honour Retry-After and raise;
                    # `suppress_error_responses=True` keeps the raise
                    # from echoing as a public toot.
                    self._sleep_on_429_then_raise(
                        self._response_headers(e), "status POST",
                    )
                err_body = e.read().decode("utf-8", "replace")
                raise RuntimeError(
                    f"mastodon post {e.code}: {err_body}"
                ) from e
            # Chain subsequent chunks to this status so the thread stays
            # ordered in clients that group replies.
            reply_id = str(resp_body.get("id") or "") or reply_id

    async def on_send(self, cmd) -> None:
        # Text only; structured content falls back to a placeholder so
        # the operator still sees something rather than a silent failure.
        if cmd.content and not (
            isinstance(cmd.content, dict) and "Text" in cmd.content
        ):
            text = "(Unsupported content type)"
        else:
            text = cmd.text or ""
        # Primary recovery: cmd.user["librefang_user"] carries the
        # status_id of the mention the bot is replying TO (set in
        # _parse_notification — librefang_user round-trips bytewise
        # through the bridge regardless of capabilities/overrides).
        # Fallback to cmd.thread_id for the forward-compat
        # threading=true path (would also require a future
        # `thread` capability declaration).
        in_reply_to: "Optional[str]" = None
        user = getattr(cmd, "user", None) or {}
        if isinstance(user, dict):
            candidate = user.get("librefang_user")
            # Guard: librefang_user is shared across channels (dingtalk
            # puts a sessionWebhook URL, telegram puts @username, …).
            # Mastodon status ids are typically pure-digit strings on
            # mastodon.social but opaque alphanumerics on some forks —
            # keep the guard generic (no URL, no whitespace, no @).
            if (isinstance(candidate, str) and candidate
                    and not candidate.startswith(("http://", "https://", "@"))
                    and " " not in candidate
                    and "\t" not in candidate):
                in_reply_to = candidate
        if in_reply_to is None:
            thread_id = getattr(cmd, "thread_id", None)
            if thread_id is not None and not isinstance(thread_id, str):
                thread_id = str(thread_id) if thread_id else None
            in_reply_to = thread_id

        await asyncio.get_event_loop().run_in_executor(
            None, self._post_status, text, in_reply_to,
        )


if __name__ == "__main__":
    run_stdio_main(MastodonAdapter)
