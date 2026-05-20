#!/usr/bin/env python3
"""Email (IMAP + SMTP) sidecar channel adapter for LibreFang.

Replaces the former in-process Rust ``librefang-channels::email``
adapter, removed in this migration. Stdlib-only — uses `imaplib`,
`smtplib`, the `email` package, and `ssl`. No third-party deps.

Behaviour parity with the Rust adapter — every assertion has a
file/line citation against ``crates/librefang-channels/src/email.rs``
on the pre-migration tree.

* **IMAP polling**: every ``EMAIL_POLL_INTERVAL_SECS`` (default 30 s)
  the sidecar connects to ``EMAIL_IMAP_HOST:EMAIL_IMAP_PORT`` over
  TLS, runs ``UID SEARCH UNSEEN UNKEYWORD Librefang-Quarantine``
  (falls back to plain ``UNSEEN`` if the server rejects the
  custom keyword) across every configured folder (default
  ``["INBOX"]``), fetches up to 50 UIDs per cycle, and parses each
  body as MIME — mirrors ``email.rs:496-623``.

* **Auth fallback**: ``LOGIN`` first; on failure retries with
  ``AUTHENTICATE PLAIN`` (SASL ``\\0user\\0pass``). Lark / Larksuite
  and some self-hosted servers reject ``LOGIN`` outright and only
  advertise ``AUTH=PLAIN`` — mirrors ``email.rs:515-528``.

* **TLS knobs**: ``EMAIL_TLS_ROOT_CA_PATH`` trusts an extra PEM
  bundle on top of system roots (``email.rs:1041`` + #4877);
  ``EMAIL_TLS_ACCEPT_INVALID_CERTS=1`` disables hostname / chain /
  signature validation entirely and logs a WARN on every connect.

* **Sender allowlist**: exact address or ``@domain`` match;
  substring matches are rejected by design (#3463 — see
  ``email.rs:259-284``). Empty list = allow all.

* **Subject-tag routing**: ``[agent] Subject text`` extracts
  ``agent`` and exposes the cleaned subject. The Rust kernel uses
  the tag itself; the sidecar surfaces both the raw subject and
  the cleaned subject so the bridge can still derive the agent
  hint without re-parsing.

* **Reply threading**: per-sender ``(subject, message_id)`` cache
  is consulted on outbound to set ``In-Reply-To`` + ``References``
  for thread continuity (``email.rs:799-808`` / ``912-932``).

* **MIME body extraction**: walks the parsed tree looking for the
  first ``text/plain`` part; falls back to the first subpart body
  if absent (``email.rs:296-313``).

* **Flag management**: on success → ``+FLAGS (\\Seen)``; on parse
  failure / disallowed sender → ``+FLAGS (\\Seen Librefang-Quarantine)``
  so the same poison-pill / spam doesn't loop forever on the next
  poll (``email.rs:639-642`` + #3481).

* **Outbound send**: SMTP_SSL on port 465 (implicit TLS) or SMTP +
  STARTTLS on 587 (or any other). Supports the
  ``Subject: ...\\n\\nbody`` convention so the bot can override the
  default ``"Re: <original>"`` (``email.rs:903-919``).

Improvements on top of the Rust adapter:

* **inbound dedupe on Message-ID**: Rust marked Seen after emit; a
  flag-update failure could leave a duplicate UNSEEN. The sidecar
  also runs a bounded `SeenSet` on Message-ID so a flag-update
  hiccup or RFC-violating server that delivers the same message
  twice doesn't double-emit.

* **explicit timeouts everywhere**: `imaplib` and `smtplib` allow
  a `timeout` argument on the constructor — the sidecar passes
  ``EMAIL_NET_TIMEOUT_SECS`` (default 60 s) on every connection.
  Rust relied on whatever the IMAP/SMTP crates defaulted to.
"""
from __future__ import annotations

import asyncio
import email
import email.message
import email.utils
import imaplib
import os
import re
import smtplib
import ssl
import threading
from email.message import EmailMessage
from email.parser import BytesParser
from email.policy import default as default_policy
from typing import Any, Callable, Optional

from .. import logging as log
from .. import protocol
from ..common import (
    SeenSet as _SeenSet,
    split_csv as _split_csv,
)
from ..protocol import Content, Field, Schema
from ..runtime import SidecarAdapter, run_stdio_main


