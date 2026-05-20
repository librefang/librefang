#!/usr/bin/env python3
"""Reddit OAuth2 sidecar channel adapter for LibreFang.

Replaces the former in-process Rust ``librefang-channels::reddit``
adapter (removed in this sidecar migration; same pattern as ntfy
#5224, telegram #5241, gotify #5263, mastodon #5264, bluesky #5277).

Behaviour is preserved except for two intentional improvements
explicitly ack'd by the maintainer:

* **Outbound reply target** (improvement / bugfix over Rust parity).
  The Rust adapter set ``thread_id = subreddit`` on inbound, then
  ``send()`` passed ``user.platform_id`` as the parent fullname to
  ``POST /api/comment``. But ``user.platform_id`` was set to the
  comment author's username (``parse_reddit_comment`` writes the
  author there), not the fullname (``t1_<id>``). The Rust send-path
  unit tests dodged this by manually mocking ``platform_id=fullname``,
  but in production a real bridge call would 400 with
  ``"thing_id must be a fullname"``. This sidecar surfaces the
  fullname (``t1_<comment_id>``) as ``thread_id``, so the
  ``cmd.thread_id`` daemon round-trips to ``on_send`` is the parent
  fullname the Reddit API actually needs. Per-comment threading also
  matches the Bluesky / Mastodon adapters (each mention → its own
  agent session, rather than one giant per-subreddit session).
* **``suppress_error_responses = True``**. Reddit comments are
  public; internal errors must not echo as a comment. Same rationale
  as Mastodon / Bluesky.

Inbound: 5 s polling per subreddit of
``GET /r/{sub}/comments?limit=25&sort=new`` via OAuth-authenticated
``oauth.reddit.com``. Skip ``kind != "t1"`` (posts), skip own
comments (``own_username`` resolved at startup via
``GET /api/v1/me``), skip ``[deleted]`` / ``[removed]``. ``/cmd args``
→ Command, else Text; sender = author; metadata carries
``fullname``, ``subreddit``, ``link_id``, ``parent_id``, ``permalink``.
``seen_comments`` set caps growth at 10 000 IDs (oldest half evicted).

Outbound: ``POST /api/comment`` form-encoded with
``api_type=json``, ``thing_id=<parent_fullname>``,
``text=<chunked>``. Reddit allows one reply per parent, so chunks
join with ``\\n\\n---\\n\\n`` (matches the Rust adapter).
Non-text content falls back to a placeholder string.

OAuth2: ``POST {token_url}`` with basic auth (client_id, client_secret)
and form ``grant_type=password&username=…&password=…`` (script-app
flow). Tokens cached with a 300 s refresh buffer.
Reddit requires a unique User-Agent — we send
``librefang:v{pkg-version} (by /u/librefang-bot)`` to mirror the
Rust adapter.

Stdlib-only (the SDK has zero runtime deps): HTTP via
``urllib.request``, polling on a worker thread.

Configure via ``[[sidecar_channels]]``:

    [[sidecar_channels]]
    name = "reddit"
    command = "python3"
    args = ["-m", "librefang.sidecar.adapters.reddit"]
    channel_type = "reddit"
    [sidecar_channels.env]
    REDDIT_CLIENT_ID = "abc123"
    REDDIT_USERNAME = "librefang-bot"
    REDDIT_SUBREDDITS = "rust,programming"           # comma-separated
    REDDIT_USER_AGENT = "myorg/1.0 (by /u/me)"       # REQUIRED — see below
    # REDDIT_ACCOUNT_ID = "prod"                      # optional, multi-bot routing
    # REDDIT_POLL_INTERVAL_SECS = "30"                # optional, default 30, floor 5

Secrets via ``~/.librefang/secrets.env``:
``REDDIT_CLIENT_SECRET`` and ``REDDIT_PASSWORD``.

Ban-avoidance defaults
======================

Reddit is strict about bot behaviour and the adapter ships with the
guardrails that the operator most often forgets:

* **REDDIT_USER_AGENT is required and validated.** Reddit's API rules
  require the UA to identify the maintainer (``by /u/<your-handle>``)
  and treat fake / impersonating UAs as grounds for IP+account ban.
  The literal default UA contains ``/u/librefang-bot`` (NOT a real
  account); the adapter rejects it at startup so an operator can't
  ship-by-accident. Use your own maintainer Reddit handle.
* **30-second default polling interval.** The Rust adapter polled
  every 5 s; with N subreddits that's the fastest path to burning
  the 60 req/min/account budget and getting a short-ban. The default
  here is 30 s (operator-tunable via ``REDDIT_POLL_INTERVAL_SECS``,
  floored at 5 s).
* **``X-Ratelimit-*`` aware throttling.** Every response is inspected:
  when ``X-Ratelimit-Remaining`` falls below 10, the poller sleeps
  until the reset window (capped at 60 s) before issuing the next
  sub-fetch. This converts "burn budget then get 429'd" into a
  smooth pre-emptive slow-down, which is what Reddit's anti-abuse
  side actually wants to see.
* **429 handling.** A 429 on polling honours ``Retry-After`` and
  raises, letting the producer loop back off. A 429 on
  ``/api/comment`` honours ``Retry-After`` and retries once before
  surfacing the error.

What this adapter does **not** enforce (operator responsibility):

* Account age and karma minimums (Reddit shadowbans young accounts).
* Per-subreddit posting permission — get mod approval first.
* Trigger gating — by default every new comment is emitted; bound it
  with ``group_trigger_patterns`` in the agent's config to only
  respond to ``/cmd`` or named-mention so the bot isn't spammy.
"""
from __future__ import annotations

