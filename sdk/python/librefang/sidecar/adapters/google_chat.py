#!/usr/bin/env python3
"""Google Chat sidecar channel adapter for LibreFang.

Replaces the former in-process Rust ``librefang-channels::google_chat``
adapter, removed in this migration. Stdlib-only — uses ``http.server``
for the webhook listener, ``urllib`` for outbound REST calls, and a
small in-module PEM/RSA implementation for service-account JWT signing
(no ``cryptography`` / ``pyjwt`` / ``google-auth`` dependency).

Behaviour parity with the Rust adapter — every assertion below has a
file/line citation against ``crates/librefang-channels/src/google_chat.rs``
on the pre-migration tree.

* **Auth**: service-account JSON key in ``GOOGLE_CHAT_SERVICE_ACCOUNT_JSON``
  (the full JSON blob, not a path — keeps the env-only contract). Two
  modes, mirroring google_chat.rs:146-196:
    1. JWT-based: PEM ``private_key`` + ``client_email`` → sign RS256
       JWT → exchange at ``token_uri`` for an OAuth2 access token,
       cached until ``expires_in - 300 s``.
    2. Pre-supplied: ``access_token`` field in the JSON; used as-is
       for testing or pre-authorized tokens (no expiry — the token
       is trusted until the operator rotates it).
  Token endpoint URLs are validated against
  ``ALLOWED_TOKEN_URI_PREFIXES`` (mirrors google_chat.rs:33-36) so a
  crafted JSON cannot SSRF to a non-Google endpoint.
* **Scope**: ``https://www.googleapis.com/auth/chat.bot`` only
  (google_chat.rs:30). Pin scope here too; a future feature that needs
  a wider scope adds it explicitly.
* **Inbound (webhook)**: ``POST /webhook`` on
  ``GOOGLE_CHAT_WEBHOOK_PORT`` (default 8090). Parses Google Chat
  webhook payloads:
    - ``type=MESSAGE`` only (other events: ignored with 200 OK).
    - ``space.name`` filter against ``GOOGLE_CHAT_SPACE_IDS`` (comma-
      separated). Empty list = allow all spaces. Mirrors
      google_chat.rs:380-395.
    - ``message.text`` starting with ``/`` becomes a ``Command``
      payload (name + args); everything else is plain text.
* **Outbound (REST)**: ``POST {api_base}/{space_name}/messages`` with
  ``Authorization: Bearer <token>``, mirrors google_chat.rs:294-325.
  Text is chunked at ``MAX_MESSAGE_LEN = 4096`` (google_chat.rs:27).
* **Multi-bot routing**: ``GOOGLE_CHAT_ACCOUNT_ID`` is the
  multi-instance discriminator surfaced in the kernel's
  ``channel_defaults`` keyed as ``google_chat:<account_id>``
  (google_chat.rs:501-509). Optional.

Improvements on top of the Rust adapter:

* **Honest fail mode for missing crypto**: when the service-account
  key supplies a ``private_key`` but Python can't sign RSA-SHA256
  (corrupt key, bad PEM), the adapter logs a single ``error`` line
  and refuses to start — clearer than the Rust adapter, which
  surfaced the same as a generic ``aead::Error``.
* **No tokio runtime tax**: single-threaded HTTPServer + a background
  worker for outbound sends. The daemon decides how many bots to
  spawn.
"""
from __future__ import annotations

import asyncio
import hashlib
import json
import os
import socketserver
import struct
import threading
import time
import urllib.error
import urllib.parse
import urllib.request
from base64 import b64decode, urlsafe_b64encode
from http.server import BaseHTTPRequestHandler
from typing import Any, Callable, Optional

from .. import logging as log
from .. import protocol
from ..protocol import Content, Field, Schema
from ..runtime import SidecarAdapter, run_stdio_main


# ---------------------------------------------------------------------------
# Constants — mirror crates/librefang-channels/src/google_chat.rs:27-92.
# ---------------------------------------------------------------------------

