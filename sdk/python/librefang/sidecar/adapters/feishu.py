#!/usr/bin/env python3
"""Feishu / Lark sidecar channel adapter for LibreFang.

Replaces the former in-process Rust ``librefang-channels::feishu``
adapter, removed in this migration. Unified support for both regions
(Feishu CN: ``open.feishu.cn``; Lark intl: ``open.larksuite.com``)
and both receive modes (WebSocket gateway / HTTP webhook).

Behaviour parity with the Rust adapter — every assertion below has
a file/line citation against ``crates/librefang-channels/src/feishu.rs``
on the pre-migration tree.

* **Region + auth probe**: ``POST /open-apis/auth/v3/tenant_access_token/internal``
  with ``app_id`` + ``app_secret`` gets a tenant token (default
  expiry 7200 s, refreshed at ``expire - 300 s`` per feishu.rs:111).
  Validation hits ``GET /open-apis/bot/v3/info`` to confirm
  credentials (feishu.rs:273-303).

* **WebSocket** (``FEISHU_RECEIVE_MODE=websocket``, default): two-step
  endpoint discovery. ``POST /callback/ws/endpoint`` with
  ``{AppID, AppSecret}`` returns the real ``wss://`` URL +
  ``ClientConfig.PingInterval`` (feishu.rs:982-1017). The adapter
  connects to that URL, sends ``{"type":"ping"}`` JSON frames at
  the server-supplied interval (default 120 s, matching the official
  Go SDK), and parses both text and binary frames. Binary frames
  carry protobuf-wrapped JSON; the adapter slices out the embedded
  JSON object the same way Rust did (find first ``{``, last ``}``).
  Reconnect: 2 s → 60 s exponential backoff
  (``WS_INITIAL_BACKOFF`` / ``WS_MAX_BACKOFF`` at feishu.rs:114-117).

* **Webhook** (``FEISHU_RECEIVE_MODE=webhook``): stdlib
  ``http.server`` listens on ``FEISHU_WEBHOOK_PORT`` (default 8453).
  Decrypts AES-256-CBC + PKCS7 payloads when an ``encrypt`` field is
  present and ``FEISHU_ENCRYPT_KEY`` is configured (feishu.rs:1168-1219).
  Honours URL verification challenges by echoing the ``challenge`` field
  (feishu.rs:587-604). Verifies the static ``verification_token`` when
  configured (feishu.rs:587-619). Returns 403 / 400 on token / decrypt
  failure to match the Rust adapter's HTTP status contract.

* **Inbound parsing**: two schemas. ``schema == "2.0"`` (modern v2)
  events are dispatched through ``parse_feishu_event`` (text
  message receive) or ``parse_card_action`` (approval card button
  click). Anything without ``schema`` is the legacy v1 payload
  handled by ``parse_feishu_event_v1`` (feishu.rs:1455-1676).
  Self-skip checks ``sender_type in ("app", "bot")`` per feishu.rs:1542
  to break the agent-echo loop documented at #2435.

* **@mention expansion**: replaces ``@_user_<n>`` placeholders in v2
  text with ``@<display_name>``; ``@_all`` renders as ``@all``
  (feishu.rs:1480-1507).

* **Event dedup**: 5-minute sliding window on ``header.event_id``
  with a soft cap of 10 000 entries
  (``EVENT_DEDUP_WINDOW`` / ``EVENT_DEDUP_MAX_ENTRIES`` at
  feishu.rs:119-125).

* **Outbound send**: ``POST /open-apis/im/v1/messages`` with the
  ``receive_id_type`` query param (always ``chat_id`` for now since
  the Rust adapter hard-coded it at feishu.rs:1769). Text chunking
  at ``MAX_MESSAGE_LEN = 4096`` (feishu.rs:108). Interactive cards go
  through the same endpoint with ``msg_type: "interactive"`` and a
  JSON-encoded ``content`` string (feishu.rs:468-510).

* **Approval-card flow**: ``build_approval_card`` returns a card
  template with Approve / Deny buttons whose ``value`` carries the
  ``request_id`` + ``action``. The webhook / WS gateway routes the
  resulting ``card.action.trigger`` event into a ``/approve <req>``
  or ``/reject <req>`` ``Command`` content (feishu.rs:1336-1414).

* **Processing reaction**: when a message arrives, the adapter posts
  a ``Typing`` emoji reaction to give the user immediate feedback
  (``POST /open-apis/im/v1/messages/{msg}/reactions``). The
  reaction id is held in ``pending_reactions`` keyed by chat_id;
  the next outbound ``Send`` to that chat issues the DELETE before
  the reply lands. Fail-open: reaction errors never block message
  processing (feishu.rs:402-462 / 1083-1166).

Improvements on top of the Rust adapter:

* **No tokio runtime tax**: a single producer thread per adapter, no
  per-task spawn; the daemon decides how many bots to run.
* **Honest fail mode for encrypted webhooks**: a stdlib-only AES-CBC
  implementation lives at the bottom of this module so webhook mode
  with ``FEISHU_ENCRYPT_KEY`` works without pulling ``cryptography``
  into the sidecar's stdlib-only dependency contract.
"""
from __future__ import annotations

import asyncio
import hashlib
import json
import os
import threading
import time
import urllib.error
import urllib.parse
import urllib.request
from base64 import b64decode
from collections import OrderedDict
from http.server import BaseHTTPRequestHandler, HTTPServer
from typing import Any, Callable, Optional

from librefang.sidecar import Content, Field, Schema, SidecarAdapter, protocol, run_stdio_main
from librefang.sidecar import logging as log
from librefang.sidecar.common import (
    MAX_BACKOFF_SECS,
    RETRY_AFTER_DEFAULT_SECS,
    http_request as _http_request,
    parse_retry_after as _parse_retry_after,
    split_message as _split_message,
)
from librefang.sidecar.ws import WebSocketClient as _WebSocketClient


# ---------------------------------------------------------------------------
# Constants — mirror crates/librefang-channels/src/feishu.rs:108-125.
# ---------------------------------------------------------------------------

MAX_MESSAGE_LEN = 4096            # feishu.rs:108
TOKEN_REFRESH_BUFFER_SECS = 300   # feishu.rs:111
WS_INITIAL_BACKOFF_SECS = 2.0     # feishu.rs:114
WS_MAX_BACKOFF_SECS = 60.0        # feishu.rs:117
EVENT_DEDUP_WINDOW_SECS = 300.0   # feishu.rs:122
EVENT_DEDUP_MAX_ENTRIES = 10_000  # feishu.rs:125

SEND_TIMEOUT_SECS = 30.0
READ_TICK_SECS = 30.0
DEFAULT_PING_INTERVAL_SECS = 120.0  # Go SDK default

INITIAL_BACKOFF_SECS = 1.0  # auth-retry initial backoff

# Hard cap on webhook request body. Feishu events are well under 64 KB
# in practice; cap at 1 MiB so a malicious actor can't OOM the sidecar
# by claiming Content-Length: 10G. The Rust adapter inherited axum's
# default 2 MiB body cap; we keep parity within a factor of two.
WEBHOOK_MAX_BODY_BYTES = 1 * 1024 * 1024


# ---------------------------------------------------------------------------
# Region
# ---------------------------------------------------------------------------


class FeishuRegion:
    """Feishu (CN) vs Lark (international). Pure value type — no
    instance state, just module-level constants."""

    CN = "cn"
    INTL = "intl"

    @staticmethod
    def api_base(region: str) -> str:
        if region == FeishuRegion.INTL:
            return "https://open.larksuite.com"
        return "https://open.feishu.cn"

    @staticmethod
    def label(region: str) -> str:
        return "Lark" if region == FeishuRegion.INTL else "Feishu"

    @staticmethod
    def channel_label(region: str) -> str:
        # Inbound ChannelMessage carries the lowercase label as
        # ChannelType::Custom("feishu" | "lark") per feishu.rs:1393.
        return FeishuRegion.label(region).lower()