import asyncio
import base64
import json
import os
import time
import urllib.error
import urllib.parse
import urllib.request
from typing import Any

from librefang.sidecar import Content, Field, Schema, SidecarAdapter, protocol, run_stdio_main
from librefang.sidecar import logging as log
from librefang.sidecar.common import split_message as _split_message

DEFAULT_TOKEN_URL = "https://www.reddit.com/api/v1/access_token"
DEFAULT_API_BASE = "https://oauth.reddit.com"
# Reddit comments cap at 10000 chars. We chunk slightly under and join
# chunks with a separator because Reddit only accepts one reply per
# parent (matches the Rust adapter).
MAX_MESSAGE_LEN = 10000
# Default polling interval. 30 s is the safer default for ban-avoidance:
# the Rust adapter used 5 s, but 5 s × N subreddits is the quickest way
# to burn Reddit's 60 req/min/account budget and trigger an anti-abuse
# short-ban. Operators with a high-churn sub can override via
# REDDIT_POLL_INTERVAL_SECS (floored at MIN_POLL_INTERVAL_SECS).
DEFAULT_POLL_INTERVAL_SECS = 30
MIN_POLL_INTERVAL_SECS = 5
SEND_TIMEOUT_SECS = 15
MAX_BACKOFF_SECS = 60.0
# Refresh OAuth tokens 5 minutes before expiry.
TOKEN_REFRESH_BUFFER_SECS = 300
# Cap the dedupe set; oldest half is evicted on overflow.
SEEN_COMMENTS_MAX = 10000
SEEN_COMMENTS_EVICT = 5000
# Chunk join string when a reply exceeds MAX_MESSAGE_LEN. Reddit allows
# one comment per parent, so multiple chunks become one comment with
# clear visual separators.
CHUNK_JOIN = "\n\n---\n\n"
# Marker appended when a reply is truncated to fit Reddit's 10 000-char
# comment cap. Visible to the operator in the posted comment so an
# overlong agent response is obvious rather than silently mangled.
TRUNCATION_MARKER = "\n\n[…truncated]"
# Reddit requires a unique, descriptive UA per its API guidelines. The
# default deliberately contains '/u/librefang-bot' which is NOT a real
# account; __init__ rejects it so the operator must configure
# REDDIT_USER_AGENT with their own maintainer handle before the sidecar
# will start. Reddit treats fake or impersonating UAs as grounds for
# IP+account ban — this is the single biggest source of bot bans, and
# it has to be impossible to ship-by-accident.
DEFAULT_USER_AGENT = "librefang:sidecar (by /u/librefang-bot)"
# Substrings in the UA that mean "operator forgot to set a real one".
# Case-insensitive match.
PLACEHOLDER_UA_FRAGMENTS = ("librefang-bot", "/u/your", "/u/example")
# Rate-limit response-header handling. Reddit returns:
#   X-Ratelimit-Used:      fractional req count used in current window
#   X-Ratelimit-Remaining: float remaining in current window
#   X-Ratelimit-Reset:     seconds until the window resets
# When `remaining` drops below the floor we pre-emptively sleep until
# reset so we don't burn through the budget and trip a 429. Capped at
# MAX_BACKOFF_SECS so we never block the poller for more than a minute.
RATELIMIT_REMAINING_FLOOR = 10.0
# Default Retry-After when Reddit 429s without the header (rare but
# documented). 60 s matches Reddit's API guideline minimum back-off.
RETRY_AFTER_DEFAULT_SECS = 60.0