MAX_MESSAGE_LEN = 4096  # google_chat.rs:27
TOKEN_REFRESH_MARGIN_SECS = 300  # google_chat.rs:28
DEFAULT_TOKEN_LIFETIME_SECS = 3600  # google_chat.rs:29
GOOGLE_CHAT_SCOPE = "https://www.googleapis.com/auth/chat.bot"  # :30
GOOGLE_CHAT_API_BASE = "https://chat.googleapis.com/v1"  # :86

# google_chat.rs:33-36 — SSRF allowlist for `token_uri`.
ALLOWED_TOKEN_URI_PREFIXES = (
    "https://oauth2.googleapis.com/",
    "https://accounts.google.com/",
)

# 1 MiB cap on the inbound webhook body — matches what axum's default
# DefaultBodyLimit gives the Rust adapter, within a factor of two.
WEBHOOK_MAX_BODY_BYTES = 1 * 1024 * 1024

HTTP_TIMEOUT_SECS = 30

# Default webhook bind address. `0.0.0.0` matches the established
# sidecar convention (teams / webhook); operators behind a reverse
# proxy override via `GOOGLE_CHAT_BIND_HOST = "127.0.0.1"`.
DEFAULT_BIND_HOST = "0.0.0.0"


# ---------------------------------------------------------------------------
# Token cache
# ---------------------------------------------------------------------------


class _TokenCache:
    """Thread-safe access to the OAuth2 access token + its expiry.

    Mirrors the Rust ``Arc<RwLock<Option<(String, Instant)>>>``
    (google_chat.rs:112). Contention is low — one refresh per ~hour
    per adapter instance — so a plain ``threading.Lock`` suffices."""

    def __init__(self) -> None:
        self._lock = threading.Lock()
        self._token: Optional[str] = None
        self._expiry_at: float = 0.0  # monotonic seconds

    def get(self) -> Optional[str]:
        with self._lock:
            if self._token is None:
                return None
            if time.monotonic() >= self._expiry_at:
                return None
            return self._token

    def set(self, token: str, expires_in_secs: float) -> None:
        with self._lock:
            self._token = token
            self._expiry_at = time.monotonic() + max(
                0.0, expires_in_secs - TOKEN_REFRESH_MARGIN_SECS,
            )

    def clear(self) -> None:
        with self._lock:
            self._token = None
            self._expiry_at = 0.0


# ---------------------------------------------------------------------------
# Service-account key parsing
# ---------------------------------------------------------------------------


class ServiceAccountKey:
    """Fields extracted from a Google service account JSON key file.

    The Rust adapter (google_chat.rs:42-55) zeroizes the
    ``private_key`` on drop; Python has no equivalent, but the
    sidecar process exits when the daemon kills it so the lifetime
    is bounded anyway."""

    __slots__ = ("client_email", "private_key", "token_uri", "access_token")

    def __init__(self, blob: str) -> None:
        try:
            data = json.loads(blob)
        except (TypeError, ValueError) as e:
            raise ValueError(f"invalid service account key JSON: {e}") from e
        if not isinstance(data, dict):
            raise ValueError("service account key must be a JSON object")
        self.client_email: str = str(data.get("client_email") or "")
        self.private_key: str = str(data.get("private_key") or "")
        self.token_uri: str = str(
            data.get("token_uri") or "https://oauth2.googleapis.com/token"
        )
        # Optional pre-supplied access token. The Rust adapter accepts
        # this for "testing or pre-authorized tokens" (google_chat.rs:54).
        at = data.get("access_token")
        self.access_token: Optional[str] = str(at) if at else None


# ---------------------------------------------------------------------------
# Stdlib RSA-SHA256 PKCS#1 v1.5 signer (RS256 JWT)
# ---------------------------------------------------------------------------
#
# Google service-account auth requires signing a JWT with RS256
# (RSASSA-PKCS1-v1_5 over SHA-256). Sidecar SDK policy is stdlib-only,
# so we can't pull in ``cryptography`` / ``pyjwt`` / ``google-auth``.
# What we need:
#   1. Parse a PKCS#8-PEM private key into (n, d).
#   2. Sign with ``int.pow`` (built-in modular exponentiation).
#   3. Wrap the SHA-256 digest in the PKCS#1 v1.5 padding + the
#      SHA-256 DER OID prefix.
#
# Risk: we SIGN, not verify, so the worst case of a bug is "Google
# rejects our JWT" rather than a security hole. The PEM/DER parser
# is intentionally minimal — it understands exactly the PKCS#8
# RSA-PRIVATE-KEY shape that ``gcloud iam service-accounts keys
# create`` emits.