# ---------------------------------------------------------------------------
# Token cache
# ---------------------------------------------------------------------------


class _TokenCache:
    """Thread-safe access to the tenant access token + its expiry.

    The Rust adapter used ``RwLock<Option<(String, Instant)>>``; we
    mirror that with a plain ``threading.Lock`` since contention is
    low (one refresh every ~2 hours per adapter instance)."""

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

    def set(self, token: str, ttl_secs: float) -> None:
        with self._lock:
            self._token = token
            self._expiry_at = time.monotonic() + max(
                0.0, ttl_secs - TOKEN_REFRESH_BUFFER_SECS,
            )

    def clear(self) -> None:
        with self._lock:
            self._token = None
            self._expiry_at = 0.0


# ---------------------------------------------------------------------------
# Event dedup
# ---------------------------------------------------------------------------


class _EventDedup:
    """Sliding-window dedup on ``header.event_id``.

    Mirrors feishu.rs:1420-1453. ``True`` means "seen — skip"; ``False``
    means freshly recorded. Lazy purge when the map grows past the soft
    cap (10 000 entries default). Thread-safe.
    """

    def __init__(
        self,
        *,
        window_secs: float = EVENT_DEDUP_WINDOW_SECS,
        max_entries: int = EVENT_DEDUP_MAX_ENTRIES,
    ) -> None:
        self._lock = threading.Lock()
        self._window_secs = window_secs
        self._max_entries = max_entries
        # OrderedDict so purge can drop the oldest entries first when
        # the soft cap is exceeded.
        self._seen: "OrderedDict[str, float]" = OrderedDict()

    def is_duplicate(self, event_id: Optional[str]) -> bool:
        if not event_id:
            # No event_id (e.g. challenge, pong) → not dedup-able.
            return False
        now = time.monotonic()
        with self._lock:
            if len(self._seen) >= self._max_entries:
                # Purge expired entries.
                cutoff = now - self._window_secs
                expired = [k for k, ts in self._seen.items() if ts < cutoff]
                for k in expired:
                    self._seen.pop(k, None)
            prev = self._seen.get(event_id)
            if prev is not None and (now - prev) < self._window_secs:
                return True
            # Re-insert at the tail so OrderedDict eviction is FIFO.
            self._seen.pop(event_id, None)
            self._seen[event_id] = now
            return False


# ---------------------------------------------------------------------------
# Approval card builder — mirrors feishu.rs:1231-1325.
# ---------------------------------------------------------------------------


def build_approval_card(
    request_id: str,
    agent_id: str,
    tool_name: str,
    action_summary: str,
    risk_level: str,
) -> dict:
    """Build an interactive Feishu/Lark card for an agent approval
    request. The card carries Approve / Deny buttons whose ``value``
    payload is later routed by ``parse_card_action`` into a
    ``Command`` content with name ``"approve"`` / ``"reject"`` and
    args ``[request_id]``."""
    header_color = {
        "critical": "red",
        "high": "orange",
        "medium": "yellow",
    }.get(risk_level, "blue")
    return {
        "config": {"wide_screen_mode": True},
        "header": {
            "title": {
                "tag": "plain_text",
                "content": f"Agent Permission Request [{risk_level}]",
            },
            "template": header_color,
        },
        "elements": [
            {
                "tag": "div",
                "fields": [
                    {
                        "is_short": True,
                        "text": {
                            "tag": "lark_md",
                            "content": f"**Agent:** {agent_id}",
                        },
                    },
                    {
                        "is_short": True,
                        "text": {
                            "tag": "lark_md",
                            "content": f"**Tool:** `{tool_name}`",
                        },
                    },
                ],
            },
            {
                "tag": "div",
                "text": {
                    "tag": "lark_md",
                    "content": f"**Action:** {action_summary}",
                },
            },
            {
                "tag": "div",
                "text": {
                    "tag": "lark_md",
                    "content": f"**Request ID:** `{request_id}`",
                },
            },
            {"tag": "hr"},
            {
                "tag": "action",
                "actions": [
                    {
                        "tag": "button",
                        "text": {"tag": "plain_text", "content": "Approve"},
                        "type": "primary",
                        "value": {
                            "action": "approve",
                            "request_id": request_id,
                        },
                    },
                    {
                        "tag": "button",
                        "text": {"tag": "plain_text", "content": "Deny"},
                        "type": "danger",
                        "value": {
                            "action": "reject",
                            "request_id": request_id,
                        },
                    },
                ],
            },
        ],
    }


# ---------------------------------------------------------------------------
# Pure-Python AES-256-CBC + PKCS7 — only consumer is encrypted webhook
# payload decryption. Inlined here so the sidecar SDK stays stdlib-only.
# Implementation follows FIPS 197 and PKCS#7 padding (RFC 5652 §6.3).
# ---------------------------------------------------------------------------


# AES S-box.
_AES_SBOX = bytes((
    0x63, 0x7c, 0x77, 0x7b, 0xf2, 0x6b, 0x6f, 0xc5, 0x30, 0x01, 0x67, 0x2b,
    0xfe, 0xd7, 0xab, 0x76, 0xca, 0x82, 0xc9, 0x7d, 0xfa, 0x59, 0x47, 0xf0,
    0xad, 0xd4, 0xa2, 0xaf, 0x9c, 0xa4, 0x72, 0xc0, 0xb7, 0xfd, 0x93, 0x26,
    0x36, 0x3f, 0xf7, 0xcc, 0x34, 0xa5, 0xe5, 0xf1, 0x71, 0xd8, 0x31, 0x15,
    0x04, 0xc7, 0x23, 0xc3, 0x18, 0x96, 0x05, 0x9a, 0x07, 0x12, 0x80, 0xe2,
    0xeb, 0x27, 0xb2, 0x75, 0x09, 0x83, 0x2c, 0x1a, 0x1b, 0x6e, 0x5a, 0xa0,
    0x52, 0x3b, 0xd6, 0xb3, 0x29, 0xe3, 0x2f, 0x84, 0x53, 0xd1, 0x00, 0xed,
    0x20, 0xfc, 0xb1, 0x5b, 0x6a, 0xcb, 0xbe, 0x39, 0x4a, 0x4c, 0x58, 0xcf,
    0xd0, 0xef, 0xaa, 0xfb, 0x43, 0x4d, 0x33, 0x85, 0x45, 0xf9, 0x02, 0x7f,
    0x50, 0x3c, 0x9f, 0xa8, 0x51, 0xa3, 0x40, 0x8f, 0x92, 0x9d, 0x38, 0xf5,
    0xbc, 0xb6, 0xda, 0x21, 0x10, 0xff, 0xf3, 0xd2, 0xcd, 0x0c, 0x13, 0xec,
    0x5f, 0x97, 0x44, 0x17, 0xc4, 0xa7, 0x7e, 0x3d, 0x64, 0x5d, 0x19, 0x73,
    0x60, 0x81, 0x4f, 0xdc, 0x22, 0x2a, 0x90, 0x88, 0x46, 0xee, 0xb8, 0x14,
    0xde, 0x5e, 0x0b, 0xdb, 0xe0, 0x32, 0x3a, 0x0a, 0x49, 0x06, 0x24, 0x5c,
    0xc2, 0xd3, 0xac, 0x62, 0x91, 0x95, 0xe4, 0x79, 0xe7, 0xc8, 0x37, 0x6d,
    0x8d, 0xd5, 0x4e, 0xa9, 0x6c, 0x56, 0xf4, 0xea, 0x65, 0x7a, 0xae, 0x08,
    0xba, 0x78, 0x25, 0x2e, 0x1c, 0xa6, 0xb4, 0xc6, 0xe8, 0xdd, 0x74, 0x1f,
    0x4b, 0xbd, 0x8b, 0x8a, 0x70, 0x3e, 0xb5, 0x66, 0x48, 0x03, 0xf6, 0x0e,
    0x61, 0x35, 0x57, 0xb9, 0x86, 0xc1, 0x1d, 0x9e, 0xe1, 0xf8, 0x98, 0x11,
    0x69, 0xd9, 0x8e, 0x94, 0x9b, 0x1e, 0x87, 0xe9, 0xce, 0x55, 0x28, 0xdf,
    0x8c, 0xa1, 0x89, 0x0d, 0xbf, 0xe6, 0x42, 0x68, 0x41, 0x99, 0x2d, 0x0f,
    0xb0, 0x54, 0xbb, 0x16,
))