def _normalise_subreddit(value: str) -> str:
    """Strip whitespace and a leading ``r/`` prefix, then drop trailing
    slashes. ``"r/rust/"`` → ``"rust"``; ``"rust"`` → ``"rust"``.
    Returns the empty string for whitespace-only input."""
    s = value.strip().rstrip("/")
    if s.startswith("r/"):
        s = s[2:]
    return s.strip("/")

def _parse_reddit_comment(comment: dict, own_username: str) -> dict | None:
    """Parse a Reddit comment JSON object into a ``message`` event.

    Returns ``None`` if the comment should be skipped (post, self,
    deleted/removed, empty body, or malformed shape). Mirrors the
    Rust adapter's ``parse_reddit_comment``."""
    if not isinstance(comment, dict):
        return None
    if comment.get("kind") != "t1":  # t1 = comment, t3 = post
        return None
    data = comment.get("data")
    if not isinstance(data, dict):
        return None
    author = str(data.get("author") or "")
    if not author:
        return None
    if author.lower() == own_username.lower():
        return None
    if author in ("[deleted]", "[removed]"):
        return None
    body = str(data.get("body") or "")
    if not body:
        return None

    comment_id = str(data.get("id") or "")
    fullname = str(data.get("name") or "")  # e.g. "t1_abc123"
    subreddit = str(data.get("subreddit") or "")
    link_id = str(data.get("link_id") or "")
    parent_id = str(data.get("parent_id") or "")
    permalink = str(data.get("permalink") or "")

    if body.startswith("/"):
        head, _, tail = body[1:].partition(" ")
        content = Content.command(head, tail.split() if tail else [])
    else:
        content = Content.text(body)

    metadata: dict[str, Any] = {
        "fullname": fullname,
        "subreddit": subreddit,
        "link_id": link_id,
        "parent_id": parent_id,
    }
    if permalink:
        metadata["permalink"] = permalink

    return protocol.message(
        user_id=author,
        user_name=author,
        content=content,
        message_id=comment_id,
        is_group=True,  # Subreddit comments are public/group, like Rust.
        # P1: surface fullname as thread_id so on_send uses it as the
        # parent fullname for POST /api/comment. The Rust adapter's
        # thread_id was the subreddit, which broke per-comment replies
        # in production (user.platform_id was the author, not the
        # fullname /api/comment requires).
        thread_id=fullname or None,
        metadata=metadata,
    )