def _b64url(b: bytes) -> str:
    """URL-safe base64 encode without padding (JWT RFC 7515)."""
    return urlsafe_b64encode(b).rstrip(b"=").decode("ascii")


def _read_pem(pem: str, marker: str) -> bytes:
    """Strip the ``-----BEGIN <marker>-----`` / ``-----END <marker>-----``
    headers and decode the inner base64. Tolerates CR/LF and extra
    whitespace.
    """
    begin = f"-----BEGIN {marker}-----"
    end = f"-----END {marker}-----"
    text = pem.replace("\r", "")
    try:
        body = text.split(begin, 1)[1].split(end, 1)[0]
    except IndexError as e:
        raise ValueError(f"PEM missing {begin} / {end} markers") from e
    return b64decode("".join(body.split()))


def _der_read_len(buf: bytes, pos: int) -> tuple[int, int]:
    """Read a DER length field. Returns (length, new_pos)."""
    first = buf[pos]
    pos += 1
    if first & 0x80 == 0:
        return first, pos
    n = first & 0x7F
    if n == 0 or n > 4:
        raise ValueError(f"unsupported DER length form (n={n})")
    length = int.from_bytes(buf[pos : pos + n], "big")
    return length, pos + n


def _der_read_tag(buf: bytes, pos: int, expected: int) -> int:
    """Assert the tag at ``pos`` equals ``expected`` and step past it,
    returning ``new_pos`` AT the start of the length field."""
    if buf[pos] != expected:
        raise ValueError(
            f"DER tag mismatch at offset {pos}: got 0x{buf[pos]:02x}, "
            f"expected 0x{expected:02x}"
        )
    return pos + 1


def _der_read_integer(buf: bytes, pos: int) -> tuple[int, int]:
    """Read an ASN.1 INTEGER. Returns (value, new_pos)."""
    pos = _der_read_tag(buf, pos, 0x02)  # INTEGER
    length, pos = _der_read_len(buf, pos)
    raw = buf[pos : pos + length]
    return int.from_bytes(raw, "big", signed=False), pos + length


def _parse_pkcs8_rsa_private_key(pem: str) -> tuple[int, int]:
    """Parse a PKCS#8 ``PRIVATE KEY`` PEM and return ``(n, d)``.

    Only handles the ``ssh-keygen``-style and ``gcloud iam
    service-accounts keys create``-style PKCS#8 wrapper around an
    RSAPrivateKey (PKCS#1 §A.1.2). Other algorithms (EC, DSA, Ed25519)
    raise ``ValueError`` — Google service-account keys are always RSA.
    """
    der = _read_pem(pem, "PRIVATE KEY")
    # PKCS#8 outer structure:
    #   SEQUENCE {
    #     INTEGER version (0),
    #     SEQUENCE { OID rsaEncryption, NULL },
    #     OCTET STRING privateKey  -- contains the PKCS#1 RSAPrivateKey
    #   }
    pos = _der_read_tag(der, 0, 0x30)  # SEQUENCE
    _outer_len, pos = _der_read_len(der, pos)
    # version
    _ver, pos = _der_read_integer(der, pos)
    # AlgorithmIdentifier — skip past it.
    pos = _der_read_tag(der, pos, 0x30)
    alg_len, pos = _der_read_len(der, pos)
    pos += alg_len
    # OCTET STRING containing the PKCS#1 RSAPrivateKey blob.
    pos = _der_read_tag(der, pos, 0x04)
    pk_len, pos = _der_read_len(der, pos)
    inner = der[pos : pos + pk_len]
    # PKCS#1 RSAPrivateKey:
    #   SEQUENCE {
    #     INTEGER version, modulus n, publicExponent e,
    #     privateExponent d, prime1, prime2, exp1, exp2, coefficient
    #   }
    ipos = _der_read_tag(inner, 0, 0x30)
    _seq_len, ipos = _der_read_len(inner, ipos)
    _ver, ipos = _der_read_integer(inner, ipos)
    n, ipos = _der_read_integer(inner, ipos)
    _e, ipos = _der_read_integer(inner, ipos)
    d, _ipos = _der_read_integer(inner, ipos)
    return n, d