# ---------------------------------------------------------------------------
# Constants — mirror crates/librefang-channels/src/email.rs.
# ---------------------------------------------------------------------------

DEFAULT_IMAP_PORT = 993
DEFAULT_SMTP_PORT = 587
DEFAULT_POLL_INTERVAL_SECS = 30
DEFAULT_NET_TIMEOUT_SECS = 60.0
FETCH_BATCH = 50  # email.rs:557 — 50-UID per poll cycle cap

# Quarantine keyword. Servers that don't support custom keywords fall
# back to plain UNSEEN; the sidecar quarantines via \\Seen instead so a
# poison-pill doesn't loop.
QUARANTINE_KEYWORD = "Librefang-Quarantine"

# Dedupe envelope — same shape as recent sidecars (mattermost, signal,
# qq, matrix, wecom).
SEEN_MESSAGES_MAX = 10_000
SEEN_MESSAGES_EVICT = 5_000

INITIAL_BACKOFF_SECS = 1.0
MAX_BACKOFF_SECS = 60.0


# ---------------------------------------------------------------------------
# Pure helpers — easy to unit-test
# ---------------------------------------------------------------------------


_ADDR_ANGLE_RE = re.compile(r"<([^<>]+)>")
_AGENT_TAG_RE = re.compile(r"^\s*\[([^\]]+)\]\s*(.*)$", re.DOTALL)


def extract_email_addr(raw: str) -> str:
    """Extract ``user@domain`` from ``"Name <user@domain>"``-style raw
    From / To strings. Mirrors email.rs:247-257.

    Falls back to the trimmed raw string when there's no angle-bracket
    address part (some servers send bare addresses).
    """
    if not isinstance(raw, str):
        return ""
    raw = raw.strip()
    m = _ADDR_ANGLE_RE.search(raw)
    if m:
        return m.group(1).strip()
    return raw


def sender_matches_allowlist(sender: str, allowed: list[str]) -> bool:
    """Exact-address or ``@domain`` match. Empty allowed list returns
    True (allow-all is the caller's responsibility). Mirrors
    email.rs:259-284 — substring matches REJECTED by design (#3463)."""
    addr = extract_email_addr(sender).strip()
    if not addr or "@" not in addr:
        return False
    domain = addr.rsplit("@", 1)[-1]
    if not domain:
        return False
    addr_lower = addr.lower()
    domain_lower = domain.lower()
    for entry in allowed:
        e = entry.strip()
        if not e:
            continue
        if e.startswith("@"):
            d = e[1:]
            if d and d.lower() == domain_lower:
                return True
        elif addr_lower == e.lower():
            return True
    return False


def extract_agent_from_subject(subject: str) -> Optional[str]:
    """``"[coder] Fix the bug"`` → ``"coder"``. Returns None when no
    bracket tag is present. Mirrors email.rs:184-195."""
    if not isinstance(subject, str):
        return None
    m = _AGENT_TAG_RE.match(subject)
    if not m:
        return None
    name = m.group(1).strip()
    return name or None


def strip_agent_tag(subject: str) -> str:
    """Inverse of :func:`extract_agent_from_subject`. Returns the
    subject with any leading ``[tag]`` removed. Mirrors
    email.rs:198-206."""
    if not isinstance(subject, str):
        return ""
    m = _AGENT_TAG_RE.match(subject)
    if m:
        return m.group(2).strip()
    return subject.strip()


