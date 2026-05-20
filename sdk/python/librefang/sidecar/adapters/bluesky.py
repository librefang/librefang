#!/usr/bin/env python3
"""Bluesky / AT Protocol sidecar channel adapter for LibreFang.

Replaces the former in-process Rust ``librefang-channels::bluesky``
adapter (removed in this sidecar migration; same pattern as ntfy
#5224, telegram #5241, gotify #5263, mastodon #5264). Behaviour is
preserved except for two intentional improvements explicitly
ack'd by the maintainer:

* **Outbound threading** (improvement over Rust parity). The Rust
  adapter captured `record.reply` on inbound but its `send()` always
  passed `None` for the reply ref, so reply chains never threaded.
  This sidecar maintains an in-memory LRU cache keyed by the
  notification URI; when LibreFang asks to reply (via
  ``cmd.thread_id``), the sidecar reconstructs the proper
  `{root, parent}` reply struct from the cache. Cache miss
  (sidecar restart, eviction) silently falls back to a non-threaded
  post — matching the old Rust behaviour.
* **`suppress_error_responses = True`** (improvement). Bluesky
  posts are public; mirroring Mastodon's policy, internal errors
  must not echo as a toot.

Inbound: 5s polling of ``app.bsky.notification.listNotifications``
with `seenAt` high-water-mark and a follow-up
``app.bsky.notification.updateSeen`` to mark each batch read.
Filter to ``reason in {"mention", "reply"}``; skip own
notifications; ``/cmd args`` → Command, else Text; sender =
``displayName`` fallback ``handle``; metadata carries uri / cid /
handle / reason / indexed_at / reply_ref-if-present;
``thread_id = notification.uri`` so the cache lookup on outbound
fires.

Outbound: ``com.atproto.repo.createRecord`` with the
``app.bsky.feed.post`` lexicon. Chunked at 300 characters (Bluesky
post length cap; like the Rust adapter we count Python str
code-points, an approximation of grapheme clusters). Bearer auth
on every request. Session refresh via
``com.atproto.server.refreshSession`` 5 minutes before the ~90-min
window expires; on refresh-fail or 401 we recreate the session
from identifier + app-password.

Stdlib-only (the SDK has zero runtime deps): all HTTP via
``urllib.request``, polling on a worker thread, in-memory
``OrderedDict``-backed LRU for the thread cache.

Configure via ``[[sidecar_channels]]``:

    [[sidecar_channels]]
    name = "bluesky"
    command = "python3"
    args = ["-m", "librefang.sidecar.adapters.bluesky"]
    channel_type = "bluesky"
    [sidecar_channels.env]
    BLUESKY_IDENTIFIER = "alice.bsky.social"   # handle or DID
    # BLUESKY_SERVICE_URL = "https://bsky.social"  # default; override for custom PDS
    # BLUESKY_ACCOUNT_ID = "prod"              # optional, multi-bot routing key

The Bluesky app password is read from the ``BLUESKY_APP_PASSWORD``
env var (lives in ``~/.librefang/secrets.env``).
"""
from __future__ import annotations

import asyncio
import json
import os
import time
import urllib.error
import urllib.parse
import urllib.request
from collections import OrderedDict
from typing import Any

from librefang.sidecar import Content, Field, Schema, SidecarAdapter, protocol, run_stdio_main
from librefang.sidecar import logging as log

DEFAULT_SERVICE_URL = "https://bsky.social"
# Bluesky post length cap (graphemes per spec; we approximate with
# Python str code-points like the Rust adapter approximates with chars).
MAX_MESSAGE_LEN = 300
POLL_INTERVAL_SECS = 5
SEND_TIMEOUT_SECS = 15
MAX_BACKOFF_SECS = 60.0
# Default fallback when Bluesky 429s without a `Retry-After` header.
# AT Protocol's XRPC layer typically sends one (seconds form, alongside
# `RateLimit-Reset` epoch); fall back to a sane wait so we don't busy-
# loop at 1 s when the header is absent or unparseable.
RETRY_AFTER_DEFAULT_SECS = 30.0
# Sessions last ~2h on bsky.social; refresh 5 min before the 90-min
# safety mark used by the Rust adapter.
SESSION_LIFE_SECS = 5400
SESSION_REFRESH_BUFFER_SECS = 300
# Thread-context cache size. 200 entries × ~200 B = ~40 KB, negligible.
THREAD_CACHE_MAX = 200