# DER prefix that PKCS#1 v1.5 wraps a SHA-256 digest in (RFC 8017
# §9.2 Notes 1, plus the SHA-256 OID). Constant across all signers.
_PKCS1_SHA256_PREFIX = bytes.fromhex(
    "3031300d060960864801650304020105000420"
)


def _pkcs1_sign_sha256(message: bytes, n: int, d: int) -> bytes:
    """RSASSA-PKCS1-v1_5 sign over SHA-256(message).

    Output length is ``k = ceil(bitlen(n) / 8)`` (RFC 8017 §8.2).
    """
    k = (n.bit_length() + 7) // 8
    digest = hashlib.sha256(message).digest()
    t = _PKCS1_SHA256_PREFIX + digest
    if len(t) > k - 11:
        raise ValueError("RSA modulus too small for SHA-256 PKCS#1 v1.5")
    # EMSA-PKCS1-v1_5 encoded message:
    #   0x00 || 0x01 || PS (0xFF × (k - tLen - 3)) || 0x00 || T
    ps_len = k - len(t) - 3
    em = b"\x00\x01" + (b"\xff" * ps_len) + b"\x00" + t
    m = int.from_bytes(em, "big")
    sig_int = pow(m, d, n)
    return sig_int.to_bytes(k, "big")


def _sign_rs256_jwt(claims: dict, n: int, d: int) -> str:
    """Build a signed RS256 JWT from the given claims + RSA key."""
    header = {"alg": "RS256", "typ": "JWT"}
    header_b64 = _b64url(
        json.dumps(header, separators=(",", ":")).encode("utf-8")
    )
    claims_b64 = _b64url(
        json.dumps(claims, separators=(",", ":")).encode("utf-8")
    )
    signing_input = f"{header_b64}.{claims_b64}".encode("ascii")
    sig = _pkcs1_sign_sha256(signing_input, n, d)
    return f"{header_b64}.{claims_b64}.{_b64url(sig)}"


# ---------------------------------------------------------------------------
# OAuth2 token exchange
# ---------------------------------------------------------------------------


def _exchange_jwt_for_token(token_uri: str, jwt: str) -> tuple[str, float]:
    """POST the JWT assertion to ``token_uri`` and return
    ``(access_token, expires_in_secs)``. Raises ``RuntimeError`` on
    any HTTP / parse failure so the caller can clear the cache and
    retry on the next send.
    """
    if not any(token_uri.startswith(p) for p in ALLOWED_TOKEN_URI_PREFIXES):
        raise RuntimeError(
            f"untrusted token_uri {token_uri!r}: must start with one of "
            f"{list(ALLOWED_TOKEN_URI_PREFIXES)}"
        )
    body = urllib.parse.urlencode(
        {
            "grant_type": "urn:ietf:params:oauth:grant-type:jwt-bearer",
            "assertion": jwt,
        }
    ).encode("ascii")
    req = urllib.request.Request(
        token_uri,
        data=body,
        method="POST",
        headers={
            "Content-Type": "application/x-www-form-urlencoded",
        },
    )
    try:
        with urllib.request.urlopen(req, timeout=HTTP_TIMEOUT_SECS) as resp:
            raw = resp.read()
    except urllib.error.HTTPError as e:
        text = (e.read() or b"").decode("utf-8", errors="replace")
        raise RuntimeError(
            f"token exchange failed ({e.code}): {text}"
        ) from e
    except urllib.error.URLError as e:
        raise RuntimeError(f"token exchange request failed: {e.reason}") from e
    try:
        data = json.loads(raw.decode("utf-8"))
    except (UnicodeDecodeError, ValueError) as e:
        raise RuntimeError(f"token response not JSON: {e}") from e
    access_token = data.get("access_token")
    if not isinstance(access_token, str) or not access_token:
        raise RuntimeError("token response missing access_token field")
    expires_in = data.get("expires_in")
    if not isinstance(expires_in, (int, float)):
        expires_in = DEFAULT_TOKEN_LIFETIME_SECS
    return access_token, float(expires_in)