# AES inverse S-box.
_AES_INV_SBOX = bytes((
    0x52, 0x09, 0x6a, 0xd5, 0x30, 0x36, 0xa5, 0x38, 0xbf, 0x40, 0xa3, 0x9e,
    0x81, 0xf3, 0xd7, 0xfb, 0x7c, 0xe3, 0x39, 0x82, 0x9b, 0x2f, 0xff, 0x87,
    0x34, 0x8e, 0x43, 0x44, 0xc4, 0xde, 0xe9, 0xcb, 0x54, 0x7b, 0x94, 0x32,
    0xa6, 0xc2, 0x23, 0x3d, 0xee, 0x4c, 0x95, 0x0b, 0x42, 0xfa, 0xc3, 0x4e,
    0x08, 0x2e, 0xa1, 0x66, 0x28, 0xd9, 0x24, 0xb2, 0x76, 0x5b, 0xa2, 0x49,
    0x6d, 0x8b, 0xd1, 0x25, 0x72, 0xf8, 0xf6, 0x64, 0x86, 0x68, 0x98, 0x16,
    0xd4, 0xa4, 0x5c, 0xcc, 0x5d, 0x65, 0xb6, 0x92, 0x6c, 0x70, 0x48, 0x50,
    0xfd, 0xed, 0xb9, 0xda, 0x5e, 0x15, 0x46, 0x57, 0xa7, 0x8d, 0x9d, 0x84,
    0x90, 0xd8, 0xab, 0x00, 0x8c, 0xbc, 0xd3, 0x0a, 0xf7, 0xe4, 0x58, 0x05,
    0xb8, 0xb3, 0x45, 0x06, 0xd0, 0x2c, 0x1e, 0x8f, 0xca, 0x3f, 0x0f, 0x02,
    0xc1, 0xaf, 0xbd, 0x03, 0x01, 0x13, 0x8a, 0x6b, 0x3a, 0x91, 0x11, 0x41,
    0x4f, 0x67, 0xdc, 0xea, 0x97, 0xf2, 0xcf, 0xce, 0xf0, 0xb4, 0xe6, 0x73,
    0x96, 0xac, 0x74, 0x22, 0xe7, 0xad, 0x35, 0x85, 0xe2, 0xf9, 0x37, 0xe8,
    0x1c, 0x75, 0xdf, 0x6e, 0x47, 0xf1, 0x1a, 0x71, 0x1d, 0x29, 0xc5, 0x89,
    0x6f, 0xb7, 0x62, 0x0e, 0xaa, 0x18, 0xbe, 0x1b, 0xfc, 0x56, 0x3e, 0x4b,
    0xc6, 0xd2, 0x79, 0x20, 0x9a, 0xdb, 0xc0, 0xfe, 0x78, 0xcd, 0x5a, 0xf4,
    0x1f, 0xdd, 0xa8, 0x33, 0x88, 0x07, 0xc7, 0x31, 0xb1, 0x12, 0x10, 0x59,
    0x27, 0x80, 0xec, 0x5f, 0x60, 0x51, 0x7f, 0xa9, 0x19, 0xb5, 0x4a, 0x0d,
    0x2d, 0xe5, 0x7a, 0x9f, 0x93, 0xc9, 0x9c, 0xef, 0xa0, 0xe0, 0x3b, 0x4d,
    0xae, 0x2a, 0xf5, 0xb0, 0xc8, 0xeb, 0xbb, 0x3c, 0x83, 0x53, 0x99, 0x61,
    0x17, 0x2b, 0x04, 0x7e, 0xba, 0x77, 0xd6, 0x26, 0xe1, 0x69, 0x14, 0x63,
    0x55, 0x21, 0x0c, 0x7d,
))

# AES Rcon for key expansion.
_AES_RCON = (
    0x01, 0x02, 0x04, 0x08, 0x10, 0x20, 0x40, 0x80, 0x1b, 0x36, 0x6c, 0xd8,
)


def _aes_xtime(b: int) -> int:
    return ((b << 1) ^ 0x1b) & 0xff if b & 0x80 else (b << 1) & 0xff


def _aes_mul(a: int, b: int) -> int:
    """Multiply two bytes in GF(2^8) for MixColumns."""
    res = 0
    for _ in range(8):
        if b & 1:
            res ^= a
        a = _aes_xtime(a)
        b >>= 1
    return res & 0xff