def _split_message(text: str, max_len: int) -> list[str]:
    """Chunk `text` into <= max_len pieces, preferring newline splits.
    Same shape as the ntfy / mastodon / Rust ``split_message`` helper."""
    if len(text) <= max_len:
        return [text]
    chunks: list[str] = []
    rest = text
    while len(rest) > max_len:
        window = rest[:max_len]
        cut = window.rfind("\n")
        if cut <= 0:
            cut = max_len
        chunks.append(rest[:cut])
        rest = rest[cut:].lstrip("\n") if cut < max_len else rest[cut:]
    if rest:
        chunks.append(rest)
    return chunks


class _LruCache:
    """Tiny fixed-size LRU. OrderedDict-backed: re-insert on get to mark
    as recently used; pop oldest on overflow. Sidecar-local, lost on
    process restart — that's acceptable because a missing reply ref just
    degrades to a non-threaded post (which is the old Rust behaviour)."""

    def __init__(self, max_size: int):
        self._max = max_size
        self._d: OrderedDict[str, dict] = OrderedDict()

    def get(self, key: str) -> dict | None:
        if key in self._d:
            self._d.move_to_end(key)
            return self._d[key]
        return None

    def put(self, key: str, value: dict) -> None:
        if key in self._d:
            self._d.move_to_end(key)
        self._d[key] = value
        while len(self._d) > self._max:
            self._d.popitem(last=False)

    def __len__(self) -> int:
        return len(self._d)