# ---------------------------------------------------------------------------
# Adapter
# ---------------------------------------------------------------------------


SCHEMA = Schema(
    name="google_chat",
    display_name="Google Chat",
    description=(
        "Google Chat adapter (out-of-process sidecar). Service-account "
        "JWT auth + REST API send, HTTP webhook receive."
    ),
    fields=[
        Field(
            "GOOGLE_CHAT_SERVICE_ACCOUNT_JSON",
            "Service account JSON (the full key file blob)",
            "secret",
            required=True,
            placeholder='{"client_email": "...", "private_key": "-----BEGIN PRIVATE KEY-----..."}',
        ),
        Field(
            "GOOGLE_CHAT_SPACE_IDS",
            "Space IDs (comma-separated, e.g. spaces/AAAA,spaces/BBBB; empty = all spaces)",
            "text",
        ),
        Field(
            "GOOGLE_CHAT_WEBHOOK_PORT",
            "Webhook listen port",
            "text",
            placeholder="8090",
        ),
        Field(
            "GOOGLE_CHAT_BIND_HOST",
            "Bind address (default 0.0.0.0; set 127.0.0.1 behind a reverse proxy)",
            "text",
            advanced=True,
            placeholder=DEFAULT_BIND_HOST,
        ),
        Field(
            "GOOGLE_CHAT_ACCOUNT_ID",
            "Account ID (multi-bot routing — surfaces as google_chat:<id> in channel_defaults)",
            "text",
            advanced=True,
        ),
        Field(
            "GOOGLE_CHAT_API_BASE",
            "API base URL (override for testing only)",
            "text",
            advanced=True,
            placeholder=GOOGLE_CHAT_API_BASE,
        ),
    ],
)