def _aes_expand_key_256(key: bytes) -> list:
    """AES-256 key schedule. Returns 60 4-byte words (15 round keys)."""
    if len(key) != 32:
        raise ValueError("AES-256 key must be 32 bytes")
    nk = 8  # key words
    nr = 14  # rounds
    nb = 4  # block words
    words: list = []
    for i in range(nk):
        words.append(tuple(key[i * 4: i * 4 + 4]))
    for i in range(nk, nb * (nr + 1)):
        temp = list(words[i - 1])
        if i % nk == 0:
            # RotWord + SubWord + Rcon
            temp = [temp[1], temp[2], temp[3], temp[0]]
            temp = [_AES_SBOX[b] for b in temp]
            temp[0] ^= _AES_RCON[(i // nk) - 1]
        elif i % nk == 4:
            # SubWord (no RotWord, no Rcon)
            temp = [_AES_SBOX[b] for b in temp]
        prev = words[i - nk]
        words.append(tuple(prev[j] ^ temp[j] for j in range(4)))
    return words


def _aes_inv_sub_bytes(state: list) -> None:
    for i in range(16):
        state[i] = _AES_INV_SBOX[state[i]]


def _aes_inv_shift_rows(state: list) -> None:
    # state is column-major: state[col*4 + row]
    s = state[:]
    state[0], state[4], state[8], state[12] = s[0], s[4], s[8], s[12]
    state[1], state[5], state[9], state[13] = s[13], s[1], s[5], s[9]
    state[2], state[6], state[10], state[14] = s[10], s[14], s[2], s[6]
    state[3], state[7], state[11], state[15] = s[7], s[11], s[15], s[3]


def _aes_inv_mix_columns(state: list) -> None:
    for c in range(4):
        a0 = state[c * 4 + 0]
        a1 = state[c * 4 + 1]
        a2 = state[c * 4 + 2]
        a3 = state[c * 4 + 3]
        state[c * 4 + 0] = (
            _aes_mul(a0, 0x0e) ^ _aes_mul(a1, 0x0b)
            ^ _aes_mul(a2, 0x0d) ^ _aes_mul(a3, 0x09)
        )
        state[c * 4 + 1] = (
            _aes_mul(a0, 0x09) ^ _aes_mul(a1, 0x0e)
            ^ _aes_mul(a2, 0x0b) ^ _aes_mul(a3, 0x0d)
        )
        state[c * 4 + 2] = (
            _aes_mul(a0, 0x0d) ^ _aes_mul(a1, 0x09)
            ^ _aes_mul(a2, 0x0e) ^ _aes_mul(a3, 0x0b)
        )
        state[c * 4 + 3] = (
            _aes_mul(a0, 0x0b) ^ _aes_mul(a1, 0x0d)
            ^ _aes_mul(a2, 0x09) ^ _aes_mul(a3, 0x0e)
        )


def _aes_add_round_key(state: list, words: list, round_idx: int) -> None:
    for c in range(4):
        w = words[round_idx * 4 + c]
        for r in range(4):
            state[c * 4 + r] ^= w[r]


def _aes_decrypt_block_256(block: bytes, expanded: list) -> bytes:
    """Decrypt one 16-byte AES-256 block."""
    state = list(block)
    nr = 14
    _aes_add_round_key(state, expanded, nr)
    for rnd in range(nr - 1, 0, -1):
        _aes_inv_shift_rows(state)
        _aes_inv_sub_bytes(state)
        _aes_add_round_key(state, expanded, rnd)
        _aes_inv_mix_columns(state)
    _aes_inv_shift_rows(state)
    _aes_inv_sub_bytes(state)
    _aes_add_round_key(state, expanded, 0)
    return bytes(state)


def _aes_256_cbc_decrypt(key: bytes, iv: bytes, ciphertext: bytes) -> bytes:
    """AES-256-CBC decrypt with PKCS#7 unpadding."""
    if len(iv) != 16:
        raise ValueError("AES-CBC IV must be 16 bytes")
    if len(ciphertext) % 16 != 0:
        raise ValueError("ciphertext is not block-aligned")
    if not ciphertext:
        raise ValueError("ciphertext is empty")
    expanded = _aes_expand_key_256(key)
    out = bytearray()
    prev = iv
    for i in range(0, len(ciphertext), 16):
        block = ciphertext[i:i + 16]
        plain = _aes_decrypt_block_256(block, expanded)
        out.extend(bytes(p ^ q for p, q in zip(plain, prev)))
        prev = block
    # PKCS#7 unpad.
    pad = out[-1]
    if pad < 1 or pad > 16:
        raise ValueError("invalid PKCS#7 padding")
    if any(b != pad for b in out[-pad:]):
        raise ValueError("invalid PKCS#7 padding bytes")
    return bytes(out[:-pad])


def decrypt_feishu_payload(encrypted_b64: str, encrypt_key: str) -> dict:
    """Decrypt a Feishu webhook payload. Mirrors feishu.rs:1185-1219.

    The key is ``SHA256(encrypt_key)``; the first 16 bytes of the
    base64-decoded payload are the IV; the rest is AES-256-CBC
    ciphertext with PKCS#7 padding. Returns the parsed JSON dict.
    Raises ``ValueError`` on any decoding / decryption failure.
    """
    raw = b64decode(encrypted_b64.strip(), validate=False)
    if len(raw) < 16:
        raise ValueError("encrypted payload too short")
    iv = raw[:16]
    ciphertext = raw[16:]
    if not ciphertext:
        raise ValueError("encrypted payload ciphertext is empty")
    if len(ciphertext) % 16 != 0:
        raise ValueError("ciphertext is not block-aligned")
    key = hashlib.sha256(encrypt_key.encode("utf-8")).digest()
    plain = _aes_256_cbc_decrypt(key, iv, ciphertext)
    try:
        decoded = json.loads(plain.decode("utf-8"))
    except (UnicodeDecodeError, ValueError) as e:
        raise ValueError(f"decrypted payload is not valid JSON: {e}") from e
    if not isinstance(decoded, dict):
        raise ValueError("decrypted payload is not a JSON object")
    return decoded


def decrypt_feishu_payload_if_needed(
    payload: dict, encrypt_key: Optional[str],
) -> dict:
    """Pass-through unless ``payload`` carries an ``encrypt`` field
    (Feishu's encrypted-events flag). When encrypted, decrypts using
    ``encrypt_key`` and returns the plaintext JSON. Mirrors
    feishu.rs:1168-1183."""
    encrypted = payload.get("encrypt")
    if not isinstance(encrypted, str):
        return payload
    key = (encrypt_key or "").strip()
    if not key:
        raise ValueError(
            "encrypted payload received but no Feishu encrypt_key is configured",
        )
    return decrypt_feishu_payload(encrypted, key)


# ---------------------------------------------------------------------------
# Event parsers — mirror feishu.rs:1336-1676.
# ---------------------------------------------------------------------------


def _content_command_or_text(text: str) -> dict:
    """Slash-prefix routes to Command, anything else to Text. Mirrors
    the dispatch at feishu.rs:1549-1562 / 1646-1659."""
    if text.startswith("/"):
        head, _, tail = text[1:].partition(" ")
        args = tail.split() if tail else []
        return Content.command(head, args)
    return Content.text(text)


def _expand_mentions(text: str, mentions: Any) -> str:
    """Replace ``@_user_<n>`` placeholders in v2 text with
    ``@<display_name>``; ``@_all`` becomes ``@all``. Mirrors
    feishu.rs:1480-1507."""
    if not isinstance(mentions, list) or not mentions:
        return text
    out = text
    for m in mentions:
        if not isinstance(m, dict):
            continue
        key = m.get("key")
        if not isinstance(key, str) or not key:
            continue
        if key == "@_all":
            replacement = "@all"
        else:
            name = m.get("name") if isinstance(m.get("name"), str) else None
            if not name:
                inner = m.get("id")
                if isinstance(inner, dict):
                    open_id = inner.get("open_id")
                    if isinstance(open_id, str) and open_id:
                        name = open_id
            if not name:
                name = "user"
            replacement = f"@{name}"
        out = out.replace(key, replacement)
    return out


def parse_feishu_event(payload: dict, region: str) -> Optional[dict]:
    """Parse a v2 (``schema == "2.0"``) ``im.message.receive_v1``
    event into a sidecar ``message`` event ready to ``emit``.
    Returns ``None`` when the payload should be skipped (wrong event
    type, self, empty body)."""
    if not isinstance(payload, dict):
        return None
    header = payload.get("header")
    if not isinstance(header, dict):
        return None
    if header.get("event_type") != "im.message.receive_v1":
        return None
    event_data = payload.get("event")
    if not isinstance(event_data, dict):
        return None
    message = event_data.get("message")
    sender = event_data.get("sender")
    if not isinstance(message, dict) or not isinstance(sender, dict):
        return None
    if message.get("message_type") != "text":
        return None
    content_str = message.get("content")
    if not isinstance(content_str, str):
        content_str = "{}"
    try:
        content_json = json.loads(content_str)
    except (TypeError, ValueError):
        content_json = {}
    text = ""
    if isinstance(content_json, dict):
        t = content_json.get("text")
        if isinstance(t, str):
            text = t
    text = _expand_mentions(text, message.get("mentions")).strip()
    if not text:
        return None

    sender_type = sender.get("sender_type")
    # Self-skip — feishu.rs:1542. ``app`` is the documented value; we
    # accept the historical ``bot`` string too for proxy-normalised
    # payloads.
    if sender_type in ("app", "bot"):
        return None
    sender_id_block = sender.get("sender_id")
    sender_open_id = ""
    if isinstance(sender_id_block, dict):
        v = sender_id_block.get("open_id")
        if isinstance(v, str):
            sender_open_id = v

    msg_id = message.get("message_id")
    chat_id = message.get("chat_id")
    chat_type = message.get("chat_type")
    root_id = message.get("root_id")
    msg_id = msg_id if isinstance(msg_id, str) else ""
    chat_id = chat_id if isinstance(chat_id, str) else ""
    chat_type = chat_type if isinstance(chat_type, str) else "p2p"
    thread_id = root_id if isinstance(root_id, str) and root_id else None

    channel_label = FeishuRegion.channel_label(region)
    metadata: dict = {
        "chat_id": chat_id,
        "message_id": msg_id,
        "chat_type": chat_type,
        "sender_id": sender_open_id,
        "region": channel_label,
        "was_mentioned": bool(
            isinstance(message.get("mentions"), list)
            and message.get("mentions"),
        ),
    }
    mentions = message.get("mentions")
    if isinstance(mentions, list):
        metadata["mentions"] = mentions

    is_group = chat_type == "group"
    return protocol.message(
        user_id=chat_id,
        user_name=sender_open_id,
        content=_content_command_or_text(text),
        message_id=msg_id,
        channel_id=chat_id,
        thread_id=thread_id,
        is_group=is_group,
        metadata=metadata,
    )


def parse_feishu_event_v1(payload: dict, region: str) -> Optional[dict]:
    """Parse a legacy v1 webhook event. Mirrors feishu.rs:1618-1676.

    v1 has no ``sender_type`` field; we self-skip when ``open_id`` is
    empty (likely bot-originated or malformed)."""
    if not isinstance(payload, dict):
        return None
    event = payload.get("event")
    if not isinstance(event, dict):
        return None
    if event.get("type") != "message":
        return None
    open_id = event.get("open_id")
    if not isinstance(open_id, str) or not open_id:
        return None
    text = event.get("text")
    if not isinstance(text, str) or not text:
        return None
    chat_id_v = event.get("open_chat_id")
    msg_id = event.get("open_message_id")
    chat_id = chat_id_v if isinstance(chat_id_v, str) else ""
    msg_id = msg_id if isinstance(msg_id, str) else ""
    chat_type = event.get("chat_type")
    is_group = isinstance(chat_type, str) and chat_type == "group"
    channel_label = FeishuRegion.channel_label(region)
    metadata = {"region": channel_label}
    return protocol.message(
        user_id=chat_id,
        user_name=open_id,
        content=_content_command_or_text(text),
        message_id=msg_id,
        channel_id=chat_id,
        thread_id=None,
        is_group=is_group,
        metadata=metadata,
    )


def parse_card_action(payload: dict, region: str) -> Optional[dict]:
    """Parse a ``card.action.trigger`` event from an approval card
    button click into a ``Command`` message. Mirrors
    feishu.rs:1336-1414."""
    if not isinstance(payload, dict):
        return None
    header = payload.get("header")
    if not isinstance(header, dict):
        return None
    if header.get("event_type") != "card.action.trigger":
        return None
    event_data = payload.get("event")
    if not isinstance(event_data, dict):
        return None
    action = event_data.get("action")
    if not isinstance(action, dict):
        return None
    value = action.get("value")
    if not isinstance(value, dict):
        return None
    action_type = value.get("action")
    request_id = value.get("request_id")
    if not isinstance(action_type, str) or not isinstance(request_id, str):
        return None
    if action_type not in ("approve", "reject"):
        return None
    operator = event_data.get("operator")
    if not isinstance(operator, dict):
        return None
    open_id = operator.get("open_id") if isinstance(operator.get("open_id"), str) else ""
    open_chat_id = event_data.get("open_chat_id")
    open_message_id = event_data.get("open_message_id")
    open_chat_id = open_chat_id if isinstance(open_chat_id, str) else ""
    open_message_id = open_message_id if isinstance(open_message_id, str) else ""
    channel_label = FeishuRegion.channel_label(region)
    metadata = {
        "chat_id": open_chat_id,
        "message_id": open_message_id,
        "card_action": True,
        "operator_id": open_id,
        "region": channel_label,
    }
    return protocol.message(
        user_id=open_id,
        user_name=open_id,
        content=Content.command(action_type, [request_id]),
        message_id=open_message_id,
        channel_id=open_chat_id,
        thread_id=None,
        is_group=False,
        metadata=metadata,
    )


# ---------------------------------------------------------------------------
# Adapter
# ---------------------------------------------------------------------------


class FeishuAdapter(SidecarAdapter):
    """Feishu / Lark sidecar adapter — unified for both regions and
    both receive modes (WS gateway / HTTP webhook)."""

    # Feishu's surface — text only outbound (Rust adapter parity).
    # Native interactive cards are not wired through ChannelContent
    # by the daemon today; `build_approval_card` + `_send_card` ship
    # as a public utility for callers that build their own cards.
    # Don't claim "interactive" — the on_send branch for Interactive
    # only does text + `[label]` button-hint fallback, same shape
    # the daemon would do itself when the capability is absent.
    capabilities: list = []
    # Inbound is group/p2p mixed — keep error replies on (matches
    # mattermost / matrix default).
    suppress_error_responses: bool = False

    SCHEMA = Schema(
        name="feishu",
        display_name="Feishu / Lark",
        description=(
            "Unified Feishu (CN) / Lark (international) sidecar adapter."
        ),
        fields=[
            Field("FEISHU_APP_ID", "App ID", "text",
                  required=True,
                  placeholder="cli_a..."),
            Field("FEISHU_APP_SECRET", "App Secret", "secret",
                  required=True,
                  placeholder="..."),
            Field("FEISHU_REGION", "Region (cn|intl)", "text",
                  placeholder="cn",
                  advanced=True),
            Field("FEISHU_RECEIVE_MODE",
                  "Receive mode (websocket|webhook)", "text",
                  placeholder="websocket",
                  advanced=True),
            Field("FEISHU_WEBHOOK_PORT",
                  "Webhook port (webhook mode only)", "text",
                  placeholder="8453",
                  advanced=True),
            Field("FEISHU_VERIFICATION_TOKEN",
                  "Verification token (webhook mode)", "secret",
                  advanced=True),
            Field("FEISHU_ENCRYPT_KEY",
                  "Encrypt key (webhook mode)", "secret",
                  advanced=True),
            Field("FEISHU_ACCOUNT_ID",
                  "Account ID (multi-bot routing)", "text",
                  advanced=True),
        ],
    )

    def __init__(self) -> None:
        app_id = os.environ.get("FEISHU_APP_ID", "").strip()
        app_secret = os.environ.get("FEISHU_APP_SECRET", "").strip()
        missing: list[str] = []
        if not app_id:
            missing.append("FEISHU_APP_ID")
        if not app_secret:
            missing.append("FEISHU_APP_SECRET")
        if missing:
            log.error("feishu required env vars missing", missing=missing)
            raise SystemExit(2)

        region = (
            os.environ.get("FEISHU_REGION", "").strip().lower() or "cn"
        )
        if region not in (FeishuRegion.CN, FeishuRegion.INTL):
            log.warn(
                "FEISHU_REGION not 'cn' or 'intl'; defaulting to cn",
                value=region,
            )
            region = FeishuRegion.CN
        self.region = region

        mode = os.environ.get("FEISHU_RECEIVE_MODE", "").strip().lower() or "websocket"
        if mode not in ("websocket", "webhook"):
            log.warn(
                "FEISHU_RECEIVE_MODE not 'websocket' or 'webhook'; "
                "defaulting to websocket",
                value=mode,
            )
            mode = "websocket"
        self.receive_mode = mode

        webhook_port_raw = os.environ.get("FEISHU_WEBHOOK_PORT", "").strip()
        try:
            self.webhook_port = int(webhook_port_raw) if webhook_port_raw else 8453
        except ValueError:
            log.warn(
                "FEISHU_WEBHOOK_PORT not an int; defaulting to 8453",
                value=webhook_port_raw,
            )
            self.webhook_port = 8453

        verification_token = os.environ.get(
            "FEISHU_VERIFICATION_TOKEN", "",
        ).strip()
        self.verification_token: Optional[str] = verification_token or None
        encrypt_key = os.environ.get("FEISHU_ENCRYPT_KEY", "").strip()
        self.encrypt_key: Optional[str] = encrypt_key or None

        self.app_id = app_id
        self.app_secret = app_secret
        acct = os.environ.get("FEISHU_ACCOUNT_ID", "").strip()
        self.account_id = acct or None

        # Test seam — override the REST API base for tests.
        self.api_base_override = os.environ.get(
            "FEISHU_API_BASE_OVERRIDE", "",
        ).strip() or None

        self._token_cache = _TokenCache()
        self._dedup = _EventDedup()
        # Processing-reaction tracking: chat_id → (reaction_id, message_id).
        self._pending_reactions_lock = threading.Lock()
        self._pending_reactions: dict[str, tuple[str, str]] = {}

        self._shutdown = threading.Event()
        self._http_server: Optional[HTTPServer] = None

    # ---- properties --------------------------------------------------

    @property
    def api_base(self) -> str:
        return self.api_base_override or FeishuRegion.api_base(self.region)

    @property
    def label(self) -> str:
        return FeishuRegion.label(self.region)

    @property
    def channel_label(self) -> str:
        return FeishuRegion.channel_label(self.region)

    # ---- HTTP helpers ------------------------------------------------

    def _http_json(
        self,
        url: str,
        *,
        method: str = "POST",
        body: Optional[dict] = None,
        token: Optional[str] = None,
        timeout: float = SEND_TIMEOUT_SECS,
        retry_429: bool = True,
    ) -> tuple[int, Any, bytes, dict]:
        headers: dict = {
            "Content-Type": "application/json; charset=utf-8",
            "User-Agent": "librefang-feishu-sidecar/1 (https://librefang.org)",
        }
        if token:
            headers["Authorization"] = f"Bearer {token}"
        body_bytes: Optional[bytes] = None
        if body is not None:
            body_bytes = json.dumps(body).encode("utf-8")
        status, parsed, raw, resp_hdrs = _http_request(
            url, method=method, body=body_bytes, headers=headers,
            timeout=timeout,
        )
        # Honour Retry-After on 429 — feishu / lark publish hard rate
        # limits per app (e.g. 1000/min for im.message.send) plus a
        # tenant-wide bucket. Without this the next outbound burst
        # extends the rate-limit window.
        if status == 429 and retry_429:
            wait = _parse_retry_after(
                resp_hdrs, default_secs=RETRY_AFTER_DEFAULT_SECS,
            )
            log.warn(
                "feishu 429; sleeping then retrying once",
                url=url,
                retry_after_secs=wait,
            )
            time.sleep(wait)
            return self._http_json(
                url, method=method, body=body, token=token,
                timeout=timeout, retry_429=False,
            )
        return status, parsed, raw, resp_hdrs

    # ---- token + validation ------------------------------------------

    def _refresh_token(self) -> str:
        """Refresh the tenant access token via
        ``/open-apis/auth/v3/tenant_access_token/internal``. Mirrors
        feishu.rs:1021-1075."""
        url = f"{self.api_base}/open-apis/auth/v3/tenant_access_token/internal"
        status, body, raw, _hdrs = self._http_json(
            url, body={"app_id": self.app_id, "app_secret": self.app_secret},
        )
        if status != 200 or not isinstance(body, dict):
            snippet = raw[:200].decode("utf-8", "replace") if raw else ""
            raise RuntimeError(
                f"{self.label} token request failed "
                f"(status={status}): {snippet}",
            )
        code = body.get("code", -1)
        if code != 0:
            msg = body.get("msg", "unknown error")
            raise RuntimeError(f"{self.label} token error: {msg}")
        token = body.get("tenant_access_token")
        if not isinstance(token, str) or not token:
            raise RuntimeError(
                f"{self.label} token response missing tenant_access_token",
            )
        expire = body.get("expire")
        if not isinstance(expire, (int, float)) or expire <= 0:
            expire = 7200
        self._token_cache.set(token, float(expire))
        return token

    def _get_token(self) -> str:
        cached = self._token_cache.get()
        if cached is not None:
            return cached
        return self._refresh_token()

    def _validate(self) -> str:
        """Confirm credentials via ``GET /open-apis/bot/v3/info``.
        Returns the bot's ``app_name`` (used in startup log). Mirrors
        feishu.rs:274-303."""
        token = self._get_token()
        url = f"{self.api_base}/open-apis/bot/v3/info"
        status, body, raw, _hdrs = self._http_json(
            url, method="GET", token=token,
        )
        if status != 200 or not isinstance(body, dict):
            snippet = raw[:200].decode("utf-8", "replace") if raw else ""
            raise RuntimeError(
                f"{self.label} authentication failed "
                f"(status={status}): {snippet}",
            )
        code = body.get("code", -1)
        if code != 0:
            msg = body.get("msg", "unknown error")
            raise RuntimeError(f"{self.label} bot info error: {msg}")
        bot = body.get("bot")
        if isinstance(bot, dict):
            name = bot.get("app_name")
            if isinstance(name, str) and name:
                return name
        return f"{self.label} Bot"

    # ---- outbound send -----------------------------------------------

    def _send_text(self, chat_id: str, text: str) -> None:
        if not chat_id:
            log.warn("feishu send_text: empty chat_id, dropping")
            return
        token = self._get_token()
        encoded_type = urllib.parse.quote("chat_id", safe="")
        url = (
            f"{self.api_base}/open-apis/im/v1/messages"
            f"?receive_id_type={encoded_type}"
        )
        for chunk in _split_message(text, MAX_MESSAGE_LEN):
            content_json = json.dumps({"text": chunk})
            payload = {
                "receive_id": chat_id,
                "msg_type": "text",
                "content": content_json,
            }
            status, body, raw, _hdrs = self._http_json(
                url, body=payload, token=token,
            )
            if status < 200 or status >= 300:
                snippet = raw[:200].decode("utf-8", "replace") if raw else ""
                raise RuntimeError(
                    f"{self.label} send message error "
                    f"(status={status}): {snippet}",
                )
            if isinstance(body, dict):
                code = body.get("code", -1)
                if code != 0:
                    log.warn(
                        "feishu send message API error",
                        code=code, msg=body.get("msg", "unknown"),
                    )

    def _send_card(self, chat_id: str, card: dict) -> None:
        if not chat_id:
            log.warn("feishu send_card: empty chat_id, dropping")
            return
        token = self._get_token()
        encoded_type = urllib.parse.quote("chat_id", safe="")
        url = (
            f"{self.api_base}/open-apis/im/v1/messages"
            f"?receive_id_type={encoded_type}"
        )
        payload = {
            "receive_id": chat_id,
            "msg_type": "interactive",
            "content": json.dumps(card),
        }
        status, body, raw, _hdrs = self._http_json(
            url, body=payload, token=token,
        )
        if status < 200 or status >= 300:
            snippet = raw[:200].decode("utf-8", "replace") if raw else ""
            raise RuntimeError(
                f"{self.label} send card error "
                f"(status={status}): {snippet}",
            )
        if isinstance(body, dict):
            code = body.get("code", -1)
            if code != 0:
                raise RuntimeError(
                    f"{self.label} send card API error "
                    f"(code={code}): {body.get('msg', 'unknown')}",
                )

    # ---- processing reaction (best-effort) ---------------------------

    def _add_processing_reaction(self, chat_id: str, msg_id: str) -> None:
        """Post a ``Typing`` emoji reaction to give the user visual
        feedback. Tracked in ``_pending_reactions`` so the next outbound
        Send to ``chat_id`` removes it. Fail-open. Mirrors
        feishu.rs:1083-1166."""
        if not chat_id or not msg_id:
            return
        with self._pending_reactions_lock:
            if chat_id in self._pending_reactions:
                return  # de-dup
        try:
            token = self._get_token()
        except Exception as e:  # noqa: BLE001
            log.warn("feishu reaction add: get_token failed", error=str(e))
            return
        url = (
            f"{self.api_base}/open-apis/im/v1/messages/"
            f"{urllib.parse.quote(msg_id, safe='')}/reactions"
        )
        try:
            status, body, _raw, _hdrs = self._http_json(
                url, body={"reaction_type": {"emoji_type": "Typing"}},
                token=token,
            )
        except Exception as e:  # noqa: BLE001
            log.warn("feishu reaction add: HTTP failed", error=str(e))
            return
        if status >= 300 or not isinstance(body, dict):
            return
        if body.get("code", -1) != 0:
            log.warn(
                "feishu reaction add API error",
                msg=body.get("msg", "unknown"),
            )
            return
        data = body.get("data")
        if isinstance(data, dict):
            reaction_id = data.get("reaction_id")
            if isinstance(reaction_id, str) and reaction_id:
                with self._pending_reactions_lock:
                    self._pending_reactions[chat_id] = (reaction_id, msg_id)

    def _remove_processing_reaction(self, chat_id: str) -> None:
        if not chat_id:
            return
        with self._pending_reactions_lock:
            entry = self._pending_reactions.pop(chat_id, None)
        if entry is None:
            return
        reaction_id, msg_id = entry
        try:
            token = self._get_token()
        except Exception as e:  # noqa: BLE001
            log.warn("feishu reaction remove: get_token failed", error=str(e))
            return
        url = (
            f"{self.api_base}/open-apis/im/v1/messages/"
            f"{urllib.parse.quote(msg_id, safe='')}/reactions/"
            f"{urllib.parse.quote(reaction_id, safe='')}"
        )
        try:
            self._http_json(url, method="DELETE", token=token, body=None)
        except Exception as e:  # noqa: BLE001
            log.warn("feishu reaction remove: HTTP failed", error=str(e))

    # ---- sidecar surface ---------------------------------------------

    async def on_send(self, cmd) -> None:
        chat_id = (
            cmd.channel_id
            or (cmd.user.get("platform_id") if cmd.user else "")
            or ""
        )
        if not chat_id:
            log.warn("feishu on_send: empty chat_id, dropping")
            return
        loop = asyncio.get_event_loop()
        # Best-effort: remove the processing reaction before sending
        # so the user sees Typing clear at the same moment the reply
        # arrives. Fire-and-forget — the send is what matters, an
        # extra HTTP roundtrip waiting on DELETE would just slow
        # every reply by ~100-300 ms. Matches the Rust adapter's
        # `tokio::spawn` shape (feishu.rs:420).
        threading.Thread(
            target=self._remove_processing_reaction,
            args=(chat_id,),
            daemon=True,
        ).start()
        text = cmd.text or ""
        content = cmd.content
        if isinstance(content, dict) and "Text" in content:
            await loop.run_in_executor(
                None, lambda: self._send_text(chat_id, text),
            )
            return
        if isinstance(content, dict) and "Interactive" in content:
            # Render Interactive into an approval-style card with
            # button hints. The daemon-side ChannelContent::Interactive
            # carries text + buttons; Feishu has native cards but the
            # Rust adapter (feishu.rs:1767-1776) only ever sent text +
            # falling back for everything else, so the simplest faithful
            # behaviour is text rendering of the prompt.
            ix = content.get("Interactive")
            if isinstance(ix, dict):
                txt = ix.get("text", "")
                btns = ix.get("buttons") or []
                rendered = self._format_buttons(txt, btns)
                await loop.run_in_executor(
                    None, lambda: self._send_text(chat_id, rendered),
                )
                return
        # Fallback for any other variant — match Rust's
        # "(Unsupported content type)" placeholder (feishu.rs:1772-1774).
        await loop.run_in_executor(
            None,
            lambda: self._send_text(
                chat_id, text or "(Unsupported content type)",
            ),
        )

    @staticmethod
    def _format_buttons(text: str, buttons: list) -> str:
        if not buttons:
            return text
        out = text
        for i, row in enumerate(buttons):
            if not isinstance(row, list):
                continue
            if i > 0 or out:
                out += "\n"
            for btn in row:
                if not isinstance(btn, dict):
                    continue
                label = btn.get("label", "")
                if isinstance(label, str):
                    out += f"[{label}] "
        return out.rstrip()

    # ---- produce: dispatch to WS or webhook mode ---------------------

    async def produce(self, emit: Callable[[dict], None]) -> None:
        loop = asyncio.get_event_loop()
        if self.receive_mode == "webhook":
            await loop.run_in_executor(
                None, self._webhook_loop, emit,
            )
        else:
            await loop.run_in_executor(
                None, self._ws_loop, emit,
            )

    async def on_shutdown(self) -> None:
        self._shutdown.set()
        if self._http_server is not None:
            try:
                self._http_server.shutdown()
            except Exception:  # noqa: BLE001
                pass

    # ---- WS gateway loop ---------------------------------------------

    def _get_ws_endpoint(self) -> tuple[str, dict]:
        """Two-step endpoint discovery — POST app credentials to
        ``/callback/ws/endpoint`` and read the real ``wss://`` URL
        out of the response. Returns ``(url, client_config)``.
        Mirrors feishu.rs:982-1017."""
        url = f"{self.api_base}/callback/ws/endpoint"
        status, body, raw, _hdrs = self._http_json(
            url, body={"AppID": self.app_id, "AppSecret": self.app_secret},
        )
        if status < 200 or status >= 300 or not isinstance(body, dict):
            snippet = raw[:200].decode("utf-8", "replace") if raw else ""
            raise RuntimeError(
                f"feishu ws endpoint failed (status={status}): {snippet}",
            )
        code = body.get("code", -1)
        if code != 0:
            raise RuntimeError(
                f"feishu ws endpoint error: code={code} "
                f"msg={body.get('msg', 'unknown')}",
            )
        data = body.get("data") or {}
        ws_url = data.get("URL") if isinstance(data, dict) else None
        if not isinstance(ws_url, str) or not ws_url:
            raise RuntimeError("feishu ws endpoint returned empty URL")
        client_config = (
            data.get("ClientConfig", {})
            if isinstance(data, dict) else {}
        )
        return ws_url, client_config if isinstance(client_config, dict) else {}

    def _ws_loop(self, emit: Callable[[dict], None]) -> None:
        """Outer reconnect loop. Validates auth (with retries), then
        loops the WS session with 2→60 s exponential reconnect
        backoff."""
        # Auth validation gate.
        backoff = INITIAL_BACKOFF_SECS
        while not self._shutdown.is_set():
            try:
                name = self._validate()
                log.info("feishu authenticated", bot_name=name)
                break
            except Exception as e:  # noqa: BLE001
                log.warn("feishu auth failed; will retry",
                         error=str(e), delay=backoff)
                if self._shutdown.wait(backoff):
                    return
                backoff = min(backoff * 2.0, MAX_BACKOFF_SECS)

        backoff = WS_INITIAL_BACKOFF_SECS
        while not self._shutdown.is_set():
            try:
                ws_url, client_cfg = self._get_ws_endpoint()
            except Exception as e:  # noqa: BLE001
                log.warn("feishu ws endpoint failed",
                         error=str(e), delay=backoff)
                if self._shutdown.wait(backoff):
                    return
                backoff = min(backoff * 2.0, WS_MAX_BACKOFF_SECS)
                continue

            ping_secs = DEFAULT_PING_INTERVAL_SECS
            ping_cfg = client_cfg.get("PingInterval") if isinstance(client_cfg, dict) else None
            if isinstance(ping_cfg, (int, float)) and ping_cfg > 0:
                ping_secs = float(ping_cfg)

            # Log only the host, never the full URL. Feishu's gateway
            # URLs carry a session signature in the query string; even
            # short-lived, it doesn't belong in operator logs.
            ws_host = urllib.parse.urlparse(ws_url).hostname or "<unknown>"
            log.info("feishu ws connecting", host=ws_host)
            try:
                with self._make_ws(ws_url) as ws:
                    backoff = WS_INITIAL_BACKOFF_SECS
                    self._run_ws_session(ws, emit, ping_secs=ping_secs)
            except Exception as e:  # noqa: BLE001
                log.warn("feishu ws session error", error=str(e))

            if self._shutdown.is_set():
                return
            log.warn("feishu ws disconnected, reconnecting",
                     delay=backoff)
            if self._shutdown.wait(backoff):
                return
            backoff = min(backoff * 2.0, WS_MAX_BACKOFF_SECS)

    def _make_ws(self, url: str) -> _WebSocketClient:
        """Test seam."""
        return _WebSocketClient(url, headers={})

    def _run_ws_session(
        self,
        ws: _WebSocketClient,
        emit: Callable[[dict], None],
        *,
        ping_secs: float,
    ) -> None:
        last_ping = time.monotonic()
        ws.settimeout(None)
        while not self._shutdown.is_set():
            # Heartbeat tick gate.
            now = time.monotonic()
            if now - last_ping >= ping_secs:
                try:
                    ws.send_text(json.dumps({"type": "ping"}))
                except OSError as e:
                    log.warn("feishu ws ping failed", error=str(e))
                    return
                last_ping = now
            tick = max(1.0, ping_secs - (now - last_ping))
            if not ws.wait_readable(min(tick, READ_TICK_SECS)):
                continue
            try:
                text, binary, close = ws.recv_any_frame()
            except (EOFError, OSError) as e:
                log.warn("feishu ws socket dropped", error=str(e))
                return
            if close is not None:
                code, reason = close
                log.info("feishu ws closed",
                         code=code,
                         reason=reason.decode("utf-8", "replace"))
                return
            if text is not None:
                self._handle_ws_text(text, emit)
            elif binary is not None:
                self._handle_ws_binary(binary, emit)
            # Both None → pong / unknown — ignore.

    def _handle_ws_text(
        self, text: str, emit: Callable[[dict], None],
    ) -> None:
        try:
            payload = json.loads(text)
        except (ValueError, TypeError):
            log.warn("feishu ws text JSON parse failed")
            return
        if not isinstance(payload, dict):
            return
        if payload.get("type") == "pong":
            return
        self._dispatch_event(payload, emit)

    def _handle_ws_binary(
        self, data: bytes, emit: Callable[[dict], None],
    ) -> None:
        # Feishu sends events as protobuf-wrapped binary frames; the
        # JSON object is embedded between the first '{' and last '}'.
        # Same heuristic as feishu.rs:855-866.
        start = data.find(b"{")
        end = data.rfind(b"}")
        if start < 0 or end < 0 or end < start:
            return
        try:
            text = data[start:end + 1].decode("utf-8")
            payload = json.loads(text)
        except (UnicodeDecodeError, ValueError, TypeError):
            log.warn("feishu ws binary JSON parse failed")
            return
        if not isinstance(payload, dict):
            return
        self._dispatch_event(payload, emit)

    # ---- webhook server loop -----------------------------------------

    def _webhook_loop(self, emit: Callable[[dict], None]) -> None:
        try:
            bot_name = self._validate()
            log.info("feishu authenticated", bot_name=bot_name)
        except Exception as e:  # noqa: BLE001
            # Don't fail-hard — webhook can still RECEIVE events even
            # without a valid token; reply paths will surface the auth
            # error themselves. But escalate to error-level so operators
            # see this in the log instead of dismissing a warning.
            log.error(
                "feishu webhook validation failed; bot will receive "
                "events but be unable to reply until credentials are "
                "fixed",
                error=str(e),
            )
        handler_cls = _make_webhook_handler(self, emit)
        try:
            srv = HTTPServer(("0.0.0.0", self.webhook_port), handler_cls)
        except OSError as e:
            # Bind failure inside the runtime loop — surface as a
            # regular exception so the runtime's producer-crash wrapper
            # (see runtime.py) can log + cleanly exit with non-zero
            # status. SystemExit raised from inside the event loop
            # bypasses the cleanup path on older Pythons.
            log.error(
                "feishu webhook bind failed",
                port=self.webhook_port, error=str(e),
            )
            raise RuntimeError(
                f"feishu webhook bind failed on port "
                f"{self.webhook_port}: {e}",
            ) from e
        self._http_server = srv
        log.info("feishu webhook listening", port=self.webhook_port)
        try:
            srv.serve_forever(poll_interval=0.5)
        except Exception as e:  # noqa: BLE001
            log.warn("feishu webhook serve loop ended", error=str(e))
        finally:
            try:
                srv.server_close()
            except Exception:  # noqa: BLE001
                pass

    # ---- common event dispatch (WS + webhook) ------------------------

    def _dispatch_event(
        self, payload: dict, emit: Callable[[dict], None],
    ) -> None:
        event_id = None
        header = payload.get("header")
        if isinstance(header, dict):
            ev = header.get("event_id")
            if isinstance(ev, str):
                event_id = ev
        if self._dedup.is_duplicate(event_id):
            return
        schema = payload.get("schema")
        ev_dict: Optional[dict]
        if isinstance(schema, str) and schema == "2.0":
            ev_dict = parse_feishu_event(payload, self.region)
            if ev_dict is None:
                ev_dict = parse_card_action(payload, self.region)
        else:
            ev_dict = parse_feishu_event_v1(payload, self.region)
        if ev_dict is None:
            return
        # Inject account_id metadata when configured.
        if self.account_id is not None:
            params = ev_dict.get("params")
            if isinstance(params, dict):
                meta = params.setdefault("metadata", {})
                if isinstance(meta, dict):
                    meta.setdefault("account_id", self.account_id)
        # Spawn the processing reaction (fail-open, best effort).
        params = ev_dict.get("params")
        if isinstance(params, dict):
            meta = params.get("metadata")
            chat_id = ""
            msg_id = ""
            if isinstance(meta, dict):
                v = meta.get("chat_id")
                chat_id = v if isinstance(v, str) else ""
                v = meta.get("message_id")
                msg_id = v if isinstance(v, str) else ""
            if chat_id and msg_id:
                threading.Thread(
                    target=self._add_processing_reaction,
                    args=(chat_id, msg_id),
                    daemon=True,
                ).start()
        emit(ev_dict)


# ---------------------------------------------------------------------------
# Webhook HTTP handler factory
# ---------------------------------------------------------------------------


def _make_webhook_handler(adapter: FeishuAdapter, emit: Callable[[dict], None]):
    """Build a BaseHTTPRequestHandler subclass that closes over the
    given adapter + emit callback. Routes ``POST /webhook`` to the
    Feishu event-processing pipeline. Mirrors feishu.rs:536-682."""

    class _Handler(BaseHTTPRequestHandler):
        def log_message(self, format, *args):  # noqa: A002 — match signature
            # Quiet by default — we have our own structured logger.
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
                # Bail before allocating — a malicious actor advertising
                # Content-Length: 10G would otherwise drag the sidecar OOM.
                # Mirrors axum's default `DefaultBodyLimit`.
                log.warn(
                    "feishu webhook rejected oversized body",
                    content_length=length, cap=WEBHOOK_MAX_BODY_BYTES,
                )
                self.send_response(413)
                self.end_headers()
                return
            raw = self.rfile.read(length) if length > 0 else b""
            try:
                body = json.loads(raw.decode("utf-8")) if raw else {}
            except (UnicodeDecodeError, ValueError):
                self.send_response(400)
                self.end_headers()
                return
            if not isinstance(body, dict):
                self.send_response(400)
                self.end_headers()
                return

            # Decrypt encrypted payload if needed.
            try:
                payload = decrypt_feishu_payload_if_needed(
                    body, adapter.encrypt_key,
                )
            except ValueError as e:
                log.warn("feishu webhook decrypt failed", error=str(e))
                self.send_response(400)
                self.end_headers()
                return

            # URL verification challenge (sent by Feishu when the
            # webhook is configured in the admin console).
            challenge = payload.get("challenge")
            if isinstance(challenge, str):
                if adapter.verification_token is not None:
                    token = payload.get("token")
                    if token != adapter.verification_token:
                        log.warn(
                            "feishu webhook: invalid verification token "
                            "on challenge",
                        )
                        self.send_response(403)
                        self.end_headers()
                        return
                resp_body = json.dumps({"challenge": challenge}).encode("utf-8")
                self.send_response(200)
                self.send_header("Content-Type", "application/json")
                self.send_header("Content-Length", str(len(resp_body)))
                self.end_headers()
                self.wfile.write(resp_body)
                return

            # Verify token on non-challenge events.
            if adapter.verification_token is not None:
                actual = payload.get("token")
                if not isinstance(actual, str):
                    actual_hdr = payload.get("header")
                    if isinstance(actual_hdr, dict):
                        v = actual_hdr.get("token")
                        if isinstance(v, str):
                            actual = v
                if actual != adapter.verification_token:
                    log.warn(
                        "feishu webhook: invalid verification token on event",
                    )
                    self.send_response(403)
                    self.end_headers()
                    return

            adapter._dispatch_event(payload, emit)
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", "2")
            self.end_headers()
            self.wfile.write(b"{}")

    return _Handler


if __name__ == "__main__":
    run_stdio_main(FeishuAdapter)