class RedditAdapter(SidecarAdapter):
    # Reddit comments are public — never echo internal errors as a
    # reply. (Improvement over Rust parity, ack'd by maintainer.)
    suppress_error_responses = True
    # No typing / reaction / interactive / streaming concept on Reddit.
    capabilities: list = []

    SCHEMA = Schema(
        name="reddit",
        display_name="Reddit",
        description="Reddit OAuth2 API (out-of-process sidecar)",
        fields=[
            Field("REDDIT_CLIENT_ID", "OAuth2 Client ID", "text",
                  required=True,
                  placeholder="from https://www.reddit.com/prefs/apps"),
            Field("REDDIT_CLIENT_SECRET", "OAuth2 Client Secret",
                  "secret", required=True),
            Field("REDDIT_USERNAME", "Bot Username", "text",
                  required=True, placeholder="librefang-bot"),
            Field("REDDIT_PASSWORD", "Bot Password", "secret",
                  required=True),
            Field("REDDIT_SUBREDDITS", "Subreddits (comma-separated)",
                  "text", required=True,
                  placeholder="rust,programming"),
            Field("REDDIT_ACCOUNT_ID",
                  "Account ID (multi-bot routing)", "text",
                  placeholder="prod", advanced=True),
            Field("REDDIT_USER_AGENT",
                  "User-Agent (REQUIRED — must contain your real "
                  "/u/<username>; the placeholder default is rejected "
                  "at startup to prevent IP/account bans)",
                  "text",
                  placeholder="myorg/1.0 (by /u/myactual-reddit-name)",
                  required=True),
            Field("REDDIT_POLL_INTERVAL_SECS",
                  f"Poll interval seconds (default {DEFAULT_POLL_INTERVAL_SECS}, "
                  f"floor {MIN_POLL_INTERVAL_SECS}). Higher = safer for ban-avoidance.",
                  "text",
                  placeholder=str(DEFAULT_POLL_INTERVAL_SECS),
                  advanced=True),
        ],
    )

    def __init__(self) -> None:
        self.client_id = os.environ.get("REDDIT_CLIENT_ID", "").strip()
        self.client_secret = os.environ.get("REDDIT_CLIENT_SECRET", "").strip()
        self.username = os.environ.get("REDDIT_USERNAME", "").strip()
        self.password = os.environ.get("REDDIT_PASSWORD", "").strip()
        subs_raw = os.environ.get("REDDIT_SUBREDDITS", "").strip()
        self.subreddits = [
            _normalise_subreddit(s) for s in subs_raw.split(",")
            if _normalise_subreddit(s)
        ]
        acct = os.environ.get("REDDIT_ACCOUNT_ID", "").strip()
        self.account_id = acct or None
        ua = os.environ.get("REDDIT_USER_AGENT", "").strip()
        self.user_agent = ua or DEFAULT_USER_AGENT

        # Poll interval — default 30 s (safer for ban-avoidance), floor 5 s.
        # Operator-tunable via REDDIT_POLL_INTERVAL_SECS env var.
        interval_raw = os.environ.get("REDDIT_POLL_INTERVAL_SECS", "").strip()
        try:
            interval = (
                int(interval_raw) if interval_raw else DEFAULT_POLL_INTERVAL_SECS
            )
        except (TypeError, ValueError):
            log.error(
                "REDDIT_POLL_INTERVAL_SECS invalid (must be integer)",
                value=interval_raw,
            )
            raise SystemExit(2) from None
        if interval < MIN_POLL_INTERVAL_SECS:
            log.warn(
                "REDDIT_POLL_INTERVAL_SECS below floor; clamping",
                requested=interval, floor=MIN_POLL_INTERVAL_SECS,
            )
            interval = MIN_POLL_INTERVAL_SECS
        self.poll_interval = interval

        # Optional URL overrides for test injection (no env handle —
        # tests reach in via attributes after construction).
        self.token_url = DEFAULT_TOKEN_URL
        self.api_base = DEFAULT_API_BASE

        missing: list[str] = []
        if not self.client_id:
            missing.append("REDDIT_CLIENT_ID")
        if not self.client_secret:
            missing.append("REDDIT_CLIENT_SECRET")
        if not self.username:
            missing.append("REDDIT_USERNAME")
        if not self.password:
            missing.append("REDDIT_PASSWORD")
        if not self.subreddits:
            missing.append("REDDIT_SUBREDDITS")
        if missing:
            log.error("reddit required env vars missing", missing=missing)
            raise SystemExit(2)

        # Reject the placeholder UA. Reddit's API rules require a real
        # `by /u/<maintainer>` handle and treat fake/impersonating UAs
        # as ban-worthy. The default ships with `/u/librefang-bot`
        # which is NOT a real account, so refusing to start forces the
        # operator to configure REDDIT_USER_AGENT — the single biggest
        # ban-avoidance lever and one we can enforce at boot time.
        ua_lc = self.user_agent.lower()
        if any(frag in ua_lc for frag in PLACEHOLDER_UA_FRAGMENTS):
            log.error(
                "REDDIT_USER_AGENT contains a placeholder username; "
                "Reddit's API guidelines require a real /u/<maintainer> "
                "in the UA, otherwise the bot risks an IP+account ban. "
                "Set REDDIT_USER_AGENT to e.g. "
                "'myorg/1.0 (by /u/myactual-reddit-username)' before starting.",
                user_agent=self.user_agent,
            )
            raise SystemExit(2)

        # OAuth2 token cache: (access_token, monotonic_expiry_seconds).
        self._cached_token: tuple[str, float] | None = None
        # Discovered at startup via _verify_credentials().
        self.own_username: str = ""
        # Dedupe set for already-seen comment IDs. Capped by
        # SEEN_COMMENTS_MAX with crude oldest-half eviction (matches
        # the Rust adapter's eviction policy).
        self._seen_comments: list[str] = []
        self._seen_comments_set: set[str] = set()

    # ---- HTTP helpers ------------------------------------------------

    def _headers(self, *, bearer: str | None = None,
                 extra: dict | None = None) -> dict:
        h = {"User-Agent": self.user_agent}
        if bearer:
            h["Authorization"] = f"Bearer {bearer}"
        if extra:
            h.update(extra)
        return h

    def _http(
        self,
        url: str,
        *,
        method: str = "GET",
        body: bytes | None = None,
        headers: dict | None = None,
        timeout: float = SEND_TIMEOUT_SECS,
    ) -> tuple[int, dict | None, bytes, dict]:
        """Issue an HTTP request and return
        ``(status, parsed_json_or_None, raw_body, response_headers)``.
        Response header keys are normalised to lowercase so callers
        can do case-insensitive lookups. Raises ``urllib.error.URLError``
        on transport failure; ``HTTPError`` is captured and surfaced via
        the status code so callers can branch on 401 / 4xx / 5xx
        without try/except (response headers are still returned on
        HTTPError, which is what makes ``Retry-After`` / Reddit's
        ``X-Ratelimit-*`` headers reachable after a 429)."""
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

    # ---- Reddit API rate-limit handling ------------------------------

    @staticmethod
    def _ratelimit_pause(resp_headers: dict) -> float:
        """Return seconds to sleep based on Reddit's ratelimit headers.
        Returns ``0.0`` when budget is healthy or headers are missing.
        Capped at ``MAX_BACKOFF_SECS`` so a misreported reset can't
        block the poller for more than a minute."""
        remaining_raw = resp_headers.get("x-ratelimit-remaining")
        if not remaining_raw:
            return 0.0
        try:
            remaining = float(remaining_raw)
        except (TypeError, ValueError):
            return 0.0
        if remaining >= RATELIMIT_REMAINING_FLOOR:
            return 0.0
        reset_raw = resp_headers.get("x-ratelimit-reset")
        try:
            reset = float(reset_raw) if reset_raw else MAX_BACKOFF_SECS
        except (TypeError, ValueError):
            reset = MAX_BACKOFF_SECS
        return min(max(reset, 1.0), MAX_BACKOFF_SECS)

    @staticmethod
    def _retry_after_secs(resp_headers: dict) -> float:
        """Parse ``Retry-After`` (seconds form). Falls back to
        ``RETRY_AFTER_DEFAULT_SECS`` if absent / unparseable; capped at
        ``MAX_BACKOFF_SECS``. We don't support the HTTP-date form —
        Reddit always sends seconds for rate-limit replies."""
        raw = resp_headers.get("retry-after")
        if not raw:
            return RETRY_AFTER_DEFAULT_SECS
        try:
            return min(max(float(raw), 1.0), MAX_BACKOFF_SECS)
        except (TypeError, ValueError):
            return RETRY_AFTER_DEFAULT_SECS

    # ---- OAuth2 token management -------------------------------------

    def _basic_auth_header(self) -> str:
        creds = f"{self.client_id}:{self.client_secret}".encode("utf-8")
        return "Basic " + base64.b64encode(creds).decode("ascii")

    def _fetch_token(self) -> tuple[str, float]:
        """Mint a fresh OAuth2 bearer via the password grant. Returns
        ``(access_token, monotonic_expiry)``. Raises ``RuntimeError``
        on non-200 / missing-field response."""
        body = urllib.parse.urlencode({
            "grant_type": "password",
            "username": self.username,
            "password": self.password,
        }).encode("utf-8")
        headers = self._headers(extra={
            "Authorization": self._basic_auth_header(),
            "Content-Type": "application/x-www-form-urlencoded",
        })
        status, resp, raw, _hdrs = self._http(
            self.token_url, method="POST", body=body, headers=headers,
        )
        if status != 200 or not isinstance(resp, dict):
            snippet = raw[:200].decode("utf-8", "replace")
            raise RuntimeError(
                f"reddit OAuth2 token error {status}: {snippet}"
            )
        access = resp.get("access_token")
        if not isinstance(access, str) or not access:
            raise RuntimeError("reddit OAuth2: missing access_token")
        expires_in = resp.get("expires_in")
        if not isinstance(expires_in, (int, float)) or expires_in <= 0:
            expires_in = 3600
        expiry = time.monotonic() + max(
            float(expires_in) - TOKEN_REFRESH_BUFFER_SECS, 1.0,
        )
        return access, expiry

    def _get_token(self) -> str:
        """Return a valid bearer token, fetching / refreshing as needed."""
        if self._cached_token is not None:
            token, expiry = self._cached_token
            if time.monotonic() < expiry:
                return token
        token, expiry = self._fetch_token()
        self._cached_token = (token, expiry)
        return token

    def _verify_credentials(self) -> str:
        """Call ``GET /api/v1/me`` to validate the token and discover the
        bot's own username (used to skip self-comments). Returns the
        username for logging."""
        token = self._get_token()
        url = f"{self.api_base}/api/v1/me"
        status, resp, raw, _hdrs = self._http(url, headers=self._headers(bearer=token))
        if status != 200 or not isinstance(resp, dict):
            snippet = raw[:200].decode("utf-8", "replace")
            raise RuntimeError(
                f"reddit authentication failed {status}: {snippet}"
            )
        name = str(resp.get("name") or "")
        if not name:
            raise RuntimeError("reddit /api/v1/me: missing name field")
        self.own_username = name
        return name

    # ---- inbound: poll new comments per subreddit --------------------

    def _mark_seen(self, comment_id: str) -> None:
        """Track a comment ID with crude oldest-half eviction on
        overflow. Mirrors the Rust adapter's eviction (it sorted by
        insertion order via HashMap iteration; we use an explicit
        list so the eviction is deterministic)."""
        if comment_id in self._seen_comments_set:
            return
        self._seen_comments.append(comment_id)
        self._seen_comments_set.add(comment_id)
        if len(self._seen_comments) > SEEN_COMMENTS_MAX:
            to_drop = self._seen_comments[:SEEN_COMMENTS_EVICT]
            self._seen_comments = self._seen_comments[SEEN_COMMENTS_EVICT:]
            self._seen_comments_set.difference_update(to_drop)

    def _poll_once(self, emit) -> None:
        """Poll every configured subreddit once. Errors per-subreddit
        are logged and skipped — one bad subreddit doesn't take the
        whole adapter down. Raises only on auth / token errors which
        the caller handles with backoff.

        After every request we check Reddit's ``X-Ratelimit-Remaining``
        header and, when the remaining budget falls below
        ``RATELIMIT_REMAINING_FLOOR``, sleep until ``X-Ratelimit-Reset``
        (capped at ``MAX_BACKOFF_SECS``) before issuing the next
        sub-fetch. A 429 with ``Retry-After`` is honoured the same way.
        This converts "burn budget then get 429" into a smooth
        slow-down, which is what Reddit's anti-abuse logic actually
        wants to see from a well-behaved bot."""
        token = self._get_token()
        for sub in self.subreddits:
            url = f"{self.api_base}/r/{sub}/comments?limit=25&sort=new"
            try:
                status, body, raw, resp_hdrs = self._http(
                    url, headers=self._headers(bearer=token),
                )
            except urllib.error.URLError as e:
                log.warn("reddit comment fetch transport error",
                         subreddit=sub, error=str(e))
                continue
            if status == 401:
                # Clear the cached token; caller backs off and retries.
                self._cached_token = None
                raise RuntimeError("reddit 401 — token expired")
            if status == 429:
                # Rate-limited mid-poll. Honour Retry-After (or our
                # default) and bail; the producer loop's backoff will
                # retry the whole poll pass.
                wait = self._retry_after_secs(resp_hdrs)
                log.warn(
                    "reddit 429 rate-limited; will back off and retry",
                    subreddit=sub, retry_after_secs=wait,
                )
                time.sleep(wait)
                raise RuntimeError("reddit 429 — rate-limited")
            if status != 200 or not isinstance(body, dict):
                log.warn("reddit comment fetch failed",
                         subreddit=sub, status=status)
                continue
            # Pre-emptive throttle when Reddit reports we're close to
            # the per-account budget; sleeping here delays the next
            # sub-fetch in this same poll pass and any subsequent
            # POSTs from on_send.
            pause = self._ratelimit_pause(resp_hdrs)
            if pause > 0:
                log.info(
                    "reddit ratelimit near floor; pausing",
                    subreddit=sub,
                    remaining=resp_hdrs.get("x-ratelimit-remaining"),
                    sleep_secs=pause,
                )
                time.sleep(pause)
            children = body.get("data", {}).get("children") if isinstance(
                body.get("data"), dict
            ) else None
            if not isinstance(children, list):
                continue
            # `?sort=new` returns comments newest-first. Emit them
            # oldest-first so a burst caught in one poll reaches the
            # agent in conversation order; the Rust adapter iterated the
            # raw newest-first listing and delivered such bursts
            # backwards.
            for child in reversed(children):
                if not isinstance(child, dict):
                    continue
                comment_id = str(
                    child.get("data", {}).get("id") if isinstance(
                        child.get("data"), dict,
                    ) else ""
                )
                if not comment_id or comment_id in self._seen_comments_set:
                    continue
                ev = _parse_reddit_comment(child, self.own_username)
                if ev is None:
                    # Track the id anyway so we don't reparse on every
                    # poll. Matches Rust behaviour (seen_comments was
                    # written before parse_reddit_comment ran in the
                    # Rust loop, but only when a message was produced;
                    # tracking unconditionally is cheap and avoids
                    # the redundant-parse case).
                    self._mark_seen(comment_id)
                    continue
                self._mark_seen(comment_id)
                if self.account_id is not None:
                    meta = ev["params"].setdefault("metadata", {})
                    meta["account_id"] = self.account_id
                emit(ev)

    def _producer_blocking(self, emit) -> None:
        """Verify credentials then poll forever in this worker thread.
        Mirrors mastodon / bluesky: verify-once with backoff, then
        steady-state poll loop with exponential backoff on errors."""
        verify_backoff = 1.0
        while True:
            try:
                username = self._verify_credentials()
                log.info("reddit authenticated", username=username,
                         subreddits=self.subreddits)
                break
            except Exception as e:  # noqa: BLE001
                log.warn("reddit auth failed; will retry",
                         error=str(e), delay=verify_backoff)
                time.sleep(verify_backoff)
                verify_backoff = min(verify_backoff * 2, MAX_BACKOFF_SECS)

        backoff = 1.0
        while True:
            try:
                self._poll_once(emit)
                backoff = 1.0
            except Exception as e:  # noqa: BLE001
                log.warn("reddit poll error; backing off",
                         error=str(e), delay=backoff)
                time.sleep(backoff)
                backoff = min(backoff * 2, MAX_BACKOFF_SECS)
            time.sleep(self.poll_interval)

    async def produce(self, emit) -> None:
        loop = asyncio.get_event_loop()
        await loop.run_in_executor(None, self._producer_blocking, emit)

    # ---- outbound: POST /api/comment ---------------------------------

    def _post_comment(self, parent_fullname: str, text: str) -> None:
        """Post a single comment reply. Chunks are joined with
        ``CHUNK_JOIN`` because Reddit only allows one reply per parent;
        the final joined body is hard-capped at ``MAX_MESSAGE_LEN`` so
        a long agent response can't 400 the API. The Rust adapter
        joined without the post-join cap and would 400 on any text
        longer than ~10 000 chars."""
        if not parent_fullname:
            raise RuntimeError(
                "reddit on_send: missing parent fullname "
                "(cmd.thread_id was None — daemon must round-trip the "
                "inbound thread_id)"
            )
        chunks = _split_message(text, MAX_MESSAGE_LEN)
        full_text = CHUNK_JOIN.join(chunks)
        if len(full_text) > MAX_MESSAGE_LEN:
            keep = MAX_MESSAGE_LEN - len(TRUNCATION_MARKER)
            full_text = full_text[:keep].rstrip() + TRUNCATION_MARKER
        token = self._get_token()
        url = f"{self.api_base}/api/comment"
        body = urllib.parse.urlencode({
            "api_type": "json",
            "thing_id": parent_fullname,
            "text": full_text,
        }).encode("utf-8")
        headers = self._headers(bearer=token, extra={
            "Content-Type": "application/x-www-form-urlencoded",
        })
        status, resp, raw, resp_hdrs = self._http(
            url, method="POST", body=body, headers=headers,
        )
        if status == 401:
            # Token expired mid-send: refresh once and retry.
            self._cached_token = None
            token = self._get_token()
            headers["Authorization"] = f"Bearer {token}"
            status, resp, raw, resp_hdrs = self._http(
                url, method="POST", body=body, headers=headers,
            )
        if status == 429:
            # Reddit-side rate-limited. Honour Retry-After (or our
            # default) and retry once. A second 429 falls through to
            # the >=300 branch and surfaces the error so the supervisor
            # can decide whether to back off the whole send loop.
            wait = self._retry_after_secs(resp_hdrs)
            log.warn(
                "reddit 429 on /api/comment; sleeping then retrying once",
                retry_after_secs=wait,
            )
            time.sleep(wait)
            status, resp, raw, resp_hdrs = self._http(
                url, method="POST", body=body, headers=headers,
            )
        if status >= 300:
            snippet = raw[:200].decode("utf-8", "replace")
            raise RuntimeError(
                f"reddit comment API error {status}: {snippet}"
            )
        if isinstance(resp, dict):
            errors = resp.get("json", {}).get("errors") if isinstance(
                resp.get("json"), dict,
            ) else None
            if isinstance(errors, list) and errors:
                log.warn("reddit comment API returned errors",
                         errors=errors)

    async def on_send(self, cmd) -> None:
        # Text-only; structured content falls back to a placeholder so
        # the operator still sees something rather than a silent failure
        # (matches the Rust adapter).
        if cmd.content and not (
            isinstance(cmd.content, dict) and "Text" in cmd.content
        ):
            text = "(Unsupported content type — Reddit only supports text replies)"
        else:
            text = cmd.text or ""
        # cmd.thread_id carries the parent fullname (e.g. "t1_abc123")
        # that inbound parsing surfaced. The Rust adapter used
        # user.platform_id for this, but parse_reddit_comment writes
        # the author username there — the only fullname round-trip we
        # can rely on is thread_id (see module docstring P1 note).
        thread_id = getattr(cmd, "thread_id", None)
        if thread_id is not None and not isinstance(thread_id, str):
            thread_id = str(thread_id) if thread_id else None
        await asyncio.get_event_loop().run_in_executor(
            None, self._post_comment, thread_id or "", text,
        )


if __name__ == "__main__":
    run_stdio_main(RedditAdapter)