class GoogleChatAdapter(SidecarAdapter):
    """Google Chat sidecar adapter."""

    capabilities = ["thread"]

    SCHEMA = SCHEMA

    def __init__(self) -> None:
        sa_blob = os.environ.get("GOOGLE_CHAT_SERVICE_ACCOUNT_JSON") or ""
        if not sa_blob:
            raise RuntimeError(
                "GOOGLE_CHAT_SERVICE_ACCOUNT_JSON env var is required"
            )
        self._sa = ServiceAccountKey(sa_blob)
        space_csv = os.environ.get("GOOGLE_CHAT_SPACE_IDS") or ""
        self._space_ids = [s.strip() for s in space_csv.split(",") if s.strip()]
        try:
            self._webhook_port = int(
                os.environ.get("GOOGLE_CHAT_WEBHOOK_PORT") or "8090"
            )
        except ValueError as e:
            raise RuntimeError(
                f"GOOGLE_CHAT_WEBHOOK_PORT must be an integer: {e}"
            ) from e
        self._api_base = (
            os.environ.get("GOOGLE_CHAT_API_BASE") or GOOGLE_CHAT_API_BASE
        ).rstrip("/")
        self._bind_host = (
            os.environ.get("GOOGLE_CHAT_BIND_HOST", "").strip() or DEFAULT_BIND_HOST
        )
        self.account_id = os.environ.get("GOOGLE_CHAT_ACCOUNT_ID") or None

        # Pre-parse the RSA key once so a bad PEM fails at startup
        # rather than on the first send. JWT-less mode skips this.
        self._rsa_key: Optional[tuple[int, int]] = None
        if self._sa.private_key and self._sa.client_email:
            try:
                self._rsa_key = _parse_pkcs8_rsa_private_key(
                    self._sa.private_key
                )
            except ValueError as e:
                raise RuntimeError(
                    f"invalid RSA private key in service-account JSON: {e}"
                ) from e
        elif not self._sa.access_token:
            raise RuntimeError(
                "service-account JSON has neither (client_email + private_key) "
                "for JWT auth nor access_token for the pre-supplied path"
            )

        self._token_cache = _TokenCache()
        # Seed the cache with the pre-supplied access_token if that's
        # the only auth source — keeps the JWT-less path simple.
        if self._sa.access_token and self._rsa_key is None:
            self._token_cache.set(
                self._sa.access_token, DEFAULT_TOKEN_LIFETIME_SECS
            )

        # Server handle for clean shutdown.
        self._httpd: Optional[socketserver.ThreadingTCPServer] = None
        self._server_thread: Optional[threading.Thread] = None

    # ---- Token resolution ------------------------------------------

    def _get_access_token(self) -> str:
        cached = self._token_cache.get()
        if cached:
            return cached
        if self._rsa_key is None:
            # No JWT auth configured AND the pre-supplied token expired
            # (only happens after DEFAULT_TOKEN_LIFETIME_SECS). The
            # pre-supplied path doesn't refresh — surface clearly.
            raise RuntimeError(
                "pre-supplied access_token expired and no JWT auth "
                "configured to refresh it"
            )
        n, d = self._rsa_key
        now = int(time.time())
        claims = {
            "iss": self._sa.client_email,
            "sub": self._sa.client_email,
            "scope": GOOGLE_CHAT_SCOPE,
            "aud": self._sa.token_uri,
            "iat": now,
            "exp": now + DEFAULT_TOKEN_LIFETIME_SECS,
        }
        jwt = _sign_rs256_jwt(claims, n, d)
        token, expires_in = _exchange_jwt_for_token(self._sa.token_uri, jwt)
        self._token_cache.set(token, expires_in)
        return token

    # ---- Outbound --------------------------------------------------

    def _send_text(self, space_name: str, text: str) -> None:
        """Send ``text`` to ``space_name`` (e.g. ``spaces/AAAA``).
        Chunks at ``MAX_MESSAGE_LEN`` to mirror google_chat.rs:303-322.
        """
        token = self._get_access_token()
        url = f"{self._api_base}/{space_name}/messages"
        for chunk in _split_message(text, MAX_MESSAGE_LEN):
            body = json.dumps({"text": chunk}).encode("utf-8")
            req = urllib.request.Request(
                url,
                data=body,
                method="POST",
                headers={
                    "Authorization": f"Bearer {token}",
                    "Content-Type": "application/json",
                },
            )
            try:
                with urllib.request.urlopen(
                    req, timeout=HTTP_TIMEOUT_SECS,
                ) as resp:
                    resp.read()
            except urllib.error.HTTPError as e:
                text_body = (e.read() or b"").decode("utf-8", errors="replace")
                # 401 likely means the cached token went stale early —
                # clear and let the next send retry from JWT auth.
                if e.code == 401:
                    self._token_cache.clear()
                raise RuntimeError(
                    f"Google Chat API error {e.code}: {text_body}"
                ) from e
            except urllib.error.URLError as e:
                raise RuntimeError(
                    f"Google Chat send failed: {e.reason}"
                ) from e

    async def on_send(self, cmd) -> None:
        # `Send.channel_id` carries the space name (`spaces/AAAA`),
        # which the framework derived from the inbound message
        # event's `user_id` field (set by `_parse_webhook_event` to
        # `space.name`). Fall back to `cmd.user.platform_id` so the
        # sidecar still works behind a daemon that addresses by
        # user. Mirrors the same fallback in teams.py / whatsapp.py.
        space = cmd.channel_id or (
            cmd.user.get("platform_id") if cmd.user else ""
        ) or ""
        if not space:
            log.warn("google_chat on_send: empty space id, dropping")
            return
        if not space.startswith("spaces/"):
            log.warn(
                "google_chat on_send: channel_id is not a space name, dropping",
                channel_id=space,
            )
            return
        text = cmd.text or ""
        if not text:
            log.debug("google_chat on_send: empty text, dropping")
            return
        # Stdlib HTTP is blocking; offload to a thread so we don't
        # hold the asyncio loop.
        await asyncio.get_running_loop().run_in_executor(
            None, self._send_text, space, text,
        )

    # ---- Inbound (webhook) -----------------------------------------

    async def produce(self, emit) -> None:
        # The framework's `emit` writes to stdout under a
        # `threading.Lock` (sidecar/runtime.py:344-350), so it's
        # safe to call directly from a worker thread — no need for
        # `loop.call_soon_threadsafe`. Matches teams.py / webhook.py.
        def thread_emit(event: dict) -> None:
            # Inject account_id when configured (mirrors
            # google_chat.rs:443-448). `setdefault` so the inbound
            # parser's own account_id (if any future event payload
            # carries one) wins over the adapter's instance id.
            if self.account_id:
                params = event.get("params")
                if isinstance(params, dict):
                    meta = params.setdefault("metadata", {})
                    if isinstance(meta, dict):
                        meta.setdefault("account_id", self.account_id)
            emit(event)

        # ThreadingTCPServer (not single-threaded HTTPServer) so two
        # concurrent webhook POSTs don't serialize behind each other.
        # `daemon_threads` so per-request workers don't block process
        # exit on shutdown; `allow_reuse_address` so a restart that
        # left the socket in TIME_WAIT can re-bind without EADDRINUSE.
        # Matches the established pattern from teams.py / webhook.py.
        class _ReusingServer(socketserver.ThreadingTCPServer):
            allow_reuse_address = True
            daemon_threads = True

        handler_cls = _make_webhook_handler(self, thread_emit)
        try:
            self._httpd = _ReusingServer(
                (self._bind_host, self._webhook_port), handler_cls,
            )
        except OSError as e:
            # Port in use, permission denied, etc. Log cleanly and
            # let the framework restart the sidecar with backoff
            # instead of crashing with a bare stack trace.
            log.error(
                "google_chat webhook bind failed",
                host=self._bind_host,
                port=self._webhook_port,
                error=str(e),
            )
            return

        log.info(
            "google_chat webhook listening",
            host=self._bind_host,
            port=self._webhook_port,
            api_base=self._api_base,
            spaces=len(self._space_ids),
        )

        def _serve():
            assert self._httpd is not None
            self._httpd.serve_forever()

        self._server_thread = threading.Thread(
            target=_serve, name="google-chat-webhook", daemon=True,
        )
        self._server_thread.start()

        # Block until the framework cancels this coroutine on shutdown.
        # Explicitly catch CancelledError so `_shutdown_server` runs
        # before the cancellation propagates — without this the server
        # thread + listening socket leak past `on_shutdown` (which the
        # framework may not even reach if `produce` raises first).
        try:
            while True:
                await asyncio.sleep(3600)
        except asyncio.CancelledError:
            self._shutdown_server()
            raise

    def _shutdown_server(self) -> None:
        """Drop the listening socket without blocking the caller.

        ``ThreadingTCPServer.shutdown()`` blocks until the
        ``serve_forever()`` loop exits — calling it from an asyncio
        coroutine wedges the event loop. Spawn a daemon thread to
        do the wait so the caller returns immediately. Mirrors
        teams.py's `_shutdown_server` shape.
        """
        httpd = self._httpd
        self._httpd = None
        if httpd is None:
            return
        try:
            threading.Thread(
                target=httpd.shutdown,
                name="google-chat-shutdown",
                daemon=True,
            ).start()
        except Exception:  # noqa: BLE001
            pass

    async def on_shutdown(self) -> None:
        self._shutdown_server()