class BlueskyAdapter(SidecarAdapter):
    # Bluesky replies are public — never echo internal errors as a toot.
    # (Improvement over Rust parity, ack'd by maintainer.)
    suppress_error_responses = True
    # No typing / reaction / interactive / streaming concept.
    capabilities: list = []

    SCHEMA = Schema(
        name="bluesky",
        display_name="Bluesky",
        description="Bluesky / AT Protocol (out-of-process sidecar)",
        fields=[
            Field("BLUESKY_IDENTIFIER", "Handle or DID", "text",
                  required=True,
                  placeholder="alice.bsky.social"),
            Field("BLUESKY_APP_PASSWORD", "App Password", "secret",
                  required=True,
                  placeholder="xxxx-xxxx-xxxx-xxxx"),
            Field("BLUESKY_SERVICE_URL", "PDS Service URL", "text",
                  placeholder=DEFAULT_SERVICE_URL, advanced=True),
            Field("BLUESKY_ACCOUNT_ID", "Account ID (multi-bot routing)",
                  "text", placeholder="prod", advanced=True),
        ],
    )

    def __init__(self) -> None:
        identifier = os.environ.get("BLUESKY_IDENTIFIER", "").strip()
        password = os.environ.get("BLUESKY_APP_PASSWORD", "").strip()
        service = os.environ.get("BLUESKY_SERVICE_URL", "").strip()
        self.identifier = identifier
        self.app_password = password
        self.service_url = (service.rstrip("/") if service else DEFAULT_SERVICE_URL)
        acct = os.environ.get("BLUESKY_ACCOUNT_ID", "").strip()
        self.account_id = acct or None
        missing = []
        if not self.identifier:
            missing.append("BLUESKY_IDENTIFIER")
        if not self.app_password:
            missing.append("BLUESKY_APP_PASSWORD")
        if missing:
            log.error("bluesky required env vars missing", missing=missing)
            raise SystemExit(2)
        if not (self.service_url.startswith("http://")
                or self.service_url.startswith("https://")):
            log.error(
                "BLUESKY_SERVICE_URL must start with http:// or https://",
                service_url=self.service_url,
            )
            raise SystemExit(2)

        # Cached session state. None means "create or refresh on next use".
        self._access_jwt: str | None = None
        self._refresh_jwt: str | None = None
        self._session_did: str | None = None
        self._session_created_at: float = 0.0
        # Discovered at startup via validate().
        self.own_did: str | None = None
        # Reply-thread cache: notification uri → {"root": {uri,cid},
        # "parent": {uri,cid}}.
        self._thread_cache = _LruCache(THREAD_CACHE_MAX)

    # ---- helpers -----------------------------------------------------

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

    def _post_json(self, url: str, body: dict | None,
                   *, bearer: str | None = None,
                   timeout: float = SEND_TIMEOUT_SECS,
                   ) -> tuple[int, dict | None, dict]:
        """Issue an XRPC POST. Returns
        ``(status, parsed_json_body | None, response_headers)``. Response
        header keys are normalised lowercase so callers can do
        case-insensitive lookups (notably for ``Retry-After`` on 429 —
        headers are returned on HTTPError too). The body may be None for
        the `refreshSession` endpoint which expects an empty POST body."""
        data = json.dumps(body).encode("utf-8") if body is not None else b""
        headers = {"Content-Type": "application/json"}
        if bearer:
            headers["Authorization"] = f"Bearer {bearer}"
        req = urllib.request.Request(url, data=data, headers=headers, method="POST")
        try:
            with urllib.request.urlopen(  # noqa: S310 — configured URL
                req, timeout=timeout,
            ) as resp:
                status = getattr(resp, "status", 200)
                raw = resp.read()
                resp_hdrs = self._response_headers(resp)
                if not raw:
                    return status, None, resp_hdrs
                try:
                    return status, json.loads(raw.decode("utf-8")), resp_hdrs
                except (ValueError, TypeError):
                    return status, None, resp_hdrs
        except urllib.error.HTTPError as e:
            resp_hdrs = self._response_headers(e)
            try:
                err_body = json.loads(e.read().decode("utf-8"))
            except Exception:  # noqa: BLE001
                err_body = None
            return e.code, err_body, resp_hdrs

    def _get_json(self, url: str, *, bearer: str | None = None,
                  timeout: float = SEND_TIMEOUT_SECS,
                  ) -> tuple[int, dict | None, dict]:
        headers = {}
        if bearer:
            headers["Authorization"] = f"Bearer {bearer}"
        req = urllib.request.Request(url, headers=headers)
        try:
            with urllib.request.urlopen(  # noqa: S310 — configured URL
                req, timeout=timeout,
            ) as resp:
                status = getattr(resp, "status", 200)
                raw = resp.read()
                resp_hdrs = self._response_headers(resp)
                if not raw:
                    return status, None, resp_hdrs
                try:
                    return status, json.loads(raw.decode("utf-8")), resp_hdrs
                except (ValueError, TypeError):
                    return status, None, resp_hdrs
        except urllib.error.HTTPError as e:
            resp_hdrs = self._response_headers(e)
            try:
                err_body = json.loads(e.read().decode("utf-8"))
            except Exception:  # noqa: BLE001
                err_body = None
            return e.code, err_body, resp_hdrs

    @staticmethod
    def _retry_after_secs(resp_headers: dict) -> float:
        """Parse ``Retry-After`` (seconds form). Falls back to
        ``RETRY_AFTER_DEFAULT_SECS`` if absent / unparseable, floored at
        1 s and capped at ``MAX_BACKOFF_SECS`` so a misreported value
        can't block the poller for more than a minute. We don't decode
        the HTTP-date form or AT Protocol's ``RateLimit-Reset`` epoch —
        XRPC's rate-limit replies use seconds in practice, and the
        fallback covers any divergence."""
        raw = resp_headers.get("retry-after")
        if not raw:
            return RETRY_AFTER_DEFAULT_SECS
        try:
            return min(max(float(raw), 1.0), MAX_BACKOFF_SECS)
        except (TypeError, ValueError):
            return RETRY_AFTER_DEFAULT_SECS

    # ---- session management ------------------------------------------

    def _create_session(self) -> str:
        """Mint a new session from identifier + app_password. Returns the
        DID of the authenticated account; stores session state on self."""
        url = f"{self.service_url}/xrpc/com.atproto.server.createSession"
        body = {"identifier": self.identifier, "password": self.app_password}
        status, resp, resp_hdrs = self._post_json(url, body)
        if status == 429:
            # Bluesky / PDS rate-limits createSession aggressively when
            # a single IP racks up failed logins. Honour Retry-After then
            # raise so the producer's outer backoff pauses before the
            # next attempt — without this the verify-credentials retry
            # loop would compound with the server-side window.
            wait = self._retry_after_secs(resp_hdrs)
            log.warn(
                "bluesky 429 on createSession; sleeping",
                retry_after_secs=wait,
            )
            time.sleep(wait)
            raise RuntimeError("bluesky 429 — rate-limited")
        if status != 200 or not isinstance(resp, dict):
            raise RuntimeError(
                f"bluesky createSession failed {status}: {resp!r}"
            )
        access = resp.get("accessJwt")
        refresh = resp.get("refreshJwt")
        did = resp.get("did")
        if not (isinstance(access, str) and access
                and isinstance(refresh, str) and refresh
                and isinstance(did, str) and did):
            raise RuntimeError("bluesky createSession: missing jwt/did fields")
        self._access_jwt = access
        self._refresh_jwt = refresh
        self._session_did = did
        self._session_created_at = time.monotonic()
        return did

    def _refresh_session(self) -> None:
        """Refresh the access JWT. Falls back to createSession on failure
        — matches the Rust adapter's behaviour (transient refresh
        failures are recoverable by re-authing with the password)."""
        if not self._refresh_jwt:
            self._create_session()
            return
        url = f"{self.service_url}/xrpc/com.atproto.server.refreshSession"
        status, resp, resp_hdrs = self._post_json(
            url, None, bearer=self._refresh_jwt,
        )
        if status == 429:
            # Refresh can also be throttled. Honour Retry-After and
            # raise; the caller (`_get_token`) propagates this to the
            # poll / send paths whose outer loops back off.
            wait = self._retry_after_secs(resp_hdrs)
            log.warn(
                "bluesky 429 on refreshSession; sleeping",
                retry_after_secs=wait,
            )
            time.sleep(wait)
            raise RuntimeError("bluesky 429 — rate-limited")
        if status != 200 or not isinstance(resp, dict):
            log.info(
                "bluesky refreshSession failed; re-creating session",
                status=status,
            )
            self._create_session()
            return
        access = resp.get("accessJwt")
        new_refresh = resp.get("refreshJwt")
        did = resp.get("did")
        if not (isinstance(access, str) and access
                and isinstance(new_refresh, str) and new_refresh
                and isinstance(did, str) and did):
            self._create_session()
            return
        self._access_jwt = access
        self._refresh_jwt = new_refresh
        self._session_did = did
        self._session_created_at = time.monotonic()

    def _get_token(self) -> tuple[str, str]:
        """Return (access_jwt, did), refreshing if the session is close
        to expiry. Mirrors the Rust ``get_token`` logic."""
        if (self._access_jwt is not None
                and time.monotonic() - self._session_created_at
                < (SESSION_LIFE_SECS - SESSION_REFRESH_BUFFER_SECS)):
            assert self._session_did is not None
            return self._access_jwt, self._session_did
        if self._access_jwt is not None:
            self._refresh_session()
        else:
            self._create_session()
        assert self._access_jwt is not None and self._session_did is not None
        return self._access_jwt, self._session_did

    def _verify_credentials(self) -> str:
        """Validate at startup by creating a session and discovering the
        bot's own DID (used to skip self-mentions). Mirrors Rust's
        `validate()` step. Returns the DID for logging."""
        did = self._create_session()
        self.own_did = did
        return did

    # ---- inbound: poll listNotifications -----------------------------

    @staticmethod
    def _compute_reply_ref(notif: dict) -> dict:
        """Build the AT Protocol `reply` struct for a reply pointing at
        this notification's post.

        Per the lexicon:
          reply.parent = the post being replied to (this notification).
          reply.root   = the original post of the thread; for a direct
                         mention, root == parent; for a reply-to-a-reply,
                         root comes from the existing record.reply.root.
        """
        uri = str(notif.get("uri") or "")
        cid = str(notif.get("cid") or "")
        parent = {"uri": uri, "cid": cid}
        record = notif.get("record")
        if isinstance(record, dict):
            existing = record.get("reply")
            if (isinstance(existing, dict)
                    and isinstance(existing.get("root"), dict)):
                return {"root": existing["root"], "parent": parent}
        return {"root": parent, "parent": parent}

    def _parse_notification(self, notif: dict) -> dict | None:
        if not isinstance(notif, dict):
            return None
        reason = notif.get("reason")
        if reason not in ("mention", "reply"):
            return None
        author = notif.get("author") if isinstance(notif.get("author"), dict) else None
        if author is None:
            return None
        author_did = str(author.get("did") or "")
        if self.own_did and author_did == self.own_did:
            return None
        record = notif.get("record") if isinstance(notif.get("record"), dict) else None
        if record is None:
            return None
        text = str(record.get("text") or "")
        if not text:
            return None

        uri = str(notif.get("uri") or "")
        cid = str(notif.get("cid") or "")
        handle = str(author.get("handle") or "")
        display_name = str(author.get("displayName") or "") or handle
        indexed_at = str(notif.get("indexedAt") or "")

        if text.startswith("/"):
            head, _, tail = text[1:].partition(" ")
            content = Content.command(head, tail.split() if tail else [])
        else:
            content = Content.text(text)

        metadata: dict[str, Any] = {
            "uri": uri,
            "cid": cid,
            "handle": handle,
            "reason": str(reason),
            "indexed_at": indexed_at,
        }
        # Capture the inbound record.reply if present, matching the
        # Rust adapter's metadata shape.
        if isinstance(record.get("reply"), dict):
            metadata["reply_ref"] = record["reply"]

        # Cache the computed outbound reply struct keyed by this
        # notification's URI. on_send looks it up via cmd.thread_id.
        if uri:
            self._thread_cache.put(uri, self._compute_reply_ref(notif))

        return protocol.message(
            user_id=author_did,
            user_name=display_name,
            content=content,
            message_id=uri,
            is_group=False,
            # Surface the URI as thread_id so LibreFang threads outbound
            # replies through to on_send via cmd.thread_id; the sidecar
            # then reconstructs the reply struct from its cache.
            thread_id=uri or None,
            metadata=metadata,
        )

    def _poll_once(self, emit, last_seen_at: str | None) -> str | None:
        """One notification poll pass. Returns updated `last_seen_at`
        (max indexedAt observed) or `last_seen_at` if no progress.
        Raises on transport / auth error — caller handles backoff."""
        url = (
            f"{self.service_url}"
            f"/xrpc/app.bsky.notification.listNotifications?limit=25"
        )
        if last_seen_at:
            url += "&" + urllib.parse.urlencode({"seenAt": last_seen_at})
        token, _did = self._get_token()
        status, body, resp_hdrs = self._get_json(url, bearer=token)
        if status == 401:
            # Mirror Rust: clear session so next poll re-auths.
            self._access_jwt = None
            raise RuntimeError("bluesky 401 — session expired")
        if status == 429:
            # XRPC rate limit on listNotifications. Honour Retry-After
            # and raise so `_producer_blocking`'s outer backoff pauses
            # — the per-poll loop would otherwise keep probing inside
            # the throttling window.
            wait = self._retry_after_secs(resp_hdrs)
            log.warn(
                "bluesky 429 on listNotifications; sleeping",
                retry_after_secs=wait,
            )
            time.sleep(wait)
            raise RuntimeError("bluesky 429 — rate-limited")
        if status != 200 or not isinstance(body, dict):
            raise RuntimeError(f"bluesky listNotifications {status}: {body!r}")
        notifs = body.get("notifications")
        if not isinstance(notifs, list):
            return last_seen_at
        new_seen = last_seen_at
        # `listNotifications` returns notifications newest-first. Emit
        # them oldest-first so a burst caught in one poll reaches the
        # agent in conversation order; the high-water mark below is the
        # max `indexedAt` and so is order-independent. The Rust adapter
        # iterated the raw newest-first list and delivered such bursts
        # backwards.
        for notif in reversed(notifs):
            if not isinstance(notif, dict):
                continue
            indexed = notif.get("indexedAt")
            if isinstance(indexed, str) and (
                new_seen is None or indexed > new_seen
            ):
                new_seen = indexed
            ev = self._parse_notification(notif)
            if ev is not None:
                emit(ev)
        # Mark seen (best-effort; failure doesn't break the loop).
        if new_seen:
            self._mark_seen(token)
        return new_seen

    def _mark_seen(self, token: str) -> None:
        """Post the current wall-clock time to `updateSeen`. Matches the
        Rust adapter — Bluesky's API takes a seenAt timestamp rather
        than the last-seen indexedAt (so we send now)."""
        url = f"{self.service_url}/xrpc/app.bsky.notification.updateSeen"
        # Bluesky expects RFC3339 with milliseconds; Python's
        # datetime.isoformat omits ms unless microseconds is non-zero,
        # so format explicitly.
        import datetime
        now = datetime.datetime.now(datetime.timezone.utc)
        seen_at = now.strftime("%Y-%m-%dT%H:%M:%S.") + f"{now.microsecond // 1000:03d}Z"
        try:
            self._post_json(url, {"seenAt": seen_at}, bearer=token)
        except Exception as e:  # noqa: BLE001 — best-effort
            log.debug("bluesky updateSeen failed (best-effort)", error=str(e))

    def _producer_blocking(self, emit) -> None:
        """Verify credentials then poll forever in this worker thread.
        Mirrors mastodon's pattern: verify-once with retry, then enter
        the steady-state poll loop with exponential backoff on errors."""
        verify_backoff = 1.0
        while True:
            try:
                did = self._verify_credentials()
                log.info("bluesky authenticated", did=did)
                break
            except Exception as e:  # noqa: BLE001
                log.warn(
                    "bluesky createSession failed; will retry",
                    error=str(e), delay=verify_backoff,
                )
                time.sleep(verify_backoff)
                verify_backoff = min(verify_backoff * 2, MAX_BACKOFF_SECS)

        backoff = 1.0
        last_seen_at: str | None = None
        while True:
            try:
                last_seen_at = self._poll_once(emit, last_seen_at)
                backoff = 1.0
            except Exception as e:  # noqa: BLE001
                log.warn(
                    "bluesky poll error; backing off",
                    error=str(e), delay=backoff,
                )
                time.sleep(backoff)
                backoff = min(backoff * 2, MAX_BACKOFF_SECS)
            time.sleep(POLL_INTERVAL_SECS)

    async def produce(self, emit) -> None:
        loop = asyncio.get_event_loop()
        await loop.run_in_executor(None, self._producer_blocking, emit)

    # ---- outbound: createRecord --------------------------------------

    def _post_status(self, text: str, thread_id: str | None) -> None:
        """Create one or more `app.bsky.feed.post` records. When
        `thread_id` matches a cached notification URI, attach the
        reply struct so the post threads. Chunked posts chain by
        re-using the thread context for every chunk — the Rust adapter
        did not chain chunks, but since the user opted into improved
        threading (P1=b), we use the same reply ref for each chunk so
        the whole multi-part reply stays under one thread parent."""
        token, did = self._get_token()
        url = f"{self.service_url}/xrpc/com.atproto.repo.createRecord"
        reply_ref = self._thread_cache.get(thread_id) if thread_id else None
        import datetime
        for chunk in _split_message(text, MAX_MESSAGE_LEN):
            now = datetime.datetime.now(datetime.timezone.utc)
            created_at = now.strftime("%Y-%m-%dT%H:%M:%S.") + f"{now.microsecond // 1000:03d}Z"
            record: dict[str, Any] = {
                "$type": "app.bsky.feed.post",
                "text": chunk,
                "createdAt": created_at,
            }
            if reply_ref is not None:
                record["reply"] = reply_ref
            body = {
                "repo": did,
                "collection": "app.bsky.feed.post",
                "record": record,
            }
            status, resp, resp_hdrs = self._post_json(url, body, bearer=token)
            if status == 401:
                # Token expired mid-batch: refresh once and retry this
                # chunk. If it still fails we surface the error.
                self._access_jwt = None
                token, did = self._get_token()
                body["repo"] = did
                status, resp, resp_hdrs = self._post_json(
                    url, body, bearer=token,
                )
            if status == 429:
                # createRecord is rate-limited independently of auth.
                # Honour Retry-After and raise; `suppress_error_responses`
                # already prevents the raise from echoing as a public
                # post.
                wait = self._retry_after_secs(resp_hdrs)
                log.warn(
                    "bluesky 429 on createRecord; sleeping",
                    retry_after_secs=wait,
                )
                time.sleep(wait)
                raise RuntimeError("bluesky 429 — rate-limited")
            if status >= 300:
                raise RuntimeError(
                    f"bluesky createRecord {status}: {resp!r}"
                )

    async def on_send(self, cmd) -> None:
        # Text-only; structured content falls back to a placeholder so
        # the operator still sees something rather than a silent failure
        # (matches the Rust adapter).
        if cmd.content and not (
            isinstance(cmd.content, dict) and "Text" in cmd.content
        ):
            text = "(Unsupported content type)"
        else:
            text = cmd.text or ""
        thread_id = getattr(cmd, "thread_id", None)
        if thread_id is not None and not isinstance(thread_id, str):
            thread_id = str(thread_id) if thread_id else None
        await asyncio.get_event_loop().run_in_executor(
            None, self._post_status, text, thread_id,
        )


if __name__ == "__main__":
    run_stdio_main(BlueskyAdapter)