def extract_text_body(msg: email.message.Message) -> str:
    """Walk a parsed MIME tree looking for the first ``text/plain``
    part. Falls back to the first subpart's payload. Returns empty
    string on failure. Mirrors email.rs:296-313."""
    try:
        if msg.is_multipart():
            for part in msg.walk():
                if part.is_multipart():
                    continue
                ctype = (part.get_content_type() or "").lower()
                if ctype == "text/plain":
                    payload = part.get_payload(decode=True)
                    if payload is None:
                        continue
                    charset = part.get_content_charset() or "utf-8"
                    try:
                        return payload.decode(charset, errors="replace")
                    except (LookupError, AttributeError):
                        return payload.decode("utf-8", errors="replace")
            # Fallback to first subpart
            for part in msg.walk():
                if part.is_multipart():
                    continue
                payload = part.get_payload(decode=True)
                if payload is not None:
                    charset = part.get_content_charset() or "utf-8"
                    try:
                        return payload.decode(charset, errors="replace")
                    except (LookupError, AttributeError):
                        return payload.decode("utf-8", errors="replace")
            return ""
        payload = msg.get_payload(decode=True)
        if payload is None:
            return ""
        charset = msg.get_content_charset() or "utf-8"
        try:
            return payload.decode(charset, errors="replace")
        except (LookupError, AttributeError):
            return payload.decode("utf-8", errors="replace")
    except Exception:  # noqa: BLE001 — never raise out of body extraction
        return ""


def parse_email_message(raw_bytes: bytes) -> Optional[dict]:
    """Parse a raw RFC-822 byte blob into ``{from, subject, message_id,
    body}``. Returns None on malformed input (mirrors the
    quarantine-on-parse-fail path at email.rs:584-590)."""
    if not isinstance(raw_bytes, (bytes, bytearray)) or not raw_bytes:
        return None
    try:
        msg = BytesParser(policy=default_policy).parsebytes(bytes(raw_bytes))
    except Exception:  # noqa: BLE001
        return None
    from_hdr = msg.get("From", "") or ""
    subject_hdr = msg.get("Subject", "") or ""
    message_id_hdr = msg.get("Message-ID", "") or ""
    return {
        "from_addr": extract_email_addr(str(from_hdr)),
        "subject": str(subject_hdr),
        "message_id": str(message_id_hdr).strip(),
        "body": extract_text_body(msg),
    }


def build_outbound_subject(
    text: str,
    *,
    reply_subject: Optional[str],
    default_subject: str = "LibreFang Reply",
) -> tuple[str, str]:
    """Split a ``"Subject: <subj>\\n\\n<body>"``-prefixed text into
    ``(subject, body)``. When no prefix is present, derive the subject
    from the reply-context (``"Re: <subj>"``) or fall back to
    ``default_subject``. Mirrors email.rs:902-919."""
    if not isinstance(text, str):
        return default_subject, ""
    if text.startswith("Subject: "):
        sep_idx = text.find("\n\n")
        if sep_idx > 0:
            subj = text[len("Subject: "):sep_idx].strip()
            body = text[sep_idx + 2:]
            if subj:
                return subj, body
        # Header without body separator — treat whole thing as body.
        return default_subject, text
    if reply_subject:
        return f"Re: {reply_subject}", text
    return default_subject, text


# ---------------------------------------------------------------------------
# Reply context cache — last (subject, message_id) per sender.
# ---------------------------------------------------------------------------


class _ReplyCtxCache:
    """Thread-safe ``from_addr → (subject, message_id)``. Mirrors the
    Rust adapter's ``DashMap<String, ReplyCtx>`` at
    ``email.rs:50-53``. No eviction — sized like the Rust version,
    which also grew unbounded; in practice this is bounded by the set
    of senders the bot interacts with, which is small."""

    def __init__(self) -> None:
        self._lock = threading.Lock()
        self._data: dict[str, tuple[str, str]] = {}

    def store(self, from_addr: str, subject: str, message_id: str) -> None:
        if not from_addr:
            return
        with self._lock:
            self._data[from_addr.lower()] = (subject, message_id)

    def get(self, from_addr: str) -> Optional[tuple[str, str]]:
        with self._lock:
            return self._data.get(from_addr.lower())


# ---------------------------------------------------------------------------
# Adapter
# ---------------------------------------------------------------------------