# ---------------------------------------------------------------------------
# Webhook handler factory
# ---------------------------------------------------------------------------


def _make_webhook_handler(
    adapter: GoogleChatAdapter, emit: Callable[[dict], None],
):
    """Build a BaseHTTPRequestHandler subclass that closes over the
    given adapter + emit callback. Routes ``POST /webhook`` per
    google_chat.rs:344-466.
    """

    class _Handler(BaseHTTPRequestHandler):
        def log_message(self, format, *args):  # noqa: A002
            # Quiet by default — sidecar uses its own structured logger.
            pass

        def do_POST(self):  # noqa: N802 — http.server protocol
            if self.path != "/webhook":
                self.send_response(404)
                self.end_headers()
                return
            try:
                length = int(self.headers.get("Content-Length", "0") or "0")
            except ValueError:
                self.send_response(400)
                self.end_headers()
                return
            if length < 0:
                self.send_response(400)
                self.end_headers()
                return
            if length > WEBHOOK_MAX_BODY_BYTES:
                log.warn(
                    "google_chat webhook rejected oversized body",
                    content_length=length,
                    cap=WEBHOOK_MAX_BODY_BYTES,
                )
                self.send_response(413)
                self.end_headers()
                return
            raw = self.rfile.read(length) if length > 0 else b""
            try:
                payload = json.loads(raw.decode("utf-8")) if raw else {}
            except (UnicodeDecodeError, ValueError):
                self.send_response(400)
                self.end_headers()
                return
            if not isinstance(payload, dict):
                self.send_response(400)
                self.end_headers()
                return

            event = _parse_webhook_event(payload, adapter._space_ids)
            if event is not None:
                emit(event)

            # Always 200 — Google retries non-2xx aggressively. Even
            # rejected events (wrong space, non-MESSAGE type) ack OK
            # so the sender doesn't keep replaying. Mirrors
            # google_chat.rs:382-394.
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", "2")
            self.end_headers()
            self.wfile.write(b"{}")

    return _Handler