class EmailAdapter(SidecarAdapter):
    """Email (IMAP + SMTP) sidecar adapter.

    Text-only outbound (matches Rust adapter's ``send`` at
    ``email.rs:884-958``: any non-Text content logs a warn and is
    dropped).
    """

    capabilities: list = []
    suppress_error_responses: bool = False

    SCHEMA = Schema(
        name="email",
        display_name="Email (IMAP + SMTP)",
        description=(
            "Polls an IMAP mailbox for new messages and replies via "
            "SMTP. Stdlib-only sidecar — no third-party deps."
        ),
        fields=[
            Field("EMAIL_IMAP_HOST", "IMAP host", "text",
                  required=True,
                  placeholder="imap.example.com"),
            Field("EMAIL_IMAP_PORT", "IMAP port", "text",
                  placeholder=str(DEFAULT_IMAP_PORT),
                  advanced=True),
            Field("EMAIL_SMTP_HOST", "SMTP host", "text",
                  required=True,
                  placeholder="smtp.example.com"),
            Field("EMAIL_SMTP_PORT", "SMTP port", "text",
                  placeholder=str(DEFAULT_SMTP_PORT),
                  advanced=True),
            Field("EMAIL_USERNAME", "Email address (username)", "text",
                  required=True,
                  placeholder="bot@example.com"),
            Field("EMAIL_PASSWORD", "Account password", "secret",
                  required=True),
            Field("EMAIL_IMAP_USERNAME",
                  "IMAP-specific username (falls back to EMAIL_USERNAME)",
                  "text", advanced=True),
            Field("EMAIL_IMAP_PASSWORD",
                  "IMAP-specific password (falls back to EMAIL_PASSWORD)",
                  "secret", advanced=True),
            Field("EMAIL_SMTP_USERNAME",
                  "SMTP-specific username (falls back to EMAIL_USERNAME)",
                  "text", advanced=True),
            Field("EMAIL_SMTP_PASSWORD",
                  "SMTP-specific password (falls back to EMAIL_PASSWORD)",
                  "secret", advanced=True),
            Field("EMAIL_POLL_INTERVAL_SECS", "Poll interval (seconds)",
                  "text",
                  placeholder=str(DEFAULT_POLL_INTERVAL_SECS),
                  advanced=True),
            Field("EMAIL_FOLDERS",
                  "IMAP folders to monitor (comma-separated)",
                  "text",
                  placeholder="INBOX",
                  advanced=True),
            Field("EMAIL_ALLOWED_SENDERS",
                  "Allowed sender addresses or @domain "
                  "(comma-separated, empty = all)",
                  "text",
                  placeholder="alice@example.com,@trusted.com",
                  advanced=True),
            Field("EMAIL_ACCOUNT_ID",
                  "Account ID (multi-bot routing)", "text",
                  advanced=True),
            Field("EMAIL_TLS_ROOT_CA_PATH",
                  "Custom CA bundle (PEM) for IMAP TLS",
                  "text", advanced=True),
            Field("EMAIL_TLS_ACCEPT_INVALID_CERTS",
                  "Disable IMAP TLS validation (1/true) — DANGEROUS",
                  "text", advanced=True),
            Field("EMAIL_NET_TIMEOUT_SECS",
                  "Network timeout (seconds)", "text",
                  placeholder=str(int(DEFAULT_NET_TIMEOUT_SECS)),
                  advanced=True),
        ],
    )

    def __init__(self) -> None:
        imap_host = os.environ.get("EMAIL_IMAP_HOST", "").strip()
        smtp_host = os.environ.get("EMAIL_SMTP_HOST", "").strip()
        username = os.environ.get("EMAIL_USERNAME", "").strip()
        password = os.environ.get("EMAIL_PASSWORD", "")
        missing: list[str] = []
        if not imap_host:
            missing.append("EMAIL_IMAP_HOST")
        if not smtp_host:
            missing.append("EMAIL_SMTP_HOST")
        if not username:
            missing.append("EMAIL_USERNAME")
        if not password:
            missing.append("EMAIL_PASSWORD")
        if missing:
            log.error("email required env vars missing", missing=missing)
            raise SystemExit(2)

        self.imap_host = imap_host
        self.smtp_host = smtp_host
        self.imap_port = _env_int("EMAIL_IMAP_PORT", DEFAULT_IMAP_PORT)
        self.smtp_port = _env_int("EMAIL_SMTP_PORT", DEFAULT_SMTP_PORT)

        # Per-protocol overrides with global fallback (Rust parity:
        # imap_username / smtp_username / imap_password_env /
        # smtp_password_env fields all fall back to username /
        # password_env in `channel_bridge.rs`).
        self.imap_username = (
            os.environ.get("EMAIL_IMAP_USERNAME", "").strip() or username
        )
        self.imap_password = (
            os.environ.get("EMAIL_IMAP_PASSWORD", "") or password
        )
        self.smtp_username = (
            os.environ.get("EMAIL_SMTP_USERNAME", "").strip() or username
        )
        self.smtp_password = (
            os.environ.get("EMAIL_SMTP_PASSWORD", "") or password
        )
        self.from_address = username

        self.poll_interval_secs = max(
            5, _env_int("EMAIL_POLL_INTERVAL_SECS",
                        DEFAULT_POLL_INTERVAL_SECS),
        )
        self.net_timeout_secs = float(_env_int(
            "EMAIL_NET_TIMEOUT_SECS", int(DEFAULT_NET_TIMEOUT_SECS),
        ))

        folders_raw = _split_csv(os.environ.get("EMAIL_FOLDERS", ""))
        self.folders = folders_raw if folders_raw else ["INBOX"]
        self.allowed_senders = _split_csv(
            os.environ.get("EMAIL_ALLOWED_SENDERS", "")
        )

        acct = os.environ.get("EMAIL_ACCOUNT_ID", "").strip()
        self.account_id: Optional[str] = acct or None

        self.tls_root_ca_path = (
            os.environ.get("EMAIL_TLS_ROOT_CA_PATH", "").strip() or None
        )
        self.tls_accept_invalid_certs = _env_bool(
            "EMAIL_TLS_ACCEPT_INVALID_CERTS",
        )

        self._reply_ctx = _ReplyCtxCache()
        self._seen = _SeenSet(
            max_size=SEEN_MESSAGES_MAX, evict=SEEN_MESSAGES_EVICT,
        )
        self._shutdown = threading.Event()

    # ---- TLS context --------------------------------------------------

    def _build_ssl_context(self) -> ssl.SSLContext:
        if self.tls_accept_invalid_certs:
            # Always WARN, never just log on startup — operators forget
            # they enabled this (email.rs:336-337).
            log.warn(
                "email IMAP TLS validation is DISABLED — "
                "mailbox is exposed to MITM",
                host=self.imap_host,
            )
            ctx = ssl._create_unverified_context()  # noqa: SLF001 — stdlib API
            return ctx
        if self.tls_root_ca_path:
            return ssl.create_default_context(
                cafile=self.tls_root_ca_path,
            )
        return ssl.create_default_context()

    # ---- IMAP poll ----------------------------------------------------

    def _imap_login(self, conn: "imaplib.IMAP4") -> None:
        """LOGIN first; fall back to AUTHENTICATE PLAIN if the server
        rejects LOGIN (Lark / Larksuite + some self-hosted servers).
        Mirrors email.rs:515-528."""
        try:
            conn.login(self.imap_username, self.imap_password)
            return
        except imaplib.IMAP4.error as login_err:
            log.debug(
                "email IMAP LOGIN failed, falling back to AUTH=PLAIN",
                error=str(login_err),
            )

        def _plain_responder(_challenge: bytes) -> bytes:
            return (
                "\0" + self.imap_username + "\0" + self.imap_password
            ).encode("utf-8")

        try:
            conn.authenticate("PLAIN", _plain_responder)
        except imaplib.IMAP4.error as e:
            raise RuntimeError(
                f"IMAP login failed (both LOGIN and AUTH=PLAIN): {e}",
            ) from e

    def _connect_imap(self) -> "imaplib.IMAP4_SSL":
        ctx = self._build_ssl_context()
        conn = imaplib.IMAP4_SSL(
            self.imap_host,
            self.imap_port,
            ssl_context=ctx,
            timeout=self.net_timeout_secs,
        )
        self._imap_login(conn)
        return conn

    def _fetch_unseen(
        self, conn: "imaplib.IMAP4_SSL",
    ) -> list[tuple[str, int, dict]]:
        """Return ``[(folder, uid, parsed_email)]``. ``parsed_email`` is
        ``None`` when the body failed to parse (caller quarantines)."""
        results: list[tuple[str, int, Optional[dict]]] = []
        for folder in self.folders:
            typ, _ = conn.select(_imap_folder_arg(folder), readonly=False)
            if typ != "OK":
                log.warn(
                    "email IMAP SELECT failed, skipping folder",
                    folder=folder, response=typ,
                )
                continue

            # Try the keyword-aware search first; fall back to plain
            # UNSEEN if the server rejects custom keyword search
            # (email.rs:540-549).
            typ, data = conn.uid(
                "SEARCH", None, "UNSEEN", "UNKEYWORD", QUARANTINE_KEYWORD,
            )
            if typ != "OK":
                typ, data = conn.uid("SEARCH", None, "UNSEEN")
            if typ != "OK":
                log.warn(
                    "email IMAP SEARCH UNSEEN failed",
                    folder=folder, response=typ,
                )
                continue

            uids = _parse_uid_search(data)
            if not uids:
                log.debug("email no unseen", folder=folder)
                continue
            uids = uids[:FETCH_BATCH]

            uid_set = ",".join(str(u) for u in uids)
            typ, fetch_data = conn.uid("FETCH", uid_set, "(UID RFC822)")
            if typ != "OK":
                log.warn(
                    "email IMAP FETCH failed",
                    folder=folder, response=typ,
                )
                continue
            parsed_by_uid = _parse_fetch_response(fetch_data)
            for uid in uids:
                raw = parsed_by_uid.get(uid)
                if raw is None:
                    results.append((folder, uid, None))
                    continue
                parsed = parse_email_message(raw)
                results.append((folder, uid, parsed))
        return results

    def _mark_uids(
        self,
        conn: "imaplib.IMAP4_SSL",
        folder: str,
        uids_seen: list[int],
        uids_quarantined: list[int],
    ) -> None:
        if not uids_seen and not uids_quarantined:
            return
        # Need to SELECT the folder writable to update flags.
        typ, _ = conn.select(_imap_folder_arg(folder), readonly=False)
        if typ != "OK":
            log.warn("email IMAP SELECT for STORE failed", folder=folder)
            return
        if uids_seen:
            uid_set = ",".join(str(u) for u in uids_seen)
            typ, _ = conn.uid("STORE", uid_set, "+FLAGS", "(\\Seen)")
            if typ != "OK":
                log.warn("email IMAP STORE \\Seen failed", uids=uid_set)
        if uids_quarantined:
            uid_set = ",".join(str(u) for u in uids_quarantined)
            typ, _ = conn.uid(
                "STORE", uid_set,
                "+FLAGS", f"(\\Seen {QUARANTINE_KEYWORD})",
            )
            if typ != "OK":
                log.warn(
                    "email IMAP STORE quarantine failed", uids=uid_set,
                )

    # ---- SMTP send ----------------------------------------------------

    def _send_email(
        self,
        to_addr: str,
        subject: str,
        body: str,
        *,
        in_reply_to: Optional[str] = None,
    ) -> None:
        """Build a `text/plain` EmailMessage and send via SMTP.
        Picks SMTP_SSL on 465 and SMTP + STARTTLS otherwise."""
        msg = EmailMessage()
        msg["From"] = self.from_address
        msg["To"] = to_addr
        msg["Subject"] = subject
        msg["Date"] = email.utils.formatdate(localtime=True)
        # Generate a unique Message-ID for our outbound so the
        # downstream MUA can thread our replies too.
        msg["Message-ID"] = email.utils.make_msgid(
            domain=self.from_address.rsplit("@", 1)[-1] or "librefang.local",
        )
        if in_reply_to:
            msg["In-Reply-To"] = in_reply_to
            msg["References"] = in_reply_to
        msg.set_content(body)

        if self.smtp_port == 465:
            ctx = ssl.create_default_context()
            with smtplib.SMTP_SSL(
                self.smtp_host, self.smtp_port,
                context=ctx, timeout=self.net_timeout_secs,
            ) as smtp:
                smtp.login(self.smtp_username, self.smtp_password)
                smtp.send_message(msg)
        else:
            with smtplib.SMTP(
                self.smtp_host, self.smtp_port,
                timeout=self.net_timeout_secs,
            ) as smtp:
                smtp.ehlo()
                if smtp.has_extn("starttls"):
                    smtp.starttls(context=ssl.create_default_context())
                    smtp.ehlo()
                smtp.login(self.smtp_username, self.smtp_password)
                smtp.send_message(msg)

    # ---- sidecar surface ---------------------------------------------

    async def produce(self, emit: Callable[[dict], None]) -> None:
        loop = asyncio.get_event_loop()
        await loop.run_in_executor(None, self._poll_loop, emit)

    async def on_shutdown(self) -> None:
        self._shutdown.set()

    async def on_send(self, cmd) -> None:
        to_addr = (
            cmd.channel_id
            or (cmd.user.get("platform_id") if cmd.user else "")
            or ""
        )
        to_addr = extract_email_addr(to_addr)
        if not to_addr or "@" not in to_addr:
            log.warn("email on_send: missing/invalid recipient, dropping",
                     to=to_addr)
            return

        content = cmd.content
        text = cmd.text or ""
        if isinstance(content, dict) and "Text" in content:
            inner = content["Text"]
            if isinstance(inner, str):
                text = inner
        elif content and not (isinstance(content, dict) and "Text" in content):
            log.warn(
                "email on_send: unsupported content type, dropping",
                variant=next(iter(content)) if isinstance(content, dict) else type(content).__name__,
            )
            return
        if not text:
            return

        prev = self._reply_ctx.get(to_addr)
        reply_subject = prev[0] if prev else None
        reply_msg_id = prev[1] if prev else None
        subject, body = build_outbound_subject(
            text, reply_subject=reply_subject,
        )

        loop = asyncio.get_event_loop()
        try:
            await loop.run_in_executor(
                None,
                lambda: self._send_email(
                    to_addr, subject, body,
                    in_reply_to=reply_msg_id or None,
                ),
            )
            log.info("email sent", to=to_addr, subject=subject)
        except Exception as e:  # noqa: BLE001 — surface to operator log
            log.error(
                "email SMTP send failed",
                to=to_addr, error=str(e),
            )
            raise

    # ---- IMAP poll loop -----------------------------------------------

    def _poll_loop(self, emit: Callable[[dict], None]) -> None:
        log.info(
            "email starting poll loop",
            imap_host=self.imap_host, imap_port=self.imap_port,
            smtp_host=self.smtp_host, smtp_port=self.smtp_port,
            poll_interval_secs=self.poll_interval_secs,
            folders=self.folders,
        )
        backoff = INITIAL_BACKOFF_SECS
        while not self._shutdown.is_set():
            try:
                self._poll_once(emit)
                backoff = INITIAL_BACKOFF_SECS  # reset on success
            except Exception as e:  # noqa: BLE001 — transport varies
                if self._shutdown.is_set():
                    return
                log.warn(
                    "email poll iteration failed; backing off",
                    error=str(e), delay=backoff,
                )
                if self._shutdown.wait(backoff):
                    return
                backoff = min(backoff * 2.0, MAX_BACKOFF_SECS)
                continue
            # Use Event.wait so shutdown interrupts the sleep promptly.
            if self._shutdown.wait(self.poll_interval_secs):
                return

    def _poll_once(self, emit: Callable[[dict], None]) -> None:
        conn = self._connect_imap()
        try:
            fetched = self._fetch_unseen(conn)
            # Group flag updates by folder so we issue one STORE per
            # folder per outcome.
            by_folder: dict[str, dict[str, list[int]]] = {}

            def _mark(folder: str, uid: int, outcome: str) -> None:
                bucket = by_folder.setdefault(
                    folder, {"seen": [], "quarantined": []},
                )
                bucket[outcome].append(uid)

            for folder, uid, parsed in fetched:
                if parsed is None:
                    log.warn(
                        "email parse failed; quarantining UID",
                        folder=folder, uid=uid,
                    )
                    _mark(folder, uid, "quarantined")
                    continue
                from_addr = parsed["from_addr"]
                if (
                    self.allowed_senders
                    and not sender_matches_allowlist(
                        from_addr, self.allowed_senders,
                    )
                ):
                    log.debug(
                        "email sender not in allowlist; quarantining",
                        from_addr=from_addr,
                    )
                    _mark(folder, uid, "quarantined")
                    continue
                # Dedupe on Message-ID — defense in depth for servers
                # that redeliver despite \\Seen.
                msg_id = parsed["message_id"]
                if msg_id and not self._seen.mark(msg_id):
                    log.debug(
                        "email duplicate Message-ID, dropping",
                        message_id=msg_id,
                    )
                    _mark(folder, uid, "seen")  # idempotent flag-set
                    continue

                subject = parsed["subject"]
                clean_subject = strip_agent_tag(subject)
                body = parsed["body"].rstrip()
                if clean_subject:
                    text = f"Subject: {clean_subject}\n\n{body}"
                else:
                    text = body

                # Stash the reply context BEFORE emit, so a Send back
                # from the daemon can re-use it immediately.
                if msg_id:
                    self._reply_ctx.store(from_addr, subject, msg_id)

                metadata: dict[str, Any] = {}
                if self.account_id is not None:
                    metadata["account_id"] = self.account_id
                target_agent = extract_agent_from_subject(subject)
                if target_agent:
                    metadata["target_agent"] = target_agent
                if clean_subject:
                    metadata["subject"] = clean_subject
                if msg_id:
                    metadata["message_id"] = msg_id

                ev = protocol.message(
                    user_id=from_addr,
                    user_name=from_addr,
                    content=Content.text(text),
                    message_id=msg_id or None,
                    channel_id=from_addr,
                    metadata=metadata,
                )
                emit(ev)
                _mark(folder, uid, "seen")

            for folder, bucket in by_folder.items():
                self._mark_uids(
                    conn, folder, bucket["seen"], bucket["quarantined"],
                )
        finally:
            try:
                conn.logout()
            except Exception:  # noqa: BLE001 — best effort on close
                pass


# ---------------------------------------------------------------------------
# Env helpers
# ---------------------------------------------------------------------------


def _env_int(name: str, default: int) -> int:
    raw = os.environ.get(name, "").strip()
    if not raw:
        return default
    try:
        return int(raw)
    except ValueError:
        log.warn(
            f"email {name} not an integer; using default",
            value=raw, default=default,
        )
        return default


def _env_bool(name: str) -> bool:
    raw = os.environ.get(name, "").strip().lower()
    return raw in ("1", "true", "yes", "on")


def _imap_folder_arg(folder: str) -> str:
    """imaplib's `select` takes a mailbox name; spaces must be wrapped
    in double quotes per RFC-3501. Folders containing literal double
    quotes are dropped by design (server-side names typically don't
    contain them)."""
    name = folder.strip().replace('"', "")
    if " " in name and not (name.startswith('"') and name.endswith('"')):
        return f'"{name}"'
    return name


# Pre-compiled regex used by _parse_fetch_response to find the UID
# inside an IMAP FETCH response key like ``b"1 (UID 42 RFC822 {1234}"``.
# We're parsing the imaplib quirk-y response shape, not the wire
# protocol — `imaplib` already untangles continuations into
# `bytes`-or-`tuple` chunks.
_FETCH_UID_RE = re.compile(rb"UID (\d+)")