def _parse_webhook_event(
    payload: dict, space_ids: list,
) -> Optional[dict]:
    """Translate a Google Chat webhook payload into the sidecar
    ``message`` protocol event. Returns ``None`` when the event
    should be silently dropped (wrong type, empty body, filtered
    space). Mirrors google_chat.rs:372-449.
    """
    event_type = payload.get("type")
    if event_type != "MESSAGE":
        return None
    message = payload.get("message")
    if not isinstance(message, dict):
        return None
    text = message.get("text")
    if not isinstance(text, str) or not text:
        return None

    space = payload.get("space") or {}
    if not isinstance(space, dict):
        return None
    space_name = space.get("name") or ""
    if not isinstance(space_name, str):
        return None
    if space_ids and space_name not in space_ids:
        return None

    sender = message.get("sender") or {}
    if not isinstance(sender, dict):
        sender = {}
    sender_name = sender.get("displayName") or "unknown"
    sender_id = sender.get("name") or "unknown"
    message_name = message.get("name") or ""

    thread = message.get("thread") or {}
    thread_name = thread.get("name") if isinstance(thread, dict) else None

    space_type = space.get("type") or "ROOM"
    is_group = space_type != "DM"

    # `/cmd args...` → Command; otherwise plain text. Mirrors
    # google_chat.rs:405-418.
    if text.startswith("/"):
        head, _, rest = text.partition(" ")
        cmd_name = head[1:]
        args = rest.split() if rest else []
        content = Content.command(cmd_name, args)
    else:
        content = Content.text(text)

    # `sender_id` (the human's `users/<id>`) is a Google-Chat-specific
    # detail that doesn't fit any top-level protocol field — keep it
    # in `metadata` exactly like google_chat.rs:432-438 did. Do NOT
    # stuff `message_id` here; the framework has a top-level
    # `message_id=` kwarg that maps to `ChannelMessage.platform_message_id`
    # so reactions / edits can target the real Google Chat message.
    # And do NOT invent a `channel_label` metadata key — no kernel
    # consumer reads it; the channel identity is already on the
    # sidecar's `channel_type = "google_chat"` declaration.
    metadata = {"sender_id": str(sender_id)}

    event = protocol.message(
        user_id=str(space_name),
        user_name=str(sender_name),
        message_id=str(message_name) if message_name else None,
        content=content,
        is_group=is_group,
        thread_id=thread_name if isinstance(thread_name, str) else None,
        metadata=metadata,
    )
    return event


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------


def _split_message(text: str, max_len: int) -> list:
    """Split ``text`` into ``max_len`` chunks at UTF-8-safe char
    boundaries. Single-line copy of the Rust ``split_message`` helper
    from librefang-channels::types so the sidecar doesn't have to
    import the Rust trait. Tested separately.
    """
    if not text:
        return [text]
    if len(text.encode("utf-8")) <= max_len:
        return [text]
    out = []
    buf = []
    buf_bytes = 0
    for ch in text:
        ch_bytes = len(ch.encode("utf-8"))
        if buf_bytes + ch_bytes > max_len and buf:
            out.append("".join(buf))
            buf = [ch]
            buf_bytes = ch_bytes
        else:
            buf.append(ch)
            buf_bytes += ch_bytes
    if buf:
        out.append("".join(buf))
    return out


# ---------------------------------------------------------------------------
# Entry point
# ---------------------------------------------------------------------------


if __name__ == "__main__":
    run_stdio_main(GoogleChatAdapter)