def _parse_uid_search(data: list) -> list[int]:
    """Pull integer UIDs out of an IMAP4 UID SEARCH response."""
    if not data:
        return []
    blob = data[0]
    if blob is None:
        return []
    if isinstance(blob, bytes):
        text = blob
    else:
        text = bytes(blob)
    out: list[int] = []
    for tok in text.split():
        try:
            out.append(int(tok))
        except (ValueError, TypeError):
            continue
    return out


def _parse_fetch_response(data: list) -> dict[int, bytes]:
    """Pull ``{uid: raw_rfc822_bytes}`` out of an IMAP4 UID FETCH
    response. imaplib parses each fetch entry as a 2-tuple
    ``(metadata, body_bytes)`` interleaved with closing-paren bytes;
    we walk the list looking for tuples and pair the UID from the
    metadata header with the body bytes."""
    out: dict[int, bytes] = {}
    if not isinstance(data, list):
        return out
    for entry in data:
        if not isinstance(entry, tuple) or len(entry) != 2:
            continue
        meta, body = entry
        if not isinstance(meta, (bytes, bytearray)) or not isinstance(
            body, (bytes, bytearray),
        ):
            continue
        m = _FETCH_UID_RE.search(bytes(meta))
        if not m:
            continue
        try:
            uid = int(m.group(1))
        except (ValueError, TypeError):
            continue
        out[uid] = bytes(body)
    return out


if __name__ == "__main__":
    run_stdio_main(EmailAdapter)
